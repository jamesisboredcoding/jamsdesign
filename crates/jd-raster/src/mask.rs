//! Layer masks (Photoshop's pixel masks): white shows the layer, black hides it, greys are
//! partly transparent. Non-destructive: the layer's pixels never change; [`Layer::rendered`]
//! draws them through the mask.
//!
//! A mask is stored as an ordinary [`TileGrid`] of opaque greys, so everything that edits
//! pixels (brushes, fills, filters, gradients, transforms, saving) works on it unchanged.
//! Where it has no tile, or a transparent pixel (moved off the canvas, erased), it reads as
//! its `default`: white for "Reveal All" masks, black for "Hide All".
//!
//! [`Layer::rendered`]: crate::Layer::rendered

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, LazyLock, Mutex};

use rayon::prelude::*;

use crate::selection::Selection;
use crate::{TILE_SIZE, Tile, TileGrid};

#[derive(Debug, Clone)]
pub struct Mask {
    /// Greys (only the red channel is read), blended over `default` by their alpha.
    pub pixels: TileGrid,
    /// The value wherever the grid has nothing: 255 (reveal) or 0 (hide).
    pub default: u8,
    /// Shift+click on the mask thumbnail switches it off without deleting it.
    pub enabled: bool,
}

/// One mask pixel's value from its stored grey and the mask's default.
#[inline]
fn value(px: &[u8], default: u8) -> u8 {
    let a = px[3] as u32;
    ((px[0] as u32 * a + default as u32 * (255 - a) + 127) / 255) as u8
}

/// A tile of one opaque grey.
fn solid(v: u8) -> Arc<Tile> {
    Arc::new(Tile::new([v, v, v, 255].repeat((TILE_SIZE * TILE_SIZE) as usize).into_boxed_slice()))
}

impl Mask {
    /// Layer > Layer Mask > Reveal All (255) or Hide All (0).
    pub fn new(width: u32, height: u32, v: u8) -> Self {
        let mut pixels = TileGrid::empty(width, height);
        let tile = solid(v);
        for ty in 0..pixels.rows() {
            for tx in 0..pixels.cols() {
                pixels.set_tile(tx, ty, Some(tile.clone()));
            }
        }
        Self { pixels, default: v, enabled: true }
    }

    /// Reveal Selection (or Hide Selection when `hide`): the selection's coverage becomes the
    /// mask, and everything outside the canvas follows the unselected part.
    pub fn from_selection(sel: &Selection, hide: bool) -> Self {
        let (w, h) = (sel.width(), sel.height());
        let outside = if hide { 255 } else { 0 };
        let mut mask = Self::new(w, h, outside);
        let ts = TILE_SIZE as usize;
        for ty in 0..mask.pixels.rows() {
            for tx in 0..mask.pixels.cols() {
                let (x0, y0) = (tx * TILE_SIZE, ty * TILE_SIZE);
                let mut data = vec![0u8; ts * ts * 4];
                let mut any = false;
                for ly in 0..TILE_SIZE {
                    for lx in 0..TILE_SIZE {
                        let c = sel.get(x0 + lx, y0 + ly);
                        let v = if hide { 255 - c } else { c };
                        any |= v != outside;
                        let i = (ly as usize * ts + lx as usize) * 4;
                        data[i..i + 4].copy_from_slice(&[v, v, v, 255]);
                    }
                }
                if any {
                    mask.pixels.set_tile(tx, ty, Some(Arc::new(Tile::new(data.into_boxed_slice()))));
                }
            }
        }
        mask
    }

    /// The mask's value anywhere (on or off the canvas).
    pub fn value_at(&self, x: i64, y: i64) -> u8 {
        value(&self.pixels.pixel_at(x, y), self.default)
    }

    /// The mask as a selection (Ctrl+click on its thumbnail).
    pub fn to_selection(&self) -> Selection {
        let (w, h) = (self.pixels.width(), self.pixels.height());
        Selection::from_fn(w, h, [0, 0, w, h], |x, y| self.value_at(x as i64, y as i64))
    }

    /// A small greyscale preview fitting `max`×`max` (nearest-sampled), as RGBA8.
    pub fn thumbnail(&self, max: u32) -> (u32, u32, Vec<u8>) {
        let (w, h) = (self.pixels.width().max(1), self.pixels.height().max(1));
        let scale = (max as f32 / w as f32).min(max as f32 / h as f32).min(1.0);
        let tw = ((w as f32 * scale).round() as u32).max(1);
        let th = ((h as f32 * scale).round() as u32).max(1);
        let mut out = Vec::with_capacity((tw * th * 4) as usize);
        for ty in 0..th {
            for tx in 0..tw {
                let x = (((tx as f32 + 0.5) / scale) as u32).min(w - 1);
                let y = (((ty as f32 + 0.5) / scale) as u32).min(h - 1);
                let v = self.value_at(x as i64, y as i64);
                out.extend_from_slice(&[v, v, v, 255]);
            }
        }
        (tw, th, out)
    }

