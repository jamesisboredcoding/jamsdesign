//! Pixel selections: a coverage mask (0 = not selected, 255 = fully selected) over the
//! document, stored as 256×256 tiles like layers. Empty tiles cost nothing and fully selected
//! tiles all share one buffer, so "Select All" on a huge image is cheap, and undo snapshots
//! share unchanged tiles.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use crate::{TILE_SIZE, TileGrid};

const TS: usize = TILE_SIZE as usize;
const TILE_LEN: usize = TS * TS;

static FULL: LazyLock<Arc<[u8]>> = LazyLock::new(|| vec![255u8; TILE_LEN].into());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Combine {
    /// Replace the current selection.
    #[default]
    Replace,
    Add,
    Subtract,
    Intersect,
}

fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone)]
pub struct Selection {
    /// Changes whenever the contents change; clones share it. Lets UIs cache the outline.
    id: u64,
    width: u32,
    height: u32,
    cols: u32,
    rows: u32,
    tiles: Vec<Option<Arc<[u8]>>>,
}

/// Horizontal or vertical outline segment in document pixel corners: (x0, y0) to (x1, y1).
pub type Segment = [u32; 4];

impl Selection {
    pub fn empty(width: u32, height: u32) -> Self {
        let cols = width.div_ceil(TILE_SIZE);
        let rows = height.div_ceil(TILE_SIZE);
        Self { id: next_id(), width, height, cols, rows, tiles: vec![None; (cols * rows) as usize] }
    }

    pub fn all(width: u32, height: u32) -> Self {
        Self::from_fn(width, height, [0, 0, width, height], |_, _| 255)
    }

    /// Edge tiles extend past the document; they can never be the shared `FULL` tile.
    fn is_partial(&self, tx: u32, ty: u32) -> bool {
        (tx + 1) * TILE_SIZE > self.width || (ty + 1) * TILE_SIZE > self.height
    }

    /// Builds a selection from a per-pixel function (called only inside `bounds`).
    fn from_fn(width: u32, height: u32, bounds: [u32; 4], f: impl Fn(u32, u32) -> u8 + Sync) -> Self {
        let mut s = Self::empty(width, height);
        let [x0, y0, x1, y1] = clamp_bounds(bounds, width, height);
        if x0 >= x1 || y0 >= y1 {
            return s;
        }
        for ty in y0 / TILE_SIZE..=(y1 - 1) / TILE_SIZE {
            for tx in x0 / TILE_SIZE..=(x1 - 1) / TILE_SIZE {
                let mut buf = vec![0u8; TILE_LEN];
                let (mut any, mut full) = (false, !s.is_partial(tx, ty));
                for ly in 0..TILE_SIZE {
                    for lx in 0..TILE_SIZE {
                        let (x, y) = (tx * TILE_SIZE + lx, ty * TILE_SIZE + ly);
                        let v = if x >= x0 && x < x1 && y >= y0 && y < y1 { f(x, y) } else { 0 };
                        buf[(ly * TILE_SIZE + lx) as usize] = v;
                        any |= v != 0;
                        full &= v == 255;
                    }
                }
                let i = (ty * s.cols + tx) as usize;
                s.tiles[i] = if !any {
                    None
                } else if full {
                    Some(FULL.clone())
                } else {
                    Some(buf.into())
                };
            }
        }
        s
    }

    /// An anti-aliased rectangle (fractional edges get partial coverage).
    pub fn rect(width: u32, height: u32, x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        let (x0, x1) = (x0.min(x1), x0.max(x1));
        let (y0, y1) = (y0.min(y1), y0.max(y1));
        let cover = |a: f32, lo: f32, hi: f32| (hi.min(a + 1.0) - lo.max(a)).clamp(0.0, 1.0);
        Self::from_fn(width, height, bounds_of(x0, y0, x1, y1), |x, y| {
            let c = cover(x as f32, x0, x1) * cover(y as f32, y0, y1);
            (c * 255.0 + 0.5) as u8
        })
    }

