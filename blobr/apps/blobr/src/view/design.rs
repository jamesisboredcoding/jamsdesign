//! The Design panel: properties of the selection. Typed values apply on Enter.

use iced::alignment::Vertical;
use iced::widget::{button, checkbox, column, container, pick_list, row, scrollable, space, text, text_input};
use iced::{Border, Color, Element, Fill, Padding, Theme};
use jd_ui::widget::{hline, panel_header};
use jd_ui::{Icon, icon, style, tokens::*};
use jd_vector::component::parse_variant;
use jd_vector::{Direction, ExportFormat, Node, NodeKind, Paint, PaintKind, PropDef, PropValue, Sizing};

use super::COMPONENT;
use crate::app::{Blobr, Field, Message, PropLink};

/// A choice in a pick list: an id and what to show.
#[derive(Debug, Clone, PartialEq)]
struct Choice<T>(T, String);

impl<T> std::fmt::Display for Choice<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.1)
    }
}

fn picker<'a, T: Clone + PartialEq + 'a>(options: Vec<Choice<T>>, selected: Option<Choice<T>>, on: impl Fn(T) -> Message + 'a) -> Element<'a, Message> {
    pick_list(options, selected, move |c: Choice<T>| on(c.0))
        .text_size(TEXT_SIZE_SMALL)
        .padding(Padding::from([3, 6]))
        .style(style::pick_list)
        .width(Fill)
        .into()
}

/// The selected component's properties: add, remove, defaults.
fn component_props<'a>(app: &'a Blobr, n: &'a Node) -> Option<Element<'a, Message>> {
    let NodeKind::Component { properties, .. } = &n.kind else { return None };
    let mut col = column![].spacing(6);
    for (name, def) in properties {
        let control: Element<'_, Message> = match def {
            PropDef::Text { default } => field(app, "", Field::PropDefault(name.clone()), || default.clone()),
            PropDef::Boolean { default } => {
                let nm = name.clone();
                checkbox(*default).on_toggle(move |b| Message::SetPropertyDefault(nm.clone(), PropValue::Bool(b))).size(14).into()
            }
            PropDef::InstanceSwap { default } => {
                let nm = name.clone();
                let opts = swap_choices(app);
                let cur = opts.iter().find(|c| c.0 == *default).cloned();
                picker(opts, cur, move |c| Message::SetPropertyDefault(nm.clone(), PropValue::Node(c)))
            }
        };
        col = col.push(
            row![
                text(name.clone()).size(TEXT_SIZE_SMALL).color(COMPONENT).width(70),
                control,
                button(icon(Icon::Minus, 12.0, ICON)).padding(3).style(style::ghost(false)).on_press(Message::RemoveProperty(name.clone())),
            ]
            .spacing(6)
            .align_y(Vertical::Center),
        );
    }
    let add = |label: &'static str, kind: &'static str| {
        button(text(label).size(TEXT_SIZE_SMALL)).padding(Padding::from([3, 8])).style(style::ghost(false)).on_press(Message::AddProperty(kind))
    };
    col = col.push(row![add("+ Text", "text"), add("+ Boolean", "boolean"), add("+ Swap", "swap")].spacing(6));
    if properties.is_empty() {
        col = col.push(text("Then link layers inside the component to them.").size(10).color(TEXT_MUTED));
    }
    Some(column![section("Properties", None), body(col)].into())
}

