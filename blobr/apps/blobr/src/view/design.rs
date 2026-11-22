//! The Design panel: properties of the selection. Typed values apply on Enter.

use iced::alignment::Vertical;
use iced::widget::{button, checkbox, column, container, pick_list, row, scrollable, space, text, text_input};
use iced::{Border, Color, Element, Fill, Padding, Theme};
use jd_ui::widget::{hline, panel_header};
use jd_ui::{Icon, icon, style, tokens::*};
use jd_vector::component::parse_variant;
use jd_vector::{Direction, ExportFormat, Node, NodeKind, Paint, PaintKind, PropDef, PropValue, Sizing};

use super::COMPONENT;
use crate::app::{Blobr, Field, Message};

pub fn view(app: &Blobr) -> Element<'_, Message> {
    let body: Element<'_, Message> = match app.selection.as_slice() {
        [] => page(app),
        [one] => match app.doc().node(*one) {
            Some(n) => node(app, n),
            None => space().into(),
        },
        many => column![
            section(format!("{} layers", many.len()), None),
            container(text("Edit one layer at a time for now.").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY)).padding(Padding::from([0, 12])),
        ]
        .into(),
    };
    column![panel_header(&["Design", "Prototype", "Inspect"], 0, |_| Message::Noop), scrollable(body).height(Fill)].into()
}

fn input_style(_: &Theme, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: BG_INPUT.into(),
        border: Border { radius: RADIUS.into(), width: 1.0, color: if focused { ACCENT_PRIMARY } else { BG_INPUT } },
        icon: ICON,
        placeholder: TEXT_MUTED,
        value: TEXT_PRIMARY,
        selection: BG_SELECTED,
    }
}

/// A labelled value box.
fn field<'a>(app: &Blobr, label: &'a str, f: Field, value: impl FnOnce() -> String) -> Element<'a, Message> {
    let current = app.draft(&f, value);
    let f2 = f.clone();
    row![
        text(label).size(10).color(TEXT_MUTED).width(if label.is_empty() { 0 } else { 14 }),
        text_input("", &current)
            .on_input(move |s| Message::FieldInput(f2.clone(), s))
            .on_submit(Message::FieldSubmit(f))
            .size(TEXT_SIZE_SMALL)
            .padding(Padding::from([3, 4]))
            .style(input_style),
    ]
    .spacing(4)
    .align_y(Vertical::Center)
    .width(Fill)
    .into()
}

fn num(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
}

fn section<'a>(title: impl text::IntoFragment<'a>, action: Option<(Icon, Message)>) -> Element<'a, Message> {
    let mut r = row![text(title).size(TEXT_SIZE_SMALL).font(jd_ui::widget::bold()).color(TEXT_PRIMARY), space::horizontal()]
        .align_y(Vertical::Center)
        .height(32);
    if let Some((i, m)) = action {
        r = r.push(button(icon(i, 12.0, ICON)).padding(4).style(style::ghost(false)).on_press(m));
    }
    container(r).padding(Padding::from([0, 12])).into()
}

fn body<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    column![container(content).padding(Padding { top: 0.0, right: 12.0, bottom: 12.0, left: 12.0 }), hline(BG_PANEL)].into()
}

fn pair<'a>(a: Element<'a, Message>, b: Element<'a, Message>) -> Element<'a, Message> {
    row![a, b].spacing(8).into()
}

fn page(app: &Blobr) -> Element<'_, Message> {
    let doc = app.doc();
    let bg = match doc.node(app.page).map(|n| &n.kind) {
        Some(NodeKind::Page { background }) => background.to_hex(),
        _ => String::new(),
    };
    column![
        section("Page", None),
        body(column![row![swatch(&bg), field(app, "", Field::PageBackground, || bg.clone())].spacing(8).align_y(Vertical::Center)]),
        section("Get started", None),
        body(
            text("F frame  ·  R rectangle  ·  O ellipse  ·  T text\nDrag to draw, click to select, Space or middle-drag to pan, Ctrl+scroll to zoom.")
                .size(TEXT_SIZE_SMALL)
                .color(TEXT_SECONDARY)
        ),
    ]
    .into()
}

fn swatch<'a>(hex: &str) -> Element<'a, Message> {
    let c = jd_core::Rgba::from_hex(hex).map_or(Color::TRANSPARENT, |c| {
        let [r, g, b] = c.to_srgb();
        Color::from_rgb(r, g, b)
    });
    container(space().width(14).height(14))
        .style(move |_| container::Style {
            background: Some(c.into()),
            border: Border { radius: 2.0.into(), width: 1.0, color: BORDER_SUBTLE },
            ..Default::default()
        })
        .into()
}

