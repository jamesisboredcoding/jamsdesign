//! In-app modal dialogs.

use iced::alignment::Vertical;
use iced::widget::{button, center, column, container, opaque, row, space, text};
use iced::{Color, Element, Padding};
use jd_ui::widget::bold;
use jd_ui::{style, tokens::*};

use crate::app::{AfterConfirm, Choice, Message, Photo};

/// "Save changes?" before closing a tab or quitting. Blocks the UI underneath.
pub fn confirm(app: &Photo) -> Option<Element<'_, Message>> {
    let c = app.confirm?;
    let doc = app.docs.iter().find(|d| d.id == c.doc)?;
    let saving = doc.saving;

    let action = match c.then {
        AfterConfirm::CloseTab => "closing",
        AfterConfirm::Quit => "quitting",
    };
    let choice = |label: &'static str, ch: Choice, primary: bool| {
        let b = button(text(label).size(TEXT_SIZE).font(if primary { bold() } else { iced::Font::DEFAULT }))
            .padding(Padding::from([6, 14]))
            .on_press_maybe((!saving).then_some(Message::Confirmed(ch)));
        if primary { b.style(style::primary) } else { b.style(style::ghost(false)) }
    };

    let card = container(
        column![
            text(format!("Save changes to \"{}\" before {action}?", doc.doc.name))
                .size(14)
                .font(bold()),
            text(if saving { "Saving..." } else { "Your changes will be lost if you don't save them." })
                .size(TEXT_SIZE)
                .color(TEXT_SECONDARY),
            space().height(6),
            row![
                choice("Don't Save", Choice::Discard, false),
                space::horizontal(),
                choice("Cancel", Choice::Cancel, false),
                choice("Save", Choice::Save, true),
            ]
            .spacing(8)
            .align_y(Vertical::Center),
        ]
        .spacing(8)
        .width(420),
    )
    .padding(20)
    .style(|_| container::Style {
        background: Some(BG_PANEL.into()),
        border: iced::Border { color: BORDER_SUBTLE, width: 1.0, radius: 8.0.into() },
        shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.5), offset: iced::Vector::new(0.0, 8.0), blur_radius: 30.0 },
        ..Default::default()
    });

    Some(
        opaque(center(card).style(|_| container::Style {
            background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.45).into()),
            ..Default::default()
        }))
        .into(),
    )
}

/// Ctrl+N: preset sizes on the left, name / size / background on the right.
pub fn new_document(app: &Photo) -> Option<Element<'_, Message>> {
    use crate::app::{NewBackground, PRESETS};
    use iced::widget::{scrollable, text_input};
    let f = app.new_doc.as_ref()?;

    let mut presets = column![].spacing(2);
    let mut group = "";
    for (i, &(g, name, w, h)) in PRESETS.iter().enumerate() {
        if g != group {
            group = g;
            presets = presets.push(
                container(text(g).size(TEXT_SIZE_SMALL).color(TEXT_MUTED)).padding(Padding::ZERO.top(10).bottom(4).left(8)),
            );
        }
        presets = presets.push(
            button(
                row![
                    text(name).size(TEXT_SIZE).width(iced::Fill),
                    text(format!("{w} × {h}")).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY),
                ]
                .align_y(Vertical::Center),
            )
            .width(iced::Fill)
            .padding(Padding::from([6, 8]))
            .style(style::row(f.preset == Some(i)))
            .on_press(Message::NewDocPreset(i)),
        );
    }

    let field = |label: &'static str, value: &str, on: fn(String) -> Message| {
        column![
            text(label).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY),
            text_input("", value).size(TEXT_SIZE).padding(Padding::from([5, 8])).on_input(on).on_submit(Message::CreateNewDoc),
        ]
        .spacing(4)
    };
    let bg_choice = |label: &'static str, b: NewBackground| {
        button(text(label).size(TEXT_SIZE_SMALL))
            .padding(Padding::from([5, 10]))
            .style(style::ghost(f.background == b))
            .on_press(Message::NewDocBackground(b))
    };
    let valid = f.size();
    let summary = match valid {
        Some((w, h)) => {
            let mb = w as f64 * h as f64 * 4.0 / 1_000_000.0;
            format!("{w} × {h} px · {:.1} megapixels · about {mb:.0} MB per full layer", w as f64 * h as f64 / 1e6)
        }
        None => "Width and height must be 1 to 30000 px".into(),
    };

    let form = column![
        field("Name", &f.name, Message::NewDocName),
        row![
            field("Width (px)", &f.width, Message::NewDocWidth),
            field("Height (px)", &f.height, Message::NewDocHeight),
        ]
        .spacing(10),
        column![
            text("Background").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY),
            row![
                bg_choice("White", NewBackground::White),
                bg_choice("Transparent", NewBackground::Transparent),
                bg_choice("Background color", NewBackground::BackgroundColor),
            ]
            .spacing(6),
        ]
        .spacing(4),
        text(summary).size(TEXT_SIZE_SMALL).color(if valid.is_some() { TEXT_MUTED } else { DANGER }),
        space::vertical(),
        row![
            space::horizontal(),
            button(text("Cancel").size(TEXT_SIZE)).padding(Padding::from([6, 14])).style(style::ghost(false)).on_press(Message::CancelNewDoc),
            button(text("Create").size(TEXT_SIZE).font(bold()))
                .padding(Padding::from([6, 14]))
                .style(style::primary)
                .on_press_maybe(valid.map(|_| Message::CreateNewDoc)),
        ]
        .spacing(8),
    ]
    .spacing(14)
    .width(iced::Fill)
    .height(iced::Fill);

    let card = container(
        column![
            text("New Document").size(16).font(bold()),
            row![
                // Right padding keeps the sizes clear of the scrollbar.
                container(scrollable(container(presets).padding(Padding::from([6, 14]).left(6))))
                    .width(280)
                    .height(iced::Fill)
                    .style(|_| container::Style {
                        background: Some(BG_PANEL_HEADER.into()),
                        border: iced::Border { radius: 6.0.into(), ..Default::default() },
                        ..Default::default()
                    }),
                form,
            ]
            .spacing(20)
            .height(iced::Fill),
        ]
        .spacing(16),
    )
    .width(720)
    .height(500)
    .padding(24)
    .style(card_style);

    Some(opaque(center(card).style(scrim)).into())
}