/// For a layer inside a component: which component properties drive it.
fn property_links<'a>(app: &'a Blobr, n: &'a Node) -> Option<Element<'a, Message>> {
    let owner = app.owning_component(n.id).filter(|&c| c != n.id)?;
    let NodeKind::Component { properties, .. } = &app.doc().node(owner)?.kind else { return None };
    let of = |want: fn(&PropDef) -> bool| -> Vec<Choice<Option<String>>> {
        std::iter::once(Choice(None, "None".into()))
            .chain(properties.iter().filter(|(_, d)| want(d)).map(|(k, _)| Choice(Some(k.clone()), k.clone())))
            .collect()
    };
    let current = |v: &Option<String>| Some(Choice(v.clone(), v.clone().unwrap_or_else(|| "None".into())));
    let line = |label: &'static str, opts: Vec<Choice<Option<String>>>, cur: &Option<String>, link: PropLink| {
        row![text(label).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(70), picker(opts, current(cur), move |p| Message::LinkProperty(link, p))]
            .align_y(Vertical::Center)
    };
    let refs = &n.props.refs;
    let mut col = column![line("Visible", of(|d| matches!(d, PropDef::Boolean { .. })), &refs.visible, PropLink::Visible)].spacing(6);
    if matches!(n.kind, NodeKind::Text { .. }) {
        col = col.push(line("Text", of(|d| matches!(d, PropDef::Text { .. })), &refs.text, PropLink::Text));
    }
    if matches!(n.kind, NodeKind::Instance { .. }) {
        col = col.push(line("Swap", of(|d| matches!(d, PropDef::InstanceSwap { .. })), &refs.swap, PropLink::Swap));
    }
    let name = app.doc().node(owner).map_or(String::new(), |c| c.name.clone());
    Some(column![section(format!("Component property ({name})"), None), body(col)].into())
}

pub fn view(app: &Blobr) -> Element<'_, Message> {
    let body: Element<'_, Message> = match app.selection.as_slice() {
        [] => page(app),
        [first, ..] => match app.doc().node(*first) {
            Some(n) => node(app, n),
            None => space().into(),
        },
    };
    let header = panel_header(&["Design", "Prototype", "Inspect"], app.right_tab, Message::RightTab);
    match app.right_tab {
        1 => column![header, super::prototype::view(app)].into(),
        2 => column![header, super::inspect::view(app)].into(),
        _ => column![header, scrollable(body).height(Fill)].into(),
    }
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
pub(super) fn field<'a>(app: &Blobr, label: &'a str, f: Field, value: impl FnOnce() -> String) -> Element<'a, Message> {
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

/// A value shown for the whole selection: the shared one, or "Mixed".
fn shared(app: &Blobr, f: impl Fn(&Node) -> String) -> String {
    let mut values = app.selection.iter().filter_map(|&id| app.doc().node(id)).map(f);
    let Some(first) = values.next() else { return String::new() };
    if values.all(|v| v == first) { first } else { "Mixed".into() }
}

/// Whether every selected layer passes `f`.
fn all(app: &Blobr, f: impl Fn(&Node) -> bool) -> bool {
    app.selection.iter().filter_map(|&id| app.doc().node(id)).all(f)
}

/// The align buttons (and distribute, with three or more layers).
fn arrange(app: &Blobr) -> Element<'_, Message> {
    use jd_vector::arrange::Align;
    let b = |i: Icon, tip: &'static str, m: Message| jd_ui::widget::hover_tip(button(icon(i, 14.0, ICON)).padding(4).style(style::ghost(false)).on_press(m), tip, iced::widget::tooltip::Position::Bottom);
    let mut r = row![
        b(Icon::AlignLeft, "Align left (Alt+A)", Message::Align(Align::Left)),
        b(Icon::AlignCenter, "Align horizontal centers (Alt+H)", Message::Align(Align::Center)),
        b(Icon::AlignRight, "Align right (Alt+D)", Message::Align(Align::Right)),
        b(Icon::AlignTop, "Align top (Alt+W)", Message::Align(Align::Top)),
        b(Icon::AlignMiddle, "Align vertical centers (Alt+V)", Message::Align(Align::Middle)),
        b(Icon::AlignBottom, "Align bottom (Alt+S)", Message::Align(Align::Bottom)),
    ]
    .spacing(2)
    .align_y(Vertical::Center);
    let col_extra = (app.selection.len() >= 2).then(|| {
        use jd_vector::boolean::BoolOp;
        let t = |label: &'static str, tip: &'static str, m: Message| {
            jd_ui::widget::hover_tip(
                button(text(label).size(TEXT_SIZE_SMALL)).padding(Padding::from([3, 7])).style(style::ghost(false)).on_press(m),
                tip,
                iced::widget::tooltip::Position::Bottom,
            )
        };
        row![
            t("Union", "Union selection", Message::Boolean(BoolOp::Union)),
            t("Subtract", "Subtract the top layers from the bottom one", Message::Boolean(BoolOp::Subtract)),
            t("Intersect", "Keep where they all overlap", Message::Boolean(BoolOp::Intersect)),
            t("Exclude", "Keep where they don't overlap", Message::Boolean(BoolOp::Exclude)),
        ]
        .spacing(2)
    });
    if app.selection.len() >= 3 {
        let d = |label: &'static str, tip: &'static str, m: Message| {
            jd_ui::widget::hover_tip(button(text(label).size(TEXT_SIZE_SMALL)).padding(Padding::from([3, 6])).style(style::ghost(false)).on_press(m), tip, iced::widget::tooltip::Position::Bottom)
        };
        r = r.push(space::horizontal());
        r = r.push(d("↔", "Distribute horizontal spacing (Alt+Shift+H)", Message::Distribute(true)));
        r = r.push(d("↕", "Distribute vertical spacing (Alt+Shift+V)", Message::Distribute(false)));
    }
    let mut c = column![r].spacing(4);
    if let Some(extra) = col_extra {
        c = c.push(extra);
    }
    container(c).padding(Padding::from([6, 10])).into()
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
        section("Local variables", None),
        body(
            button(text(variables_summary(app)).size(TEXT_SIZE_SMALL))
                .padding(Padding::from([4, 10]))
                .style(style::ghost(false))
                .on_press(Message::ShowVariables(true))
        ),
        local_styles(app),
        section("Get started", None),
        body(
            text("F frame  ·  R rectangle  ·  O ellipse  ·  T text\nDrag to draw, click to select, Space or middle-drag to pan, Ctrl+scroll to zoom.")
                .size(TEXT_SIZE_SMALL)
                .color(TEXT_SECONDARY)
        ),
    ]
    .into()
}

