//! Components, variants and instances.
//!
//! An instance holds a full copy of its component's layers, so it renders, lays out and hit
//! tests like any other node. After every batch the copy is rebuilt ([`sync_all`]) from:
//! the component, the instance's property values, and its overrides.
//!
//! Each copied layer is identified by a *source key*: the id of the component layer it copies
//! (`"12"`), or for layers under a swapped nested instance, the path through it (`"12;40"`).
//! The instance's `map` keeps the same node id for the same key across rebuilds, and its
//! `overrides` are `Patch`es by key (`""` is the instance itself).
//!
//! Only the *outermost* instance (the owner) has a map and overrides; instances nested inside
//! it are plain copies whose changes are overrides on the owner, as in Figma.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::doc::Document;
use crate::node::*;
use crate::ops::{Patch, apply_patch};

/// Properties that belong to an instance itself and never come from its component.
pub(crate) fn keep_own(dst: &mut Props, own: &Props) {
    dst.x = own.x;
    dst.y = own.y;
    dst.rotation = own.rotation;
    dst.absolute = own.absolute;
    dst.constraints = own.constraints;
}

/// Clears the instance-owned fields from a patch recorded as an override on the instance.
pub(crate) fn without_own(p: &Patch) -> Patch {
    Patch { x: None, y: None, rotation: None, absolute: None, constraints: None, component: None, values: None, ..p.clone() }
}

impl Document {
    /// The instance whose copy `id` is part of (its outermost instance ancestor, or itself).
    pub fn owner_instance(&self, id: NodeId) -> Option<NodeId> {
        let mut found = None;
        let mut cur = Some(id);
        while let Some(c) = cur {
            let n = self.node(c)?;
            if matches!(n.kind, NodeKind::Instance { .. }) {
                found = Some(c);
            }
            cur = n.parent;
        }
        found
    }

    /// Whether `id` is a layer inside an instance (not the instance itself).
    pub fn is_inside_instance(&self, id: NodeId) -> bool {
        self.owner_instance(id).is_some_and(|o| o != id)
    }

    /// The source key of `id` within its owner instance.
    pub fn source_key(&self, owner: NodeId, id: NodeId) -> Option<String> {
        if owner == id {
            return Some(String::new());
        }
        let NodeKind::Instance { map, .. } = &self.node(owner)?.kind else { return None };
        map.iter().find(|(_, v)| **v == id).map(|(k, _)| k.clone())
    }

    /// The main component a node is (or is a variant of): itself, or for a variant, its set.
    pub fn component_set_of(&self, component: NodeId) -> Option<NodeId> {
        let p = self.parent(component)?;
        matches!(self.node(p)?.kind, NodeKind::ComponentSet { .. }).then_some(p)
    }

