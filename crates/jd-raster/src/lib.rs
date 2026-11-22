//! The raster document model.
//!
//! Layers hold their pixels in a [`TileGrid`]. This crate must never depend on any UI toolkit.

pub mod adjust;
pub mod blend;
pub mod composite;
pub mod filter;
pub mod gradient;
pub mod group;
pub mod heal;
pub mod style;
pub mod text;
pub mod image_ops;
pub mod mask;
pub mod render;
mod history;
pub mod paint;
pub mod path;
pub mod selection;
pub mod transform;
mod tile;

pub use blend::BlendMode;
pub use composite::BlendSpace;
pub use history::History;
pub use tile::{TILE_SIZE, Tile, TileGrid, TileId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerId(u64);

#[derive(Debug, Clone)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    /// Lock image pixels: painting, fills, filters and adjustments are refused.
    pub lock_pixels: bool,
    /// Lock position: Move and Free Transform are refused.
    pub lock_position: bool,
    /// 0.0..=1.0
    pub opacity: f32,
    pub blend: BlendMode,
    pub pixels: TileGrid,
    /// Layer effects (drop shadow, stroke, ...), drawn on top of `pixels` when rendering.
    pub style: style::LayerStyle,
    /// Set on text layers (the Type tool); `pixels` hold the rendered text.
    pub text: Option<text::TextLayer>,
    /// Layer mask: hides parts of the layer without changing its pixels.
    pub mask: Option<mask::Mask>,
    /// Set on adjustment layers: instead of pixels, they show the layers below adjusted
    /// (see [`render`]).
    pub adjustment: Option<adjust::Adjustment>,
    /// The group this layer is in (see [`group`]).
    pub parent: Option<LayerId>,
    /// Set on groups (they have no pixels of their own; see [`render`]).
    pub group: Option<Group>,
}

/// A layer group's own settings (its opacity, blend mode, mask... are the layer's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    /// Photoshop's "Pass Through" blend mode: the layers inside blend with everything below
    /// as if they weren't grouped. Otherwise they're combined first, then blended as one
    /// layer with the group's blend mode.
    pub pass_through: bool,
    /// Expanded in the layers panel.
    pub open: bool,
}

impl Layer {
    /// An empty, visible, normal layer.
    pub fn new(id: LayerId, name: String, width: u32, height: u32) -> Self {
        Self {
            id,
            name,
            visible: true,
            locked: false,
            lock_pixels: false,
            lock_position: false,
            opacity: 1.0,
            blend: BlendMode::Normal,
            pixels: TileGrid::empty(width, height),
            style: style::LayerStyle::default(),
            text: None,
            mask: None,
            adjustment: None,
            parent: None,
            group: None,
        }
    }

    /// What the layer looks like: its pixels plus its effects (cached; the pixels themselves
    /// when it has none). Everything that draws or flattens layers uses this.
    pub fn rendered(&self) -> std::borrow::Cow<'_, TileGrid> {
        use std::borrow::Cow;
        // The mask cuts the layer first, so effects follow the visible shape (as in Photoshop).
        let masked = match &self.mask {
            Some(m) if m.enabled => Cow::Owned(mask::apply(&self.pixels, m)),
            _ => Cow::Borrowed(&self.pixels),
        };
        if self.style.is_empty() { masked } else { Cow::Owned(style::render_cached(&masked, &self.style)) }
    }

    /// The grid that edits go to: the mask when `mask` is set and the layer has one.
    pub fn target(&self, mask: bool) -> &TileGrid {
        match &self.mask {
            Some(m) if mask => &m.pixels,
            _ => &self.pixels,
        }
    }

    pub fn target_mut(&mut self, mask: bool) -> &mut TileGrid {
        match &mut self.mask {
            Some(m) if mask => &mut m.pixels,
            _ => &mut self.pixels,
        }
    }
}

/// A guide dragged out of a ruler: a vertical line at x = `pos`, or a horizontal one at
/// y = `pos` (document pixels).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guide {
    pub vertical: bool,
    pub pos: f32,
}

