use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct WebSocketMessage {
    pub name: String,
    pub color: String,
    pub msg: String,
}
