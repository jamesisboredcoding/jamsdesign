//! Every document change is an [`Op`]. [`Document::apply`] runs a batch atomically: on a cheap
//! clone, validating each op as it goes, then laying out, and only then replacing the document.
//! If any op fails, nothing changes and the error says which op and why.
//!
//! Ops are plain data (serde), so the same batches can come from the GUI, from tests, from a
//! file, or (later) from an MCP client. Within a batch, a created node can be given a `key`
//! and referred to by that key in later ops, before its id is known.

use std::collections::BTreeMap;
use std::fmt;

use jd_core::Rgba;
use serde::{Deserialize, Deserializer, Serialize};

use crate::doc::Document;
use crate::node::*;
use crate::text::TextMeasure;

/// A node reference in an op: an existing id, or the `key` of a node created earlier in the
/// same batch. In JSON, a number is an id and a string is a key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Ref {
    Id(NodeId),
    Key(String),
}

impl From<NodeId> for Ref {
    fn from(id: NodeId) -> Self {
        Self::Id(id)
    }
}

impl From<&str> for Ref {
    fn from(key: &str) -> Self {
        Self::Key(key.to_owned())
    }
}

/// A new node: name, kind and properties.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeSpec {
    pub name: String,
    #[serde(flatten)]
    pub kind: NodeKind,
    #[serde(flatten)]
    pub props: Props,
}

impl NodeSpec {
    /// A node of `kind` with the defaults a user would get from the matching tool.
    pub fn new(name: &str, kind: NodeKind) -> Self {
        let mut props = Props { width: 100.0, height: 100.0, ..Props::default() };
        match &kind {
            NodeKind::Frame => {
                props.fills = vec![Paint::hex("#FFFFFF")];
                props.clip = true;
            }
            NodeKind::Section => props.fills = vec![Paint::hex("#2C2C2C")],
            NodeKind::Rectangle | NodeKind::Ellipse { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. } => {
                props.fills = vec![Paint::hex("#D9D9D9")];
            }
            NodeKind::Line | NodeKind::Vector { .. } => {
                props.strokes = vec![Paint::hex("#000000")];
                props.stroke.align = StrokeAlign::Center;
                if matches!(kind, NodeKind::Line) {
                    props.height = 0.0;
                }
            }
            NodeKind::Text { .. } => {
                props.fills = vec![Paint::hex("#000000")];
                props.sizing_h = Sizing::Hug;
                props.sizing_v = Sizing::Hug;
            }
            NodeKind::Group | NodeKind::Page { .. } => {}
        }
        Self { name: name.to_owned(), kind, props }
    }

    pub fn frame(name: &str, width: f64, height: f64) -> Self {
        Self::new(name, NodeKind::Frame).size(width, height)
    }

    pub fn rect(name: &str, width: f64, height: f64) -> Self {
        Self::new(name, NodeKind::Rectangle).size(width, height)
    }

    pub fn ellipse(name: &str, width: f64, height: f64) -> Self {
        Self::new(name, NodeKind::Ellipse { arc: EllipseArc::default() }).size(width, height)
    }

    pub fn group(name: &str) -> Self {
        Self::new(name, NodeKind::Group)
    }

    /// Text that hugs its content.
    pub fn text(name: &str, content: &str, style: TextStyle) -> Self {
        Self::new(name, NodeKind::Text { text: TextData { content: content.to_owned(), style } })
    }

    pub fn vector(name: &str, path: VectorPath) -> Self {
        let [w, h] = path.base;
        Self::new(name, NodeKind::Vector { path }).size(w, h)
    }

    pub fn at(mut self, x: f64, y: f64) -> Self {
        self.props.x = x;
        self.props.y = y;
        self
    }

    pub fn size(mut self, width: f64, height: f64) -> Self {
        self.props.width = width;
        self.props.height = height;
        self
    }

    /// Replace the fills with one paint.
    pub fn fill(mut self, paint: Paint) -> Self {
        self.props.fills = vec![paint];
        self
    }

    pub fn no_fill(mut self) -> Self {
        self.props.fills.clear();
        self
    }

