use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate};
use common::messages::{
    AgentHello, Heartbeat, HeartbeatUser, ServerMessage, UsageSync, UserListUpdate,
    MSG_AGENT_HELLO, MSG_HEARTBEAT, MSG_USAGE_SYNC, MSG_USER_LIST_UPDATE,
};
use common::models::{EnforceAction, LocalUser, UsageEntry};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex};

use crate::db::{AgentMode, Db};
use crate::dbus::SessionEvent;
use crate::enforcement::{evaluate_enforcement, execute_lock, handle_midnight};
use crate::users::{diff_users, scan_local_users, users_to_map};
use crate::ws_client::{self, ConnectionEvent};

pub struct HeartbeatLoop {
    db: Arc<Mutex<Db>>,
    outbound_tx: mpsc::Sender<common::protocol::WssMessage>,
    inbound_rx: mpsc::Receiver<ServerMessage>,
    connection_rx: mpsc::Receiver<ConnectionEvent>,
    session_rx: mpsc::Receiver<SessionEvent>,
    heartbeat_interval: Duration,
    user_scan_interval: Duration,
    min_uid: u32,
    agent_version: String,
    cache_ttl_hours: u64,
    /// Tracks which warning thresholds have already fired per uid today.
    /// Cleared at midnight and when remaining goes back above a threshold.
    notified_thresholds: HashMap<u32, HashSet<i32>>,
}

