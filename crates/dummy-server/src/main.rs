use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceInfo};
use common::messages::{
    AgentHello, ConfigPush, Heartbeat, LockNow, PairingAccepted, PairingRequest, RemainingUpdate,
    UsageSync, UserListUpdate, MSG_AGENT_HELLO, MSG_CONFIG_PUSH, MSG_HEARTBEAT, MSG_LOCK_NOW,
    MSG_PAIRING_ACCEPTED, MSG_PAIRING_REQUEST, MSG_REMAINING_UPDATE, MSG_USAGE_SYNC,
    MSG_USER_LIST_UPDATE,
};
use common::models::{
    DailyLimit, EnforceAction, LocalUser, RemainingEntry, Schedule, UserConfig, UserStatus,
};
use common::protocol::WssMessage;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_tungstenite::{accept_async, tungstenite::Message};

const LISTEN_ADDR: &str = "0.0.0.0:8765";

// ── shared state ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct AgentRecord {
    hostname: String,
    users: Vec<LocalUser>,
    config_version: i64,
}

type AgentMap = Arc<Mutex<HashMap<String, AgentRecord>>>;

// Commands sent from REPL to agent connection tasks.
#[derive(Debug, Clone)]
enum Cmd {
    ConfigPush { agent_id: String },
    LockNow { agent_id: String, uid: u32 },
}

// ── entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let agents: AgentMap = Arc::new(Mutex::new(HashMap::new()));
    let listener = TcpListener::bind(LISTEN_ADDR).await?;
    println!("Dummy server listening on ws://{LISTEN_ADDR}");
    println!("Commands: status | config-push <agent_id> | lock <agent_id> <uid> | help");

    // Advertise via mDNS so agents can discover this server automatically.
    let _mdns = register_mdns(8765);
    println!("mDNS: advertising _parctrl._tcp.local. on port 8765");

    // Broadcast channel for REPL commands to all agent tasks.
    let (cmd_tx, _) = tokio::sync::broadcast::channel::<Cmd>(16);

    // REPL task.
    let agents_repl = agents.clone();
    let cmd_tx_repl = cmd_tx.clone();
    tokio::spawn(async move {
        repl(agents_repl, cmd_tx_repl).await;
    });

    // Accept loop.
    loop {
        let (stream, addr) = listener.accept().await?;
        tracing::info!("New TCP connection from {addr}");
        let agents = agents.clone();
        let cmd_rx = cmd_tx.subscribe();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, agents, cmd_rx).await {
                tracing::warn!("Connection error: {e}");
            }
        });
    }
}

// ── connection handler ────────────────────────────────────────────────────────

