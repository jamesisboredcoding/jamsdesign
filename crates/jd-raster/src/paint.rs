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
    /// What the brush does besides plain painting (clone, heal, dodge, burn).
    pub effect: Effect,
}

/// Brush behaviours other than painting a colour.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Effect {
    #[default]
    Paint,
    /// Clone Stamp: paint the layer's own pixels from (dx, dy) away (source = point + offset).
    Clone { dx: i64, dy: i64 },
    /// Healing Brush: clone, then [`Stroke::finish`] matches the copied area's colour and
    /// brightness to its surroundings (keeping the source's texture).
    Heal { dx: i64, dy: i64 },
    /// Lighten (Dodge) by up to `exposure` (0..1).
    Dodge { exposure: f32 },
    /// Darken (Burn) by up to `exposure` (0..1).
    Burn { exposure: f32 },
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
            effect: Effect::Paint,
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
                        let (x, y) = ((tx * TILE_SIZE + lx) as i64, (ty * TILE_SIZE + ly) as i64);
                        let out = if m <= 0.0 {
                            o
                        } else if p.erase {
                            let a = (o[3] as f32 * (1.0 - m) + 0.5) as u8;
                            if a == 0 { [0; 4] } else { [o[0], o[1], o[2], a] }
                        } else {
                            match p.effect {
                                Effect::Paint => {
                                    let src = [color[0] * m, color[1] * m, color[2] * m, m];
                                    to_srgb8(blend::composite(p.mode, to_working(o, self.space), src), self.space)
                                }
                                Effect::Clone { dx, dy } | Effect::Heal { dx, dy } => {
                                    // Source pixels come from the layer as it was when the stroke began.
                                    let s = self.original.pixel_at(x + dx, y + dy);
                                    let mut src = to_working(s, self.space);
                                    for c in &mut src {
                                        *c *= m;
                                    }
                                    to_srgb8(blend::composite(p.mode, to_working(o, self.space), src), self.space)
                                }
                                Effect::Dodge { exposure } | Effect::Burn { exposure } => {
                                    let k = (exposure * m).clamp(0.0, 1.0);
                                    let dodge = matches!(p.effect, Effect::Dodge { .. });
                                    let f = |v: u8| {
                                        let v = v as f32 / 255.0;
                                        let r = if dodge { v + (1.0 - v) * k * v.sqrt() } else { v * (1.0 - k * (1.0 - v).sqrt()) };
                                        (r * 255.0 + 0.5).clamp(0.0, 255.0) as u8
                                    };
                                    [f(o[0]), f(o[1]), f(o[2]), o[3]]
                                }
                            }
                        };
                        data[i * 4..i * 4 + 4].copy_from_slice(&out);
                    }
                }
            }
        }
    }
}

impl Stroke {
    /// Bounds of everything this stroke touched: x0, y0, x1, y1.
    fn touched(&self) -> Option<Region> {
        let ts = TILE_SIZE;
        self.mask.keys().fold(None, |acc, &(tx, ty)| {
            let r = [tx * ts, ty * ts, ((tx + 1) * ts).min(self.width), ((ty + 1) * ts).min(self.height)];
            Some(match acc {
                None => r,
                Some(a) => [a[0].min(r[0]), a[1].min(r[1]), a[2].max(r[2]), a[3].max(r[3])],
            })
        })
    }

