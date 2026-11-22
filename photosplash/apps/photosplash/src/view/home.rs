//! The Home screen (Photoshop's): New and Open, and the files opened or saved lately as
//! thumbnails. Shown while nothing is open, or from the Home button in the options bar.

use std::time::SystemTime;

use iced::alignment::Vertical;
use iced::widget::{button, column, container, image, mouse_area, row, scrollable, space, stack, text};
use iced::{Alignment, ContentFit, Element, Fill, Padding};
use jd_ui::widget::bold;
use jd_ui::{Icon, icon, style, tokens::*};

use crate::app::{Message, Photo, RecentItem};

const CARD: f32 = 200.0;
const THUMB_H: f32 = 140.0;

fn card(item: &RecentItem, now: SystemTime) -> Element<'_, Message> {
    let picture: Element<'_, Message> = match &item.thumb {
        Some(h) => image(h.clone()).content_fit(ContentFit::Contain).width(Fill).height(Fill).into(),
        None => icon(Icon::Image, 32.0, TEXT_MUTED).into(),
    };
    let name = item.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let when = item.modified.map(|t| crate::recent::ago(t, now)).unwrap_or_default();
    let kind = item.path.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
    let body = column![
        container(picture).width(Fill).height(THUMB_H).center_x(Fill).center_y(THUMB_H).padding(6).style(style::input),
        text(name).size(TEXT_SIZE).wrapping(text::Wrapping::None),
        text(format!("{kind} · {when}")).size(TEXT_SIZE_SMALL).color(TEXT_MUTED),
    ]
    .spacing(6);
    // Right-click takes it off the list.
    mouse_area(
        button(body)
            .width(CARD)
            .padding(8)
            .style(style::ghost(false))
            .on_press(Message::Open(vec![item.path.clone()])),
    )
    .on_right_press(Message::ForgetRecent(item.path.clone()))
    .into()
}

pub fn view(app: &Photo) -> Element<'_, Message> {
    let new = button(text("New…").size(TEXT_SIZE))
        .padding([7, 18])
        .style(style::ghost(true))
        .on_press(Message::NewDocDialog);
    let open = button(text("Open…").size(TEXT_SIZE).font(bold()))
        .padding([7, 18])
        .style(style::primary)
        .on_press(Message::OpenDialog);
    let subtitle = if app.loading > 0 { "Opening…" } else { "Open something recent, or start something new." };
    let header = row![
        column![
            text("Welcome to Photosplash").size(22).font(bold()),
            text(subtitle).size(TEXT_SIZE).color(TEXT_SECONDARY),
        ]
        .spacing(6),
        space::horizontal(),
        new,
        open,
    ]
    .spacing(10)
    .align_y(Vertical::Center);

    let now = SystemTime::now();
    let recent: Element<'_, Message> = if app.recent.is_empty() {
        container(
            column![
                icon(Icon::Image, 40.0, TEXT_MUTED),
                text("No recent files yet").size(TEXT_SIZE).color(TEXT_SECONDARY),
                text("Files you open or save show up here. Ctrl+N new · Ctrl+O open").size(TEXT_SIZE_SMALL).color(TEXT_MUTED),
            ]
            .spacing(10)
            .align_x(Alignment::Center),
        )
        .width(Fill)
        .padding(Padding::from([60, 0]))
        .center_x(Fill)
        .into()
    } else {
        row(app.recent.iter().map(|r| card(r, now))).spacing(12).wrap().vertical_spacing(12).into()
    };
    let clear = button(text("Clear").size(TEXT_SIZE_SMALL))
        .padding([3, 10])
        .style(style::ghost(false))
        .on_press_maybe((!app.recent.is_empty()).then_some(Message::ClearRecent));
    let page = column![
        header,
        row![
            text("Recent").size(TEXT_SIZE).font(bold()),
            text("right-click one to remove it").size(TEXT_SIZE_SMALL).color(TEXT_MUTED),
            space::horizontal(),
            clear,
        ]
        .spacing(10)
        .align_y(Vertical::Center),
        recent,
    ]
    .spacing(24)
    .padding(36)
    .max_width(1180);

    stack![
        super::canvas::sizer(app),
        container(scrollable(container(page).center_x(Fill)).height(Fill)).width(Fill).height(Fill).style(style::fill(BG_CANVAS)),
    ]
    .into()
}
