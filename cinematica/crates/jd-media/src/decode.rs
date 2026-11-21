//! Frame-accurate video decoding to RGBA8.

use std::path::Path;

use ffmpeg_next as ff;
use ff::format::Pixel;
use ff::media::Type;
use ff::software::scaling;

use crate::probe::{Rate, rate_of, seconds};
use crate::{Error, Result};

/// A decoded picture, tightly packed RGBA8 (sRGB, straight alpha).
#[derive(Clone, PartialEq)]
pub struct Frame {
    /// Presentation time in seconds from the start of the media.
    pub time: f64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Frame({}x{} @ {:.3}s)", self.width, self.height, self.time)
    }
}

/// Decodes one file's video stream. Frames come out scaled to fit inside `max_size` (never
/// upscaled), which keeps monitor playback and thumbnails cheap.
pub struct VideoDecoder {
    input: ff::format::context::Input,
    stream: usize,
    time_base: ff::Rational,
    /// Stream start offset in seconds (subtracted from every timestamp).
    start: f64,
    decoder: ff::decoder::Video,
    /// The last frame decoded, and its time.
    raw: ff::frame::Video,
    raw_time: Option<f64>,
    scratch: ff::frame::Video,
    scaler: Option<(Pixel, u32, u32, u32, u32, scaling::Context)>,
    max_size: (u32, u32),
    rate: Rate,
    duration: f64,
    still: bool,
    /// Clockwise quarter turns that make the picture upright (phones record portrait video
    /// as landscape plus a display matrix).
    turns: u8,
    /// End of file sent to the decoder.
    drained: bool,
}

impl VideoDecoder {
    pub fn open(path: &Path, max_size: (u32, u32)) -> Result<Self> {
        crate::init();
        let input = ff::format::input(path)?;
        let info = crate::probe(path)?;
        let still = info.video.as_ref().is_some_and(|v| v.still);
        let (stream, time_base, start, rate, params, turns) = {
            let s = input.streams().best(Type::Video).ok_or(Error::NoVideo)?;
            let turns = crate::probe::quarter_turns(&s);
            let start = if s.start_time() == ff::ffi::AV_NOPTS_VALUE { 0.0 } else { seconds(s.start_time(), s.time_base()) };
            (s.index(), s.time_base(), start, rate_of(&s), s.parameters(), turns)
        };
        let mut ctx = ff::codec::context::Context::from_parameters(params)?;
        ctx.set_threading(ff::codec::threading::Config { kind: ff::codec::threading::Type::Frame, count: 0 });
        let decoder = ctx.decoder().video()?;
        Ok(Self {
            input,
            stream,
            time_base,
            start,
            decoder,
            raw: ff::frame::Video::empty(),
            raw_time: None,
            scratch: ff::frame::Video::empty(),
            scaler: None,
            max_size,
            rate,
            duration: info.duration,
            still,
            turns,
            drained: false,
        })
    }

    pub fn rate(&self) -> Rate {
        self.rate
    }

    pub fn duration(&self) -> f64 {
        self.duration
    }

    pub fn is_still(&self) -> bool {
        self.still
    }

    /// Decodes the next frame in presentation order and returns its time, or `None` at the
    /// end. The frame isn't converted until [`Self::frame`] is called, so frames can be
    /// skipped cheaply.
    pub fn next_time(&mut self) -> Result<Option<f64>> {
        loop {
            match self.decoder.receive_frame(&mut self.scratch) {
                Ok(()) => {
                    std::mem::swap(&mut self.raw, &mut self.scratch);
                    let ts = self.raw.timestamp().or(self.raw.pts()).unwrap_or(0);
                    let t = seconds(ts, self.time_base) - self.start;
                    self.raw_time = Some(t);
                    return Ok(Some(t));
                }
                Err(ff::Error::Eof) => return Ok(None),
                Err(ff::Error::Other { errno }) if errno == ff::util::error::EAGAIN => {}
                Err(e) => return Err(e.into()),
            }
            if self.drained {
                return Ok(None);
            }
            let mut fed = false;
            for (s, packet) in self.input.packets() {
                if s.index() != self.stream {
                    continue;
                }
                match self.decoder.send_packet(&packet) {
                    // A damaged packet shouldn't stop playback: skip it.
                    Ok(()) | Err(ff::Error::InvalidData) => {}
                    Err(e) => return Err(e.into()),
                }
                fed = true;
                break;
            }
            if !fed {
                self.decoder.send_eof()?;
                self.drained = true;
            }
        }
    }

