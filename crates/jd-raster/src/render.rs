//! What each layer looks like in a stack, bottom to top. Most layers look like their own pixels
//! (plus mask and effects, see [`Layer::rendered`]); an adjustment layer (Photoshop's
//! Levels / Curves / Hue/Saturation... layers) looks like *the composite of everything below
//! it, adjusted*, cut by its mask. Drawn with its own blend mode and opacity, that is exactly
//! Photoshop's adjustment layer, so the GPU and CPU compositors need nothing special.
//!
//! Adjusted tiles are cached by what produced them (the tiles below, their blending, the
//! adjustment, the mask tile), so painting under an adjustment layer recomputes only the
//! tiles the brush touched.

use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, LazyLock, Mutex};

use rayon::prelude::*;

use crate::adjust::Adjustment;
use crate::composite::{BlendSpace, layer_source, to_srgb8};
use crate::{Layer, TILE_SIZE, Tile, TileGrid, blend};

/// Each layer's look, bottom to top (see the module docs).
pub fn render_layers<'a>(layers: &[&'a Layer], space: BlendSpace, width: u32, height: u32) -> Vec<Cow<'a, TileGrid>> {
    let mut out: Vec<Cow<'a, TileGrid>> = Vec::with_capacity(layers.len());
    for (i, layer) in layers.iter().enumerate() {
        let grid = match &layer.adjustment {
            Some(adj) => Cow::Owned(adjusted_below(&layers[..i], &out, layer, adj, space, width, height)),
            None => layer.rendered(),
        };
        out.push(grid);
    }
    out
}

fn fnv(h: &mut u64, v: u64) {
    *h = (*h ^ v).wrapping_mul(0x0100_0000_01b3);
}

/// The composite of `below` (with their looks `grids`), adjusted and masked, on the canvas.
fn adjusted_below(
    below: &[&Layer],
    grids: &[Cow<'_, TileGrid>],
    layer: &Layer,
    adj: &Adjustment,
    space: BlendSpace,
    width: u32,
    height: u32,
) -> TileGrid {
    let mut out = TileGrid::empty(width, height);
    // What decides the composite below a tile, besides which tiles are there.
    let mut base: u64 = 0xcbf2_9ce4_8422_2325;
    fnv(&mut base, space as u64);
    fnv(&mut base, below.len() as u64);
    for l in below {
        fnv(&mut base, l.visible as u64 | (l.opacity.to_bits() as u64) << 1 | (l.blend as u64) << 40);
    }
    // ...and what decides the adjusted tile on top of that.
    let mut extra: u64 = 0x9e37_79b9_7f4a_7c15;
    for b in format!("{adj:?}").bytes() {
        fnv(&mut extra, b as u64);
    }
    let mask = layer.mask.as_ref().filter(|m| m.enabled);
    if let Some(m) = mask {
        fnv(&mut extra, 0x100 | m.default as u64);
    }

    let mut todo = Vec::new();
    {
        let cache = CACHE.lock().ok();
        for ty in 0..out.rows() {
            for tx in 0..out.cols() {
                let mut key = base;
                let mut any = false;
                for (l, g) in below.iter().zip(grids) {
                    let t = g.tile(tx, ty).filter(|_| l.visible && l.opacity > 0.0);
                    any |= t.is_some();
                    fnv(&mut key, t.map_or(0, |t| t.id().raw()));
                }
                if !any {
                    continue;
                }
                let below_key = key;
                fnv(&mut key, extra);
                if let Some(m) = mask {
                    fnv(&mut key, m.pixels.tile(tx, ty).map_or(0, |t| t.id().raw()));
                }
                match cache.as_ref().and_then(|c| c.tiles.get(&key)) {
                    Some(hit) => out.set_tile(tx, ty, hit.clone()),
                    None => todo.push((tx, ty, key, below_key)),
                }
            }
        }
    }
    let f = adj.pixel_fn();
    let done: Vec<_> = todo
        .into_par_iter()
        .map(|(tx, ty, key, below_key)| {
            let ts = TILE_SIZE as usize;
            // The composite below (cached too: changing only the adjustment, e.g. dragging a
            // slider, just re-applies it).
            let cached = BELOW.lock().ok().and_then(|c| c.tiles.get(&below_key).cloned()).flatten();
            let composite = cached.unwrap_or_else(|| {
                let mut data = vec![0u8; ts * ts * 4];
                for ly in 0..TILE_SIZE {
                    for lx in 0..TILE_SIZE {
                        let (x, y) = (tx * TILE_SIZE + lx, ty * TILE_SIZE + ly);
                        if x >= width || y >= height {
                            continue;
                        }
                        let mut acc = [0.0f32; 4];
                        for (i, (l, g)) in below.iter().zip(grids).enumerate() {
                            if let Some(src) = layer_source(l, g, i, x, y, space) {
                                acc = blend::composite(l.blend, acc, src);
                            }
                        }
                        let i = (ly as usize * ts + lx as usize) * 4;
                        data[i..i + 4].copy_from_slice(&to_srgb8(acc, space));
                    }
                }
                let tile = Arc::new(Tile::new(data.into_boxed_slice()));
                if let Ok(mut c) = BELOW.lock() {
                    c.insert(below_key, Some(tile.clone()));
                }
                tile
            });
            let src = composite.data();
            let mut data = vec![0u8; ts * ts * 4];
            let mut any = false;
            for ly in 0..TILE_SIZE {
                for lx in 0..TILE_SIZE {
                    let (x, y) = (tx * TILE_SIZE + lx, ty * TILE_SIZE + ly);
                    if x >= width || y >= height {
                        continue;
                    }
                    let i = (ly as usize * ts + lx as usize) * 4;
                    let c: [u8; 4] = src[i..i + 4].try_into().expect("4 bytes");
                    if c[3] == 0 {
                        continue;
                    }
                    let mut px = f(c);
                    if let Some(m) = mask {
                        px[3] = ((px[3] as u32 * m.value_at(x as i64, y as i64) as u32 + 127) / 255) as u8;
                    }
                    if px[3] != 0 {
                        any = true;
                        data[i..i + 4].copy_from_slice(&px);
                    }
                }
            }
            (tx, ty, key, any.then(|| Arc::new(Tile::new(data.into_boxed_slice()))))
        })
        .collect();
    let mut cache = CACHE.lock().ok();
    for (tx, ty, key, tile) in done {
        out.set_tile(tx, ty, tile.clone());
        if let Some(c) = cache.as_mut() {
            c.insert(key, tile);
        }
    }
    out
}

struct Cache {
    tiles: HashMap<u64, Option<Arc<Tile>>>,
    order: VecDeque<u64>,
}

impl Cache {
    fn new() -> Self {
        Self { tiles: HashMap::new(), order: VecDeque::new() }
    }

    fn insert(&mut self, key: u64, tile: Option<Arc<Tile>>) {
        // About 256 MB of tiles at most per cache; the oldest go first.
        if self.order.len() >= 1024
            && let Some(old) = self.order.pop_front()
        {
            self.tiles.remove(&old);
        }
        if self.tiles.insert(key, tile).is_none() {
            self.order.push_back(key);
        }
    }
}

/// Adjusted tiles.
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(Cache::new()));
/// Composites of the layers below an adjustment layer, before adjusting.
static BELOW: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(Cache::new()));

