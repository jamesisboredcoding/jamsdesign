//! Style functions for iced widgets, all built from [`tokens`].

use iced::widget::{button, container};
use iced::widget::{pick_list as pl, slider as sl};
use iced::{Background, Border, Color, Theme, border};

use crate::tokens::*;

pub fn fill(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(color.into()),
        text_color: Some(TEXT_PRIMARY),
        ..Default::default()
    }
}

pub fn input(_: &Theme) -> container::Style {
    container::Style {
        background: Some(BG_INPUT.into()),
        border: Border {
            color: BORDER_SUBTLE,
            width: 1.0,
            radius: RADIUS.into(),
        },
        text_color: Some(TEXT_PRIMARY),
        ..Default::default()
    }
}

/// Flat button that only shows a background on hover / when `active`.
pub fn ghost(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let background = match status {
            _ if active => Some(ACCENT_ACTIVE_TOOL),
            button::Status::Hovered | button::Status::Pressed => Some(BG_HOVER),
            _ => None,
        };
        button::Style {
            background: background.map(Background::from),
            text_color: TEXT_PRIMARY,
            border: border::rounded(4),
            ..Default::default()
        }
    }
}

/// Title-bar window buttons: square, hover grey (or red for close).
pub fn window_control(danger: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hover = if danger { DANGER } else { BG_HOVER };
        let background = match status {
            button::Status::Hovered => Some(hover),
            button::Status::Pressed => Some(Color { a: 0.8, ..hover }),
            _ => None,
        };
        button::Style {
            background: background.map(Background::from),
            text_color: TEXT_PRIMARY,
            ..Default::default()
        }
    }
}

pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    let base = ACCENT_PRIMARY;
    let background = match status {
        button::Status::Hovered => Color { a: 0.85, ..base },
        button::Status::Pressed => Color { a: 0.7, ..base },
        _ => base,
    };
    button::Style {
        background: Some(background.into()),
        text_color: TEXT_PRIMARY,
        border: border::rounded(4),
        ..Default::default()
    }
}

pub fn tab(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: Some(
            match (active, status) {
                (true, _) => BG_PANEL,
                (false, button::Status::Hovered) => BG_HOVER,
                _ => BG_PANEL_HEADER,
            }
            .into(),
        ),
        text_color: if active { TEXT_PRIMARY } else { TEXT_SECONDARY },
        ..Default::default()
    }
}

pub fn row(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| button::Style {
        background: Some(
            match (selected, status) {
                (true, _) => BG_SELECTED,
                (false, button::Status::Hovered) => Color::from_rgb8(0x33, 0x33, 0x33),
                _ => BG_PANEL,
            }
            .into(),
        ),
        text_color: TEXT_PRIMARY,
        ..Default::default()
    }
}

pub fn slider(_: &Theme, status: sl::Status) -> sl::Style {
    let handle = match status {
        sl::Status::Active => Color::from_rgb8(0xEB, 0xEB, 0xEB),
        _ => Color::WHITE,
    };
    sl::Style {
        rail: sl::Rail {
            backgrounds: (ACCENT_PRIMARY.into(), BG_INPUT.into()),
            width: 4.0,
            border: border::rounded(2),
        },
        handle: sl::Handle {
            shape: sl::HandleShape::Circle { radius: 5.0 },
            background: handle.into(),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}

pub fn pick_list(_: &Theme, status: pl::Status) -> pl::Style {
    pl::Style {
        text_color: TEXT_PRIMARY,
        placeholder_color: TEXT_MUTED,
        handle_color: TEXT_SECONDARY,
        background: BG_INPUT.into(),
        border: Border {
            color: match status {
                pl::Status::Hovered | pl::Status::Opened { .. } => TEXT_MUTED,
                _ => BORDER_SUBTLE,
            },
            width: 1.0,
            radius: RADIUS.into(),
        },
    }
}
