//! Design tokens: variables (in collections with modes) and styles.
//!
//! A node binds a property to a variable through `Props::bind`, keyed by a property path:
//!
//! | path | variable type |
//! |---|---|
//! | `x`, `y`, `width`, `height`, `rotation`, `opacity`, `stroke_weight`, `font_size` | number |
//! | `corner`, `corner/top_left` (and the other corners) | number |
//! | `gap`, `row_gap`, `padding`, `padding/top` (and the other sides) | number |
//! | `visible` | boolean |
//! | `text` | text (or number) |
//! | `fills/0`, `strokes/0` (any index) | colour |
//!
//! A frame picks a mode per collection (`Props::modes`) for itself and everything inside;
//! otherwise a collection's first mode applies. Styles (paints, text, effects) are copied into
//! the nodes that use them (`Props::styles`). Both happen after every batch, in
//! [`resolve_all`], so the renderer only ever sees plain values.
//!
//! Collections, variables and styles share the id space with nodes, so an id is never both.

use std::collections::BTreeMap;

use jd_core::Rgba;
use serde::{Deserialize, Serialize};

use crate::color;
use crate::doc::Document;
use crate::node::*;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Tokens {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub collections: BTreeMap<NodeId, Collection>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub variables: BTreeMap<NodeId, Variable>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub styles: BTreeMap<NodeId, Style>,
}

