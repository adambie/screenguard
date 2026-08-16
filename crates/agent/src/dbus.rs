use anyhow::{Context, Result};
use futures_util::StreamExt;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use tokio::sync::mpsc;
use zbus::{Connection, proxy};

const CINNAMON_LOCK_VERIFIED: i32 = 0;
const CINNAMON_UNAVAILABLE: i32 = 10;
const CINNAMON_LOCK_FAILED: i32 = 11;
const CINNAMON_VERIFICATION_FAILED: i32 = 12;
const CINNAMON_NOT_ACTIVE: i32 = 13;
const CINNAMON_HELPER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
const LOGIND_LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const DESKTOP_QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
const CINNAMON_VERIFY_ATTEMPTS: usize = 5;
const CINNAMON_VERIFY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

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

    #[zbus(property)]
    fn desktop(&self) -> zbus::Result<String>;

    fn lock(&self) -> zbus::Result<()>;

    fn unlock(&self) -> zbus::Result<()>;

    fn terminate(&self) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.cinnamon.ScreenSaver",
    default_service = "org.cinnamon.ScreenSaver",
    default_path = "/org/cinnamon/ScreenSaver"
)]
trait CinnamonScreenSaver {
    fn lock(&self, message: &str) -> zbus::Result<()>;

    fn get_active(&self) -> zbus::Result<bool>;
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum CinnamonLockOutcome {
    Verified,
    VerifiedForAnotherSession,
    NotApplicable,
    Unavailable,
    LockFailed,
    VerificationFailed,
    NotActive,
    HelperFailed(String),
}

impl CinnamonLockOutcome {
    fn fallback_reason(&self) -> &'static str {
        match self {
            Self::Verified => "Cinnamon lock was verified",
            Self::VerifiedForAnotherSession => {
                "Cinnamon lock was verified only for another session"
            }
            Self::NotApplicable => "session is not running Cinnamon",
            Self::Unavailable => "Cinnamon ScreenSaver is unavailable",
            Self::LockFailed => "Cinnamon ScreenSaver.Lock failed",
            Self::VerificationFailed => "Cinnamon ScreenSaver.GetActive failed",
            Self::NotActive => "Cinnamon ScreenSaver.GetActive returned false",
            Self::HelperFailed(_) => "Cinnamon lock helper failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LockBackend {
    Cinnamon,
    Logind,
}

async fn lock_with_fallback<C, CFut, L, LFut>(
    uid: u32,
    session_id: &str,
    cinnamon_lock: C,
    logind_lock: L,
) -> Result<LockBackend>
where
    C: FnOnce() -> CFut,
    CFut: Future<Output = CinnamonLockOutcome>,
    L: FnOnce() -> LFut,
    LFut: Future<Output = Result<()>>,
{
    let cinnamon_outcome = cinnamon_lock().await;
    if cinnamon_outcome == CinnamonLockOutcome::Verified {
        tracing::info!(
            "Cinnamon lock verified for uid={uid} session={session_id} via GetActive"
        );
        return Ok(LockBackend::Cinnamon);
    }

    match &cinnamon_outcome {
        CinnamonLockOutcome::VerifiedForAnotherSession
        | CinnamonLockOutcome::NotApplicable
        | CinnamonLockOutcome::Unavailable => tracing::debug!(
                "{} for uid={uid} session={session_id}; using logind",
                cinnamon_outcome.fallback_reason()
            ),
        CinnamonLockOutcome::HelperFailed(error) => tracing::warn!(
            "{} for uid={uid} session={session_id}: {error}; falling back to logind",
            cinnamon_outcome.fallback_reason()
        ),
        _ => tracing::warn!(
            "{} for uid={uid} session={session_id}; falling back to logind",
            cinnamon_outcome.fallback_reason()
        ),
    }

    match tokio::time::timeout(LOGIND_LOCK_TIMEOUT, logind_lock()).await {
        Ok(result) => result.with_context(|| {
            format!("logind Session.Lock() failed for uid={uid} session={session_id}")
        })?,
        Err(_) => anyhow::bail!(
            "logind Session.Lock() timed out after {}s for uid={uid} session={session_id}",
            LOGIND_LOCK_TIMEOUT.as_secs()
        ),
    }
    tracing::info!(
        "logind Session.Lock() fallback request completed for uid={uid} session={session_id}"
    );
    Ok(LockBackend::Logind)
}

async fn cinnamon_outcome_for_session<C, CFut>(
    uid: u32,
    cached_outcomes: &mut HashMap<u32, CinnamonLockOutcome>,
    cinnamon_lock: C,
) -> CinnamonLockOutcome
where
    C: FnOnce() -> CFut,
    CFut: Future<Output = CinnamonLockOutcome>,
{
    if let Some(outcome) = cached_outcomes.get(&uid) {
        return outcome.clone();
    }

    let outcome = cinnamon_lock().await;
    let cached_outcome = if outcome == CinnamonLockOutcome::Verified {
        CinnamonLockOutcome::VerifiedForAnotherSession
    } else {
        outcome.clone()
    };
    cached_outcomes.insert(uid, cached_outcome);
    outcome
}

async fn cinnamon_lock_for_uid(uid: u32) -> CinnamonLockOutcome {
    let socket = format!("/run/user/{uid}/bus");
    if !std::path::Path::new(&socket).exists() {
        return CinnamonLockOutcome::Unavailable;
    }

    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => return CinnamonLockOutcome::HelperFailed(error.to_string()),
    };
    let gid = lookup_gid_for_uid(uid).unwrap_or(uid);

