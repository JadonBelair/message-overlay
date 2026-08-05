use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct WebSocketMessage {
    pub name: String,
    pub msg: String,
    pub color: String,
    pub font_size: i32,
    pub speed: i32,
}