impl HeartbeatLoop {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Arc<Mutex<Db>>,
        outbound_tx: mpsc::Sender<common::protocol::WssMessage>,
        inbound_rx: mpsc::Receiver<ServerMessage>,
        connection_rx: mpsc::Receiver<ConnectionEvent>,
        session_rx: mpsc::Receiver<SessionEvent>,
        heartbeat_interval_secs: u64,
        user_scan_interval_secs: u64,
        min_uid: u32,
        cache_ttl_hours: u64,
    ) -> Self {
        Self {
            db,
            outbound_tx,
            inbound_rx,
            connection_rx,
            session_rx,
            heartbeat_interval: Duration::from_secs(heartbeat_interval_secs),
            user_scan_interval: Duration::from_secs(user_scan_interval_secs),
            min_uid,
            agent_version: env!("CARGO_PKG_VERSION").to_string(),
            cache_ttl_hours,
            notified_thresholds: HashMap::new(),
        }
    }

    pub async fn run(mut self) -> Result<()> {
        let mut heartbeat_ticker = tokio::time::interval(self.heartbeat_interval);
        let mut user_scan_ticker = tokio::time::interval(self.user_scan_interval);
        let mut last_date: NaiveDate = Local::now().date_naive();

        let mut active_since: HashMap<u32, Option<Instant>> = HashMap::new();
        let mut known_users: HashMap<u32, LocalUser> = {
            let users = scan_local_users(self.min_uid).unwrap_or_default();
            users_to_map(&users)
        };

        loop {
            tokio::select! {
                Some(event) = self.connection_rx.recv() => {
                    self.handle_connection_event(event).await?;
                }

                Some(event) = self.session_rx.recv() => {
                    self.handle_session_event(event, &mut active_since).await;
                }

                Some(msg) = self.inbound_rx.recv() => {
                    self.handle_server_message(msg).await?;
                }

                _ = heartbeat_ticker.tick() => {
                    let now = Local::now();
                    let today = now.date_naive();

                    if today != last_date {
                        handle_midnight(&self.db).await?;
                        last_date = today;
                        self.notified_thresholds.clear();
                    }

                    let online = {
                        let db = self.db.lock().await;
                        db.get_agent_mode()? == AgentMode::Online
                    };

                    self.check_cache_ttl_warning().await;
                    self.send_heartbeat(&mut active_since, online, &today.to_string()).await?;
                }

                _ = user_scan_ticker.tick() => {
                    self.maybe_rescan_users(&mut known_users).await?;
                }
            }
        }
    }

    async fn handle_connection_event(&self, event: ConnectionEvent) -> Result<()> {
        match event {
            ConnectionEvent::Connected => {
                tracing::info!("Connection established — running reconnect sequence");
                {
                    let db = self.db.lock().await;
                    db.set_agent_mode(AgentMode::Online)?;
                }
                self.send_agent_hello().await?;
                self.send_usage_sync().await?;
                self.send_user_list_update().await?;
            }
            ConnectionEvent::Disconnected => {
                tracing::warn!("Connection lost — switching to offline mode");
                let db = self.db.lock().await;
                db.set_agent_mode(AgentMode::Offline)?;
            }
        }
        Ok(())
    }

    async fn handle_session_event(
        &self,
        event: SessionEvent,
        active_since: &mut HashMap<u32, Option<Instant>>,
    ) {
        match event {
            SessionEvent::SessionStarted { uid, session_id } => {
                let db = self.db.lock().await;
                let _ = db.upsert_session(uid, &session_id, false);
                active_since.entry(uid).or_insert(Some(Instant::now()));
            }
            SessionEvent::SessionEnded { uid, session_id } => {
                let db = self.db.lock().await;
                let _ = db.remove_session(uid, &session_id);
                if db.count_active_sessions(uid).unwrap_or(0) == 0 {
                    active_since.remove(&uid);
                }
            }
            SessionEvent::IdleChanged { uid, session_id, idle } => {
                let db = self.db.lock().await;
                let _ = db.upsert_session(uid, &session_id, idle);
                if idle {
                    active_since.insert(uid, None);
                } else {
                    active_since.entry(uid).or_insert(Some(Instant::now()));
                }
            }
            SessionEvent::PrepareForSleep { suspend: true } => {
                for val in active_since.values_mut() {
                    *val = None;
                }
            }
            SessionEvent::PrepareForSleep { suspend: false } => {
                for val in active_since.values_mut() {
                    if val.is_none() {
                        *val = Some(Instant::now());
                    }
                }
            }
        }
    }

    async fn handle_server_message(&mut self, msg: ServerMessage) -> Result<()> {
        match msg {
            ServerMessage::NotifyUser(n) => {
                tracing::info!("Received notify_user for uid={}: {}", n.local_uid, n.summary);
                let uid = n.local_uid;
                let summary = n.summary.clone();
                let body = n.body.clone();
                tokio::spawn(async move {
                    if let Err(e) = crate::dbus::send_desktop_notification(uid, &summary, &body).await {
                        tracing::warn!("Desktop notification failed for uid={uid}: {e}");
                    }
                });
            }
            ServerMessage::ConfigPush(push) => {
                tracing::info!("Received config_push v{}", push.config_version);
                let today = chrono::Local::now().date_naive().to_string();

                // Snapshot state before applying so we can detect what changed.
                let snapshots: Vec<(u32, Vec<_>, i32)> = {
                    let db = self.db.lock().await;
                    push.users.iter().map(|u| {
                        let schedules = db.get_cached_schedules(u.local_uid).unwrap_or_default();
                        let adj = db.get_cached_adjustment(u.local_uid, &today).unwrap_or(0);
                        (u.local_uid, schedules, adj)
                    }).collect()
                };

                {
                    let db = self.db.lock().await;
                    db.apply_config_push(&push.users)?;
                    db.save_config_version(push.config_version)?;
                }

                // Notify users about what changed.
                for (u, (uid, old_schedules, old_adj)) in push.users.iter().zip(snapshots) {
                    let new_adj = u.adjustments_today;

                    // Normalise schedules to a comparable form.
                    let mut old_sig: Vec<_> = old_schedules.iter()
                        .map(|s| (s.day_of_week, s.start_time.clone(), s.end_time.clone()))
                        .collect();
                    let mut new_sig: Vec<_> = u.schedules.iter()
                        .map(|s| (s.day_of_week,
                                  s.start_time.format("%H:%M").to_string(),
                                  s.end_time.format("%H:%M").to_string()))
                        .collect();
                    old_sig.sort_unstable();
                    new_sig.sort_unstable();

                    let schedule_changed = old_sig != new_sig;
                    let adj_delta = new_adj - old_adj;

                    if schedule_changed {
                        tokio::spawn(async move {
                            let _ = crate::dbus::send_desktop_notification(
                                uid,
                                "Schedule updated",
                                "Your allowed screen time schedule has been changed.",
                            ).await;
                        });
                    }

                    if adj_delta != 0 {
                        // Calculate remaining after adjustment.
                        let dow = chrono::Local::now().date_naive()
                            .weekday()
                            .num_days_from_monday() as u8;
                        let limit = u.daily_limits.iter()
                            .find(|l| l.day_of_week == dow)
                            .map(|l| l.allowed_minutes as i32)
                            .unwrap_or(1440);
                        let used_min = {
                            let db = self.db.lock().await;
                            (db.get_usage_seconds(uid, &today).unwrap_or(0) / 60) as i32
                        };
                        let remaining = (limit + new_adj - used_min).max(0);
                        let reason = u.adjustment_message.clone();

                        if adj_delta > 0 {
                            tokio::spawn(async move {
                                let mut body = format!(
                                    "+{adj_delta} minutes granted. {remaining} minutes remaining today."
                                );
                                if let Some(r) = reason {
                                    body = format!("{body}\n\"{r}\"");
                                }
                                let _ = crate::dbus::send_desktop_notification(
                                    uid, "Screen time added", &body,
                                ).await;
                            });
                        } else {
                            let removed = -adj_delta;
                            tokio::spawn(async move {
                                let mut body = format!(
                                    "-{removed} minutes taken. {remaining} minutes remaining today."
                                );
                                if let Some(r) = reason {
                                    body = format!("{body}\n\"{r}\"");
                                }
                                let _ = crate::dbus::send_desktop_notification(
                                    uid, "Screen time reduced", &body,
                                ).await;
                            });
                        }
                    }
                }
            }
            ServerMessage::RemainingUpdate(update) => {
                let db = self.db.lock().await;
                for entry in &update.users {
                    let enforce_str = match entry.enforce {
                        EnforceAction::Allow => "allow",
                        EnforceAction::Warn => "warn",
                        EnforceAction::Lock => "lock",
                    };
                    db.upsert_server_remaining(
                        entry.local_uid,
                        entry.remaining_minutes,
                        enforce_str,
                    )?;
                }
                drop(db);

                for entry in &update.users {
                    if entry.enforce == EnforceAction::Lock {
                        let db = self.db.clone();
                        let uid = entry.local_uid;
                        tokio::spawn(async move {
                            if let Err(e) = execute_lock(uid, &db).await {
                                tracing::error!("Lock failed for uid={uid}: {e}");
                            }
                        });
                    } else if entry.enforce == EnforceAction::Warn {
                        let uid = entry.local_uid;
                        let remaining = entry.remaining_minutes;
                        tracing::warn!("uid={uid} has {remaining} minutes remaining");
                        self.fire_threshold_notifications(uid, remaining).await;
                    } else {
                        // Allow — clear notified set so thresholds re-arm if time is added later.
                        self.notified_thresholds.remove(&entry.local_uid);
                    }
                }
            }
            ServerMessage::LockNow(lock) => {
                tracing::info!("Received lock_now for uid={}", lock.local_uid);
                let db = self.db.clone();
                let uid = lock.local_uid;
                tokio::spawn(async move {
                    if let Err(e) = execute_lock(uid, &db).await {
                        tracing::error!("lock_now failed for uid={uid}: {e}");
                    }
                });
            }
            ServerMessage::ConfigReload => {
                tracing::info!("Received config_reload — re-sending agent_hello");
                self.send_agent_hello().await?;
            }
            ServerMessage::Unknown(t) => {
                tracing::debug!("Unknown message type from server: {t}");
            }
            ServerMessage::PairingAccepted(_) => {}
        }
        Ok(())
    }

    async fn fire_threshold_notifications(&mut self, uid: u32, remaining: i32) {
        let thresholds = {
            let db = self.db.lock().await;
            db.get_cached_enforcement(uid)
                .map(|e| e.warning_thresholds)
                .unwrap_or_default()
        };

        let notified = self.notified_thresholds.entry(uid).or_default();

        // Fire for each threshold we've crossed but haven't notified yet.
        let mut to_notify: Vec<i32> = thresholds
            .iter()
            .map(|&t| t as i32)
            .filter(|&t| remaining <= t && !notified.contains(&t))
            .collect();
        to_notify.sort_unstable_by(|a, b| b.cmp(a)); // highest first

        for t in to_notify {
            notified.insert(t);
            let body = format!("{remaining} minutes of screen time remaining today.");
            tokio::spawn(async move {
                if let Err(e) = crate::dbus::send_desktop_notification(
                    uid, "Screen time warning", &body,
                ).await {
                    tracing::warn!("Warn notification failed for uid={uid}: {e}");
                }
            });
        }

        // If remaining went back up past a threshold, un-arm it so it can fire again.
        notified.retain(|&t| remaining <= t);
    }

    async fn send_heartbeat(
        &self,
        active_since: &mut HashMap<u32, Option<Instant>>,
        online: bool,
        today: &str,
    ) -> Result<()> {
        let managed_uids = {
            let db = self.db.lock().await;
            db.get_managed_uids()?
        };

        let interval_secs = self.heartbeat_interval.as_secs();
        let mut hb_users = Vec::new();

        for uid in &managed_uids {
            let uid = *uid;
            let session_count = {
                let db = self.db.lock().await;
                db.count_active_sessions(uid)?
            };

            if session_count == 0 {
                continue;
            }

            let (active_secs, idle) = match active_since.get(&uid) {
                Some(Some(since)) => (since.elapsed().as_secs().min(interval_secs) as u32, false),
                Some(None) | None => (0, true),
            };

            if active_secs > 0 {
                let db = self.db.lock().await;
                db.add_usage_seconds(uid, today, active_secs as u64)?;
            }

            if let Some(ts) = active_since.get_mut(&uid)
                && ts.is_some()
            {
                *ts = Some(Instant::now());
            }

            hb_users.push(HeartbeatUser {
                local_uid: uid,
                active_seconds_since_last: active_secs,
                idle,
                session_count,
            });
        }

        if hb_users.is_empty() {
            return Ok(());
        }

        if online {
            ws_client::send(&self.outbound_tx, MSG_HEARTBEAT, &Heartbeat { users: hb_users })
                .await?;
        } else {
            for hb_user in &hb_users {
                let uid = hb_user.local_uid;
                let action = evaluate_enforcement(uid, &self.db, false).await?;
                match action {
                    EnforceAction::Lock => {
                        let db = self.db.clone();
                        tokio::spawn(async move {
                            let _ = execute_lock(uid, &db).await;
                        });
                    }
                    EnforceAction::Warn => {
                        tracing::warn!("uid={uid} is approaching their limit (offline mode)");
                    }
                    EnforceAction::Allow => {}
                }
            }
        }

        Ok(())
    }

    async fn send_agent_hello(&self) -> Result<()> {
        let config_version = {
            let db = self.db.lock().await;
            db.get_config_version()?
        };

        ws_client::send(
            &self.outbound_tx,
            MSG_AGENT_HELLO,
            &AgentHello {
                machine_id: read_machine_id(),
                hostname: gethostname(),
                timezone: local_timezone(),
                agent_version: self.agent_version.clone(),
                last_config_version: config_version,
            },
        )
        .await
    }

    async fn send_user_list_update(&self) -> Result<()> {
        let users = scan_local_users(self.min_uid).unwrap_or_default();
        ws_client::send(
            &self.outbound_tx,
            MSG_USER_LIST_UPDATE,
            &UserListUpdate { users, removed_uids: vec![] },
        )
        .await
    }

    pub async fn send_usage_sync(&self) -> Result<()> {
        let unsynced = {
            let db = self.db.lock().await;
            db.get_unsynced_usage()?
        };

        if unsynced.is_empty() {
            return Ok(());
        }

        let usage: Vec<UsageEntry> = unsynced
            .iter()
            .filter_map(|(uid, date, secs)| {
                date.parse::<NaiveDate>().ok().map(|d| UsageEntry {
                    local_uid: *uid,
                    date: d,
                    used_seconds: *secs,
                })
            })
            .collect();

        ws_client::send(&self.outbound_tx, MSG_USAGE_SYNC, &UsageSync { usage }).await?;

        let db = self.db.lock().await;
        for (uid, date, _) in &unsynced {
            db.mark_usage_synced(*uid, date)?;
        }
        db.update_last_sync()?;
        Ok(())
    }

    async fn maybe_rescan_users(&self, known_users: &mut HashMap<u32, LocalUser>) -> Result<()> {
        let current = scan_local_users(self.min_uid).unwrap_or_default();
        let (_added, removed_uids) = diff_users(known_users, &current);

        if _added.is_empty() && removed_uids.is_empty() {
            return Ok(());
        }

        ws_client::send(
            &self.outbound_tx,
            MSG_USER_LIST_UPDATE,
            &UserListUpdate { users: current.clone(), removed_uids },
        )
        .await?;
        *known_users = users_to_map(&current);
        Ok(())
    }

    async fn check_cache_ttl_warning(&self) {
        let offline_since = {
            let db = self.db.lock().await;
            db.get_offline_since().unwrap_or(None)
        };
        if let Some(since_ts) = offline_since {
            let offline_hours =
                (chrono::Utc::now().timestamp() - since_ts) as u64 / 3600;
            if offline_hours >= self.cache_ttl_hours {
                tracing::warn!(
                    "Agent has been offline for {offline_hours}h (TTL is {}h). \
                     Continuing to enforce cached rules.",
                    self.cache_ttl_hours
                );
            }
        }
    }
}

fn gethostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

fn read_machine_id() -> String {
    std::fs::read_to_string("/etc/machine-id")
        .or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string())
}

fn local_timezone() -> String {
    std::fs::read_to_string("/etc/timezone")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "UTC".to_string())
}
