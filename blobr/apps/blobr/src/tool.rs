//! Canvas tools and their shortcuts (Figma's).

use jd_ui::Icon;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Move,
    Frame,
    Rectangle,
    Ellipse,
    Line,
    Pen,
    Text,
    Comment,
    Hand,
    Zoom,
}

impl Tool {
    /// Toolbar order; `None` marks a divider.
    pub const BAR: &[Option<Tool>] = &[
        Some(Tool::Move),
        Some(Tool::Frame),
        Some(Tool::Rectangle),
        Some(Tool::Ellipse),
        Some(Tool::Line),
        None,
        Some(Tool::Pen),
        Some(Tool::Text),
        Some(Tool::Comment),
        None,
        Some(Tool::Hand),
        Some(Tool::Zoom),
    ];

    pub fn icon(self) -> Icon {
        match self {
            Tool::Move => Icon::Select,
            Tool::Frame => Icon::Frame,
            Tool::Rectangle => Icon::Shape,
            Tool::Ellipse => Icon::EllipseShape,
            Tool::Line => Icon::Line,
            Tool::Pen => Icon::Pen,
            Tool::Text => Icon::Type,
            Tool::Comment => Icon::Comment,
            Tool::Hand => Icon::Hand,
            Tool::Zoom => Icon::Zoom,
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            Tool::Move => "Move (V)",
            Tool::Frame => "Frame (F): drag, or pick a device size in the Design panel",
            Tool::Rectangle => "Rectangle (R)",
            Tool::Ellipse => "Ellipse (O)",
            Tool::Line => "Line (L): Shift keeps to 45 degree steps",
            Tool::Pen => "Pen (P): click for corners, drag for curves, Enter to finish",
            Tool::Text => "Text (T)",
            Tool::Comment => "Comment (C): click to pin a comment",
            Tool::Hand => "Hand (H, or hold Space)",
            Tool::Zoom => "Zoom (Z): click to zoom in, Alt-click out, drag to zoom to an area",
        }
    }

    pub fn from_key(c: &str) -> Option<Tool> {
        Some(match c {
            "v" => Tool::Move,
            "f" | "a" => Tool::Frame,
            "r" => Tool::Rectangle,
            "o" => Tool::Ellipse,
            "l" => Tool::Line,
            "p" => Tool::Pen,
            "t" => Tool::Text,
            "c" => Tool::Comment,
            "h" => Tool::Hand,
            "z" => Tool::Zoom,
            _ => return None,
        })
    }

    /// Tools that work by dragging out an area (or a line) on the canvas.
    pub fn creates(self) -> bool {
        matches!(self, Tool::Frame | Tool::Rectangle | Tool::Ellipse | Tool::Line | Tool::Text | Tool::Zoom)
    }
}