pub(super) fn swatch<'a>(hex: &str) -> Element<'a, Message> {
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
    let many = app.selection.len() > 1;
    let kind = if many { format!("{} layers", app.selection.len()) } else { n.kind.type_name().replace('_', " ") };
    let mut col = column![
        arrange(app),
        hline(BG_PANEL),
        container(
            row![icon(if many { Icon::Layers } else { super::kind_icon(&n.kind) }, 12.0, ICON), text(kind).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY)]
                .spacing(6)
                .align_y(Vertical::Center)
        )
        .padding(Padding { top: 10.0, right: 12.0, bottom: 4.0, left: 12.0 }),
        body(field(app, "", Field::Name, || shared(app, |n| n.name.clone()))),
    ];

    if !many && let NodeKind::Instance { component, .. } = &n.kind {
        col = col.push(instance(app, n.id, *component));
    }
    if many && all(app, |n| matches!(n.kind, NodeKind::Component { .. })) {
        col = col.push(body(
            button(text("Combine as variants").size(TEXT_SIZE_SMALL)).padding(Padding::from([4, 10])).style(style::ghost(false)).on_press(Message::CombineVariants),
        ));
    }
    if !many {
        if let Some(props) = component_props(app, n) {
            col = col.push(props);
        }
        if let Some(links) = property_links(app, n) {
            col = col.push(links);
        }
    }

    col = col.push(section("Layout", None));
    let mut layout = column![
        pair(field(app, "X", Field::X, || shared(app, |n| num(n.props.x))), field(app, "Y", Field::Y, || shared(app, |n| num(n.props.y)))),
        pair(field(app, "W", Field::W, || shared(app, |n| num(n.props.width))), field(app, "H", Field::H, || shared(app, |n| num(n.props.height)))),
        pair(
            field(app, "°", Field::Rotation, || shared(app, |n| num(n.props.rotation))),
            field(app, "R", Field::Radius, || shared(app, |n| num(n.props.corner.top_left))),
        ),
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

    if all(app, |n| n.kind.is_frame_like()) {
        let on = p.layout.is_some();
        col = col.push(section("Auto layout", Some((if on { Icon::Minus } else { Icon::Plus }, Message::ToggleAutoLayout))));
        if let Some(l) = &p.layout {
            let dir = |d: Direction, i: Icon| {
                button(icon(i, 14.0, ICON)).padding(4).style(style::ghost(l.direction == d)).on_press(Message::SetDirection(d))
            };
            col = col.push(body(
                column![
                    row![
                        dir(Direction::Horizontal, Icon::ChevronRight),
                        dir(Direction::Vertical, Icon::Chevron),
                        space::horizontal(),
                        field(app, "Gap", Field::Gap, || shared(app, |n| n.props.layout.as_ref().map_or(String::new(), |l| num(l.gap))))
                    ]
                    .spacing(2)
                    .align_y(Vertical::Center),
                    pair(
                        field(app, "↔", Field::PadH, || shared(app, |n| n.props.layout.as_ref().map_or(String::new(), |l| num(l.padding.left)))),
                        field(app, "↕", Field::PadV, || shared(app, |n| n.props.layout.as_ref().map_or(String::new(), |l| num(l.padding.top)))),
                    ),
                ]
                .spacing(8),
            ));
        }
    }

    if !many
        && n.kind.is_frame_like()
        && let Some(m) = modes(app, n)
    {
        col = col.push(m);
    }
    col = col.push(section("Appearance", None));
    col = col.push(body(field(app, "%", Field::Opacity, || shared(app, |n| num(n.props.opacity as f64 * 100.0)))));
    if let Some(b) = number_bindings(app, n) {
        col = col.push(b);
    }

    let text_of = |n: &Node| match &n.kind {
        NodeKind::Text { text } => Some(text.clone()),
        _ => None,
    };
    if all(app, |n| matches!(n.kind, NodeKind::Text { .. }))
        && let NodeKind::Text { text: t } = &n.kind
    {
        col = col.push(section("Text", None));
        col = col.push(body(
            column![
                text_tokens(app, n),
                field(app, "", Field::Text, || shared(app, |n| text_of(n).map_or(String::new(), |t| t.content.replace('\n', "\\n")))),
                pair(
                    field(app, "Sz", Field::FontSize, || shared(app, |n| text_of(n).map_or(String::new(), |t| num(t.style.size as f64)))),
                    field(app, "", Field::FontStyle, || shared(app, |n| text_of(n).map_or(String::new(), |t| t.style.style.clone()))),
                ),
                text(format!("Font: {}", if t.style.family.is_empty() { "system sans-serif" } else { &t.style.family }))
                    .size(10)
                    .color(TEXT_MUTED),
            ]
            .spacing(8),
        ));
    }

    if all(app, |n| !matches!(n.kind, NodeKind::Group | NodeKind::ComponentSet { .. })) {
        let mixed = |fills: bool| -> Element<'a, Message> {
            let same = all(app, |o| if fills { o.props.fills == p.fills } else { o.props.strokes == p.strokes });
            if same {
                paints(app, if fills { &p.fills } else { &p.strokes }, fills)
            } else {
                text("Mixed: + adds one to each layer").size(TEXT_SIZE_SMALL).color(TEXT_MUTED).into()
            }
        };
        col = col.push(section("Fill", Some((Icon::Plus, Message::AddFill))));
        col = col.push(body(column![fill_tokens(app, n), mixed(true)].spacing(8)));
        col = col.push(section("Stroke", Some((Icon::Plus, Message::AddStroke))));
        let mut stroke = column![mixed(false)].spacing(8);
        if !p.strokes.is_empty() {
            stroke = stroke.push(field(app, "Wt", Field::StrokeWeight, || shared(app, |n| num(n.props.stroke.weight as f64))));
        }
        col = col.push(body(stroke));
    }

    if all(app, |n| !matches!(n.kind, NodeKind::ComponentSet { .. })) {
        col = col.push(section("Effects", Some((Icon::Plus, Message::AddEffect))));
        col = col.push(body(effects(app, n)));
    }
    col = col.push(section("Export", None));
    let export = |label: &'a str, f: ExportFormat| {
        button(text(label).size(TEXT_SIZE_SMALL)).padding(Padding::from([4, 10])).style(style::ghost(false)).on_press(Message::Export(f))
    };
    col = col.push(body(
        row![
            export("PNG", ExportFormat::Png),
            export("SVG", ExportFormat::Svg),
            button(text("PDF").size(TEXT_SIZE_SMALL)).padding(Padding::from([4, 10])).style(style::ghost(false)).on_press(Message::ExportPdf),
        ]
        .spacing(8),
    ));
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
            (PropDef::InstanceSwap { .. }, PropValue::Node(c)) => {
                let n = name.clone();
                let opts = swap_choices(app);
                let cur = opts.iter().find(|o| o.0 == c).cloned();
                picker(opts, cur, move |v| Message::SetPropValue(n.clone(), PropValue::Node(v)))
            }
            _ => text("?").size(TEXT_SIZE_SMALL).color(TEXT_MUTED).into(),
        };
        col = col.push(row![label, control].align_y(Vertical::Center));
    }
    // Swap: any other placeable component (a set places its first variant).
    let doc = app.doc();
    let options: Vec<Choice<jd_vector::NodeId>> = app
        .components()
        .into_iter()
        .filter_map(|(c, _)| {
            let n = doc.node(c)?;
            let target = if matches!(n.kind, NodeKind::ComponentSet { .. }) { *doc.children(c).first()? } else { c };
            Some(Choice(target, n.name.clone()))
        })
        .collect();
    let current = options.iter().find(|c| c.0 == component || doc.parent(component) == doc.parent(c.0) && doc.component_set_of(component).is_some()).cloned();
    col = col.push(row![text("Component").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(70), picker(options, current, Message::SwapComponent)].align_y(Vertical::Center));
    let action = |label: &'a str, m: Message| button(text(label).size(TEXT_SIZE_SMALL)).padding(Padding::from([4, 10])).style(style::ghost(false)).on_press(m);
    col = col.push(row![action("Detach", Message::Detach), action("Reset", Message::ResetOverrides)].spacing(8));
    body(col)
}

