//! Layer styles (Photoshop's Layer Style dialog): non-destructive effects drawn from a layer's
//! shape (its alpha). The layer's pixels never change; [`render`] produces "pixels + effects",
//! cached so the renderer gets the same tiles (and GPU textures) until something changes.

use std::sync::Mutex;
#[cfg(test)]
use std::sync::Arc;

use crate::selection::{Combine, Selection, over_srgb8};
use crate::transform::composite_over;
use crate::TileGrid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StrokePosition {
    #[default]
    Outside,
    Inside,
    Center,
}

impl StrokePosition {
    pub const ALL: [StrokePosition; 3] = [StrokePosition::Outside, StrokePosition::Inside, StrokePosition::Center];

    pub fn key(self) -> &'static str {
        match self {
            StrokePosition::Outside => "outside",
            StrokePosition::Inside => "inside",
            StrokePosition::Center => "center",
        }
    }

    pub fn from_key(k: &str) -> Self {
        Self::ALL.into_iter().find(|p| p.key() == k).unwrap_or_default()
    }
}

impl std::fmt::Display for StrokePosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            StrokePosition::Outside => "Outside",
            StrokePosition::Inside => "Inside",
            StrokePosition::Center => "Center",
        })
    }
}

/// Every effect keeps its settings while switched off, like Photoshop's checkboxes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropShadow {
    pub on: bool,
    pub color: [u8; 3],
    pub opacity: f32,
    /// Light direction in degrees (Photoshop's default 120 = from the upper left).
    pub angle: f32,
    pub distance: f32,
    /// 0..1: how much of `size` is solid growth rather than blur.
    pub spread: f32,
    pub size: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OuterGlow {
    pub on: bool,
    pub color: [u8; 3],
    pub opacity: f32,
    pub spread: f32,
    pub size: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stroke {
    pub on: bool,
    pub color: [u8; 3],
    pub opacity: f32,
    pub size: f32,
    pub position: StrokePosition,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorOverlay {
    pub on: bool,
    pub color: [u8; 3],
    pub opacity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerStyle {
    pub drop_shadow: DropShadow,
    pub outer_glow: OuterGlow,
    pub stroke: Stroke,
    pub color_overlay: ColorOverlay,
}

impl Default for LayerStyle {
    /// Photoshop's defaults, all switched off.
    fn default() -> Self {
        Self {
            drop_shadow: DropShadow { on: false, color: [0, 0, 0], opacity: 0.75, angle: 120.0, distance: 5.0, spread: 0.0, size: 5.0 },
            outer_glow: OuterGlow { on: false, color: [255, 255, 190], opacity: 0.75, spread: 0.0, size: 5.0 },
            stroke: Stroke { on: false, color: [0, 0, 0], opacity: 1.0, size: 3.0, position: StrokePosition::Outside },
            color_overlay: ColorOverlay { on: false, color: [255, 0, 0], opacity: 1.0 },
        }
    }
}

impl LayerStyle {
    /// No effect is switched on.
    pub fn is_empty(&self) -> bool {
        !(self.drop_shadow.on || self.outer_glow.on || self.stroke.on || self.color_overlay.on)
    }
}

/// Paints `color` through `mask` (coverage x opacity) over `out`.
fn paint(out: &mut TileGrid, mask: &Selection, color: [u8; 3], opacity: f32) {
    let Some([x0, y0, x1, y1]) = mask.bounds() else { return };
    for y in y0..y1 {
        for x in x0..x1 {
            let m = mask.get(x, y);
            if m == 0 {
                continue;
            }
            let a = (m as f32 * opacity + 0.5) as u8;
            if a == 0 {
                continue;
            }
            let d = out.pixel(x, y);
            out.set_pixel(x, y, over_srgb8(d, [color[0], color[1], color[2], a]));
        }
    }
}

/// The layer's pixels with its effects, bottom to top: drop shadow, outer glow, the pixels
/// (with colour overlay), stroke. Only on-canvas content casts effects.
pub fn render(pixels: &TileGrid, style: &LayerStyle) -> TileGrid {
    if style.is_empty() {
        return pixels.clone();
    }
    let (w, h) = (pixels.width(), pixels.height());
    let shape = Selection::from_alpha(pixels);
    let mut out = TileGrid::empty(w, h);

    let s = style.drop_shadow;
    if s.on && s.opacity > 0.0 {
        let a = s.angle.to_radians();
        let (dx, dy) = ((-a.cos() * s.distance).round() as i32, (a.sin() * s.distance).round() as i32);
        let mut m = shape.translated(dx, dy);
        let grow = (s.size * s.spread).round() as u32;
        if grow > 0 {
            m = m.expand(grow);
        }
        let blur = s.size * (1.0 - s.spread);
        if blur >= 0.5 {
            m = m.feather(blur / 2.0);
        }
        paint(&mut out, &m, s.color, s.opacity);
    }

    let g = style.outer_glow;
    if g.on && g.opacity > 0.0 && g.size > 0.0 {
        let grow = (g.size * g.spread).round() as u32;
        let mut m = if grow > 0 { shape.expand(grow) } else { shape.clone() };
        let blur = g.size * (1.0 - g.spread);
        if blur >= 0.5 {
            m = m.feather(blur / 2.0);
        }
        paint(&mut out, &m, g.color, g.opacity);
    }

    let o = style.color_overlay;
    if o.on && o.opacity > 0.0 {
        let mut tinted = pixels.clone();
        let tiles: Vec<(u32, u32)> = pixels.iter().map(|(x, y, _)| (x, y)).collect();
        for (tx, ty) in tiles {
            if let Some(tile) = tinted.tile_mut(tx, ty) {
                for p in tile.data_mut().chunks_exact_mut(4) {
                    for c in 0..3 {
                        p[c] = (p[c] as f32 * (1.0 - o.opacity) + o.color[c] as f32 * o.opacity + 0.5) as u8;
                    }
                }
            }
        }
        composite_over(&mut out, &tinted);
    } else {
        composite_over(&mut out, pixels);
    }

    let st = style.stroke;
    if st.on && st.opacity > 0.0 && st.size >= 0.5 {
        let r = st.size.round().max(1.0) as u32;
        let ring = match st.position {
            StrokePosition::Outside => {
                let mut m = shape.expand(r);
                m.combine(&shape, Combine::Subtract);
                m
            }
            StrokePosition::Inside => {
                let mut m = shape.clone();
                m.combine(&shape.contract(r), Combine::Subtract);
                m
            }
            StrokePosition::Center => {
                let half = (r / 2).max(1);
                let mut m = shape.expand(half);
                m.combine(&shape.contract(r - half), Combine::Subtract);
                m
            }
        };
        paint(&mut out, &ring, st.color, st.opacity);
    }
    out
}

/// Cached [`render`]: same pixels (tile ids) and style give back the same tiles, so the GPU
/// keeps its textures and nothing is recomputed while only other layers change.
pub fn render_cached(pixels: &TileGrid, style: &LayerStyle) -> TileGrid {
    if style.is_empty() {
        return pixels.clone();
    }
    type Entry = (Vec<u64>, LayerStyle, TileGrid);
    static CACHE: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
    let mut key: Vec<u64> = vec![pixels.width() as u64, pixels.height() as u64];
    for (x, y, t) in pixels.iter_all() {
        key.extend([x as i64 as u64, y as i64 as u64, t.id().raw()]);
    }
    if let Ok(cache) = CACHE.lock()
        && let Some((_, _, grid)) = cache.iter().find(|(k, s, _)| *k == key && s == style)
    {
        return grid.clone();
    }
    let grid = render(pixels, style);
    if let Ok(mut cache) = CACHE.lock() {
        // A handful of styled layers at once is typical; keep the most recent ones.
        if cache.len() >= 32 {
            cache.remove(0);
        }
        cache.push((key, *style, grid.clone()));
    }
    grid
}

/// Tiles in the effect render that came straight from the layer (for tests).
#[cfg(test)]
fn shares_tiles(a: &TileGrid, b: &TileGrid) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| Arc::ptr_eq(x.2, y.2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> TileGrid {
        let mut g = TileGrid::empty(100, 100);
        for y in 40..60 {
            for x in 40..60 {
                g.set_pixel(x, y, [200, 200, 200, 255]);
            }
        }
        g
    }

    #[test]
    fn empty_style_is_the_layer_itself() {
        let g = square();
        assert!(shares_tiles(&render_cached(&g, &LayerStyle::default()), &g));
    }

    #[test]
    fn drop_shadow_falls_down_and_right_under_the_layer() {
        let mut style = LayerStyle::default();
        style.drop_shadow = DropShadow { on: true, distance: 10.0, size: 0.0, opacity: 1.0, ..style.drop_shadow };
        let out = render(&square(), &style);
        assert_eq!(out.pixel(50, 50), [200, 200, 200, 255], "the layer stays on top");
        // 120 degrees: light from the upper left, so the shadow is offset (+5, +9).
        assert_eq!(out.pixel(63, 66), [0, 0, 0, 255]);
        assert_eq!(out.pixel(35, 35)[3], 0, "nothing up-left");
    }

    #[test]
    fn stroke_positions() {
        let mut style = LayerStyle::default();
        style.stroke = Stroke { on: true, size: 3.0, color: [255, 0, 0], ..style.stroke };
        let out = render(&square(), &style);
        assert_eq!(out.pixel(38, 50), [255, 0, 0, 255], "outside ring");
        assert_eq!(out.pixel(41, 50), [200, 200, 200, 255], "inside untouched");
        style.stroke.position = StrokePosition::Inside;
        let out = render(&square(), &style);
        assert_eq!(out.pixel(38, 50)[3], 0);
        assert_eq!(out.pixel(41, 50), [255, 0, 0, 255]);
    }

    #[test]
    fn overlay_and_glow() {
        let mut style = LayerStyle::default();
        style.color_overlay = ColorOverlay { on: true, color: [0, 0, 255], opacity: 1.0 };
        style.outer_glow = OuterGlow { on: true, size: 10.0, spread: 0.5, ..style.outer_glow };
        let out = render(&square(), &style);
        assert_eq!(out.pixel(50, 50), [0, 0, 255, 255]);
        assert!(out.pixel(36, 50)[3] > 0, "glow around the shape");
        assert_eq!(out.pixel(5, 5)[3], 0);
    }

    #[test]
    fn cache_returns_the_same_tiles() {
        let mut style = LayerStyle::default();
        style.stroke.on = true;
        let g = square();
        let (a, b) = (render_cached(&g, &style), render_cached(&g, &style));
        assert!(shares_tiles(&a, &b));
    }
}