#[derive(Debug, Clone)]
pub struct Document {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub blend_space: BlendSpace,
    /// Selected pixels. `None` means nothing is selected (edits apply everywhere). Part of the
    /// document so selection changes are undoable, like in Photoshop. Not saved to files.
    pub selection: Option<selection::Selection>,
    /// Edits go to the active layer's mask instead of its pixels (Photoshop: the mask
    /// thumbnail is targeted). Only meaningful while the active layer has a mask; see
    /// [`Document::editing_mask`].
    pub mask_target: bool,
    /// Vector paths (the Paths panel), and the one being shown and edited.
    pub paths: Vec<path::VectorPath>,
    pub active_path: Option<usize>,
    /// Ruler guides (View > Guides).
    pub guides: Vec<Guide>,
    /// Bottom-to-top, like the compositing order. UIs usually display this reversed.
    layers: Vec<Layer>,
    active: Option<LayerId>,
    next_id: u64,
}

impl Document {
    /// An empty document with a locked, transparent "Background" layer.
    pub fn new(name: impl Into<String>, width: u32, height: u32) -> Self {
        let mut doc = Self::blank(name, width, height);
        let bg = doc.add_layer("Background");
        doc.layer_mut(bg).expect("just added").locked = true;
        doc
    }

    /// A document whose locked "Background" layer holds the given RGBA8 (sRGB, straight alpha)
    /// pixels: what you get when opening an image file.
    pub fn from_rgba8(name: impl Into<String>, width: u32, height: u32, pixels: &[u8]) -> Self {
        let mut doc = Self::new(name, width, height);
        let bg = doc.layers[0].id;
        doc.layer_mut(bg).expect("background").pixels = TileGrid::from_rgba8(width, height, pixels);
        doc
    }

