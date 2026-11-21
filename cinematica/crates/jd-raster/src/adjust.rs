//! Image > Adjustments: tone and colour changes applied pixel by pixel.
//!
//! Like Photoshop, adjustments work on sRGB-encoded values. Tone adjustments (levels, curves,
//! brightness/contrast, threshold, posterize, invert) reduce to a 256-entry lookup table; the
//! others are per-pixel colour maths. Work runs tile by tile in parallel; tiles with nothing
//! selected are shared with the original instead of copied.

use std::sync::Arc;

use jd_core::Hsv;
use rayon::prelude::*;

use crate::selection::Selection;
use crate::{TILE_SIZE, Tile, TileGrid};

#[derive(Debug, Clone, PartialEq)]
pub enum Adjustment {
    /// brightness -150..150, contrast -50..100 (Photoshop's ranges).
    BrightnessContrast { brightness: f32, contrast: f32 },
    /// Input black/white points, midtone gamma (0.1..9.99), output black/white points.
    Levels { in_black: u8, in_white: u8, gamma: f32, out_black: u8, out_white: u8 },
    /// Control points (input, output), any order; at least two.
    Curves { points: Vec<(u8, u8)> },
    /// hue -180..180 degrees, saturation -100..100, lightness -100..100.
    HueSaturation { hue: f32, saturation: f32, lightness: f32 },
    Threshold { level: u8 },
    /// 2..255 levels per channel.
    Posterize { levels: u8 },
    Invert,
    Desaturate,
}

impl Adjustment {
    pub fn name(&self) -> &'static str {
        match self {
            Self::BrightnessContrast { .. } => "Brightness/Contrast",
            Self::Levels { .. } => "Levels",
            Self::Curves { .. } => "Curves",
            Self::HueSaturation { .. } => "Hue/Saturation",
            Self::Threshold { .. } => "Threshold",
            Self::Posterize { .. } => "Posterize",
            Self::Invert => "Invert",
            Self::Desaturate => "Desaturate",
        }
    }

    /// The same value for every channel, for tone adjustments.
    fn lut(&self) -> Option<[u8; 256]> {
        let f: Box<dyn Fn(f32) -> f32> = match *self {
            Self::BrightnessContrast { brightness, contrast } => {
                // Brightness bends the midtones (keeps black and white), contrast pivots on grey.
                let gamma = 2f32.powf(-brightness / 100.0);
                let k = if contrast >= 0.0 { 1.0 + contrast / 50.0 } else { 1.0 + contrast / 100.0 };
                Box::new(move |v| (0.5 + (v.powf(gamma) - 0.5) * k).clamp(0.0, 1.0))
            }
            Self::Levels { in_black, in_white, gamma, out_black, out_white } => {
                let (ib, iw) = (in_black as f32 / 255.0, (in_white.max(in_black.saturating_add(1))) as f32 / 255.0);
                let (ob, ow) = (out_black as f32 / 255.0, out_white as f32 / 255.0);
                let g = gamma.clamp(0.1, 9.99);
                Box::new(move |v| {
                    let t = ((v - ib) / (iw - ib)).clamp(0.0, 1.0).powf(1.0 / g);
                    ob + t * (ow - ob)
                })
            }
            Self::Curves { ref points } => {
                let curve = curve_table(points);
                return Some(curve);
            }
            Self::Threshold { level } => {
                let l = level as f32 / 255.0;
                Box::new(move |v| if v >= l { 1.0 } else { 0.0 })
            }
            Self::Posterize { levels } => {
                let n = levels.max(2) as f32 - 1.0;
                Box::new(move |v| (v * n).round() / n)
            }
            Self::Invert => Box::new(|v| 1.0 - v),
            Self::HueSaturation { .. } | Self::Desaturate => return None,
        };
        Some(std::array::from_fn(|i| (f(i as f32 / 255.0) * 255.0).round().clamp(0.0, 255.0) as u8))
    }

    /// A per-pixel function equivalent to this adjustment.
    pub fn pixel_fn(&self) -> impl Fn([u8; 4]) -> [u8; 4] + Sync + use<> {
        let lut = self.lut();
        let this = self.clone();
        move |p: [u8; 4]| {
            if let Some(lut) = &lut {
                return match this {
                    // Threshold works on luminance, producing black or white.
                    Adjustment::Threshold { .. } => {
                        let v = lut[luma(p) as usize];
                        [v, v, v, p[3]]
                    }
                    _ => [lut[p[0] as usize], lut[p[1] as usize], lut[p[2] as usize], p[3]],
                };
            }
            match this {
                Adjustment::Desaturate => {
                    // Photoshop's Desaturate: HSL lightness, (max + min) / 2.
                    let v = ((p[0].max(p[1]).max(p[2]) as u16 + p[0].min(p[1]).min(p[2]) as u16 + 1) / 2) as u8;
                    [v, v, v, p[3]]
                }
                Adjustment::HueSaturation { hue, saturation, lightness } => {
                    let rgb = [p[0], p[1], p[2]].map(|c| c as f32 / 255.0);
                    let mut hsv = Hsv::from_srgb(rgb);
                    hsv.h = (hsv.h + hue).rem_euclid(360.0);
                    hsv.s = (hsv.s * (1.0 + saturation / 100.0)).clamp(0.0, 1.0);
                    let mut c = hsv.to_srgb();
                    let l = lightness / 100.0;
                    for v in &mut c {
                        *v = if l >= 0.0 { *v + (1.0 - *v) * l } else { *v * (1.0 + l) };
                    }
                    let [r, g, b] = c.map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8);
                    [r, g, b, p[3]]
                }
                _ => p,
            }
        }
    }
}

