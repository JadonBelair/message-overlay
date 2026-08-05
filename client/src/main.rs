use std::{cell::RefCell, rc::Rc, time::Duration};

use common::WebSocketMessage;

use futures_util::StreamExt;
use gtk4::{
    cairo::Region,
    gdk::{
        self,
        prelude::{GdkCairoContextExt, SurfaceExt},
        RGBA,
    },
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
    color: RGBA,
    outline_color: RGBA,
    font_desc: gdk::pango::FontDescription,
    current_x: f64,
    current_y: f64,
    speed: f64,
    width: f64,
}

#[derive(Clone)]
struct AppState {
    active_messages: Rc<RefCell<Vec<ScrollingMessage>>>,
    drawing_box: DrawingArea,
}

impl AppState {
    fn new(drawing_box: DrawingArea) -> Rc<Self> {
        let app = Rc::new(Self {
            active_messages: Rc::new(RefCell::new(Vec::new())),
            drawing_box,
        });

        app.setup_drawing_box();
        app
    }

    fn setup_drawing_box(&self) {
        self.drawing_box.set_draw_func(glib::clone!(
            #[strong(rename_to = app_state)]
            self,
            move |_area, cr, width, height| {
                cr.rectangle(0.0, 0.0, width as f64, height as f64);
                cr.clip();

                cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);

                cr.set_operator(gdk::cairo::Operator::Clear);
                cr.paint().unwrap();
                cr.set_operator(gdk::cairo::Operator::Over);

                let layout = pangocairo::functions::create_layout(cr);
                layout.set_alignment(gdk::pango::Alignment::Right);

                let active_messages = app_state.active_messages.borrow();
                for active_message in active_messages.iter() {
                    layout.set_text(&active_message.text);
                    layout.set_font_description(Some(&active_message.font_desc));

                    cr.move_to(active_message.current_x, active_message.current_y);
                    pangocairo::functions::layout_path(cr, &layout);

                    cr.set_source_color(&active_message.outline_color);

                    cr.set_line_width(2.0);
                    cr.set_line_join(gdk::cairo::LineJoin::Round);
                    cr.stroke_preserve().unwrap();

                    cr.set_source_color(&active_message.color);
                    cr.fill().unwrap();
                }
            }
        ));

        let last_frame_time = Rc::new(RefCell::new(Option::<i64>::None));
        self.drawing_box.add_tick_callback(glib::clone!(
            #[strong(rename_to = app_state)]
            self,
            move |area, frame_clock| {
                let frame_time = frame_clock.frame_time();

                if let Some(prev_time) = *last_frame_time.borrow() {
                    let delta_seconds = (frame_time - prev_time) as f64 / 1000000.0;

                    let mut msgs = app_state.active_messages.borrow_mut();
                    msgs.retain_mut(|msg| {
                        let pixels_to_move = msg.speed * delta_seconds;
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
            }
        ));
    }

    fn spawn_new_message(&self, msg: WebSocketMessage) {
        let window_width = self.drawing_box.width() as f64;
        let window_height = self.drawing_box.height() as f64;

        let font_desc =
            gdk::pango::FontDescription::from_string(&format!("Sans Bold {}", msg.font_size));

        let color = RGBA::parse(msg.color).unwrap_or(RGBA::BLACK);

        let outline_shade =
            1.0 - (0.2126 * color.red() + 0.7152 * color.green() + 0.0722 * color.blue()) as f32;

        let outline_color = RGBA::new(outline_shade, outline_shade, outline_shade, 1.0);

        let text = format!(
            "{}\n{}, {}",
            msg.msg,
            SIGN_OFFS.choose(&mut rand::rng()).unwrap(),
            msg.name
        );

        let pango_ctx = self.drawing_box.pango_context();
        let layout = gdk::pango::Layout::new(&pango_ctx);
        layout.set_text(&text);
        layout.set_font_description(Some(&font_desc));

        let (width, height) = layout.pixel_size();
        let spawn_y = rand::random_range(10.0..(window_height - height as f64 - 10.0));

        self.active_messages.borrow_mut().push(ScrollingMessage {
            text,
            color,
            outline_color,
            font_desc,

            current_x: window_width,
            current_y: spawn_y,
            speed: msg.speed as f64,

            width: width as f64,
        });
    }
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