    pub fn stroke(mut self, paint: Paint, weight: f32) -> Self {
        self.props.strokes = vec![paint];
        self.props.stroke.weight = weight;
        self
    }

    pub fn radius(mut self, r: f64) -> Self {
        self.props.corner = CornerRadii::all(r);
        self
    }

    /// Give the frame auto layout. Both axes hug the content until told otherwise.
    pub fn auto_layout(mut self, layout: AutoLayout) -> Self {
        self.props.layout = Some(layout);
        self.props.sizing_h = Sizing::Hug;
        self.props.sizing_v = Sizing::Hug;
        self
    }

    pub fn sizing(mut self, h: Sizing, v: Sizing) -> Self {
        self.props.sizing_h = h;
        self.props.sizing_v = v;
        self
    }

    /// Change any other property.
    pub fn with(mut self, f: impl FnOnce(&mut Props)) -> Self {
        f(&mut self.props);
        self
    }
}

/// Changes to an existing node. Fields left `None` stay as they are. For the optional
/// properties (`layout`, `min_width`, ...), `Some(None)` removes the value (`null` in JSON).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Patch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    /// Sets the width as is, without moving children by their constraints (see
    /// [`Op::Resize`] for that). A hugging or filling width becomes fixed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blend: Option<BlendMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fills: Option<Vec<Paint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strokes: Option<Vec<Paint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke: Option<StrokeStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corner: Option<CornerRadii>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<Effect>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip: Option<bool>,
    #[serde(deserialize_with = "double", skip_serializing_if = "Option::is_none")]
    pub layout: Option<Option<AutoLayout>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sizing_h: Option<Sizing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sizing_v: Option<Sizing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub absolute: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraints: Option<Constraints>,
    #[serde(deserialize_with = "double", skip_serializing_if = "Option::is_none")]
    pub min_width: Option<Option<f64>>,
    #[serde(deserialize_with = "double", skip_serializing_if = "Option::is_none")]
    pub max_width: Option<Option<f64>>,
    #[serde(deserialize_with = "double", skip_serializing_if = "Option::is_none")]
    pub min_height: Option<Option<f64>>,
    #[serde(deserialize_with = "double", skip_serializing_if = "Option::is_none")]
    pub max_height: Option<Option<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exports: Option<Vec<ExportSetting>>,
    /// Text nodes: the characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Text nodes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_style: Option<TextStyle>,
    /// Vector nodes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<VectorPath>,
    /// Polygons.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sides: Option<u32>,
    /// Stars.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<u32>,
    /// Stars: inner radius ratio.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inner: Option<f32>,
    /// Ellipses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arc: Option<EllipseArc>,
    /// Pages.
    #[serde(with = "opt_hex", skip_serializing_if = "Option::is_none")]
    pub background: Option<Rgba>,
}

/// Distinguishes a missing field (`None`) from an explicit `null` (`Some(None)`).
fn double<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

