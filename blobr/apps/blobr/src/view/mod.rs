//! The main window, laid out like the Figma mockup: toolbar, Layers panel, canvas and Design
//! panel on a dark gutter, under the suite's title bar.

mod assets;
mod canvas;
mod design;
mod inspect;
mod layers;
mod menus;
mod present;
mod prototype;
mod variables;

use iced::alignment::Vertical;
use iced::widget::{button, column, container, mouse_area, row, space, stack, text};
use iced::{Element, Fill, Padding, Size};
use jd_ui::chrome::{self, TITLE_BAR_HEIGHT, title_bar};
use jd_ui::widget::{bold, field_box, hline, tool_button, value, vline};
use jd_ui::{Icon, icon, style, tokens::*};
use jd_vector::ExportFormat;

use crate::APP_NAME;
use crate::app::{After, Blobr, Message};
use crate::tool::Tool;

pub use layers::ROW as LAYER_ROW;
pub use menus::MENU_COUNT;

/// Gap between panels.
pub const GUTTER: f32 = 3.0;
pub const TOOLBAR_WIDTH: f32 = 44.0;
pub const LAYERS_WIDTH: f32 = 248.0;
pub const DESIGN_WIDTH: f32 = 272.0;
pub const TABS_HEIGHT: f32 = 28.0;
pub const STATUS_HEIGHT: f32 = 24.0;

/// The canvas size for a window size (the panels are fixed width).
pub fn canvas_size(window: Size) -> Size {
    let w = window.width - 5.0 * GUTTER - TOOLBAR_WIDTH - LAYERS_WIDTH - DESIGN_WIDTH;
    let h = window.height - TITLE_BAR_HEIGHT - 1.0 - 2.0 * GUTTER - TABS_HEIGHT - STATUS_HEIGHT;
    Size::new(w.max(1.0), h.max(1.0))
}

pub fn view(app: &Blobr) -> Element<'_, Message> {
    let center = column![doc_tabs(app), canvas::view(app), status_bar(app)].width(Fill).height(Fill);
    let workspace = container(
        row![
            panel(toolbar(app), Some(TOOLBAR_WIDTH)),
            panel(layers::view(app), Some(LAYERS_WIDTH)),
            panel(center, None),
            panel(design::view(app), Some(DESIGN_WIDTH)),
        ]
        .spacing(GUTTER),
    )
    .padding(GUTTER)
    .width(Fill)
    .height(Fill)
    .style(style::fill(BORDER_DEFAULT));

    let content = container(column![
        title_bar(&app.chrome, crate::LOGO_SVG, menus::MENUS, format!("{} - {APP_NAME}", app.file_name()), title_actions(), Message::Quit, Message::Chrome),
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
    if app.variables_open {
        layers = layers.push(variables::view(app));
    }
    if let Some(p) = present::view(app) {
        // Presenting covers the whole window.
        return p;
    }
    if let Some(after) = app.confirm {
        layers = layers.push(confirm(app, after));
    }
    layers.into()
}

fn panel<'a>(content: impl Into<Element<'a, Message>>, width: Option<f32>) -> Element<'a, Message> {
    let c = container(content).height(Fill).clip(true).style(|_| container::Style {
        background: Some(BG_APP.into()),
        border: iced::Border { radius: 4.0.into(), ..Default::default() },
        ..Default::default()
    });
    match width {
        Some(w) => c.width(w).into(),
        None => c.width(Fill).into(),
    }
}

/// "Save changes?" before quitting, opening or starting a new file.
fn confirm(app: &Blobr, after: After) -> Element<'_, Message> {
    let verb = match after {
        After::Quit => "closing",
        After::New => "starting a new file",
        After::Open => "opening another file",
    };
    let name = app.path.as_ref().and_then(|p| p.file_name()).map_or("Untitled".into(), |n| n.to_string_lossy().into_owned());
    let action = |label: &'static str, m: Message, primary: bool| {
        let b = button(text(label).size(TEXT_SIZE_SMALL)).padding(Padding::from([5, 14])).on_press(m);
        if primary { b.style(style::primary) } else { b.style(style::ghost(false)) }
    };
    let card = container(
        column![
            text(format!("Save changes to {name}?")).size(TEXT_SIZE).font(bold()).color(TEXT_PRIMARY),
            text(format!("Your changes will be lost if you don't save them before {verb}.")).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY),
            row![
                action("Don't Save", Message::ConfirmDiscard, false),
                space::horizontal(),
                action("Cancel", Message::ConfirmCancel, false),
                action("Save", Message::ConfirmSave, true),
            ]
            .spacing(8),
        ]
        .spacing(12),
    )
    .padding(20)
    .width(380)
    .style(|_| container::Style {
        background: Some(BG_PANEL.into()),
        border: iced::Border { radius: 6.0.into(), width: 1.0, color: BORDER_DEFAULT },
        ..Default::default()
    });
    // A dimmed backdrop that swallows clicks.
    iced::widget::opaque(
        mouse_area(container(card).center(Fill).style(|_| container::Style {
            background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.45).into()),
            ..Default::default()
        }))
        .on_press(Message::Noop),
    )
}