/// Rec. 601 luma, like Photoshop's grayscale conversion for Threshold.
pub fn luma(p: [u8; 4]) -> u8 {
    ((p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114 + 500) / 1000) as u8
}

/// Smooth curve through the control points (monotone cubic, so it never overshoots between
/// points). Inputs before the first / after the last point clamp to those points' outputs.
pub fn curve_table(points: &[(u8, u8)]) -> [u8; 256] {
    let mut pts: Vec<(f32, f32)> = points.iter().map(|&(x, y)| (x as f32, y as f32)).collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    pts.dedup_by(|a, b| a.0 == b.0);
    if pts.len() < 2 {
        return std::array::from_fn(|i| i as u8);
    }
    let n = pts.len();
    let d: Vec<f32> = (0..n - 1).map(|i| (pts[i + 1].1 - pts[i].1) / (pts[i + 1].0 - pts[i].0)).collect();
    // Fritsch-Carlson tangents.
    let mut m = vec![0f32; n];
    m[0] = d[0];
    m[n - 1] = d[n - 2];
    for i in 1..n - 1 {
        m[i] = if d[i - 1] * d[i] <= 0.0 { 0.0 } else { (d[i - 1] + d[i]) / 2.0 };
    }
    for i in 0..n - 1 {
        if d[i] == 0.0 {
            m[i] = 0.0;
            m[i + 1] = 0.0;
        } else {
            let (a, b) = (m[i] / d[i], m[i + 1] / d[i]);
            let s = a * a + b * b;
            if s > 9.0 {
                let t = 3.0 / s.sqrt();
                m[i] = t * a * d[i];
                m[i + 1] = t * b * d[i];
            }
        }
    }
    std::array::from_fn(|xi| {
        let x = xi as f32;
        if x <= pts[0].0 {
            return pts[0].1.round().clamp(0.0, 255.0) as u8;
        }
        if x >= pts[n - 1].0 {
            return pts[n - 1].1.round().clamp(0.0, 255.0) as u8;
        }
        let i = (0..n - 1).find(|&i| x <= pts[i + 1].0).unwrap_or(n - 2);
        let h = pts[i + 1].0 - pts[i].0;
        let t = (x - pts[i].0) / h;
        let (t2, t3) = (t * t, t * t * t);
        let y = (2.0 * t3 - 3.0 * t2 + 1.0) * pts[i].1
            + (t3 - 2.0 * t2 + t) * h * m[i]
            + (-2.0 * t3 + 3.0 * t2) * pts[i + 1].1
            + (t3 - t2) * h * m[i + 1];
        y.round().clamp(0.0, 255.0) as u8
    })
}

