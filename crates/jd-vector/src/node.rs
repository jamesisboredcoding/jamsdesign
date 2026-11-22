//! Nodes and their properties.
//!
//! Geometry is in pixels, relative to the parent node (pages are the origin). `x`/`y` is where
//! the node's top-left corner lands after rotation, and `rotation` is in degrees,
//! counter-clockwise, about that corner (the same convention Figma uses, so it's familiar).
//! Children are ordered bottom to top: the last child draws on top.

use std::collections::BTreeMap;
use std::fmt;

use jd_core::Rgba;
use kurbo::{BezPath, PathEl, Point};
use serde::{Deserialize, Serialize};

use crate::color;
use crate::ops::Patch;

/// A node's identity. Stable for the life of the document, including across saves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u64);

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    /// `None` for pages.
    pub parent: Option<NodeId>,
    /// Bottom to top.
    pub children: Vec<NodeId>,
    pub kind: NodeKind,
    pub props: Props,
}

impl Node {
    pub fn is_page(&self) -> bool {
        matches!(self.kind, NodeKind::Page { .. })
    }

    /// Whether this node has auto layout.
    pub fn auto_layout(&self) -> Option<&AutoLayout> {
        self.props.layout.as_ref()
    }
}

/// What a node is, plus the data only that kind has.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeKind {
    Page {
        #[serde(with = "color::hex", default = "page_background")]
        background: Rgba,
    },
    Frame,
    /// Sized to fit its children after every edit; has no fill or clip of its own.
    Group,
    Section,
    Rectangle,
    Ellipse {
        #[serde(default, skip_serializing_if = "is_default")]
        arc: EllipseArc,
    },
    /// Runs from (0, 0) to (width, 0) in the node's own space; rotate it for other angles.
    Line,
    Polygon {
        sides: u32,
    },
    Star {
        points: u32,
        /// Inner radius as a fraction of the outer one.
        inner: f32,
    },
    Vector {
        path: VectorPath,
    },
    Text {
        text: TextData,
    },
    /// A reusable design. Behaves like a frame; its instances follow it.
    Component {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        description: String,
        /// Properties instances can set, by name.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        properties: BTreeMap<String, PropDef>,
    },
    /// Holds the variants of one component. Each child component is named like
    /// `Size=Large, State=Default`. Sized to fit its children.
    ComponentSet {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        description: String,
    },
    /// A copy of a component that follows it. Its children are rebuilt from the component
    /// after every batch; edits to them are kept as overrides.
    Instance {
        component: NodeId,
        /// Component property values set on this instance.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        values: BTreeMap<String, PropValue>,
        /// Changes to the copied layers, by source key (see [`crate::component`]). The key
        /// `""` is the instance itself.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        overrides: BTreeMap<String, Patch>,
        /// Which of this instance's nodes copies which source node, by source key, so ids stay
        /// the same each time the copy is rebuilt.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        map: BTreeMap<String, NodeId>,
    },
}

/// A component property and its default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PropDef {
    Text { default: String },
    Boolean { default: bool },
    /// Which component a nested instance shows.
    InstanceSwap { default: NodeId },
}

impl PropDef {
    pub fn default_value(&self) -> PropValue {
        match self {
            Self::Text { default } => PropValue::Text(default.clone()),
            Self::Boolean { default } => PropValue::Bool(*default),
            Self::InstanceSwap { default } => PropValue::Node(*default),
        }
    }
}

/// A component property value. In JSON: `true`/`false`, a node id number, or a string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PropValue {
    Bool(bool),
    Node(NodeId),
    Text(String),
}

/// Which component properties drive this node (set on nodes inside a component).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PropRefs {
    /// A text property that sets this text node's characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// A boolean property that shows or hides this node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<String>,
    /// An instance-swap property that picks this nested instance's component.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swap: Option<String>,
}

/// Styles a node uses (ids of styles in [`crate::tokens`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleRefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<NodeId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke: Option<NodeId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<NodeId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<NodeId>,
}

impl NodeKind {
    /// Kinds that can hold children.
    pub fn is_container(&self) -> bool {
        matches!(
            self,
            Self::Page { .. } | Self::Frame | Self::Group | Self::Section | Self::Component { .. } | Self::ComponentSet { .. } | Self::Instance { .. }
        )
    }