fn title_actions<'a>() -> Element<'a, Message> {
    let workspace = field_box(row![value("Design"), icon(Icon::Chevron, 12.0, TEXT_SECONDARY)].spacing(6).align_y(Vertical::Center));
    let export = button(
        row![icon(Icon::Share, 12.0, TEXT_PRIMARY), text("Export").size(TEXT_SIZE_SMALL).font(bold())]
            .spacing(6)
            .align_y(Vertical::Center),
    )
    .padding(Padding::from([4, 10]))
    .style(style::primary)
    .on_press(Message::Export(ExportFormat::Png));
    let play = jd_ui::widget::hover_tip(
        button(icon(Icon::Play, 14.0, ICON)).padding(4).style(style::ghost(false)).on_press(Message::Present(None)),
        "Present (Ctrl+Alt+Enter)",
        iced::widget::tooltip::Position::Bottom,
    );
    row![icon(Icon::Zoom, 14.0, ICON), workspace, play, export].spacing(12).align_y(Vertical::Center).into()
}

fn toolbar(app: &Blobr) -> Element<'_, Message> {
    let mut col = column![container(space().width(20).height(3)).style(style::fill(BORDER_SUBTLE))]
        .spacing(2)
        .padding(Padding::ZERO.top(6))
        .align_x(iced::alignment::Horizontal::Center)
        .width(Fill);
    for t in Tool::BAR {
        col = match t {
            Some(t) => col.push(tool_button(t.icon(), app.tool == *t, false, t.tip(), Message::Tool(*t))),
            None => col.push(container(space().width(20).height(1)).style(style::fill(BORDER_SUBTLE))),
        };
    }
    col.into()
}

fn doc_tabs(app: &Blobr) -> Element<'_, Message> {
    let tab = container(
        row![text(app.file_name()).size(TEXT_SIZE_SMALL).color(TEXT_PRIMARY), icon(Icon::Close, 10.0, TEXT_SECONDARY)]
            .spacing(10)
            .align_y(Vertical::Center),
    )
    .padding(Padding::from([7, 12]))
    .height(TABS_HEIGHT)
    .style(style::fill(BG_PANEL));
    container(row![tab]).width(Fill).height(TABS_HEIGHT).style(style::fill(BG_PANEL_HEADER)).into()
}

fn status_bar(app: &Blobr) -> Element<'_, Message> {
    let doc = app.doc();
    let zoom = field_box(value(format!("{:.0}%", app.camera.zoom * 100.0)));
    let page = doc.node(app.page).map_or(String::new(), |n| n.name.clone());
    let selection = match app.selection.as_slice() {
        [] => String::new(),
        [one] => doc.node(*one).map_or(String::new(), |n| format!("{}  ·  {:.0} × {:.0}", n.name, n.props.width, n.props.height)),
        many => format!("{} selected", many.len()),
    };
    let status = text(app.status.text.clone()).size(TEXT_SIZE_SMALL).wrapping(text::Wrapping::None).color(if app.status.error { DANGER } else { TEXT_SECONDARY });
    let saved = text(if app.ed.history.is_dirty() { "Unsaved" } else { "Saved" }).size(TEXT_SIZE_SMALL).color(TEXT_MUTED);
    container(
        row![
            zoom,
            text(page).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY),
            text(selection).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY),
            space::horizontal(),
            status,
            vline(12, BORDER_SUBTLE),
            saved
        ]
        .spacing(16)
        .align_y(Vertical::Center),
    )
    .padding(Padding::from([0, 10]))
    .height(STATUS_HEIGHT)
    .align_y(Vertical::Center)
    .width(Fill)
    .style(style::fill(BG_PANEL_HEADER))
    .into()
}

/// Icon for a node kind in the Layers panel.
pub fn kind_icon(kind: &jd_vector::NodeKind) -> Icon {
    use jd_vector::NodeKind as K;
    match kind {
        K::Page { .. } => Icon::Page,
        K::Frame | K::Section => Icon::Frame,
        K::Group => Icon::Layers,
        K::Rectangle | K::Polygon { .. } | K::Star { .. } => Icon::Shape,
        K::Ellipse { .. } => Icon::EllipseShape,
        K::Line => Icon::Minus,
        K::Vector { .. } => Icon::Pen,
        K::Text { .. } => Icon::Type,
        K::Component { .. } | K::ComponentSet { .. } => Icon::Component,
        K::Instance { .. } => Icon::Instance,
    }
}

/// Components and instances are purple, as in Figma.
pub const COMPONENT: iced::Color = iced::Color::from_rgb8(0xB4, 0x8E, 0xF0);