fn variables_summary(app: &Blobr) -> String {
    let t = app.doc().tokens();
    match (t.collections.len(), t.variables.len()) {
        (0, _) => "Open variables".into(),
        (c, v) => format!("{v} variable{} in {c} collection{}", if v == 1 { "" } else { "s" }, if c == 1 { "" } else { "s" }),
    }
}

/// The document's styles (nothing selected): swatch or sample, name to rename, delete.
fn local_styles(app: &Blobr) -> Element<'_, Message> {
    let styles = &app.doc().tokens().styles;
    let mut col = column![].spacing(6);
    for (&id, st) in styles {
        let sample: Element<'_, Message> = match &st.kind {
            jd_vector::StyleKind::Paint { paints } => match paints.first().map(|p| &p.kind) {
                Some(PaintKind::Solid { color }) => swatch(&color.to_hex()),
                _ => icon(Icon::Gradient, 12.0, ICON).into(),
            },
            jd_vector::StyleKind::Text { text: t } => text(format!("Ag {}", t.size)).size(10).color(TEXT_SECONDARY).width(36).into(),
            jd_vector::StyleKind::Effect { .. } => icon(Icon::Fx, 12.0, ICON).into(),
        };
        col = col.push(
            row![
                sample,
                field(app, "", Field::StyleName(id), || st.name.clone()),
                button(icon(Icon::Minus, 12.0, ICON)).padding(3).style(style::ghost(false)).on_press(Message::DeleteToken(id)),
            ]
            .spacing(8)
            .align_y(Vertical::Center),
        );
    }
    if styles.is_empty() {
        col = col.push(text("Select a layer and use the style button by Fill or Text to make one.").size(10).color(TEXT_MUTED));
    }
    column![section("Local styles", None), body(col)].into()
}