fn node<'a>(app: &'a Blobr, n: &'a Node) -> Element<'a, Message> {
    let doc = app.doc();
    let p = &n.props;
    let mut col = column![
        container(
            row![
                icon(super::kind_icon(&n.kind), 12.0, ICON),
                text(n.kind.type_name().replace('_', " ")).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY)
            ]
            .spacing(6)
            .align_y(Vertical::Center)
        )
        .padding(Padding { top: 10.0, right: 12.0, bottom: 4.0, left: 12.0 }),
        body(field(app, "", Field::Name, || n.name.clone())),
    ];

    if let NodeKind::Instance { component, .. } = &n.kind {
        col = col.push(instance(app, n.id, *component));
    }

    col = col.push(section("Layout", None));
    let mut layout = column![
        pair(field(app, "X", Field::X, || num(p.x)), field(app, "Y", Field::Y, || num(p.y))),
        pair(field(app, "W", Field::W, || num(p.width)), field(app, "H", Field::H, || num(p.height))),
        pair(field(app, "°", Field::Rotation, || num(p.rotation)), field(app, "R", Field::Radius, || num(p.corner.top_left))),
    ]
    .spacing(8);
    let parent_layout = n.parent.and_then(|pid| doc.node(pid)).is_some_and(|pn| pn.props.layout.is_some());
    if parent_layout || p.layout.is_some() || matches!(n.kind, NodeKind::Text { .. }) {
        let choices = [Sizing::Fixed, Sizing::Hug, Sizing::Fill];
        let names = |s: &Sizing| match s {
            Sizing::Fixed => "Fixed",
            Sizing::Hug => "Hug",
            Sizing::Fill => "Fill",
        };
        let picker = |horizontal: bool, current: Sizing| -> Element<'a, Message> {
            let opts: Vec<SizingChoice> = choices.iter().map(|s| SizingChoice(*s, names(s))).collect();
            pick_list(opts, Some(SizingChoice(current, names(&current))), move |c| Message::SetSizing { horizontal, sizing: c.0 })
                .text_size(TEXT_SIZE_SMALL)
                .padding(Padding::from([3, 6]))
                .style(style::pick_list)
                .width(Fill)
                .into()
        };
        layout = layout.push(pair(
            row![text("W").size(10).color(TEXT_MUTED).width(14), picker(true, p.sizing_h)].spacing(4).align_y(Vertical::Center).into(),
            row![text("H").size(10).color(TEXT_MUTED).width(14), picker(false, p.sizing_v)].spacing(4).align_y(Vertical::Center).into(),
        ));
    }
    col = col.push(body(layout));

    if n.kind.is_frame_like() {
        let on = p.layout.is_some();
        col = col.push(section("Auto layout", Some((if on { Icon::Minus } else { Icon::Plus }, Message::ToggleAutoLayout))));
        if let Some(l) = &p.layout {
            let dir = |d: Direction, i: Icon| {
                button(icon(i, 14.0, ICON)).padding(4).style(style::ghost(l.direction == d)).on_press(Message::SetDirection(d))
            };
            col = col.push(body(
                column![
                    row![dir(Direction::Horizontal, Icon::ChevronRight), dir(Direction::Vertical, Icon::Chevron), space::horizontal(), field(app, "Gap", Field::Gap, || num(l.gap))]
                        .spacing(2)
                        .align_y(Vertical::Center),
                    pair(field(app, "↔", Field::PadH, || num(l.padding.left)), field(app, "↕", Field::PadV, || num(l.padding.top))),
                ]
                .spacing(8),
            ));
        }
    }

    col = col.push(section("Appearance", None));
    col = col.push(body(field(app, "%", Field::Opacity, || num(p.opacity as f64 * 100.0))));

    if let NodeKind::Text { text: t } = &n.kind {
        col = col.push(section("Text", None));
        col = col.push(body(
            column![
                field(app, "", Field::Text, || t.content.replace('\n', "\\n")),
                pair(field(app, "Sz", Field::FontSize, || num(t.style.size as f64)), field(app, "", Field::FontStyle, || t.style.style.clone())),
                text(format!("Font: {}", if t.style.family.is_empty() { "system sans-serif" } else { &t.style.family }))
                    .size(10)
                    .color(TEXT_MUTED),
            ]
            .spacing(8),
        ));
    }

    if !matches!(n.kind, NodeKind::Group | NodeKind::ComponentSet { .. }) {
        col = col.push(section("Fill", Some((Icon::Plus, Message::AddFill))));
        col = col.push(body(paints(app, &p.fills, true)));
        col = col.push(section("Stroke", Some((Icon::Plus, Message::AddStroke))));
        let mut stroke = column![paints(app, &p.strokes, false)].spacing(8);
        if !p.strokes.is_empty() {
            stroke = stroke.push(field(app, "Wt", Field::StrokeWeight, || num(p.stroke.weight as f64)));
        }
        col = col.push(body(stroke));
    }

    col = col.push(section("Export", None));
    let export = |label: &'a str, f: ExportFormat| {
        button(text(label).size(TEXT_SIZE_SMALL)).padding(Padding::from([4, 10])).style(style::ghost(false)).on_press(Message::Export(f))
    };
    col = col.push(body(row![export("Export PNG", ExportFormat::Png), export("Export SVG", ExportFormat::Svg)].spacing(8)));
    col.into()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SizingChoice(Sizing, &'static str);

impl std::fmt::Display for SizingChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.1)
    }
}

