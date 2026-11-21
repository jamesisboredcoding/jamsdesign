//! The raster document model.
//!
//! Layers hold their pixels in a [`TileGrid`]. This crate must never depend on any UI toolkit.

pub mod adjust;
pub mod blend;
pub mod composite;
pub mod filter;
pub mod gradient;
pub mod heal;
pub mod style;
pub mod image_ops;
mod history;
pub mod paint;
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
    /// 0.0..=1.0
    pub opacity: f32,
    pub blend: BlendMode,
    pub pixels: TileGrid,
    /// Layer effects (drop shadow, stroke, ...), drawn on top of `pixels` when rendering.
    pub style: style::LayerStyle,
}

impl Layer {
    /// What the layer looks like: its pixels plus its effects (cached; the pixels themselves
    /// when it has none). Everything that draws or flattens layers uses this.
    pub fn rendered(&self) -> std::borrow::Cow<'_, TileGrid> {
        if self.style.is_empty() {
            std::borrow::Cow::Borrowed(&self.pixels)
        } else {
            std::borrow::Cow::Owned(style::render_cached(&self.pixels, &self.style))
        }
    }
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
            layers: Vec::new(),
            active: None,
            next_id: 0,
        }
    }

    /// Adds a layer above the active one (or on top) and makes it active.
    pub fn add_layer(&mut self, name: impl Into<String>) -> LayerId {
        let id = LayerId(self.next_id);
        self.next_id += 1;
        let layer = Layer {
            id,
            name: name.into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: BlendMode::Normal,
            pixels: TileGrid::empty(self.width, self.height),
            style: style::LayerStyle::default(),
        };
        let index = self
            .active
            .and_then(|a| self.index_of(a))
            .map_or(self.layers.len(), |i| i + 1);
        self.layers.insert(index, layer);
        self.active = Some(id);
        id
    }

    /// Copies a layer (sharing its tiles) above itself and makes the copy active.
    pub fn duplicate_layer(&mut self, id: LayerId) -> Option<LayerId> {
        let index = self.index_of(id)?;
        let mut copy = self.layers[index].clone();
        copy.id = LayerId(self.next_id);
        self.next_id += 1;
        copy.name = format!("{} copy", copy.name);
        copy.locked = false;
        let new_id = copy.id;
        self.layers.insert(index + 1, copy);
        self.active = Some(new_id);
        Some(new_id)
    }

    /// Moves a layer up (+1, towards the top) or down (-1). Returns whether it moved.
    pub fn move_layer(&mut self, id: LayerId, delta: isize) -> bool {
        let Some(index) = self.index_of(id) else { return false };
        let target = index as isize + delta;
        if target < 0 || target >= self.layers.len() as isize {
            return false;
        }
        let layer = self.layers.remove(index);
        self.layers.insert(target as usize, layer);
        true
    }

    /// Removes a layer. The last remaining layer can't be removed.
    pub fn remove_layer(&mut self, id: LayerId) -> Option<Layer> {
        if self.layers.len() <= 1 {
            return None;
        }
        let index = self.index_of(id)?;
        let removed = self.layers.remove(index);
        if self.active == Some(id) {
            self.active = Some(self.layers[index.saturating_sub(1)].id);
        }
        Some(removed)
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
            self.active = Some(id);
        }
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
