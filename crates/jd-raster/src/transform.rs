//! Moving, scaling and rotating pixels.
//!
//! While the user drags, the GPU previews the change by transforming tile quads. On commit we
//! resample here: whole-pixel moves are exact copies; anything else is bilinear in
//! premultiplied alpha (so edges don't pick up dark fringes).

use rayon::prelude::*;

use crate::selection::{Selection, over_srgb8};
use crate::{TILE_SIZE, TileGrid};

/// 2D affine map: `x' = a*x + c*y + e`, `y' = b*x + d*y + f` (column-major, like CSS/SVG).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Self = Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub fn translate(x: f32, y: f32) -> Self {
        Self { e: x, f: y, ..Self::IDENTITY }
    }

    /// Maps `center` (the original box centre) to `new_center`, rotated by `angle` radians and
    /// scaled by (sx, sy) in the box's own axes. This is how Free Transform is parameterised.
    pub fn from_parts(center: (f32, f32), new_center: (f32, f32), angle: f32, sx: f32, sy: f32) -> Self {
        let (s, c) = angle.sin_cos();
        let (a, b, cc, d) = (c * sx, s * sx, -s * sy, c * sy);
        Self {
            a,
            b,
            c: cc,
            d,
            e: new_center.0 - (a * center.0 + cc * center.1),
            f: new_center.1 - (b * center.0 + d * center.1),
        }
    }

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    pub fn inverse(&self) -> Option<Self> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-8 {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Self { a, b, c, d, e: -(a * self.e + c * self.f), f: -(b * self.e + d * self.f) })
    }

    /// `Some((dx, dy))` if this is a pure whole-pixel translation.
    pub fn integer_translation(&self) -> Option<(i32, i32)> {
        let near = |v: f32, t: f32| (v - t).abs() < 1e-4;
        (near(self.a, 1.0) && near(self.b, 0.0) && near(self.c, 0.0) && near(self.d, 1.0)
            && near(self.e, self.e.round())
            && near(self.f, self.f.round()))
        .then(|| (self.e.round() as i32, self.f.round() as i32))
    }

    pub fn as_array(&self) -> [f32; 6] {
        [self.a, self.b, self.c, self.d, self.e, self.f]
    }
}

/// Tight bounds of non-transparent pixels: x0, y0, x1, y1 (exclusive).
pub fn content_bounds(grid: &TileGrid) -> Option<[u32; 4]> {
    let mut b: Option<[u32; 4]> = None;
    for (tx, ty, tile) in grid.iter() {
        let data = tile.data();
        for ly in 0..TILE_SIZE {
            for lx in 0..TILE_SIZE {
                if data[((ly * TILE_SIZE + lx) * 4 + 3) as usize] == 0 {
                    continue;
                }
                let (x, y) = (tx * TILE_SIZE + lx, ty * TILE_SIZE + ly);
                if x >= grid.width() || y >= grid.height() {
                    continue;
                }
                b = Some(match b {
                    None => [x, y, x + 1, y + 1],
                    Some([a, c, d, e]) => [a.min(x), c.min(y), d.max(x + 1), e.max(y + 1)],
                });
            }
        }
    }
    b
}

/// Where `bounds` lands after `m`, as a pixel box clipped to `w`×`h`.
fn mapped_box(bounds: [u32; 4], m: &Affine, w: u32, h: u32) -> Option<[u32; 4]> {
    let [x0, y0, x1, y1] = bounds.map(|v| v as f32);
    let pts = [m.apply(x0, y0), m.apply(x1, y0), m.apply(x0, y1), m.apply(x1, y1)];
    let minx = pts.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor().max(0.0) as u32;
    let miny = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor().max(0.0) as u32;
    let maxx = (pts.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil().max(0.0) as u32).min(w);
    let maxy = (pts.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil().max(0.0) as u32).min(h);
    (minx < maxx && miny < maxy).then_some([minx, miny, maxx, maxy])
}

