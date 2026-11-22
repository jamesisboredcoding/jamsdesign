//! Healing maths, on small dense regions of straight RGBA (0..255 as f32).
//!
//! - [`seamless`]: the Healing Brush. Copies the source but solves Laplace's equation for the
//!   difference to the destination, so the copy takes on the colour and brightness of its new
//!   surroundings while keeping the source's texture (Poisson / seamless cloning).
//! - [`inpaint`]: the Spot Healing Brush (content-aware). Fills a hole with patches from around
//!   it: multi-scale PatchMatch with expectation-maximisation voting (Wexler et al.,
//!   "Space-Time Completion of Video"; Barnes et al., "PatchMatch").

pub type Px = [f32; 4];

/// Patch radius for inpainting (7x7 patches).
const R: i32 = 3;

/// Fills `v` inside `hole` with the smoothest surface that meets the values around it
/// (Laplace's equation), coarse to fine so large holes converge quickly.
pub fn membrane(v: &mut [Px], hole: &[bool], w: usize, h: usize) {
    let holes: Vec<usize> = (0..w * h).filter(|&i| hole[i]).collect();
    if holes.is_empty() || holes.len() == w * h {
        return;
    }
    let coarse = w > 6 && h > 6;
    if coarse {
        // Coarse grid: a cell is known if any of its fine pixels is (average of the known ones).
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let mut cv = vec![[0f32; 4]; cw * ch];
        let mut chole = vec![true; cw * ch];
        for cy in 0..ch {
            for cx in 0..cw {
                let mut sum = [0f32; 4];
                let mut n = 0.0;
                for (x, y) in [(2 * cx, 2 * cy), (2 * cx + 1, 2 * cy), (2 * cx, 2 * cy + 1), (2 * cx + 1, 2 * cy + 1)] {
                    if x < w && y < h && !hole[y * w + x] {
                        for c in 0..4 {
                            sum[c] += v[y * w + x][c];
                        }
                        n += 1.0;
                    }
                }
                if n > 0.0 {
                    cv[cy * cw + cx] = sum.map(|s| s / n);
                    chole[cy * cw + cx] = false;
                }
            }
        }
        membrane(&mut cv, &chole, cw, ch);
        for &i in &holes {
            let (x, y) = (i % w, i / w);
            v[i] = cv[(y / 2) * cw + x / 2];
        }
    }
    // Over-relaxed Gauss-Seidel; the coarse start means few sweeps are needed.
    let sweeps = if coarse { 24 } else { 300 };
    for _ in 0..sweeps {
        for &i in &holes {
            let (x, y) = (i % w, i / w);
            let mut sum = [0f32; 4];
            let mut n = 0.0;
            for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
                if nx < w && ny < h {
                    for c in 0..4 {
                        sum[c] += v[ny * w + nx][c];
                    }
                    n += 1.0;
                }
            }
            for c in 0..4 {
                v[i][c] += 1.7 * (sum[c] / n - v[i][c]);
            }
        }
    }
}

