//! The Layers panel: pages, then the layer tree of the current page (top layer first).

use iced::alignment::Vertical;
use iced::widget::{button, column, container, row, scrollable, space, svg, text};
use iced::{Element, Fill, Padding};
use jd_ui::widget::{hline, panel_header};
use jd_ui::{Icon, icon, style, tokens::*};
use jd_vector::{NodeId, NodeKind};

use super::{COMPONENT, kind_icon};
use crate::app::{Blobr, Message};

const ROW: f32 = 26.0;
const INDENT: f32 = 14.0;

pub fn view(app: &Blobr) -> Element<'_, Message> {
    let doc = app.doc();
    let mut pages = column![];
    for &p in doc.pages() {
        let n = doc.node(p).expect("page");
        let current = p == app.page;
        let label = text(n.name.clone()).size(TEXT_SIZE_SMALL).color(TEXT_PRIMARY).font(if current { jd_ui::widget::bold() } else { iced::Font::DEFAULT });
        pages = pages.push(
            button(row![icon(Icon::Page, 12.0, ICON), label].spacing(8).height(Fill).align_y(Vertical::Center))
                .width(Fill)
                .height(ROW)
                .padding(Padding::from([0, 10]))
                .style(style::row(current))
                .on_press(Message::SelectPage(p)),
        );
    }

    let mut tree = Vec::new();
    for &c in doc.children(app.page).iter().rev() {
        rows(app, c, 0, &mut tree);
    }
    let tree = iced::widget::Column::with_children(tree);

    column![
        panel_header(&["Layers", "Assets"], 0, |_| Message::Noop),
        section("Pages", Some(Message::AddPage)),
        container(pages).padding(Padding::from([0, 6])),
        container(hline(BG_PANEL)).padding(Padding::from([6, 0])),
        section("Layers", None),
        scrollable(container(tree).padding(Padding::from([0, 6]))).height(Fill),
        footer(app),
    ]
    .into()
}

fn section<'a>(title: &'a str, add: Option<Message>) -> Element<'a, Message> {
    let mut r = row![text(title).size(TEXT_SIZE_SMALL).font(jd_ui::widget::bold()).color(TEXT_PRIMARY), space::horizontal()]
        .align_y(Vertical::Center)
        .height(32)
        .padding(Padding::from([0, 12]));
    if let Some(m) = add {
        r = r.push(button(icon(Icon::Plus, 12.0, ICON)).padding(4).style(style::ghost(false)).on_press(m));
    }
    r.into()
}

fn rows<'a>(app: &'a Blobr, id: NodeId, depth: usize, out: &mut Vec<Element<'a, Message>>) {
    let doc = app.doc();
    let Some(n) = doc.node(id) else { return };
    let selected = app.selection.contains(&id);
    let has_kids = !n.children.is_empty();
    let open = app.expanded.contains(&id);
    let purple = matches!(n.kind, NodeKind::Component { .. } | NodeKind::ComponentSet { .. } | NodeKind::Instance { .. });
    let color = if !n.props.visible { TEXT_MUTED } else if purple { COMPONENT } else { TEXT_PRIMARY };

    let disclosure: Element<'_, Message> = if has_kids {
        let chevron = svg(Icon::Chevron.handle())
            .width(10)
            .height(10)
            .rotation(if open { 0.0 } else { -std::f32::consts::FRAC_PI_2 })
            .style(|_, _| svg::Style { color: Some(TEXT_SECONDARY) });
        button(chevron).padding(3).style(style::ghost(false)).on_press(Message::ToggleExpand(id)).into()
    } else {
        space().width(16).into()
    };
    let mut r = row![
        space().width(depth as f32 * INDENT),
        disclosure,
        icon(kind_icon(&n.kind), 12.0, if purple { COMPONENT } else { ICON }),
        text(n.name.clone()).size(TEXT_SIZE_SMALL).color(color),
        space::horizontal(),
    ]
    .spacing(6)
    .height(Fill)
    .align_y(Vertical::Center);
    if n.props.locked || selected {
        r = r.push(
            button(icon(Icon::Lock, 12.0, if n.props.locked { ICON } else { TEXT_MUTED }))
                .padding(2)
                .style(style::ghost(false))
                .on_press(Message::ToggleLock(id)),
        );
    }
    if !n.props.visible || selected {
        r = r.push(
            button(icon(if n.props.visible { Icon::Eye } else { Icon::EyeOff }, 12.0, ICON))
                .padding(2)
                .style(style::ghost(false))
                .on_press(Message::ToggleVisible(id)),
        );
    }
    out.push(
        button(r)
            .width(Fill)
            .height(ROW)
            .padding(Padding::from([0, 8]))
            .style(style::row(selected))
            .on_press(Message::Select(id, false))
            .into(),
    );
    if has_kids && open {
        for &c in n.children.iter().rev() {
            rows(app, c, depth + 1, out);
        }
    }
}

fn footer(app: &Blobr) -> Element<'_, Message> {
    let count = app.doc().descendants(app.page).len();
    container(
        row![text(format!("{count} layers")).size(10).color(TEXT_MUTED), space::horizontal()]
            .align_y(Vertical::Center),
    )
    .padding(Padding::from([0, 10]))
    .height(30)
    .align_y(Vertical::Center)
    .width(Fill)
    .style(style::fill(BG_PANEL_HEADER))
    .into()
}
