use std::collections::BTreeMap;
use std::sync::Arc;

use kurbo::{Affine, Point, Rect};

use crate::node::{Node, NodeId, NodeKind, Props, page_background};

/// A Blobr document: pages, each the root of a tree of nodes, plus the images they use.
///
/// Reading is direct; changing goes through [`Document::apply`](crate::ops) so every edit is
/// atomic, validated, laid out and undoable.
#[derive(Debug, Clone)]
pub struct Document {
    pub(crate) nodes: BTreeMap<NodeId, Arc<Node>>,
    pub(crate) pages: Vec<NodeId>,
    pub(crate) next_id: u64,
    pub(crate) images: BTreeMap<String, Arc<Vec<u8>>>,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    /// A new document with one empty page.
    pub fn new() -> Self {
        let mut doc = Self::empty();
        let id = doc.alloc_id();
        doc.insert(Node {
            id,
            name: "Page 1".into(),
            parent: None,
            children: Vec::new(),
            kind: NodeKind::Page { background: page_background() },
            props: Props::default(),
        });
        doc.pages.push(id);
        doc
    }

    /// No pages at all (only valid as a starting point for loading).
    pub(crate) fn empty() -> Self {
        Self { nodes: BTreeMap::new(), pages: Vec::new(), next_id: 1, images: BTreeMap::new() }
    }

    pub fn pages(&self) -> &[NodeId] {
        &self.pages
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id).map(|n| &**n)
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    /// Number of nodes, pages included.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.node(id).map_or(&[], |n| &n.children)
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id)?.parent
    }

    /// The page a node is on (a page is on itself).
    pub fn page_of(&self, id: NodeId) -> Option<NodeId> {
        let mut cur = id;
        loop {
            let n = self.node(cur)?;
            match n.parent {
                Some(p) => cur = p,
                None => return n.is_page().then_some(cur),
            }
        }
    }

    /// Whether `ancestor` is `id` or one of its ancestors.
    pub fn is_ancestor(&self, ancestor: NodeId, id: NodeId) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.parent(c);
        }
        false
    }

    /// Every node below `id`, parents before their children.
    pub fn descendants(&self, id: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = self.children(id).iter().rev().copied().collect();
        while let Some(n) = stack.pop() {
            out.push(n);
            stack.extend(self.children(n).iter().rev().copied());
        }
        out
    }

    /// The first node named `name` anywhere in the document (pages first, then depth first).
    pub fn find_by_name(&self, name: &str) -> Option<NodeId> {
        self.pages.iter().find_map(|&p| {
            std::iter::once(p).chain(self.descendants(p)).find(|&id| self.node(id).is_some_and(|n| n.name == name))
        })
    }

    /// Maps the node's own space to its parent's.
    pub fn local_transform(node: &Node) -> Affine {
        let p = &node.props;
        // Counter-clockwise on screen (y down) is a negative angle for kurbo.
        Affine::translate((p.x, p.y)) * Affine::rotate(-p.rotation.to_radians())
    }

    /// Maps the node's own space to its page's.
    pub fn world_transform(&self, id: NodeId) -> Affine {
        let mut t = Affine::IDENTITY;
        let mut cur = self.node(id);
        while let Some(n) = cur {
            if n.is_page() {
                break;
            }
            t = Self::local_transform(n) * t;
            cur = n.parent.and_then(|p| self.node(p));
        }
        t
    }

    /// Axis-aligned bounds of the node's box in page coordinates.
    pub fn world_bounds(&self, id: NodeId) -> Option<Rect> {
        let n = self.node(id)?;
        let t = self.world_transform(id);
        let (w, h) = (n.props.width, n.props.height);
        let corners = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|(x, y)| t * Point::new(x, y));
        let mut r = Rect::from_points(corners[0], corners[0]);
        for c in &corners[1..] {
            r = r.union_pt(*c);
        }
        Some(r)
    }

    /// Image bytes (PNG, JPEG, ...) by key, as used by image paints.
    pub fn image(&self, key: &str) -> Option<&Arc<Vec<u8>>> {
        self.images.get(key)
    }

    /// Store image bytes and return their key (a content hash, so adding the same image twice
    /// stores it once). Unused images are dropped when the document is saved.
    pub fn add_image(&mut self, bytes: Vec<u8>) -> String {
        let key = content_key(&bytes);
        self.images.entry(key.clone()).or_insert_with(|| Arc::new(bytes));
        key
    }

    pub(crate) fn alloc_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        id
    }

    pub(crate) fn insert(&mut self, node: Node) {
        self.nodes.insert(node.id, Arc::new(node));
    }

    /// Copy-on-write access: only the node touched gets a new `Arc`, which is how a batch
    /// works out what changed.
    pub(crate) fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(&id).map(Arc::make_mut)
    }

    pub(crate) fn same_node(&self, other: &Document, id: NodeId) -> bool {
        match (self.nodes.get(&id), other.nodes.get(&id)) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

/// FNV-1a over the bytes, plus the length.
fn content_key(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}-{}", bytes.len())
}