    /// Variant properties of a component set: each property and its values, in order seen.
    pub fn variant_properties(&self, set: NodeId) -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for &c in self.children(set) {
            if let Some(n) = self.node(c) {
                for (k, v) in parse_variant(&n.name) {
                    let vals = out.entry(k).or_default();
                    if !vals.contains(&v) {
                        vals.push(v);
                    }
                }
            }
        }
        out
    }

    /// The variant in `set` matching `want`, taking unspecified properties from `from`.
    pub fn find_variant(&self, set: NodeId, from: Option<NodeId>, want: &BTreeMap<String, String>) -> Option<NodeId> {
        let mut target = from.and_then(|f| self.node(f)).map(|n| parse_variant(&n.name)).unwrap_or_default();
        target.extend(want.iter().map(|(k, v)| (k.clone(), v.clone())));
        self.children(set).iter().copied().find(|&c| self.node(c).is_some_and(|n| parse_variant(&n.name) == target))
    }

    /// Components used (through instances, swaps and swap defaults) anywhere inside `id`,
    /// directly or further down.
    pub fn components_used(&self, id: NodeId) -> BTreeSet<NodeId> {
        let mut out = BTreeSet::new();
        let mut stack = vec![id];
        while let Some(c) = stack.pop() {
            for n in std::iter::once(c).chain(self.descendants(c)) {
                let Some(node) = self.node(n) else { continue };
                let mut refs = Vec::new();
                if let NodeKind::Instance { component, values, overrides, .. } = &node.kind {
                    refs.push(*component);
                    refs.extend(values.values().filter_map(|v| if let PropValue::Node(t) = v { Some(*t) } else { None }));
                    refs.extend(overrides.values().filter_map(|p| p.component));
                }
                if let NodeKind::Component { properties, .. } = &node.kind {
                    refs.extend(properties.values().filter_map(|d| if let PropDef::InstanceSwap { default } = d { Some(*default) } else { None }));
                }
                for r in refs {
                    if out.insert(r) {
                        stack.push(r);
                    }
                }
            }
        }
        out
    }

    /// Whether putting an instance of `component` inside `parent` would make a component
    /// contain itself.
    pub fn would_cycle(&self, parent: NodeId, component: NodeId) -> bool {
        let mut used = self.components_used(component);
        used.insert(component);
        let mut cur = Some(parent);
        while let Some(c) = cur {
            if used.contains(&c) {
                return true;
            }
            cur = self.parent(c);
        }
        false
    }

    /// Layer names from just under `root` down to `id`.
    pub(crate) fn name_path(&self, root: NodeId, id: NodeId) -> Option<Vec<String>> {
        let mut path = Vec::new();
        let mut cur = id;
        while cur != root {
            let n = self.node(cur)?;
            path.push(n.name.clone());
            cur = n.parent?;
        }
        path.reverse();
        Some(path)
    }

    pub(crate) fn find_path(&self, root: NodeId, path: &[String]) -> Option<NodeId> {
        let mut cur = root;
        for name in path {
            cur = self.children(cur).iter().copied().find(|&c| self.node(c).is_some_and(|n| &n.name == name))?;
        }
        Some(cur)
    }
}

/// `Size=Large, State=Default` as pairs. Names without `=` have no variant properties.
pub fn parse_variant(name: &str) -> BTreeMap<String, String> {
    name.split(',')
        .filter_map(|part| part.split_once('=').map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())))
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

/// Rewrites source keys from one component to another by matching layer-name paths, so
/// overrides survive switching variants. Keys with no match are dropped.
pub(crate) fn remap_keys<T: Clone>(doc: &Document, from: NodeId, to: NodeId, keyed: &BTreeMap<String, T>) -> BTreeMap<String, T> {
    let mut out = BTreeMap::new();
    for (key, v) in keyed {
        if key.is_empty() {
            out.insert(key.clone(), v.clone());
            continue;
        }
        let (head, rest) = key.split_once(';').map_or((key.as_str(), None), |(h, r)| (h, Some(r)));
        let Ok(src) = head.parse::<u64>() else { continue };
        let Some(path) = doc.name_path(from, NodeId(src)) else { continue };
        let Some(dst) = doc.find_path(to, &path) else { continue };
        let new = match rest {
            Some(r) => format!("{};{r}", dst.0),
            None => dst.0.to_string(),
        };
        out.insert(new, v.clone());
    }
    out
}

fn defaults(doc: &Document, component: NodeId, values: &BTreeMap<String, PropValue>) -> BTreeMap<String, PropValue> {
    let mut scope: BTreeMap<String, PropValue> = match doc.node(component).map(|n| &n.kind) {
        Some(NodeKind::Component { properties, .. }) => properties.iter().map(|(k, d)| (k.clone(), d.default_value())).collect(),
        _ => BTreeMap::new(),
    };
    scope.extend(values.iter().map(|(k, v)| (k.clone(), v.clone())));
    scope
}

fn is_component(doc: &Document, id: NodeId) -> bool {
    doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Component { .. }))
}

/// Writes a node unless it's already exactly that (so untouched nodes keep their `Arc`).
fn put(doc: &mut Document, node: Node) {
    if doc.node(node.id) != Some(&node) {
        doc.insert(node);
    }
}

#[derive(Default)]
struct Sync {
    done: HashSet<NodeId>,
    done_components: HashSet<NodeId>,
    visiting: HashSet<NodeId>,
}

/// Rebuilds every instance from its component.
pub(crate) fn sync_all(doc: &mut Document) {
    let owners: Vec<NodeId> = doc
        .nodes
        .iter()
        .filter(|(_, n)| matches!(n.kind, NodeKind::Instance { .. }))
        .map(|(id, _)| *id)
        .filter(|&id| doc.owner_instance(id) == Some(id))
        .collect();
    let mut st = Sync::default();
    for o in owners {
        sync_instance(doc, o, &mut st);
    }
}