impl Tokens {
    pub fn is_empty(&self) -> bool {
        self.collections.is_empty() && self.variables.is_empty() && self.styles.is_empty()
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.collections.contains_key(&id) || self.variables.contains_key(&id) || self.styles.contains_key(&id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    pub name: String,
    /// Mode names; the first is the default.
    pub modes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Variable {
    pub name: String,
    pub collection: NodeId,
    /// Value per mode name. A mode without a value uses the first mode's.
    pub values: BTreeMap<String, VarValue>,
}

/// In JSON: `{"color": "#4C8DFF"}`, `{"number": 8}`, `{"text": "Hi"}`, `{"bool": true}` or
/// `{"alias": 12}` (another variable's value).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VarValue {
    Color(#[serde(with = "color::hex")] Rgba),
    Number(f64),
    Text(String),
    Bool(bool),
    Alias(NodeId),
}

impl VarValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Color(_) => "color",
            Self::Number(_) => "number",
            Self::Text(_) => "text",
            Self::Bool(_) => "bool",
            Self::Alias(_) => "alias",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Style {
    pub name: String,
    #[serde(flatten)]
    pub kind: StyleKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StyleKind {
    Paint { paints: Vec<Paint> },
    Text { text: TextStyle },
    Effect { effects: Vec<Effect> },
}

/// What a binding path expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    Number,
    Bool,
    Text,
    Color,
}

/// Checks a binding path and says what type of variable it takes.
pub fn binding_type(path: &str) -> Option<Expect> {
    let (head, tail) = path.split_once('/').map_or((path, None), |(h, t)| (h, Some(t)));
    match (head, tail) {
        ("x" | "y" | "width" | "height" | "rotation" | "opacity" | "stroke_weight" | "font_size" | "gap" | "row_gap", None) => {
            Some(Expect::Number)
        }
        ("corner", None | Some("top_left" | "top_right" | "bottom_right" | "bottom_left")) => Some(Expect::Number),
        ("padding", None | Some("top" | "right" | "bottom" | "left")) => Some(Expect::Number),
        ("visible", None) => Some(Expect::Bool),
        ("text", None) => Some(Expect::Text),
        ("fills" | "strokes", Some(i)) if i.parse::<usize>().is_ok() => Some(Expect::Color),
        _ => None,
    }
}

/// Binding paths a direct edit of a property replaces (typing a value unbinds it).
pub(crate) fn unbinds(field: &str) -> &'static [&'static str] {
    match field {
        "x" => &["x"],
        "y" => &["y"],
        "width" => &["width"],
        "height" => &["height"],
        "rotation" => &["rotation"],
        "opacity" => &["opacity"],
        "visible" => &["visible"],
        "fills" => &["fills"],
        "strokes" => &["strokes"],
        "stroke" => &["stroke_weight"],
        "corner" => &["corner"],
        "layout" => &["gap", "row_gap", "padding"],
        "text" => &["text"],
        "text_style" => &["font_size"],
        _ => &[],
    }
}

impl Document {
    pub fn tokens(&self) -> &Tokens {
        &self.tokens
    }

    /// The mode of `collection` that applies at `id`.
    pub fn mode_at(&self, id: NodeId, collection: NodeId) -> Option<String> {
        let mut cur = Some(id);
        while let Some(c) = cur {
            let n = self.node(c)?;
            if let Some(m) = n.props.modes.get(&collection) {
                return Some(m.clone());
            }
            cur = n.parent;
        }
        self.tokens.collections.get(&collection)?.modes.first().cloned()
    }

    /// A variable's value at `id` (following aliases).
    pub fn resolve_variable(&self, var: NodeId, at: NodeId) -> Option<VarValue> {
        let mut var = var;
        for _ in 0..16 {
            let v = self.tokens.variables.get(&var)?;
            let mode = self.mode_at(at, v.collection)?;
            let first = self.tokens.collections.get(&v.collection)?.modes.first()?;
            let value = v.values.get(&mode).or_else(|| v.values.get(first))?;
            match value {
                VarValue::Alias(next) => var = *next,
                other => return Some(other.clone()),
            }
        }
        None
    }
}

/// Writes resolved variables and styles into every node that uses them.
pub(crate) fn resolve_all(doc: &mut Document) {
    if doc.tokens.is_empty() {
        return;
    }
    let ids: Vec<NodeId> = doc
        .nodes
        .iter()
        .filter(|(_, n)| !n.props.bind.is_empty() || n.props.styles != StyleRefs::default())
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        let n = doc.node(id).expect("listed");
        let mut node = n.clone();
        apply_styles(doc, &mut node);
        for (path, var) in n.props.bind.clone() {
            if let Some(v) = doc.resolve_variable(var, id) {
                write_binding(&mut node, &path, &v);
            }
        }
        if node != *doc.node(id).expect("listed") {
            *doc.node_mut(id).expect("listed") = node;
        }
    }
}

fn apply_styles(doc: &Document, node: &mut Node) {
    let st = node.props.styles;
    let get = |id: Option<NodeId>| id.and_then(|i| doc.tokens.styles.get(&i)).map(|s| &s.kind);
    if let Some(StyleKind::Paint { paints }) = get(st.fill) {
        node.props.fills = paints.clone();
    }
    if let Some(StyleKind::Paint { paints }) = get(st.stroke) {
        node.props.strokes = paints.clone();
    }
    if let Some(StyleKind::Effect { effects }) = get(st.effect) {
        node.props.effects = effects.clone();
    }
    if let (Some(StyleKind::Text { text }), NodeKind::Text { text: data }) = (get(st.text), &mut node.kind) {
        data.style = text.clone();
    }
}

fn write_binding(node: &mut Node, path: &str, v: &VarValue) {
    let p = &mut node.props;
    let num = match v {
        VarValue::Number(n) => Some(*n),
        _ => None,
    };
    let (head, tail) = path.split_once('/').map_or((path, None), |(h, t)| (h, Some(t)));
    match (head, tail, num) {
        ("x", _, Some(n)) => p.x = n,
        ("y", _, Some(n)) => p.y = n,
        ("width", _, Some(n)) => p.width = n.max(0.0),
        ("height", _, Some(n)) => p.height = n.max(0.0),
        ("rotation", _, Some(n)) => p.rotation = n,
        ("opacity", _, Some(n)) => p.opacity = n.clamp(0.0, 1.0) as f32,
        ("stroke_weight", _, Some(n)) => p.stroke.weight = n.max(0.0) as f32,
        ("corner", None, Some(n)) => p.corner = CornerRadii::all(n),
        ("corner", Some("top_left"), Some(n)) => p.corner.top_left = n,
        ("corner", Some("top_right"), Some(n)) => p.corner.top_right = n,
        ("corner", Some("bottom_right"), Some(n)) => p.corner.bottom_right = n,
        ("corner", Some("bottom_left"), Some(n)) => p.corner.bottom_left = n,
        ("gap" | "row_gap" | "padding", _, Some(n)) => {
            if let Some(l) = p.layout.as_mut() {
                match (head, tail) {
                    ("gap", _) => l.gap = n,
                    ("row_gap", _) => l.row_gap = n,
                    ("padding", None) => l.padding = Padding::all(n),
                    ("padding", Some("top")) => l.padding.top = n,
                    ("padding", Some("right")) => l.padding.right = n,
                    ("padding", Some("bottom")) => l.padding.bottom = n,
                    ("padding", Some("left")) => l.padding.left = n,
                    _ => {}
                }
            }
        }
        ("font_size", _, Some(n)) => {
            if let NodeKind::Text { text } = &mut node.kind {
                text.style.size = n.max(1.0) as f32;
            }
        }
        ("visible", _, _) => {
            if let VarValue::Bool(b) = v {
                p.visible = *b;
            }
        }
        ("text", _, _) => {
            if let NodeKind::Text { text } = &mut node.kind {
                match v {
                    VarValue::Text(t) => text.content = t.clone(),
                    VarValue::Number(n) => text.content = format!("{n}"),
                    _ => {}
                }
            }
        }
        ("fills" | "strokes", Some(i), _) => {
            let (VarValue::Color(c), Ok(i)) = (v, i.parse::<usize>()) else { return };
            let list = if head == "fills" { &mut p.fills } else { &mut p.strokes };
            if let Some(paint) = list.get_mut(i) {
                paint.kind = PaintKind::Solid { color: *c };
            } else if i == list.len() {
                list.push(Paint::solid(*c));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn variables_follow_modes_and_aliases() {
        let mut doc = Document::new();
        let p = doc.pages()[0];
        let done = doc
            .apply(
                &[
                    Op::CreateCollection { name: "Theme".into(), modes: vec!["Light".into(), "Dark".into()], key: Some("theme".into()) },
                    Op::CreateVariable {
                        collection: "theme".into(),
                        name: "surface".into(),
                        values: [("Light".into(), VarValue::Color(Rgba::from_srgb8(255, 255, 255))), ("Dark".into(), VarValue::Color(Rgba::from_srgb8(0x1E, 0x1E, 0x1E)))].into(),
                        key: Some("surface".into()),
                    },
                    Op::CreateVariable {
                        collection: "theme".into(),
                        name: "card".into(),
                        values: Default::default(),
                        key: Some("card".into()),
                    },
                    Op::create_as("light", p, NodeSpec::frame("Light", 100.0, 100.0)),
                    Op::create_as("dark", p, NodeSpec::frame("Dark", 100.0, 100.0)),
                    Op::create_as("a", "light", NodeSpec::rect("A", 10.0, 10.0)),
                    Op::create_as("b", "dark", NodeSpec::rect("B", 10.0, 10.0)),
                ],
                &ApproxMeasure,
            )
            .unwrap();
        let id = |k| done.id(k).unwrap();
        let (surface, card) = (id("surface"), id("card"));
        let bind = |var| Patch { bind: Some([("fills/0".to_string(), Some(var))].into()), ..Patch::default() };
        doc.apply(
            &[
                Op::SetVariable { id: card.into(), name: None, mode: Some("Light".into()), value: Some(VarValue::Alias(surface)) },
                Op::set(id("dark"), Patch { modes: Some([(id("theme"), Some("Dark".to_string()))].into()), ..Patch::default() }),
                Op::set(id("a"), bind(card)),
                Op::set(id("b"), bind(card)),
            ],
            &ApproxMeasure,
        )
        .unwrap();
        let fill = |doc: &Document, k| match &doc.node(id(k)).unwrap().props.fills[0].kind {
            PaintKind::Solid { color } => color.to_hex(),
            other => panic!("{other:?}"),
        };
        assert_eq!(fill(&doc, "a"), "FFFFFF");
        // "card" has no Dark value, so it uses its Light one: an alias to "surface", which
        // resolves in Dark mode.
        assert_eq!(fill(&doc, "b"), "1E1E1E");

        // Changing the variable updates every node bound to it.
        doc.apply(
            &[Op::SetVariable { id: surface.into(), name: None, mode: Some("Light".into()), value: Some(VarValue::Color(Rgba::from_srgb8(1, 2, 3))) }],
            &ApproxMeasure,
        )
        .unwrap();
        assert_eq!(fill(&doc, "a"), "010203");

        // Typing a value unbinds it.
        doc.apply(&[Op::set(id("a"), Patch { fills: Some(vec![Paint::hex("#00FF00")]), ..Patch::default() })], &ApproxMeasure).unwrap();
        assert!(doc.node(id("a")).unwrap().props.bind.is_empty());
    }

    #[test]
    fn number_variables_drive_layout_and_types_are_checked() {
        let mut doc = Document::new();
        let p = doc.pages()[0];
        let done = doc
            .apply(
                &[
                    Op::CreateCollection { name: "Spacing".into(), modes: vec!["Default".into()], key: Some("c".into()) },
                    Op::CreateVariable { collection: "c".into(), name: "space/md".into(), values: [("Default".into(), VarValue::Number(12.0))].into(), key: Some("md".into()) },
                    Op::create_as("row", p, NodeSpec::frame("Row", 0.0, 0.0).auto_layout(AutoLayout::default())),
                    Op::create("row", NodeSpec::rect("A", 10.0, 10.0)),
                    Op::create("row", NodeSpec::rect("B", 10.0, 10.0)),
                    Op::set("row", Patch { bind: Some([("gap".to_string(), Some(NodeId(0)))].into()), ..Patch::default() }),
                ],
                &ApproxMeasure,
            );
        assert!(done.unwrap_err().message.contains("doesn't exist"));
        let done = doc
            .apply(
                &[
                    Op::CreateCollection { name: "Spacing".into(), modes: vec!["Default".into()], key: Some("c".into()) },
                    Op::CreateVariable { collection: "c".into(), name: "space/md".into(), values: [("Default".into(), VarValue::Number(12.0))].into(), key: Some("md".into()) },
                    Op::create_as("row", p, NodeSpec::frame("Row", 0.0, 0.0).auto_layout(AutoLayout::default())),
                    Op::create("row", NodeSpec::rect("A", 10.0, 10.0)),
                    Op::create("row", NodeSpec::rect("B", 10.0, 10.0)),
                ],
                &ApproxMeasure,
            )
            .unwrap();
        let (row, md) = (done.id("row").unwrap(), done.id("md").unwrap());
        let bind = |path: &str| Op::set(row, Patch { bind: Some([(path.to_string(), Some(md))].into()), ..Patch::default() });
        doc.apply(&[bind("gap")], &ApproxMeasure).unwrap();
        assert_eq!(doc.node(row).unwrap().props.width, 32.0);
        let err = doc.apply(&[bind("fills/0")], &ApproxMeasure).unwrap_err();
        assert!(err.message.contains("needs a color variable"), "{err}");
        assert!(doc.apply(&[bind("nonsense")], &ApproxMeasure).is_err());
    }

    #[test]
    fn styles_are_copied_into_nodes() {
        let mut doc = Document::new();
        let p = doc.pages()[0];
        let done = doc
            .apply(
                &[
                    Op::CreateStyle { name: "Brand/Blue".into(), style: StyleKind::Paint { paints: vec![Paint::hex("#4C8DFF")] }, key: Some("blue".into()) },
                    Op::CreateStyle { name: "Heading".into(), style: StyleKind::Text { text: TextStyle { size: 30.0, style: "Bold".into(), ..TextStyle::default() } }, key: Some("h".into()) },
                    Op::create_as("t", p, NodeSpec::text("Title", "Hi", TextStyle::default())),
                ],
                &ApproxMeasure,
            )
            .unwrap();
        let (t, blue, h) = (done.id("t").unwrap(), done.id("blue").unwrap(), done.id("h").unwrap());
        doc.apply(&[Op::set(t, Patch { styles: Some(StyleRefs { fill: Some(blue), text: Some(h), ..StyleRefs::default() }), ..Patch::default() })], &ApproxMeasure)
            .unwrap();
        let n = doc.node(t).unwrap();
        assert_eq!(n.props.fills, vec![Paint::hex("#4C8DFF")]);
        let NodeKind::Text { text } = &n.kind else { panic!() };
        assert_eq!(text.style.size, 30.0);
        assert_eq!(n.props.height, 36.0, "layout saw the new size");
        doc.apply(&[Op::SetStyle { id: blue.into(), name: None, style: Some(StyleKind::Paint { paints: vec![Paint::hex("#E01B5A")] }) }], &ApproxMeasure)
            .unwrap();
        assert_eq!(doc.node(t).unwrap().props.fills, vec![Paint::hex("#E01B5A")]);
    }
}
