//! CPU mip generation for RGBA8 sRGB tiles.
//!
//! Averages in linear light with alpha weighting (premultiply → box filter → unpremultiply), so
//! zoomed-out views don't darken or grow dark fringes around transparent edges.

use std::sync::LazyLock;

/// Smallest mip we bother making. Below this, a tile is a handful of screen pixels anyway.
const MIN_SIZE: u32 = 8;

static TO_LINEAR: LazyLock<[f32; 256]> = LazyLock::new(|| {
    std::array::from_fn(|i| {
        let v = i as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    })
});

/// Linear → sRGB8 through a 4096-entry table: tiles are re-mipped on every brush update, and
/// `powf` per pixel dominated that. Max error vs exact is under one 8-bit level.
static TO_SRGB8: LazyLock<[u8; 4096]> = LazyLock::new(|| {
    std::array::from_fn(|i| {
        let v = i as f32 / 4095.0;
        let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0 + 0.5) as u8
    })
});

fn to_srgb8(v: f32) -> u8 {
    TO_SRGB8[(v.clamp(0.0, 1.0) * 4095.0 + 0.5) as usize]
}

/// Copies the last valid column rightwards and the last valid row downwards, filling the
/// part of an edge tile that lies outside the document.
pub fn pad_edges(src: &[u8], size: u32, valid: [u32; 2]) -> Vec<u8> {
    let (size, vw, vh) = (size as usize, valid[0].max(1) as usize, valid[1].max(1) as usize);
    let mut out = src.to_vec();
    for y in 0..vh {
        let row = y * size * 4;
        let last: [u8; 4] = out[row + (vw - 1) * 4..row + vw * 4].try_into().expect("4 bytes");
        for x in vw..size {
            out[row + x * 4..row + x * 4 + 4].copy_from_slice(&last);
        }
    }
    let last_row = out[(vh - 1) * size * 4..vh * size * 4].to_vec();
    for y in vh..size {
        out[y * size * 4..(y + 1) * size * 4].copy_from_slice(&last_row);
    }
    out
}

/// Returns mip levels 1.. (level 0 is the input), halving until `MIN_SIZE`.
pub fn chain(base: &[u8], size: u32) -> Vec<Vec<u8>> {
    let mut levels: Vec<Vec<u8>> = Vec::new();
    let mut size = size;
    while size / 2 >= MIN_SIZE {
        let src = levels.last().map_or(base, |v| v.as_slice());
        levels.push(downsample(src, size));
        size /= 2;
    }
    levels
}

fn downsample(src: &[u8], size: u32) -> Vec<u8> {
    let lut = &*TO_LINEAR;
    let half = size / 2;
    let mut out = vec![0u8; (half * half * 4) as usize];
    for y in 0..half {
        for x in 0..half {
            let mut acc = [0.0f32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let i = (((y * 2 + dy) * size + x * 2 + dx) * 4) as usize;
                let a = src[i + 3] as f32 / 255.0;
                acc[0] += lut[src[i] as usize] * a;
                acc[1] += lut[src[i + 1] as usize] * a;
                acc[2] += lut[src[i + 2] as usize] * a;
                acc[3] += a;
            }
            let o = ((y * half + x) * 4) as usize;
            if acc[3] > 0.0 {
                out[o] = to_srgb8(acc[0] / acc[3]);
                out[o + 1] = to_srgb8(acc[1] / acc[3]);
                out[o + 2] = to_srgb8(acc[2] / acc[3]);
            }
            out[o + 3] = (acc[3] / 4.0 * 255.0 + 0.5) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_sizes() {
        let base = vec![255u8; 256 * 256 * 4];
        let levels = chain(&base, 256);
        let sizes: Vec<usize> = levels.iter().map(|l| l.len() / 4).collect();
        assert_eq!(sizes, [128 * 128, 64 * 64, 32 * 32, 16 * 16, 8 * 8]);
        assert!(levels.iter().all(|l| l.iter().all(|&b| b == 255)));
    }

    #[test]
    fn black_white_averages_in_linear_light() {
        // 2×2 checker of black and white → linear 0.5 → sRGB ~188, not 128.
        let px = [0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255];
        let out = downsample(&px, 2);
        assert_eq!(out[3], 255);
        assert!((186..=190).contains(&out[0]), "got {}", out[0]);
    }

    #[test]
    fn srgb_table_matches_exact_within_one_level() {
        for i in 0..=1000 {
            let v = i as f32 / 1000.0;
            let exact = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
            let exact = (exact * 255.0 + 0.5) as i32;
            assert!((to_srgb8(v) as i32 - exact).abs() <= 1, "{v}");
        }
    }

    #[test]
    fn pad_edges_replicates_last_row_and_column() {
        // 4×4 tile, valid 2×1: [A B . .] → every pixel right/below copies its nearest valid one.
        let mut px = vec![0u8; 4 * 4 * 4];
        px[..4].copy_from_slice(&[1, 1, 1, 255]);
        px[4..8].copy_from_slice(&[2, 2, 2, 255]);
        let out = pad_edges(&px, 4, [2, 1]);
        let at = |x: usize, y: usize| out[(y * 4 + x) * 4];
        assert_eq!([at(0, 0), at(1, 0), at(3, 0), at(0, 3), at(3, 3)], [1, 2, 2, 1, 2]);
    }

    #[test]
    fn transparent_pixels_dont_tint() {
        // red opaque + transparent black → red at half alpha, not dark red.
        let px = [255, 0, 0, 255, 0, 0, 0, 0, 255, 0, 0, 255, 0, 0, 0, 0];
        let out = downsample(&px, 2);
        assert_eq!(&out[..3], &[255, 0, 0]);
        assert_eq!(out[3], 128);
    }
}
