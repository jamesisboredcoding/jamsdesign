//! Edit > Preferences (Ctrl+K): sections on the left, their settings on the right. Changes
//! apply straight away; OK keeps them (and saves them), Cancel puts the old ones back.

use iced::alignment::Vertical;
use iced::widget::{button, column, container, mouse_area, opaque, pick_list, pin, row, slider, space, text};
use iced::{Element, Fill, Padding};
use jd_ui::widget::{bold, label};
use jd_ui::{style, tokens::*};

use crate::app::{Message, Photo};
use crate::prefs::{BrushCursor, HISTORY_RANGE, Prefs};

const SECTIONS: [&str; 3] = ["General", "Tools", "Background Removal"];

impl std::fmt::Display for BrushCursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// A check box row that flips one preference.
fn check<'a>(name: &'a str, hint: &'a str, on: bool, p: &Prefs, set: fn(&mut Prefs, bool)) -> Element<'a, Message> {
    let mut next = p.clone();
    set(&mut next, !on);
    let mark = container(text(if on { "✓" } else { "" }).size(11)).center(16).style(move |_| container::Style {
        background: Some(if on { ACCENT_PRIMARY } else { BG_INPUT }.into()),
        text_color: Some(TEXT_PRIMARY),
        border: iced::Border { color: BORDER_DEFAULT, width: 1.0, radius: 3.0.into() },
        ..Default::default()
    });
    button(
        row![mark, column![text(name).size(TEXT_SIZE), text(hint).size(TEXT_SIZE_SMALL).color(TEXT_MUTED)].spacing(2)]
            .spacing(10)
            .align_y(Vertical::Center),
    )
    .width(Fill)
    .padding(Padding::from([4, 4]))
    .style(style::ghost(false))
    .on_press(Message::PrefsSet(Box::new(next)))
    .into()
}

fn general<'a>(app: &'a Photo, p: &Prefs) -> Element<'a, Message> {
    let history = p.history_states as f32;
    let set_history = {
        let p = p.clone();
        move |v: f32| {
            let mut n = p.clone();
            n.history_states = v.round() as u32;
            Message::PrefsSet(Box::new(n))
        }
    };
    column![
        text("History States").size(TEXT_SIZE).font(bold()),
        row![
            slider(HISTORY_RANGE, history, set_history.clone()).step(1.0f32).width(Fill).style(style::slider),
            super::field::number(app, "prefs-history", format!("{history:.0}"), 60, 1.0, HISTORY_RANGE, set_history),
        ]
        .spacing(10)
        .align_y(Vertical::Center),
        label("Undo steps kept per document. More steps use more memory on big images."),
        space().height(4),
        check(
            "Blend new images in linear light",
            "Off: like Photoshop (perceptual). Saved documents keep their own setting.",
            p.linear_blending,
            p,
            |p, v| p.linear_blending = v,
        ),
        check(
            "Remember the workspace and panel layout",
            "Panels, their order and sizes, and the dock width come back next time.",
            p.remember_layout,
            p,
            |p, v| p.remember_layout = v,
        ),
    ]
    .spacing(8)
    .into()
}

fn tools<'a>(p: &Prefs) -> Element<'a, Message> {
    let cursor = {
        let p = p.clone();
        pick_list(BrushCursor::ALL, Some(p.brush_cursor), move |c| {
            let mut n = p.clone();
            n.brush_cursor = c;
            Message::PrefsSet(Box::new(n))
        })
        .text_size(TEXT_SIZE_SMALL)
        .padding(Padding::from([4, 8]))
        .width(240)
        .style(style::pick_list)
    };
    column![
        check(
            "Zoom with the scroll wheel",
            "Off: the wheel scrolls (Shift: sideways) and Ctrl+wheel zooms.",
            p.wheel_zooms,
            p,
            |p, v| p.wheel_zooms = v,
        ),
        check("Pen pressure", "Tablet pressure changes brush size (and opacity if set in the options bar).", p.pen_pressure, p, |p, v| {
            p.pen_pressure = v
        }),
        check("Snap at startup", "View > Snap starts switched on.", p.snap, p, |p, v| p.snap = v),
        space().height(4),
        row![text("Painting cursor").size(TEXT_SIZE).width(Fill), cursor].align_y(Vertical::Center),
    ]
    .spacing(8)
    .into()
}

fn background_removal<'a>() -> Element<'a, Message> {
    let model = &jd_ai::ISNET;
    let installed = model.is_installed();
    column![
        text("Remove Background model").size(TEXT_SIZE).font(bold()),
        label(if installed {
            format!("IS-Net ({} MB), downloaded. It runs on this computer; images are never uploaded.", model.size / 1_000_000)
        } else {
            format!("IS-Net ({} MB), not downloaded. Image > Remove Background asks before downloading it.", model.size / 1_000_000)
        }),
        text(format!("Kept in {}", jd_ai::models_dir().display())).size(TEXT_SIZE_SMALL).color(TEXT_MUTED),
        button(text("Delete Downloaded Model").size(TEXT_SIZE_SMALL))
            .padding(Padding::from([5, 10]))
            .style(style::ghost(false))
            .on_press_maybe(installed.then_some(Message::DeleteAiModel)),
    ]
    .spacing(8)
    .into()
}

pub fn view(app: &Photo) -> Option<Element<'_, Message>> {
    let d = app.prefs_dialog.as_ref()?;
    let p = &app.prefs;
    let mut list = column![].spacing(2).width(170);
    for (i, name) in SECTIONS.iter().enumerate() {
        list = list.push(
            button(text(*name).size(TEXT_SIZE))
                .width(Fill)
                .padding(Padding::from([6, 10]))
                .style(style::row(d.tab == i as u8))
                .on_press(Message::PrefsTab(i as u8)),
        );
    }
    let body = match d.tab {
        1 => tools(p),
        2 => background_removal(),
        _ => general(app, p),
    };
    let card = container(
        column![
            mouse_area(row![text("Preferences").size(15).font(bold()), space::horizontal()])
                .on_move(Message::DialogTitlePointer)
                .on_press(Message::DialogDragStart)
                .interaction(iced::mouse::Interaction::Grab),
            row![container(list).padding(4).style(style::input), container(body).width(Fill)].spacing(16).height(300),
            row![
                space::horizontal(),
                button(text("Cancel").size(TEXT_SIZE)).padding(Padding::from([6, 14])).style(style::ghost(false)).on_press(Message::PrefsCancel),
                button(text("OK").size(TEXT_SIZE).font(bold())).padding(Padding::from([6, 18])).style(style::primary).on_press(Message::PrefsOk),
            ]
            .spacing(8),
        ]
        .spacing(16)
        .width(640),
    )
    .padding(20)
    .style(super::dialog::card_style);
    Some(opaque(container(pin(card).x(d.pos.x).y(d.pos.y)).width(Fill).height(Fill).style(super::dialog::scrim)).into())
}