#[cfg(test)]
mod tests {
    use crate::adjust::Adjustment;
    use crate::mask::Mask;
    use crate::{Document, composite};

    fn doc() -> Document {
        let mut d = Document::from_rgba8("t", 300, 10, &[200, 100, 50, 255].repeat(3000));
        let id = d.add_layer("Invert");
        let l = d.layer_mut(id).unwrap();
        l.adjustment = Some(Adjustment::Invert);
        l.mask = Some(Mask::new(300, 10, 255));
        d
    }

    #[test]
    fn adjusts_what_is_below_through_its_mask_and_opacity() {
        let mut d = doc();
        assert_eq!(composite::pixel_srgb8(&d, 5, 5), [55, 155, 205, 255]);
        let id = d.active_id().unwrap();
        d.layer_mut(id).unwrap().mask.as_mut().unwrap().pixels.set_pixel(5, 5, [0, 0, 0, 255]);
        assert_eq!(composite::pixel_srgb8(&d, 5, 5), [200, 100, 50, 255], "masked out");
        d.layer_mut(id).unwrap().opacity = 0.5;
        let p = composite::pixel_srgb8(&d, 6, 5);
        assert!((p[0] as i32 - 128).abs() <= 1, "{p:?}");
        d.layer_mut(id).unwrap().visible = false;
        assert_eq!(composite::pixel_srgb8(&d, 6, 5), [200, 100, 50, 255]);
    }

    #[test]
    fn layers_above_are_not_adjusted_and_tiles_are_reused() {
        let mut d = doc();
        let top = d.add_layer("top");
        d.layer_mut(top).unwrap().pixels.set_pixel(1, 1, [10, 20, 30, 255]);
        assert_eq!(composite::pixel_srgb8(&d, 1, 1), [10, 20, 30, 255]);
        let ids = |d: &Document| d.rendered_layers()[1].iter().map(|(_, _, t)| t.id()).collect::<Vec<_>>();
        assert_eq!(ids(&d), ids(&d), "cached");
        // Painting below changes only that tile.
        let before = ids(&d);
        let bg = d.layers()[0].id;
        d.layer_mut(bg).unwrap().pixels.set_pixel(290, 1, [0, 0, 0, 255]);
        let after = ids(&d);
        assert_eq!(before[0], after[0]);
        assert_ne!(before[1], after[1]);
    }
}