    /// Ends the stroke. For the Healing Brush this blends the cloned pixels into their
    /// surroundings: the result keeps the source's fine detail but takes the destination's
    /// broad colour and brightness (`result = src + blur(dst) - blur(src)`).
    pub fn finish(self, doc: &mut Document) {
        let Effect::Heal { dx, dy } = self.params.effect else { return };
        let Some([x0, y0, x1, y1]) = self.touched() else { return };
        let radius = (self.params.size / 4.0).max(2.0);
        let margin = (radius * 3.0).ceil() as u32 + 1;
        let (w, h) = (self.width, self.height);
        let grow = [x0.saturating_sub(margin), y0.saturating_sub(margin), (x1 + margin).min(w), (y1 + margin).min(h)];
        let area = Selection::rect(w, h, grow[0] as f32, grow[1] as f32, grow[2] as f32, grow[3] as f32);
        let blur = crate::filter::Filter::GaussianBlur { radius };
        // Blurred destination (before the stroke) and blurred, shifted source.
        let dst_low = crate::filter::apply(&self.original, &blur, Some(&area));
        let shifted = crate::transform::place(&self.original, w, h, -dx, -dy);
        let src_low = crate::filter::apply(&shifted, &blur, Some(&area));
        let Some(layer) = doc.layer_mut(self.layer) else { return };
        for y in y0..y1 {
            for x in x0..x1 {
                let Some(mask) = self.mask.get(&(x / TILE_SIZE, y / TILE_SIZE)) else { continue };
                let mut m = mask[((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize] * self.params.opacity;
                if let Some(clip) = &self.clip {
                    m *= clip.get(x, y) as f32 / 255.0;
                }
                if m <= 0.0 {
                    continue;
                }
                let (o, s, dl, sl) = (self.original.pixel(x, y), shifted.pixel(x, y), dst_low.pixel(x, y), src_low.pixel(x, y));
                let mut out = o;
                for c in 0..3 {
                    let healed = (s[c] as f32 + dl[c] as f32 - sl[c] as f32).clamp(0.0, 255.0);
                    out[c] = (o[c] as f32 * (1.0 - m) + healed * m + 0.5) as u8;
                }
                out[3] = (o[3] as f32 * (1.0 - m) + s[3].max(o[3]) as f32 * m + 0.5) as u8;
                layer.pixels.set_pixel(x, y, out);
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
    fn clone_copies_from_the_offset() {
        let mut px = [255u8, 255, 255, 255].repeat(64 * 64);
        for y in 0..64 {
            for x in 0..10 {
                px[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4].copy_from_slice(&[200, 0, 0, 255]);
            }
        }
        let mut doc = Document::from_rgba8("t", 64, 64, &px);
        let p = BrushParams { size: 6.0, hardness: 1.0, effect: Effect::Clone { dx: -40, dy: 0 }, ..params() };
        let mut s = Stroke::begin(&doc, p).unwrap();
        s.add_point(&mut doc, 45.0, 30.0, 1.0);
        assert_eq!(doc.layers()[0].pixels.pixel(45, 30), [200, 0, 0, 255], "red from 40px to the left");
    }

    #[test]
    fn dodge_lightens_and_burn_darkens() {
        for (effect, lighter) in [(Effect::Dodge { exposure: 0.5 }, true), (Effect::Burn { exposure: 0.5 }, false)] {
            let mut doc = Document::from_rgba8("t", 32, 32, &[100, 100, 100, 255].repeat(32 * 32));
            let mut s = Stroke::begin(&doc, BrushParams { size: 8.0, effect, ..params() }).unwrap();
            s.add_point(&mut doc, 16.0, 16.0, 1.0);
            let v = doc.layers()[0].pixels.pixel(16, 16)[0];
            assert_eq!(v > 100, lighter, "{effect:?}: {v}");
        }
    }

    #[test]
    fn heal_takes_destination_tone_with_source_texture() {
        // Left half dark with a bright speck pattern (texture), right half mid grey.
        let mut px = vec![0u8; 80 * 40 * 4];
        for y in 0..40 {
            for x in 0..80 {
                let v = if x < 40 { if (x + y) % 4 == 0 { 60 } else { 20 } } else { 150 };
                px[(y * 80 + x) * 4..(y * 80 + x) * 4 + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let mut doc = Document::from_rgba8("t", 80, 40, &px);
        let p = BrushParams { size: 10.0, hardness: 1.0, effect: Effect::Heal { dx: -40, dy: 0 }, ..params() };
        let mut s = Stroke::begin(&doc, p).unwrap();
        s.add_point(&mut doc, 60.0, 20.0, 1.0);
        let cloned = doc.layers()[0].pixels.pixel(60, 20)[0];
        assert!(cloned < 70, "while painting it's a plain clone ({cloned})");
        s.finish(&mut doc);
        let healed = doc.layers()[0].pixels.pixel(60, 20)[0];
        assert!(healed > 110, "after finishing it matches the grey around it ({healed})");
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
