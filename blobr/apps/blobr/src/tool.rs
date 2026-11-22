//! Canvas tools and their shortcuts (Figma's).

use jd_ui::Icon;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Move,
    Frame,
    Rectangle,
    Ellipse,
    Text,
    Hand,
}

impl Tool {
    /// Toolbar order; `None` marks a divider.
    pub const BAR: &[Option<Tool>] = &[
        Some(Tool::Move),
        Some(Tool::Frame),
        Some(Tool::Rectangle),
        Some(Tool::Ellipse),
        None,
        Some(Tool::Text),
        None,
        Some(Tool::Hand),
    ];

    pub fn icon(self) -> Icon {
        match self {
            Tool::Move => Icon::Select,
            Tool::Frame => Icon::Frame,
            Tool::Rectangle => Icon::Shape,
            Tool::Ellipse => Icon::EllipseShape,
            Tool::Text => Icon::Type,
            Tool::Hand => Icon::Hand,
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            Tool::Move => "Move (V)",
            Tool::Frame => "Frame (F)",
            Tool::Rectangle => "Rectangle (R)",
            Tool::Ellipse => "Ellipse (O)",
            Tool::Text => "Text (T)",
            Tool::Hand => "Hand (H, or hold Space)",
        }
    }

    pub fn from_key(c: &str) -> Option<Tool> {
        Some(match c {
            "v" => Tool::Move,
            "f" | "a" => Tool::Frame,
            "r" => Tool::Rectangle,
            "o" => Tool::Ellipse,
            "t" => Tool::Text,
            "h" => Tool::Hand,
            _ => return None,
        })
    }

    /// Tools that draw a new node by dragging.
    pub fn creates(self) -> bool {
        matches!(self, Tool::Frame | Tool::Rectangle | Tool::Ellipse | Tool::Text)
    }
}