/// Style and variable controls for the first selected layer's fills.
fn fill_tokens<'a>(app: &'a Blobr, n: &'a Node) -> Element<'a, Message> {
    let tokens = app.doc().tokens();
    let paint_styles: Vec<Choice<Option<jd_vector::NodeId>>> = std::iter::once(Choice(None, "No style".into()))
        .chain(tokens.styles.iter().filter(|(_, s)| matches!(s.kind, jd_vector::StyleKind::Paint { .. })).map(|(&id, s)| Choice(Some(id), s.name.clone())))
        .collect();
    let current_style = paint_styles.iter().find(|c| c.0 == n.props.styles.fill).cloned();
    let mut col = column![
        row![
            text("Style").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(56),
            picker(paint_styles, current_style, Message::UseFillStyle),
            jd_ui::widget::hover_tip(
                button(icon(Icon::Plus, 12.0, ICON)).padding(3).style(style::ghost(false)).on_press(Message::CreateFillStyle),
                "Create a style from these fills",
                iced::widget::tooltip::Position::Bottom,
            ),
        ]
        .spacing(6)
        .align_y(Vertical::Center)
    ]
    .spacing(6);
    if n.props.styles.fill.is_none() && !n.props.fills.is_empty() {
        let colors: Vec<Choice<Option<jd_vector::NodeId>>> = std::iter::once(Choice(None, "No variable".into()))
            .chain(
                tokens
                    .variables
                    .iter()
                    .filter(|(_, v)| v.values.values().next().is_some_and(|x| matches!(x, jd_vector::VarValue::Color(_) | jd_vector::VarValue::Alias(_))))
                    .map(|(&id, v)| Choice(Some(id), v.name.clone())),
            )
            .collect();
        let bound = n.props.bind.get("fills/0").copied();
        let current = colors.iter().find(|c| c.0 == bound).cloned();
        col = col.push(
            row![text("Variable").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(56), picker(colors, current, |v| Message::BindFill(0, v))]
                .spacing(6)
                .align_y(Vertical::Center),
        );
    }
    col.into()
}

