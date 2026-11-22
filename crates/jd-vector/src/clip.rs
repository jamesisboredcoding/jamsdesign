//! Copied layers. A [`Clip`] holds whole subtrees (nested, like a file) plus the images they
//! use, so it can be pasted into the same document or another one, and travels through the
//! system clipboard as text. Pasting gives every layer a new id.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kurbo::Rect;
use serde::{Deserialize, Serialize};

use crate::doc::Document;
use crate::file::{FileNode, to_file};
use crate::node::{Node, NodeId, NodeKind, PaintKind};

/// Marks clipboard text as Blobr layers.
const PREFIX: &str = "blobr-layers:";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    roots: Vec<FileNode>,
    /// Where the copied layers sat on their page, for pasting in place.
    bounds: Option<[f64; 4]>,
    /// The parents they were copied from.
    parents: Vec<NodeId>,
    /// Each layer's origin on its page, to place it again in another parent.
    #[serde(default)]
    origins: Vec<[f64; 2]>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    images: BTreeMap<String, Vec<u8>>,
}

impl Clip {
    /// Copies `ids` (bottom to top as they stack; descendants of other copied layers are
    /// skipped, pages are left out).
    pub fn copy(doc: &Document, ids: &[NodeId]) -> Option<Clip> {
        let mut picked: Vec<NodeId> = ids
            .iter()
            .copied()
            .filter(|&id| doc.parent(id).is_some() && !ids.iter().any(|&o| o != id && doc.is_ancestor(o, id)))
            .collect();
        if picked.is_empty() {
            return None;
        }
        // In stacking order, so pasting keeps it.
        let order = |id: NodeId| {
            let mut path = Vec::new();
            let mut cur = Some(id);
            while let Some(c) = cur {
                let p = doc.parent(c);
                path.push(p.map_or(0, |p| doc.children(p).iter().position(|&x| x == c).unwrap_or(0)));
                cur = p;
            }
            path.reverse();
            path
        };
        picked.sort_by_key(|&id| order(id));
        let roots: Vec<FileNode> = picked.iter().map(|&id| to_file(doc, id)).collect();
        let bounds = picked.iter().filter_map(|&id| doc.world_bounds(id)).reduce(|a, b| a.union(b)).map(|b| [b.x0, b.y0, b.x1, b.y1]);
        let parents = picked.iter().filter_map(|&id| doc.parent(id)).collect();
        let origins = picked
            .iter()
            .map(|&id| {
                let [.., x, y] = doc.world_transform(id).as_coeffs();
                [x, y]
            })
            .collect();
        let mut keys = BTreeSet::new();
        fn images(n: &FileNode, keys: &mut BTreeSet<String>) {
            for p in n.props.fills.iter().chain(&n.props.strokes) {
                if let PaintKind::Image { image, .. } = &p.kind {
                    keys.insert(image.clone());
                }
            }
            for c in &n.children {
                images(c, keys);
            }
        }
        for r in &roots {
            images(r, &mut keys);
        }
        let images = keys.into_iter().filter_map(|k| doc.image(&k).map(|b| (k, (**b).clone()))).collect();
        Some(Clip { roots, bounds, parents, origins, images })
    }

    /// A rectangle filled with an image (PNG, JPEG, ...) of `width` x `height`, as pasted
    /// from another app.
    pub fn image(name: &str, width: f64, height: f64, bytes: Vec<u8>) -> Clip {
        let key = crate::doc::content_key(&bytes);
        let mut spec = crate::NodeSpec::rect(name, width, height);
        spec.props.fills = vec![crate::Paint { kind: PaintKind::Image { image: key.clone(), fit: Default::default() }, ..crate::Paint::hex("#000000") }];
        let root = FileNode { id: NodeId(0), name: spec.name, kind: spec.kind, props: spec.props, children: Vec::new() };
        Clip { roots: vec![root], bounds: Some([0.0, 0.0, width, height]), parents: Vec::new(), origins: vec![[0.0, 0.0]], images: [(key, bytes)].into() }
    }

