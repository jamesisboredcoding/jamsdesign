//! Colour picker: saturation/value square + vertical hue strip. Click or drag to pick.

use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, gradient};
use iced::widget::Canvas;
use iced::{Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme};
use jd_core::Hsv;

use super::to_color;
use crate::app::Message;

pub fn sv_square<'a>(hsv: Hsv) -> Element<'a, Message> {
    Canvas::new(SvSquare { hsv }).width(Fill).height(120).into()
}

pub fn hue_strip<'a>(hsv: Hsv) -> Element<'a, Message> {
    Canvas::new(HueStrip { hue: hsv.h }).width(14).height(120).into()
}

#[derive(Default)]
pub struct Drag(bool);

/// Shared press/drag handling: returns the cursor position (clamped to bounds) while dragging.
fn drag_position(drag: &mut Drag, event: &canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<Point> {
    match event {
        canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
            cursor.position_in(bounds)?;
            drag.0 = true;
        }
        canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
            drag.0 = false;
            return None;
        }
        canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if drag.0 => {}
        _ => return None,
    }
    let p = cursor.position()?;
    Some(Point::new(
        (p.x - bounds.x).clamp(0.0, bounds.width),
        (p.y - bounds.y).clamp(0.0, bounds.height),
    ))
}

struct SvSquare {
    hsv: Hsv,
}

impl canvas::Program<Message> for SvSquare {
    type State = Drag;

    fn update(&self, drag: &mut Drag, event: &canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<Message>> {
        let p = drag_position(drag, event, bounds, cursor)?;
        let msg = Message::PickSv(p.x / bounds.width, 1.0 - p.y / bounds.height);
        Some(canvas::Action::publish(msg).and_capture())
    }

    fn draw(&self, _: &Drag, renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let size = bounds.size();
        let rect = Path::rounded_rectangle(Point::ORIGIN, size, 3.0.into());
        let pure = to_color(Hsv { h: self.hsv.h, s: 1.0, v: 1.0 });
        frame.fill(&rect, pure);
        frame.fill(
            &rect,
            gradient::Linear::new(Point::ORIGIN, Point::new(size.width, 0.0))
                .add_stop(0.0, Color::WHITE)
                .add_stop(1.0, Color::TRANSPARENT),
        );
        frame.fill(
            &rect,
            gradient::Linear::new(Point::ORIGIN, Point::new(0.0, size.height))
                .add_stop(0.0, Color::TRANSPARENT)
                .add_stop(1.0, Color::BLACK),
        );
        let marker = Point::new(self.hsv.s * size.width, (1.0 - self.hsv.v) * size.height);
        let ring = Path::circle(marker, 5.0);
        frame.stroke(&ring, Stroke::default().with_width(3.0).with_color(Color::from_rgba(0.0, 0.0, 0.0, 0.4)));
        frame.stroke(&ring, Stroke::default().with_width(1.5).with_color(Color::WHITE));
        vec![frame.into_geometry()]
    }
}

struct HueStrip {
    hue: f32,
}

impl canvas::Program<Message> for HueStrip {
    type State = Drag;

    fn update(&self, drag: &mut Drag, event: &canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<Message>> {
        let p = drag_position(drag, event, bounds, cursor)?;
        Some(canvas::Action::publish(Message::PickHue(p.y / bounds.height * 360.0)).and_capture())
    }

    fn draw(&self, _: &Drag, renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let size = bounds.size();
        let mut g = gradient::Linear::new(Point::ORIGIN, Point::new(0.0, size.height));
        for i in 0..=6 {
            let h = i as f32 * 60.0;
            g = g.add_stop(i as f32 / 6.0, to_color(Hsv { h: h % 360.0, s: 1.0, v: 1.0 }));
        }
        frame.fill(&Path::rounded_rectangle(Point::ORIGIN, size, 3.0.into()), g);
        let y = self.hue / 360.0 * size.height;
        let knob = Path::rounded_rectangle(Point::new(0.5, y - 2.0), Size::new(size.width - 1.0, 4.0), 1.0.into());
        frame.stroke(&knob, Stroke::default().with_width(1.5).with_color(Color::WHITE));
        vec![frame.into_geometry()]
    }
}