    /// Kinds that behave like a frame: a box with fills, clipping and optional auto layout.
    pub fn is_frame_like(&self) -> bool {
        matches!(self, Self::Frame | Self::Component { .. } | Self::Instance { .. })
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Page { .. } => "page",
            Self::Frame => "frame",
            Self::Group => "group",
            Self::Section => "section",
            Self::Rectangle => "rectangle",
            Self::Ellipse { .. } => "ellipse",
            Self::Line => "line",
            Self::Polygon { .. } => "polygon",
            Self::Star { .. } => "star",
            Self::Vector { .. } => "vector",
            Self::Text { .. } => "text",
            Self::Component { .. } => "component",
            Self::ComponentSet { .. } => "component_set",
            Self::Instance { .. } => "instance",
        }
    }
}

pub(crate) fn page_background() -> Rgba {
    Rgba::from_srgb8(0x1E, 0x1E, 0x1E)
}

/// An ellipse can be cut into an arc, a pie or a ring.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EllipseArc {
    /// Degrees, counter-clockwise from 3 o'clock.
    pub start: f32,
    /// Degrees; 360 is a whole ellipse.
    pub sweep: f32,
    /// Hole size as a fraction of the radius (0 = solid).
    pub inner: f32,
}

impl Default for EllipseArc {
    fn default() -> Self {
        Self { start: 0.0, sweep: 360.0, inner: 0.0 }
    }
}

/// Properties every node has. Kinds that don't use one ignore it (an ellipse has no corner
/// radius), and validation rejects the combinations that make no sense (see `ops`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Props {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(skip_serializing_if = "is_zero")]
    pub rotation: f64,
    #[serde(skip_serializing_if = "is_true")]
    pub visible: bool,
    #[serde(skip_serializing_if = "is_default")]
    pub locked: bool,
    #[serde(skip_serializing_if = "is_one")]
    pub opacity: f32,
    #[serde(skip_serializing_if = "is_default")]
    pub blend: BlendMode,
    /// Bottom to top, like layers.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fills: Vec<Paint>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub strokes: Vec<Paint>,
    #[serde(skip_serializing_if = "is_default")]
    pub stroke: StrokeStyle,
    #[serde(skip_serializing_if = "is_default")]
    pub corner: CornerRadii,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// Frames: hide children outside the frame.
    #[serde(skip_serializing_if = "is_default")]
    pub clip: bool,
    /// Frames: lay children out automatically.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<AutoLayout>,
    /// How the width is decided. `Hug` needs auto layout or text; `Fill` needs an auto layout
    /// parent.
    #[serde(skip_serializing_if = "is_default")]
    pub sizing_h: Sizing,
    #[serde(skip_serializing_if = "is_default")]
    pub sizing_v: Sizing,
    /// Inside an auto layout parent: keep `x`/`y` instead of joining the flow.
    #[serde(skip_serializing_if = "is_default")]
    pub absolute: bool,
    /// How this node follows its parent when the parent is resized (not in auto layout).
    #[serde(skip_serializing_if = "is_default")]
    pub constraints: Constraints,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_height: Option<f64>,
    /// What File > Export makes of this node.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<ExportSetting>,
    /// Inside a component: properties that drive this node.
    #[serde(skip_serializing_if = "is_default")]
    pub refs: PropRefs,
    /// Variables bound to properties, by property path (see [`crate::tokens`]).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub bind: BTreeMap<String, NodeId>,
    /// Variable modes chosen for this node and everything in it, by collection id.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub modes: BTreeMap<NodeId, String>,
    #[serde(skip_serializing_if = "is_default")]
    pub styles: StyleRefs,
}

/// One file to export a node as.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSetting {
    pub format: ExportFormat,
    /// Pixels per document pixel (PNG only).
    #[serde(default = "one64")]
    pub scale: f64,
    /// Added to the node's name for the file name, e.g. "@2x".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub suffix: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Png,
    Svg,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Svg => "svg",
        }
    }
}

impl Default for Props {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            rotation: 0.0,
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: BlendMode::default(),
            fills: Vec::new(),
            strokes: Vec::new(),
            stroke: StrokeStyle::default(),
            corner: CornerRadii::default(),
            effects: Vec::new(),
            clip: false,
            layout: None,
            sizing_h: Sizing::Fixed,
            sizing_v: Sizing::Fixed,
            absolute: false,
            constraints: Constraints::default(),
            min_width: None,
            max_width: None,
            min_height: None,
            max_height: None,
            exports: Vec::new(),
            refs: PropRefs::default(),
            bind: BTreeMap::new(),
            modes: BTreeMap::new(),
            styles: StyleRefs::default(),
        }
    }
}

