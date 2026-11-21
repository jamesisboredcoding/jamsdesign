//! The Gradient tool: fills the layer (or selection) from one colour to another.

use std::sync::Arc;

use rayon::prelude::*;

use crate::blend::{self, BlendMode};
use crate::composite::{to_srgb8, to_working};
use crate::selection::Selection;
use crate::{BlendSpace, TILE_SIZE, Tile, TileGrid};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Along the dragged line.
    Linear,
    /// Circles around the start point, out to the end point.
    Radial,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gradient {
    pub shape: Shape,
    pub from: (f32, f32),
    pub to: (f32, f32),
    /// sRGB start / end colours.
    pub start: [u8; 3],
    pub end: [u8; 3],
    pub opacity: f32,
    pub mode: BlendMode,
}

impl Gradient {
    /// 0..1 position of a pixel centre along the gradient.
    fn t(&self, x: f32, y: f32) -> f32 {
        let (dx, dy) = (self.to.0 - self.from.0, self.to.1 - self.from.1);
        let len2 = (dx * dx + dy * dy).max(1e-6);
        match self.shape {
            Shape::Linear => ((x - self.from.0) * dx + (y - self.from.1) * dy) / len2,
            Shape::Radial => ((x - self.from.0).powi(2) + (y - self.from.1).powi(2)).sqrt() / len2.sqrt(),
        }
        .clamp(0.0, 1.0)
    }
}

/// Draws `g` over every canvas pixel of `grid` (limited to `sel`), blending in working space.
pub fn apply(grid: &TileGrid, g: &Gradient, sel: Option<&Selection>, space: BlendSpace) -> TileGrid {
    let (w, h) = (grid.width(), grid.height());
    let (a, b) = (to_working([g.start[0], g.start[1], g.start[2], 255], space), to_working([g.end[0], g.end[1], g.end[2], 255], space));
    let coords: Vec<(u32, u32)> = (0..h.div_ceil(TILE_SIZE)).flat_map(|ty| (0..w.div_ceil(TILE_SIZE)).map(move |tx| (tx, ty))).collect();
    let tiles: Vec<(u32, u32, Option<Arc<Tile>>)> = coords
        .into_par_iter()
        .map(|(tx, ty)| {
            let original = grid.tile(tx, ty).cloned();
            if sel.is_some_and(|s| s.tile_is_empty(tx, ty)) {
                return (tx, ty, original);
            }
            let mut data = original.as_ref().map_or_else(|| vec![0u8; (TILE_SIZE * TILE_SIZE * 4) as usize], |t| t.data().to_vec());
            for (i, px) in data.chunks_exact_mut(4).enumerate() {
                let (x, y) = (tx * TILE_SIZE + i as u32 % TILE_SIZE, ty * TILE_SIZE + i as u32 / TILE_SIZE);
                if x >= w || y >= h {
                    continue;
                }
                let mut m = g.opacity;
                if let Some(s) = sel {
                    m *= s.get(x, y) as f32 / 255.0;
                }
                if m <= 0.0 {
                    continue;
                }
                let t = g.t(x as f32 + 0.5, y as f32 + 0.5);
                let src = [0, 1, 2].map(|c| (a[c] + (b[c] - a[c]) * t) * m);
                let o: [u8; 4] = (&*px).try_into().expect("4 bytes");
                let out = to_srgb8(blend::composite(g.mode, to_working(o, space), [src[0], src[1], src[2], m]), space);
                px.copy_from_slice(&out);
            }
            (tx, ty, Some(Arc::new(Tile::new(data.into_boxed_slice()))))
        })
        .collect();
    let mut out = grid.clone();
    for (tx, ty, t) in tiles {
        out.set_tile(tx, ty, t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(shape: Shape) -> Gradient {
        Gradient { shape, from: (0.0, 0.0), to: (100.0, 0.0), start: [0, 0, 0], end: [255, 255, 255], opacity: 1.0, mode: BlendMode::Normal }
    }

    #[test]
    fn linear_ramps_along_the_line_and_clamps() {
        let out = apply(&TileGrid::empty(300, 4), &g(Shape::Linear), None, BlendSpace::Perceptual);
        assert_eq!(out.pixel(0, 0)[0], 1);
        let mid = out.pixel(50, 2)[0];
        assert!((126..=130).contains(&mid), "{mid}");
        assert_eq!(out.pixel(250, 0), [255, 255, 255, 255], "past the end stays the end colour");
    }

    #[test]
    fn radial_and_selection() {
        let sel = Selection::rect(200, 200, 0.0, 0.0, 50.0, 200.0);
        let out = apply(&TileGrid::empty(200, 200), &g(Shape::Radial), Some(&sel), BlendSpace::Perceptual);
        assert!(out.pixel(10, 10)[0] < out.pixel(40, 40)[0]);
        assert_eq!(out.pixel(100, 0)[3], 0, "outside the selection untouched");
    }
}