    /// Cache key part for the mask's contents.
    fn tile_key(&self, tx: i32, ty: i32) -> Option<u64> {
        self.pixels.tile_at(tx, ty).map(|t| t.id().raw())
    }
}

/// `pixels` with each pixel's alpha scaled by the mask.
fn mask_tile(px: &Tile, m: Option<&Tile>, default: u8) -> Option<Arc<Tile>> {
    let mut data = px.data().to_vec();
    let mut any = false;
    for (i, p) in data.chunks_exact_mut(4).enumerate() {
        let v = match m {
            Some(m) => value(&m.data()[i * 4..i * 4 + 4], default),
            None => default,
        } as u32;
        p[3] = ((p[3] as u32 * v + 127) / 255) as u8;
        any |= p[3] != 0;
    }
    any.then(|| Arc::new(Tile::new(data.into_boxed_slice())))
}

type Key = (u64, Option<u64>, u8);

struct Cache {
    tiles: HashMap<Key, Option<Arc<Tile>>>,
    order: VecDeque<Key>,
}

/// Masked tiles by (pixel tile, mask tile, default): painting on a mask redoes only the tiles
/// it touched, and unchanged layers give the GPU the same tiles every frame.
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(Cache { tiles: HashMap::new(), order: VecDeque::new() }));

/// About 256 MB of masked tiles at most.
const CACHE_TILES: usize = 1024;

/// `pixels` drawn through `mask` (cached per tile).
pub fn apply(pixels: &TileGrid, mask: &Mask) -> TileGrid {
    let mut out = TileGrid::empty(pixels.width(), pixels.height());
    let mut todo = Vec::new();
    {
        let cache = CACHE.lock().ok();
        for (tx, ty, t) in pixels.iter_all() {
            let mk = mask.tile_key(tx, ty);
            if mk.is_none() {
                // No mask tile: the whole tile is the default.
                if mask.default == 255 {
                    out.set_tile_at(tx, ty, Some(t.clone()));
                    continue;
                }
                if mask.default == 0 {
                    continue;
                }
            }
            let key = (t.id().raw(), mk, mask.default);
            match cache.as_ref().and_then(|c| c.tiles.get(&key)) {
                Some(hit) => out.set_tile_at(tx, ty, hit.clone()),
                None => todo.push((tx, ty, key, t.clone())),
            }
        }
    }
    let done: Vec<_> = todo
        .into_par_iter()
        .map(|(tx, ty, key, t)| {
            let m = mask.pixels.tile_at(tx, ty).map(|m| &**m);
            (tx, ty, key, mask_tile(&t, m, mask.default))
        })
        .collect();
    let mut cache = CACHE.lock().ok();
    for (tx, ty, key, tile) in done {
        out.set_tile_at(tx, ty, tile.clone());
        if let Some(c) = cache.as_mut() {
            if c.order.len() >= CACHE_TILES
                && let Some(old) = c.order.pop_front()
            {
                c.tiles.remove(&old);
            }
            c.tiles.insert(key, tile);
            c.order.push_back(key);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer() -> TileGrid {
        TileGrid::from_rgba8(300, 300, &[200, 100, 50, 255].repeat(300 * 300))
    }

    #[test]
    fn reveal_all_shares_the_layer_tiles() {
        let px = layer();
        let out = apply(&px, &Mask::new(300, 300, 255));
        assert_eq!(out.pixel(10, 10), [200, 100, 50, 255]);
        let hidden = apply(&px, &Mask::new(300, 300, 0));
        assert_eq!(hidden.pixel(10, 10)[3], 0);
        assert_eq!(hidden.iter_all().count(), 0, "hidden tiles are dropped");
    }

    #[test]
    fn painted_greys_set_opacity_and_cache_hits() {
        let px = layer();
        let mut m = Mask::new(300, 300, 255);
        m.pixels.set_pixel(5, 5, [128, 128, 128, 255]);
        m.pixels.set_pixel(6, 5, [0, 0, 0, 0]); // erased: reads as the default (white)
        let a = apply(&px, &m);
        assert_eq!(a.pixel(5, 5), [200, 100, 50, 128]);
        assert_eq!(a.pixel(6, 5)[3], 255);
        let b = apply(&px, &m);
        let ids = |g: &TileGrid| g.iter_all().map(|(_, _, t)| t.id()).collect::<Vec<_>>();
        assert_eq!(ids(&a), ids(&b), "same tiles from the cache");
    }

    #[test]
    fn selection_round_trip() {
        let mut sel = Selection::empty(300, 300);
        sel.set(20, 30, 255);
        sel.set(21, 30, 100);
        let m = Mask::from_selection(&sel, false);
        assert_eq!((m.value_at(20, 30), m.value_at(21, 30), m.value_at(0, 0)), (255, 100, 0));
        assert_eq!(m.value_at(-500, 40), 0, "off the canvas follows the unselected part");
        let back = m.to_selection();
        assert_eq!((back.get(20, 30), back.get(21, 30), back.get(0, 0)), (255, 100, 0));
        let hide = Mask::from_selection(&sel, true);
        assert_eq!((hide.value_at(20, 30), hide.value_at(0, 0)), (0, 255));
    }
}
