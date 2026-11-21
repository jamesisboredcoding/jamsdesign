//! Project panel: the project's media bin (thumbnail grid or list), with search.

use iced::alignment::{Horizontal, Vertical};
use iced::widget::{button, column, container, image, mouse_area, row, scrollable, space, stack, text, text_input};
use iced::{Border, Color, ContentFit, Element, Fill, Length, Padding, Theme};
use jd_ui::widget::{icon_button, label, panel_header};
use jd_ui::{Icon, icon, style, tokens::*};

use super::{panel, toggle_button};
use crate::app::{Editor, Message, timecode};
use crate::media::{Kind, MediaItem};

const TABS: &[&str] = &["Project: Untitled", "Media Browser", "Effects", "History"];
const TILE_WIDTH: f32 = 112.0;
const TILE_HEIGHT: f32 = 63.0;
const COLUMNS: usize = 3;

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
    let n = app.media.len();
    let count = match (n, app.selected_media) {
        (1, _) => "1 item".to_owned(),
        (n, Some(_)) => format!("1 of {n} items selected"),
        (n, None) => format!("{n} items"),
    };
    let header = row![
        icon(Icon::Folder, 12.0, TEXT_SECONDARY),
        text(format!("{}.jdv", app.project_name)).size(TEXT_SIZE_SMALL),
        space::horizontal(),
        label(count).size(10),
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
    let mut top = column![header, container(search).padding(Padding::from([0, 10]).bottom(6))];
    if let Some(notice) = &app.notice {
        top = top.push(container(text(notice).size(10).color(DANGER)).padding(Padding::from([0, 10]).bottom(6)));
    }

    let items: Vec<&MediaItem> = app.media.iter().filter(|m| m.matches(&app.search)).collect();
    let content: Element<'_, Message> = if app.media.is_empty() {
        empty_bin()
    } else if items.is_empty() {
        hint("No media matches the search")
    } else if app.grid_view {
        let mut grid = column![].spacing(10);
        for chunk in items.chunks(COLUMNS) {
            let mut r = row![].spacing(12);
            for m in chunk {
                r = r.push(tile(app, m));
            }
            grid = grid.push(r);
        }
        scrollable(container(grid).padding(10)).height(Fill).into()
    } else {
        let mut list = column![];
        for m in items {
            list = list.push(list_row(app, m));
        }
        scrollable(list).height(Fill).into()
    };

    let footer = container(
        row![
            toggle_button(Icon::List, !app.grid_view, "List View", Some(Message::GridView(false))),
            toggle_button(Icon::Grid, app.grid_view, "Icon View", Some(Message::GridView(true))),
            space::horizontal(),
            icon_button(Icon::Folder, 14.0, false, None),
            icon_button(Icon::Plus, 14.0, false, Some(Message::ImportDialog)),
            icon_button(Icon::Trash, 14.0, false, None),
        ]
        .spacing(4)
        .align_y(Vertical::Center),
    )
    .padding(Padding::from([0, 6]))
    .height(30)
    .align_y(Vertical::Center)
    .style(style::fill(BG_PANEL_HEADER));

    column![top, content, footer].into()
}

fn empty_bin<'a>() -> Element<'a, Message> {
    let import = button(text("Import Media...").size(TEXT_SIZE_SMALL))
        .padding(Padding::from([5, 12]))
        .style(style::primary)
        .on_press(Message::ImportDialog);
    container(
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
    .center(Fill)
    .into()
}

/// Thumbnail, or an icon standing in for one.
fn picture<'a>(m: &'a MediaItem, size: f32) -> Element<'a, Message> {
    if let Some(handle) = &m.thumb {
        return image(handle.clone()).content_fit(ContentFit::Cover).width(Fill).height(Fill).into();
    }
    let (glyph, bg): (Element<'a, Message>, Color) = match m.kind() {
        Kind::Audio => (icon(Icon::Music, size, ICON).into(), Color::from_rgb8(0x24, 0x45, 0x3A)),
        Kind::Pending => (text("...").size(TEXT_SIZE).color(TEXT_MUTED).into(), BG_CANVAS),
        Kind::Unreadable => (text("!").size(size).color(DANGER).into(), BG_CANVAS),
        Kind::Video | Kind::Still => (icon(Icon::Film, size, TEXT_MUTED).into(), BG_CANVAS),
    };
    container(glyph).center(Fill).style(style::fill(bg)).into()
}

fn duration_label(app: &Editor, m: &MediaItem) -> String {
    match m.kind() {
        Kind::Pending => String::new(),
        Kind::Unreadable => "unreadable".into(),
        _ => timecode(m.duration(), m.fps(app.sequence.rate.timecode_base())),
    }
}

fn tile<'a>(app: &'a Editor, m: &'a MediaItem) -> Element<'a, Message> {
    let selected = app.selected_media == Some(m.id);
    let dur = duration_label(app, m);
    let mut layers = stack![picture(m, 22.0)];
    if !dur.is_empty() {
        layers = layers.push(
            container(
                container(text(dur).size(8).color(TEXT_PRIMARY))
                    .padding(Padding::from([1, 3]))
                    .style(|_| container::Style {
                        background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.6).into()),
                        border: iced::border::rounded(2),
                        ..Default::default()
                    }),
            )
            .width(Fill)
            .height(Fill)
            .align_x(Horizontal::Right)
            .align_y(Vertical::Bottom)
            .padding(4),
        );
    }
    let thumb = container(layers)
        .width(TILE_WIDTH)
        .height(TILE_HEIGHT)
        .clip(true)
        .style(move |_| container::Style {
            border: Border {
                color: if selected { ACCENT_PRIMARY } else { BORDER_SUBTLE },
                width: if selected { 2.0 } else { 1.0 },
                radius: RADIUS.into(),
            },
            ..Default::default()
        });
    let name = container(text(&m.name).size(10).wrapping(text::Wrapping::None))
        .width(TILE_WIDTH)
        .clip(true);
    mouse_area(column![thumb, name].spacing(4).width(Length::Fixed(TILE_WIDTH)))
        .on_press(Message::SelectMedia(m.id))
        .on_double_click(Message::OpenInSource(m.id))
        .into()
}

fn list_row<'a>(app: &'a Editor, m: &'a MediaItem) -> Element<'a, Message> {
    let selected = app.selected_media == Some(m.id);
    let kind_icon = match m.kind() {
        Kind::Audio => Icon::Music,
        Kind::Still => Icon::Image,
        _ => Icon::Film,
    };
    let detail = m
        .info
        .as_ref()
        .and_then(|i| i.video.as_ref())
        .map(|v| if v.still { format!("{}×{}", v.width, v.height) } else { format!("{:.2} fps", v.rate.as_f64()) })
        .unwrap_or_default();
    let body = row![
        icon(kind_icon, 13.0, ICON),
        container(text(&m.name).size(TEXT_SIZE_SMALL).wrapping(text::Wrapping::None)).width(Fill).clip(true),
        text(detail).size(10).color(TEXT_MUTED),
        text(duration_label(app, m)).size(10).color(TEXT_SECONDARY).width(70),
    ]
    .spacing(8)
    .align_y(Vertical::Center);
    mouse_area(
        container(body)
            .padding(Padding::from([0, 10]))
            .height(26)
            .width(Fill)
            .align_y(Vertical::Center)
            .style(style::fill(if selected { BG_SELECTED } else { BG_APP })),
    )
    .on_press(Message::SelectMedia(m.id))
    .on_double_click(Message::OpenInSource(m.id))
    .into()
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
