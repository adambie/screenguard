use std::collections::HashSet;
use anyhow::{Context, Result};
use futures_util::StreamExt;
use std::collections::HashMap;
use tokio::sync::mpsc;
use zbus::{Connection, proxy};

#[derive(Debug, Clone)]
pub enum SessionEvent {
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
            for (session_id, uid, _user, _seat, path) in sessions {
                if !self.is_graphical_session(&path).await {
                    continue;
                }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionTarget {
    Valid,
    StaleOwnership,
    Missing,
}

fn classify_session(
    expected_uid: u32,
    requested_sid: &str,
    live_session: Option<(&str, u32)>,
) -> SessionTarget {
    let Some((live_sid, live_uid)) = live_session else {
        return SessionTarget::Missing;
    };

    if requested_sid != live_sid {
        return SessionTarget::Missing;
    }

    if expected_uid != live_uid {
        return SessionTarget::StaleOwnership;
    }

    SessionTarget::Valid
}

/// Lock all requested sessions via DBus, but only when the live session owner
/// still matches the UID that ScreenGuard expects.
pub async fn lock_sessions(expected_uid: u32, session_ids: &[String]) -> Result<()> {
    let conn = Connection::system().await?;
    let manager = Login1ManagerProxy::new(&conn).await?;
    let sessions = manager.list_sessions().await?;

    let mut found = std::collections::HashSet::new();
    let mut locked = 0usize;

    for (sid, uid, _user, _seat, path) in &sessions {
        if !session_ids.contains(sid) {
            continue;
        }

        match classify_session(expected_uid, sid, Some((sid, *uid))) {
            SessionTarget::StaleOwnership => {
                found.insert(sid.as_str());
                tracing::warn!(
                    "refusing session={sid}: cached session ID is stale/reused \
                     (expected uid={expected_uid}, live uid={uid}); skipping lock"
                );
                continue;
            }
            SessionTarget::Missing => continue,
            SessionTarget::Valid => {}
        }

        found.insert(sid.as_str());

        if let Ok(session) = Login1SessionProxy::builder(&conn)
            .path(path.as_ref())?
            .build()
            .await
        {
            if session.lock().await.is_ok() {
                locked += 1;
            }
        }
    }

    for session_id in session_ids {
        if !found.contains(session_id.as_str()) {
            tracing::debug!(
                "Requested session={session_id} disappeared before it could be locked"
            );
        }
    }

    tracing::info!(
        "Locked {locked} session(s) for uid={expected_uid}: {:?}",
        session_ids
    );
    Ok(())
}

/// Unlock all sessions in the given list via DBus.
/// Called when enforcement is lifted (e.g. admin grants more time).
pub async fn unlock_sessions(expected_uid: u32, session_ids: &[String]) -> Result<()> {
    let conn = Connection::system().await?;
    let manager = Login1ManagerProxy::new(&conn).await?;
    let sessions = manager.list_sessions().await?;

    let mut found = HashSet::new();
    let mut succeeded = Vec::new();
    for (sid, live_uid, _user, _seat, path) in &sessions {
        if !session_ids.contains(sid) {
            continue;
        }
        match classify_session(expected_uid, sid, Some((sid, *live_uid))) {
            SessionTarget::StaleOwnership => {
                found.insert(sid.as_str());
                tracing::warn!(
                    "refusing session={sid}: cached session ID is stale/reused \
                     (expected uid={expected_uid}, live uid={live_uid}); skipping unlock"
                );
                continue;
            }
            SessionTarget::Missing => continue,
            SessionTarget::Valid => found.insert(sid.as_str()),
        };
        let Ok(builder) = Login1SessionProxy::builder(&conn).path(path.as_ref()) else {
            continue;
        };
        let Ok(session) = builder.build().await else {
            continue;
        };
        if session.unlock().await.is_ok() {
            succeeded.push(sid.clone());
        }
    }
    for session_id in session_ids.iter().filter(|sid| !found.contains(sid.as_str())) {
        tracing::debug!("Requested session={session_id} disappeared before it could be unlocked");
    }
    tracing::info!("Unlocked {} session(s): {:?}", succeeded.len(), succeeded);
    Ok(())
}

/// Terminate all sessions in the given list via DBus.
pub async fn terminate_sessions(expected_uid: u32, session_ids: &[String]) -> Result<()> {
    let conn = Connection::system().await?;
    let manager = Login1ManagerProxy::new(&conn).await?;
    let sessions = manager.list_sessions().await?;

    let mut found = HashSet::new();
    let mut succeeded = Vec::new();
    for (sid, live_uid, _user, _seat, path) in &sessions {
        if !session_ids.contains(sid) {
            continue;
        }
        match classify_session(expected_uid, sid, Some((sid, *live_uid))) {
            SessionTarget::StaleOwnership => {
                found.insert(sid.as_str());
                tracing::warn!(
                    "refusing session={sid}: cached session ID is stale/reused \
                     (expected uid={expected_uid}, live uid={live_uid}); skipping terminate"
                );
                continue;
            }
            SessionTarget::Missing => continue,
            SessionTarget::Valid => found.insert(sid.as_str()),
        };
        let Ok(builder) = Login1SessionProxy::builder(&conn).path(path.as_ref()) else {
            continue;
        };
        let Ok(session) = builder.build().await else {
            continue;
        };
        if session.terminate().await.is_ok() {
            succeeded.push(sid.clone());
        }
    }
    for session_id in session_ids.iter().filter(|sid| !found.contains(sid.as_str())) {
        tracing::debug!("Requested session={session_id} disappeared before it could be terminated");
    }
    tracing::info!("Terminated {} session(s): {:?}", succeeded.len(), succeeded);
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
use super::{
        classify_session, session_counts_usage, uid_counts_usage, SessionTarget,
        SessionUsageState,
    };

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
    fn reused_session_id_is_classified_as_stale_ownership() {
        assert_eq!(
            classify_session(1001, "c9", Some(("c9", 1002))),
            SessionTarget::StaleOwnership
        );
    }

    #[test]
    fn matching_session_id_and_uid_is_a_valid_target() {
        assert_eq!(
            classify_session(1001, "c2", Some(("c2", 1001))),
            SessionTarget::Valid
        );
    }

    #[test]
    fn absent_requested_session_is_classified_as_missing() {
        assert_eq!(classify_session(1001, "c9", None), SessionTarget::Missing);
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
