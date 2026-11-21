//! Premiere's Editing workspace: Source and Program monitors on top; Project, Tools,
//! Timeline and audio meters below. Panels sit on a dark gutter, like the Figma mockup.

mod menus;
mod meters;
mod monitor;
mod project;
mod timeline;

use iced::alignment::Vertical;
use iced::widget::{button, column, container, row, stack, text};
use iced::{Element, Fill, FillPortion, Padding};
use jd_ui::chrome::{self, title_bar};
use jd_ui::widget::{bold, field_box, hline, hover_tip, tool_button, value};
use jd_ui::{Icon, icon, style, tokens::*};

use crate::APP_NAME;
use crate::app::{Editor, Message};
use crate::tool::Tool;

pub use menus::MENU_COUNT;

/// Gap between panels.
const GUTTER: f32 = 3.0;

pub fn view(app: &Editor) -> Element<'_, Message> {
    let top = row![monitor::source(app), monitor::program(app)]
        .spacing(GUTTER)
        .height(FillPortion(52));
    let bottom = row![project::view(app), tools(app), timeline::view(app), meters::view(app)]
        .spacing(GUTTER)
        .height(FillPortion(48));
    let workspace = container(column![top, bottom].spacing(GUTTER))
        .padding(GUTTER)
        .width(Fill)
        .height(Fill)
        .style(style::fill(BORDER_DEFAULT));

    let content = container(column![
        title_bar(
            &app.chrome,
            crate::LOGO_SVG,
            menus::MENUS,
            format!("{} - {APP_NAME}", app.project_name),
            title_actions(),
            Message::Quit,
            Message::Chrome,
        ),
        hline(BORDER_DEFAULT),
        workspace,
    ])
    .width(Fill)
    .height(Fill)
    .style(style::fill(BG_APP));

    let mut layers = stack![content];
    if !(app.chrome.maximized || app.chrome.fullscreen) {
        layers = layers.push(chrome::resize_edges(Message::Chrome));
    }
    if let Some(popup) = menus::popup(app) {
        layers = layers.push(popup);
    }
    layers.into()
}

fn title_actions<'a>() -> Element<'a, Message> {
    let workspace = field_box(
        row![value("Editing"), icon(Icon::Chevron, 12.0, TEXT_SECONDARY)]
            .spacing(6)
            .align_y(Vertical::Center),
    );
    // Enabled once a sequence can be rendered (M5).
    let export = button(
        row![icon(Icon::Share, 12.0, TEXT_PRIMARY), text("Export").size(TEXT_SIZE_SMALL).font(bold())]
            .spacing(6)
            .align_y(Vertical::Center),
    )
    .padding(Padding::from([4, 10]))
    .style(style::primary);
    row![icon(Icon::Zoom, 14.0, ICON), workspace, export]
        .spacing(12)
        .align_y(Vertical::Center)
        .into()
}

/// A panel's body background (the gutter shows between panels).
fn panel<'a>(content: impl Into<Element<'a, Message>>) -> container::Container<'a, Message> {
    container(content).style(style::fill(BG_APP)).clip(true)
}

fn tools(app: &Editor) -> Element<'_, Message> {
    let mut col = column![].spacing(2).width(Fill).align_x(iced::Alignment::Center).padding(Padding::from([6, 0]));
    for (i, group) in Tool::GROUPS.iter().enumerate() {
        if i > 0 {
            col = col.push(container(hline(BORDER_SUBTLE)).width(22).padding(Padding::from([3, 0])));
        }
        for &t in *group {
            col = col.push(tool_button(t.icon(), app.tool == t, false, t.tooltip(), Message::SelectTool(t)));
        }
    }
    panel(col).width(40).height(Fill).into()
}

/// A small square toggle (snap, linked selection...) with a tooltip.
fn toggle_button<'a>(i: Icon, on: bool, tip: &'a str, msg: Option<Message>) -> Element<'a, Message> {
    let color = if msg.is_none() { TEXT_MUTED } else if on { TEXT_PRIMARY } else { ICON };
    hover_tip(
        button(icon(i, 14.0, color)).padding(5).style(style::ghost(on)).on_press_maybe(msg),
        tip,
        iced::widget::tooltip::Position::Bottom,
    )
}
