//! CPU compositing: flattening a document for export, thumbnails, and as the reference the GPU
//! renderer is tested against.

use std::sync::LazyLock;

use crate::blend::{self, BlendMode};
use crate::{Document, Layer};

/// The colour space layers are blended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlendSpace {
    /// On sRGB-encoded values, like Photoshop's default. Multiply/Screen etc. look the way
    /// Photoshop users expect.
    #[default]
    Perceptual,
    /// On linear-light values. Physically correct; blurs and light mixing look more natural.
    Linear,
}

impl BlendSpace {
    pub fn key(self) -> &'static str {
        match self {
            Self::Perceptual => "perceptual",
            Self::Linear => "linear",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        [Self::Perceptual, Self::Linear].into_iter().find(|s| s.key() == key)
    }
}

static TO_LINEAR: LazyLock<[f32; 256]> = LazyLock::new(|| {
    std::array::from_fn(|i| {
        let v = i as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    })
});

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// RGBA8 sRGB straight → premultiplied working-space floats.
pub(crate) fn to_working(px: [u8; 4], space: BlendSpace) -> [f32; 4] {
    let a = px[3] as f32 / 255.0;
    let c = |v: u8| match space {
        BlendSpace::Perceptual => v as f32 / 255.0,
        BlendSpace::Linear => TO_LINEAR[v as usize],
    };
    [c(px[0]) * a, c(px[1]) * a, c(px[2]) * a, a]
}

/// Premultiplied working-space floats → RGBA8 sRGB straight.
pub(crate) fn to_srgb8(c: [f32; 4], space: BlendSpace) -> [u8; 4] {
    let a = c[3].clamp(0.0, 1.0);
    if a <= 0.0 {
        return [0; 4];
    }
    let enc = |v: f32| {
        let v = (v / a).clamp(0.0, 1.0);
        let v = match space {
            BlendSpace::Perceptual => v,
            BlendSpace::Linear => linear_to_srgb(v),
        };
        (v * 255.0 + 0.5) as u8
    };
    [enc(c[0]), enc(c[1]), enc(c[2]), (a * 255.0 + 0.5) as u8]
}

/// Seed for a layer's Dissolve noise, so each layer dissolves differently (shared with GPU).
pub fn dissolve_seed(layer_index: usize) -> u32 {
    layer_index as u32 + 1
}

fn layer_source(layer: &Layer, index: usize, x: u32, y: u32, space: BlendSpace) -> Option<[f32; 4]> {
    if !layer.visible || layer.opacity <= 0.0 {
        return None;
    }
    let px = layer.pixels.pixel(x, y);
    if px[3] == 0 {
        return None;
    }
    let mut s = to_working(px, space);
    for c in &mut s {
        *c *= layer.opacity;
    }
    if layer.blend == BlendMode::Dissolve {
        // Coverage becomes all-or-nothing, with probability = alpha.
        let keep = blend::dissolve_noise(x, y, dissolve_seed(index)) < s[3];
        if !keep {
            return None;
        }
        let a = s[3];
        s = [s[0] / a, s[1] / a, s[2] / a, 1.0];
    }
    Some(s)
}

/// The composited colour of one pixel: premultiplied, working space.
pub fn pixel(doc: &Document, x: u32, y: u32) -> [f32; 4] {
    let space = doc.blend_space;
    let mut acc = [0.0f32; 4];
    for (i, layer) in doc.layers().iter().enumerate() {
        if let Some(src) = layer_source(layer, i, x, y, space) {
            acc = blend::composite(layer.blend, acc, src);
        }
    }
    acc
}

/// The composited pixel as RGBA8 sRGB, straight alpha.
pub fn pixel_srgb8(doc: &Document, x: u32, y: u32) -> [u8; 4] {
    to_srgb8(pixel(doc, x, y), doc.blend_space)
}