/// Healing Brush: `src` placed into `dst` inside `hole`, blended seamlessly. Pixels outside the
/// hole are returned unchanged.
pub fn seamless(dst: &[Px], src: &[Px], hole: &[bool], w: usize, h: usize) -> Vec<Px> {
    // The correction (dst - src) is known around the hole and smooth inside it.
    let mut diff: Vec<Px> = dst.iter().zip(src).map(|(d, s)| [0, 1, 2, 3].map(|c| d[c] - s[c])).collect();
    membrane(&mut diff, hole, w, h);
    (0..w * h)
        .map(|i| if hole[i] { [0, 1, 2, 3].map(|c| (src[i][c] + diff[i][c]).clamp(0.0, 255.0)) } else { dst[i] })
        .collect()
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// One pyramid level.
struct Level {
    w: usize,
    h: usize,
    img: Vec<Px>,
    hole: Vec<bool>,
}

impl Level {
    /// Half size. A coarse pixel is a hole if any of its fine pixels is.
    fn down(&self) -> Level {
        let (w, h) = (self.w.div_ceil(2), self.h.div_ceil(2));
        let mut img = vec![[0f32; 4]; w * h];
        let mut hole = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0f32; 4];
                let mut n = 0.0;
                for (fx, fy) in [(2 * x, 2 * y), (2 * x + 1, 2 * y), (2 * x, 2 * y + 1), (2 * x + 1, 2 * y + 1)] {
                    if fx < self.w && fy < self.h {
                        let i = fy * self.w + fx;
                        hole[y * w + x] |= self.hole[i];
                        for c in 0..4 {
                            sum[c] += self.img[i][c];
                        }
                        n += 1.0;
                    }
                }
                img[y * w + x] = sum.map(|s| s / n);
            }
        }
        Level { w, h, img, hole }
    }

    /// Source patch centres: patch fully inside the image and touching no hole pixel.
    fn valid_sources(&self) -> (Vec<bool>, Vec<u32>) {
        let (w, h) = (self.w, self.h);
        // Integral image of the hole.
        let mut sat = vec![0u32; (w + 1) * (h + 1)];
        for y in 0..h {
            for x in 0..w {
                sat[(y + 1) * (w + 1) + x + 1] =
                    self.hole[y * w + x] as u32 + sat[y * (w + 1) + x + 1] + sat[(y + 1) * (w + 1) + x] - sat[y * (w + 1) + x];
            }
        }
        let r = R as usize;
        let mut valid = vec![false; w * h];
        let mut list = Vec::new();
        if w > 2 * r && h > 2 * r {
            for y in r..h - r {
                for x in r..w - r {
                    let (x0, y0, x1, y1) = (x - r, y - r, x + r + 1, y + r + 1);
                    let holes = sat[y1 * (w + 1) + x1] + sat[y0 * (w + 1) + x0] - sat[y0 * (w + 1) + x1] - sat[y1 * (w + 1) + x0];
                    if holes == 0 {
                        valid[y * w + x] = true;
                        list.push((y * w + x) as u32);
                    }
                }
            }
        }
        (valid, list)
    }

    /// Patch distance (sum of squared differences) between centres `t` and `s`, skipping
    /// offsets where the target patch leaves the image. Stops early above `limit`.
    fn dist(&self, t: usize, s: usize, limit: f32) -> f32 {
        let (w, h) = (self.w as i32, self.h as i32);
        let (tx, ty) = ((t % self.w) as i32, (t / self.w) as i32);
        let (sx, sy) = ((s % self.w) as i32, (s / self.w) as i32);
        let mut d = 0.0;
        for oy in -R..=R {
            let (ty2, sy2) = (ty + oy, sy + oy);
            if ty2 < 0 || ty2 >= h {
                continue;
            }
            for ox in -R..=R {
                let tx2 = tx + ox;
                if tx2 < 0 || tx2 >= w {
                    continue;
                }
                let a = self.img[(ty2 * w + tx2) as usize];
                let b = self.img[(sy2 * w + sx + ox) as usize];
                // Alpha counts too, so transparent areas aren't used to fill opaque ones.
                d += (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2) + (a[3] - b[3]).powi(2);
            }
            if d > limit {
                return d;
            }
        }
        d
    }
}

