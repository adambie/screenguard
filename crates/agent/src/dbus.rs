use anyhow::{Context, Result};
use futures_util::StreamExt;
use std::collections::HashMap;
use tokio::sync::mpsc;
use zbus::{Connection, proxy};

#[derive(Debug, Clone)]
pub enum SessionEvent {
    SessionStarted { uid: u32, session_id: String },
    SessionEnded { uid: u32, session_id: String },
    IdleChanged { uid: u32, session_id: String, idle: bool },
    PrepareForSleep { suspend: bool },
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

        // Emit events for graphical sessions already active at startup.
        // TTY/SSH sessions are excluded — they are never idle and must not count as screen time.
        if let Ok(sessions) = manager.list_sessions().await {
            for (session_id, uid, _user, _seat, path) in sessions {
                if !self.is_graphical_session(&path).await {
                    continue;
                }
                let idle = self.get_session_idle(&path).await.unwrap_or(false);
                let _ = self.tx.send(SessionEvent::SessionStarted {
                    uid,
                    session_id: session_id.clone(),
                }).await;
                let _ = self.tx.send(SessionEvent::IdleChanged {
                    uid,
                    session_id: session_id.clone(),
                    idle,
                }).await;
                let tx = self.tx.clone();
                let conn = self.conn.clone();
                tokio::spawn(async move {
                    let _ = watch_idle(conn, path, uid, session_id, tx).await;
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
                        let idle = self.get_session_idle(&path).await.unwrap_or(false);
                        let _ = self.tx.send(SessionEvent::SessionStarted {
                            uid,
                            session_id: session_id.clone(),
                        }).await;
                        let tx = self.tx.clone();
                        let conn = self.conn.clone();
                        let sid = session_id.clone();
                        let p = path.clone();
                        tokio::spawn(async move {
                            let _ = watch_idle(conn, p, uid, sid, tx).await;
                        });
                        let _ = self.tx.send(SessionEvent::IdleChanged {
                            uid,
                            session_id,
                            idle,
                        }).await;
                    }
                }
                Some(signal) = removed_stream.next() => {
                    let args = signal.args()?;
                    let session_id = args.session_id.to_string();
                    let path = args.object_path.clone();
                    let uid = self.get_session_uid(&path).await.unwrap_or(0);
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

    async fn get_session_idle(&self, path: &zbus::zvariant::OwnedObjectPath) -> Result<bool> {
        let session = Login1SessionProxy::builder(&self.conn)
            .path(path.as_ref())?
            .build()
            .await?;
        Ok(session.idle_hint().await?)
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

async fn watch_idle(
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

    let mut stream = session.receive_idle_hint_changed().await;
    while let Some(change) = stream.next().await {
        if let Ok(idle) = change.get().await {
            let _ = tx.send(SessionEvent::IdleChanged {
                uid,
                session_id: session_id.clone(),
                idle,
            }).await;
        }
    }
    Ok(())
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
    Ok(())
}

/// Terminate all sessions in the given list via DBus.
pub async fn terminate_sessions(session_ids: &[String]) -> Result<()> {
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
                let _ = session.terminate().await;
            }
    }
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
