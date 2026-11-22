//! Building blocks matching the Figma components (Tool Button, Panel Tab, Layer Row...).

use iced::alignment::{Horizontal, Vertical};
use iced::widget::{button, column, container, mouse_area, row, space, stack, text, tooltip};
use iced::{Color, Element, Fill, Length, Padding};

use crate::icon::{Icon, icon};
use crate::style;
use crate::tokens::*;

pub fn label<'a>(s: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(s).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY)
}

pub fn value<'a>(s: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(s).size(TEXT_SIZE_SMALL).color(TEXT_PRIMARY)
}

/// 1px vertical line.
pub fn vline<'a, M: 'a>(height: impl Into<Length>, color: Color) -> Element<'a, M> {
    container(space()).width(1).height(height).style(style::fill(color)).into()
}

/// 1px horizontal line filling the available width.
pub fn hline<'a, M: 'a>(color: Color) -> Element<'a, M> {
    container(space()).width(Fill).height(1).style(style::fill(color)).into()
}

/// A value shown in an inset box (read-only for now; becomes an editable scrub field later).
pub fn field_box<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> container::Container<'a, M> {
    container(content)
        .padding(Padding::from([3, 6]))
        .style(style::input)
}

pub fn labeled<'a, M: 'a>(
    name: &'a str,
    content: impl Into<Element<'a, M>>,
) -> Element<'a, M> {
    row![label(name), content.into()]
        .spacing(6)
        .align_y(Vertical::Center)
        .into()
}

pub fn icon_button<'a, M: Clone + 'a>(
    i: Icon,
    size: f32,
    active: bool,
    on_press: Option<M>,
) -> button::Button<'a, M> {
    // No action means "not available yet": dim it.
    let color = if on_press.is_some() { ICON } else { TEXT_MUTED };
    button(icon(i, size, color))
        .padding(4)
        .style(style::ghost(active))
        .on_press_maybe(on_press)
}

/// Left-toolbar button: 32×32, icon centred, optional flyout triangle bottom-right.
pub fn tool_button<'a, M: Clone + 'a>(
    i: Icon,
    active: bool,
    has_group: bool,
    tip: &'a str,
    on_press: M,
) -> Element<'a, M> {
    let glyph = container(icon(i, 16.0, ICON)).center(Fill);
    let mut layers = stack![glyph];
    if has_group {
        layers = layers.push(
            container(icon(Icon::Corner, 4.0, TEXT_MUTED))
                .width(Fill)
                .height(Fill)
                .align_x(Horizontal::Right)
                .align_y(Vertical::Bottom)
                .padding(3),
        );
    }
    let b = button(layers)
        .width(32)
        .height(32)
        .padding(0)
        .style(style::ghost(active))
        .on_press(on_press);
    hover_tip(b, tip, tooltip::Position::Right)
}

/// Photoshop-style tooltip: appears after hovering for a moment.
pub fn hover_tip<'a, M: 'a>(
    content: impl Into<Element<'a, M>>,
    tip: &'a str,
    position: tooltip::Position,
) -> Element<'a, M> {
    tooltip(
        content,
        container(text(tip).size(TEXT_SIZE_SMALL).color(TEXT_PRIMARY))
            .padding(Padding::from([4, 8]))
            .style(|_| container::Style {
                background: Some(BG_PANEL_HEADER.into()),
                border: iced::Border { color: BORDER_SUBTLE, width: 1.0, radius: RADIUS.into() },
                shadow: iced::Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.4),
                    offset: iced::Vector::new(0.0, 2.0),
                    blur_radius: 8.0,
                },
                ..Default::default()
            }),
        position,
    )
    .gap(6)
    .delay(std::time::Duration::from_millis(600))
    .into()
}

