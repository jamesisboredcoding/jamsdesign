//! The main window, laid out like the Figma mockup: toolbar, Layers panel, canvas and Design
//! panel on a dark gutter, under the suite's title bar.

mod assets;
pub(crate) mod canvas;
mod comments;
mod design;
mod dialogs;
mod inspect;
mod layers;
mod menus;
mod present;
mod prototype;
mod variables;

use iced::alignment::Vertical;
use iced::widget::{button, column, container, mouse_area, row, space, stack, text, text_input};
use iced::{Element, Fill, Padding, Size};
use jd_ui::chrome::{self, TITLE_BAR_HEIGHT, title_bar};
use jd_ui::widget::{bold, field_box, hline, tool_button, value, vline};
use jd_ui::{Icon, icon, style, tokens::*};
use jd_vector::ExportFormat;

use crate::APP_NAME;
use crate::app::{After, Blobr, Dropdown, Message, SEARCH_INPUT, Side, ZOOM_INPUT, ZOOM_STEPS};
use crate::tool::Tool;

pub use layers::ROW as LAYER_ROW;
pub use menus::MENU_COUNT;

/// Gap between panels.
pub const GUTTER: f32 = 3.0;
pub const TOOLBAR_WIDTH: f32 = 44.0;
/// The gaps either side of the canvas, which can be dragged to resize the panels.
pub const HANDLE: f32 = 5.0;
pub const TABS_HEIGHT: f32 = 28.0;
pub const STATUS_HEIGHT: f32 = 24.0;

/// The widths of the left and right side panels (Layers and Design, unless swapped).
pub fn side_widths(app: &Blobr) -> (f32, f32) {
    let p = &app.prefs;
    if p.swap_panels { (p.design_width, p.layers_width) } else { (p.layers_width, p.design_width) }
}

/// Where the canvas starts, from the window's left edge.
pub fn canvas_left(app: &Blobr) -> f32 {
    if app.prefs.hide_panels {
        return GUTTER;
    }
    GUTTER + TOOLBAR_WIDTH + GUTTER + side_widths(app).0 + HANDLE
}

/// The canvas size for a window size (until the canvas reports its own).
pub fn canvas_size(app: &Blobr, window: Size) -> Size {
    let (l, r) = side_widths(app);
    let panels = if app.prefs.hide_panels { 0.0 } else { TOOLBAR_WIDTH + GUTTER + l + r + 2.0 * HANDLE };
    let w = window.width - 2.0 * GUTTER - panels;
    let h = window.height - TITLE_BAR_HEIGHT - 1.0 - 2.0 * GUTTER - TABS_HEIGHT - STATUS_HEIGHT;
    Size::new(w.max(1.0), h.max(1.0))
}

/// A side panel's tab strip; its menu icon opens that panel's menu.
pub fn panel_header<'a>(app: &Blobr, tabs: &[&'a str], active: usize, on_tab: impl Fn(usize) -> Message, layers: bool) -> Element<'a, Message> {
    let open = Dropdown::Panel { layers };
    jd_ui::widget::panel_header_menu(tabs, active, on_tab, Message::Dropdown((app.dropdown != Some(open)).then_some(open)))
}

/// The width a side panel gets when its inner edge is dragged to window x.
pub fn panel_width_at(app: &Blobr, side: Side, x: f32) -> f32 {
    let w = match side {
        Side::Left => x - (GUTTER + TOOLBAR_WIDTH + GUTTER) - HANDLE / 2.0,
        Side::Right => app.chrome.size.width - GUTTER - x - HANDLE / 2.0,
    };
    w.clamp(crate::prefs::PANEL_MIN, crate::prefs::PANEL_MAX)
}

/// The gap between two panels: a plain gutter, or a handle that resizes `side`'s panel.
fn gap<'a>(side: Option<Side>) -> Element<'a, Message> {
    match side {
        None => space().width(GUTTER).height(Fill).into(),
        Some(side) => mouse_area(space().width(HANDLE).height(Fill)).on_press(Message::PanelDrag(side)).interaction(iced::mouse::Interaction::ResizingHorizontally).into(),
    }
}