    /// A document with no layers at all, for loaders that add their own.
    pub fn blank(name: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            name: name.into(),
            width,
            height,
            blend_space: BlendSpace::default(),
            selection: None,
            mask_target: false,
            paths: Vec::new(),
            active_path: None,
            guides: Vec::new(),
            layers: Vec::new(),
            active: None,
            next_id: 0,
        }
    }

    /// Adds a layer above the active one (or on top) and makes it active.
    pub fn add_layer(&mut self, name: impl Into<String>) -> LayerId {
        let id = LayerId(self.next_id);
        self.next_id += 1;
        let mut layer = Layer::new(id, name.into(), self.width, self.height);
        // Next to the active layer, in the same group (above a group, outside it).
        layer.parent = self.active().and_then(|a| a.parent);
        let index = self
            .active
            .and_then(|a| self.index_of(a))
            .map_or(self.layers.len(), |i| i + 1);
        self.layers.insert(index, layer);
        self.active = Some(id);
        self.mask_target = false;
        id
    }

    /// Copies a layer (sharing its tiles) above itself and makes the copy active. A group is
    /// copied with everything inside it.
    pub fn duplicate_layer(&mut self, id: LayerId) -> Option<LayerId> {
        let index = self.index_of(id)?;
        let block = self.block(index);
        let mut copies: Vec<Layer> = self.layers[block.clone()].to_vec();
        // New ids, with the copies' groups pointing at the copied groups.
        let mut ids = std::collections::HashMap::new();
        for l in &mut copies {
            let new = LayerId(self.next_id);
            self.next_id += 1;
            ids.insert(l.id, new);
            l.id = new;
        }
        for l in &mut copies {
            l.parent = l.parent.map(|p| ids.get(&p).copied().unwrap_or(p));
        }
        let top = copies.last_mut().expect("a block has its layer");
        top.name = format!("{} copy", top.name);
        top.locked = false;
        let new_id = top.id;
        let at = *block.end() + 1;
        for (k, l) in copies.into_iter().enumerate() {
            self.layers.insert(at + k, l);
        }
        self.active = Some(new_id);
        self.mask_target = false;
        Some(new_id)
    }

    /// Removes a layer (a group with everything inside it). The last remaining layer can't be
    /// removed. Returns the removed layer itself.
    pub fn remove_layer(&mut self, id: LayerId) -> Option<Layer> {
        let index = self.index_of(id)?;
        let block = self.block(index);
        if block.clone().count() >= self.layers.len() {
            return None;
        }
        let start = *block.start();
        let mut removed: Vec<Layer> = self.layers.drain(block).collect();
        if self.active.is_none_or(|a| self.index_of(a).is_none()) {
            self.active = Some(self.layers[start.saturating_sub(1).min(self.layers.len() - 1)].id);
            self.mask_target = false;
        }
        removed.pop()
    }

    /// Bottom-to-top.
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    pub(crate) fn layers_mut(&mut self) -> &mut [Layer] {
        &mut self.layers
    }

    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn layer_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    pub fn active(&self) -> Option<&Layer> {
        self.active.and_then(|id| self.layer(id))
    }

    pub fn active_id(&self) -> Option<LayerId> {
        self.active
    }

    pub fn set_active(&mut self, id: LayerId) {
        if self.index_of(id).is_some() {
            if self.active != Some(id) {
                self.mask_target = false;
            }
            self.active = Some(id);
        }
    }

    /// Whether edits currently go to the active layer's mask (always, on an adjustment
    /// layer: it has no pixels of its own).
    pub fn editing_mask(&self) -> bool {
        self.active().is_some_and(|l| l.mask.is_some() && (self.mask_target || l.adjustment.is_some()))
    }

    /// What each layer looks like, bottom to top: pixels with mask and effects, and for
    /// adjustment layers the adjusted layers below. Everything that draws layers uses this.
    pub fn rendered_layers(&self) -> Vec<std::borrow::Cow<'_, TileGrid>> {
        let layers: Vec<&Layer> = self.layers.iter().collect();
        render::render_layers(&layers, self.blend_space, self.width, self.height)
    }

    fn index_of(&self, id: LayerId) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_document_has_locked_background() {
        let doc = Document::new("t", 10, 10);
        assert_eq!(doc.layers().len(), 1);
        assert!(doc.active().unwrap().locked);
    }

    #[test]
    fn from_rgba8_fills_background() {
        let px: Vec<u8> = [10, 20, 30, 255].repeat(300 * 200);
        let doc = Document::from_rgba8("t", 300, 200, &px);
        assert_eq!(doc.layers()[0].pixels.pixel(299, 199), [10, 20, 30, 255]);
    }

    #[test]
    fn add_inserts_above_active() {
        let mut doc = Document::new("t", 10, 10);
        let a = doc.add_layer("A");
        let b = doc.add_layer("B");
        doc.set_active(a);
        let c = doc.add_layer("C");
        let order: Vec<_> = doc.layers().iter().map(|l| l.id).collect();
        assert_eq!(order[1..], [a, c, b]);
    }

    #[test]
    fn duplicate_shares_tiles_and_goes_above() {
        let px: Vec<u8> = [9, 9, 9, 255].repeat(300 * 10);
        let mut doc = Document::from_rgba8("t", 300, 10, &px);
        let bg = doc.layers()[0].id;
        let copy = doc.duplicate_layer(bg).unwrap();
        assert_eq!(doc.layers()[1].id, copy);
        assert_eq!(doc.layers()[1].name, "Background copy");
        assert!(!doc.layers()[1].locked);
        let ids = |i: usize| doc.layers()[i].pixels.iter().map(|(_, _, t)| t.id()).collect::<Vec<_>>();
        assert_eq!(ids(0), ids(1));
    }

    #[test]
    fn move_layer_clamps_at_ends() {
        let mut doc = Document::new("t", 10, 10);
        let a = doc.add_layer("A");
        assert!(!doc.move_layer(a, 1));
        assert!(doc.move_layer(a, -1));
        assert_eq!(doc.layers()[0].id, a);
        assert!(!doc.move_layer(a, -1));
    }

    #[test]
    fn remove_keeps_last_layer_and_moves_active_down() {
        let mut doc = Document::new("t", 10, 10);
        let bg = doc.layers()[0].id;
        let a = doc.add_layer("A");
        assert!(doc.remove_layer(a).is_some());
        assert_eq!(doc.active_id(), Some(bg));
        assert!(doc.remove_layer(bg).is_none());
    }
}