/// Makes sure the instances inside a component are up to date before it is copied.
fn sync_component(doc: &mut Document, c: NodeId, st: &mut Sync) {
    if st.done_components.contains(&c) || st.visiting.contains(&c) {
        return;
    }
    st.visiting.insert(c);
    let owners: Vec<NodeId> = doc
        .descendants(c)
        .into_iter()
        .filter(|&id| matches!(doc.node(id).map(|n| &n.kind), Some(NodeKind::Instance { .. })) && doc.owner_instance(id) == Some(id))
        .collect();
    for o in owners {
        sync_instance(doc, o, st);
    }
    st.visiting.remove(&c);
    st.done_components.insert(c);
}

fn sync_instance(doc: &mut Document, i: NodeId, st: &mut Sync) {
    if !st.done.insert(i) {
        return;
    }
    let Some(inst) = doc.node(i).cloned() else { return };
    let NodeKind::Instance { component, values, overrides, map } = &inst.kind else { return };
    if !is_component(doc, *component) || st.visiting.contains(component) {
        return;
    }
    sync_component(doc, *component, st);
    let comp = doc.node(*component).expect("checked").clone();

    let mut props = comp.props.clone();
    if let Some(p) = overrides.get("") {
        let mut tmp = Node { props, ..comp.clone() };
        let _ = apply_patch(&mut tmp, p);
        props = tmp.props;
    }
    keep_own(&mut props, &inst.props);

    let before: HashSet<NodeId> = doc.descendants(i).into_iter().collect();
    let scope = defaults(doc, *component, values);
    let mut b = Builder { old: map, new: BTreeMap::new(), overrides };
    let children: Vec<NodeId> = comp.children.iter().map(|&s| b.build(doc, s, i, "", &scope, st)).collect();
    let kept: HashSet<NodeId> = b.new.values().copied().collect();
    for gone in before.difference(&kept) {
        doc.nodes.remove(gone);
    }
    let used: BTreeMap<String, Patch> =
        overrides.iter().filter(|(k, _)| k.is_empty() || b.new.contains_key(*k)).map(|(k, v)| (k.clone(), v.clone())).collect();
    let node = Node {
        id: i,
        name: inst.name.clone(),
        parent: inst.parent,
        children,
        kind: NodeKind::Instance { component: *component, values: values.clone(), overrides: used, map: b.new },
        props,
    };
    put(doc, node);
}

struct Builder<'a> {
    old: &'a BTreeMap<String, NodeId>,
    new: BTreeMap<String, NodeId>,
    overrides: &'a BTreeMap<String, Patch>,
}