    let mut command = tokio::process::Command::new(exe);
    command
        .arg("--lock-cinnamon")
        .env_clear()
        .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path=/run/user/{uid}/bus"))
        .env("XDG_RUNTIME_DIR", format!("/run/user/{uid}"))
        .uid(uid)
        .gid(gid)
        .kill_on_drop(true);

    let status = match tokio::time::timeout(CINNAMON_HELPER_TIMEOUT, command.status()).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => return CinnamonLockOutcome::HelperFailed(error.to_string()),
        Err(_) => {
            return CinnamonLockOutcome::HelperFailed(format!(
                "timed out after {}s",
                CINNAMON_HELPER_TIMEOUT.as_secs()
            ));
        }
    };

    match status.code() {
        Some(CINNAMON_LOCK_VERIFIED) => CinnamonLockOutcome::Verified,
        Some(CINNAMON_UNAVAILABLE) => CinnamonLockOutcome::Unavailable,
        Some(CINNAMON_LOCK_FAILED) => CinnamonLockOutcome::LockFailed,
        Some(CINNAMON_VERIFICATION_FAILED) => CinnamonLockOutcome::VerificationFailed,
        Some(CINNAMON_NOT_ACTIVE) => CinnamonLockOutcome::NotActive,
        Some(code) => CinnamonLockOutcome::HelperFailed(format!("unexpected exit status {code}")),
        None => CinnamonLockOutcome::HelperFailed("terminated by signal".to_string()),
    }
}

/// Called when the agent binary is invoked with `--lock-cinnamon` as the target user.
/// Returns an exit code that lets the root agent select or skip the logind fallback.
pub async fn lock_cinnamon_as_current_user() -> i32 {
    let conn = match Connection::session().await {
        Ok(conn) => conn,
        Err(error) => {
            tracing::debug!("Cannot connect to the user session bus for Cinnamon locking: {error}");
            return CINNAMON_UNAVAILABLE;
        }
    };

    let proxy = match CinnamonScreenSaverProxy::new(&conn).await {
        Ok(proxy) => proxy,
        Err(error) => {
            tracing::debug!("Cannot create Cinnamon ScreenSaver proxy: {error}");
            return CINNAMON_UNAVAILABLE;
        }
    };
    if let Err(error) = proxy.lock("ScreenGuard").await {
        if cinnamon_service_unavailable(&error) {
            tracing::debug!("Cinnamon ScreenSaver is unavailable: {error}");
            return CINNAMON_UNAVAILABLE;
        }
        tracing::warn!("Cinnamon ScreenSaver.Lock failed: {error}");
        return CINNAMON_LOCK_FAILED;
    }

    let mut last_error = None;
    for attempt in 0..CINNAMON_VERIFY_ATTEMPTS {
        match proxy.get_active().await {
            Ok(true) => return CINNAMON_LOCK_VERIFIED,
            Ok(false) => last_error = None,
            Err(error) => last_error = Some(error.to_string()),
        }
        if attempt + 1 < CINNAMON_VERIFY_ATTEMPTS {
            tokio::time::sleep(CINNAMON_VERIFY_INTERVAL).await;
        }
    }

    if let Some(error) = last_error {
        tracing::warn!(
            "Cinnamon ScreenSaver.GetActive failed after {CINNAMON_VERIFY_ATTEMPTS} attempts: {error}"
        );
        CINNAMON_VERIFICATION_FAILED
    } else {
        CINNAMON_NOT_ACTIVE
    }
}