/// Dock panel: tab strip header + padded content.
pub fn panel<'a, M: Clone + 'a>(
    tabs: &[&'a str],
    active: usize,
    on_tab: impl Fn(usize) -> M,
    content: impl Into<Element<'a, M>>,
    height: Length,
) -> Element<'a, M> {
    column![
        panel_header(tabs, active, on_tab),
        container(content).width(Fill).height(Fill).padding(10),
    ]
    .height(height)
    .into()
}

/// Just the tab strip, for panels that lay out their own content edge to edge.
pub fn panel_header<'a, M: Clone + 'a>(
    tabs: &[&'a str],
    active: usize,
    on_tab: impl Fn(usize) -> M,
) -> Element<'a, M> {
    let mut header = row![].align_y(Vertical::Center);
    for (i, name) in tabs.iter().enumerate() {
        let is_active = i == active;
        let tab_text = text(*name)
            .size(TEXT_SIZE_SMALL)
            .font(if is_active { bold() } else { iced::Font::DEFAULT });
        let tab = button(tab_text)
            .padding(Padding::from([6, 12]))
            .style(style::tab(is_active))
            .on_press(on_tab(i));
        // Active tab gets the 2px accent line on top, like the Figma component.
        let line = if is_active { ACCENT_PRIMARY } else { Color::TRANSPARENT };
        header = header.push(column![
            container(space()).width(Fill).height(2).style(style::fill(line)),
            tab
        ].width(Length::Shrink));
    }
    header = header
        .push(space::horizontal())
        .push(container(icon(Icon::Menu, 14.0, TEXT_SECONDARY)).padding(Padding::from([0, 8])));

    container(header)
        .width(Fill)
        .style(style::fill(BG_PANEL_HEADER))
        .into()
}

/// A panel's tab strip whose empty part is a grip (press it to start dragging the panel)
/// and whose menu icon is a button.
pub fn panel_header_grip<'a, M: Clone + 'a>(
    tabs: &[&'a str],
    active: usize,
    on_tab: impl Fn(usize) -> M,
    on_grab: M,
    on_menu: M,
) -> Element<'a, M> {
    let mut header = row![].align_y(Vertical::Center).height(30);
    for (i, name) in tabs.iter().enumerate() {
        let is_active = i == active;
        let tab_text = text(*name).size(TEXT_SIZE_SMALL).font(if is_active { bold() } else { iced::Font::DEFAULT });
        let tab = button(tab_text).padding(Padding::from([6, 12])).style(style::tab(is_active)).on_press(on_tab(i));
        let line = if is_active { ACCENT_PRIMARY } else { Color::TRANSPARENT };
        header = header.push(column![container(space()).width(Fill).height(2).style(style::fill(line)), tab].width(Length::Shrink));
    }
    header = header
        .push(
            mouse_area(container(space()).width(Fill).height(Fill))
                .on_press(on_grab)
                .interaction(iced::mouse::Interaction::Grab),
        )
        .push(
            button(icon(Icon::Menu, 14.0, TEXT_SECONDARY))
                .padding(Padding::from([4, 8]))
                .style(style::ghost(false))
                .on_press(on_menu),
        );
    container(header).width(Fill).style(style::fill(BG_PANEL_HEADER)).into()
}