/// Flattens all visible layers into tightly packed RGBA8 sRGB (straight alpha). Keeps
/// transparency. Walks every pixel on the CPU (rows in parallel); call it off the UI thread.
pub fn flatten(doc: &Document) -> Vec<u8> {
    use rayon::prelude::*;
    let w = doc.width as usize;
    let mut out = vec![0u8; w * doc.height as usize * 4];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for (x, px) in row.chunks_exact_mut(4).enumerate() {
            px.copy_from_slice(&pixel_srgb8(doc, x as u32, y as u32));
        }
    });
    out
}

/// A small preview fitting `max`×`max`, nearest-sampled. Returns (width, height, RGBA8).
/// With `layer`, only that layer (ignoring visibility/opacity, like a layers panel thumbnail).
pub fn thumbnail(doc: &Document, max: u32, layer: Option<&Layer>) -> (u32, u32, Vec<u8>) {
    let scale = (max as f32 / doc.width as f32).min(max as f32 / doc.height as f32).min(1.0);
    let tw = ((doc.width as f32 * scale).round() as u32).max(1);
    let th = ((doc.height as f32 * scale).round() as u32).max(1);
    let mut out = Vec::with_capacity((tw * th * 4) as usize);
    for ty in 0..th {
        for tx in 0..tw {
            let x = ((tx as f32 + 0.5) / scale) as u32;
            let y = ((ty as f32 + 0.5) / scale) as u32;
            let (x, y) = (x.min(doc.width - 1), y.min(doc.height - 1));
            out.extend_from_slice(&match layer {
                Some(l) => l.pixels.pixel(x, y),
                None => pixel_srgb8(doc, x, y),
            });
        }
    }
    (tw, th, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
        px.repeat((w * h) as usize)
    }

    #[test]
    fn single_layer_flattens_to_itself() {
        let doc = Document::from_rgba8("t", 3, 2, &solid(3, 2, [10, 200, 30, 255]));
        assert_eq!(flatten(&doc), solid(3, 2, [10, 200, 30, 255]));
    }

    #[test]
    fn hidden_and_transparent_layers_dont_contribute() {
        let mut doc = Document::from_rgba8("t", 2, 2, &solid(2, 2, [100, 100, 100, 255]));
        let top = doc.add_layer("top");
        doc.layer_mut(top).unwrap().pixels = crate::TileGrid::from_rgba8(2, 2, &solid(2, 2, [255, 0, 0, 255]));
        doc.layer_mut(top).unwrap().visible = false;
        assert_eq!(pixel_srgb8(&doc, 0, 0), [100, 100, 100, 255]);
    }

    #[test]
    fn multiply_perceptual_matches_photoshop_maths() {
        // 50% grey × 50% grey in sRGB values = 25% → 64.
        let mut doc = Document::from_rgba8("t", 1, 1, &[128, 128, 128, 255]);
        let top = doc.add_layer("top");
        let l = doc.layer_mut(top).unwrap();
        l.pixels = crate::TileGrid::from_rgba8(1, 1, &[128, 128, 128, 255]);
        l.blend = BlendMode::Multiply;
        let [r, ..] = pixel_srgb8(&doc, 0, 0);
        assert!((63..=65).contains(&r), "{r}");

        doc.blend_space = BlendSpace::Linear;
        let [r, ..] = pixel_srgb8(&doc, 0, 0);
        // Linear: 0.2158² = 0.0466 linear, which encodes back to sRGB 61.
        assert_eq!(r, 61);
    }

    #[test]
    fn opacity_scales_contribution() {
        let mut doc = Document::from_rgba8("t", 1, 1, &[0, 0, 0, 255]);
        let top = doc.add_layer("top");
        let l = doc.layer_mut(top).unwrap();
        l.pixels = crate::TileGrid::from_rgba8(1, 1, &[255, 255, 255, 255]);
        l.opacity = 0.5;
        let [r, ..] = pixel_srgb8(&doc, 0, 0);
        assert!((127..=128).contains(&r), "{r}");
    }

    #[test]
    fn thumbnail_fits_box() {
        let doc = Document::from_rgba8("t", 400, 100, &solid(400, 100, [1, 2, 3, 255]));
        let (w, h, px) = thumbnail(&doc, 48, None);
        assert_eq!((w, h), (48, 12));
        assert_eq!(&px[..4], &[1, 2, 3, 255]);
    }
}