fn cinnamon_service_unavailable(error: &zbus::Error) -> bool {
    match error {
        zbus::Error::MethodError(name, _, _) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
        ),
        zbus::Error::FDO(error) => matches!(
            error.as_ref(),
            zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_)
        ),
        _ => false,
    }
}

fn cinnamon_backend_applies(desktop: Option<&str>) -> bool {
    let Some(desktop) = desktop.map(str::trim).filter(|desktop| !desktop.is_empty()) else {
        return true;
    };

    desktop.split([':', ';']).any(|name| {
        let name = name.trim();
        name.eq_ignore_ascii_case("cinnamon")
            || name.eq_ignore_ascii_case("x-cinnamon")
            || name.eq_ignore_ascii_case("cinnamon2d")
            || name.eq_ignore_ascii_case("x-cinnamon2d")
    })
}

/// Lock all requested sessions, preferring a verified desktop-native backend.
pub async fn lock_sessions(session_ids: &[String]) -> Result<()> {
    let conn = Connection::system().await?;
    let manager = Login1ManagerProxy::new(&conn).await?;
    let sessions = manager.list_sessions().await?;
    let requested: HashSet<&str> = session_ids.iter().map(String::as_str).collect();
    let mut matched = HashSet::new();
    let mut errors = Vec::new();
    let mut cinnamon_outcomes: HashMap<u32, CinnamonLockOutcome> = HashMap::new();

    for (sid, uid, _user, _seat, path) in &sessions {
        if !requested.contains(sid.as_str()) {
            continue;
        }
        matched.insert(sid.as_str());

        let session = match Login1SessionProxy::builder(&conn)
            .path(path.as_ref())
        {
            Ok(builder) => match builder.build().await {
                Ok(session) => session,
                Err(error) => {
                    let error = anyhow::Error::from(error).context(format!(
                        "cannot create logind session proxy for uid={uid} session={sid}"
                    ));
                    tracing::error!("{error:#}");
                    errors.push(format!("{error:#}"));
                    continue;
                }
            },
            Err(error) => {
                let error = anyhow::Error::from(error).context(format!(
                    "invalid logind object path for uid={uid} session={sid}"
                ));
                tracing::error!("{error:#}");
                errors.push(format!("{error:#}"));
                continue;
            }
        };

        let desktop = match tokio::time::timeout(DESKTOP_QUERY_TIMEOUT, session.desktop()).await {
            Ok(Ok(desktop)) => Some(desktop),
            Ok(Err(error)) => {
                tracing::debug!(
                    "Cannot identify desktop for uid={uid} session={sid}: {error}; trying Cinnamon"
                );
                None
            }
            Err(_) => {
                tracing::debug!(
                    "Desktop query timed out for uid={uid} session={sid}; trying Cinnamon"
                );
                None
            }
        };
        let cinnamon_outcome = if !cinnamon_backend_applies(desktop.as_deref()) {
            CinnamonLockOutcome::NotApplicable
        } else {
            cinnamon_outcome_for_session(*uid, &mut cinnamon_outcomes, || {
                cinnamon_lock_for_uid(*uid)
            })
            .await
        };

        let result = lock_with_fallback(
            *uid,
            sid,
            || async { cinnamon_outcome },
            || async move {
                session.lock().await?;
                Ok(())
            },
        )
        .await;
        if let Err(error) = result {
            tracing::error!("{error:#}");
            errors.push(format!("{error:#}"));
        }
    }

    for session_id in requested.difference(&matched) {
        tracing::debug!(
            "Requested session={session_id} disappeared before it could be locked"
        );
    }

    if errors.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "failed to lock {} session(s): {}",
            errors.len(),
            errors.join("; ")
        )
    }
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
    use super::{
        cinnamon_backend_applies, cinnamon_outcome_for_session, lock_with_fallback,
        session_counts_usage, uid_counts_usage, CinnamonLockOutcome, LockBackend,
        SessionUsageState,
    };
    use std::collections::HashMap;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
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

    #[test]
    fn cinnamon_backend_applies_only_to_cinnamon_or_unknown_desktops() {
        assert!(cinnamon_backend_applies(None));
        assert!(cinnamon_backend_applies(Some("")));
        assert!(cinnamon_backend_applies(Some("cinnamon")));
        assert!(cinnamon_backend_applies(Some("X-Cinnamon")));
        assert!(cinnamon_backend_applies(Some("GNOME:Cinnamon")));
        assert!(cinnamon_backend_applies(Some("cinnamon2d")));
        assert!(!cinnamon_backend_applies(Some("GNOME")));
        assert!(!cinnamon_backend_applies(Some("KDE")));
    }

    async fn assert_logind_fallback(outcome: CinnamonLockOutcome) {
        let called = Arc::new(AtomicBool::new(false));
        let called_by_fallback = called.clone();
        let backend = lock_with_fallback(
            1000,
            "c1",
            || async { outcome },
            || async move {
                called_by_fallback.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
        .unwrap();

        assert_eq!(backend, LockBackend::Logind);
        assert!(called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn cinnamon_verified_success_skips_logind() {
        let called = Arc::new(AtomicBool::new(false));
        let called_by_fallback = called.clone();
        let backend = lock_with_fallback(
            1000,
            "c1",
            || async { CinnamonLockOutcome::Verified },
            || async move {
                called_by_fallback.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
        .unwrap();

        assert_eq!(backend, LockBackend::Cinnamon);
        assert!(!called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn same_uid_sessions_do_not_share_cinnamon_verification() {
        let helper_calls = Arc::new(AtomicUsize::new(0));
        let logind_calls = Arc::new(AtomicUsize::new(0));
        let mut cached_outcomes = HashMap::new();

        let first_helper_calls = helper_calls.clone();
        let first_outcome = cinnamon_outcome_for_session(
            1000,
            &mut cached_outcomes,
            || async move {
                first_helper_calls.fetch_add(1, Ordering::SeqCst);
                CinnamonLockOutcome::Verified
            },
        )
        .await;
        let first_logind_calls = logind_calls.clone();
        let first_backend = lock_with_fallback(
            1000,
            "c1",
            || async { first_outcome },
            || async move {
                first_logind_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
        .unwrap();

        let second_helper_calls = helper_calls.clone();
        let second_outcome = cinnamon_outcome_for_session(
            1000,
            &mut cached_outcomes,
            || async move {
                second_helper_calls.fetch_add(1, Ordering::SeqCst);
                CinnamonLockOutcome::Verified
            },
        )
        .await;
        let second_logind_calls = logind_calls.clone();
        let second_backend = lock_with_fallback(
            1000,
            "c2",
            || async { second_outcome },
            || async move {
                second_logind_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
        .unwrap();

        assert_eq!(first_backend, LockBackend::Cinnamon);
        assert_eq!(second_backend, LockBackend::Logind);
        assert_eq!(helper_calls.load(Ordering::SeqCst), 1);
        assert_eq!(logind_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cinnamon_unavailable_uses_logind() {
        assert_logind_fallback(CinnamonLockOutcome::Unavailable).await;
    }

    #[tokio::test]
    async fn cinnamon_lock_error_uses_logind() {
        assert_logind_fallback(CinnamonLockOutcome::LockFailed).await;
    }

    #[tokio::test]
    async fn cinnamon_inactive_after_lock_uses_logind() {
        assert_logind_fallback(CinnamonLockOutcome::NotActive).await;
    }

    #[tokio::test]
    async fn cinnamon_verification_error_uses_logind() {
        assert_logind_fallback(CinnamonLockOutcome::VerificationFailed).await;
    }

    #[tokio::test]
    async fn logind_error_is_propagated() {
        let error = lock_with_fallback(
            1000,
            "c1",
            || async { CinnamonLockOutcome::Unavailable },
            || async { anyhow::bail!("permission denied") },
        )
        .await
        .unwrap_err();

        let message = format!("{error:#}");
        assert!(message.contains("logind Session.Lock() failed for uid=1000 session=c1"));
        assert!(message.contains("permission denied"));
    }
}