/// One row of a layers panel. `thumb` and `name` are supplied by the app (an image and a
/// label or text field), so this stays independent of how pixels are stored. `mask` is the
/// layer mask's thumbnail, if it has one; `mask_targeted` outlines it instead of `thumb` to
/// show that edits go to the mask (Photoshop's targeted thumbnail).
#[allow(clippy::too_many_arguments)]
pub fn layer_row<'a, M: Clone + 'a>(
    name: impl Into<Element<'a, M>>,
    thumb: impl Into<Element<'a, M>>,
    mask: Option<Element<'a, M>>,
    mask_targeted: bool,
    visible: bool,
    locked: bool,
    selected: bool,
    on_select: M,
    on_toggle_visible: M,
    on_toggle_lock: M,
    on_context: M,
) -> Element<'a, M> {
    // Hidden layers show a crossed-out eye (dimmed), not an empty slot.
    let eye = button(
        container(if visible {
            icon(Icon::Eye, 16.0, ICON)
        } else {
            icon(Icon::EyeOff, 16.0, TEXT_MUTED)
        })
        .center(Length::Shrink),
    )
    .padding(2)
    .style(style::ghost(false))
    .on_press(on_toggle_visible);

    // Light backdrop so transparent thumbnails read as "empty". With a mask, the thumbnail
    // that edits go to is outlined.
    let has_mask = mask.is_some();
    let frame = move |content: Element<'a, M>, targeted: bool| {
        let outlined = selected && has_mask && targeted;
        container(content).width(40).height(28).center_x(40).center_y(28).style(move |_| container::Style {
            background: Some(Color::from_rgb8(0xD8, 0xD8, 0xD8).into()),
            border: if outlined {
                iced::Border { color: TEXT_PRIMARY, width: 2.0, radius: 2.0.into() }
            } else {
                iced::Border { color: BORDER_SUBTLE, width: 1.0, radius: 2.0.into() }
            },
            ..Default::default()
        })
    };

    let mut body = row![frame(thumb.into(), !mask_targeted)].spacing(8).height(Fill).align_y(Vertical::Center);
    if let Some(mask) = mask {
        body = body.push(frame(mask, mask_targeted));
    }
    let mut body = body.push(container(name).width(Fill));
    if locked {
        body = body.push(hover_tip(
            button(icon(Icon::Lock, 13.0, TEXT_SECONDARY))
                .padding(3)
                .style(style::ghost(false))
                .on_press(on_toggle_lock),
            "Unlock layer",
            tooltip::Position::Left,
        ));
    }
    // A mouse area rather than a button: buttons only fire on release, and the press has to
    // be seen immediately so the app can start a layer drag.
    let body = mouse_area(container(body).width(Fill).height(Fill).padding(Padding::from([0, 10]).left(4)))
        .on_press(on_select)
        .on_right_press(on_context)
        .interaction(iced::mouse::Interaction::Pointer);

    column![
        container(row![eye, body].spacing(4).align_y(Vertical::Center).height(40))
            .padding(Padding::ZERO.left(6))
            .style(style::fill(if selected { BG_SELECTED } else { BG_PANEL })),
        hline(BORDER_DEFAULT),
    ]
    .into()
}

pub fn bold() -> iced::Font {
    iced::Font {
        weight: iced::font::Weight::Semibold,
        ..iced::Font::DEFAULT
    }
}

/// One row of a dropdown / context menu.
pub enum MenuEntry<'a, M> {
    /// Label, shortcut hint, and the message (None = shown disabled).
    Item(&'a str, &'a str, Option<M>),
    /// An item with a leading icon (tool flyouts).
    IconItem(Icon, &'a str, &'a str, Option<M>),
    /// Opens a nested menu on hover (one level deep).
    Submenu(&'a str, Vec<MenuEntry<'a, M>>),
    Separator,
}

/// Fixed row heights, so a submenu can be placed next to its row.
pub const MENU_ITEM_HEIGHT: f32 = 26.0;
pub const MENU_SEPARATOR_HEIGHT: f32 = 9.0;
pub const MENU_PADDING: f32 = 4.0;

