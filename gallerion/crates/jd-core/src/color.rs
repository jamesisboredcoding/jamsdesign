/// A colour in **linear** light, straight (non-premultiplied) alpha, 0.0..=1.0 per channel.
///
/// All compositing in jamsdesign happens in linear space. Convert to/from sRGB only at the
/// edges (UI display, 8-bit file IO, hex input).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0, 1.0);
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0, 1.0);
    pub const TRANSPARENT: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Build from 8-bit sRGB-encoded values (what users see in hex pickers).
    pub fn from_srgb8(r: u8, g: u8, b: u8) -> Self {
        Self::from_srgb([r, g, b].map(|c| c as f32 / 255.0))
    }

    /// Build from sRGB-encoded floats in 0.0..=1.0.
    pub fn from_srgb([r, g, b]: [f32; 3]) -> Self {
        Self::new(srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b), 1.0)
    }

    /// sRGB-encoded floats in 0.0..=1.0 (alpha dropped).
    pub fn to_srgb(self) -> [f32; 3] {
        [self.r, self.g, self.b].map(|c| linear_to_srgb(c.clamp(0.0, 1.0)))
    }

    /// Parse `#RRGGBB` / `RRGGBB` (sRGB).
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim().trim_start_matches('#');
        if hex.len() != 6 || !hex.is_ascii() {
            return None;
        }
        let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Self::from_srgb8(channel(0)?, channel(2)?, channel(4)?))
    }

    /// Encode to 8-bit sRGB (alpha dropped).
    pub fn to_srgb8(self) -> [u8; 3] {
        self.to_srgb().map(|c| (c * 255.0).round() as u8)
    }

    pub fn to_hex(self) -> String {
        let [r, g, b] = self.to_srgb8();
        format!("{r:02X}{g:02X}{b:02X}")
    }
}

/// Hue/saturation/value over **sRGB-encoded** values, the way colour pickers present them.
/// `h` in degrees 0..360, `s` and `v` in 0..=1. Keeping a picker's state as HSV preserves the
/// hue when saturation or value hits zero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsv {
    pub h: f32,
    pub s: f32,
    pub v: f32,
}

impl Hsv {
    pub fn from_srgb([r, g, b]: [f32; 3]) -> Self {
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        let h = if d == 0.0 {
            0.0
        } else if max == r {
            60.0 * ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        let s = if max == 0.0 { 0.0 } else { d / max };
        Self { h, s, v: max }
    }

    pub fn to_srgb(self) -> [f32; 3] {
        let c = self.v * self.s;
        let hp = self.h.rem_euclid(360.0) / 60.0;
        let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
        let (r, g, b) = match hp as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = self.v - c;
        [r + m, g + m, b + m]
    }

    pub fn to_rgba(self) -> Rgba {
        Rgba::from_srgb(self.to_srgb())
    }

    pub fn from_rgba(c: Rgba) -> Self {
        Self::from_srgb(c.to_srgb())
    }
}

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb8_roundtrips_every_value() {
        for v in 0..=255u8 {
            let [r, _, _] = Rgba::from_srgb8(v, 0, 0).to_srgb8();
            assert_eq!(r, v);
        }
    }

    #[test]
    fn hex_parse_and_format() {
        let c = Rgba::from_hex("#F2733F").unwrap();
        assert_eq!(c.to_hex(), "F2733F");
        assert!(Rgba::from_hex("xyz").is_none());
        assert!(Rgba::from_hex("#12345G").is_none());
    }

    #[test]
    fn hsv_roundtrip() {
        for hex in ["F2733F", "000000", "FFFFFF", "4C8DFF", "12AB34", "808080"] {
            let c = Rgba::from_hex(hex).unwrap();
            assert_eq!(Hsv::from_rgba(c).to_rgba().to_hex(), hex);
        }
        let red = Hsv { h: 0.0, s: 1.0, v: 1.0 }.to_srgb();
        assert_eq!(red, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn mid_grey_is_darker_in_linear() {
        let c = Rgba::from_srgb8(128, 128, 128);
        assert!((c.r - 0.2158).abs() < 0.001);
    }
}
