//! Audio decoding to interleaved stereo `f32` at a chosen sample rate, with sample-accurate
//! seeking.

use std::path::Path;

use ffmpeg_next as ff;
use ff::ChannelLayout;
use ff::format::Sample;
use ff::format::sample::Type as SampleType;
use ff::media::Type;
use ff::software::resampling;

use crate::probe::seconds;
use crate::{Error, Result};

/// Decodes one file's audio stream, resampled to stereo `f32` at `rate`.
pub struct AudioDecoder {
    input: ff::format::context::Input,
    stream: usize,
    time_base: ff::Rational,
    start: f64,
    decoder: ff::decoder::Audio,
    resampler: Option<((Sample, u64, u32), resampling::Context)>,
    rate: u32,
    frame: ff::frame::Audio,
    /// Decoded samples not read yet (interleaved L, R).
    pending: Vec<f32>,
    read_at: usize,
    /// After a seek: drop samples before this time (or pad up to it with silence).
    seek_to: Option<f64>,
    drained: bool,
    finished: bool,
}

impl AudioDecoder {
    pub fn open(path: &Path, rate: u32) -> Result<Self> {
        crate::init();
        let input = ff::format::input(path)?;
        let (stream, time_base, start, params) = {
            let s = input.streams().best(Type::Audio).ok_or(Error::NoAudio)?;
            let start = if s.start_time() == ff::ffi::AV_NOPTS_VALUE { 0.0 } else { seconds(s.start_time(), s.time_base()) };
            (s.index(), s.time_base(), start, s.parameters())
        };
        let decoder = ff::codec::context::Context::from_parameters(params)?.decoder().audio()?;
        Ok(Self {
            input,
            stream,
            time_base,
            start,
            decoder,
            resampler: None,
            rate,
            frame: ff::frame::Audio::empty(),
            pending: Vec::new(),
            read_at: 0,
            seek_to: Some(0.0),
            drained: false,
            finished: false,
        })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// The next [`Self::read`] starts at `t` seconds.
    pub fn seek(&mut self, t: f64) -> Result<()> {
        let t = t.max(0.0);
        // Land a little early: audio frames are short, and the excess is dropped.
        let from = (t - 0.25).max(0.0) + self.start;
        let ts = (from * f64::from(ff::ffi::AV_TIME_BASE)) as i64;
        self.input.seek(ts, ..ts)?;
        self.decoder.flush();
        self.pending.clear();
        self.read_at = 0;
        self.seek_to = Some(t);
        self.drained = false;
        self.finished = false;
        Ok(())
    }

    /// Fills `out` (interleaved stereo) and returns how many sample frames were written.
    /// Fewer than asked means the end of the audio.
    pub fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let want = out.len() / 2 * 2;
        let mut written = 0;
        while written < want {
            let available = self.pending.len() - self.read_at;
            if available == 0 {
                self.pending.clear();
                self.read_at = 0;
                if !self.decode_more()? {
                    break;
                }
                continue;
            }
            let n = available.min(want - written);
            out[written..written + n].copy_from_slice(&self.pending[self.read_at..self.read_at + n]);
            self.read_at += n;
            written += n;
        }
        Ok(written / 2)
    }

    /// Decodes one more frame into `pending`. False at the end.
    fn decode_more(&mut self) -> Result<bool> {
        loop {
            if self.finished {
                return Ok(false);
            }
            match self.decoder.receive_frame(&mut self.frame) {
                Ok(()) => {
                    let ts = self.frame.timestamp().or(self.frame.pts());
                    let t = ts.map(|ts| seconds(ts, self.time_base) - self.start);
                    self.resample(t)?;
                    if self.read_at < self.pending.len() {
                        return Ok(true);
                    }
                    continue;
                }
                Err(ff::Error::Eof) => {
                    self.finished = true;
                    return Ok(false);
                }
                Err(ff::Error::Other { errno }) if errno == ff::util::error::EAGAIN => {}
                Err(e) => return Err(e.into()),
            }
            if self.drained {
                self.finished = true;
                return Ok(false);
            }
            let mut fed = false;
            for (s, packet) in self.input.packets() {
                if s.index() != self.stream {
                    continue;
                }
                match self.decoder.send_packet(&packet) {
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

    /// Converts the decoded frame (which starts at `time`) and appends it to `pending`,
    /// trimming or padding the first frame after a seek so reads start exactly there.
    fn resample(&mut self, time: Option<f64>) -> Result<()> {
        let f = &self.frame;
        let layout = if f.channel_layout().channels() > 0 { f.channel_layout() } else { ChannelLayout::default(f.channels() as i32) };
        let key = (f.format(), layout.bits(), f.rate());
        if self.resampler.as_ref().is_none_or(|(k, _)| *k != key) {
            let ctx = resampling::Context::get(f.format(), layout, f.rate(), Sample::F32(SampleType::Packed), ChannelLayout::STEREO, self.rate)?;
            self.resampler = Some((key, ctx));
        }
        let (_, ctx) = self.resampler.as_mut().expect("resampler just set");
        let capacity = f.samples() * self.rate as usize / f.rate().max(1) as usize + 256;
        let mut out = ff::frame::Audio::new(Sample::F32(SampleType::Packed), capacity, ChannelLayout::STEREO);
        ctx.run(f, &mut out)?;
        let n = out.samples() * 2;
        let bytes = &out.data(0)[..n * 4];
        let mut samples: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])).collect();

        if let Some(target) = self.seek_to {
            let t = time.unwrap_or(target);
            let offset = ((target - t) * self.rate as f64).round() as i64 * 2;
            if offset > 0 {
                // Frame starts before the target: drop the excess (maybe all of it).
                let drop = (offset as usize).min(samples.len());
                samples.drain(..drop);
                if samples.is_empty() {
                    return Ok(());
                }
            } else if offset < 0 {
                // Frame starts after the target (a gap): pad with silence.
                let mut padded = vec![0.0; (-offset) as usize];
                padded.extend(samples);
                samples = padded;
            }
            self.seek_to = None;
        }
        self.pending.extend(samples);
        Ok(())
    }
}

/// A file's whole audio as peak levels (0..=1), `buckets_per_second` of them per second,
/// for timeline waveforms.
pub fn waveform(path: &Path, buckets_per_second: f64) -> Result<Vec<f32>> {
    const RATE: u32 = 8_000;
    let mut d = AudioDecoder::open(path, RATE)?;
    let per_bucket = ((RATE as f64 / buckets_per_second).round() as usize).max(1);
    let mut buf = vec![0.0f32; per_bucket * 2];
    let mut peaks = Vec::new();
    loop {
        let n = d.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let peak = buf[..n * 2].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        peaks.push(peak.min(1.0));
        if n < per_bucket {
            break;
        }
    }
    Ok(peaks)
}