/// A text layer's text style.
fn text_tokens<'a>(app: &'a Blobr, n: &'a Node) -> Element<'a, Message> {
    let tokens = app.doc().tokens();
    let styles: Vec<Choice<Option<jd_vector::NodeId>>> = std::iter::once(Choice(None, "No style".into()))
        .chain(tokens.styles.iter().filter(|(_, s)| matches!(s.kind, jd_vector::StyleKind::Text { .. })).map(|(&id, s)| Choice(Some(id), s.name.clone())))
        .collect();
    let current = styles.iter().find(|c| c.0 == n.props.styles.text).cloned();
    row![
        text("Style").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(56),
        picker(styles, current, Message::UseTextStyle),
        jd_ui::widget::hover_tip(
            button(icon(Icon::Plus, 12.0, ICON)).padding(3).style(style::ghost(false)).on_press(Message::CreateTextStyle),
            "Create a text style from this text",
            iced::widget::tooltip::Position::Bottom,
        ),
    ]
    .spacing(6)
    .align_y(Vertical::Center)
    .into()
}

/// A frame's mode for each collection with more than one mode.
fn modes<'a>(app: &'a Blobr, n: &'a Node) -> Option<Element<'a, Message>> {
    let tokens = app.doc().tokens();
    let mut col = column![].spacing(6);
    let mut any = false;
    for (&cid, c) in tokens.collections.iter().filter(|(_, c)| c.modes.len() > 1) {
        any = true;
        let opts: Vec<Choice<Option<String>>> = std::iter::once(Choice(None, "Auto (inherit)".into())).chain(c.modes.iter().map(|m| Choice(Some(m.clone()), m.clone()))).collect();
        let current = opts.iter().find(|o| o.0.as_ref() == n.props.modes.get(&cid)).cloned();
        col = col.push(
            row![text(c.name.clone()).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(70), picker(opts, current, move |m| Message::SetMode(cid, m))]
                .align_y(Vertical::Center),
        );
    }
    any.then(|| column![section("Variable modes", None), body(col)].into())
}

/// Components an instance can swap to (a set offers its first variant).
fn swap_choices(app: &Blobr) -> Vec<Choice<jd_vector::NodeId>> {
    let doc = app.doc();
    app.components()
        .into_iter()
        .filter_map(|(c, _)| {
            let n = doc.node(c)?;
            let target = if matches!(n.kind, NodeKind::ComponentSet { .. }) { *doc.children(c).first()? } else { c };
            Some(Choice(target, n.name.clone()))
        })
        .collect()
}