fn card_style(_: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(BG_PANEL.into()),
        border: iced::Border { color: BORDER_SUBTLE, width: 1.0, radius: 8.0.into() },
        shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.5), offset: iced::Vector::new(0.0, 8.0), blur_radius: 30.0 },
        ..Default::default()
    }
}

fn scrim(_: &iced::Theme) -> container::Style {
    container::Style { background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.45).into()), ..Default::default() }
}

/// Image Size, Canvas Size and the one-value Select/Edit dialogs.
pub fn small(app: &Photo) -> Option<Element<'_, Message>> {
    use crate::app::DialogKind;
    use iced::widget::text_input;
    let d = app.dialog.as_ref()?;
    let field = |label: &'static str, value: &str, on: fn(String) -> Message| {
        column![
            text(label).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY),
            text_input("", value).size(TEXT_SIZE).padding(Padding::from([5, 8])).on_input(on).on_submit(Message::DialogOk),
        ]
        .spacing(4)
    };
    let (title, body): (&str, Element<'_, Message>) = match d.kind {
        DialogKind::ImageSize => (
            "Image Size",
            column![
                row![field("Width (px)", &d.a, Message::DialogA), field("Height (px)", &d.b, Message::DialogB)].spacing(10),
                button(text(if d.constrain { "Keep proportions: on" } else { "Keep proportions: off" }).size(TEXT_SIZE_SMALL))
                    .padding(Padding::from([4, 10]))
                    .style(style::ghost(d.constrain))
                    .on_press(Message::DialogConstrain),
                text("Resamples every layer (smooth averaging when shrinking).").size(TEXT_SIZE_SMALL).color(TEXT_MUTED),
            ]
            .spacing(10)
            .into(),
        ),
        DialogKind::CanvasSize => {
            let mut grid = column![].spacing(3);
            for cy in 0..3u8 {
                let mut r = row![].spacing(3);
                for cx in 0..3u8 {
                    let on = d.anchor == (cx, cy);
                    r = r.push(
                        button(space().width(14).height(14))
                            .padding(0)
                            .style(move |_, status| button::Style {
                                background: Some(
                                    if on { ACCENT_PRIMARY } else if matches!(status, button::Status::Hovered) { BG_HOVER } else { BG_INPUT }.into(),
                                ),
                                border: iced::Border { color: BORDER_SUBTLE, width: 1.0, radius: 2.0.into() },
                                ..Default::default()
                            })
                            .on_press(Message::DialogAnchor(cx, cy)),
                    );
                }
                grid = grid.push(r);
            }
            (
                "Canvas Size",
                column![
                    row![field("Width (px)", &d.a, Message::DialogA), field("Height (px)", &d.b, Message::DialogB)].spacing(10),
                    row![text("Anchor").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(60), grid].spacing(10),
                    text("New area on the Background is filled with the background colour.").size(TEXT_SIZE_SMALL).color(TEXT_MUTED),
                ]
                .spacing(10)
                .into(),
            )
        }
        kind => {
            let (title, label) = match kind {
                DialogKind::Expand => ("Expand Selection", "Expand by (px)"),
                DialogKind::Contract => ("Contract Selection", "Contract by (px)"),
                DialogKind::Feather => ("Feather Selection", "Feather radius (px)"),
                _ => ("Stroke", "Width (px), in the foreground colour, centred on the edge"),
            };
            (title, field(label, &d.a, Message::DialogA).into())
        }
    };
    let card = container(
        column![
            text(title).size(15).font(bold()),
            body,
            row![
                space::horizontal(),
                button(text("Cancel").size(TEXT_SIZE)).padding(Padding::from([6, 14])).style(style::ghost(false)).on_press(Message::DialogCancel),
                button(text("OK").size(TEXT_SIZE).font(bold())).padding(Padding::from([6, 18])).style(style::primary).on_press(Message::DialogOk),
            ]
            .spacing(8),
        ]
        .spacing(16)
        .width(360),
    )
    .padding(20)
    .style(card_style);
    Some(opaque(center(card).style(scrim)).into())
}