impl Builder<'_> {
    /// Copies source node `s` (and what's in it) under `parent`, returning the copy's id.
    fn build(
        &mut self,
        doc: &mut Document,
        s: NodeId,
        parent: NodeId,
        prefix: &str,
        scope: &BTreeMap<String, PropValue>,
        st: &mut Sync,
    ) -> NodeId {
        let src = doc.node(s).expect("source exists").clone();
        let key = format!("{prefix}{}", s.0);
        let id = match self.old.get(&key) {
            Some(&id) => id,
            None => doc.alloc_id(),
        };
        self.new.insert(key.clone(), id);
        let mut node = Node { id, name: src.name.clone(), parent: Some(parent), children: Vec::new(), kind: src.kind.clone(), props: src.props.clone() };

        let refs = &src.props.refs;
        if let (Some(p), NodeKind::Text { text }) = (&refs.text, &mut node.kind)
            && let Some(PropValue::Text(t)) = scope.get(p)
        {
            text.content = t.clone();
        }
        if let Some(p) = &refs.visible
            && let Some(PropValue::Bool(b)) = scope.get(p)
        {
            node.props.visible = *b;
        }
        let ov = self.overrides.get(&key);

        match &src.kind {
            NodeKind::Instance { component, values, .. } => {
                let mut swap = None;
                if let Some(p) = &refs.swap
                    && let Some(PropValue::Node(t)) = scope.get(p)
                {
                    swap = Some(*t);
                }
                if let Some(c) = ov.and_then(|o| o.component) {
                    swap = Some(c);
                }
                let target = swap.filter(|t| *t != *component && is_component(doc, *t) && !st.visiting.contains(t));
                let mut vals = values.clone();
                if let Some(v) = ov.and_then(|o| o.values.as_ref()) {
                    vals.extend(v.iter().map(|(k, v)| (k.clone(), v.clone())));
                }
                let (comp, kids, child_prefix) = match target {
                    Some(t) => {
                        sync_component(doc, t, st);
                        let tn = doc.node(t).expect("checked").clone();
                        let mut props = tn.props.clone();
                        keep_own(&mut props, &src.props);
                        props.visible = node.props.visible;
                        node.props = props;
                        (t, tn.children.clone(), format!("{key};"))
                    }
                    None => (*component, src.children.clone(), prefix.to_owned()),
                };
                if let Some(o) = ov {
                    let _ = apply_patch(&mut node, o);
                }
                let inner = defaults(doc, comp, &vals);
                node.kind = NodeKind::Instance { component: comp, values: vals, overrides: BTreeMap::new(), map: BTreeMap::new() };
                node.children = kids.iter().map(|&c| self.build(doc, c, id, &child_prefix, &inner, st)).collect();
            }
            _ => {
                if let Some(o) = ov {
                    let _ = apply_patch(&mut node, o);
                }
                node.children = src.children.iter().map(|&c| self.build(doc, c, id, prefix, scope, st)).collect();
            }
        }
        put(doc, node);
        id
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    fn text_of(doc: &Document, id: NodeId) -> String {
        match &doc.node(id).unwrap().kind {
            NodeKind::Text { text } => text.content.clone(),
            k => panic!("{k:?}"),
        }
    }

    /// A "Button" component (auto layout, blue, with a "Label" text bound to a "Label" text
    /// property and an "Icon" ellipse bound to a "Show icon" boolean).
    fn button(doc: &mut Document) -> Applied {
        let p = doc.pages()[0];
        let props = [
            ("Label".to_string(), PropDef::Text { default: "Button".into() }),
            ("Show icon".to_string(), PropDef::Boolean { default: false }),
        ];
        doc.apply(
            &[
                Op::create_as(
                    "btn",
                    p,
                    NodeSpec::new("Button", NodeKind::Component { description: String::new(), properties: props.into() })
                        .fill(Paint::hex("#4C8DFF"))
                        .auto_layout(AutoLayout { gap: 8.0, padding: Padding::hv(16.0, 8.0), ..AutoLayout::default() }),
                ),
                Op::create_as("icon", "btn", NodeSpec::ellipse("Icon", 10.0, 10.0).with(|p| p.refs.visible = Some("Show icon".into()))),
                Op::create_as(
                    "label",
                    "btn",
                    NodeSpec::text("Label", "Button", TextStyle { size: 10.0, ..TextStyle::default() }).with(|p| p.refs.text = Some("Label".into())),
                ),
            ],
            &ApproxMeasure,
        )
        .unwrap()
    }

    fn instance(doc: &mut Document, component: NodeId) -> NodeId {
        let p = doc.pages()[0];
        let done = doc
            .apply(&[Op::CreateInstance { parent: p.into(), component: component.into(), key: None, index: None, x: 0.0, y: 200.0 }], &ApproxMeasure)
            .unwrap();
        done.created[0]
    }

    fn child(doc: &Document, of: NodeId, name: &str) -> NodeId {
        doc.descendants(of).into_iter().find(|&c| doc.node(c).unwrap().name == name).unwrap()
    }

    #[test]
    fn instances_copy_their_component_and_follow_edits() {
        let mut doc = Document::new();
        let made = button(&mut doc);
        let btn = made.id("btn").unwrap();
        let inst = instance(&mut doc, btn);
        let label = child(&doc, inst, "Label");
        assert_eq!(text_of(&doc, label), "Button");
        assert!(!doc.node(child(&doc, inst, "Icon")).unwrap().props.visible, "boolean default hides the icon");
        assert_eq!(doc.node(inst).unwrap().props.y, 200.0);
        let w = doc.node(inst).unwrap().props.width;

        // Editing the component updates the instance; the copy keeps its ids.
        doc.apply(&[Op::set(made.id("btn").unwrap(), Patch { fills: Some(vec![Paint::hex("#E01B5A")]), ..Patch::default() })], &ApproxMeasure)
            .unwrap();
        assert_eq!(doc.node(inst).unwrap().props.fills, vec![Paint::hex("#E01B5A")]);
        assert_eq!(child(&doc, inst, "Label"), label);
        assert_eq!(doc.node(inst).unwrap().props.y, 200.0, "the instance keeps its own position");

        // Property values.
        let values = [("Label".to_string(), PropValue::Text("Get started".into())), ("Show icon".to_string(), PropValue::Bool(true))];
        doc.apply(&[Op::set(inst, Patch { values: Some(values.into()), ..Patch::default() })], &ApproxMeasure).unwrap();
        assert_eq!(text_of(&doc, label), "Get started");
        assert!(doc.node(child(&doc, inst, "Icon")).unwrap().props.visible);
        assert!(doc.node(inst).unwrap().props.width > w, "auto layout grew with the longer label");
    }

    #[test]
    fn overrides_survive_component_changes_and_can_be_reset() {
        let mut doc = Document::new();
        let made = button(&mut doc);
        let inst = instance(&mut doc, made.id("btn").unwrap());
        let icon = child(&doc, inst, "Icon");
        doc.apply(&[Op::set(icon, Patch { fills: Some(vec![Paint::hex("#00FF00")]), ..Patch::default() })], &ApproxMeasure).unwrap();
        // The component's icon changes size but the instance keeps its green override.
        doc.apply(&[Op::Resize { id: made.id("icon").unwrap().into(), width: 20.0, height: 20.0 }], &ApproxMeasure).unwrap();
        let n = doc.node(icon).unwrap();
        assert_eq!((n.props.fills.clone(), n.props.width), (vec![Paint::hex("#00FF00")], 20.0));

        // Structural edits inside an instance are refused.
        let err = doc.apply(&[Op::Delete { id: icon.into() }], &ApproxMeasure).unwrap_err();
        assert!(err.message.contains("detach"), "{err}");
        assert!(doc.apply(&[Op::create(inst, NodeSpec::rect("X", 1.0, 1.0))], &ApproxMeasure).is_err());

        doc.apply(&[Op::ResetOverrides { id: icon.into() }], &ApproxMeasure).unwrap();
        assert_eq!(doc.node(icon).unwrap().props.fills, vec![Paint::hex("#D9D9D9")]);
    }

    #[test]
    fn variants_switch_by_property_and_keep_overrides() {
        let mut doc = Document::new();
        let p = doc.pages()[0];
        let comp = |name: &str, color: &str| {
            NodeSpec::new(name, NodeKind::Component { description: String::new(), properties: Default::default() })
                .size(80.0, 30.0)
                .fill(Paint::hex(color))
        };
        let done = doc
            .apply(
                &[
                    Op::create_as("a", p, comp("Size=Small, State=Default", "#111111")),
                    Op::create_as("b", p, comp("Size=Large, State=Default", "#222222").at(100.0, 0.0)),
                    Op::create("a", NodeSpec::text("Label", "Small", TextStyle::default())),
                    Op::create("b", NodeSpec::text("Label", "Large", TextStyle::default())),
                    Op::CombineAsVariants { ids: vec!["a".into(), "b".into()], name: "Button".into(), key: Some("set".into()) },
                ],
                &ApproxMeasure,
            )
            .unwrap();
        let set = done.id("set").unwrap();
        assert_eq!(doc.variant_properties(set)["Size"], ["Small", "Large"]);
        let set_node = doc.node(set).unwrap();
        assert_eq!((set_node.props.width, set_node.props.height), (180.0 + 40.0, 30.0 + 40.0), "fits the variants plus padding");

        let inst = instance(&mut doc, done.id("a").unwrap());
        let label = child(&doc, inst, "Label");
        doc.apply(&[Op::set(label, Patch { text: Some("Mine".into()), ..Patch::default() })], &ApproxMeasure).unwrap();
        doc.apply(&[Op::set(inst, Patch { values: Some([("Size".to_string(), PropValue::Text("Large".into()))].into()), ..Patch::default() })], &ApproxMeasure)
            .unwrap();
        let n = doc.node(inst).unwrap();
        assert!(matches!(n.kind, NodeKind::Instance { component, .. } if component == done.id("b").unwrap()));
        assert_eq!(n.props.fills, vec![Paint::hex("#222222")]);
        assert_eq!(child(&doc, inst, "Label"), label, "same layer id after switching");
        assert_eq!(text_of(&doc, label), "Mine", "the text override followed the layer");
        let err = doc
            .apply(&[Op::set(inst, Patch { values: Some([("Size".to_string(), PropValue::Text("Huge".into()))].into()), ..Patch::default() })], &ApproxMeasure)
            .unwrap_err();
        assert!(err.message.contains("no variant"), "{err}");
    }

    #[test]
    fn nested_instances_can_be_swapped_and_cycles_are_refused() {
        let mut doc = Document::new();
        let p = doc.pages()[0];
        let comp = |name: &str| NodeSpec::new(name, NodeKind::Component { description: String::new(), properties: Default::default() }).size(20.0, 20.0);
        let done = doc
            .apply(
                &[
                    Op::create_as("star", p, comp("Star").fill(Paint::hex("#FFC53D"))),
                    Op::create_as("heart", p, comp("Heart").fill(Paint::hex("#E01B5A")).at(40.0, 0.0)),
                    Op::create_as("card", p, comp("Card").size(100.0, 100.0).at(0.0, 100.0)),
                    Op::CreateInstance { parent: "card".into(), component: "star".into(), key: Some("icon".into()), index: None, x: 10.0, y: 10.0 },
                ],
                &ApproxMeasure,
            )
            .unwrap();
        let (card, heart) = (done.id("card").unwrap(), done.id("heart").unwrap());
        let inst = instance(&mut doc, card);
        let nested = doc.children(inst)[0];
        assert!(matches!(doc.node(nested).unwrap().kind, NodeKind::Instance { .. }));
        doc.apply(&[Op::set(nested, Patch { component: Some(heart), ..Patch::default() })], &ApproxMeasure).unwrap();
        let swapped = doc.children(inst)[0];
        assert_eq!(swapped, nested, "same layer, other component");
        assert_eq!(doc.node(swapped).unwrap().props.fills, vec![Paint::hex("#E01B5A")]);
        assert_eq!(doc.node(swapped).unwrap().props.x, 10.0);

        // A card instance inside the star component would make Card contain itself.
        let err = doc
            .apply(&[Op::CreateInstance { parent: done.id("star").unwrap().into(), component: card.into(), key: None, index: None, x: 0.0, y: 0.0 }], &ApproxMeasure)
            .unwrap_err();
        assert!(err.message.contains("contain itself"), "{err}");
    }

    #[test]
    fn detach_and_duplicate_and_save() {
        let mut doc = Document::new();
        let made = button(&mut doc);
        let inst = instance(&mut doc, made.id("btn").unwrap());
        let copy = doc.apply(&[Op::Duplicate { id: inst.into(), key: None }], &ApproxMeasure).unwrap().created[0];
        assert_eq!(doc.children(copy).len(), 2);
        assert!(doc.children(copy).iter().all(|c| !doc.children(inst).contains(c)), "the copy has its own layers");
        let icon = made.id("icon").unwrap();
        let icon_w = |doc: &Document, of| doc.node(child(doc, of, "Icon")).unwrap().props.width;
        doc.apply(&[Op::Resize { id: icon.into(), width: 30.0, height: 30.0 }], &ApproxMeasure).unwrap();
        assert_eq!(icon_w(&doc, copy), 30.0);

        let bytes = file::to_json(&doc);
        let back = file::from_json(&bytes).unwrap();
        assert_eq!(back.node(inst), doc.node(inst), "instances round-trip with their maps");

        doc.apply(&[Op::Detach { id: inst.into() }], &ApproxMeasure).unwrap();
        assert!(matches!(doc.node(inst).unwrap().kind, NodeKind::Frame));
        doc.apply(&[Op::Resize { id: icon.into(), width: 40.0, height: 40.0 }], &ApproxMeasure).unwrap();
        assert_eq!(icon_w(&doc, inst), 30.0, "a detached copy no longer follows");
        assert_eq!(icon_w(&doc, copy), 40.0);
    }
}
