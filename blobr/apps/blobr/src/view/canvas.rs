//! The canvas: the rendered page (an image from `jd-vector-render`) with an overlay on top
//! that takes the mouse and draws selection, handles, hover, marquee and frame names.

use iced::mouse::{self, Cursor, ScrollDelta};
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text};
use iced::widget::{container, image, space, stack};
use iced::{Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme};
use jd_ui::tokens::*;

use crate::app::{Blobr, Drag, Handle, Message};
use crate::tool::Tool;

pub fn view(app: &Blobr) -> Element<'_, Message> {
    let picture: Element<'_, Message> = match &app.canvas {
        Some(a) => image(a.handle().clone()).width(app.viewport.width).height(app.viewport.height).filter_method(image::FilterMethod::Linear).into(),
        None => space().into(),
    };
    container(stack![picture, canvas::Canvas::new(Overlay { app }).width(Fill).height(Fill)])
        .width(Fill)
        .height(Fill)
        .clip(true)
        .style(jd_ui::style::fill(BG_CANVAS))
        .into()
}

struct Overlay<'a> {
    app: &'a Blobr,
}

impl canvas::Program<Message> for Overlay<'_> {
    type State = ();

    fn update(&self, _: &mut (), event: &canvas::Event, bounds: Rectangle, cursor: Cursor) -> Option<canvas::Action<Message>> {
        let app = self.app;
        if (bounds.width - app.viewport.width).abs() > 0.5 || (bounds.height - app.viewport.height).abs() > 0.5 {
            return Some(canvas::Action::publish(Message::Viewport(bounds.size())));
        }
        let canvas::Event::Mouse(event) = event else { return None };
        let publish = |m| Some(canvas::Action::publish(m).and_capture());
        let local = |p: Point| Point::new(p.x - bounds.x, p.y - bounds.y);
        match event {
            mouse::Event::ButtonPressed(button) => {
                let p = cursor.position_in(bounds)?;
                publish(Message::CanvasPress { at: p, button: *button })
            }
            mouse::Event::CursorMoved { position } => {
                // While dragging, follow the pointer even outside the canvas.
                if app.drag.is_some() || bounds.contains(*position) {
                    publish(Message::CanvasMove(local(*position)))
                } else {
                    None
                }
            }
            mouse::Event::ButtonReleased(_) if app.drag.is_some() => {
                let p = cursor.position().map_or(Point::ORIGIN, local);
                publish(Message::CanvasRelease(p))
            }
            mouse::Event::WheelScrolled { delta } => {
                let p = cursor.position_in(bounds)?;
                let d = match *delta {
                    ScrollDelta::Lines { x, y } => iced::Vector::new(x * 60.0, y * 60.0),
                    ScrollDelta::Pixels { x, y } => iced::Vector::new(x, y),
                };
                publish(Message::CanvasScroll { delta: d, at: p })
            }
            _ => None,
        }
    }

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: Cursor) -> Vec<Geometry> {
        let app = self.app;
        let doc = app.doc();
        let mut frame = Frame::new(renderer, bounds.size());

        // Names above top-level frames, like Figma.
        for &c in doc.children(app.page) {
            let Some(n) = doc.node(c) else { continue };
            if !(n.kind.is_frame_like() || matches!(n.kind, jd_vector::NodeKind::Section | jd_vector::NodeKind::ComponentSet { .. })) {
                continue;
            }
            let Some(r) = app.screen_bounds(c) else { continue };
            let selected = app.selection.contains(&c);
            frame.fill_text(Text {
                content: n.name.clone(),
                position: Point::new(r.x, r.y - 16.0),
                color: if selected { ACCENT_PRIMARY } else { TEXT_SECONDARY },
                size: 11.0.into(),
                ..Text::default()
            });
        }

        if let Some(h) = app.hover.filter(|h| !app.selection.contains(h))
            && let Some(r) = app.screen_bounds(h)
        {
            frame.stroke(&Path::rectangle(r.position(), r.size()), Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
        }

        for &id in &app.selection {
            let Some(r) = app.screen_bounds(id) else { continue };
            frame.stroke(&Path::rectangle(r.position(), r.size()), Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
        }
        if let [one] = app.selection[..]
            && let Some(r) = app.screen_bounds(one)
        {
            for h in Handle::ALL {
                let (ax, ay) = h.anchor();
                let c = Point::new(r.x + r.width * ax as f32, r.y + r.height * ay as f32);
                let sq = Path::rectangle(Point::new(c.x - 3.5, c.y - 3.5), Size::new(7.0, 7.0));
                frame.fill(&sq, Color::WHITE);
                frame.stroke(&sq, Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
            }
            if let Some(n) = doc.node(one) {
                let label = format!("{:.0} × {:.0}", n.props.width, n.props.height);
                let w = label.chars().count() as f32 * 6.0 + 10.0;
                let at = Point::new(r.x + r.width / 2.0 - w / 2.0, r.y + r.height + 8.0);
                frame.fill(&Path::rounded_rectangle(at, Size::new(w, 16.0), 3.0.into()), ACCENT_PRIMARY);
                frame.fill_text(Text {
                    content: label,
                    position: Point::new(at.x + 5.0, at.y + 2.0),
                    color: Color::WHITE,
                    size: 10.0.into(),
                    ..Text::default()
                });
            }
        }

        // Smart guides while a drag snaps, and Alt-hover distances, in Figma's red.
        let red = Color::from_rgb8(0xF2, 0x48, 0x22);
        for g in &app.guides {
            let line = Path::line(app.camera.to_screen(g.from), app.camera.to_screen(g.to));
            frame.stroke(&line, Stroke::default().with_width(1.0).with_color(red));
        }
        if app.modifiers.alt()
            && app.drag.is_none()
            && let Some(a) = app.selection_bounds()
        {
            let other = app.hover.filter(|h| !app.selection.contains(h)).or_else(|| {
                let s = *app.selection.first()?;
                doc.parent(s).filter(|&p| p != app.page)
            });
            if let Some(b) = other.and_then(|o| doc.world_bounds(o)) {
                for (l, d) in jd_vector::snap::measure(a, b) {
                    let (p, q) = (app.camera.to_screen(l.from), app.camera.to_screen(l.to));
                    frame.stroke(&Path::line(p, q), Stroke::default().with_width(1.0).with_color(red));
                    let label = format!("{}", (d * 100.0).round() / 100.0);
                    let w = label.chars().count() as f32 * 6.0 + 8.0;
                    let at = Point::new((p.x + q.x) / 2.0 - w / 2.0, (p.y + q.y) / 2.0 - 8.0);
                    frame.fill(&Path::rounded_rectangle(at, Size::new(w, 16.0), 3.0.into()), red);
                    frame.fill_text(Text { content: label, position: Point::new(at.x + 4.0, at.y + 2.0), color: Color::WHITE, size: 10.0.into(), ..Text::default() });
                }
                let r = Rectangle::new(app.camera.to_screen(kurbo::Point::new(b.x0, b.y0)), Size::new((b.width() * app.camera.zoom) as f32, (b.height() * app.camera.zoom) as f32));
                frame.stroke(&Path::rectangle(r.position(), r.size()), Stroke::default().with_width(1.0).with_color(red));
            }
        }

        match &app.drag {
            Some(Drag::Marquee { start, now }) => {
                let (a, b) = (app.camera.to_screen(*start), app.camera.to_screen(*now));
                let r = Rectangle::new(Point::new(a.x.min(b.x), a.y.min(b.y)), Size::new((a.x - b.x).abs(), (a.y - b.y).abs()));
                let p = Path::rectangle(r.position(), r.size());
                frame.fill(&p, Color { a: 0.1, ..ACCENT_PRIMARY });
                frame.stroke(&p, Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
            }
            Some(Drag::Create { tool, start, now }) => {
                let (a, b) = (app.camera.to_screen(*start), app.camera.to_screen(*now));
                let r = Rectangle::new(Point::new(a.x.min(b.x), a.y.min(b.y)), Size::new((a.x - b.x).abs(), (a.y - b.y).abs()));
                let p = if *tool == Tool::Ellipse {
                    Path::new(|b| {
                        b.ellipse(iced::widget::canvas::path::arc::Elliptical {
                            center: r.center(),
                            radii: iced::Vector::new(r.width / 2.0, r.height / 2.0),
                            rotation: iced::Radians(0.0),
                            start_angle: iced::Radians(0.0),
                            end_angle: iced::Radians(std::f32::consts::TAU),
                        })
                    })
                } else {
                    Path::rectangle(r.position(), r.size())
                };
                frame.stroke(&p, Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
            }
            _ => {}
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, _: &(), bounds: Rectangle, cursor: Cursor) -> mouse::Interaction {
        let app = self.app;
        match app.drag {
            Some(Drag::Pan { .. }) => return mouse::Interaction::Grabbing,
            Some(Drag::Create { .. }) => return mouse::Interaction::Crosshair,
            _ => {}
        }
        let Some(p) = cursor.position_in(bounds) else { return mouse::Interaction::default() };
        if app.space_held || app.tool == Tool::Hand {
            return mouse::Interaction::Grab;
        }
        if app.tool.creates() {
            return mouse::Interaction::Crosshair;
        }
        match app.handle_at(p).map(|(_, h)| h) {
            Some(Handle::Left | Handle::Right) => mouse::Interaction::ResizingHorizontally,
            Some(Handle::Top | Handle::Bottom) => mouse::Interaction::ResizingVertically,
            Some(Handle::TopLeft | Handle::BottomRight) => mouse::Interaction::ResizingDiagonallyDown,
            Some(Handle::TopRight | Handle::BottomLeft) => mouse::Interaction::ResizingDiagonallyUp,
            None => mouse::Interaction::default(),
        }
    }
}