/// Resamples `src` through `m` into a new grid of the same size (content outside is cropped).
pub fn transform_grid(src: &TileGrid, m: &Affine) -> TileGrid {
    let (w, h) = (src.width(), src.height());
    let mut out = TileGrid::empty(w, h);
    let Some(bounds) = content_bounds(src) else { return out };
    if let Some((dx, dy)) = m.integer_translation() {
        // Exact copy, row by row.
        let [x0, y0, x1, y1] = bounds;
        let rw = x1 - x0;
        let mut row = vec![0u8; rw as usize * 4];
        for y in y0..y1 {
            let ny = y as i64 + dy as i64;
            if ny < 0 || ny >= h as i64 {
                continue;
            }
            for (i, x) in (x0..x1).enumerate() {
                row[i * 4..i * 4 + 4].copy_from_slice(&src.pixel(x, y));
            }
            // Clip on the left by skipping source pixels that land at x < 0.
            let nx = x0 as i64 + dx as i64;
            let skip = (-nx).max(0) as usize;
            if skip >= rw as usize {
                continue;
            }
            let start = (nx.max(0)) as u32;
            out.write_region(start, ny as u32, rw - skip as u32, 1, &row[skip * 4..]);
        }
        return out;
    }
    let Some(inv) = m.inverse() else { return out };
    let Some([ox0, oy0, ox1, oy1]) = mapped_box(bounds, m, w, h) else { return out };
    let ow = (ox1 - ox0) as usize;

    // Premultiplied bilinear sample at continuous source position (pixel centres at +0.5).
    let sample = |sx: f32, sy: f32| -> [f32; 4] {
        let (fx, fy) = (sx - 0.5, sy - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let mut acc = [0f32; 4];
        for (dx, dy, wgt) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
            let (x, y) = (x0 as i64 + dx, y0 as i64 + dy);
            if x < 0 || y < 0 || wgt <= 0.0 {
                continue;
            }
            let p = src.pixel(x as u32, y as u32);
            let a = p[3] as f32 / 255.0 * wgt;
            acc[0] += p[0] as f32 * a;
            acc[1] += p[1] as f32 * a;
            acc[2] += p[2] as f32 * a;
            acc[3] += a;
        }
        acc
    };

    let rows: Vec<Vec<u8>> = (oy0..oy1)
        .into_par_iter()
        .map(|y| {
            let mut row = vec![0u8; ow * 4];
            for (i, x) in (ox0..ox1).enumerate() {
                let (sx, sy) = inv.apply(x as f32 + 0.5, y as f32 + 0.5);
                let [r, g, b, a] = sample(sx, sy);
                if a > 0.0 {
                    row[i * 4] = (r / a + 0.5).min(255.0) as u8;
                    row[i * 4 + 1] = (g / a + 0.5).min(255.0) as u8;
                    row[i * 4 + 2] = (b / a + 0.5).min(255.0) as u8;
                    row[i * 4 + 3] = (a * 255.0 + 0.5).min(255.0) as u8;
                }
            }
            row
        })
        .collect();
    let flat: Vec<u8> = rows.concat();
    out.write_region(ox0, oy0, ow as u32, oy1 - oy0, &flat);
    out
}

/// Resamples a selection mask through `m` (bilinear on coverage).
pub fn transform_selection(sel: &Selection, m: &Affine) -> Selection {
    let (w, h) = (sel.width(), sel.height());
    if let Some((dx, dy)) = m.integer_translation() {
        return sel.translated(dx, dy);
    }
    let mut out = Selection::empty(w, h);
    let (Some(bounds), Some(inv)) = (sel.bounds(), m.inverse()) else { return out };
    let Some([ox0, oy0, ox1, oy1]) = mapped_box(bounds, m, w, h) else { return out };
    let get = |x: f32, y: f32| -> f32 {
        if x < 0.0 || y < 0.0 { 0.0 } else { sel.get(x as u32, y as u32) as f32 }
    };
    for y in oy0..oy1 {
        for x in ox0..ox1 {
            let (sx, sy) = inv.apply(x as f32 + 0.5, y as f32 + 0.5);
            let (fx, fy) = (sx - 0.5, sy - 0.5);
            let (x0, y0) = (fx.floor(), fy.floor());
            let (tx, ty) = (fx - x0, fy - y0);
            let v = get(x0, y0) * (1.0 - tx) * (1.0 - ty)
                + get(x0 + 1.0, y0) * tx * (1.0 - ty)
                + get(x0, y0 + 1.0) * (1.0 - tx) * ty
                + get(x0 + 1.0, y0 + 1.0) * tx * ty;
            if v >= 0.5 {
                out.set(x, y, (v + 0.5).min(255.0) as u8);
            }
        }
    }
    out
}

