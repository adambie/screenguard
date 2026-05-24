use anyhow::Result;
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

    fn lock(&self) -> zbus::Result<()>;

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

        // Emit events for sessions already active at startup, and start idle watchers.
        if let Ok(sessions) = manager.list_sessions().await {
            for (session_id, uid, _user, _seat, path) in sessions {
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
                    if uid > 0 {
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

/// Send a desktop notification to a user by connecting to their session D-Bus.
/// Silently does nothing if the user has no active session bus (e.g. not logged in).
pub async fn send_desktop_notification(uid: u32, summary: &str, body: &str) -> Result<()> {
    let socket = format!("/run/user/{uid}/bus");
    if !std::path::Path::new(&socket).exists() {
        tracing::debug!("No session bus for uid={uid}, skipping notification");
        return Ok(());
    }

    let address = format!("unix:path={socket}");
    let conn = match zbus::connection::Builder::address(address.as_str())?
        .build()
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Cannot connect to session bus for uid={uid}: {e}");
            return Ok(());
        }
    };

    let proxy = NotificationsProxy::new(&conn).await?;
    let hints: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
    let _ = proxy
        .notify(
            "Parental Controller",
            0,
            "dialog-information",
            summary,
            body,
            &[],
            hints,
            5000,
        )
        .await;

    Ok(())
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
