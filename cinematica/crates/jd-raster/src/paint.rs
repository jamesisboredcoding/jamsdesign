//! The brush engine: stamps round dabs along a stroke and writes them into a layer's tiles.
//!
//! Model (same as Photoshop):
//! - Each dab adds coverage to a per-stroke *mask*: `mask += dab * (1 - mask)`. Flow scales
//!   each dab, so low flow builds up gradually as you go over the same spot.
//! - Opacity caps the whole stroke: the layer gets `color` at `mask * opacity`, so crossing
//!   your own stroke never goes darker than the opacity.
//! - Every update recomputes touched pixels from the layer as it was *before* the stroke plus
//!   the mask, so nothing accumulates rounding error and the blend mode is applied once.

use std::collections::HashMap;

use crate::blend::{self, BlendMode};
use crate::composite::{to_srgb8, to_working};
use crate::selection::Selection;
use crate::{BlendSpace, Document, LayerId, TILE_SIZE, TileGrid};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrushParams {
    /// Diameter in document pixels at full pressure.
    pub size: f32,
    /// 0 = very soft, 1 = hard edge.
    pub hardness: f32,
    /// Max coverage of one stroke, 0..1.
    pub opacity: f32,
    /// Coverage each dab adds, 0..1.
    pub flow: f32,
    /// Distance between dabs, as a fraction of the diameter.
    pub spacing: f32,
    pub pressure_size: bool,
    pub pressure_opacity: bool,
    /// sRGB colour.
    pub color: [u8; 3],
    pub mode: BlendMode,
    /// Remove pixels instead of painting.
    pub erase: bool,
}

impl Default for BrushParams {
    fn default() -> Self {
        Self {
            size: 30.0,
            hardness: 0.8,
            opacity: 1.0,
            flow: 1.0,
            spacing: 0.1,
            pressure_size: true,
            pressure_opacity: false,
            color: [0, 0, 0],
            mode: BlendMode::Normal,
            erase: false,
        }
    }
}

/// A rectangle of document pixels touched by an update: x0, y0, x1, y1 (exclusive).
pub type Region = [u32; 4];

pub struct Stroke {
    params: BrushParams,
    layer: LayerId,
    space: BlendSpace,
    width: u32,
    height: u32,
    /// The layer's pixels when the stroke started.
    original: TileGrid,
    /// Painting is limited to the selection, if there is one.
    clip: Option<Selection>,
    /// Coverage per touched tile, 0..1.
    mask: HashMap<(u32, u32), Box<[f32]>>,
    last: Option<(f32, f32, f32)>,
    /// Distance travelled since the last dab.
    carry: f32,
}

impl Stroke {
    /// Starts a stroke on the active layer. `None` if there is no active layer.
    pub fn begin(doc: &Document, params: BrushParams) -> Option<Self> {
        let layer = doc.active()?;
        Some(Self {
            params,
            layer: layer.id,
            space: doc.blend_space,
            width: doc.width,
            height: doc.height,
            original: layer.pixels.clone(),
            clip: doc.selection.clone(),
            mask: HashMap::new(),
            last: None,
            carry: 0.0,
        })
    }

    pub fn layer(&self) -> LayerId {
        self.layer
    }

    fn radius(&self, pressure: f32) -> f32 {
        let p = if self.params.pressure_size { pressure } else { 1.0 };
        (self.params.size * p / 2.0).max(0.5)
    }

    fn dab_alpha(&self, pressure: f32) -> f32 {
        let p = if self.params.pressure_opacity { pressure } else { 1.0 };
        self.params.flow * p
    }

    /// Moves the brush to (x, y) in document pixels with `pressure` 0..1, placing dabs along
    /// the way, and writes the result into `doc`. Returns the touched region, if any.
    pub fn add_point(&mut self, doc: &mut Document, x: f32, y: f32, pressure: f32) -> Option<Region> {
        let pressure = pressure.clamp(0.0, 1.0);
        let mut region: Option<Region> = None;
        let mut grow = |r: Region| {
            region = Some(match region {
                None => r,
                Some(a) => [a[0].min(r[0]), a[1].min(r[1]), a[2].max(r[2]), a[3].max(r[3])],
            });
        };

        match self.last {
            None => {
                if let Some(r) = self.dab(x, y, pressure) {
                    grow(r);
                }
            }
            Some((lx, ly, lp)) => {
                let (dx, dy) = (x - lx, y - ly);
                let dist = (dx * dx + dy * dy).sqrt();
                let mut t = 0.0;
                // Walk the segment, dropping a dab every `step` pixels.
                loop {
                    let p = lp + (pressure - lp) * (t / dist.max(1e-6));
                    let step = (self.radius(p) * 2.0 * self.params.spacing).max(0.5);
                    let next = step - self.carry;
                    if t + next > dist {
                        self.carry += dist - t;
                        break;
                    }
                    t += next;
                    self.carry = 0.0;
                    let f = t / dist;
                    if let Some(r) = self.dab(lx + dx * f, ly + dy * f, lp + (pressure - lp) * f) {
                        grow(r);
                    }
                }
            }
        }
        self.last = Some((x, y, pressure));
        if let Some(r) = region {
            self.write(doc, r);
        }
        region
    }

