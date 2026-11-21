//! Timeline tools, with Premiere's shortcuts.

use jd_ui::Icon;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Selection,
    TrackSelectForward,
    RippleEdit,
    Razor,
    Slip,
    Pen,
    Hand,
    Type,
}

impl Tool {
    /// Toolbar order, in groups separated by a divider.
    pub const GROUPS: &[&[Tool]] = &[
        &[Tool::Selection, Tool::TrackSelectForward],
        &[Tool::RippleEdit, Tool::Razor, Tool::Slip],
        &[Tool::Pen, Tool::Hand, Tool::Type],
    ];

    pub fn icon(self) -> Icon {
        match self {
            Tool::Selection => Icon::Select,
            Tool::TrackSelectForward => Icon::TrackSelect,
            Tool::RippleEdit => Icon::Ripple,
            Tool::Razor => Icon::Razor,
            Tool::Slip => Icon::Slip,
            Tool::Pen => Icon::Pen,
            Tool::Hand => Icon::Hand,
            Tool::Type => Icon::Type,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Tool::Selection => "v",
            Tool::TrackSelectForward => "a",
            Tool::RippleEdit => "b",
            Tool::Razor => "c",
            Tool::Slip => "y",
            Tool::Pen => "p",
            Tool::Hand => "h",
            Tool::Type => "t",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Tool::Selection => "Selection Tool (V)",
            Tool::TrackSelectForward => "Track Select Forward Tool (A)",
            Tool::RippleEdit => "Ripple Edit Tool (B)",
            Tool::Razor => "Razor Tool (C)",
            Tool::Slip => "Slip Tool (Y)",
            Tool::Pen => "Pen Tool (P)",
            Tool::Hand => "Hand Tool (H)",
            Tool::Type => "Type Tool (T)",
        }
    }

    pub fn from_key(key: &str) -> Option<Tool> {
        Tool::GROUPS.iter().flat_map(|g| g.iter()).copied().find(|t| t.key() == key)
    }
}