/// Number properties bound to number variables (shown once the file has some).
fn number_bindings<'a>(app: &'a Blobr, n: &'a Node) -> Option<Element<'a, Message>> {
    let tokens = app.doc().tokens();
    let vars: Vec<Choice<Option<jd_vector::NodeId>>> = std::iter::once(Choice(None, "None".into()))
        .chain(
            tokens
                .variables
                .iter()
                .filter(|(_, v)| v.values.values().next().is_some_and(|x| matches!(x, jd_vector::VarValue::Number(_) | jd_vector::VarValue::Alias(_))))
                .map(|(&id, v)| Choice(Some(id), v.name.clone())),
        )
        .collect();
    if vars.len() == 1 {
        return None;
    }
    let mut paths: Vec<(&'static str, &'static str)> = vec![("Width", "width"), ("Height", "height"), ("Radius", "corner"), ("Opacity", "opacity")];
    if n.props.layout.is_some() {
        paths.extend([("Gap", "gap"), ("Padding", "padding")]);
    }
    if matches!(n.kind, NodeKind::Text { .. }) {
        paths.push(("Font size", "font_size"));
    }
    if !n.props.strokes.is_empty() {
        paths.push(("Stroke", "stroke_weight"));
    }
    let mut col = column![].spacing(6);
    for (label, path) in paths {
        let bound = n.props.bind.get(path).copied();
        let cur = vars.iter().find(|c| c.0 == bound).cloned();
        col = col.push(
            row![text(label).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(70), picker(vars.clone(), cur, move |v| Message::Bind(path.into(), v))]
                .align_y(Vertical::Center),
        );
    }
    Some(column![section("Number variables", None), body(col)].into())
}

/// The first selected layer's effects, and an effect style picker.
fn effects<'a>(app: &'a Blobr, n: &'a Node) -> Element<'a, Message> {
    use jd_vector::Effect as E;
    let tokens = app.doc().tokens();
    let styles: Vec<Choice<Option<jd_vector::NodeId>>> = std::iter::once(Choice(None, "No style".into()))
        .chain(tokens.styles.iter().filter(|(_, s)| matches!(s.kind, jd_vector::StyleKind::Effect { .. })).map(|(&id, s)| Choice(Some(id), s.name.clone())))
        .collect();
    let cur = styles.iter().find(|c| c.0 == n.props.styles.effect).cloned();
    let mut col = column![
        row![
            text("Style").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(56),
            picker(styles, cur, Message::UseEffectStyle),
            jd_ui::widget::hover_tip(
                button(icon(Icon::Plus, 12.0, ICON)).padding(3).style(style::ghost(false)).on_press(Message::CreateEffectStyle),
                "Create an effect style from these effects",
                iced::widget::tooltip::Position::Bottom,
            ),
        ]
        .spacing(6)
        .align_y(Vertical::Center)
    ]
    .spacing(8);
    let kinds = vec![Choice("drop", "Drop shadow".into()), Choice("inner", "Inner shadow".into()), Choice("blur", "Layer blur".into()), Choice("background", "Background blur".into())];
    for (i, e) in n.props.effects.iter().enumerate() {
        let kind = match e {
            E::DropShadow(_) => "drop",
            E::InnerShadow(_) => "inner",
            E::LayerBlur { .. } => "blur",
            E::BackgroundBlur { .. } => "background",
        };
        let cur = kinds.iter().find(|k| k.0 == kind).cloned();
        col = col.push(
            row![picker(kinds.clone(), cur, move |k| Message::SetEffectKind(i, k)), button(icon(Icon::Minus, 12.0, ICON)).padding(3).style(style::ghost(false)).on_press(Message::RemoveEffect(i))]
                .spacing(6)
                .align_y(Vertical::Center),
        );
        match e {
            E::DropShadow(s) | E::InnerShadow(s) => {
                let hex = jd_vector::color::to_hex(s.color);
                col = col.push(column![
                    pair(field(app, "X", Field::Effect(i, "x"), || num(s.x as f64)), field(app, "Y", Field::Effect(i, "y"), || num(s.y as f64))),
                    pair(field(app, "B", Field::Effect(i, "blur"), || num(s.blur as f64)), field(app, "S", Field::Effect(i, "spread"), || num(s.spread as f64))),
                    row![swatch(&hex), field(app, "", Field::Effect(i, "color"), move || hex.clone())].spacing(8).align_y(Vertical::Center),
                ]
                .spacing(6));
            }
            E::LayerBlur { radius, .. } | E::BackgroundBlur { radius, .. } => {
                col = col.push(field(app, "R", Field::Effect(i, "radius"), || num(*radius as f64)));
            }
        }
    }
    col.into()
}