    /// Adds one dab to the mask. Returns its pixel bounds clipped to the document.
    fn dab(&mut self, cx: f32, cy: f32, pressure: f32) -> Option<Region> {
        let r = self.radius(pressure);
        let alpha = self.dab_alpha(pressure);
        if alpha <= 0.0 {
            return None;
        }
        let x0 = (cx - r).floor().max(0.0) as u32;
        let y0 = (cy - r).floor().max(0.0) as u32;
        let x1 = ((cx + r).ceil() as i64).clamp(0, self.width as i64) as u32;
        let y1 = ((cy + r).ceil() as i64).clamp(0, self.height as i64) as u32;
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        let inner = r * self.params.hardness.clamp(0.0, 1.0);
        let ts = TILE_SIZE as usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let (px, py) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let d = (px * px + py * py).sqrt();
                if d >= r + 0.5 {
                    continue;
                }
                // Soft falloff between `inner` and `r`, plus 1px anti-aliasing at the rim.
                let soft = if d <= inner || r <= inner {
                    1.0
                } else {
                    let t = ((r - d) / (r - inner)).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                };
                let a = alpha * soft * (r - d + 0.5).clamp(0.0, 1.0);
                if a <= 0.0 {
                    continue;
                }
                let key = (x / TILE_SIZE, y / TILE_SIZE);
                let m = self.mask.entry(key).or_insert_with(|| vec![0.0; ts * ts].into_boxed_slice());
                let i = (y % TILE_SIZE) as usize * ts + (x % TILE_SIZE) as usize;
                m[i] += a * (1.0 - m[i]);
            }
        }
        Some([x0, y0, x1, y1])
    }

    /// Recomputes `region` of the stroke's layer from the original pixels and the mask.
    fn write(&self, doc: &mut Document, region: Region) {
        let Some(layer) = doc.layer_mut(self.layer) else { return };
        let p = &self.params;
        let color = {
            let c = to_working([p.color[0], p.color[1], p.color[2], 255], self.space);
            [c[0], c[1], c[2]]
        };
        let [x0, y0, x1, y1] = region;
        for ty in y0 / TILE_SIZE..=(y1 - 1) / TILE_SIZE {
            for tx in x0 / TILE_SIZE..=(x1 - 1) / TILE_SIZE {
                let Some(mask) = self.mask.get(&(tx, ty)) else { continue };
                let original = self.original.tile(tx, ty).cloned();
                let Some(tile) = layer.pixels.tile_mut(tx, ty) else { continue };
                let data = tile.data_mut();
                let lx0 = x0.max(tx * TILE_SIZE) - tx * TILE_SIZE;
                let ly0 = y0.max(ty * TILE_SIZE) - ty * TILE_SIZE;
                let lx1 = x1.min((tx + 1) * TILE_SIZE) - tx * TILE_SIZE;
                let ly1 = y1.min((ty + 1) * TILE_SIZE) - ty * TILE_SIZE;
                for ly in ly0..ly1 {
                    for lx in lx0..lx1 {
                        let i = (ly * TILE_SIZE + lx) as usize;
                        let o: [u8; 4] = match &original {
                            Some(t) => t.data()[i * 4..i * 4 + 4].try_into().expect("4 bytes"),
                            None => [0; 4],
                        };
                        let mut m = mask[i] * p.opacity;
                        if let Some(clip) = &self.clip {
                            m *= clip.get(tx * TILE_SIZE + lx, ty * TILE_SIZE + ly) as f32 / 255.0;
                        }
                        let out = if m <= 0.0 {
                            o
                        } else if p.erase {
                            let a = (o[3] as f32 * (1.0 - m) + 0.5) as u8;
                            if a == 0 { [0; 4] } else { [o[0], o[1], o[2], a] }
                        } else {
                            let src = [color[0] * m, color[1] * m, color[2] * m, m];
                            to_srgb8(blend::composite(p.mode, to_working(o, self.space), src), self.space)
                        };
                        data[i * 4..i * 4 + 4].copy_from_slice(&out);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn white_doc(w: u32, h: u32) -> Document {
        Document::from_rgba8("t", w, h, &[255, 255, 255, 255].repeat((w * h) as usize))
    }

    fn params() -> BrushParams {
        BrushParams { size: 10.0, hardness: 1.0, color: [0, 0, 0], ..Default::default() }
    }

    #[test]
    fn single_dab_paints_centre_and_leaves_far_pixels() {
        let mut doc = white_doc(64, 64);
        let mut s = Stroke::begin(&doc, params()).unwrap();
        let region = s.add_point(&mut doc, 32.0, 32.0, 1.0).unwrap();
        assert!(region[0] <= 27 && region[2] >= 37);
        let px = |x, y| doc.layers()[0].pixels.pixel(x, y);
        assert_eq!(px(32, 32), [0, 0, 0, 255]);
        assert_eq!(px(5, 5), [255, 255, 255, 255]);
    }

    #[test]
    fn opacity_caps_overlapping_stroke() {
        let mut doc = white_doc(64, 64);
        let mut s = Stroke::begin(&doc, BrushParams { opacity: 0.5, ..params() }).unwrap();
        for _ in 0..3 {
            s.add_point(&mut doc, 10.0, 32.0, 1.0);
            s.add_point(&mut doc, 50.0, 32.0, 1.0);
        }
        // Black at 50% over white in perceptual space: 127/128, never darker.
        let [r, ..] = doc.layers()[0].pixels.pixel(30, 32);
        assert!((126..=129).contains(&r), "{r}");
    }

    #[test]
    fn low_flow_builds_up() {
        let mut doc = white_doc(64, 64);
        let mut s = Stroke::begin(&doc, BrushParams { flow: 0.2, ..params() }).unwrap();
        s.add_point(&mut doc, 32.0, 32.0, 1.0);
        let first = doc.layers()[0].pixels.pixel(32, 32)[0];
        for _ in 0..10 {
            s.add_point(&mut doc, 32.0, 32.0, 1.0);
            s.add_point(&mut doc, 32.5, 32.0, 1.0);
        }
        let later = doc.layers()[0].pixels.pixel(32, 32)[0];
        assert!(first > 180 && later < first, "{first} → {later}");
    }

    #[test]
    fn spacing_makes_a_continuous_line() {
        let mut doc = white_doc(128, 32);
        let mut s = Stroke::begin(&doc, params()).unwrap();
        s.add_point(&mut doc, 10.0, 16.0, 1.0);
        s.add_point(&mut doc, 110.0, 16.0, 1.0);
        for x in 10..110 {
            assert_eq!(doc.layers()[0].pixels.pixel(x, 16)[0], 0, "gap at {x}");
        }
    }

    #[test]
    fn pressure_scales_size() {
        let mut doc = white_doc(64, 64);
        let mut s = Stroke::begin(&doc, BrushParams { size: 20.0, ..params() }).unwrap();
        s.add_point(&mut doc, 32.0, 32.0, 0.25);
        let px = |x| doc.layers()[0].pixels.pixel(x, 32)[0];
        assert_eq!(px(32), 0);
        assert_eq!(px(32 + 6), 255, "radius should be 2.5 at 25% pressure");
    }

    #[test]
    fn eraser_removes_alpha_and_paint_doesnt_touch_original_tiles() {
        let mut doc = white_doc(64, 64);
        let before = doc.clone();
        let mut s = Stroke::begin(&doc, BrushParams { erase: true, ..params() }).unwrap();
        s.add_point(&mut doc, 32.0, 32.0, 1.0);
        assert_eq!(doc.layers()[0].pixels.pixel(32, 32)[3], 0);
        // Copy-on-write: the snapshot (what undo restores) is untouched.
        assert_eq!(before.layers()[0].pixels.pixel(32, 32), [255, 255, 255, 255]);
    }

    #[test]
    fn selection_clips_painting() {
        let mut doc = white_doc(64, 64);
        doc.selection = Some(Selection::rect(64, 64, 0.0, 0.0, 32.0, 64.0));
        let mut s = Stroke::begin(&doc, BrushParams { size: 20.0, ..params() }).unwrap();
        s.add_point(&mut doc, 32.0, 32.0, 1.0);
        assert_eq!(doc.layers()[0].pixels.pixel(28, 32)[0], 0);
        assert_eq!(doc.layers()[0].pixels.pixel(36, 32)[0], 255);
    }

    #[test]
    fn painting_on_an_empty_layer_creates_tiles() {
        let mut doc = white_doc(600, 300);
        doc.add_layer("empty");
        let mut s = Stroke::begin(&doc, BrushParams { color: [255, 0, 0], ..params() }).unwrap();
        s.add_point(&mut doc, 300.0, 100.0, 1.0);
        let top = &doc.layers()[1];
        assert_eq!(top.pixels.pixel(300, 100), [255, 0, 0, 255]);
        assert_eq!(top.pixels.iter().count(), 1);
    }
}
