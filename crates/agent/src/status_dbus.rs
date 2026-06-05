use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use zbus::{interface, Connection};

struct PerUid {
    remaining_seconds: i64,
    enforce: String,
    updated_at: u64,
    language: String,
}

struct Shared {
    per_uid: HashMap<u32, PerUid>,
    server_url: String,
}

struct AgentIface {
    shared: Arc<Mutex<Shared>>,
}

#[interface(name = "org.screenguard.Agent1")]
impl AgentIface {
    /// Returns (remaining_seconds, enforce, updated_at, server_url, language) for the given UID.
    async fn status(&self, uid: u32) -> (i64, String, u64, String, String) {
        let s = self.shared.lock().await;
        let server_url = s.server_url.clone();
        match s.per_uid.get(&uid) {
            Some(u) => (u.remaining_seconds, u.enforce.clone(), u.updated_at, server_url, u.language.clone()),
            None => (0i64, "allow".into(), 0u64, server_url, "en".into()),
        }
    }
}

pub struct Handle {
    shared: Arc<Mutex<Shared>>,
    _conn: Connection,
}

impl Handle {
    pub async fn update_uid(&self, uid: u32, remaining_seconds: i64, enforce: &str, language: &str) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut s = self.shared.lock().await;
        s.per_uid.insert(uid, PerUid {
            remaining_seconds,
            enforce: enforce.to_string(),
            updated_at: now,
            language: language.to_string(),
        });
    }
}

pub async fn start(server_url: String) -> Result<Handle> {
    let shared = Arc::new(Mutex::new(Shared {
        per_uid: HashMap::new(),
        server_url,
    }));

    let conn = Connection::system().await?;
    conn.request_name("org.screenguard.Agent").await?;
    conn.object_server()
        .at("/org/screenguard/Agent", AgentIface { shared: shared.clone() })
        .await?;

    Ok(Handle { shared, _conn: conn })
}
