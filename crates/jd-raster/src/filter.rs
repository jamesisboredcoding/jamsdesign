//! Filters that look at neighbouring pixels (blur, sharpen, mosaic) and noise.
//!
//! Neighbourhood filters copy the affected area (plus a margin) into a dense premultiplied
//! float buffer, process it, and write back only where the selection allows. Premultiplied
//! maths means transparent areas don't bleed dark colour into edges.

use rayon::prelude::*;

use crate::adjust::map_pixels_xy;
use crate::blend::dissolve_noise;
use crate::selection::Selection;
use crate::transform::content_bounds;
use crate::{TILE_SIZE, TileGrid};

#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    /// Radius ~ standard deviation in pixels, like Photoshop's.
    GaussianBlur { radius: f32 },
    /// amount in percent (1..500), radius in pixels, threshold in levels (0..255).
    UnsharpMask { amount: f32, radius: f32, threshold: u8 },
    /// amount in percent (0..100); `monochrome` uses the same noise for all channels.
    AddNoise { amount: f32, monochrome: bool, seed: u32 },
    /// Square cells of this many pixels.
    Mosaic { cell: u32 },
    /// Plain average over a square of this radius.
    BoxBlur { radius: f32 },
    /// Keeps detail finer than `radius`, flattening the rest to grey (for sharpening/texture).
    HighPass { radius: f32 },
    /// One-step sharpening (a fixed, gentle Unsharp Mask), like Photoshop's Filter > Sharpen.
    Sharpen,
}

impl Filter {
    pub fn name(&self) -> &'static str {
        match self {
            Self::GaussianBlur { .. } => "Gaussian Blur",
            Self::UnsharpMask { .. } => "Unsharp Mask",
            Self::AddNoise { .. } => "Add Noise",
            Self::Mosaic { .. } => "Mosaic",
            Self::BoxBlur { .. } => "Box Blur",
            Self::HighPass { .. } => "High Pass",
            Self::Sharpen => "Sharpen",
        }
    }

    /// How far a pixel's result can depend on its neighbours.
    fn margin(&self) -> u32 {
        match *self {
            Self::GaussianBlur { radius } | Self::UnsharpMask { radius, .. } | Self::HighPass { radius } => (radius * 3.0).ceil() as u32 + 1,
            Self::BoxBlur { radius } => radius.ceil() as u32 + 1,
            Self::Mosaic { cell } => cell,
            Self::AddNoise { .. } => 0,
            Self::Sharpen => 4,
        }
    }
}