/// Applies a per-pixel function to `grid`, limited to `sel` (blended by coverage). Tiles with
/// nothing selected are shared, not copied. Runs tiles in parallel.
pub fn map_pixels(grid: &TileGrid, sel: Option<&Selection>, f: impl Fn([u8; 4]) -> [u8; 4] + Sync) -> TileGrid {
    map_pixels_xy(grid, sel, |_, _, p| f(p))
}

/// Like [`map_pixels`], but the function also gets the pixel's document coordinates.
pub fn map_pixels_xy(
    grid: &TileGrid,
    sel: Option<&Selection>,
    f: impl Fn(u32, u32, [u8; 4]) -> [u8; 4] + Sync,
) -> TileGrid {
    let tiles: Vec<(u32, u32, Arc<Tile>)> = grid.iter().map(|(x, y, t)| (x, y, t.clone())).collect();
    let done: Vec<(u32, u32, Arc<Tile>)> = tiles
        .into_par_iter()
        .map(|(tx, ty, tile)| {
            if sel.is_some_and(|s| s.tile_is_empty(tx, ty)) {
                return (tx, ty, tile);
            }
            let mut data = tile.data().to_vec();
            for (i, px) in data.chunks_exact_mut(4).enumerate() {
                let p: [u8; 4] = px.try_into().expect("4 bytes");
                if p[3] == 0 {
                    continue;
                }
                let (x, y) = (tx * TILE_SIZE + i as u32 % TILE_SIZE, ty * TILE_SIZE + i as u32 / TILE_SIZE);
                let mut out = f(x, y, p);
                if let Some(s) = sel {
                    let m = s.get(x, y) as u32;
                    if m == 0 {
                        continue;
                    }
                    if m < 255 {
                        for c in 0..4 {
                            out[c] = ((out[c] as u32 * m + p[c] as u32 * (255 - m) + 127) / 255) as u8;
                        }
                    }
                }
                px.copy_from_slice(&out);
            }
            (tx, ty, Arc::new(Tile::new(data.into_boxed_slice())))
        })
        .collect();
    let mut out = TileGrid::empty(grid.width(), grid.height());
    for (x, y, t) in done {
        out.set_tile(x, y, Some(t));
    }
    out
}

pub fn apply(grid: &TileGrid, adj: &Adjustment, sel: Option<&Selection>) -> TileGrid {
    map_pixels(grid, sel, adj.pixel_fn())
}