    pub fn len(&self) -> usize {
        self.roots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// The copied layers' box on their page when copied.
    pub fn bounds(&self) -> Option<Rect> {
        self.bounds.map(|[a, b, c, d]| Rect::new(a, b, c, d))
    }

    /// Where each copied layer came from.
    pub fn parents(&self) -> &[NodeId] {
        &self.parents
    }

    /// The ids the copied layers had where they were copied from.
    pub fn sources(&self) -> Vec<NodeId> {
        self.roots.iter().map(|r| r.id).collect()
    }

    /// Each copied layer's origin on its page when copied.
    pub fn origins(&self) -> &[[f64; 2]] {
        &self.origins
    }

    /// For the system clipboard.
    pub fn to_text(&self) -> String {
        format!("{PREFIX}{}", serde_json::to_string(self).expect("clips always serialize"))
    }

    /// Clipboard text back to layers, if it is some.
    pub fn from_text(text: &str) -> Option<Clip> {
        serde_json::from_str(text.strip_prefix(PREFIX)?).ok()
    }
}

/// Adds the clip's layers to `doc` under `parent` (not yet in its child list) and returns the
/// new roots. Instances keep their component when `doc` has it, otherwise they become frames.
pub(crate) fn paste(doc: &mut Document, clip: &Clip, parent: NodeId, created: &mut Vec<NodeId>) -> Vec<NodeId> {
    for (k, v) in &clip.images {
        doc.images.entry(k.clone()).or_insert_with(|| Arc::new(v.clone()));
    }
    let mut old_to_new = BTreeMap::new();
    fn add(doc: &mut Document, f: &FileNode, parent: NodeId, created: &mut Vec<NodeId>, map: &mut BTreeMap<NodeId, NodeId>) -> NodeId {
        let id = doc.alloc_id();
        created.push(id);
        map.insert(f.id, id);
        let children = f.children.iter().map(|c| add(doc, c, id, created, map)).collect();
        doc.insert(Node { id, name: f.name.clone(), parent: Some(parent), children, kind: f.kind.clone(), props: f.props.clone() });
        id
    }
    let roots: Vec<NodeId> = clip.roots.iter().map(|r| add(doc, r, parent, created, &mut old_to_new)).collect();
    // Prototype links between pasted layers point at the copies.
    for &new in old_to_new.values() {
        let n = doc.node_mut(new).expect("pasted");
        for r in &mut n.props.reactions {
            if let crate::node::Action::Navigate { to } | crate::node::Action::Overlay { to } = &mut r.action
                && let Some(&t) = old_to_new.get(to)
            {
                *to = t;
            }
        }
    }
    for &new in old_to_new.values() {
        let NodeKind::Instance { component, .. } = &doc.node(new).expect("pasted").kind else { continue };
        let component = *component;
        // The same id in another document may be anything: it must still be a component.
        let ok = doc.node(component).is_some_and(|c| matches!(c.kind, NodeKind::Component { .. }));
        let n = doc.node_mut(new).expect("pasted");
        if ok {
            if let NodeKind::Instance { map, .. } = &mut n.kind {
                *map = map.iter().filter_map(|(k, v)| old_to_new.get(v).map(|nv| (k.clone(), *nv))).collect();
            }
        } else {
            n.kind = NodeKind::Frame;
        }
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApproxMeasure, NodeSpec, Op};

    #[test]
    fn copy_paste_keeps_trees_order_and_instances() {
        let mut doc = Document::new();
        let page = doc.pages()[0];
        let done = doc
            .apply(
                &[
                    Op::create_as("card", page, NodeSpec::frame("Card", 100.0, 100.0).at(10.0, 10.0)),
                    Op::create_as("dot", "card", NodeSpec::ellipse("Dot", 10.0, 10.0)),
                    Op::create_as("btn", page, NodeSpec::frame("Button", 40.0, 20.0).at(200.0, 0.0)),
                    Op::MakeComponent { id: "btn".into() },
                    Op::CreateInstance { parent: page.into(), component: "btn".into(), key: Some("inst".into()), index: None, x: 300.0, y: 0.0 },
                ],
                &ApproxMeasure,
            )
            .unwrap();
        let id = |k| done.id(k).unwrap();
        // Copying the card and its own dot copies the card once.
        let clip = Clip::copy(&doc, &[id("inst"), id("dot"), id("card")]).unwrap();
        assert_eq!(clip.len(), 2);
        assert_eq!(clip.bounds(), Some(Rect::new(10.0, 0.0, 340.0, 110.0)));
        let clip = Clip::from_text(&clip.to_text()).unwrap();

        let pasted = doc.apply(&[Op::Paste { parent: page.into(), clip: clip.clone(), index: None, key: Some("p".into()), offset: [5.0, 0.0] }], &ApproxMeasure).unwrap();
        let (card, inst) = (pasted.id("p0").unwrap(), pasted.id("p1").unwrap());
        assert_ne!(card, id("card"));
        assert_eq!(doc.node(card).unwrap().props.x, 15.0);
        assert_eq!(doc.children(card).len(), 1);
        assert!(matches!(doc.node(inst).unwrap().kind, NodeKind::Instance { component, .. } if component == id("btn")));
        assert_eq!(doc.children(page).last(), Some(&inst), "stacking order kept, on top");

        // Into a document without the component: the instance becomes a frame.
        let mut other = Document::new();
        let op = other.pages()[0];
        let done = other.apply(&[Op::Paste { parent: op.into(), clip, index: None, key: Some("p".into()), offset: [0.0, 0.0] }], &ApproxMeasure).unwrap();
        assert!(matches!(other.node(done.id("p1").unwrap()).unwrap().kind, NodeKind::Frame));
        assert_eq!(Clip::from_text("hello"), None);
    }
}