    /// An anti-aliased ellipse inside the box (x0, y0)-(x1, y1).
    pub fn ellipse(width: u32, height: u32, x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (rx, ry) = (((x1 - x0) / 2.0).abs().max(0.01), ((y1 - y0) / 2.0).abs().max(0.01));
        Self::from_fn(width, height, bounds_of(x0, y0, x1, y1), |x, y| {
            // 4×4 supersampling.
            let mut n = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = (x as f32 + (sx as f32 + 0.5) / 4.0 - cx) / rx;
                    let py = (y as f32 + (sy as f32 + 0.5) / 4.0 - cy) / ry;
                    n += (px * px + py * py <= 1.0) as u32;
                }
            }
            (n * 255 / 16) as u8
        })
    }

    /// A polygon (lasso), even-odd fill, anti-aliased with 4 sub-scanlines per pixel row and
    /// exact horizontal coverage.
    pub fn polygon(width: u32, height: u32, points: &[(f32, f32)]) -> Self {
        if points.len() < 3 {
            return Self::empty(width, height);
        }
        let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x, y) in points {
            (minx, miny, maxx, maxy) = (minx.min(x), miny.min(y), maxx.max(x), maxy.max(y));
        }
        let [bx0, by0, bx1, by1] = clamp_bounds(bounds_of(minx, miny, maxx, maxy), width, height);
        let mut s = Self::empty(width, height);
        if bx0 >= bx1 || by0 >= by1 {
            return s;
        }
        let bw = (bx1 - bx0) as usize;
        let mut row = vec![0f32; bw];
        let mut xs = Vec::new();
        for y in by0..by1 {
            row.iter_mut().for_each(|v| *v = 0.0);
            for sub in 0..4 {
                let sy = y as f32 + (sub as f32 + 0.5) / 4.0;
                xs.clear();
                for i in 0..points.len() {
                    let (ax, ay) = points[i];
                    let (bx, by) = points[(i + 1) % points.len()];
                    if (ay <= sy && by > sy) || (by <= sy && ay > sy) {
                        xs.push(ax + (sy - ay) / (by - ay) * (bx - ax));
                    }
                }
                xs.sort_by(f32::total_cmp);
                for pair in xs.chunks_exact(2) {
                    let (a, b) = (pair[0] - bx0 as f32, pair[1] - bx0 as f32);
                    let (ia, ib) = (a.floor().max(0.0) as usize, (b.ceil() as usize).min(bw));
                    for (px, v) in row.iter_mut().enumerate().take(ib).skip(ia) {
                        let c = (b.min(px as f32 + 1.0) - a.max(px as f32)).clamp(0.0, 1.0);
                        *v += c / 4.0;
                    }
                }
            }
            for (i, v) in row.iter().enumerate() {
                if *v > 0.0 {
                    s.set(bx0 + i as u32, y, (v.min(1.0) * 255.0 + 0.5) as u8);
                }
            }
        }
        s
    }

    /// Magic wand: pixels whose colour is within `tolerance` (0..255, per channel incl. alpha)
    /// of the clicked pixel. `contiguous` only grows from the click through matching pixels.
    pub fn magic_wand(
        width: u32,
        height: u32,
        x: u32,
        y: u32,
        tolerance: u8,
        contiguous: bool,
        sample: impl Fn(u32, u32) -> [u8; 4],
    ) -> Self {
        let mut s = Self::empty(width, height);
        if x >= width || y >= height {
            return s;
        }
        let seed = sample(x, y);
        let t = tolerance as i32;
        let matches = |p: [u8; 4]| (0..4).all(|i| (p[i] as i32 - seed[i] as i32).abs() <= t);
        if !contiguous {
            for yy in 0..height {
                for xx in 0..width {
                    if matches(sample(xx, yy)) {
                        s.set(xx, yy, 255);
                    }
                }
            }
            return s;
        }
        // Scanline flood fill.
        let mut seen = vec![false; width as usize * height as usize];
        let mut stack = vec![(x, y)];
        while let Some((sx, sy)) = stack.pop() {
            let idx = |x: u32| sy as usize * width as usize + x as usize;
            if seen[idx(sx)] || !matches(sample(sx, sy)) {
                continue;
            }
            let mut l = sx;
            while l > 0 && !seen[idx(l - 1)] && matches(sample(l - 1, sy)) {
                l -= 1;
            }
            let mut r = sx;
            while r + 1 < width && !seen[idx(r + 1)] && matches(sample(r + 1, sy)) {
                r += 1;
            }
            for xx in l..=r {
                seen[idx(xx)] = true;
                s.set(xx, sy, 255);
                if sy > 0 && !seen[(sy as usize - 1) * width as usize + xx as usize] {
                    stack.push((xx, sy - 1));
                }
                if sy + 1 < height && !seen[(sy as usize + 1) * width as usize + xx as usize] {
                    stack.push((xx, sy + 1));
                }
            }
        }
        s
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn get(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        match &self.tiles[((y / TILE_SIZE) * self.cols + x / TILE_SIZE) as usize] {
            Some(t) => t[((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize],
            None => 0,
        }
    }

    pub fn set(&mut self, x: u32, y: u32, v: u8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let slot = &mut self.tiles[((y / TILE_SIZE) * self.cols + x / TILE_SIZE) as usize];
        if slot.is_none() && v == 0 {
            return;
        }
        let tile = slot.get_or_insert_with(|| vec![0u8; TILE_LEN].into());
        let i = ((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) as usize;
        if tile[i] != v {
            Arc::make_mut(tile)[i] = v;
            self.id = next_id();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.iter().all(|t| t.as_ref().is_none_or(|t| t.iter().all(|&v| v == 0)))
    }

    /// Tight bounds of selected pixels: x0, y0, x1, y1 (exclusive).
    pub fn bounds(&self) -> Option<[u32; 4]> {
        let mut b: Option<[u32; 4]> = None;
        for ty in 0..self.rows {
            for tx in 0..self.cols {
                let Some(t) = &self.tiles[(ty * self.cols + tx) as usize] else { continue };
                for ly in 0..TILE_SIZE {
                    for lx in 0..TILE_SIZE {
                        if t[(ly * TILE_SIZE + lx) as usize] == 0 {
                            continue;
                        }
                        let (x, y) = (tx * TILE_SIZE + lx, ty * TILE_SIZE + ly);
                        b = Some(match b {
                            None => [x, y, x + 1, y + 1],
                            Some([a, c, d, e]) => [a.min(x), c.min(y), d.max(x + 1), e.max(y + 1)],
                        });
                    }
                }
            }
        }
        b
    }

    pub fn invert(&mut self) {
        self.id = next_id();
        let (w, h, cols) = (self.width, self.height, self.cols);
        for (i, slot) in self.tiles.iter_mut().enumerate() {
            let (tx, ty) = (i as u32 % cols, i as u32 / cols);
            let partial = (tx + 1) * TILE_SIZE > w || (ty + 1) * TILE_SIZE > h;
            *slot = match slot.take() {
                None if !partial => Some(FULL.clone()),
                Some(t) if Arc::ptr_eq(&t, &FULL) => None,
                other => {
                    let mut buf: Vec<u8> = other.map_or_else(|| vec![0; TILE_LEN], |t| t.to_vec());
                    for (j, v) in buf.iter_mut().enumerate() {
                        let (x, y) = (tx * TILE_SIZE + j as u32 % TILE_SIZE, ty * TILE_SIZE + j as u32 / TILE_SIZE);
                        *v = if x < w && y < h { 255 - *v } else { 0 };
                    }
                    Some(buf.into())
                }
            };
        }
    }

    /// Combines `other` into `self`.
    pub fn combine(&mut self, other: &Selection, op: Combine) {
        if op == Combine::Replace {
            *self = other.clone();
            return;
        }
        self.id = next_id();
        for i in 0..self.tiles.len() {
            let a = self.tiles[i].clone();
            let b = other.tiles.get(i).cloned().flatten();
            self.tiles[i] = match (op, a, b) {
                (Combine::Add, None, b) | (Combine::Add, b, None) => b,
                (Combine::Subtract, a, None) => a,
                (Combine::Subtract, None, _) | (Combine::Intersect, None, _) | (Combine::Intersect, _, None) => None,
                (_, Some(a), Some(b)) => {
                    let out: Vec<u8> = a
                        .iter()
                        .zip(b.iter())
                        .map(|(&a, &b)| {
                            let (a, b) = (a as u32, b as u32);
                            (match op {
                                Combine::Add => a + b - a * b / 255,
                                Combine::Subtract => a * (255 - b) / 255,
                                _ => a * b / 255,
                            }) as u8
                        })
                        .collect();
                    if out.iter().all(|&v| v == 0) { None } else { Some(out.into()) }
                }
                (Combine::Replace, ..) => unreachable!(),
            };
        }
    }

    /// Shifts the selection by whole pixels (what Move does to it).
    pub fn translated(&self, dx: i32, dy: i32) -> Self {
        let mut out = Self::empty(self.width, self.height);
        let Some([x0, y0, x1, y1]) = self.bounds() else { return out };
        for y in y0..y1 {
            for x in x0..x1 {
                let v = self.get(x, y);
                let (nx, ny) = (x as i64 + dx as i64, y as i64 + dy as i64);
                if v != 0 && nx >= 0 && ny >= 0 {
                    out.set(nx as u32, ny as u32, v);
                }
            }
        }
        out
    }

    /// The outline between selected (>= 50%) and unselected pixels, as merged horizontal and
    /// vertical runs along pixel edges. This is what marching ants trace.
    pub fn outline(&self) -> Vec<Segment> {
        let inside = |x: i64, y: i64| x >= 0 && y >= 0 && self.get(x as u32, y as u32) >= 128;
        let mut segs = Vec::new();
        let Some([x0, y0, x1, y1]) = self.bounds() else { return segs };
        let (x0, y0, x1, y1) = (x0 as i64, y0 as i64, x1 as i64, y1 as i64);
        // Horizontal edges: between row y-1 and y.
        for y in y0..=y1 {
            let mut start: Option<i64> = None;
            for x in x0..=x1 {
                let edge = x < x1 && inside(x, y - 1) != inside(x, y);
                match (edge, start) {
                    (true, None) => start = Some(x),
                    (false, Some(s)) => {
                        segs.push([s as u32, y as u32, x as u32, y as u32]);
                        start = None;
                    }
                    _ => {}
                }
            }
        }
        // Vertical edges: between column x-1 and x.
        for x in x0..=x1 {
            let mut start: Option<i64> = None;
            for y in y0..=y1 {
                let edge = y < y1 && inside(x - 1, y) != inside(x, y);
                match (edge, start) {
                    (true, None) => start = Some(y),
                    (false, Some(s)) => {
                        segs.push([x as u32, s as u32, x as u32, y as u32]);
                        start = None;
                    }
                    _ => {}
                }
            }
        }
        segs
    }

    /// Coverage of a whole layer-sized area as a fast check: true if nothing is selected in
    /// the given tile.
    pub fn tile_is_empty(&self, tx: u32, ty: u32) -> bool {
        tx >= self.cols || ty >= self.rows || self.tiles[(ty * self.cols + tx) as usize].is_none()
    }
}

fn bounds_of(x0: f32, y0: f32, x1: f32, y1: f32) -> [u32; 4] {
    [
        x0.min(x1).floor().max(0.0) as u32,
        y0.min(y1).floor().max(0.0) as u32,
        x0.max(x1).ceil().max(0.0) as u32,
        y0.max(y1).ceil().max(0.0) as u32,
    ]
}

fn clamp_bounds(b: [u32; 4], w: u32, h: u32) -> [u32; 4] {
    [b[0].min(w), b[1].min(h), b[2].min(w), b[3].min(h)]
}

/// Copies the selected part of `pixels` into a new grid (straight alpha scaled by coverage).
pub fn extract(pixels: &TileGrid, sel: &Selection) -> TileGrid {
    let mut out = TileGrid::empty(pixels.width(), pixels.height());
    let Some([x0, y0, x1, y1]) = sel.bounds() else { return out };
    for y in y0..y1 {
        for x in x0..x1 {
            let m = sel.get(x, y);
            if m == 0 {
                continue;
            }
            let p = pixels.pixel(x, y);
            if p[3] == 0 {
                continue;
            }
            let a = (p[3] as u32 * m as u32 + 127) / 255;
            out.set_pixel(x, y, [p[0], p[1], p[2], a as u8]);
        }
    }
    out
}

/// Removes the selected part of `pixels` (alpha reduced by coverage).
pub fn clear(pixels: &mut TileGrid, sel: &Selection) {
    let Some([x0, y0, x1, y1]) = sel.bounds() else { return };
    for y in y0..y1 {
        for x in x0..x1 {
            let m = sel.get(x, y);
            if m == 0 {
                continue;
            }
            let p = pixels.pixel(x, y);
            if p[3] == 0 {
                continue;
            }
            let a = (p[3] as u32 * (255 - m as u32) + 127) / 255;
            pixels.set_pixel(x, y, if a == 0 { [0; 4] } else { [p[0], p[1], p[2], a as u8] });
        }
    }
}

/// Fills the selected part of `pixels` with an sRGB colour (normal blending, by coverage).
pub fn fill(pixels: &mut TileGrid, sel: &Selection, color: [u8; 3]) {
    let solid = [color[0], color[1], color[2], 255];
    for ty in 0..sel.rows {
        for tx in 0..sel.cols {
            let Some(mask) = &sel.tiles[(ty * sel.cols + tx) as usize] else { continue };
            // Fully selected tile: write the whole tile at once (the common Select All case).
            if Arc::ptr_eq(mask, &FULL) {
                if let Some(tile) = pixels.tile_mut(tx, ty) {
                    for px in tile.data_mut().chunks_exact_mut(4) {
                        px.copy_from_slice(&solid);
                    }
                }
                continue;
            }
            for ly in 0..TILE_SIZE {
                for lx in 0..TILE_SIZE {
                    let m = mask[(ly * TILE_SIZE + lx) as usize];
                    if m == 0 {
                        continue;
                    }
                    let (x, y) = (tx * TILE_SIZE + lx, ty * TILE_SIZE + ly);
                    let p = pixels.pixel(x, y);
                    pixels.set_pixel(x, y, over_srgb8(p, [color[0], color[1], color[2], m]));
                }
            }
        }
    }
}

/// Straight-alpha "over" on sRGB bytes.
pub fn over_srgb8(dst: [u8; 4], src: [u8; 4]) -> [u8; 4] {
    let sa = src[3] as f32 / 255.0;
    if sa >= 1.0 {
        return src;
    }
    let da = dst[3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    if oa <= 0.0 {
        return [0; 4];
    }
    let c = |s: u8, d: u8| ((s as f32 * sa + d as f32 * da * (1.0 - sa)) / oa + 0.5) as u8;
    [c(src[0], dst[0]), c(src[1], dst[1]), c(src[2], dst[2]), (oa * 255.0 + 0.5) as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_has_soft_fractional_edges() {
        let s = Selection::rect(100, 100, 10.0, 10.0, 20.5, 20.0);
        assert_eq!(s.get(15, 15), 255);
        assert_eq!(s.get(20, 15), 128);
        assert_eq!(s.get(21, 15), 0);
        assert_eq!(s.bounds(), Some([10, 10, 21, 20]));
    }

    #[test]
    fn select_all_shares_tiles_and_invert_empties_it() {
        let mut s = Selection::all(600, 300);
        assert_eq!(s.get(599, 299), 255);
        assert_eq!(s.bounds(), Some([0, 0, 600, 300]));
        s.invert();
        assert!(s.is_empty());
        s.invert();
        assert_eq!(s.get(599, 299), 255);
        assert_eq!(s.get(600, 299), 0);
    }

    #[test]
    fn ellipse_and_polygon_cover_their_shapes() {
        let e = Selection::ellipse(100, 100, 10.0, 10.0, 50.0, 30.0);
        assert_eq!(e.get(30, 20), 255);
        assert_eq!(e.get(11, 11), 0);
        let tri = Selection::polygon(100, 100, &[(10.0, 10.0), (90.0, 10.0), (10.0, 90.0)]);
        assert_eq!(tri.get(20, 20), 255);
        assert_eq!(tri.get(80, 80), 0);
        // On the diagonal edge: partly covered.
        let edge = tri.get(50, 49);
        assert!(edge > 0 && edge < 255, "{edge}");
    }

    #[test]
    fn combine_modes() {
        let a = Selection::rect(100, 100, 0.0, 0.0, 50.0, 50.0);
        let b = Selection::rect(100, 100, 25.0, 25.0, 75.0, 75.0);
        let mut add = a.clone();
        add.combine(&b, Combine::Add);
        assert_eq!((add.get(10, 10), add.get(60, 60)), (255, 255));
        let mut sub = a.clone();
        sub.combine(&b, Combine::Subtract);
        assert_eq!((sub.get(10, 10), sub.get(30, 30)), (255, 0));
        let mut int = a.clone();
        int.combine(&b, Combine::Intersect);
        assert_eq!((int.get(10, 10), int.get(30, 30)), (0, 255));
    }

    #[test]
    fn magic_wand_contiguous_vs_global() {
        // Two red squares separated by blue.
        let px = |x: u32, _y: u32| if (10..20).contains(&x) || (40..50).contains(&x) { [255, 0, 0, 255] } else { [0, 0, 255, 255] };
        let c = Selection::magic_wand(60, 10, 12, 5, 10, true, px);
        assert_eq!((c.get(15, 5), c.get(45, 5), c.get(30, 5)), (255, 0, 0));
        let g = Selection::magic_wand(60, 10, 12, 5, 10, false, px);
        assert_eq!((g.get(15, 5), g.get(45, 5)), (255, 255));
    }

    #[test]
    fn outline_of_a_square_is_four_runs() {
        let s = Selection::rect(50, 50, 10.0, 10.0, 20.0, 20.0);
        let mut o = s.outline();
        o.sort();
        assert_eq!(o, [[10, 10, 10, 20], [10, 10, 20, 10], [10, 20, 20, 20], [20, 10, 20, 20]]);
    }

    #[test]
    fn extract_clear_fill() {
        let mut px = TileGrid::from_rgba8(4, 1, &[10, 10, 10, 255].repeat(4));
        let sel = Selection::rect(4, 1, 1.0, 0.0, 3.0, 1.0);
        let cut = extract(&px, &sel);
        assert_eq!((cut.pixel(0, 0)[3], cut.pixel(1, 0)), (0, [10, 10, 10, 255]));
        clear(&mut px, &sel);
        assert_eq!((px.pixel(0, 0)[3], px.pixel(1, 0)[3]), (255, 0));
        fill(&mut px, &sel, [200, 0, 0]);
        assert_eq!(px.pixel(2, 0), [200, 0, 0, 255]);
    }

    #[test]
    fn id_changes_with_contents_and_survives_clone() {
        let mut s = Selection::rect(50, 50, 0.0, 0.0, 5.0, 5.0);
        let id = s.id();
        assert_eq!(s.clone().id(), id);
        s.invert();
        assert_ne!(s.id(), id);
    }

    #[test]
    fn translate_moves_pixels() {
        let s = Selection::rect(50, 50, 0.0, 0.0, 5.0, 5.0).translated(10, 3);
        assert_eq!(s.bounds(), Some([10, 3, 15, 8]));
    }
}
