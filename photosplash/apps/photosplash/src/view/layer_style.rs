//! The Layer Style dialog (Blending Options): the effect list on the left, the selected
//! effect's settings on the right. Every change previews on the canvas straight away.

use iced::alignment::Vertical;
use iced::widget::{button, column, container, pick_list, row, slider, space, text};
use iced::{Element, Fill, Padding};
use jd_raster::blend::BlendMode;
use jd_raster::style::{LayerStyle, StrokePosition};
use jd_ui::style;
use jd_ui::tokens::*;
use jd_ui::widget::bold;

use crate::app::{Dialog, Message, Photo};

/// Left-hand list: Blending Options, then the effects in Photoshop's order.
const TABS: [&str; 5] = ["Blending Options", "Stroke", "Color Overlay", "Outer Glow", "Drop Shadow"];

fn is_on(s: &LayerStyle, tab: u8) -> Option<bool> {
    match tab {
        1 => Some(s.stroke.on),
        2 => Some(s.color_overlay.on),
        3 => Some(s.outer_glow.on),
        4 => Some(s.drop_shadow.on),
        _ => None,
    }
}

fn set_on(mut s: LayerStyle, tab: u8, on: bool) -> LayerStyle {
    match tab {
        1 => s.stroke.on = on,
        2 => s.color_overlay.on = on,
        3 => s.outer_glow.on = on,
        4 => s.drop_shadow.on = on,
        _ => {}
    }
    s
}

/// A labelled slider row that edits one field of the style.
fn number<'a>(
    label: &'a str,
    value: f32,
    range: std::ops::RangeInclusive<f32>,
    unit: &'a str,
    style_now: LayerStyle,
    set: fn(&mut LayerStyle, f32),
) -> Element<'a, Message> {
    let shown = if unit == "%" { format!("{:.0} %", value * 100.0) } else { format!("{value:.0} {unit}") };
    row![
        text(label).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(70),
        slider(range, value, move |v| {
            let mut s = style_now;
            set(&mut s, v);
            Message::DialogStyle(s)
        })
        .step(if unit == "%" { 0.01f32 } else { 1.0 })
        .width(Fill)
        .style(style::slider),
        text(shown).size(TEXT_SIZE_SMALL).width(52),
    ]
    .spacing(10)
    .align_y(Vertical::Center)
    .into()
}

/// Colour swatch plus buttons to take the foreground or background colour.
fn colour<'a>(app: &Photo, value: [u8; 3], style_now: LayerStyle, set: fn(&mut LayerStyle, [u8; 3])) -> Element<'a, Message> {
    let swatch = container(space().width(36).height(20)).style(move |_| container::Style {
        background: Some(iced::Color::from_rgb8(value[0], value[1], value[2]).into()),
        border: iced::Border { color: BORDER_DEFAULT, width: 1.0, radius: 3.0.into() },
        ..Default::default()
    });
    let pick = |label: &'a str, c: [u8; 3]| {
        let mut s = style_now;
        set(&mut s, c);
        button(text(label).size(TEXT_SIZE_SMALL)).padding(Padding::from([3, 8])).style(style::ghost(false)).on_press(Message::DialogStyle(s))
    };
    let (fg, bg) = (app.fg.to_rgba().to_srgb8(), app.bg.to_rgba().to_srgb8());
    row![
        text("Color").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(70),
        swatch,
        pick("Use Foreground", [fg[0], fg[1], fg[2]]),
        pick("Use Background", [bg[0], bg[1], bg[2]]),
    ]
    .spacing(8)
    .align_y(Vertical::Center)
    .into()
}

