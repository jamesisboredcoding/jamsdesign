//! Presenting: the prototype full-window. Click hotspots to follow interactions; Escape
//! leaves, arrows step through frames.

use iced::widget::{column, container, image, mouse_area, pin, row, space, stack, text};
use iced::{Color, Element, Fill, Point};
use jd_ui::tokens::*;
use jd_vector::Transition;

use crate::app::{Blobr, Message, Shot};

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn picture<'a>(app: &Blobr, shot: &Shot, offset: Point, opacity: f32) -> Element<'a, Message> {
    let o = app.shot_origin(shot);
    let (w, h) = ((shot.size.width * shot.fit) as f32, (shot.size.height * shot.fit) as f32);
    pin(image(shot.image.clone()).width(w).height(h).opacity(opacity)).x(o.x + offset.x).y(o.y + offset.y).into()
}

pub fn view(app: &Blobr) -> Option<Element<'_, Message>> {
    let p = app.present.as_ref()?;
    let (w, h) = (app.chrome.size.width, app.chrome.size.height);
    let mut layers = stack![container(space()).width(Fill).height(Fill).style(jd_ui::style::fill(Color::from_rgb8(0x10, 0x10, 0x10)))];
    // Slides move both frames; a dissolve fades the new one in over the old.
    let (mut new_off, mut new_alpha) = (Point::ORIGIN, 1.0);
    if let Some((old, transition, start, d)) = &p.leaving {
        let t = ease(start.elapsed().as_secs_f32() / d);
        let (dx, dy) = match transition {
            Transition::SlideLeft => (w, 0.0),
            Transition::SlideRight => (-w, 0.0),
            Transition::SlideUp => (0.0, h),
            Transition::SlideDown => (0.0, -h),
            _ => (0.0, 0.0),
        };
        let old_off = Point::new(-dx * t, -dy * t);
        new_off = Point::new(dx * (1.0 - t), dy * (1.0 - t));
        if *transition == Transition::Dissolve {
            new_alpha = t;
        }
        layers = layers.push(picture(app, old, old_off, 1.0));
    }
    if let Some(s) = &p.shot {
        layers = layers.push(picture(app, s, new_off, new_alpha));
    }
    if let Some((_, s)) = &p.overlay {
        layers = layers.push(container(space()).width(Fill).height(Fill).style(jd_ui::style::fill(Color::from_rgba(0.0, 0.0, 0.0, 0.4))));
        layers = layers.push(picture(app, s, Point::ORIGIN, 1.0));
    }
    let name = app.doc().node(p.frame).map_or(String::new(), |n| n.name.clone());
    layers = layers.push(
        column![
            row![text(name).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY), space::horizontal(), text("Esc to exit · ← → frames").size(TEXT_SIZE_SMALL).color(TEXT_MUTED)]
                .padding(12),
            space().height(Fill),
        ],
    );
    let cursor = if p.hovered.is_some() { iced::mouse::Interaction::Pointer } else { iced::mouse::Interaction::default() };
    Some(
        iced::widget::opaque(
            mouse_area(layers)
                .on_move(Message::PresentMove)
                .on_press(Message::PresentClick)
                .interaction(cursor),
        ),
    )
}
