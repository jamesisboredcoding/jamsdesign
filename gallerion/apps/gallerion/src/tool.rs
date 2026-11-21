use jd_ui::Icon;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    Move,
    Marquee,
    EllipticalMarquee,
    Lasso,
    PolygonalLasso,
    Wand,
    Crop,
    Eyedropper,
    Heal,
    Brush,
    Stamp,
    Eraser,
    Gradient,
    Dodge,
    Burn,
    Pen,
    Type,
    Select,
    Shape,
    EllipseShape,
    Hand,
    Zoom,
}

/// Toolbar layout: groups (separated by dividers) of slots. A slot shows one tool at a time;
/// right-click it for the others, or press Shift + its shortcut to cycle (like Photoshop).
pub const GROUPS: &[&[&[Tool]]] = &[
    &[
        &[Tool::Move],
        &[Tool::Marquee, Tool::EllipticalMarquee],
        &[Tool::Lasso, Tool::PolygonalLasso],
        &[Tool::Wand],
        &[Tool::Crop],
    ],
    &[
        &[Tool::Eyedropper],
        &[Tool::Heal],
        &[Tool::Brush],
        &[Tool::Stamp],
        &[Tool::Eraser],
        &[Tool::Gradient],
        &[Tool::Dodge, Tool::Burn],
    ],
    &[&[Tool::Pen], &[Tool::Type], &[Tool::Select], &[Tool::Shape, Tool::EllipseShape]],
    &[&[Tool::Hand], &[Tool::Zoom]],
];

/// All slots in toolbar order.
pub fn slots() -> impl Iterator<Item = &'static [Tool]> {
    GROUPS.iter().flat_map(|g| g.iter().copied())
}

impl Tool {
    pub fn icon(self) -> Icon {
        match self {
            Tool::Move => Icon::Move,
            Tool::Marquee => Icon::Marquee,
            Tool::EllipticalMarquee => Icon::MarqueeEllipse,
            Tool::Lasso => Icon::Lasso,
            Tool::PolygonalLasso => Icon::PolygonLasso,
            Tool::Wand => Icon::Wand,
            Tool::Crop => Icon::Crop,
            Tool::Eyedropper => Icon::Eyedropper,
            Tool::Heal => Icon::Heal,
            Tool::Brush => Icon::Brush,
            Tool::Stamp => Icon::Stamp,
            Tool::Eraser => Icon::Eraser,
            Tool::Gradient => Icon::Gradient,
            Tool::Dodge => Icon::Dodge,
            Tool::Burn => Icon::Burn,
            Tool::Pen => Icon::Pen,
            Tool::Type => Icon::Type,
            Tool::Select => Icon::Select,
            Tool::Shape => Icon::Shape,
            Tool::EllipseShape => Icon::EllipseShape,
            Tool::Hand => Icon::Hand,
            Tool::Zoom => Icon::Zoom,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Tool::Move => "Move",
            Tool::Marquee => "Rectangular Marquee",
            Tool::EllipticalMarquee => "Elliptical Marquee",
            Tool::Lasso => "Lasso",
            Tool::PolygonalLasso => "Polygonal Lasso",
            Tool::Wand => "Magic Wand",
            Tool::Crop => "Crop",
            Tool::Eyedropper => "Eyedropper",
            Tool::Heal => "Healing Brush",
            Tool::Brush => "Brush",
            Tool::Stamp => "Clone Stamp",
            Tool::Eraser => "Eraser",
            Tool::Gradient => "Gradient",
            Tool::Dodge => "Dodge",
            Tool::Burn => "Burn",
            Tool::Pen => "Pen",
            Tool::Type => "Type",
            Tool::Select => "Path Selection",
            Tool::Shape => "Rectangle",
            Tool::EllipseShape => "Ellipse",
            Tool::Hand => "Hand",
            Tool::Zoom => "Zoom",
        }
    }

