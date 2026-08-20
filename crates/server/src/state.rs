use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, RwLock};
use uuid::Uuid;

use crate::db::DbPool;
use crate::release_check::LatestRelease;
use common::protocol::WssMessage;

/// Logical tenant identifier. Homelab always uses DEFAULT_TENANT.
/// Cloud assigns one per registered family/account.
pub type TenantId = String;
pub const DEFAULT_TENANT: &str = "default";

/// Composite key used in all per-connection maps so tenant agents never collide.
type TenantKey<K> = (TenantId, K);

/// Handle to a connected agent's outbound message channel.
#[derive(Debug)]
pub struct AgentHandle {
    pub agent_id: Uuid,
    pub outbound_tx: mpsc::Sender<WssMessage>,
}

/// Per-pending-agent channel: REST /accept sends pairing_accepted down this.
pub struct PairingHandle {
    #[allow(dead_code)]
    pub pairing_code: String,
    pub tx: oneshot::Sender<PairingDecision>,
}

pub struct PairingDecision {
    pub auth_token: String,
    pub agent_db_id: Uuid,
}

pub struct AppState {
    pub db: DbPool,
    pub jwt_secret: String,
    pub jwt_expiry_hours: u64,
    /// (tenant_id, machine_id) → AgentHandle for currently connected agents.
    pub online: Arc<RwLock<HashMap<TenantKey<String>, AgentHandle>>>,
    /// (tenant_id, machine_id) → PairingHandle for agents waiting for admin accept.
    pub pending: Arc<RwLock<HashMap<TenantKey<String>, PairingHandle>>>,
    /// (tenant_id, agent_id) → oneshot sender for pending log requests.
    pub log_requests: Arc<RwLock<HashMap<TenantKey<Uuid>, oneshot::Sender<Vec<String>>>>>,
    /// Latest Rust release version fetched from GitHub (e.g. "0.9.8").
    pub latest_agent_release: LatestRelease,
}

impl AppState {
    pub fn new(db: DbPool, jwt_secret: String, jwt_expiry_hours: u64) -> Arc<Self> {
        Arc::new(Self {
            db,
            jwt_secret,
            jwt_expiry_hours,
            online: Arc::new(RwLock::new(HashMap::new())),
            pending: Arc::new(RwLock::new(HashMap::new())),
            log_requests: Arc::new(RwLock::new(HashMap::new())),
            latest_agent_release: Arc::new(RwLock::new(None)),
        })
    }

    pub async fn add_online(&self, tenant_id: &str, machine_id: String, handle: AgentHandle) {
        self.online.write().await.insert((tenant_id.to_string(), machine_id), handle);
    }

    pub async fn remove_online(&self, tenant_id: &str, machine_id: &str) {
        self.online.write().await.remove(&(tenant_id.to_string(), machine_id.to_string()));
    }

    /// Send a message to all online agents belonging to tenant whose agent_id is in the provided set.
    pub async fn send_to_agents(&self, tenant_id: &str, agent_ids: &[Uuid], msg: WssMessage) {
        let online = self.online.read().await;
        for ((tid, _), handle) in online.iter() {
            if tid == tenant_id && agent_ids.contains(&handle.agent_id) {
                let _ = handle.outbound_tx.send(msg.clone()).await;
            }
        }
    }

    pub async fn send_to_agent_id(&self, tenant_id: &str, agent_id: Uuid, msg: WssMessage) {
        self.send_to_agents(tenant_id, &[agent_id], msg).await;
    }

    pub async fn is_online(&self, tenant_id: &str, agent_id: Uuid) -> bool {
        let online = self.online.read().await;
        online.iter().any(|((tid, _), h)| tid == tenant_id && h.agent_id == agent_id)
    }
}
