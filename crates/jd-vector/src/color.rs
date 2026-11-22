//! Colours are stored as `jd_core::Rgba` (linear) and written to files as sRGB hex,
//! `#RRGGBB` when opaque and `#RRGGBBAA` otherwise, so documents stay readable.

use jd_core::Rgba;
use serde::{Deserialize, Deserializer, Serializer};

pub fn to_hex(c: Rgba) -> String {
    let a = (c.a.clamp(0.0, 1.0) * 255.0).round() as u8;
    if a == 255 { format!("#{}", c.to_hex()) } else { format!("#{}{a:02X}", c.to_hex()) }
}

pub fn from_hex(s: &str) -> Option<Rgba> {
    let h = s.trim().trim_start_matches('#');
    match h.len() {
        6 => Rgba::from_hex(h),
        8 if h.is_ascii() => {
            let mut c = Rgba::from_hex(&h[..6])?;
            c.a = u8::from_str_radix(&h[6..], 16).ok()? as f32 / 255.0;
            Some(c)
        }
        _ => None,
    }
}

pub mod hex {
    use super::*;

    pub fn serialize<S: Serializer>(c: &Rgba, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&to_hex(*c))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Rgba, D::Error> {
        let s = String::deserialize(d)?;
        from_hex(&s).ok_or_else(|| serde::de::Error::custom(format!("bad colour {s:?}, expected #RRGGBB or #RRGGBBAA")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_with_and_without_alpha() {
        let c = from_hex("#4C8DFF").unwrap();
        assert_eq!(to_hex(c), "#4C8DFF");
        let c = from_hex("4c8dff80").unwrap();
        assert_eq!(to_hex(c), "#4C8DFF80");
        assert!(from_hex("#12345").is_none());
    }
}
