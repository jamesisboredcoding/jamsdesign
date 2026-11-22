use std::fmt;

/// Layer blend modes, grouped the way Photoshop's menu groups them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Dissolve,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    Lighten,
    Screen,
    ColorDodge,
    LinearDodge,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    pub const ALL: [BlendMode; 24] = [
        Self::Normal,
        Self::Dissolve,
        Self::Darken,
        Self::Multiply,
        Self::ColorBurn,
        Self::LinearBurn,
        Self::Lighten,
        Self::Screen,
        Self::ColorDodge,
        Self::LinearDodge,
        Self::Overlay,
        Self::SoftLight,
        Self::HardLight,
        Self::VividLight,
        Self::LinearLight,
        Self::PinLight,
        Self::Difference,
        Self::Exclusion,
        Self::Subtract,
        Self::Divide,
        Self::Hue,
        Self::Saturation,
        Self::Color,
        Self::Luminosity,
    ];

    /// Stable identifier used in files. Never change these.
    pub fn key(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Dissolve => "dissolve",
            Self::Darken => "darken",
            Self::Multiply => "multiply",
            Self::ColorBurn => "color-burn",
            Self::LinearBurn => "linear-burn",
            Self::Lighten => "lighten",
            Self::Screen => "screen",
            Self::ColorDodge => "color-dodge",
            Self::LinearDodge => "linear-dodge",
            Self::Overlay => "overlay",
            Self::SoftLight => "soft-light",
            Self::HardLight => "hard-light",
            Self::VividLight => "vivid-light",
            Self::LinearLight => "linear-light",
            Self::PinLight => "pin-light",
            Self::Difference => "difference",
            Self::Exclusion => "exclusion",
            Self::Subtract => "subtract",
            Self::Divide => "divide",
            Self::Hue => "hue",
            Self::Saturation => "saturation",
            Self::Color => "color",
            Self::Luminosity => "luminosity",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key() == key)
    }

    /// Index used by the GPU shader (`switch` in blend.wgsl). Same order as [`Self::ALL`].
    pub fn index(self) -> u32 {
        Self::ALL.iter().position(|&m| m == self).expect("in ALL") as u32
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Dissolve => "Dissolve",
            Self::Darken => "Darken",
            Self::Multiply => "Multiply",
            Self::ColorBurn => "Color Burn",
            Self::LinearBurn => "Linear Burn",
            Self::Lighten => "Lighten",
            Self::Screen => "Screen",
            Self::ColorDodge => "Color Dodge",
            Self::LinearDodge => "Linear Dodge (Add)",
            Self::Overlay => "Overlay",
            Self::SoftLight => "Soft Light",
            Self::HardLight => "Hard Light",
            Self::VividLight => "Vivid Light",
            Self::LinearLight => "Linear Light",
            Self::PinLight => "Pin Light",
            Self::Difference => "Difference",
            Self::Exclusion => "Exclusion",
            Self::Subtract => "Subtract",
            Self::Divide => "Divide",
            Self::Hue => "Hue",
            Self::Saturation => "Saturation",
            Self::Color => "Color",
            Self::Luminosity => "Luminosity",
        }
    }
}

impl fmt::Display for BlendMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

// ---- the maths ------------------------------------------------------------------------------
//
// `cb` = backdrop, `cs` = source, both unpremultiplied, 0..=1, in the document's working space.
// Formulas follow the W3C Compositing spec, plus Photoshop's extra modes. The WGSL in jd-gpu
// mirrors these exactly; a GPU test checks them against each other.

fn color_burn(cb: f32, cs: f32) -> f32 {
    if cb >= 1.0 {
        1.0
    } else if cs <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - cb) / cs).min(1.0)
    }
}

fn color_dodge(cb: f32, cs: f32) -> f32 {
    if cb <= 0.0 {
        0.0
    } else if cs >= 1.0 {
        1.0
    } else {
        (cb / (1.0 - cs)).min(1.0)
    }
}

fn hard_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        cb * 2.0 * cs
    } else {
        let s = 2.0 * cs - 1.0;
        cb + s - cb * s
    }
}

fn soft_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
    } else {
        let d = if cb <= 0.25 { ((16.0 * cb - 12.0) * cb + 4.0) * cb } else { cb.sqrt() };
        cb + (2.0 * cs - 1.0) * (d - cb)
    }
}

fn separable(mode: BlendMode, cb: f32, cs: f32) -> f32 {
    use BlendMode::*;
    match mode {
        Normal | Dissolve => cs,
        Darken => cb.min(cs),
        Multiply => cb * cs,
        ColorBurn => color_burn(cb, cs),
        LinearBurn => (cb + cs - 1.0).max(0.0),
        Lighten => cb.max(cs),
        Screen => cb + cs - cb * cs,
        ColorDodge => color_dodge(cb, cs),
        LinearDodge => (cb + cs).min(1.0),
        Overlay => hard_light(cs, cb),
        SoftLight => soft_light(cb, cs),
        HardLight => hard_light(cb, cs),
        VividLight => {
            if cs <= 0.5 { color_burn(cb, 2.0 * cs) } else { color_dodge(cb, 2.0 * (cs - 0.5)) }
        }
        LinearLight => (cb + 2.0 * cs - 1.0).clamp(0.0, 1.0),
        PinLight => {
            if cs <= 0.5 { cb.min(2.0 * cs) } else { cb.max(2.0 * cs - 1.0) }
        }
        Difference => (cb - cs).abs(),
        Exclusion => cb + cs - 2.0 * cb * cs,
        Subtract => (cb - cs).max(0.0),
        Divide => {
            if cs <= 0.0 {
                if cb <= 0.0 { 0.0 } else { 1.0 }
            } else {
                (cb / cs).min(1.0)
            }
        }
        Hue | Saturation | Color | Luminosity => unreachable!("non-separable"),
    }
}

