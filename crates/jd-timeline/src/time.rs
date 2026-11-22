//! Exact time in flicks: 705,600,000 per second. Every common frame rate (23.976, 24, 25,
//! 29.97, 30, 50, 59.94, 60, 120) and audio rate (44.1k, 48k, 96k) divides it evenly, so frame
//! boundaries are whole numbers and edits never drift.

/// A time or duration, in flicks.
pub type Time = i64;

pub const SECOND: Time = 705_600_000;

/// Frames per second as a fraction (29.97 is 30000/1001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameRate {
    pub num: u32,
    pub den: u32,
}

impl FrameRate {
    pub const fn new(num: u32, den: u32) -> Self {
        Self { num, den }
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den.max(1) as f64
    }

    /// Whole frames per second for timecode (30 for 29.97).
    pub fn timecode_base(self) -> u32 {
        self.as_f64().round().max(1.0) as u32
    }

    /// One frame's length (exact for the standard rates).
    pub fn frame(self) -> Time {
        self.from_frames(1)
    }

    pub fn from_frames(self, frames: i64) -> Time {
        (frames as i128 * SECOND as i128 * self.den as i128 / self.num.max(1) as i128) as Time
    }

    /// Whole frames before `t` (rounded down).
    pub fn to_frames(self, t: Time) -> i64 {
        (t as i128 * self.num as i128).div_euclid(SECOND as i128 * self.den.max(1) as i128) as i64
    }

    /// `t` rounded to the nearest frame boundary.
    pub fn snap(self, t: Time) -> Time {
        let f = self.frame();
        self.from_frames(self.to_frames(t + f / 2))
    }

    /// Non-drop-frame timecode, `HH:MM:SS:FF`.
    pub fn timecode(self, t: Time) -> String {
        let frames = self.to_frames(self.snap(t.max(0))).max(0) as u64;
        let base = self.timecode_base() as u64;
        let (f, s) = (frames % base, frames / base);
        format!("{:02}:{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60, f)
    }
}

pub fn from_seconds(s: f64) -> Time {
    (s * SECOND as f64).round() as Time
}

pub fn to_seconds(t: Time) -> f64 {
    t as f64 / SECOND as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_rates_have_exact_frames() {
        for (num, den) in [(24000, 1001), (24, 1), (25, 1), (30000, 1001), (30, 1), (50, 1), (60000, 1001), (60, 1), (120, 1)] {
            let r = FrameRate::new(num, den);
            assert_eq!(r.frame() * num as i64, SECOND * den as i64, "{num}/{den}");
            assert_eq!(r.to_frames(r.from_frames(123_456)), 123_456);
        }
        assert_eq!(SECOND % 44_100, 0);
        assert_eq!(SECOND % 48_000, 0);
        assert_eq!(SECOND % 96_000, 0);
    }

    #[test]
    fn snapping_and_timecode() {
        let r = FrameRate::new(25, 1);
        assert_eq!(r.snap(r.frame() * 3 + r.frame() / 3), r.frame() * 3);
        assert_eq!(r.snap(r.frame() * 3 + r.frame() * 2 / 3), r.frame() * 4);
        assert_eq!(r.timecode(from_seconds(12.32)), "00:00:12:08");
        assert_eq!(FrameRate::new(30000, 1001).timecode(FrameRate::new(30000, 1001).from_frames(30)), "00:00:01:00");
    }
}
