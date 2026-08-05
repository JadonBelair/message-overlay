use std::{cell::RefCell, rc::Rc};

use crate::{ScrollingMessage, SIGN_OFFS};

use common::WebSocketMessage;
use gtk4::{
    gdk::{self, prelude::GdkCairoContextExt, RGBA},
    glib,
    prelude::{DrawingAreaExtManual, WidgetExt, WidgetExtManual},
    DrawingArea,
};
use rand::seq::IndexedRandom;

#[derive(Clone)]
pub struct AppState {
    pub(crate) active_messages: Rc<RefCell<Vec<ScrollingMessage>>>,
    pub(crate) drawing_box: DrawingArea,
}

impl AppState {
    pub fn new(drawing_box: DrawingArea) -> Rc<Self> {
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

                let active_messages = app_state.active_messages.borrow();
                for active_message in active_messages.iter() {
                    cr.move_to(active_message.current_x, active_message.current_y);
                    pangocairo::functions::update_layout(cr, &active_message.layout);
                    pangocairo::functions::layout_path(cr, &active_message.layout);

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
                        msg.current_x >= -msg.width
                    });
                }

                area.queue_draw();

                *last_frame_time.borrow_mut() = Some(frame_time);
                gdk::glib::ControlFlow::Continue
            }
        ));
    }

    pub fn spawn_new_message(&self, msg: WebSocketMessage) {
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
        layout.set_alignment(gdk::pango::Alignment::Right);

        let (width, height) = layout.pixel_size();
        let spawn_y = rand::random_range(10.0..(window_height - height as f64 - 10.0));

        self.active_messages.borrow_mut().push(ScrollingMessage {
            color,
            outline_color,
            layout,

            current_x: window_width,
            current_y: spawn_y,
            speed: msg.speed as f64,

            width: width as f64,
        });
    }
}