/// What a window shows: the presentation in its own window, or the main window.
pub fn view(app: &Blobr, window: iced::window::Id) -> Element<'_, Message> {
    if app.present_window.is_some_and(|(w, _)| w == window) {
        return present::view(app).unwrap_or_else(|| container(space()).width(Fill).height(Fill).style(style::fill(BG_APP)).into());
    }
    main(app)
}

fn main(app: &Blobr) -> Element<'_, Message> {
    let center = column![doc_tabs(app), canvas::view(app), status_bar(app)].width(Fill).height(Fill);
    let mut panels = row![];
    if app.prefs.hide_panels {
        panels = panels.push(panel(center, None));
    } else {
        let (lw, rw) = side_widths(app);
        let (left, right): (Element<'_, Message>, Element<'_, Message>) =
            if app.prefs.swap_panels { (design::view(app), layers::view(app)) } else { (layers::view(app), design::view(app)) };
        panels = panels
            .push(panel(toolbar(app), Some(TOOLBAR_WIDTH)))
            .push(gap(None))
            .push(panel(left, Some(lw)))
            .push(gap(Some(Side::Left)))
            .push(panel(center, None))
            .push(gap(Some(Side::Right)))
            .push(panel(right, Some(rw)));
    }
    let workspace = container(panels)
    .padding(GUTTER)
    .width(Fill)
    .height(Fill)
    .style(style::fill(BORDER_DEFAULT));

    let content = container(column![
        title_bar(&app.chrome, crate::LOGO_SVG, menus::MENUS, format!("{} - {APP_NAME}", app.file_name()), title_actions(app), Message::Quit, Message::Chrome),
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
    if let Some(popup) = dropdown(app) {
        layers = layers.push(popup);
    }
    if app.panel_drag.is_some() {
        // Follows the pointer anywhere in the window until the button is let go.
        layers = layers.push(
            mouse_area(space().width(Fill).height(Fill))
                .on_move(|p| Message::PanelDragMove(p.x))
                .on_release(Message::PanelDragEnd)
                .interaction(iced::mouse::Interaction::ResizingHorizontally),
        );
    }
    let canvas_at = iced::Point::new(canvas_left(app), TITLE_BAR_HEIGHT + 1.0 + GUTTER + TABS_HEIGHT);
    if let Some(card) = comments::view(app, canvas_at) {
        layers = layers.push(card);
    }
    if app.variables_open {
        layers = layers.push(variables::view(app));
    }
    if app.present_window.is_none()
        && let Some(p) = present::view(app)
    {
        // Presenting covers the whole window (unless it has one of its own).
        return p;
    }
    if app.prefs_open {
        layers = layers.push(dialogs::prefs(app));
    }
    if app.shortcuts_open {
        layers = layers.push(dialogs::shortcuts(app));
    }
    if app.about_open {
        layers = layers.push(dialogs::about(app));
    }
    if app.text_styles_open {
        layers = layers.push(dialogs::text_styles(app));
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
        After::Quit => "quitting",
        After::CloseTab => "closing it",
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

/// The right panel's tabs, as named in the title bar's switcher.
pub const MODES: [&str; 3] = ["Design", "Prototype", "Inspect"];

fn title_actions(app: &Blobr) -> Element<'_, Message> {
    let search: Element<'_, Message> = match &app.search {
        Some(q) => text_input("Find layers", q)
            .id(SEARCH_INPUT)
            .on_input(Message::SearchInput)
            .on_submit(Message::SearchSubmit)
            .size(TEXT_SIZE_SMALL)
            .padding(Padding::from([3, 6]))
            .width(220)
            .style(design::input_style)
            .into(),
        None => jd_ui::widget::hover_tip(
            button(icon(Icon::Zoom, 14.0, ICON)).padding(4).style(style::ghost(false)).on_press(Message::Search(true)),
            "Find layers (Ctrl+F)",
            iced::widget::tooltip::Position::Bottom,
        ),
    };
    let mode = MODES.get(app.right_tab).copied().unwrap_or(MODES[0]);
    let workspace = button(field_box(row![value(mode), icon(Icon::Chevron, 12.0, TEXT_SECONDARY)].spacing(6).align_y(Vertical::Center)))
        .padding(0)
        .style(style::ghost(false))
        .on_press(Message::Dropdown(if app.dropdown == Some(Dropdown::Mode) { None } else { Some(Dropdown::Mode) }));
    let workspace = mouse_area(workspace).on_move(|p| Message::ModePointer(p.x));
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
    row![search, workspace, play, export].spacing(12).align_y(Vertical::Center).into()
}

/// The open dropdown, or search's results under the search box.
fn dropdown(app: &Blobr) -> Option<Element<'_, Message>> {
    use jd_ui::widget::MenuEntry;
    let size = app.chrome.size;
    let pick = |m: Message| Some(Message::Menu(Box::new(m)));
    if app.search.is_some() {
        let found = app.search_results(14);
        if found.is_empty() {
            return None;
        }
        let doc = app.doc();
        let entries = found
            .into_iter()
            .filter_map(|id| {
                let n = doc.node(id)?;
                let page = app.page_of(id).and_then(|p| doc.node(p)).map_or("", |p| p.name.as_str());
                Some(MenuEntry::IconItem(kind_icon(&n.kind), n.name.as_str(), page, Some(Message::SearchPick(id))))
            })
            .collect();
        let x = app.mode_left.map_or(size.width, |x| x - 12.0 - 220.0);
        return Some(chrome::popup(entries, 320.0, iced::Point::new(x, TITLE_BAR_HEIGHT), size, true, Message::Search(false), None, |_| Message::Noop));
    }
    match app.dropdown? {
        Dropdown::Mode => {
            let icons = [Icon::Adjust, Icon::Play, Icon::Info];
            let entries = MODES
                .iter()
                .enumerate()
                .map(|(i, m)| MenuEntry::IconItem(icons[i], *m, if i == app.right_tab { "Open" } else { "" }, pick(Message::RightTab(i))))
                .collect();
            let x = app.mode_left.unwrap_or(size.width);
            Some(chrome::popup(entries, 200.0, iced::Point::new(x, TITLE_BAR_HEIGHT), size, true, Message::Dropdown(None), None, |_| Message::Noop))
        }
        Dropdown::Zoom => {
            let sel = !app.selection.is_empty();
            let mut entries = vec![
                MenuEntry::Item("Zoom In", "Ctrl++", pick(Message::ZoomIn)),
                MenuEntry::Item("Zoom Out", "Ctrl+-", pick(Message::ZoomOut)),
                MenuEntry::Item("Zoom to Fit", "Shift+1", pick(Message::ZoomFit)),
                MenuEntry::Item("Zoom to Selection", "Shift+2", if sel { pick(Message::ZoomSelection) } else { None }),
                MenuEntry::Separator,
            ];
            for (z, label) in ZOOM_STEPS.iter().zip(["25%", "50%", "100%", "200%", "400%", "800%"]) {
                entries.push(MenuEntry::Item(label, if *z == 1.0 { "Shift+0" } else { "" }, pick(Message::ZoomTo(*z))));
            }
            let at = iced::Point::new(canvas_left(app), size.height - GUTTER - STATUS_HEIGHT);
            Some(chrome::popup(entries, 220.0, at, size, false, Message::Dropdown(None), None, |_| Message::Noop))
        }
        Dropdown::Panel { layers } => {
            let entries = vec![
                MenuEntry::Item("Swap Panel Sides", "", pick(Message::SwapPanels)),
                MenuEntry::Item("Hide Panels", "Ctrl+\\", pick(Message::TogglePanels)),
            ];
            // Under the menu icon, at the panel's right edge.
            let right = if layers != app.prefs.swap_panels { canvas_left(app) - HANDLE } else { size.width - GUTTER };
            let at = iced::Point::new(right - 200.0, TITLE_BAR_HEIGHT + 1.0 + GUTTER + 30.0);
            Some(chrome::popup(entries, 200.0, at, size, false, Message::Dropdown(None), None, |_| Message::Noop))
        }
    }
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
    let tabs = app.tab_titles().into_iter().enumerate().map(|(i, title)| {
        let active = i == app.active;
        let close = button(icon(Icon::Close, 10.0, TEXT_SECONDARY)).padding(2).style(style::ghost(false)).on_press(Message::TabClose(i));
        let tab = container(
            row![text(title).size(TEXT_SIZE_SMALL).color(if active { TEXT_PRIMARY } else { TEXT_SECONDARY }), close]
                .spacing(8)
                .align_y(Vertical::Center),
        )
        .padding(Padding::from([5, 8]).left(12))
        .height(TABS_HEIGHT)
        .style(style::fill(if active { BG_PANEL } else { BG_PANEL_HEADER }));
        mouse_area(tab).on_press(Message::TabSelect(i)).into()
    });
    let new = button(icon(Icon::Plus, 12.0, TEXT_SECONDARY)).padding(6).style(style::ghost(false)).on_press(Message::New);
    container(row(tabs).push(new).spacing(1).align_y(Vertical::Center)).width(Fill).height(TABS_HEIGHT).style(style::fill(BG_PANEL_HEADER)).into()
}