async fn handle_connection(
    stream: TcpStream,
    agents: AgentMap,
    mut cmd_rx: tokio::sync::broadcast::Receiver<Cmd>,
) -> Result<()> {
    let ws = accept_async(stream).await?;
    let (mut write, mut read) = ws.split();

    // Generate a fresh auth token for this session.
    let auth_token = uuid::Uuid::new_v4().to_string();

    // Phase 1: wait for either pairing_request or agent_hello (already paired).
    let (aid, mut ws_write, mut ws_read) = {
        let init_msg = loop {
            match read.next().await {
                Some(Ok(Message::Text(t))) => break t,
                Some(Ok(_)) => continue,
                _ => return Ok(()),
            }
        };

        let envelope = WssMessage::from_json(&init_msg)?;

        let resolved_id = match envelope.msg_type.as_str() {
            MSG_PAIRING_REQUEST => {
                let req: PairingRequest = envelope.parse_payload()?;
                println!(
                    "\n[PAIRING] Agent '{}' wants to pair — code: {}",
                    req.hostname, req.pairing_code
                );
                let new_id = uuid::Uuid::new_v4().to_string();
                let accepted = PairingAccepted {
                    agent_id: new_id.clone(),
                    auth_token: auth_token.clone(),
                };
                let reply = WssMessage::new(MSG_PAIRING_ACCEPTED, &accepted)?;
                write.send(Message::Text(reply.to_json()?)).await?;
                println!("[PAIRING] Accepted — agent_id={new_id}");
                agents.lock().await.insert(
                    new_id.clone(),
                    AgentRecord {
                        hostname: req.hostname,
                        users: vec![],
                        config_version: 0,
                    },
                );
                new_id
            }
            MSG_AGENT_HELLO => {
                let hello: AgentHello = envelope.parse_payload()?;
                println!(
                    "[HELLO] Agent '{}' connected (v{}, config_version={})",
                    hello.hostname, hello.agent_version, hello.last_config_version
                );
                agents.lock().await.entry(hello.machine_id.clone()).or_insert(AgentRecord {
                    hostname: hello.hostname,
                    users: vec![],
                    config_version: hello.last_config_version,
                });
                hello.machine_id
            }
            other => {
                tracing::warn!("Unexpected first message: {other}");
                return Ok(());
            }
        };

        (resolved_id, write, read)
    };

    // Phase 2: normal message loop.
    loop {
        tokio::select! {
            msg = ws_read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        handle_agent_message(&text, &aid, &agents, &mut ws_write).await?;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        ws_write.send(Message::Pong(data)).await?;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        println!("[DISCONNECT] Agent {aid}");
                        agents.lock().await.remove(&aid);
                        break;
                    }
                    Some(Err(e)) => {
                        tracing::warn!("WS error for {aid}: {e}");
                        agents.lock().await.remove(&aid);
                        break;
                    }
                    _ => {}
                }
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Ok(Cmd::ConfigPush { agent_id: target }) if target == aid => {
                        send_config_push(&aid, &agents, &mut ws_write).await?;
                    }
                    Ok(Cmd::LockNow { agent_id: target, uid }) if target == aid => {
                        let msg = WssMessage::new(MSG_LOCK_NOW, &LockNow { local_uid: uid })?;
                        ws_write.send(Message::Text(msg.to_json()?)).await?;
                        println!("[LOCK_NOW] Sent to agent {aid} for uid={uid}");
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

async fn handle_agent_message(
    text: &str,
    aid: &str,
    agents: &AgentMap,
    write: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
) -> Result<()> {
    let envelope = WssMessage::from_json(text)?;

    match envelope.msg_type.as_str() {
        MSG_AGENT_HELLO => {
            let hello: AgentHello = envelope.parse_payload()?;
            println!(
                "[HELLO] Agent '{}' re-hello (v{}, config_version={})",
                hello.hostname, hello.agent_version, hello.last_config_version
            );
            if let Some(rec) = agents.lock().await.get_mut(aid) {
                rec.config_version = hello.last_config_version;
            }
        }

        MSG_USER_LIST_UPDATE => {
            let update: UserListUpdate = envelope.parse_payload()?;
            println!("[USERS] Agent {aid} reported {} users:", update.users.len());
            for u in &update.users {
                println!("  uid={} username={} display='{}'", u.local_uid, u.username, u.display_name);
            }
            if !update.removed_uids.is_empty() {
                println!("  removed uids: {:?}", update.removed_uids);
            }
            if let Some(rec) = agents.lock().await.get_mut(aid) {
                rec.users = update.users;
            }
        }

        MSG_HEARTBEAT => {
            let hb: Heartbeat = envelope.parse_payload()?;
            println!("[HB] Agent {aid}:");
            for u in &hb.users {
                println!(
                    "  uid={} active={}s idle={} sessions={}",
                    u.local_uid, u.active_seconds_since_last, u.idle, u.session_count
                );
            }

            // Respond with remaining_update: 120 min / day, everything is allow.
            let today = chrono::Local::now().date_naive();
            let end_of_day = chrono::NaiveTime::from_hms_opt(23, 59, 0).unwrap();
            let entries: Vec<RemainingEntry> = hb
                .users
                .iter()
                .map(|u| RemainingEntry {
                    local_uid: u.local_uid,
                    remaining_minutes: 120,
                    limit_today_minutes: Some(120),
                    used_today_minutes: 0,
                    adjustments_today_minutes: 0,
                    current_window_ends_at: Some(end_of_day),
                    next_window_starts_at: None,
                    enforce: EnforceAction::Allow,
                })
                .collect();

            let reply = WssMessage::new(MSG_REMAINING_UPDATE, &RemainingUpdate { users: entries })?;
            write.send(Message::Text(reply.to_json()?)).await?;
            let _ = today; // used above
        }

        MSG_USAGE_SYNC => {
            let sync: UsageSync = envelope.parse_payload()?;
            println!("[USAGE_SYNC] Agent {aid} synced {} records:", sync.usage.len());
            for e in &sync.usage {
                println!("  uid={} date={} used={}s", e.local_uid, e.date, e.used_seconds);
            }
        }

        other => {
            tracing::debug!("[UNKNOWN] {aid}: {other}");
        }
    }

    Ok(())
}

async fn send_config_push(
    aid: &str,
    agents: &AgentMap,
    write: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
) -> Result<()> {
    let users_snapshot = {
        agents
            .lock()
            .await
            .get(aid)
            .map(|r| r.users.clone())
            .unwrap_or_default()
    };

    // Build a basic config: every known user gets 2h/day, Mon–Fri 10:00–20:00.
    let user_configs: Vec<UserConfig> = users_snapshot
        .iter()
        .map(|u| UserConfig {
            local_uid: u.local_uid,
            profile_id: uuid::Uuid::new_v4(),
            status: UserStatus::Managed,
            schedules: (0u8..5).map(|dow| Schedule {
                day_of_week: dow,
                start_time: chrono::NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                end_time: chrono::NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
            }).collect(),
            daily_limits: (0u8..7).map(|dow| DailyLimit {
                day_of_week: dow,
                allowed_minutes: 120,
            }).collect(),
            adjustments_today: 0,
            adjustment_message: None,
            lockout_grace_minutes: 5,
            warning_thresholds_minutes: vec![15, 5, 1],
        })
        .collect();

    let version = {
        let mut map = agents.lock().await;
        if let Some(rec) = map.get_mut(aid) {
            rec.config_version += 1;
            rec.config_version
        } else {
            1
        }
    };

    let push = ConfigPush { config_version: version, users: user_configs };
    let msg = WssMessage::new(MSG_CONFIG_PUSH, &push)?;
    write.send(Message::Text(msg.to_json()?)).await?;
    println!("[CONFIG_PUSH] Sent to agent {aid} (version={version}, {} users)", push.users.len());
    Ok(())
}

// ── REPL ──────────────────────────────────────────────────────────────────────

async fn repl(agents: AgentMap, cmd_tx: tokio::sync::broadcast::Sender<Cmd>) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();

    loop {
        print!("> ");
        // flush stdout so prompt appears before blocking
        use std::io::Write;
        let _ = std::io::stdout().flush();

        let line = match lines.next_line().await {
            Ok(Some(l)) => l,
            _ => break,
        };

        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["help"] | ["h"] | ["?"] => {
                println!("Commands:");
                println!("  status                      — list connected agents and their users");
                println!("  config-push <agent_id>      — send a basic 2h/day config to agent");
                println!("  lock <agent_id> <uid>       — send lock_now for uid to agent");
                println!("  help                        — this message");
            }
            ["status"] | ["s"] => {
                let map = agents.lock().await;
                if map.is_empty() {
                    println!("No agents connected.");
                } else {
                    for (id, rec) in map.iter() {
                        println!(
                            "Agent: {} (hostname='{}', config_v{})",
                            id, rec.hostname, rec.config_version
                        );
                        for u in &rec.users {
                            println!(
                                "  uid={} username={} display='{}'",
                                u.local_uid, u.username, u.display_name
                            );
                        }
                    }
                }
            }
            ["config-push", agent_id] => {
                let _ = cmd_tx.send(Cmd::ConfigPush { agent_id: agent_id.to_string() });
            }
            ["lock", agent_id, uid_str] => {
                match uid_str.parse::<u32>() {
                    Ok(uid) => {
                        let _ = cmd_tx.send(Cmd::LockNow {
                            agent_id: agent_id.to_string(),
                            uid,
                        });
                    }
                    Err(_) => println!("Invalid uid: {uid_str}"),
                }
            }
            [] | [""] => {}
            other => println!("Unknown command: {}. Type 'help'.", other.join(" ")),
        }
    }
}

// ── mDNS ──────────────────────────────────────────────────────────────────────

fn local_ip() -> std::net::IpAddr {
    // UDP trick: connect to an external address to learn which local IP the OS picks.
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").expect("bind failed");
    socket.connect("8.8.8.8:80").expect("connect failed");
    socket.local_addr().expect("local_addr failed").ip()
}

fn register_mdns(port: u16) -> ServiceDaemon {
    let mdns = ServiceDaemon::new().expect("mDNS daemon failed");
    let ip = local_ip();
    let hostname = std::fs::read_to_string("/etc/hostname")
        .unwrap_or_else(|_| "dummy-server".to_string())
        .trim()
        .to_string();
    let instance = format!("pc-dummy.{hostname}");
    let service = ServiceInfo::new(
        "_parctrl._tcp.local.",
        &instance,
        &format!("{hostname}.local."),
        ip.to_string().as_str(),
        port,
        None,
    )
    .expect("mDNS ServiceInfo failed");
    mdns.register(service).expect("mDNS register failed");
    println!("mDNS: registered on {ip}:{port}");
    mdns
}
