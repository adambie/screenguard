use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, RwLock};
use uuid::Uuid;

use crate::db::DbPool;
use common::protocol::WssMessage;

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
    /// machine_id → AgentHandle for currently connected agents.
    pub online: Arc<RwLock<HashMap<String, AgentHandle>>>,
    /// machine_id → PairingHandle for agents waiting for admin accept.
    pub pending: Arc<RwLock<HashMap<String, PairingHandle>>>,
    /// agent_id → oneshot sender for pending log requests.
    pub log_requests: Arc<RwLock<HashMap<Uuid, oneshot::Sender<Vec<String>>>>>,
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
        })
    }

    pub async fn add_online(&self, machine_id: String, handle: AgentHandle) {
        self.online.write().await.insert(machine_id, handle);
    }

    pub async fn remove_online(&self, machine_id: &str) {
        self.online.write().await.remove(machine_id);
    }

    /// Send a message to all online agents whose agent_id is in the provided set.
    pub async fn send_to_agents(&self, agent_ids: &[Uuid], msg: WssMessage) {
        let online = self.online.read().await;
        for handle in online.values() {
            if agent_ids.contains(&handle.agent_id) {
                let _ = handle.outbound_tx.send(msg.clone()).await;
            }
        }
    }

    pub async fn send_to_agent_id(&self, agent_id: Uuid, msg: WssMessage) {
        self.send_to_agents(&[agent_id], msg).await;
    }

    pub async fn is_online(&self, agent_id: Uuid) -> bool {
        self.online.read().await.values().any(|h| h.agent_id == agent_id)
    }
}