type Rgb = [f32; 3];

fn lum(c: Rgb) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(c: Rgb) -> Rgb {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut c = c;
    if n < 0.0 {
        c = c.map(|v| l + (v - l) * l / (l - n));
    }
    if x > 1.0 {
        c = c.map(|v| l + (v - l) * (1.0 - l) / (x - l));
    }
    c
}

fn set_lum(c: Rgb, l: f32) -> Rgb {
    let d = l - lum(c);
    clip_color(c.map(|v| v + d))
}

fn sat(c: Rgb) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: Rgb, s: f32) -> Rgb {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    if mx <= mn {
        return [0.0; 3];
    }
    c.map(|v| (v - mn) * s / (mx - mn))
}

/// Blends one pixel's colour: B(cb, cs).
pub fn blend_rgb(mode: BlendMode, cb: Rgb, cs: Rgb) -> Rgb {
    use BlendMode::*;
    match mode {
        Hue => set_lum(set_sat(cs, sat(cb)), lum(cb)),
        Saturation => set_lum(set_sat(cb, sat(cs)), lum(cb)),
        Color => set_lum(cs, lum(cb)),
        Luminosity => set_lum(cb, lum(cs)),
        _ => [0, 1, 2].map(|i| separable(mode, cb[i], cs[i])),
    }
}

/// Integer hash shared with the GPU for Dissolve. Returns 0..1.
pub fn dissolve_noise(x: u32, y: u32, seed: u32) -> f32 {
    let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// Composites a source pixel over a backdrop. Both premultiplied RGBA in working space.
/// `alpha` of the source already includes layer opacity.
pub fn composite(mode: BlendMode, backdrop: [f32; 4], source: [f32; 4]) -> [f32; 4] {
    let (ab, as_) = (backdrop[3], source[3]);
    if as_ <= 0.0 {
        return backdrop;
    }
    let unpremul = |c: [f32; 4]| -> Rgb {
        if c[3] > 0.0 { [c[0] / c[3], c[1] / c[3], c[2] / c[3]] } else { [0.0; 3] }
    };
    let cb = unpremul(backdrop);
    let cs = unpremul(source);
    let b = blend_rgb(mode, cb, cs);
    let ao = as_ + ab * (1.0 - as_);
    let mut out = [0.0, 0.0, 0.0, ao];
    for i in 0..3 {
        out[i] = as_ * (1.0 - ab) * cs[i] + as_ * ab * b[i] + (1.0 - as_) * ab * cb[i];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_roundtrip_and_are_unique() {
        for m in BlendMode::ALL {
            assert_eq!(BlendMode::from_key(m.key()), Some(m));
        }
        assert_eq!(BlendMode::Multiply.index(), 3);
    }

    #[test]
    fn opaque_normal_replaces() {
        let out = composite(BlendMode::Normal, [0.2, 0.2, 0.2, 1.0], [0.9, 0.1, 0.5, 1.0]);
        assert_eq!(out, [0.9, 0.1, 0.5, 1.0]);
    }

    #[test]
    fn known_values() {
        let px = |m| blend_rgb(m, [0.5, 0.25, 1.0], [0.5, 0.5, 0.0]);
        assert_eq!(px(BlendMode::Multiply), [0.25, 0.125, 0.0]);
        assert_eq!(px(BlendMode::Screen), [0.75, 0.625, 1.0]);
        assert_eq!(px(BlendMode::Difference), [0.0, 0.25, 1.0]);
        assert_eq!(px(BlendMode::Darken), [0.5, 0.25, 0.0]);
        // Luminosity of grey onto a colour keeps hue, takes the grey's lightness.
        let l = blend_rgb(BlendMode::Luminosity, [1.0, 0.0, 0.0], [0.5, 0.5, 0.5]);
        assert!((lum(l) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn half_transparent_over_transparent_keeps_source() {
        let out = composite(BlendMode::Multiply, [0.0; 4], [0.4, 0.2, 0.1, 0.5]);
        assert_eq!(out, [0.4, 0.2, 0.1, 0.5]);
    }

    #[test]
    fn all_modes_stay_in_range() {
        let vals = [0.0, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0];
        for m in BlendMode::ALL {
            for &a in &vals {
                for &b in &vals {
                    let r = blend_rgb(m, [a, b, 0.3], [b, a, 0.7]);
                    assert!(r.iter().all(|v| (-1e-5..=1.0 + 1e-5).contains(v)), "{m:?} {a} {b} → {r:?}");
                }
            }
        }
    }

    #[test]
    fn dissolve_noise_is_uniformish() {
        let mean: f32 = (0..10_000).map(|i| dissolve_noise(i % 100, i / 100, 7)).sum::<f32>() / 10_000.0;
        assert!((mean - 0.5).abs() < 0.02, "{mean}");
    }
}
