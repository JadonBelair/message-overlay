use axum::{
    extract::{
        ws::{Message, WebSocket},
        Form, State, WebSocketUpgrade,
    },
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

const INDEX_HTML: &str = include_str!("../../index.html");

#[derive(Clone)]
struct AppState {
    tx: broadcast::Sender<String>,
}

#[tokio::main]
async fn main() {
    let (tx, _) = broadcast::channel::<String>(100);
    let state = AppState { tx };

    let app = Router::new()
        .route("/", get(root).post(accept_form))
        .route("/ws", get(websocket_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();

    let _ = axum::serve(listener, app).await;
}

async fn root() -> Html<&'static str> {
    Html(INDEX_HTML)
}

#[derive(Deserialize, Serialize)]
struct Input {
    name: String,
    msg: String,
}

async fn accept_form(State(state): State<AppState>, Form(input): Form<Input>) -> impl IntoResponse {
    println!("{}: {}", input.name, input.msg);

    if let Ok(json_output) = serde_json::to_string(&input) {
        let _ = state.tx.send(json_output);
    }

    StatusCode::NO_CONTENT
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.tx.subscribe();

    while let Ok(msg) = rx.recv().await {
        if socket.send(Message::Text(msg.into())).await.is_err() {
            break;
        }
    }
}