pub fn apply(grid: &TileGrid, filter: &Filter, sel: Option<&Selection>) -> TileGrid {
    if *filter == Filter::Sharpen {
        return apply(grid, &Filter::UnsharpMask { amount: 60.0, radius: 1.0, threshold: 0 }, sel);
    }
    if let Filter::AddNoise { amount, monochrome, seed } = *filter {
        let a = amount.clamp(0.0, 400.0) / 100.0 * 127.0;
        return map_pixels_xy(grid, sel, move |x, y, p| {
            let n = |c: u32| (dissolve_noise(x, y, seed.wrapping_add(c * 7919)) * 2.0 - 1.0) * a;
            let shared = n(0);
            let mut out = p;
            for c in 0..3 {
                let v = p[c] as f32 + if monochrome { shared } else { n(c as u32) };
                out[c] = v.round().clamp(0.0, 255.0) as u8;
            }
            out
        });
    }

    let (w, h) = (grid.width(), grid.height());
    // Output area: the selection (or the content, grown by the filter's reach).
    let margin = filter.margin();
    let grow = |b: [u32; 4], m: u32| [b[0].saturating_sub(m), b[1].saturating_sub(m), (b[2] + m).min(w), (b[3] + m).min(h)];
    let Some(content) = content_bounds(grid) else { return grid.clone() };
    let out_box = match sel {
        Some(s) => match s.bounds() {
            Some(b) => b,
            None => return grid.clone(),
        },
        None => grow(content, margin),
    };
    let region = grow(out_box, margin);
    let [rx0, ry0, rx1, ry1] = region;
    let (rw, rh) = ((rx1 - rx0) as usize, (ry1 - ry0) as usize);
    if rw == 0 || rh == 0 {
        return grid.clone();
    }

    // Dense premultiplied copy of the region, 0..1 (row by row from the tiles).
    let mut buf = vec![[0f32; 4]; rw * rh];
    buf.par_chunks_mut(rw).enumerate().for_each(|(row, out)| {
        let y = ry0 + row as u32;
        let (ty, ly) = (y / TILE_SIZE, y % TILE_SIZE);
        let mut x = rx0;
        while x < rx1 {
            let tx = x / TILE_SIZE;
            let end = ((tx + 1) * TILE_SIZE).min(rx1);
            if let Some(tile) = grid.tile(tx, ty) {
                let data = tile.data();
                for xx in x..end {
                    let i = ((ly * TILE_SIZE + xx % TILE_SIZE) * 4) as usize;
                    let a = data[i + 3] as f32 / 255.0;
                    let k = a / 255.0;
                    out[(xx - rx0) as usize] = [data[i] as f32 * k, data[i + 1] as f32 * k, data[i + 2] as f32 * k, a];
                }
            }
            x = end;
        }
    });
    // Only needed to mix by partial selection coverage.
    let original = if sel.is_some() { buf.clone() } else { Vec::new() };

    match *filter {
        Filter::GaussianBlur { radius } => gaussian(&mut buf, rw, rh, radius),
        Filter::UnsharpMask { amount, radius, threshold } => {
            let mut blurred = buf.clone();
            gaussian(&mut blurred, rw, rh, radius);
            let (k, t) = (amount / 100.0, threshold as f32 / 255.0);
            buf.par_iter_mut().zip(blurred.par_iter()).for_each(|(o, b)| {
                if o[3] <= 0.0 {
                    return;
                }
                let a = o[3];
                let bu = if b[3] > 0.0 { [b[0] / b[3], b[1] / b[3], b[2] / b[3]] } else { [0.0; 3] };
                let ou = [o[0] / a, o[1] / a, o[2] / a];
                if (0..3).map(|c| (ou[c] - bu[c]).abs()).fold(0.0, f32::max) < t {
                    return;
                }
                for c in 0..3 {
                    o[c] = (ou[c] + k * (ou[c] - bu[c])).clamp(0.0, 1.0) * a;
                }
            });
        }
        Filter::Mosaic { cell } => mosaic(&mut buf, rw, rh, rx0, ry0, cell.max(1)),
        Filter::BoxBlur { radius } => {
            let r = radius.round().max(0.0) as usize;
            box_rows(&mut buf, rw, r);
            columns(&mut buf, rw, rh, |col| box_line(col, r));
        }
        Filter::HighPass { radius } => {
            let mut blurred = buf.clone();
            gaussian(&mut blurred, rw, rh, radius);
            buf.par_iter_mut().zip(blurred.par_iter()).for_each(|(o, b)| {
                if o[3] <= 0.0 {
                    return;
                }
                let a = o[3];
                for c in 0..3 {
                    let bu = if b[3] > 0.0 { b[c] / b[3] } else { 0.0 };
                    o[c] = (0.5 + o[c] / a - bu).clamp(0.0, 1.0) * a;
                }
            });
        }
        Filter::AddNoise { .. } | Filter::Sharpen => unreachable!("handled above"),
    }

    // Write back the output box, mixed with the original by selection coverage.
    let [ox0, oy0, ox1, oy1] = out_box;
    let ow = (ox1 - ox0) as usize;
    let rows: Vec<Vec<u8>> = (oy0..oy1)
        .into_par_iter()
        .map(|y| {
            let mut row = vec![0u8; ow * 4];
            for (i, x) in (ox0..ox1).enumerate() {
                let j = (y - ry0) as usize * rw + (x - rx0) as usize;
                let mut v = buf[j];
                if let Some(s) = sel {
                    let m = s.get(x, y) as f32 / 255.0;
                    let o = original[j];
                    for c in 0..4 {
                        v[c] = v[c] * m + o[c] * (1.0 - m);
                    }
                }
                if v[3] > 0.0 {
                    let a = v[3].min(1.0);
                    for c in 0..3 {
                        row[i * 4 + c] = ((v[c] / v[3]).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    }
                    row[i * 4 + 3] = (a * 255.0 + 0.5) as u8;
                }
            }
            row
        })
        .collect();
    let mut out = grid.clone();
    out.write_region(ox0, oy0, ow as u32, oy1 - oy0, &rows.concat());
    out
}

/// Gaussian blur of standard deviation `sigma`. Small radii use an exact kernel; larger ones
/// three box blurs (O(1) per pixel, within a few percent of a true Gaussian).
fn gaussian(buf: &mut [[f32; 4]], w: usize, h: usize, sigma: f32) {
    if sigma < 0.2 {
        return;
    }
    if sigma < 3.0 {
        let r = (sigma * 3.0).ceil() as usize;
        let mut k: Vec<f32> = (0..=2 * r).map(|i| (-((i as f32 - r as f32).powi(2)) / (2.0 * sigma * sigma)).exp()).collect();
        let sum: f32 = k.iter().sum();
        k.iter_mut().for_each(|v| *v /= sum);
        kernel_rows(buf, w, &k);
        columns(buf, w, h, |col| {
            let src = col.to_vec();
            convolve(&src, col, &k);
        });
        return;
    }
    // Boxes are separable: all horizontal passes, then all vertical passes per column strip.
    let radii = box_radii(sigma);
    for &r in &radii {
        box_rows(buf, w, r);
    }
    columns(buf, w, h, |col| {
        for &r in &radii {
            box_line(col, r);
        }
    });
}

/// Runs `f` on every column (as a contiguous copy), processing strips of columns in parallel
/// so memory is read row-wise and cache-friendly. Avoids transposing the whole image.
fn columns(buf: &mut [[f32; 4]], w: usize, h: usize, f: impl Fn(&mut [[f32; 4]]) + Sync) {
    const STRIP: usize = 16;
    let results: Vec<(usize, Vec<[f32; 4]>)> = (0..w.div_ceil(STRIP))
        .into_par_iter()
        .map(|s| {
            let x0 = s * STRIP;
            let sw = STRIP.min(w - x0);
            // Strip copy, column-major: column dx occupies [dx*h .. dx*h+h).
            let mut strip = vec![[0f32; 4]; sw * h];
            for y in 0..h {
                let row = &buf[y * w + x0..y * w + x0 + sw];
                for (dx, v) in row.iter().enumerate() {
                    strip[dx * h + y] = *v;
                }
            }
            for col in strip.chunks_mut(h) {
                f(col);
            }
            (x0, strip)
        })
        .collect();
    for (x0, strip) in results {
        let sw = strip.len() / h;
        for y in 0..h {
            for dx in 0..sw {
                buf[y * w + x0 + dx] = strip[dx * h + y];
            }
        }
    }
}

/// Symmetric convolution of one line; outside counts as transparent.
fn convolve(src: &[[f32; 4]], out: &mut [[f32; 4]], k: &[f32]) {
    let (r, n) = (k.len() / 2, src.len());
    for (x, o) in out.iter_mut().enumerate() {
        let mut acc = [0f32; 4];
        for (i, kv) in k.iter().enumerate() {
            let sx = x as isize + i as isize - r as isize;
            if sx < 0 || sx >= n as isize {
                continue;
            }
            for c in 0..4 {
                acc[c] += src[sx as usize][c] * kv;
            }
        }
        *o = acc;
    }
}

/// Box radii whose three passes match a Gaussian (Kovesi's method).
fn box_radii(sigma: f32) -> [usize; 3] {
    let n = 3.0;
    let ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut wl = ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let m = ((12.0 * sigma * sigma - n * (wl * wl) as f32 - 4.0 * n * wl as f32 - 3.0 * n) / (-4.0 * wl as f32 - 4.0)).round() as i32;
    std::array::from_fn(|i| (((if (i as i32) < m { wl } else { wu }) - 1) / 2).max(0) as usize)
}

/// Horizontal box blur of radius `r` on every row.
fn box_rows(buf: &mut [[f32; 4]], w: usize, r: usize) {
    buf.par_chunks_mut(w).for_each(|row| box_line(row, r));
}

/// Box blur of one line with a running sum; outside counts as transparent.
fn box_line(line: &mut [[f32; 4]], r: usize) {
    if r == 0 {
        return;
    }
    let n = line.len();
    let norm = 1.0 / (2 * r + 1) as f32;
    let src = line.to_vec();
    let mut acc = [0f32; 4];
    for px in src.iter().take(r.min(n)) {
        for c in 0..4 {
            acc[c] += px[c];
        }
    }
    for x in 0..n {
        if x + r < n {
            for c in 0..4 {
                acc[c] += src[x + r][c];
            }
        }
        if x > r {
            for c in 0..4 {
                acc[c] -= src[x - r - 1][c];
            }
        }
        for c in 0..4 {
            line[x][c] = acc[c] * norm;
        }
    }
}

/// Convolves every row with a symmetric kernel (odd length).
fn kernel_rows(buf: &mut [[f32; 4]], w: usize, k: &[f32]) {
    buf.par_chunks_mut(w).for_each(|row| {
        let src = row.to_vec();
        convolve(&src, row, k);
    });
}

/// Averages square cells aligned to the document origin (so results don't depend on the region).
fn mosaic(buf: &mut [[f32; 4]], w: usize, h: usize, rx0: u32, ry0: u32, cell: u32) {
    let cell = cell as usize;
    let (ox, oy) = (rx0 as usize % cell, ry0 as usize % cell);
    let mut y0 = 0;
    let mut first_y = true;
    while y0 < h {
        let ch = if first_y { cell - oy } else { cell }.min(h - y0);
        let mut x0 = 0;
        let mut first_x = true;
        while x0 < w {
            let cw = if first_x { cell - ox } else { cell }.min(w - x0);
            let mut acc = [0f32; 4];
            for y in y0..y0 + ch {
                for x in x0..x0 + cw {
                    for c in 0..4 {
                        acc[c] += buf[y * w + x][c];
                    }
                }
            }
            let n = (cw * ch) as f32;
            let avg = acc.map(|v| v / n);
            for y in y0..y0 + ch {
                for x in x0..x0 + cw {
                    buf[y * w + x] = avg;
                }
            }
            x0 += cw;
            first_x = false;
        }
        y0 += ch;
        first_y = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dot(w: u32, h: u32) -> TileGrid {
        let mut g = TileGrid::empty(w, h);
        for y in 20..30 {
            for x in 20..30 {
                g.set_pixel(x, y, [255, 0, 0, 255]);
            }
        }
        g
    }

    #[test]
    fn box_radii_cover_the_gaussian() {
        // Variance of three boxes ~= sigma^2 (boxes are only used from sigma 3 up).
        for sigma in [3.0f32, 10.0, 40.0] {
            let v: f32 = box_radii(sigma).iter().map(|&r| (((2 * r + 1).pow(2) - 1) as f32) / 12.0).sum();
            assert!((v.sqrt() - sigma).abs() / sigma < 0.15, "{sigma}: {}", v.sqrt());
        }
    }

    #[test]
    fn blur_spreads_alpha_without_darkening_colour() {
        let out = apply(&dot(60, 60), &Filter::GaussianBlur { radius: 3.0 }, None);
        let edge = out.pixel(18, 25);
        assert!(edge[3] > 0 && edge[3] < 255, "{edge:?}");
        assert_eq!(&edge[..3], &[255, 0, 0], "premultiplied blur keeps the colour");
        let centre = out.pixel(25, 25);
        assert!(centre[3] > 200);
        // Total coverage is (roughly) preserved.
        let sum = |g: &TileGrid| (0..60).flat_map(|y| (0..60).map(move |x| (x, y))).map(|(x, y)| g.pixel(x, y)[3] as u32).sum::<u32>();
        let (a, b) = (sum(&dot(60, 60)), sum(&out));
        assert!((a as i64 - b as i64).abs() < a as i64 / 50, "{a} vs {b}");
    }

    #[test]
    fn selection_limits_blur() {
        let sel = Selection::rect(60, 60, 0.0, 0.0, 25.0, 60.0);
        let out = apply(&dot(60, 60), &Filter::GaussianBlur { radius: 3.0 }, Some(&sel));
        assert!(out.pixel(18, 25)[3] > 0, "left edge (selected) blurred");
        assert_eq!(out.pixel(31, 25)[3], 0, "right edge (unselected) untouched");
    }

    #[test]
    fn mosaic_cells_are_flat_and_aligned() {
        let px: Vec<u8> = (0..32 * 32).flat_map(|i| [(i % 32 * 8) as u8, 0, 0, 255]).collect();
        let out = apply(&TileGrid::from_rgba8(32, 32, &px), &Filter::Mosaic { cell: 8 }, None);
        assert_eq!(out.pixel(0, 0), out.pixel(7, 7));
        assert_ne!(out.pixel(7, 0), out.pixel(8, 0));
    }

    #[test]
    fn unsharp_increases_edge_contrast() {
        let px: Vec<u8> = (0..40 * 10).flat_map(|i| if i % 40 < 20 { [80, 80, 80, 255] } else { [160, 160, 160, 255] }).collect();
        let g = TileGrid::from_rgba8(40, 10, &px);
        let out = apply(&g, &Filter::UnsharpMask { amount: 150.0, radius: 2.0, threshold: 0 }, None);
        assert!(out.pixel(19, 5)[0] < 80 && out.pixel(20, 5)[0] > 160);
        assert_eq!(out.pixel(2, 5)[0], 80, "flat areas unchanged");
    }

    #[test]
    fn box_blur_and_high_pass() {
        let out = apply(&dot(60, 60), &Filter::BoxBlur { radius: 2.0 }, None);
        assert!(out.pixel(18, 25)[3] > 0 && out.pixel(17, 25)[3] == 0);
        let flat = TileGrid::from_rgba8(30, 30, &[90, 90, 90, 255].repeat(900));
        let hp = apply(&flat, &Filter::HighPass { radius: 3.0 }, None);
        let p = hp.pixel(15, 15);
        assert!((126..=129).contains(&p[0]), "flat areas become mid grey: {p:?}");
    }

    #[test]
    fn noise_is_deterministic_and_keeps_alpha() {
        let g = TileGrid::from_rgba8(8, 8, &[128, 128, 128, 200].repeat(64));
        let f = Filter::AddNoise { amount: 20.0, monochrome: true, seed: 3 };
        let (a, b) = (apply(&g, &f, None), apply(&g, &f, None));
        assert_eq!(a.pixel(3, 4), b.pixel(3, 4));
        let p = a.pixel(3, 4);
        assert!(p[0] == p[1] && p[1] == p[2] && p[3] == 200);
    }
}
