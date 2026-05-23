use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WssMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub timestamp: DateTime<Utc>,
    pub payload: Value,
}

impl WssMessage {
    pub fn new<T: Serialize>(msg_type: &str, payload: &T) -> Result<Self, serde_json::Error> {
        Ok(Self {
            msg_type: msg_type.to_string(),
            timestamp: Utc::now(),
            payload: serde_json::to_value(payload)?,
        })
    }

    pub fn parse_payload<T: for<'de> Deserialize<'de>>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_value(self.payload.clone())
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
}