/// Luminance histogram of the selected (or all) non-transparent pixels, for Levels/Curves.
pub fn histogram(grid: &TileGrid, sel: Option<&Selection>) -> [u32; 256] {
    let mut h = [0u32; 256];
    for (tx, ty, tile) in grid.iter() {
        if sel.is_some_and(|s| s.tile_is_empty(tx, ty)) {
            continue;
        }
        for (i, p) in tile.data().chunks_exact(4).enumerate() {
            if p[3] == 0 {
                continue;
            }
            if let Some(s) = sel {
                let (x, y) = (tx * TILE_SIZE + i as u32 % TILE_SIZE, ty * TILE_SIZE + i as u32 / TILE_SIZE);
                if s.get(x, y) < 128 {
                    continue;
                }
            }
            h[luma([p[0], p[1], p[2], p[3]]) as usize] += 1;
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(adj: Adjustment, p: [u8; 4]) -> [u8; 4] {
        adj.pixel_fn()(p)
    }

    #[test]
    fn invert_threshold_posterize_desaturate() {
        assert_eq!(px(Adjustment::Invert, [0, 100, 255, 7]), [255, 155, 0, 7]);
        assert_eq!(px(Adjustment::Threshold { level: 128 }, [200, 200, 200, 255]), [255, 255, 255, 255]);
        assert_eq!(px(Adjustment::Threshold { level: 128 }, [20, 20, 200, 255]), [0, 0, 0, 255]);
        assert_eq!(px(Adjustment::Posterize { levels: 2 }, [100, 130, 255, 255]), [0, 255, 255, 255]);
        assert_eq!(px(Adjustment::Desaturate, [255, 0, 0, 255]), [128, 128, 128, 255]);
    }

    #[test]
    fn levels_maps_points_and_identity_is_identity() {
        let id = Adjustment::Levels { in_black: 0, in_white: 255, gamma: 1.0, out_black: 0, out_white: 255 };
        for v in [0u8, 1, 77, 128, 254, 255] {
            assert_eq!(px(id.clone(), [v, v, v, 255])[0], v);
        }
        let l = Adjustment::Levels { in_black: 50, in_white: 200, gamma: 1.0, out_black: 0, out_white: 255 };
        let p = px(l.clone(), [50, 200, 125, 255]);
        // 125 lands exactly on 127.5, so either neighbour is right.
        assert!(p[0] == 0 && p[1] == 255 && (127..=128).contains(&p[2]), "{p:?}");
        let dark = Adjustment::Levels { in_black: 0, in_white: 255, gamma: 0.5, out_black: 0, out_white: 255 };
        assert!(px(dark, [128, 0, 0, 255])[0] < 100);
    }

    #[test]
    fn curves_pass_through_points_and_stay_monotone() {
        let t = curve_table(&[(0, 0), (64, 100), (192, 220), (255, 255)]);
        assert_eq!((t[0], t[64], t[192], t[255]), (0, 100, 220, 255));
        assert!(t.windows(2).all(|w| w[1] >= w[0]));
        let id = curve_table(&[(0, 0), (255, 255)]);
        assert!(id.iter().enumerate().all(|(i, &v)| v as usize == i));
        // Clamps outside the points.
        let c = curve_table(&[(30, 10), (200, 240)]);
        assert_eq!((c[0], c[255]), (10, 240));
    }

    #[test]
    fn hue_saturation() {
        let shift = Adjustment::HueSaturation { hue: 120.0, saturation: 0.0, lightness: 0.0 };
        assert_eq!(px(shift, [255, 0, 0, 255]), [0, 255, 0, 255]);
        let grey = Adjustment::HueSaturation { hue: 0.0, saturation: -100.0, lightness: 0.0 };
        let p = px(grey, [200, 50, 50, 255]);
        assert!(p[0] == p[1] && p[1] == p[2]);
        let white = Adjustment::HueSaturation { hue: 0.0, saturation: 0.0, lightness: 100.0 };
        assert_eq!(px(white, [10, 20, 30, 255]), [255, 255, 255, 255]);
    }

    #[test]
    fn brightness_contrast_extremes() {
        let id = Adjustment::BrightnessContrast { brightness: 0.0, contrast: 0.0 };
        assert_eq!(px(id, [37, 128, 220, 255]), [37, 128, 220, 255]);
        let bright = Adjustment::BrightnessContrast { brightness: 100.0, contrast: 0.0 };
        let p = px(bright, [0, 128, 255, 255]);
        assert!(p[0] == 0 && p[1] > 160 && p[2] == 255, "{p:?}");
    }

    #[test]
    fn selection_limits_and_unselected_tiles_are_shared() {
        let grid = TileGrid::from_rgba8(600, 10, &[100, 100, 100, 255].repeat(6000));
        let sel = Selection::rect(600, 10, 0.0, 0.0, 10.0, 10.0);
        let out = apply(&grid, &Adjustment::Invert, Some(&sel));
        assert_eq!(out.pixel(5, 5), [155, 155, 155, 255]);
        assert_eq!(out.pixel(50, 5), [100, 100, 100, 255]);
        let id = |g: &TileGrid, x| g.tile(x, 0).unwrap().id();
        assert_eq!(id(&out, 2), id(&grid, 2), "untouched tile shared");
        assert_ne!(id(&out, 0), id(&grid, 0));
    }

    #[test]
    fn histogram_counts_visible_pixels() {
        let mut grid = TileGrid::from_rgba8(4, 1, &[0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 9, 9, 9, 0]);
        grid.set_pixel(3, 0, [9, 9, 9, 0]);
        let h = histogram(&grid, None);
        assert_eq!((h[0], h[255], h.iter().sum::<u32>()), (1, 2, 3));
    }
}