mod opt_hex {
    use jd_core::Rgba;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(c: &Option<Rgba>, s: S) -> Result<S::Ok, S::Error> {
        match c {
            Some(c) => s.serialize_str(&crate::color::to_hex(*c)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Rgba>, D::Error> {
        match Option::<String>::deserialize(d)? {
            None => Ok(None),
            Some(s) => crate::color::from_hex(&s)
                .map(Some)
                .ok_or_else(|| serde::de::Error::custom(format!("bad colour {s:?}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Add a page (at the end unless `index` says where).
    CreatePage {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
    },
    /// Add a node to a page, frame, group or section. `index` counts from the bottom; the
    /// default puts it on top.
    Create {
        parent: Ref,
        node: NodeSpec,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
    },
    Set {
        id: Ref,
        patch: Patch,
    },
    /// Resize like dragging a handle: children without auto layout follow their constraints.
    Resize {
        id: Ref,
        width: f64,
        height: f64,
    },
    /// Move a node to another parent, or to another place in the same one. Its `x`/`y` stay
    /// the same numbers, now relative to the new parent.
    Move {
        id: Ref,
        parent: Ref,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<usize>,
    },
    /// Copy a node and everything in it, placed just above the original.
    Duplicate {
        id: Ref,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
    },
    /// Delete a node and everything in it (or a page, unless it's the last one).
    Delete {
        id: Ref,
    },
}

impl Op {
    pub fn name(&self) -> &'static str {
        match self {
            Self::CreatePage { .. } => "create_page",
            Self::Create { .. } => "create",
            Self::Set { .. } => "set",
            Self::Resize { .. } => "resize",
            Self::Move { .. } => "move",
            Self::Duplicate { .. } => "duplicate",
            Self::Delete { .. } => "delete",
        }
    }

    pub fn create(parent: impl Into<Ref>, node: NodeSpec) -> Self {
        Self::Create { parent: parent.into(), node, key: None, index: None }
    }

    /// Create with a key that later ops in the batch can refer to.
    pub fn create_as(key: &str, parent: impl Into<Ref>, node: NodeSpec) -> Self {
        Self::Create { parent: parent.into(), node, key: Some(key.to_owned()), index: None }
    }

    pub fn set(id: impl Into<Ref>, patch: Patch) -> Self {
        Self::Set { id: id.into(), patch }
    }
}

/// What a successful batch did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Applied {
    /// New nodes, in the order they were created (copies made by `Duplicate` included).
    pub created: Vec<NodeId>,
    /// Ids of the nodes created with a key.
    pub keys: BTreeMap<String, NodeId>,
    /// Existing nodes that changed, directly or through layout.
    pub changed: Vec<NodeId>,
    pub removed: Vec<NodeId>,
}

impl Applied {
    pub fn id(&self, key: &str) -> Option<NodeId> {
        self.keys.get(key).copied()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpError {
    /// Position of the failing op in the batch.
    pub index: usize,
    pub op: &'static str,
    pub message: String,
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "op {} ({}): {}. Nothing was applied.", self.index, self.op, self.message)
    }
}

impl std::error::Error for OpError {}

impl Document {
    /// Apply a batch of ops atomically, then lay out. On error the document is unchanged.
    pub fn apply(&mut self, ops: &[Op], measure: &dyn TextMeasure) -> Result<Applied, OpError> {
        let mut work = self.clone();
        let mut batch = Batch::default();
        for (index, op) in ops.iter().enumerate() {
            batch.apply(&mut work, op).map_err(|message| OpError { index, op: op.name(), message })?;
        }
        crate::layout::run(&mut work, measure);

        let created: std::collections::BTreeSet<NodeId> = batch.created.iter().copied().collect();
        let changed = work
            .nodes
            .keys()
            .filter(|id| !created.contains(id) && !work.same_node(self, **id))
            .copied()
            .collect();
        let removed = self.nodes.keys().filter(|id| !work.contains(**id)).copied().collect();
        *self = work;
        Ok(Applied { created: batch.created, keys: batch.keys, changed, removed })
    }
}

#[derive(Default)]
struct Batch {
    keys: BTreeMap<String, NodeId>,
    created: Vec<NodeId>,
}

impl Batch {
    fn resolve(&self, doc: &Document, r: &Ref) -> Result<NodeId, String> {
        match r {
            Ref::Id(id) if doc.contains(*id) => Ok(*id),
            Ref::Id(id) => Err(format!("node {id} doesn't exist")),
            Ref::Key(k) => self.keys.get(k).copied().ok_or_else(|| format!("no node was created with key {k:?} earlier in this batch")),
        }
    }

    fn remember(&mut self, key: &Option<String>, id: NodeId) -> Result<(), String> {
        if let Some(k) = key {
            if self.keys.contains_key(k) {
                return Err(format!("key {k:?} is used twice in this batch"));
            }
            self.keys.insert(k.clone(), id);
        }
        Ok(())
    }

    fn apply(&mut self, doc: &mut Document, op: &Op) -> Result<(), String> {
        match op {
            Op::CreatePage { name, key, index } => {
                let id = doc.alloc_id();
                self.remember(key, id)?;
                doc.insert(Node {
                    id,
                    name: name.clone(),
                    parent: None,
                    children: Vec::new(),
                    kind: NodeKind::Page { background: page_background() },
                    props: Props::default(),
                });
                let at = index.unwrap_or(doc.pages.len()).min(doc.pages.len());
                doc.pages.insert(at, id);
                self.created.push(id);
            }
            Op::Create { parent, node, key, index } => {
                let parent = self.resolve(doc, parent)?;
                if matches!(node.kind, NodeKind::Page { .. }) {
                    return Err("pages are made with create_page".into());
                }
                let p = doc.node(parent).expect("resolved");
                if !p.kind.is_container() {
                    return Err(format!("a {} can't hold other nodes", p.kind.type_name()));
                }
                let id = doc.alloc_id();
                self.remember(key, id)?;
                doc.insert(Node {
                    id,
                    name: node.name.clone(),
                    parent: Some(parent),
                    children: Vec::new(),
                    kind: node.kind.clone(),
                    props: node.props.clone(),
                });
                let kids = &mut doc.node_mut(parent).expect("resolved").children;
                kids.insert(index.unwrap_or(kids.len()).min(kids.len()), id);
                self.created.push(id);
                validate(doc, id)?;
            }
            Op::Set { id, patch } => {
                let id = self.resolve(doc, id)?;
                set(doc, id, patch)?;
                validate(doc, id)?;
                for c in doc.children(id).to_vec() {
                    validate(doc, c)?;
                }
            }
            Op::Resize { id, width, height } => {
                let id = self.resolve(doc, id)?;
                if !(width.is_finite() && height.is_finite() && *width >= 0.0 && *height >= 0.0) {
                    return Err(format!("size must be finite and not negative, got {width} x {height}"));
                }
                resize(doc, id, *width, *height);
                validate(doc, id)?;
            }
            Op::Move { id, parent, index } => {
                let id = self.resolve(doc, id)?;
                let parent = self.resolve(doc, parent)?;
                move_node(doc, id, parent, *index)?;
                validate(doc, id)?;
            }
            Op::Duplicate { id, key } => {
                let id = self.resolve(doc, id)?;
                let Some(parent) = doc.parent(id) else { return Err("pages can't be duplicated yet".into()) };
                let copy = copy_subtree(doc, id, parent, &mut self.created);
                self.remember(key, copy)?;
                let kids = &mut doc.node_mut(parent).expect("exists").children;
                let at = kids.iter().position(|&c| c == id).map_or(kids.len(), |i| i + 1);
                kids.insert(at, copy);
            }
            Op::Delete { id } => {
                let id = self.resolve(doc, id)?;
                match doc.parent(id) {
                    None => {
                        if doc.pages.len() == 1 {
                            return Err("a document needs at least one page".into());
                        }
                        doc.pages.retain(|&p| p != id);
                    }
                    Some(p) => doc.node_mut(p).expect("exists").children.retain(|&c| c != id),
                }
                for n in doc.descendants(id).into_iter().chain([id]) {
                    doc.nodes.remove(&n);
                }
            }
        }
        Ok(())
    }
}

fn set(doc: &mut Document, id: NodeId, patch: &Patch) -> Result<(), String> {
    let n = doc.node_mut(id).expect("resolved");
    let kind = n.kind.type_name();
    let wrong = |field: &str| Err(format!("{field} doesn't apply to a {kind}"));
    if let Some(v) = &patch.text {
        let NodeKind::Text { text } = &mut n.kind else { return wrong("text") };
        text.content = v.clone();
    }
    if let Some(v) = &patch.text_style {
        let NodeKind::Text { text } = &mut n.kind else { return wrong("text_style") };
        text.style = v.clone();
    }
    if let Some(v) = &patch.path {
        let NodeKind::Vector { path } = &mut n.kind else { return wrong("path") };
        *path = v.clone();
    }
    if let Some(v) = patch.sides {
        let NodeKind::Polygon { sides } = &mut n.kind else { return wrong("sides") };
        *sides = v;
    }
    if let Some(v) = patch.points {
        let NodeKind::Star { points, .. } = &mut n.kind else { return wrong("points") };
        *points = v;
    }
    if let Some(v) = patch.inner {
        let NodeKind::Star { inner, .. } = &mut n.kind else { return wrong("inner") };
        *inner = v;
    }
    if let Some(v) = patch.arc {
        let NodeKind::Ellipse { arc } = &mut n.kind else { return wrong("arc") };
        *arc = v;
    }
    if let Some(v) = patch.background {
        let NodeKind::Page { background } = &mut n.kind else { return wrong("background") };
        *background = v;
    }
    if let Some(v) = &patch.name {
        n.name = v.clone();
    }
    let p = &mut n.props;
    macro_rules! copy {
        ($($f:ident),*) => { $( if let Some(v) = &patch.$f { p.$f = v.clone(); } )* };
    }
    copy!(x, y, rotation, visible, locked, opacity, blend, fills, strokes, stroke, corner, effects, clip);
    copy!(layout, sizing_h, sizing_v, absolute, constraints, min_width, max_width, min_height, max_height, exports);
    // Typing a size turns hug/fill into a fixed size, as in the Design panel.
    if let Some(w) = patch.width {
        p.width = w;
        if patch.sizing_h.is_none() {
            p.sizing_h = Sizing::Fixed;
        }
    }
    if let Some(h) = patch.height {
        p.height = h;
        if patch.sizing_v.is_none() {
            p.sizing_v = Sizing::Fixed;
        }
    }
    if patch.layout == Some(None) {
        // Without auto layout, children can't fill or be absolute: they keep their place.
        for c in doc.children(id).to_vec() {
            let p = &mut doc.node_mut(c).expect("child").props;
            p.absolute = false;
            p.sizing_h = fixed_unless_hug(p.sizing_h);
            p.sizing_v = fixed_unless_hug(p.sizing_v);
        }
    }
    Ok(())
}

fn fixed_unless_hug(s: Sizing) -> Sizing {
    if s == Sizing::Fill { Sizing::Fixed } else { s }
}

/// Resize, moving and stretching children by their constraints (unless auto layout places
/// them). Hugging or filling sizes become fixed.
pub(crate) fn resize(doc: &mut Document, id: NodeId, width: f64, height: f64) {
    let n = doc.node_mut(id).expect("exists");
    let (old_w, old_h) = (n.props.width, n.props.height);
    if width != old_w {
        n.props.sizing_h = Sizing::Fixed;
    }
    if height != old_h {
        n.props.sizing_v = Sizing::Fixed;
    }
    n.props.width = width;
    n.props.height = height;
    if n.props.layout.is_some() || matches!(n.kind, NodeKind::Group) {
        return;
    }
    for c in doc.children(id).to_vec() {
        let p = doc.node(c).expect("child").props.clone();
        let (x, w) = follow(p.constraints.horizontal, p.x, p.width, old_w, width);
        let (y, h) = follow(p.constraints.vertical, p.y, p.height, old_h, height);
        let cn = doc.node_mut(c).expect("child");
        cn.props.x = x;
        cn.props.y = y;
        if w != p.width || h != p.height {
            resize(doc, c, w, h);
        }
    }
}

/// New position and size along one axis when the parent goes from `old` to `new`.
fn follow(c: Constraint, pos: f64, size: f64, old: f64, new: f64) -> (f64, f64) {
    let d = new - old;
    match c {
        Constraint::Start => (pos, size),
        Constraint::End => (pos + d, size),
        Constraint::Center => (pos + d / 2.0, size),
        Constraint::Both => (pos, (size + d).max(0.0)),
        Constraint::Scale if old > 0.0 => {
            let s = new / old;
            (pos * s, size * s)
        }
        Constraint::Scale => (pos, size),
    }
}

fn move_node(doc: &mut Document, id: NodeId, parent: NodeId, index: Option<usize>) -> Result<(), String> {
    let Some(old_parent) = doc.parent(id) else { return Err("pages can't be moved into other nodes".into()) };
    let p = doc.node(parent).expect("resolved");
    if !p.kind.is_container() {
        return Err(format!("a {} can't hold other nodes", p.kind.type_name()));
    }
    if doc.is_ancestor(id, parent) {
        return Err("can't move a node into itself".into());
    }
    let parent_has_layout = p.props.layout.is_some();
    doc.node_mut(old_parent).expect("exists").children.retain(|&c| c != id);
    let kids = &mut doc.node_mut(parent).expect("resolved").children;
    kids.insert(index.unwrap_or(kids.len()).min(kids.len()), id);
    let n = doc.node_mut(id).expect("exists");
    n.parent = Some(parent);
    if !parent_has_layout {
        n.props.absolute = false;
        n.props.sizing_h = fixed_unless_hug(n.props.sizing_h);
        n.props.sizing_v = fixed_unless_hug(n.props.sizing_v);
    }
    Ok(())
}

/// Deep-copies `id` under `parent` (not yet listed in the parent's children) and returns the
/// copy's id.
fn copy_subtree(doc: &mut Document, id: NodeId, parent: NodeId, created: &mut Vec<NodeId>) -> NodeId {
    let src = doc.node(id).expect("exists").clone();
    let new = doc.alloc_id();
    created.push(new);
    let children = src.children.iter().map(|&c| copy_subtree(doc, c, new, created)).collect();
    doc.insert(Node { id: new, parent: Some(parent), children, ..src });
    new
}

/// Checks one node against the rules that keep layout meaningful.
fn validate(doc: &Document, id: NodeId) -> Result<(), String> {
    let n = doc.node(id).expect("exists");
    let p = &n.props;
    let parent_layout = n.parent.and_then(|pid| doc.node(pid)).and_then(|pn| pn.props.layout.as_ref());
    let kind = n.kind.type_name();

    for (name, v) in [("x", p.x), ("y", p.y), ("rotation", p.rotation)] {
        if !v.is_finite() {
            return Err(format!("{name} must be a finite number"));
        }
    }
    if !(p.width.is_finite() && p.height.is_finite() && p.width >= 0.0 && p.height >= 0.0) {
        return Err(format!("size must be finite and not negative, got {} x {}", p.width, p.height));
    }
    if !(0.0..=1.0).contains(&p.opacity) {
        return Err(format!("opacity must be between 0 and 1, got {}", p.opacity));
    }
    if p.layout.is_some() && !matches!(n.kind, NodeKind::Frame) {
        return Err(format!("auto layout needs a frame, not a {kind}"));
    }
    let can_hug = matches!(n.kind, NodeKind::Text { .. }) || p.layout.is_some();
    for (axis, s) in [("width", p.sizing_h), ("height", p.sizing_v)] {
        match s {
            Sizing::Hug if !can_hug => return Err(format!("only text and auto layout frames can hug their {axis}")),
            Sizing::Fill if parent_layout.is_none() => {
                return Err(format!("fill {axis} needs an auto layout parent"));
            }
            Sizing::Fill if p.absolute => return Err(format!("an absolutely positioned node can't fill its {axis}")),
            _ => {}
        }
    }
    if let Some(l) = parent_layout
        && l.wrap
        && l.direction == Direction::Horizontal
        && p.sizing_h == Sizing::Fill
        && !p.absolute
    {
        return Err("fill width isn't supported in a wrapping layout yet".into());
    }
    if p.absolute && parent_layout.is_none() {
        return Err("absolute position only applies inside an auto layout parent".into());
    }
    match &n.kind {
        NodeKind::Text { text } if !(text.style.size > 0.0) => Err("font size must be above 0".into()),
        NodeKind::Polygon { sides } if *sides < 3 => Err("a polygon needs at least 3 sides".into()),
        NodeKind::Star { points, inner } if *points < 3 || !(0.0..=1.0).contains(inner) => {
            Err("a star needs at least 3 points and an inner ratio between 0 and 1".into())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ApproxMeasure;

    fn page(doc: &Document) -> NodeId {
        doc.pages()[0]
    }

    #[test]
    fn batches_are_atomic_and_report_the_failing_op() {
        let mut doc = Document::new();
        let p = page(&doc);
        let before = doc.len();
        let err = doc
            .apply(
                &[
                    Op::create_as("card", p, NodeSpec::frame("Card", 200.0, 100.0)),
                    Op::create("card", NodeSpec::rect("Fill me", 10.0, 10.0).sizing(Sizing::Fill, Sizing::Fixed)),
                ],
                &ApproxMeasure,
            )
            .unwrap_err();
        assert_eq!(err.index, 1);
        assert!(err.to_string().contains("fill width needs an auto layout parent"), "{err}");
        assert_eq!(doc.len(), before, "the frame from op 0 was rolled back");
    }

    #[test]
    fn keys_link_ops_in_one_batch() {
        let mut doc = Document::new();
        let p = page(&doc);
        let done = doc
            .apply(
                &[
                    Op::create_as("f", p, NodeSpec::frame("Frame", 300.0, 200.0)),
                    Op::create_as("r", "f", NodeSpec::rect("Box", 20.0, 20.0).at(5.0, 5.0)),
                    Op::set("r", Patch { name: Some("Renamed".into()), ..Patch::default() }),
                ],
                &ApproxMeasure,
            )
            .unwrap();
        let (f, r) = (done.id("f").unwrap(), done.id("r").unwrap());
        assert_eq!(done.created, vec![f, r]);
        assert_eq!(doc.children(f), &[r]);
        assert_eq!(doc.node(r).unwrap().name, "Renamed");
        assert_eq!(done.changed, vec![p], "only the page's child list changed");
    }

    #[test]
    fn untouched_nodes_keep_their_arc() {
        let mut doc = Document::new();
        let p = page(&doc);
        let a = doc.apply(&[Op::create(p, NodeSpec::rect("A", 1.0, 1.0)), Op::create(p, NodeSpec::rect("B", 1.0, 1.0))], &ApproxMeasure).unwrap();
        let (ra, rb) = (a.created[0], a.created[1]);
        let done = doc.apply(&[Op::set(ra, Patch { x: Some(9.0), ..Patch::default() })], &ApproxMeasure).unwrap();
        assert_eq!(done.changed, vec![ra]);
        assert!(!done.changed.contains(&rb));
    }

    #[test]
    fn resize_follows_constraints() {
        let mut doc = Document::new();
        let p = page(&doc);
        let c = |h, v| Constraints { horizontal: h, vertical: v };
        let done = doc
            .apply(
                &[
                    Op::create_as("f", p, NodeSpec::frame("F", 100.0, 100.0)),
                    Op::create_as("pin", "f", NodeSpec::rect("Pin", 10.0, 10.0).at(80.0, 80.0).with(|p| p.constraints = c(Constraint::End, Constraint::End))),
                    Op::create_as("bar", "f", NodeSpec::rect("Bar", 80.0, 10.0).at(10.0, 0.0).with(|p| p.constraints = c(Constraint::Both, Constraint::Start))),
                    Op::create_as("mid", "f", NodeSpec::rect("Mid", 20.0, 20.0).at(40.0, 40.0).with(|p| p.constraints = c(Constraint::Center, Constraint::Scale))),
                ],
                &ApproxMeasure,
            )
            .unwrap();
        doc.apply(&[Op::Resize { id: done.id("f").unwrap().into(), width: 200.0, height: 50.0 }], &ApproxMeasure).unwrap();
        let get = |k| doc.node(done.id(k).unwrap()).unwrap().props.clone();
        let (pin, bar, mid) = (get("pin"), get("bar"), get("mid"));
        assert_eq!((pin.x, pin.y), (180.0, 30.0));
        assert_eq!((bar.x, bar.width), (10.0, 180.0));
        assert_eq!((mid.x, mid.y, mid.height), (90.0, 20.0, 10.0));
    }

    #[test]
    fn move_rejects_cycles_and_drops_fill_outside_auto_layout() {
        let mut doc = Document::new();
        let p = page(&doc);
        let done = doc
            .apply(
                &[
                    Op::create_as("row", p, NodeSpec::frame("Row", 10.0, 10.0).auto_layout(AutoLayout::default())),
                    Op::create_as("child", "row", NodeSpec::frame("Child", 10.0, 10.0).sizing(Sizing::Fill, Sizing::Fixed)),
                ],
                &ApproxMeasure,
            )
            .unwrap();
        let (row, child) = (done.id("row").unwrap(), done.id("child").unwrap());
        let err = doc.apply(&[Op::Move { id: row.into(), parent: child.into(), index: None }], &ApproxMeasure).unwrap_err();
        assert!(err.message.contains("into itself"));
        doc.apply(&[Op::Move { id: child.into(), parent: p.into(), index: None }], &ApproxMeasure).unwrap();
        assert_eq!(doc.node(child).unwrap().props.sizing_h, Sizing::Fixed);
        assert_eq!(doc.parent(child), Some(p));
    }

    #[test]
    fn duplicate_copies_the_subtree_above_the_original() {
        let mut doc = Document::new();
        let p = page(&doc);
        let done = doc
            .apply(&[Op::create_as("f", p, NodeSpec::frame("F", 10.0, 10.0)), Op::create("f", NodeSpec::rect("R", 1.0, 1.0))], &ApproxMeasure)
            .unwrap();
        let f = done.id("f").unwrap();
        let dup = doc.apply(&[Op::Duplicate { id: f.into(), key: Some("copy".into()) }], &ApproxMeasure).unwrap();
        let copy = dup.id("copy").unwrap();
        assert_eq!(doc.children(p), &[f, copy]);
        assert_eq!(dup.created.len(), 2);
        let inner = doc.children(copy)[0];
        assert_eq!(doc.parent(inner), Some(copy));
        assert_ne!(inner, doc.children(f)[0]);
    }

    #[test]
    fn delete_removes_subtrees_but_keeps_the_last_page() {
        let mut doc = Document::new();
        let p = page(&doc);
        let done = doc
            .apply(&[Op::create_as("f", p, NodeSpec::frame("F", 10.0, 10.0)), Op::create("f", NodeSpec::rect("R", 1.0, 1.0))], &ApproxMeasure)
            .unwrap();
        let gone = doc.apply(&[Op::Delete { id: done.id("f").unwrap().into() }], &ApproxMeasure).unwrap();
        assert_eq!(gone.removed.len(), 2);
        assert_eq!(doc.len(), 1);
        assert!(doc.apply(&[Op::Delete { id: p.into() }], &ApproxMeasure).is_err());
    }

    #[test]
    fn ops_read_from_json() {
        let mut doc = Document::new();
        let p = page(&doc);
        let json = format!(
            r##"[
              {{"op":"create","parent":{p},"key":"btn","node":{{"name":"Button","type":"frame","x":24,"y":668,"width":345,"height":52,
                "fills":[{{"type":"solid","color":"#4C8DFF"}}],"corner":{{"top_left":14,"top_right":14,"bottom_right":14,"bottom_left":14}}}}}},
              {{"op":"set","id":"btn","patch":{{"opacity":0.5,"layout":{{"direction":"horizontal","padding":{{"left":24,"right":24}}}}}}}},
              {{"op":"set","id":"btn","patch":{{"layout":null}}}}
            ]"##
        );
        let ops: Vec<Op> = serde_json::from_str(&json).unwrap();
        let done = doc.apply(&ops, &ApproxMeasure).unwrap();
        let b = doc.node(done.id("btn").unwrap()).unwrap();
        assert!(matches!(b.kind, NodeKind::Frame));
        assert_eq!(b.props.opacity, 0.5);
        assert_eq!(b.props.layout, None, "null removed the auto layout");
        assert_eq!(b.props.corner, CornerRadii::all(14.0));
        assert_eq!(serde_json::from_str::<Vec<Op>>(&serde_json::to_string(&ops).unwrap()).unwrap(), ops);
    }
}
