use anyhow::{Context, Result};
use futures_util::StreamExt;
use std::collections::HashMap;
use tokio::sync::mpsc;
use zbus::{Connection, proxy};

#[derive(Debug, Clone)]
pub enum SessionEvent {
    StartupSnapshot { sessions: Vec<(u32, String)> },
    SessionStarted { uid: u32, session_id: String },
    SessionEnded { uid: u32, session_id: String },
    StateChanged { uid: u32, session_id: String, state: SessionUsageState },
    PrepareForSleep { suspend: bool },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SessionUsageState {
    pub active: Option<bool>,
    pub locked: Option<bool>,
    pub idle: Option<bool>,
}

pub(crate) fn session_counts_usage(state: &SessionUsageState) -> bool {
    state.active == Some(true)
        && state.locked == Some(false)
        && state.idle == Some(false)
}

pub(crate) fn uid_counts_usage<'a>(
    states: impl IntoIterator<Item = &'a SessionUsageState>,
) -> bool {
    states.into_iter().any(session_counts_usage)
}

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Manager {
    #[zbus(signal)]
    fn session_new(&self, session_id: String, object_path: zbus::zvariant::OwnedObjectPath) -> Result<()>;

    #[zbus(signal)]
    fn session_removed(&self, session_id: String, object_path: zbus::zvariant::OwnedObjectPath) -> Result<()>;

    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> Result<()>;

    fn list_sessions(
        &self,
    ) -> zbus::Result<Vec<(String, u32, String, String, zbus::zvariant::OwnedObjectPath)>>;

    fn kill_session(&self, session_id: &str, who: &str, signal_number: i32) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait Login1Session {
    #[zbus(property)]
    fn user(&self) -> zbus::Result<(u32, zbus::zvariant::OwnedObjectPath)>;

    #[zbus(property)]
    fn idle_hint(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn active(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn locked_hint(&self) -> zbus::Result<bool>;

    #[zbus(property, name = "Type")]
    fn session_type(&self) -> zbus::Result<String>;

    fn lock(&self) -> zbus::Result<()>;

    fn unlock(&self) -> zbus::Result<()>;

    fn terminate(&self) -> zbus::Result<()>;
}

pub struct DbusMonitor {
    conn: Connection,
    tx: mpsc::Sender<SessionEvent>,
}

impl DbusMonitor {
    pub async fn new(tx: mpsc::Sender<SessionEvent>) -> Result<Self> {
        let conn = Connection::system().await?;
        Ok(Self { conn, tx })
    }

    pub async fn run(self) -> Result<()> {
        let manager = Login1ManagerProxy::new(&self.conn).await?;
        let mut session_uids = HashMap::new();

        // Emit events for graphical sessions already present at startup.
        // TTY/SSH sessions are excluded — they are never idle and must not count as screen time.
        if let Ok(sessions) = manager.list_sessions().await {
            let mut graphical = Vec::new();
            for (session_id, uid, _user, _seat, path) in sessions {
                if !self.is_graphical_session(&path).await {
                    continue;
                }
                graphical.push((session_id, uid, path));
            }

            // Snapshot first so heartbeat can reconcile DB before processing any SessionStarted.
            let _ = self.tx.send(SessionEvent::StartupSnapshot {
                sessions: graphical.iter().map(|(sid, uid, _)| (*uid, sid.clone())).collect(),
            }).await;

            for (session_id, uid, path) in graphical {
                session_uids.insert(session_id.clone(), uid);
                let state = self.get_session_state(&path, uid, &session_id).await;
                let _ = self.tx.send(SessionEvent::SessionStarted {
                    uid,
                    session_id: session_id.clone(),
                }).await;
                let _ = self.tx.send(SessionEvent::StateChanged {
                    uid,
                    session_id: session_id.clone(),
                    state,
                }).await;
                let tx = self.tx.clone();
                let conn = self.conn.clone();
                tokio::spawn(async move {
                    let _ = watch_session_state(conn, path, uid, session_id, tx).await;
                });
            }
        }

        let mut new_stream = manager.receive_session_new().await?;
        let mut removed_stream = manager.receive_session_removed().await?;
        let mut sleep_stream = manager.receive_prepare_for_sleep().await?;

        loop {
            tokio::select! {
                Some(signal) = new_stream.next() => {
                    let args = signal.args()?;
                    let session_id = args.session_id.to_string();
                    let path = args.object_path.clone();
                    let uid = self.get_session_uid(&path).await.unwrap_or(0);
                    if uid > 0 && self.is_graphical_session(&path).await {
                        tracing::info!("Session started: uid={uid} session={session_id}");
                        session_uids.insert(session_id.clone(), uid);
                        let state = self.get_session_state(&path, uid, &session_id).await;
                        let _ = self.tx.send(SessionEvent::SessionStarted {
                            uid,
                            session_id: session_id.clone(),
                        }).await;
                        let tx = self.tx.clone();
                        let conn = self.conn.clone();
                        let sid = session_id.clone();
                        let p = path.clone();
                        tokio::spawn(async move {
                            let _ = watch_session_state(conn, p, uid, sid, tx).await;
                        });
                        let _ = self.tx.send(SessionEvent::StateChanged {
                            uid,
                            session_id,
                            state,
                        }).await;
                    }
                }
                Some(signal) = removed_stream.next() => {
                    let args = signal.args()?;
                    let session_id = args.session_id.to_string();
                    let path = args.object_path.clone();
                    let uid = match session_uids.remove(&session_id) {
                        Some(uid) => uid,
                        None => self.get_session_uid(&path).await.unwrap_or(0),
                    };
                    tracing::info!("Session ended: uid={uid} session={session_id}");
                    let _ = self.tx.send(SessionEvent::SessionEnded { uid, session_id }).await;
                }
                Some(signal) = sleep_stream.next() => {
                    let args = signal.args()?;
                    let _ = self.tx.send(SessionEvent::PrepareForSleep {
                        suspend: args.start,
                    }).await;
                }
            }
        }
    }

    async fn get_session_uid(&self, path: &zbus::zvariant::OwnedObjectPath) -> Result<u32> {
        let session = Login1SessionProxy::builder(&self.conn)
            .path(path.as_ref())?
            .build()
            .await?;
        let (uid, _) = session.user().await?;
        Ok(uid)
    }

    async fn get_session_state(
        &self,
        path: &zbus::zvariant::OwnedObjectPath,
        uid: u32,
        session_id: &str,
    ) -> SessionUsageState {
        let Ok(builder) = Login1SessionProxy::builder(&self.conn).path(path.as_ref()) else {
            tracing::warn!("Cannot monitor state for uid={uid} session={session_id}: invalid object path");
            return SessionUsageState::default();
        };
        let Ok(session) = builder.build().await else {
            tracing::warn!("Cannot monitor state for uid={uid} session={session_id}: proxy unavailable");
            return SessionUsageState::default();
        };
        read_session_state(&session, uid, session_id).await
    }

    /// Returns true only for graphical sessions (x11 or wayland).
    /// TTY, SSH, and other non-graphical sessions are never counted as screen time.
    async fn is_graphical_session(&self, path: &zbus::zvariant::OwnedObjectPath) -> bool {
        let Ok(session) = Login1SessionProxy::builder(&self.conn)
            .path(path.as_ref())
            .and_then(|b| Ok(b))
        else {
            return false;
        };
        let Ok(session) = session.build().await else { return false; };
        matches!(
            session.session_type().await.as_deref(),
            Ok("x11") | Ok("wayland") | Ok("mir")
        )
    }
}

async fn watch_session_state(
    conn: Connection,
    path: zbus::zvariant::OwnedObjectPath,
    uid: u32,
    session_id: String,
    tx: mpsc::Sender<SessionEvent>,
) -> Result<()> {
    let session = Login1SessionProxy::builder(&conn)
        .path(path.as_ref())?
        .build()
        .await?;

    let mut active_stream = session.receive_active_changed().await;
    let mut locked_stream = session.receive_locked_hint_changed().await;
    let mut idle_stream = session.receive_idle_hint_changed().await;
    let state = read_session_state(&session, uid, &session_id).await;
    let _ = tx.send(SessionEvent::StateChanged {
        uid,
        session_id: session_id.clone(),
        state,
    }).await;
    loop {
        tokio::select! {
            Some(_) = active_stream.next() => {}
            Some(_) = locked_stream.next() => {}
            Some(_) = idle_stream.next() => {}
            else => break,
        }
        let state = read_session_state(&session, uid, &session_id).await;
        let _ = tx.send(SessionEvent::StateChanged {
            uid,
            session_id: session_id.clone(),
            state,
        }).await;
    }
    Ok(())
}

async fn read_session_state(
    session: &Login1SessionProxy<'_>,
    uid: u32,
    session_id: &str,
) -> SessionUsageState {
    let active = session.active().await.map_err(|e| {
        tracing::warn!("Cannot read Active for uid={uid} session={session_id}: {e}");
        e
    }).ok();
    let locked = session.locked_hint().await.map_err(|e| {
        tracing::warn!("Cannot read LockedHint for uid={uid} session={session_id}: {e}");
        e
    }).ok();
    let idle = session.idle_hint().await.map_err(|e| {
        tracing::warn!("Cannot read IdleHint for uid={uid} session={session_id}: {e}");
        e
    }).ok();
    SessionUsageState { active, locked, idle }
}

/// Lock all sessions in the given list via DBus.
pub async fn lock_sessions(session_ids: &[String]) -> Result<()> {
    let conn = Connection::system().await?;
    let manager = Login1ManagerProxy::new(&conn).await?;
    let sessions = manager.list_sessions().await?;

    for (sid, _uid, _user, _seat, path) in &sessions {
        if session_ids.contains(sid)
            && let Ok(session) = Login1SessionProxy::builder(&conn)
                .path(path.as_ref())?
                .build()
                .await
            {
                let _ = session.lock().await;
            }
    }
    tracing::info!("Locked {} session(s): {:?}", session_ids.len(), session_ids);
    Ok(())
}

/// Unlock all sessions in the given list via DBus.
/// Called when enforcement is lifted (e.g. admin grants more time).
pub async fn unlock_sessions(session_ids: &[String]) -> Result<()> {
    let conn = Connection::system().await?;
    let manager = Login1ManagerProxy::new(&conn).await?;
    let sessions = manager.list_sessions().await?;

    for (sid, _uid, _user, _seat, path) in &sessions {
        if session_ids.contains(sid)
            && let Ok(session) = Login1SessionProxy::builder(&conn)
                .path(path.as_ref())?
                .build()
                .await
            {
                let _ = session.unlock().await;
            }
    }
    tracing::info!("Unlocked {} session(s): {:?}", session_ids.len(), session_ids);
    Ok(())
}

/// Terminate all sessions in the given list via DBus.
/// Uses Manager.KillSession with SIGKILL so that lock-screen daemons (kscreenlocker,
/// swaylock, etc.) that resist SIGTERM are forcefully removed.
pub async fn terminate_sessions(session_ids: &[String]) -> Result<()> {
    let conn = Connection::system().await?;
    let manager = Login1ManagerProxy::new(&conn).await?;

    for sid in session_ids {
        match manager.kill_session(sid, "all", 9).await {
            Ok(()) => tracing::info!("Sent SIGKILL to session {sid}"),
            Err(e) => tracing::warn!("kill_session failed for {sid}: {e}"),
        }
    }
    tracing::info!("Terminated {} session(s): {:?}", session_ids.len(), session_ids);
    Ok(())
}

/// Send a desktop notification to a user by spawning the agent binary as that user.
/// This avoids the D-Bus peer-credential rejection that occurs when root connects directly
/// to a user's session bus socket.
pub async fn send_desktop_notification(uid: u32, summary: &str, body: &str) -> Result<()> {
    let socket = format!("/run/user/{uid}/bus");
    if !std::path::Path::new(&socket).exists() {
        tracing::debug!("No session bus for uid={uid}, skipping notification");
        return Ok(());
    }

    let exe = std::env::current_exe().context("Cannot determine agent executable path")?;
    let gid = lookup_gid_for_uid(uid).unwrap_or(uid);
    let summary = summary.to_string();
    let body = body.to_string();

    tokio::task::spawn_blocking(move || {
        use std::os::unix::process::CommandExt;
        let result = std::process::Command::new(&exe)
            .arg("--notify")
            .arg(&summary)
            .arg(&body)
            .env_clear()
            .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path=/run/user/{uid}/bus"))
            .env("XDG_RUNTIME_DIR", format!("/run/user/{uid}"))
            .uid(uid)
            .gid(gid)
            .spawn();
        if let Err(e) = result {
            tracing::warn!("Failed to spawn notification subprocess for uid={uid}: {e}");
        }
    })
    .await?;

    Ok(())
}

/// Called when the agent binary is invoked with `--notify`.
/// At this point the process is already running as the target user, so the session
/// bus connection uses the correct peer credentials.
pub async fn notify_as_current_user(summary: &str, body: &str) -> Result<()> {
    let conn = zbus::Connection::session().await?;
    let proxy = NotificationsProxy::new(&conn).await?;
    let hints: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
    let _ = proxy
        .notify("ScreenGuard", 0, "dialog-information", summary, body, &[], hints, 5000)
        .await;
    Ok(())
}

fn lookup_gid_for_uid(uid: u32) -> Option<u32> {
    let content = std::fs::read_to_string("/etc/passwd").ok()?;
    for line in content.lines() {
        let mut fields = line.split(':');
        let _name = fields.next()?;
        let _pass = fields.next()?;
        let uid_str = fields.next()?;
        let gid_str = fields.next()?;
        if uid_str.parse::<u32>().ok() == Some(uid) {
            return gid_str.parse().ok();
        }
    }
    None
}

#[proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, zbus::zvariant::Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;
}

#[cfg(test)]
mod tests {
    use super::{session_counts_usage, uid_counts_usage, SessionUsageState};

    fn state(active: bool, locked: bool, idle: bool) -> SessionUsageState {
        SessionUsageState {
            active: Some(active),
            locked: Some(locked),
            idle: Some(idle),
        }
    }

    #[test]
    fn active_unlocked_non_idle_session_counts() {
        assert!(session_counts_usage(&state(true, false, false)));
    }

    #[test]
    fn active_locked_non_idle_session_does_not_count() {
        assert!(!session_counts_usage(&state(true, true, false)));
    }

    #[test]
    fn inactive_unlocked_non_idle_session_does_not_count() {
        assert!(!session_counts_usage(&state(false, false, false)));
    }

    #[test]
    fn active_unlocked_idle_session_does_not_count() {
        assert!(!session_counts_usage(&state(true, false, true)));
    }

    #[test]
    fn inactive_locked_non_idle_session_does_not_count() {
        assert!(!session_counts_usage(&state(false, true, false)));
    }

    #[test]
    fn locked_idle_session_does_not_count() {
        assert!(!session_counts_usage(&state(true, true, true)));
    }

    #[test]
    fn one_qualifying_session_makes_uid_count() {
        let states = [state(false, false, false), state(true, false, false)];
        assert!(uid_counts_usage(&states));
    }

    #[test]
    fn no_qualifying_sessions_make_uid_not_count() {
        let states = [state(true, true, false), state(false, false, false)];
        assert!(!uid_counts_usage(&states));
    }

    #[test]
    fn multiple_qualifying_sessions_still_make_one_uid_count() {
        let states = [state(true, false, false), state(true, false, false)];
        assert!(uid_counts_usage(&states));
    }

    #[test]
    fn unknown_property_fails_closed() {
        let states = [SessionUsageState {
            active: Some(true),
            locked: None,
            idle: Some(false),
        }];
        assert!(!uid_counts_usage(&states));
    }
}