    /// Moves to the frame showing at time `t` (the first frame no earlier than half a
    /// frame before `t`) and returns its time. Past the end, returns the last frame.
    pub fn seek(&mut self, t: f64) -> Result<Option<f64>> {
        if self.still {
            if self.raw_time.is_none() {
                self.next_time()?;
            }
            return Ok(self.raw_time);
        }
        let tolerance = 0.5 / self.rate.as_f64().max(1.0);
        // Most files seek to the keyframe at or before the target. Some land after it;
        // then retry from further back.
        let mut back = 0.0;
        for _ in 0..4 {
            let from = (t - back).max(0.0);
            let ts = ((from + self.start) * f64::from(ff::ffi::AV_TIME_BASE)) as i64;
            self.input.seek(ts, ..ts)?;
            self.decoder.flush();
            self.drained = false;
            let mut first = true;
            loop {
                match self.next_time()? {
                    None => return Ok(self.raw_time),
                    Some(time) => {
                        if first && time > t + tolerance && from > 0.0 {
                            break;
                        }
                        first = false;
                        if time >= t - tolerance {
                            return Ok(Some(time));
                        }
                    }
                }
            }
            back = if back == 0.0 { 1.0 } else { back * 3.0 };
        }
        Ok(self.raw_time)
    }

    /// The current frame (from the last [`Self::next_time`] or [`Self::seek`]) as RGBA8.
    pub fn frame(&mut self) -> Result<Option<Frame>> {
        let Some(time) = self.raw_time else { return Ok(None) };
        let (sw, sh, format) = (self.raw.width(), self.raw.height(), self.raw.format());
        // Fit the upright picture, then scale to the stored (unrotated) shape.
        let sideways = self.turns % 2 == 1;
        let (w, h) = if sideways {
            let (uw, uh) = fit((sh, sw), self.max_size);
            (uh, uw)
        } else {
            fit((sw, sh), self.max_size)
        };
        let stale = !matches!(&self.scaler, Some((f, a, b, c, d, _)) if (*f, *a, *b, *c, *d) == (format, sw, sh, w, h));
        if stale {
            let mut ctx = scaling::Context::get(format, sw, sh, Pixel::RGBA, w, h, scaling::Flags::BILINEAR)?;
            set_colorspace(&mut ctx, &self.raw);
            self.scaler = Some((format, sw, sh, w, h, ctx));
        }
        let (.., scaler) = self.scaler.as_mut().expect("scaler just set");
        let mut out = ff::frame::Video::empty();
        scaler.run(&self.raw, &mut out)?;
        let stride = out.stride(0);
        let data = out.data(0);
        let row = w as usize * 4;
        let mut rgba = Vec::with_capacity(row * h as usize);
        for y in 0..h as usize {
            rgba.extend_from_slice(&data[y * stride..y * stride + row]);
        }
        let (rgba, w, h) = rotate(rgba, w, h, self.turns);
        Ok(Some(Frame { time, width: w, height: h, rgba }))
    }
}

