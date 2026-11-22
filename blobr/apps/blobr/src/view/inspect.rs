//! The Inspect tab: a layer's measurements, colours and type, and its code (CSS, HTML for a
//! subtree, iced) with copy buttons. While it's open, hovering shows distances (no Alt).

use iced::alignment::Vertical;
use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Element, Fill, Font, Padding};
use jd_ui::widget::{bold, hline};
use jd_ui::{style, tokens::*};
use jd_vector::{NodeKind, PaintKind};

use super::design::swatch;
use crate::app::{Blobr, Message};

fn kv<'a>(k: &'a str, v: String) -> Element<'a, Message> {
    row![text(k).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(90), text(v).size(TEXT_SIZE_SMALL).color(TEXT_PRIMARY)].into()
}

fn num(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
}

fn code<'a>(title: &'a str, body: String) -> Element<'a, Message> {
    let copy = button(text("Copy").size(10)).padding(Padding::from([2, 8])).style(style::ghost(false)).on_press(Message::CopyText(body.clone()));
    column![
        row![text(title).size(TEXT_SIZE_SMALL).font(bold()).color(TEXT_PRIMARY), space::horizontal(), copy].align_y(Vertical::Center),
        container(text(body).size(11).font(Font::MONOSPACE).color(TEXT_PRIMARY)).padding(8).width(Fill).style(style::fill(BG_CANVAS)),
    ]
    .spacing(6)
    .into()
}

pub fn view(app: &Blobr) -> Element<'_, Message> {
    let [id] = app.selection[..] else {
        let msg = if app.selection.is_empty() { "Select a layer to see its measurements and code. Hover other layers to see distances." } else { "Select one layer to inspect it." };
        return container(text(msg).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY)).padding(12).into();
    };
    let doc = app.doc();
    let Some(n) = doc.node(id) else { return space().into() };
    let p = &n.props;
    let mut col = column![
        text(n.name.clone()).size(TEXT_SIZE).font(bold()).color(TEXT_PRIMARY),
        kv("Type", n.kind.type_name().replace('_', " ")),
        kv("Position", format!("{}, {}", num(p.x), num(p.y))),
        kv("Size", format!("{} × {}", num(p.width), num(p.height))),
    ]
    .spacing(6);
    if p.rotation != 0.0 {
        col = col.push(kv("Rotation", format!("{}°", num(p.rotation))));
    }
    if p.corner.top_left != 0.0 {
        col = col.push(kv("Radius", num(p.corner.top_left)));
    }
    if p.opacity < 1.0 {
        col = col.push(kv("Opacity", format!("{:.0}%", p.opacity * 100.0)));
    }
    if let Some(l) = &p.layout {
        col = col.push(kv("Auto layout", format!("{:?}, gap {}", l.direction, num(l.gap))));
        col = col.push(kv("Padding", format!("{} {} {} {}", num(l.padding.top), num(l.padding.right), num(l.padding.bottom), num(l.padding.left))));
    }
    for (i, f) in p.fills.iter().enumerate() {
        if let PaintKind::Solid { color } = f.kind {
            let hex = color.to_hex();
            let var = p.bind.get(&format!("fills/{i}")).and_then(|v| doc.tokens().variables.get(v)).map(|v| format!("  ({})", v.name));
            col = col.push(
                row![text("Fill").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(90), swatch(&hex), text(format!("#{hex}{}", var.unwrap_or_default())).size(TEXT_SIZE_SMALL)]
                    .spacing(6)
                    .align_y(Vertical::Center),
            );
        }
    }
    if let NodeKind::Text { text: t } = &n.kind {
        col = col.push(kv("Font", format!("{} {} {}", if t.style.family.is_empty() { "sans-serif" } else { &t.style.family }, t.style.style, num(t.style.size as f64))));
    }
    col = col.push(hline(BG_PANEL));
    col = col.push(code("CSS", jd_vector::inspect::css(doc, id).join("\n")));
    if !doc.children(id).is_empty() {
        col = col.push(code("HTML", jd_vector::inspect::html(doc, id)));
    }
    col = col.push(code("Rust (iced)", jd_vector::inspect::iced(doc, id)));
    if !doc.tokens().variables.is_empty() {
        col = col.push(code("Variables as Rust constants", jd_vector::inspect::rust_tokens(doc)));
    }
    scrollable(container(col).padding(12)).height(Fill).into()
}
