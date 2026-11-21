mod adjust_dialog;
pub mod canvas;
mod chrome;
mod color;
mod dialog;
mod dock;
mod menus;

use iced::mouse::Interaction;
use iced::widget::{column, container, mouse_area, row, space, stack};
use iced::window::Direction;
use iced::{Color, Element, Fill, Length};
use jd_core::Hsv;
use jd_ui::{
    style,
    tokens::*,
    widget::{hline, vline},
};

pub use canvas::Ants;

/// Number of title-bar menus (the app keeps one measured position per menu).
pub const MENU_COUNT: usize = chrome::MENUS.len();

use crate::app::{Message, Photo};

pub fn view(app: &Photo) -> Element<'_, Message> {
    let workspace = row![
        chrome::toolbar(app),
        vline(Fill, BORDER_DEFAULT),
        column![chrome::document_tabs(app), canvas::view(app), chrome::status_bar(app)]
            .width(Fill)
            .height(Fill),
        vline(Fill, BORDER_DEFAULT),
        dock::strip_panel(app),
        chrome::collapsed_panels(app),
        vline(Fill, BORDER_DEFAULT),
        dock::view(app),
    ]
    .height(Fill);

    let content = container(column![
        chrome::title_bar(app),
        hline(BORDER_DEFAULT),
        chrome::options_bar(app),
        hline(BORDER_DEFAULT),
        workspace
    ])
    .width(Fill)
    .height(Fill)
    .style(style::fill(BG_APP));

    let mut layers = stack![content];
    if !(app.maximized || app.fullscreen) {
        layers = layers.push(resize_edges());
    }
    if let Some(dialog) = adjust_dialog::view(app) {
        layers = layers.push(dialog);
    }
    if let Some(popup) = menus::popup(app) {
        layers = layers.push(popup);
    }
    if let Some(new) = dialog::new_document(app) {
        layers = layers.push(new);
    }
    if let Some(confirm) = dialog::confirm(app) {
        layers = layers.push(confirm);
    }
    layers.into()
}

/// Invisible grab strips along the window edges. Without system decorations the compositor
/// gives us no resize border, so we start the resize ourselves.
fn resize_edges<'a>() -> Element<'a, Message> {
    const EDGE: f32 = 5.0;
    let grip = |w: Length, h: Length, dir: Direction, cursor: Interaction| -> Element<'a, Message> {
        mouse_area(space().width(w).height(h))
            .interaction(cursor)
            .on_press(Message::ResizeWindow(dir))
            .into()
    };
    let e = Length::Fixed(EDGE);
    column![
        row![
            grip(e, e, Direction::NorthWest, Interaction::ResizingDiagonallyDown),
            grip(Fill, e, Direction::North, Interaction::ResizingVertically),
            grip(e, e, Direction::NorthEast, Interaction::ResizingDiagonallyUp),
        ],
        row![
            grip(e, Fill, Direction::West, Interaction::ResizingHorizontally),
            space().width(Fill).height(Fill),
            grip(e, Fill, Direction::East, Interaction::ResizingHorizontally),
        ]
        .height(Fill),
        row![
            grip(e, e, Direction::SouthWest, Interaction::ResizingDiagonallyUp),
            grip(Fill, e, Direction::South, Interaction::ResizingVertically),
            grip(e, e, Direction::SouthEast, Interaction::ResizingDiagonallyDown),
        ],
    ]
    .into()
}

pub fn to_color(hsv: Hsv) -> Color {
    let [r, g, b] = hsv.to_srgb();
    Color::from_rgb(r, g, b)
}