fn status_bar(app: &Blobr) -> Element<'_, Message> {
    let doc = app.doc();
    // Type a zoom (Enter applies), or pick one from the menu.
    let shown = app.zoom_draft.clone().unwrap_or_else(|| format!("{:.0}%", app.camera.zoom * 100.0));
    let zoom = row![
        text_input("", &shown)
            .id(ZOOM_INPUT)
            .on_input(Message::ZoomInput)
            .on_submit(Message::ZoomSubmit)
            .size(TEXT_SIZE_SMALL)
            .padding(Padding::from([1, 4]))
            .width(52)
            .style(design::input_style),
        button(icon(Icon::Chevron, 10.0, TEXT_SECONDARY))
            .padding(3)
            .style(style::ghost(false))
            .on_press(Message::Dropdown(if app.dropdown == Some(Dropdown::Zoom) { None } else { Some(Dropdown::Zoom) })),
    ]
    .spacing(2)
    .align_y(Vertical::Center);
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
            agent_chip(app),
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

/// "Agent" while an AI agent is connected over MCP.
fn agent_chip(app: &Blobr) -> Element<'_, Message> {
    if app.agents == 0 {
        return space::horizontal().width(0).into();
    }
    let label = if app.agents == 1 { "Agent connected".to_owned() } else { format!("{} agents connected", app.agents) };
    let accent = jd_ui::tokens::ACCENT_PRIMARY;
    container(text(label).size(TEXT_SIZE_SMALL).color(accent))
        .padding(Padding::from([1, 8]))
        .style(style::fill(iced::Color { a: 0.15, ..accent }))
        .into()
}

/// Icon for a node kind in the Layers panel.
pub fn kind_icon(kind: &jd_vector::NodeKind) -> Icon {
    use jd_vector::NodeKind as K;
    match kind {
        K::Page { .. } => Icon::Page,
        K::Frame | K::Section => Icon::Frame,
        K::Group => Icon::Layers,
        K::Boolean { .. } => Icon::Union,
        K::Rectangle | K::Polygon { .. } | K::Star { .. } => Icon::Shape,
        K::Ellipse { .. } => Icon::EllipseShape,
        K::Line => Icon::Line,
        K::Vector { .. } => Icon::Pen,
        K::Text { .. } => Icon::Type,
        K::Component { .. } | K::ComponentSet { .. } => Icon::Component,
        K::Instance { .. } => Icon::Instance,
    }
}

/// Components and instances are purple, as in Figma.
pub const COMPONENT: iced::Color = iced::Color::from_rgb8(0xB4, 0x8E, 0xF0);
