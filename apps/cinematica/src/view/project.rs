//! Project panel: the project's media bin (list or thumbnail grid), with search.

use iced::alignment::Vertical;
use iced::widget::{button, column, container, row, space, text, text_input};
use iced::{Border, Element, Fill, Padding, Theme};
use jd_ui::widget::{icon_button, label, panel_header};
use jd_ui::{Icon, icon, style, tokens::*};

use super::{panel, toggle_button};
use crate::app::{Editor, Message};

const TABS: &[&str] = &["Project: Untitled", "Media Browser", "Effects", "History"];

pub fn view(app: &Editor) -> Element<'_, Message> {
    let body: Element<'_, Message> = match app.project_tab {
        0 => bin(app),
        1 => hint("Browse folders and preview media before importing"),
        2 => hint("Video and audio effects and transitions, ready to drag onto clips"),
        _ => hint("Every edit, newest last; click one to go back to it"),
    };
    panel(column![panel_header(TABS, app.project_tab, Message::ProjectTab), body])
        .width(380)
        .height(Fill)
        .into()
}

fn hint<'a>(s: &'a str) -> Element<'a, Message> {
    container(text(s).size(TEXT_SIZE_SMALL).color(TEXT_MUTED)).center(Fill).padding(20).into()
}

fn bin(app: &Editor) -> Element<'_, Message> {
    let header = row![
        icon(Icon::Folder, 12.0, TEXT_SECONDARY),
        text(format!("{}.jdv", app.project_name)).size(TEXT_SIZE_SMALL),
        space::horizontal(),
        label("0 items").size(10),
    ]
    .spacing(6)
    .align_y(Vertical::Center)
    .padding(Padding::from([0, 10]))
    .height(28);

    let search = text_input("Search", &app.search)
        .on_input(Message::Search)
        .size(TEXT_SIZE_SMALL)
        .padding(Padding::from([4, 8]))
        .style(input_style);
    let search = container(search).padding(Padding::from([0, 10]).bottom(6));

    // Import arrives with M1 (media decoding).
    let import = button(text("Import Media...").size(TEXT_SIZE_SMALL))
        .padding(Padding::from([5, 12]))
        .style(style::primary);
    let empty = container(
        column![
            icon(Icon::Film, 32.0, TEXT_MUTED),
            text("Import media to start").size(TEXT_SIZE).color(TEXT_SECONDARY),
            label("Ctrl+I, or drag files here").size(10).color(TEXT_MUTED),
            space().height(4),
            import,
        ]
        .spacing(6)
        .align_x(iced::Alignment::Center),
    )
    .center(Fill);

    let footer = container(
        row![
            toggle_button(Icon::List, !app.grid_view, "List View", Some(Message::GridView(false))),
            toggle_button(Icon::Grid, app.grid_view, "Icon View", Some(Message::GridView(true))),
            space::horizontal(),
            icon_button(Icon::Folder, 14.0, false, None),
            icon_button(Icon::Plus, 14.0, false, None),
            icon_button(Icon::Trash, 14.0, false, None),
        ]
        .spacing(4)
        .align_y(Vertical::Center),
    )
    .padding(Padding::from([0, 6]))
    .height(30)
    .align_y(Vertical::Center)
    .style(style::fill(BG_PANEL_HEADER));

    column![header, search, empty, footer].into()
}

fn input_style(_: &Theme, status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: BG_INPUT.into(),
        border: Border {
            color: if matches!(status, text_input::Status::Focused { .. }) { ACCENT_PRIMARY } else { BORDER_SUBTLE },
            width: 1.0,
            radius: RADIUS.into(),
        },
        icon: TEXT_SECONDARY,
        placeholder: TEXT_MUTED,
        value: TEXT_PRIMARY,
        selection: BG_SELECTED,
    }
}