/// Height a list of entries takes in a menu.
pub fn menu_height<M>(entries: &[MenuEntry<'_, M>]) -> f32 {
    entries
        .iter()
        .map(|e| if matches!(e, MenuEntry::Separator) { MENU_SEPARATOR_HEIGHT } else { MENU_ITEM_HEIGHT })
        .sum::<f32>()
        + 2.0 * MENU_PADDING
}

/// A rendered menu, plus its open submenu (y offset of its row, and the submenu itself).
pub struct MenuView<'a, M> {
    pub main: Element<'a, M>,
    pub sub: Option<(f32, Element<'a, M>)>,
}

/// A Photoshop-style popup menu: dark card, items with right-aligned shortcuts, optional
/// submenus. `on_hover(Some(i))` fires when the pointer enters submenu row `i`, `None` for any
/// other row; the caller keeps `open_sub` and positions `MenuView::sub`.
pub fn menu<'a, M: Clone + 'a>(
    entries: Vec<MenuEntry<'a, M>>,
    width: f32,
    open_sub: Option<usize>,
    on_hover: impl Fn(Option<usize>) -> M + 'a,
) -> MenuView<'a, M> {
    menu_inner(entries, width, open_sub, &on_hover)
}

// Not generic over the closure, so building a submenu doesn't recurse through new types.
fn menu_inner<'a, M: Clone + 'a>(
    entries: Vec<MenuEntry<'a, M>>,
    width: f32,
    open_sub: Option<usize>,
    on_hover: &dyn Fn(Option<usize>) -> M,
) -> MenuView<'a, M> {
    let mut col = column![].width(width);
    let mut sub = None;
    let mut y = MENU_PADDING;
    for (i, entry) in entries.into_iter().enumerate() {
        let (lead, label_text, shortcut, msg, children) = match entry {
            MenuEntry::Separator => {
                col = col.push(
                    container(hline(BORDER_SUBTLE)).height(MENU_SEPARATOR_HEIGHT).center_y(MENU_SEPARATOR_HEIGHT),
                );
                y += MENU_SEPARATOR_HEIGHT;
                continue;
            }
            MenuEntry::Item(l, s, m) => (None, l, s, m, None),
            MenuEntry::IconItem(i, l, s, m) => (Some(i), l, s, m, None),
            MenuEntry::Submenu(l, children) => (None, l, "›", None, Some(children)),
        };
        let is_sub = children.is_some();
        let open = is_sub && open_sub == Some(i);
        let enabled = msg.is_some() || is_sub;
        let color = if enabled { TEXT_PRIMARY } else { TEXT_MUTED };
        let mut content = row![].spacing(8).align_y(Vertical::Center);
        if let Some(i) = lead {
            content = content.push(icon(i, 14.0, if enabled { ICON } else { TEXT_MUTED }));
        }
        content = content
            .push(text(label_text).size(TEXT_SIZE).color(color).width(Fill))
            .push(text(shortcut).size(if is_sub { TEXT_SIZE + 2.0 } else { TEXT_SIZE_SMALL }).color(TEXT_MUTED));
        let press = if is_sub { Some(on_hover(Some(i))) } else { msg };
        let item = button(content)
            .width(Fill)
            .height(MENU_ITEM_HEIGHT)
            .padding(Padding::from([5, 12]))
            .style(move |_, status| button::Style {
                background: (enabled && (open || matches!(status, button::Status::Hovered | button::Status::Pressed)))
                    .then(|| ACCENT_PRIMARY.into()),
                text_color: TEXT_PRIMARY,
                border: iced::border::rounded(3),
                ..Default::default()
            })
            .on_press_maybe(press);
        col = col.push(mouse_area(item).on_enter(on_hover(is_sub.then_some(i))));
        if open && let Some(children) = children {
            // Hovering inside the submenu keeps it open.
            let keep = on_hover(Some(i));
            sub = Some((y - MENU_PADDING, menu_inner(children, width, None, &move |_| keep.clone()).main));
        }
        y += MENU_ITEM_HEIGHT;
    }
    let main = container(col)
        .padding(MENU_PADDING)
        .style(|_| container::Style {
            background: Some(BG_PANEL_HEADER.into()),
            border: iced::Border { color: BORDER_SUBTLE, width: 1.0, radius: 6.0.into() },
            shadow: iced::Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.5),
                offset: iced::Vector::new(0.0, 6.0),
                blur_radius: 20.0,
            },
            ..Default::default()
        })
        .into();
    MenuView { main, sub }
}