impl Props {
    pub fn clamp_width(&self, w: f64) -> f64 {
        clamp_opt(w, self.min_width, self.max_width)
    }

    pub fn clamp_height(&self, h: f64) -> f64 {
        clamp_opt(h, self.min_height, self.max_height)
    }
}

fn clamp_opt(v: f64, min: Option<f64>, max: Option<f64>) -> f64 {
    let v = max.map_or(v, |m| v.min(m));
    min.map_or(v, |m| v.max(m)).max(0.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    /// Frames and groups: children blend with what's below the frame, as if it weren't there.
    PassThrough,
    #[default]
    Normal,
    Darken,
    Multiply,
    ColorBurn,
    Lighten,
    Screen,
    ColorDodge,
    Overlay,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

/// One fill or stroke layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Paint {
    #[serde(flatten)]
    pub kind: PaintKind,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub opacity: f32,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    pub blend: BlendMode,
}

impl Paint {
    pub fn solid(color: Rgba) -> Self {
        Self { kind: PaintKind::Solid { color }, opacity: 1.0, visible: true, blend: BlendMode::Normal }
    }

    /// A solid paint from `#RRGGBB` or `#RRGGBBAA`. Panics on a malformed literal; use
    /// [`Paint::solid`] for colours that come from users.
    pub fn hex(hex: &str) -> Self {
        Self::solid(color::from_hex(hex).unwrap_or_else(|| panic!("bad colour literal {hex:?}")))
    }
}

/// Gradient and image positions are fractions of the node's box: (0, 0) top-left, (1, 1)
/// bottom-right.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaintKind {
    Solid {
        #[serde(with = "color::hex")]
        color: Rgba,
    },
    LinearGradient {
        from: [f64; 2],
        to: [f64; 2],
        stops: Vec<GradientStop>,
    },
    RadialGradient {
        center: [f64; 2],
        /// Horizontal and vertical radius.
        radius: [f64; 2],
        stops: Vec<GradientStop>,
    },
    Image {
        /// Key into [`Document::images`](crate::Document::images).
        image: String,
        #[serde(default)]
        fit: ImageFit,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub position: f32,
    #[serde(with = "color::hex")]
    pub color: Rgba,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFit {
    /// Cover the box, cropping what doesn't fit.
    #[default]
    Fill,
    /// Fit inside the box, leaving empty space.
    Fit,
    /// Stretch to the box.
    Stretch,
    /// Repeat at the image's own size.
    Tile,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StrokeStyle {
    pub weight: f32,
    pub align: StrokeAlign,
    /// Dash and gap lengths in pixels; empty for a solid line.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dash: Vec<f32>,
    pub cap: StrokeCap,
    pub join: StrokeJoin,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        Self { weight: 1.0, align: StrokeAlign::Inside, dash: Vec::new(), cap: StrokeCap::None, join: StrokeJoin::Miter }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokeAlign {
    #[default]
    Inside,
    Center,
    Outside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokeCap {
    #[default]
    None,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokeJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CornerRadii {
    pub top_left: f64,
    pub top_right: f64,
    pub bottom_right: f64,
    pub bottom_left: f64,
}

impl CornerRadii {
    pub fn all(r: f64) -> Self {
        Self { top_left: r, top_right: r, bottom_right: r, bottom_left: r }
    }

    pub fn is_uniform(&self) -> bool {
        self.top_left == self.top_right && self.top_left == self.bottom_right && self.top_left == self.bottom_left
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Effect {
    DropShadow(Shadow),
    InnerShadow(Shadow),
    LayerBlur {
        radius: f32,
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        visible: bool,
    },
    BackgroundBlur {
        radius: f32,
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        visible: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    #[serde(with = "color::hex")]
    pub color: Rgba,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub blur: f32,
    #[serde(default)]
    pub spread: f32,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub visible: bool,
}

/// Auto layout on a frame.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoLayout {
    pub direction: Direction,
    /// Space between children along the direction (ignored with `SpaceBetween`).
    pub gap: f64,
    pub padding: Padding,
    /// Alignment along the direction.
    pub align: Align,
    /// Alignment across the direction.
    pub counter_align: CounterAlign,
    /// Horizontal layouts only: start a new row when the next child doesn't fit.
    #[serde(skip_serializing_if = "is_default")]
    pub wrap: bool,
    /// Space between wrapped rows.
    #[serde(skip_serializing_if = "is_zero")]
    pub row_gap: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CounterAlign {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Padding {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl Padding {
    pub fn all(v: f64) -> Self {
        Self { top: v, right: v, bottom: v, left: v }
    }

    /// Horizontal and vertical padding.
    pub fn hv(h: f64, v: f64) -> Self {
        Self { top: v, right: h, bottom: v, left: h }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sizing {
    #[default]
    Fixed,
    /// Shrink to the content (auto layout frames and text).
    Hug,
    /// Take the space the auto layout parent has left (or its full size across the direction).
    Fill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Constraints {
    pub horizontal: Constraint,
    pub vertical: Constraint,
}

/// `Start` is left or top, `End` right or bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Constraint {
    #[default]
    Start,
    End,
    /// Keep both margins, stretching the node.
    Both,
    Center,
    Scale,
}

/// Text content and its style. Mixed styles within one text node come later; one style per
/// node for now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextData {
    pub content: String,
    #[serde(default)]
    pub style: TextStyle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextStyle {
    pub family: String,
    /// Style name as the font calls it ("Regular", "Semi Bold", ...).
    pub style: String,
    pub size: f32,
    pub line_height: LineHeight,
    /// Extra space between letters, in pixels.
    #[serde(skip_serializing_if = "is_zero32")]
    pub letter_spacing: f32,
    pub align: TextAlign,
    pub valign: TextVAlign,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: "Inter".into(),
            style: "Regular".into(),
            size: 14.0,
            line_height: LineHeight::Auto,
            letter_spacing: 0.0,
            align: TextAlign::Left,
            valign: TextVAlign::Top,
        }
    }
}

impl TextStyle {
    /// Line height in pixels (`Auto` is 1.2 x the font size).
    pub fn line_px(&self) -> f64 {
        match self.line_height {
            LineHeight::Auto => self.size as f64 * 1.2,
            LineHeight::Px(px) => px as f64,
            LineHeight::Percent(p) => self.size as f64 * p as f64 / 100.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineHeight {
    #[default]
    Auto,
    Px(f32),
    Percent(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    Justified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextVAlign {
    #[default]
    Top,
    Center,
    Bottom,
}

/// A bezier path in the node's own space, drawn when the node was `base` sized. Resizing the
/// node scales the path to the new size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorPath {
    pub subpaths: Vec<Subpath>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub fill_rule: FillRule,
    /// Width and height the anchors were drawn at.
    pub base: [f64; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subpath {
    pub anchors: Vec<Anchor>,
    #[serde(default)]
    pub closed: bool,
}

/// An anchor point with optional absolute handle positions.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub p: [f64; 2],
    #[serde(default, rename = "in", skip_serializing_if = "Option::is_none")]
    pub handle_in: Option<[f64; 2]>,
    #[serde(default, rename = "out", skip_serializing_if = "Option::is_none")]
    pub handle_out: Option<[f64; 2]>,
}

impl Anchor {
    pub fn corner(x: f64, y: f64) -> Self {
        Self { p: [x, y], handle_in: None, handle_out: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

impl VectorPath {
    /// Parse SVG path data (`d`), drawn in a `width` x `height` box.
    pub fn from_svg(d: &str, width: f64, height: f64) -> Result<Self, String> {
        let bez = BezPath::from_svg(d).map_err(|e| format!("bad SVG path: {e:?}"))?;
        Ok(Self::from_bez(&bez, width, height))
    }

    pub fn from_bez(bez: &BezPath, width: f64, height: f64) -> Self {
        let pt = |p: Point| [p.x, p.y];
        let mut subpaths: Vec<Subpath> = Vec::new();
        let finish = |sp: &mut Option<Subpath>, subpaths: &mut Vec<Subpath>| {
            if let Some(mut s) = sp.take() {
                // A closing segment that returns to the start point duplicates the first
                // anchor: fold it back in.
                if s.closed && s.anchors.len() > 1 {
                    let (first, last) = (s.anchors[0], s.anchors[s.anchors.len() - 1]);
                    if first.p == last.p {
                        s.anchors[0].handle_in = last.handle_in;
                        s.anchors.pop();
                    }
                }
                subpaths.push(s);
            }
        };
        let mut cur: Option<Subpath> = None;
        for el in bez.elements() {
            match *el {
                PathEl::MoveTo(p) => {
                    finish(&mut cur, &mut subpaths);
                    cur = Some(Subpath { anchors: vec![Anchor::corner(p.x, p.y)], closed: false });
                }
                PathEl::LineTo(p) => {
                    let s = cur.get_or_insert_with(|| Subpath { anchors: Vec::new(), closed: false });
                    s.anchors.push(Anchor::corner(p.x, p.y));
                }
                PathEl::QuadTo(c, p) => {
                    let s = cur.get_or_insert_with(|| Subpath { anchors: Vec::new(), closed: false });
                    let start = s.anchors.last().map_or(c, |a| Point::new(a.p[0], a.p[1]));
                    let c1 = start + (c - start) * (2.0 / 3.0);
                    let c2 = p + (c - p) * (2.0 / 3.0);
                    if let Some(last) = s.anchors.last_mut() {
                        last.handle_out = Some(pt(c1));
                    }
                    s.anchors.push(Anchor { p: pt(p), handle_in: Some(pt(c2)), handle_out: None });
                }
                PathEl::CurveTo(c1, c2, p) => {
                    let s = cur.get_or_insert_with(|| Subpath { anchors: Vec::new(), closed: false });
                    if let Some(last) = s.anchors.last_mut() {
                        last.handle_out = Some(pt(c1));
                    }
                    s.anchors.push(Anchor { p: pt(p), handle_in: Some(pt(c2)), handle_out: None });
                }
                PathEl::ClosePath => {
                    if let Some(s) = cur.as_mut() {
                        s.closed = true;
                    }
                    finish(&mut cur, &mut subpaths);
                }
            }
        }
        finish(&mut cur, &mut subpaths);
        Self { subpaths, fill_rule: FillRule::NonZero, base: [width, height] }
    }

    /// The path scaled from `base` to a `width` x `height` node.
    pub fn to_bez(&self, width: f64, height: f64) -> BezPath {
        let sx = if self.base[0] > 0.0 { width / self.base[0] } else { 1.0 };
        let sy = if self.base[1] > 0.0 { height / self.base[1] } else { 1.0 };
        let pt = |p: [f64; 2]| Point::new(p[0] * sx, p[1] * sy);
        let mut bez = BezPath::new();
        for s in &self.subpaths {
            let Some(first) = s.anchors.first() else { continue };
            bez.move_to(pt(first.p));
            let segment = |bez: &mut BezPath, a: &Anchor, b: &Anchor| match (a.handle_out, b.handle_in) {
                (None, None) => bez.line_to(pt(b.p)),
                (o, i) => bez.curve_to(pt(o.unwrap_or(a.p)), pt(i.unwrap_or(b.p)), pt(b.p)),
            };
            for w in s.anchors.windows(2) {
                segment(&mut bez, &w[0], &w[1]);
            }
            if s.closed {
                if s.anchors.len() > 1 {
                    segment(&mut bez, &s.anchors[s.anchors.len() - 1], first);
                }
                bez.close_path();
            }
        }
        bez
    }
}

pub(crate) fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}
fn is_true(v: &bool) -> bool {
    *v
}
fn is_one(v: &f32) -> bool {
    *v == 1.0
}
fn is_zero(v: &f64) -> bool {
    *v == 0.0
}
fn is_zero32(v: &f32) -> bool {
    *v == 0.0
}
fn one() -> f32 {
    1.0
}
fn one64() -> f64 {
    1.0
}
fn yes() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_paths_round_trip_through_anchors() {
        let v = VectorPath::from_svg("M2 2h12v12H2z M5 8c0-2 6-2 6 0", 16.0, 16.0).unwrap();
        assert_eq!(v.subpaths.len(), 2);
        assert!(v.subpaths[0].closed);
        assert_eq!(v.subpaths[0].anchors.len(), 4, "closing point folds into the first anchor");
        assert_eq!(v.subpaths[1].anchors[0].handle_out, Some([5.0, 6.0]));
        // Drawn at 16x16, shown at 32x32: everything doubles.
        let bez = v.to_bez(32.0, 32.0);
        let b = kurbo::Shape::bounding_box(&bez);
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (4.0, 4.0, 28.0, 28.0));
    }

    #[test]
    fn props_skip_defaults_when_serialized() {
        let p = Props { width: 10.0, height: 5.0, ..Props::default() };
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, r#"{"x":0.0,"y":0.0,"width":10.0,"height":5.0}"#);
        let back: Props = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }
}
