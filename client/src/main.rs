use std::{cell::RefCell, rc::Rc, time::Duration};

use common::WebSocketMessage;

use futures_util::StreamExt;
use gtk4::{
    cairo::Region,
    gdk::{self, prelude::SurfaceExt, RGBA},
    gio::prelude::{ApplicationExt, ApplicationExtManual},
    glib,
    prelude::{DrawingAreaExtManual, GtkWindowExt, NativeExt, WidgetExt, WidgetExtManual},
    style_context_add_provider_for_display, CssProvider, DrawingArea,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use rand::seq::IndexedRandom;
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    time::sleep,
};
use tokio_tungstenite::tungstenite::protocol::Message;

const SCROLL_SPEED: f64 = 150.0;
const FONT_SIZE: i32 = 24;

const SIGN_OFFS: &[&'static str] = &[
    "Sincerely",
    "Yours truly",
    "Best wishes",
    "Love",
    "Respectfully",
    "Cheers",
    "Kind regards",
];

struct ScrollingMessage {
    text: String,
    color: String,
    current_x: f64,
    current_y: f64,
    width: f64,
}

fn activate(application: &gtk4::Application, mut rx: UnboundedReceiver<WebSocketMessage>) {
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

    let active_messages = Rc::new(RefCell::new(Vec::<ScrollingMessage>::new()));

    let active_messages_spawn = active_messages.clone();
    let drawing_spawn = drawing_box.clone();
    let spawn_new_message = move |msg: WebSocketMessage| {
        let window_width = drawing_spawn.width() as f64;
        let window_height = drawing_spawn.height() as f64;

        // im just making an assumption that 2x the font size
        // will be enough for both the message and the sign off
        let spawn_y = rand::random_range(10.0..(window_height - (FONT_SIZE as f64 * 2.0) - 10.0));

        let sign_off = format!(
            "{}, {}",
            SIGN_OFFS.choose(&mut rand::rng()).unwrap(),
            msg.name
        );

        let msg_len = if sign_off.len() > msg.msg.len() {
            sign_off.len()
        } else {
            msg.msg.len()
        } as i32;

        active_messages_spawn.borrow_mut().push(ScrollingMessage {
            // also assuming that the font size will be enough
            // for the width, with a small bit of padding
            width: (msg_len * FONT_SIZE + 10) as f64,

            text: format!("{}\n{}", msg.msg, sign_off),
            color: msg.color,
            current_x: window_width,
            current_y: spawn_y,
        });
    };

    let active_messages_draw = active_messages.clone();
    drawing_box.set_draw_func(move |_area, cr, width, height| {
        cr.rectangle(0.0, 0.0, width as f64, height as f64);
        cr.clip();

        cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
        cr.paint().unwrap();

        let layout = pangocairo::functions::create_layout(cr);

        let active_messages = active_messages_draw.borrow();
        for active_message in active_messages.iter() {
            layout.set_text(&active_message.text);

            let font_desc =
                gdk::pango::FontDescription::from_string(&format!("Sans Bold {FONT_SIZE}"));
            layout.set_font_description(Some(&font_desc));
            layout.set_alignment(gdk::pango::Alignment::Right);

            cr.move_to(active_message.current_x, active_message.current_y);

            pangocairo::functions::layout_path(cr, &layout);

            let text_color = RGBA::parse(&active_message.color).unwrap();

            // uses the colors luminance to determine best contrast outline color
            let outline_shade = 1.0
                - (0.2126 * text_color.red()
                    + 0.7152 * text_color.green()
                    + 0.0722 * text_color.blue()) as f64;

            cr.set_source_rgb(outline_shade, outline_shade, outline_shade);

            cr.set_line_width(5.0);
            cr.set_line_join(gdk::cairo::LineJoin::Round);
            cr.stroke_preserve().unwrap();

            cr.set_source_rgb(
                text_color.red() as f64,
                text_color.green() as f64,
                text_color.blue() as f64,
            );
            cr.fill().unwrap();
        }
    });

    let active_messages_tick = active_messages.clone();
    let last_frame_time = Rc::new(RefCell::new(Option::<i64>::None));
    drawing_box.add_tick_callback(move |area, frame_clock| {
        let frame_time = frame_clock.frame_time();

        if let Some(prev_time) = *last_frame_time.borrow() {
            let delta_seconds = (frame_time - prev_time) as f64 / 1000000.0;
            let pixels_to_move = SCROLL_SPEED * delta_seconds;

            let mut msgs = active_messages_tick.borrow_mut();

            msgs.retain_mut(|msg| {
                msg.current_x -= pixels_to_move;

                if msg.current_x < -msg.width {
                    false
                } else {
                    true
                }
            });
        }

        area.queue_draw();

        *last_frame_time.borrow_mut() = Some(frame_time);
        gdk::glib::ControlFlow::Continue
    });

    glib::MainContext::default().spawn_local(async move {
        while let Some(msg) = rx.recv().await {
            spawn_new_message(msg);
        }
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

fn spawn_websocket_client_thread(tx: UnboundedSender<WebSocketMessage>) {
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
                                            let _ = tx.send(msg);
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

    let (tx, rx) = mpsc::unbounded_channel::<WebSocketMessage>();

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