pub fn body<'a>(app: &'a Photo, d: &'a Dialog) -> Element<'a, Message> {
    let Some(orig) = d.original else { return space().into() };
    let Some(layer) = app.document().and_then(|doc| doc.layer(orig.id)) else { return space().into() };
    let s = layer.style;

    let mut list = column![].spacing(2).width(170);
    for (i, name) in TABS.iter().enumerate() {
        let tab = i as u8;
        let selected = d.fx_tab == tab;
        let mut r = row![].spacing(8).align_y(Vertical::Center);
        if let Some(on) = is_on(&s, tab) {
            let mark = container(text(if on { "✓" } else { "" }).size(11)).center(14).style(move |_| container::Style {
                background: Some(if on { ACCENT_PRIMARY } else { BG_INPUT }.into()),
                text_color: Some(TEXT_PRIMARY),
                border: iced::Border { color: BORDER_DEFAULT, width: 1.0, radius: 2.0.into() },
                ..Default::default()
            });
            r = r.push(button(mark).padding(0).style(style::ghost(false)).on_press(Message::DialogStyle(set_on(s, tab, !on))));
        } else {
            r = r.push(space().width(14));
        }
        r = r.push(text(*name).size(TEXT_SIZE).font(if tab == 0 { bold() } else { iced::Font::default() }));
        list = list.push(
            button(r)
                .width(Fill)
                .padding(Padding::from([5, 8]))
                .style(style::row(selected))
                // Clicking an effect's name shows its settings and switches it on, like Photoshop.
                .on_press(Message::DialogFxTab(tab, (tab != 0).then(|| set_on(s, tab, true)))),
        );
    }

    let heading = |t: &'a str| text(t).size(TEXT_SIZE).font(bold());
    let settings: Element<'a, Message> = match d.fx_tab {
        1 => column![
            heading("Stroke"),
            number("Size", s.stroke.size, 1.0..=100.0, "px", s, |s, v| s.stroke.size = v),
            row![
                text("Position").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(70),
                pick_list(StrokePosition::ALL, Some(s.stroke.position), move |p| {
                    let mut n = s;
                    n.stroke.position = p;
                    Message::DialogStyle(n)
                })
                .text_size(TEXT_SIZE_SMALL)
                .padding(Padding::from([4, 8]))
                .width(130)
                .style(style::pick_list),
            ]
            .spacing(10)
            .align_y(Vertical::Center),
            number("Opacity", s.stroke.opacity, 0.0..=1.0, "%", s, |s, v| s.stroke.opacity = v),
            colour(app, s.stroke.color, s, |s, c| s.stroke.color = c),
        ]
        .spacing(12)
        .into(),
        2 => column![
            heading("Color Overlay"),
            number("Opacity", s.color_overlay.opacity, 0.0..=1.0, "%", s, |s, v| s.color_overlay.opacity = v),
            colour(app, s.color_overlay.color, s, |s, c| s.color_overlay.color = c),
        ]
        .spacing(12)
        .into(),
        3 => column![
            heading("Outer Glow"),
            number("Opacity", s.outer_glow.opacity, 0.0..=1.0, "%", s, |s, v| s.outer_glow.opacity = v),
            number("Spread", s.outer_glow.spread, 0.0..=1.0, "%", s, |s, v| s.outer_glow.spread = v),
            number("Size", s.outer_glow.size, 0.0..=150.0, "px", s, |s, v| s.outer_glow.size = v),
            colour(app, s.outer_glow.color, s, |s, c| s.outer_glow.color = c),
        ]
        .spacing(12)
        .into(),
        4 => column![
            heading("Drop Shadow"),
            number("Opacity", s.drop_shadow.opacity, 0.0..=1.0, "%", s, |s, v| s.drop_shadow.opacity = v),
            number("Angle", s.drop_shadow.angle, -180.0..=180.0, "°", s, |s, v| s.drop_shadow.angle = v),
            number("Distance", s.drop_shadow.distance, 0.0..=150.0, "px", s, |s, v| s.drop_shadow.distance = v),
            number("Spread", s.drop_shadow.spread, 0.0..=1.0, "%", s, |s, v| s.drop_shadow.spread = v),
            number("Size", s.drop_shadow.size, 0.0..=150.0, "px", s, |s, v| s.drop_shadow.size = v),
            colour(app, s.drop_shadow.color, s, |s, c| s.drop_shadow.color = c),
        ]
        .spacing(12)
        .into(),
        _ => {
            let (mode, opacity) = (layer.blend, layer.opacity);
            column![
                heading("General Blending"),
                row![
                    text("Blend Mode").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(70),
                    pick_list(BlendMode::ALL, Some(mode), Message::DialogBlend)
                        .text_size(TEXT_SIZE_SMALL)
                        .padding(Padding::from([4, 8]))
                        .width(Fill)
                        .style(style::pick_list),
                ]
                .spacing(10)
                .align_y(Vertical::Center),
                row![
                    text("Opacity").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(70),
                    slider(0.0..=1.0, opacity, move |o| Message::DialogOpacity(o, mode)).step(0.01f32).width(Fill).style(style::slider),
                    text(format!("{:.0} %", opacity * 100.0)).size(TEXT_SIZE_SMALL).width(52),
                ]
                .spacing(10)
                .align_y(Vertical::Center),
                text("Pick an effect on the left to add it. Changes show on the canvas as you make them.")
                    .size(TEXT_SIZE_SMALL)
                    .color(TEXT_MUTED),
            ]
            .spacing(12)
            .into()
        }
    };

    row![
        container(list).padding(4).style(style::input),
        container(settings).width(Fill).padding(Padding::from([4, 4])),
    ]
    .spacing(16)
    .height(260)
    .into()
}
