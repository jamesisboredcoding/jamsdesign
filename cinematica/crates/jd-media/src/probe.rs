//! What's inside a media file.

use std::path::Path;

use ffmpeg_next as ff;
use ff::media::Type;

use crate::Result;

/// A frame rate as a fraction (29.97 is 30000/1001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
    pub num: i32,
    pub den: i32,
}

impl Rate {
    pub fn as_f64(self) -> f64 {
        if self.den == 0 { 0.0 } else { self.num as f64 / self.den as f64 }
    }

    /// Nearest whole frames per second (timecode frame counting).
    pub fn rounded(self) -> u32 {
        self.as_f64().round().max(1.0) as u32
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub rate: Rate,
    pub codec: String,
    /// A single image (PNG, JPEG...) rather than moving video.
    pub still: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub codec: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    /// Seconds. Stills report 0 (the timeline gives them a default length).
    pub duration: f64,
    pub video: Option<VideoInfo>,
    pub audio: Option<AudioInfo>,
}

pub(crate) fn rate_of(stream: &ff::format::stream::Stream) -> Rate {
    let r = stream.avg_frame_rate();
    let r = if r.numerator() > 0 && r.denominator() > 0 { r } else { stream.rate() };
    if r.numerator() > 0 && r.denominator() > 0 {
        Rate { num: r.numerator(), den: r.denominator() }
    } else {
        Rate { num: 25, den: 1 }
    }
}

/// Seconds for a stream timestamp.
pub(crate) fn seconds(ts: i64, tb: ff::Rational) -> f64 {
    ts as f64 * tb.numerator() as f64 / tb.denominator().max(1) as f64
}

/// Clockwise quarter turns that display a stream upright, from its display matrix.
pub(crate) fn quarter_turns(stream: &ff::format::stream::Stream) -> u8 {
    use ff::ffi;
    let degrees = unsafe {
        let par = stream.parameters();
        let par = par.as_ptr();
        let sd = ffi::av_packet_side_data_get((*par).coded_side_data, (*par).nb_coded_side_data, ffi::AVPacketSideDataType::AV_PKT_DATA_DISPLAYMATRIX);
        if sd.is_null() || (*sd).size < 36 {
            return 0;
        }
        // The matrix's rotation is anticlockwise; displaying upright needs the opposite.
        -ffi::av_display_rotation_get((*sd).data as *const i32)
    };
    if !degrees.is_finite() {
        return 0;
    }
    ((degrees / 90.0).round() as i64).rem_euclid(4) as u8
}

pub fn probe(path: &Path) -> Result<MediaInfo> {
    crate::init();
    let input = ff::format::input(path)?;
    let format = input.format().name().to_owned();

    let video = input.streams().best(Type::Video).map(|s| -> Result<VideoInfo> {
        let ctx = ff::codec::context::Context::from_parameters(s.parameters())?;
        let codec = ctx.id().name().to_owned();
        let v = ctx.decoder().video()?;
        // Image demuxers ("image2", "png_pipe"...) and cover art are single pictures.
        let still = format.contains("image2")
            || format.ends_with("_pipe")
            || s.disposition().contains(ff::format::stream::Disposition::ATTACHED_PIC);
        // Report the upright size (a portrait phone clip is taller than wide).
        let (width, height) = if quarter_turns(&s) % 2 == 1 { (v.height(), v.width()) } else { (v.width(), v.height()) };
        Ok(VideoInfo { width, height, rate: rate_of(&s), codec, still })
    });
    let video = video.transpose()?;

    let audio = input.streams().best(Type::Audio).map(|s| -> Result<AudioInfo> {
        let ctx = ff::codec::context::Context::from_parameters(s.parameters())?;
        let codec = ctx.id().name().to_owned();
        let a = ctx.decoder().audio()?;
        Ok(AudioInfo { sample_rate: a.rate(), channels: a.channels(), codec })
    });
    let audio = audio.transpose()?;

    let duration = if video.as_ref().is_some_and(|v| v.still) && audio.is_none() {
        0.0
    } else if input.duration() > 0 {
        input.duration() as f64 / f64::from(ff::ffi::AV_TIME_BASE)
    } else {
        input
            .streams()
            .filter(|s| s.duration() > 0)
            .map(|s| seconds(s.duration(), s.time_base()))
            .fold(0.0, f64::max)
    };
    Ok(MediaInfo { duration, video, audio })
}