    /// Hover text: name and shortcut, e.g. "Brush Tool (B)".
    pub fn tooltip(self) -> &'static str {
        match self {
            Tool::Move => "Move Tool (V)",
            Tool::Marquee => "Rectangular Marquee Tool (M)",
            Tool::EllipticalMarquee => "Elliptical Marquee Tool (M)",
            Tool::Lasso => "Lasso Tool (L)",
            Tool::PolygonalLasso => "Polygonal Lasso Tool (L)",
            Tool::Wand => "Magic Wand Tool (W)",
            Tool::Crop => "Crop Tool (C)",
            Tool::Eyedropper => "Eyedropper Tool (I)",
            Tool::Heal => "Healing Brush Tool (J)",
            Tool::Brush => "Brush Tool (B)",
            Tool::Stamp => "Clone Stamp Tool (S)",
            Tool::Eraser => "Eraser Tool (E)",
            Tool::Gradient => "Gradient Tool (G)",
            Tool::Dodge => "Dodge Tool (O)",
            Tool::Burn => "Burn Tool (O)",
            Tool::Pen => "Pen Tool (P)",
            Tool::Type => "Type Tool (T)",
            Tool::Select => "Path Selection Tool (A)",
            Tool::Shape => "Rectangle Tool (U)",
            Tool::EllipseShape => "Ellipse Tool (U)",
            Tool::Hand => "Hand Tool (H)",
            Tool::Zoom => "Zoom Tool (Z)",
        }
    }

    /// Photoshop-compatible single-key shortcut (shared by a slot's tools).
    pub fn shortcut(self) -> char {
        match self {
            Tool::Move => 'v',
            Tool::Marquee | Tool::EllipticalMarquee => 'm',
            Tool::Lasso | Tool::PolygonalLasso => 'l',
            Tool::Wand => 'w',
            Tool::Crop => 'c',
            Tool::Eyedropper => 'i',
            Tool::Heal => 'j',
            Tool::Brush => 'b',
            Tool::Stamp => 's',
            Tool::Eraser => 'e',
            Tool::Gradient => 'g',
            Tool::Dodge | Tool::Burn => 'o',
            Tool::Pen => 'p',
            Tool::Type => 't',
            Tool::Select => 'a',
            Tool::Shape | Tool::EllipseShape => 'u',
            Tool::Hand => 'h',
            Tool::Zoom => 'z',
        }
    }

    /// Index of the toolbar slot holding this tool.
    pub fn slot(self) -> usize {
        slots().position(|s| s.contains(&self)).expect("every tool is in a slot")
    }

    /// The slot whose shortcut is `c`.
    pub fn slot_for_shortcut(c: char) -> Option<usize> {
        slots().position(|s| s[0].shortcut() == c)
    }

    /// Tools that paint with the brush engine.
    pub fn paints(self) -> bool {
        matches!(self, Tool::Brush | Tool::Eraser | Tool::Stamp | Tool::Heal | Tool::Dodge | Tool::Burn)
    }

    /// Tools whose options bar shows brush settings.
    pub fn uses_brush(self) -> bool {
        self.paints()
    }

    /// Clone Stamp and Healing Brush paint from a source set with Alt+click.
    pub fn clones(self) -> bool {
        matches!(self, Tool::Stamp | Tool::Heal)
    }

    pub fn is_shape(self) -> bool {
        matches!(self, Tool::Shape | Tool::EllipseShape)
    }

    pub fn is_selection(self) -> bool {
        matches!(
            self,
            Tool::Marquee | Tool::EllipticalMarquee | Tool::Lasso | Tool::PolygonalLasso | Tool::Wand
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_is_in_exactly_one_slot_and_shortcuts_are_per_slot() {
        let all: Vec<Tool> = slots().flat_map(|s| s.iter().copied()).collect();
        for t in &all {
            assert_eq!(all.iter().filter(|x| *x == t).count(), 1, "{t:?}");
            let key = t.shortcut().to_ascii_uppercase();
            assert!(t.tooltip().ends_with(&format!("({key})")), "{t:?}");
            assert_eq!(Tool::slot_for_shortcut(t.shortcut()), Some(t.slot()), "{t:?}");
        }
        let mut keys: Vec<char> = slots().map(|s| s[0].shortcut()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), slots().count());
    }
}