/// Composites `src` onto `dst` with normal blending (used to drop a floating selection).
pub fn composite_over(dst: &mut TileGrid, src: &TileGrid) {
    let Some([x0, y0, x1, y1]) = content_bounds(src) else { return };
    for y in y0..y1 {
        for x in x0..x1 {
            let s = src.pixel(x, y);
            if s[3] == 0 {
                continue;
            }
            let d = dst.pixel(x, y);
            dst.set_pixel(x, y, over_srgb8(d, s));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(w: u32, h: u32) -> TileGrid {
        let px: Vec<u8> = (0..w * h).flat_map(|i| if (i % w + i / w) % 2 == 0 { [255, 0, 0, 255] } else { [0, 0, 255, 255] }).collect();
        TileGrid::from_rgba8(w, h, &px)
    }

    #[test]
    fn affine_inverse_roundtrips() {
        let m = Affine::from_parts((50.0, 40.0), (70.0, 10.0), 0.7, 1.5, 0.5);
        let inv = m.inverse().unwrap();
        let (x, y) = m.apply(13.0, 27.0);
        let (bx, by) = inv.apply(x, y);
        assert!((bx - 13.0).abs() < 1e-3 && (by - 27.0).abs() < 1e-3);
        assert_eq!(m.apply(50.0, 40.0), (70.0, 10.0));
    }

    #[test]
    fn integer_move_is_exact_and_clips() {
        let src = checker(300, 20);
        let out = transform_grid(&src, &Affine::translate(-5.0, 3.0));
        assert_eq!(out.pixel(0, 3), src.pixel(5, 0));
        assert_eq!(out.pixel(290, 22 - 3), src.pixel(295, 16));
        assert_eq!(out.pixel(299, 0), [0; 4]);
        assert_eq!(content_bounds(&out), Some([0, 3, 295, 20]));
    }

    #[test]
    fn scale_by_two_doubles_the_content_box() {
        let mut src = TileGrid::empty(100, 100);
        for y in 10..20 {
            for x in 10..20 {
                src.set_pixel(x, y, [0, 200, 0, 255]);
            }
        }
        let m = Affine::from_parts((15.0, 15.0), (30.0, 30.0), 0.0, 2.0, 2.0);
        let out = transform_grid(&src, &m);
        let [x0, y0, x1, y1] = content_bounds(&out).unwrap();
        assert!((19..=21).contains(&x0) && (39..=41).contains(&x1), "{x0} {x1}");
        assert!((19..=21).contains(&y0) && (39..=41).contains(&y1));
        assert_eq!(out.pixel(30, 30), [0, 200, 0, 255]);
    }

    #[test]
    fn rotation_keeps_colour_without_dark_fringes() {
        let mut src = TileGrid::empty(64, 64);
        for y in 20..44 {
            for x in 20..44 {
                src.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        let out = transform_grid(&src, &Affine::from_parts((32.0, 32.0), (32.0, 32.0), 0.5, 1.0, 1.0));
        // Every partially covered edge pixel is still white, just less opaque.
        for y in 0..64 {
            for x in 0..64 {
                let p = out.pixel(x, y);
                if p[3] > 0 {
                    assert!(p[0] >= 254, "fringe at {x},{y}: {p:?}");
                }
            }
        }
    }

    #[test]
    fn composite_over_drops_floating_pixels() {
        let mut dst = TileGrid::from_rgba8(2, 1, &[0, 0, 0, 255, 0, 0, 0, 255]);
        let mut src = TileGrid::empty(2, 1);
        src.set_pixel(1, 0, [255, 255, 255, 255]);
        composite_over(&mut dst, &src);
        assert_eq!((dst.pixel(0, 0), dst.pixel(1, 0)), ([0, 0, 0, 255], [255, 255, 255, 255]));
    }
}

/// Copies `src`'s content into a new `w`×`h` grid, shifted by (dx, dy). Used to move layers
/// between documents and for crop / canvas size. Same size and no shift shares the tiles.
pub fn place(src: &TileGrid, w: u32, h: u32, dx: i64, dy: i64) -> TileGrid {
    if dx == 0 && dy == 0 && src.width() == w && src.height() == h {
        return src.clone();
    }
    let mut out = TileGrid::empty(w, h);
    let Some([x0, y0, x1, y1]) = content_bounds(src) else { return out };
    // Visible part in destination coordinates.
    let (ox0, oy0) = ((x0 as i64 + dx).max(0), (y0 as i64 + dy).max(0));
    let (ox1, oy1) = ((x1 as i64 + dx).min(w as i64), (y1 as i64 + dy).min(h as i64));
    if ox0 >= ox1 || oy0 >= oy1 {
        return out;
    }
    let ow = (ox1 - ox0) as usize;
    let rows: Vec<Vec<u8>> = (oy0..oy1)
        .into_par_iter()
        .map(|y| {
            let mut row = vec![0u8; ow * 4];
            for (i, x) in (ox0..ox1).enumerate() {
                row[i * 4..i * 4 + 4].copy_from_slice(&src.pixel((x - dx) as u32, (y - dy) as u32));
            }
            row
        })
        .collect();
    out.write_region(ox0 as u32, oy0 as u32, ow as u32, (oy1 - oy0) as u32, &rows.concat());
    out
}

/// Pixel-exact remap into a `w`×`h` grid: output (x, y) takes `src` at `f(x, y)`. For rotating
/// and flipping the canvas.
pub fn remap(src: &TileGrid, w: u32, h: u32, f: impl Fn(u32, u32) -> (u32, u32) + Sync) -> TileGrid {
    let rows: Vec<Vec<u8>> = (0..h)
        .into_par_iter()
        .map(|y| {
            let mut row = vec![0u8; w as usize * 4];
            for x in 0..w {
                let (sx, sy) = f(x, y);
                row[x as usize * 4..x as usize * 4 + 4].copy_from_slice(&src.pixel(sx, sy));
            }
            row
        })
        .collect();
    let mut out = TileGrid::empty(w, h);
    out.write_region(0, 0, w, h, &rows.concat());
    out
}

/// Resamples the whole grid to `w`×`h` (Image Size). Separable filter that averages over the
/// source footprint when shrinking (no aliasing) and is bilinear when enlarging. Premultiplied.
pub fn resize(src: &TileGrid, w: u32, h: u32) -> TileGrid {
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let (w, h) = (w.max(1) as usize, h.max(1) as usize);
    let mut buf: Vec<[f32; 4]> = (0..sh)
        .into_par_iter()
        .flat_map_iter(|y| {
            (0..sw).map(move |x| {
                let p = src.pixel(x as u32, y as u32);
                let a = p[3] as f32 / 255.0;
                [p[0] as f32 * a, p[1] as f32 * a, p[2] as f32 * a, a]
            })
        })
        .collect();
    // Horizontal pass: sw -> w for each row, then vertical: sh -> h for each column.
    buf = resample_rows(&buf, sw, sh, w);
    let t = transpose4(&buf, w, sh);
    let t = resample_rows(&t, sh, w, h);
    let out = transpose4(&t, h, w);
    let rows: Vec<u8> = out
        .iter()
        .flat_map(|v| {
            if v[3] <= 0.0 {
                [0u8; 4]
            } else {
                let a = v[3].min(1.0);
                [(v[0] / v[3]).clamp(0.0, 255.0) as u8, (v[1] / v[3]).clamp(0.0, 255.0) as u8, (v[2] / v[3]).clamp(0.0, 255.0) as u8, (a * 255.0 + 0.5) as u8]
            }
        })
        .collect();
    let mut grid = TileGrid::empty(w as u32, h as u32);
    grid.write_region(0, 0, w as u32, h as u32, &rows);
    grid
}

fn resample_rows(buf: &[[f32; 4]], sw: usize, rows: usize, w: usize) -> Vec<[f32; 4]> {
    let scale = w as f32 / sw as f32;
    let support = (1.0 / scale).max(1.0);
    // Precompute weights per output column (triangle filter widened by the shrink factor).
    let taps: Vec<(usize, Vec<f32>)> = (0..w)
        .map(|x| {
            let centre = (x as f32 + 0.5) / scale;
            let lo = (centre - support).floor().max(0.0) as usize;
            let hi = ((centre + support).ceil() as usize).min(sw);
            let mut weights: Vec<f32> = (lo..hi).map(|i| (1.0 - ((i as f32 + 0.5 - centre) / support).abs()).max(0.0)).collect();
            let sum: f32 = weights.iter().sum();
            if sum > 0.0 {
                weights.iter_mut().for_each(|v| *v /= sum);
            }
            (lo, weights)
        })
        .collect();
    let mut out = vec![[0f32; 4]; w * rows];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let src = &buf[y * sw..(y + 1) * sw];
        for (x, (lo, weights)) in taps.iter().enumerate() {
            let mut acc = [0f32; 4];
            for (k, wt) in weights.iter().enumerate() {
                let p = src[lo + k];
                for c in 0..4 {
                    acc[c] += p[c] * wt;
                }
            }
            row[x] = acc;
        }
    });
    out
}

fn transpose4(buf: &[[f32; 4]], w: usize, h: usize) -> Vec<[f32; 4]> {
    let mut out = vec![[0f32; 4]; w * h];
    out.par_chunks_mut(h).enumerate().for_each(|(x, col)| {
        for (y, v) in col.iter_mut().enumerate() {
            *v = buf[y * w + x];
        }
    });
    out
}

#[cfg(test)]
mod place_tests {
    use super::*;

    #[test]
    fn place_shifts_and_clips_and_shares_when_unchanged() {
        let mut g = TileGrid::empty(300, 20);
        g.set_pixel(10, 5, [1, 2, 3, 255]);
        g.set_pixel(290, 5, [4, 5, 6, 255]);
        let out = place(&g, 100, 50, -5, 10);
        assert_eq!(out.pixel(5, 15), [1, 2, 3, 255]);
        assert_eq!((out.width(), out.height()), (100, 50));
        assert_eq!(content_bounds(&out), Some([5, 15, 6, 16]), "the far pixel is clipped off");
        let same = place(&g, 300, 20, 0, 0);
        assert_eq!(same.tile(0, 0).unwrap().id(), g.tile(0, 0).unwrap().id());
    }

    #[test]
    fn remap_rotates_exactly() {
        let mut g = TileGrid::empty(3, 2);
        g.set_pixel(0, 0, [9, 9, 9, 255]);
        // 90° clockwise: (x, y) in the 2×3 output reads src (y, h-1-x).
        let out = remap(&g, 2, 3, |x, y| (y, 1 - x));
        assert_eq!(out.pixel(1, 0), [9, 9, 9, 255]);
    }

    #[test]
    fn resize_averages_when_shrinking() {
        // Fine black/white stripes shrunk 4x become mid grey, not aliased stripes.
        let px: Vec<u8> = (0..64 * 4).flat_map(|i| if i % 2 == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }).collect();
        let g = TileGrid::from_rgba8(64, 4, &px);
        let out = resize(&g, 16, 1);
        for x in 1..15 {
            let v = out.pixel(x, 0)[0];
            assert!((110..=145).contains(&v), "x={x}: {v}");
        }
        let up = resize(&TileGrid::from_rgba8(2, 1, &[0, 0, 0, 255, 200, 200, 200, 255]), 8, 1);
        assert!(up.pixel(0, 0)[0] < up.pixel(4, 0)[0] && up.pixel(4, 0)[0] < up.pixel(7, 0)[0] + 1);
    }
}
