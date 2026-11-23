//! The card beside a comment pin: the comment and its replies, a box to reply (or to write a
//! new comment), and Resolve / Delete.

use iced::alignment::Vertical;
use iced::widget::{button, column, container, mouse_area, row, space, text, text_input};
use iced::{Element, Fill, Padding, Point};
use jd_ui::tokens::*;
use jd_ui::{Icon, icon, style};

use crate::app::{Blobr, COMMENT_INPUT, Message, PIN_RADIUS};

const WIDTH: f32 = 280.0;

/// The card, placed in the window next to its pin (`canvas_at`: the canvas's top-left corner
/// in the window).
pub fn view(app: &Blobr, canvas_at: Point) -> Option<Element<'_, Message>> {
    let (pin, comment) = match (app.comment_new, app.comment_open) {
        (Some(p), _) => (app.camera.to_screen(p), None),
        (None, Some(id)) => {
            let c = app.doc().comments().get(&id)?;
            (app.camera.to_screen(kurbo::Point::new(c.x, c.y)), Some((id, c)))
        }
        _ => return None,
    };
    let small = |t: String, color| text(t).size(TEXT_SIZE_SMALL).color(color);
    let mut body = column![].spacing(8);
    if let Some((id, c)) = comment {
        let actions = row![
            small(if c.resolved { "Resolved".into() } else { "Comment".into() }, TEXT_SECONDARY),
            space::horizontal(),
            button(small(if c.resolved { "Reopen".into() } else { "Resolve".into() }, TEXT_PRIMARY))
                .padding(Padding::from([2, 8]))
                .style(style::ghost(false))
                .on_press(Message::CommentResolve(id, !c.resolved)),
            button(icon(Icon::Trash, 12.0, TEXT_SECONDARY)).padding(3).style(style::ghost(false)).on_press(Message::CommentDelete(id)),
            button(icon(Icon::Close, 10.0, TEXT_SECONDARY)).padding(4).style(style::ghost(false)).on_press(Message::CommentOpen(None)),
        ]
        .spacing(4)
        .align_y(Vertical::Center);
        body = body.push(actions).push(text(c.text.clone()).size(TEXT_SIZE).color(TEXT_PRIMARY));
        for r in &c.replies {
            body = body.push(container(small(r.clone(), TEXT_PRIMARY)).padding(Padding::ZERO.left(10)));
        }
    }
    let placeholder = if comment.is_some() { "Reply" } else { "Add a comment" };
    let input = text_input(placeholder, &app.comment_draft)
        .id(COMMENT_INPUT)
        .size(TEXT_SIZE_SMALL)
        .padding(6)
        .on_input(Message::CommentDraft)
        .on_submit(Message::CommentSubmit);
    let post = button(small(if comment.is_some() { "Reply".into() } else { "Post".into() }, TEXT_PRIMARY))
        .padding(Padding::from([5, 10]))
        .style(style::primary)
        .on_press(Message::CommentSubmit);
    body = body.push(row![input, post].spacing(6).align_y(Vertical::Center));

    let card = container(body).padding(12).width(WIDTH).style(|_| container::Style {
        background: Some(BG_PANEL.into()),
        border: iced::Border { radius: 6.0.into(), width: 1.0, color: BORDER_DEFAULT },
        shadow: iced::Shadow { color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.4), offset: iced::Vector::new(0.0, 4.0), blur_radius: 16.0 },
        ..Default::default()
    });
    // To the right of the pin, top-aligned with it; swallows clicks so the canvas doesn't
    // close it.
    let x = canvas_at.x + pin.x + PIN_RADIUS + 8.0;
    let y = (canvas_at.y + pin.y - PIN_RADIUS).max(canvas_at.y);
    Some(
        container(mouse_area(card).on_press(Message::Noop))
            .padding(Padding { top: y, left: x, right: 0.0, bottom: 0.0 })
            .width(Fill)
            .height(Fill)
            .into(),
    )
}