/// Spot Healing: fills `hole` from the rest of the image. Returns the whole region with the
/// hole filled (pixels outside the hole unchanged).
pub fn inpaint(img: &[Px], hole: &[bool], w: usize, h: usize) -> Vec<Px> {
    let mut levels = vec![Level { w, h, img: img.to_vec(), hole: hole.to_vec() }];
    while levels.len() < 7 {
        let last = levels.last().expect("non-empty");
        if last.w.min(last.h) < 4 * (2 * R as usize + 1) {
            break;
        }
        let next = last.down();
        // Stop while there's still something to copy from.
        if next.valid_sources().1.len() < 16 {
            break;
        }
        levels.push(next);
    }

    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    // Nearest-neighbour field of the previous (coarser) level: target -> source centre.
    let mut prev_nnf: Option<(usize, usize, Vec<u32>)> = None;
    let coarsest = levels.len() - 1;
    for li in (0..levels.len()).rev() {
        let lv = &mut levels[li];
        let (w, h) = (lv.w, lv.h);
        let (valid, sources) = lv.valid_sources();
        if li == coarsest {
            let hole = lv.hole.clone();
            membrane(&mut lv.img, &hole, w, h);
        } else if let Some(up) = levels.get(li + 1).map(|c| (c.w, c.img.clone())) {
            // Start from the coarser result.
            let lv = &mut levels[li];
            for i in 0..w * h {
                if lv.hole[i] {
                    lv.img[i] = up.1[(i / w / 2) * up.0 + (i % w) / 2];
                }
            }
        }
        let lv = &mut levels[li];
        if sources.is_empty() {
            prev_nnf = None;
            continue;
        }
        // Targets: patch centres whose patch overlaps the hole.
        let mut is_target = vec![false; w * h];
        for i in 0..w * h {
            if lv.hole[i] {
                let (x, y) = ((i % w) as i32, (i / w) as i32);
                for ty in (y - R).max(0)..=(y + R).min(h as i32 - 1) {
                    for tx in (x - R).max(0)..=(x + R).min(w as i32 - 1) {
                        is_target[ty as usize * w + tx as usize] = true;
                    }
                }
            }
        }
        let targets: Vec<usize> = (0..w * h).filter(|&i| is_target[i]).collect();
        let mut nnf = vec![0u32; w * h];
        let mut dist = vec![f32::MAX; w * h];
        for &t in &targets {
            let (x, y) = (t % w, t / w);
            let mut s = sources[rng.below(sources.len())];
            if let Some((pw, ph, pn)) = &prev_nnf {
                let (cx, cy) = ((x / 2).min(pw - 1), (y / 2).min(ph - 1));
                let c = pn[cy * pw + cx] as usize;
                if c != u32::MAX as usize {
                    let (sx, sy) = ((c % pw) * 2 + x % 2, (c / pw) * 2 + y % 2);
                    if sx < w && sy < h && valid[sy * w + sx] {
                        s = (sy * w + sx) as u32;
                    }
                }
            }
            nnf[t] = s;
        }

        let em_iters = if li == coarsest { 10 } else if li == 0 { 3 } else { 5 };
        for _ in 0..em_iters {
            for &t in &targets {
                dist[t] = lv.dist(t, nnf[t] as usize, f32::MAX);
            }
            // PatchMatch: propagate good matches from neighbours, then random search.
            for pass in 0..2 {
                let fwd = pass % 2 == 0;
                let order: Box<dyn Iterator<Item = &usize>> = if fwd { Box::new(targets.iter()) } else { Box::new(targets.iter().rev()) };
                for &t in order {
                    let (x, y) = ((t % w) as i32, (t / w) as i32);
                    let step: i32 = if fwd { -1 } else { 1 };
                    let try_s = |cand: i32, cy: i32, best: &mut (u32, f32), lv: &Level| {
                        if cand < 0 || cy < 0 || cand >= w as i32 || cy >= h as i32 {
                            return;
                        }
                        let s = cy as usize * w + cand as usize;
                        if valid[s] && s as u32 != best.0 {
                            let d = lv.dist(t, s, best.1);
                            if d < best.1 {
                                *best = (s as u32, d);
                            }
                        }
                    };
                    let mut best = (nnf[t], dist[t]);
                    // Neighbour to the left/right and above/below: shift its match by one.
                    for (nx, ny) in [(x + step, y), (x, y + step)] {
                        if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                            let n = ny as usize * w + nx as usize;
                            if is_target[n] {
                                let s = nnf[n] as usize;
                                let (sx, sy) = ((s % w) as i32 - (nx - x), (s / w) as i32 - (ny - y));
                                try_s(sx, sy, &mut best, lv);
                            }
                        }
                    }
                    let mut radius = w.max(h) as i32;
                    while radius >= 1 {
                        let s = best.0 as usize;
                        let sx = (s % w) as i32 + rng.below(2 * radius as usize + 1) as i32 - radius;
                        let sy = (s / w) as i32 + rng.below(2 * radius as usize + 1) as i32 - radius;
                        try_s(sx, sy, &mut best, lv);
                        radius /= 2;
                    }
                    nnf[t] = best.0;
                    dist[t] = best.1;
                }
            }

            // Vote: each hole pixel becomes the weighted average of what overlapping patches say.
            let mut sorted: Vec<f32> = targets.iter().map(|&t| dist[t]).collect();
            sorted.sort_by(|a, b| a.total_cmp(b));
            let sigma2 = 2.0 * sorted[sorted.len() * 3 / 4].max(1.0);
            let weight: Vec<f32> = (0..w * h).map(|i| if is_target[i] { (-dist[i] / sigma2).exp() + 1e-6 } else { 0.0 }).collect();
            let mut next = lv.img.clone();
            for i in 0..w * h {
                if !lv.hole[i] {
                    continue;
                }
                let (x, y) = ((i % w) as i32, (i / w) as i32);
                let mut acc = [0f32; 4];
                let mut wsum = 0.0;
                for oy in -R..=R {
                    for ox in -R..=R {
                        let (tx, ty) = (x - ox, y - oy);
                        if tx < 0 || ty < 0 || tx >= w as i32 || ty >= h as i32 {
                            continue;
                        }
                        let t = ty as usize * w + tx as usize;
                        if !is_target[t] {
                            continue;
                        }
                        let s = nnf[t] as usize;
                        let sp = ((s / w) as i32 + oy) as usize * w + ((s % w) as i32 + ox) as usize;
                        let p = lv.img[sp];
                        for c in 0..4 {
                            acc[c] += p[c] * weight[t];
                        }
                        wsum += weight[t];
                    }
                }
                if wsum > 0.0 {
                    next[i] = acc.map(|a| a / wsum);
                }
            }
            lv.img = next;
        }
        let mut field = vec![u32::MAX; w * h];
        for &t in &targets {
            field[t] = nnf[t];
        }
        prev_nnf = Some((w, h, field));
    }
    levels.swap_remove(0).img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn membrane_interpolates_linearly() {
        // Left column 0, right column 100: the hole between becomes a ramp.
        let (w, h) = (11, 5);
        let mut v = vec![[0f32; 4]; w * h];
        let mut hole = vec![false; w * h];
        for y in 0..h {
            v[y * w + w - 1] = [100.0; 4];
            for x in 1..w - 1 {
                hole[y * w + x] = true;
                v[y * w + x] = [50.0; 4];
            }
        }
        membrane(&mut v, &hole, w, h);
        assert!((v[2 * w + 5][0] - 50.0).abs() < 2.0, "{}", v[2 * w + 5][0]);
        assert!((v[2 * w + 2][0] - 20.0).abs() < 3.0, "{}", v[2 * w + 2][0]);
    }

    #[test]
    fn seamless_keeps_texture_and_takes_tone() {
        // Source: dark with stripes. Destination: bright and flat.
        let (w, h) = (40, 40);
        let src: Vec<Px> = (0..w * h).map(|i| if (i % w) % 4 == 0 { [60.0; 4] } else { [20.0; 4] }).collect();
        let dst = vec![[200.0f32; 4]; w * h];
        let hole: Vec<bool> = (0..w * h).map(|i| (10..30).contains(&(i % w)) && (10..30).contains(&(i / w))).collect();
        let out = seamless(&dst, &src, &hole, w, h);
        let (a, b) = (out[20 * w + 20][0], out[20 * w + 21][0]);
        assert!((a - b).abs() > 30.0, "texture kept: {a} vs {b}");
        assert!(a.min(b) > 150.0, "tone taken from the destination: {a}, {b}");
    }

    #[test]
    fn inpaint_removes_an_object_from_a_texture() {
        // Vertical stripes (period 6) with a red square on top: the square should become stripes.
        let (w, h) = (96, 96);
        let stripe = |x: usize| if x % 6 < 3 { [30.0, 30.0, 30.0, 255.0] } else { [220.0, 220.0, 220.0, 255.0] };
        let mut img: Vec<Px> = (0..w * h).map(|i| stripe(i % w)).collect();
        let hole: Vec<bool> = (0..w * h).map(|i| (40..56).contains(&(i % w)) && (40..56).contains(&(i / w))).collect();
        for i in 0..w * h {
            if hole[i] {
                img[i] = [255.0, 0.0, 0.0, 255.0];
            }
        }
        let out = inpaint(&img, &hole, w, h);
        let mut err = 0.0;
        let mut n = 0.0;
        for i in 0..w * h {
            if hole[i] {
                assert!(out[i][1] > 1.0 || out[i][0] < 200.0, "no red left at {i}: {:?}", out[i]);
                err += (out[i][0] - stripe(i % w)[0]).abs();
                n += 1.0;
            }
        }
        assert!(err / n < 50.0, "mean error {}", err / n);
    }
}
