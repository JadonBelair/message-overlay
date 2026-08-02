use std::{cell::RefCell, rc::Rc, time::Duration};

use futures_util::StreamExt;
use gtk4::{
    cairo::Region,
    gdk::{self, prelude::SurfaceExt},
    gio::prelude::{ApplicationExt, ApplicationExtManual},
    glib,
    prelude::{FixedExt, GtkWindowExt, NativeExt, WidgetExt, WidgetExtManual},
    style_context_add_provider_for_display, CssProvider, Fixed, Label, Orientation, Overflow,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde::{Deserialize, Serialize};
use tokio::{sync::mpsc, time::sleep};
use tokio_tungstenite::tungstenite::protocol::Message;

const SCROLL_SPEED: f64 = 150.0;
const FONT_SIZE: i32 = 24;

struct ScrollingMessage {
    label: Label,
    current_x: f64,
    current_y: f64,
    width: f64,
}

#[derive(Serialize, Deserialize)]
struct WebSocketMessage {
    name: String,
    msg: String,
}

fn activate(application: &gtk4::Application) {
    let window = gtk4::ApplicationWindow::new(application);

    window.init_layer_shell();

    window.set_layer(Layer::Overlay);

    window.set_keyboard_mode(KeyboardMode::None);

    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Bottom, true);
    window.set_anchor(Edge::Left, true);
    window.set_anchor(Edge::Right, true);

    let outer_box = Fixed::new();
    outer_box.set_overflow(Overflow::Hidden);
    outer_box.add_css_class("message-container");

    window.set_child(Some(&outer_box));

    let provider = CssProvider::new();
    provider.load_from_data(
        format!(
            "
        window {{
            background-color: transparent;
        }}

        .message-text {{
            color: #00FFCC;
            text-shadow: 
                -2px -2px 0 #000,
                 2px -2px 0 #000,
                -2px  2px 0 #000,
                 2px  2px 0 #000;
            font-size: {FONT_SIZE}px;
            font-weight: bold;
        }}
        "
        )
        .as_str(),
    );

    style_context_add_provider_for_display(
        &gdk::Display::default().expect("Could not connect to a display."),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let active_messages = Rc::new(RefCell::new(Vec::<ScrollingMessage>::new()));
    let active_messages_spawm = active_messages.clone();
    let fixed_spawn = outer_box.clone();

    let spawn_new_message = move |text: &str| {
        let label = Label::new(Some(text));
        label.add_css_class("message-text");

        fixed_spawn.put(&label, 0.0, 0.0);
        let (_, label_width, _, _) = label.measure(Orientation::Horizontal, -1);

        let window_width = 1920.0 * 2.0;
        let window_height = 1080.0;

        let spawn_y = rand::random_range(0.0..(window_height - FONT_SIZE as f64));

        active_messages_spawm.borrow_mut().push(ScrollingMessage {
            label: label.clone(),
            current_x: window_width,
            current_y: spawn_y,
            width: label_width as f64,
        });

        fixed_spawn.move_(&label, window_width, spawn_y);
    };

    let active_messages_tick = active_messages.clone();
    let fixed_tick = outer_box.clone();
    let last_frame_time = Rc::new(RefCell::new(Option::<i64>::None));

    outer_box.add_tick_callback(move |_, frame_clock| {
        let frame_time = frame_clock.frame_time();

        if let Some(prev_time) = *last_frame_time.borrow() {
            let delta_seconds = (frame_time - prev_time) as f64 / 1000000.0;
            let pixels_to_move = SCROLL_SPEED * delta_seconds;

            let mut msgs = active_messages_tick.borrow_mut();

            msgs.retain_mut(|msg| {
                msg.current_x -= pixels_to_move;

                if msg.current_x < -msg.width {
                    fixed_tick.remove(&msg.label);
                    false
                } else {
                    fixed_tick.move_(&msg.label, msg.current_x, msg.current_y);
                    true
                }
            });
        }

        *last_frame_time.borrow_mut() = Some(frame_time);
        gdk::glib::ControlFlow::Continue
    });

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    let spawn_handle = spawn_new_message.clone();
    glib::MainContext::default().spawn_local(async move {
        while let Some(msg) = rx.recv().await {
            spawn_handle(&msg);
        }
    });

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
                                            let _ = tx.send(format!("{}: {}", msg.name, msg.msg));
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

    // this gives us mouse passthrough
    window.connect_realize(|win| {
        if let Some(surface) = win.surface() {
            let empty_region = Region::create();
            surface.set_input_region(Some(&empty_region));
        }
    });

    window.show();
}

fn main() {
    let application = gtk4::Application::new(Some("jadon.message-overlay"), Default::default());

    application.connect_activate(|app| {
        activate(app);
    });

    application.run();
}