fn paints<'a>(app: &Blobr, list: &[Paint], fills: bool) -> Element<'a, Message> {
    let mut col = column![].spacing(6);
    for (i, p) in list.iter().enumerate() {
        let hex = match &p.kind {
            PaintKind::Solid { color } => color.to_hex(),
            PaintKind::LinearGradient { .. } => "Linear gradient".into(),
            PaintKind::RadialGradient { .. } => "Radial gradient".into(),
            PaintKind::Image { .. } => "Image".into(),
        };
        let f = if fills { Field::Fill(i) } else { Field::Stroke(i) };
        let remove = if fills { Message::RemoveFill(i) } else { Message::RemoveStroke(i) };
        let shown = hex.clone();
        col = col.push(
            row![
                swatch(&hex),
                field(app, "", f, move || shown),
                text(format!("{:.0}%", p.opacity * 100.0)).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY),
                button(icon(Icon::Minus, 12.0, ICON)).padding(3).style(style::ghost(false)).on_press(remove),
            ]
            .spacing(8)
            .align_y(Vertical::Center),
        );
    }
    col.into()
}

fn instance<'a>(app: &'a Blobr, id: jd_vector::NodeId, component: jd_vector::NodeId) -> Element<'a, Message> {
    let doc = app.doc();
    let cname = doc.node(component).map_or("Missing component".into(), |c| c.name.clone());
    let mut col = column![
        row![icon(Icon::Instance, 12.0, COMPONENT), text(cname.clone()).size(TEXT_SIZE_SMALL).color(COMPONENT).font(jd_ui::widget::bold())]
            .spacing(6)
            .align_y(Vertical::Center),
    ]
    .spacing(8);

    // Variants: a picker per variant property.
    if let Some(set) = doc.component_set_of(component) {
        let current = parse_variant(&cname);
        for (prop, values) in doc.variant_properties(set) {
            let selected = current.get(&prop).cloned();
            let name = prop.clone();
            col = col.push(
                row![
                    text(prop).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(70),
                    pick_list(values, selected, move |v| Message::SetVariant(name.clone(), v))
                        .text_size(TEXT_SIZE_SMALL)
                        .padding(Padding::from([3, 6]))
                        .style(style::pick_list)
                        .width(Fill),
                ]
                .align_y(Vertical::Center),
            );
        }
    }
    for (name, def, value) in app.instance_props(id) {
        let label = text(name.clone()).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(70);
        let control: Element<'_, Message> = match (def, value) {
            (PropDef::Boolean { .. }, PropValue::Bool(b)) => {
                let n = name.clone();
                checkbox(b).on_toggle(move |v| Message::SetBool(n.clone(), v)).size(14).into()
            }
            (PropDef::Text { .. }, PropValue::Text(t)) => field(app, "", Field::Prop(name.clone()), move || t),
            _ => text("swap").size(TEXT_SIZE_SMALL).color(TEXT_MUTED).into(),
        };
        col = col.push(row![label, control].align_y(Vertical::Center));
    }
    let action = |label: &'a str, m: Message| button(text(label).size(TEXT_SIZE_SMALL)).padding(Padding::from([4, 10])).style(style::ghost(false)).on_press(m);
    col = col.push(row![action("Detach", Message::Detach), action("Reset", Message::ResetOverrides)].spacing(8));
    body(col)
}
