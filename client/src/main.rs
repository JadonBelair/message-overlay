use std::{cell::RefCell, rc::Rc, time::Duration};

use crate::state::AppState;

use common::WebSocketMessage;
use futures_util::StreamExt;
use gtk4::{
    cairo::Region,
    gdk::{self, prelude::SurfaceExt, RGBA},
    gio::prelude::{ApplicationExt, ApplicationExtManual},
    glib,
    prelude::{GtkWindowExt, NativeExt, WidgetExt},
    style_context_add_provider_for_display, CssProvider, DrawingArea,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use tokio::{
    sync::mpsc::{self, Receiver, Sender},
    time::sleep,
};
use tokio_tungstenite::tungstenite::protocol::Message;

mod state;

pub const SIGN_OFFS: &[&'static str] = &[
    "Sincerely",
    "Yours truly",
    "Best wishes",
    "Love",
    "Respectfully",
    "Cheers",
    "Kind regards",
];

pub struct ScrollingMessage {
    pub(crate) color: RGBA,
    pub(crate) outline_color: RGBA,
    pub(crate) layout: gdk::pango::Layout,
    pub(crate) current_x: f64,
    pub(crate) current_y: f64,
    pub(crate) speed: f64,
    pub(crate) width: f64,
}

fn activate(application: &gtk4::Application, mut rx: Receiver<WebSocketMessage>) {
    let window = gtk4::ApplicationWindow::new(application);

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);

    window.set_keyboard_mode(KeyboardMode::None);

    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Bottom, true);
    window.set_anchor(Edge::Left, true);
    window.set_anchor(Edge::Right, true);

    let drawing_box = DrawingArea::new();
    window.set_child(Some(&drawing_box));

    let provider = CssProvider::new();
    provider.load_from_data(
        "
        window {
            background-color: transparent;
        }
        ",
    );

    style_context_add_provider_for_display(
        &gdk::Display::default().expect("Could not connect to a display."),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let app_state = AppState::new(drawing_box);

    glib::MainContext::default().spawn_local(glib::clone!(
        #[strong]
        app_state,
        async move {
            while let Some(msg) = rx.recv().await {
                app_state.spawn_new_message(msg);
            }
        }
    ));

    // this gives us mouse passthrough
    window.connect_realize(|win| {
        if let Some(surface) = win.surface() {
            let empty_region = Region::create();
            surface.set_input_region(Some(&empty_region));
        }
    });

    window.present();
}

fn spawn_websocket_client_thread(tx: Sender<WebSocketMessage>) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();

        rt.block_on(async move {
            let url = format!(
                "ws://{}",
                std::env::var("WEBSOCKET_URL").unwrap_or("127.0.0.1:3000/ws".to_string())
            );

            let mut backoff_sec = 1;
            let max_backoff = 32;

            loop {
                match tokio_tungstenite::connect_async(&url).await {
                    Ok((ws_stream, _)) => {
                        // reset backoff cooldown on successful connection
                        backoff_sec = 1;

                        let (_, mut read_stream) = ws_stream.split();

                        while let Some(msg_result) = read_stream.next().await {
                            match msg_result {
                                Ok(Message::Text(text)) => {
                                    match serde_json::from_str::<WebSocketMessage>(&text) {
                                        Ok(msg) => {
                                            println!("{:?}", msg);
                                            let _ = tx.send(msg).await;
                                        }
                                        Err(_) => {
                                            eprintln!("Received invalid text: {text}");
                                        }
                                    }
                                }
                                Ok(Message::Close(_)) => {
                                    println!("Websocket connection closed");
                                    break;
                                }
                                Ok(other) => {
                                    println!("Received unknown frame: {other:?}");
                                }
                                Err(e) => {
                                    eprintln!("Websocket error: {e}");
                                    break;
                                }
                            }
                        }
                    }
                    Err(_) => eprintln!("Failed to connect to server"),
                }

                println!("Reconnecting in {backoff_sec} seconds");
                sleep(Duration::from_secs(backoff_sec)).await;

                backoff_sec = (backoff_sec * 2).min(max_backoff);
            }
        });
    });
}

fn main() {
    let application = gtk4::Application::new(Some("jadon.message-overlay"), Default::default());

    let (tx, rx) = mpsc::channel::<WebSocketMessage>(100);

    // shenanigans needed to pass the receiver to the application
    let rx = Rc::new(RefCell::new(Some(rx)));

    application.connect_activate(move |app| {
        if let Some(rx) = rx.borrow_mut().take() {
            activate(app, rx);
        }
    });

    spawn_websocket_client_thread(tx);

    application.run();
}