/// Tells the scaler which YUV matrix and range the video uses, so colours come out right:
/// FFmpeg otherwise assumes BT.601 (standard definition) for everything.
fn set_colorspace(ctx: &mut scaling::Context, frame: &ff::frame::Video) {
    use ff::color::{Range, Space};
    use ff::ffi;
    let space = match frame.color_space() {
        Space::BT709 => ffi::SWS_CS_ITU709,
        Space::BT2020NCL | Space::BT2020CL => ffi::SWS_CS_BT2020,
        Space::BT470BG | Space::SMPTE170M => ffi::SWS_CS_ITU601,
        Space::SMPTE240M => ffi::SWS_CS_SMPTE240M,
        // Untagged: HD and up is almost always BT.709.
        _ if frame.height() >= 720 => ffi::SWS_CS_ITU709,
        _ => ffi::SWS_CS_ITU601,
    } as i32;
    let full_range = i32::from(frame.color_range() == Range::JPEG);
    // RGB formats ignore these; only YUV sources need them.
    unsafe {
        let src = ffi::sws_getCoefficients(space);
        let dst = ffi::sws_getCoefficients(ffi::SWS_CS_DEFAULT as i32);
        ffi::sws_setColorspaceDetails(ctx.as_mut_ptr(), src, full_range, dst, 1, 0, 1 << 16, 1 << 16);
    }
}

/// Rotates a packed RGBA8 picture by `turns` clockwise quarter turns.
fn rotate(rgba: Vec<u8>, w: u32, h: u32, turns: u8) -> (Vec<u8>, u32, u32) {
    let (w, h) = (w as usize, h as usize);
    let px = |x: usize, y: usize| (y * w + x) * 4;
    match turns % 4 {
        0 => (rgba, w as u32, h as u32),
        2 => {
            let mut out = vec![0u8; rgba.len()];
            for y in 0..h {
                for x in 0..w {
                    let d = px(w - 1 - x, h - 1 - y);
                    out[d..d + 4].copy_from_slice(&rgba[px(x, y)..px(x, y) + 4]);
                }
            }
            (out, w as u32, h as u32)
        }
        t => {
            // Quarter turn: the output is h wide and w tall.
            let mut out = vec![0u8; rgba.len()];
            for y in 0..h {
                for x in 0..w {
                    let (nx, ny) = if t == 1 { (h - 1 - y, x) } else { (y, w - 1 - x) };
                    let d = (ny * h + nx) * 4;
                    out[d..d + 4].copy_from_slice(&rgba[px(x, y)..px(x, y) + 4]);
                }
            }
            (out, h as u32, w as u32)
        }
    }
}

/// Largest size inside `max` with the same aspect ratio, never larger than `size`.
fn fit(size: (u32, u32), max: (u32, u32)) -> (u32, u32) {
    let (w, h) = (size.0.max(1) as f64, size.1.max(1) as f64);
    let scale = (max.0 as f64 / w).min(max.1 as f64 / h).min(1.0);
    (((w * scale).round() as u32).max(1), ((h * scale).round() as u32).max(1))
}

/// One frame at `time` (clamped into the media), scaled to fit `max_size`.
pub fn thumbnail(path: &Path, time: f64, max_size: (u32, u32)) -> Result<Frame> {
    let mut d = VideoDecoder::open(path, max_size)?;
    let t = time.clamp(0.0, (d.duration() - 0.1).max(0.0));
    d.seek(t)?;
    d.frame()?.ok_or(Error::NoVideo)
}

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn quarter_turns_rotate_clockwise() {
        // 2×1: red, green.
        let img = vec![255, 0, 0, 255, 0, 255, 0, 255];
        let (out, w, h) = super::rotate(img.clone(), 2, 1, 1);
        assert_eq!((w, h), (1, 2));
        assert_eq!(out, img, "clockwise: red on top, green below");
        let (out, ..) = super::rotate(img.clone(), 2, 1, 3);
        assert_eq!(out, [0, 255, 0, 255, 255, 0, 0, 255], "anticlockwise: green on top");
        let (out, w, h) = super::rotate(img, 2, 1, 2);
        assert_eq!((w, h, out), (2, 1, vec![0, 255, 0, 255, 255, 0, 0, 255]));
    }

    #[test]
    fn fit_keeps_aspect_and_never_upscales() {
        assert_eq!(fit((1920, 1080), (640, 640)), (640, 360));
        assert_eq!(fit((1080, 1920), (640, 640)), (360, 640));
        assert_eq!(fit((320, 240), (640, 640)), (320, 240));
    }
}
