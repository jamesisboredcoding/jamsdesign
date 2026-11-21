use iced::Theme;
use iced::theme::Palette;

/// Colour tokens. Names match the Figma variables (`bg/app` → `BG_APP`, ...).
/// Change them here and every app follows.
pub mod tokens {
    use iced::Color;

    pub const BG_APP: Color = Color::from_rgb8(0x1E, 0x1E, 0x1E);
    pub const BG_PANEL: Color = Color::from_rgb8(0x2B, 0x2B, 0x2B);
    pub const BG_PANEL_HEADER: Color = Color::from_rgb8(0x25, 0x25, 0x25);
    pub const BG_CANVAS: Color = Color::from_rgb8(0x14, 0x14, 0x14);
    pub const BG_INPUT: Color = Color::from_rgb8(0x1A, 0x1A, 0x1A);
    pub const BG_HOVER: Color = Color::from_rgb8(0x3A, 0x3A, 0x3A);
    pub const BG_SELECTED: Color = Color::from_rgb8(0x3D, 0x4F, 0x6B);
    pub const BORDER_DEFAULT: Color = Color::from_rgb8(0x11, 0x11, 0x11);
    pub const BORDER_SUBTLE: Color = Color::from_rgb8(0x3A, 0x3A, 0x3A);
    pub const TEXT_PRIMARY: Color = Color::from_rgb8(0xE6, 0xE6, 0xE6);
    pub const TEXT_SECONDARY: Color = Color::from_rgb8(0xA0, 0xA0, 0xA0);
    pub const TEXT_MUTED: Color = Color::from_rgb8(0x6E, 0x6E, 0x6E);
    pub const ACCENT_PRIMARY: Color = Color::from_rgb8(0x4C, 0x8D, 0xFF);
    pub const ACCENT_ACTIVE_TOOL: Color = Color::from_rgb8(0x45, 0x45, 0x45);
    pub const ICON: Color = Color::from_rgb8(0xCF, 0xCF, 0xCF);

    pub const DANGER: Color = Color::from_rgb8(0xE5, 0x59, 0x4D);
    pub const SUCCESS: Color = Color::from_rgb8(0x4C, 0xC3, 0x8A);
    pub const WARNING: Color = Color::from_rgb8(0xF2, 0xB1, 0x3F);

    pub const TEXT_SIZE: f32 = 12.0;
    pub const TEXT_SIZE_SMALL: f32 = 11.0;
    pub const RADIUS: f32 = 3.0;
}

pub fn theme() -> Theme {
    use tokens::*;
    Theme::custom(
        "jamsdesign Dark",
        Palette {
            background: BG_APP,
            text: TEXT_PRIMARY,
            primary: ACCENT_PRIMARY,
            success: SUCCESS,
            warning: WARNING,
            danger: DANGER,
        },
    )
}
