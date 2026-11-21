//! Mixes a sequence's audible audio clips into interleaved stereo, chunk by chunk.

use std::collections::HashMap;
use std::path::PathBuf;

use jd_media::AudioDecoder;
use jd_timeline::{Clip, MediaId, SECOND, Sequence, Time, to_seconds};

pub fn frames_to_flicks(frames: u64, rate: u32) -> Time {
    (frames as i128 * SECOND as i128 / rate.max(1) as i128) as Time
}

fn flicks_to_frames(t: Time, rate: u32) -> i64 {
    (t as i128 * rate as i128 / SECOND as i128) as i64
}

struct Source {
    clip: Clip,
    path: Option<PathBuf>,
    decoder: Option<AudioDecoder>,
    /// Couldn't be opened: stays silent instead of retrying every chunk.
    failed: bool,
}

pub struct Mixer {
    sources: Vec<Source>,
    rate: u32,
    /// Sequence time of the next sample.
    pos: Time,
    scratch: Vec<f32>,
}

impl Mixer {
    /// Mixes the audible tracks: unmuted, and only soloed ones if any track is soloed.
    pub fn new(seq: &Sequence, media: &HashMap<MediaId, PathBuf>, rate: u32, from: Time) -> Self {
        let any_solo = seq.audio.iter().any(|t| t.solo);
        let sources = seq
            .audio
            .iter()
            .filter(|t| t.enabled && (!any_solo || t.solo))
            .flat_map(|t| &t.clips)
            .filter(|c| c.end() > from)
            .map(|c| Source { clip: c.clone(), path: media.get(&c.media).cloned(), decoder: None, failed: false })
            .collect();
        Self { sources, rate, pos: from, scratch: Vec::new() }
    }

    /// The next `frames` sample frames (interleaved stereo).
    pub fn mix(&mut self, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * 2];
        let t0 = self.pos;
        let t1 = t0 + frames_to_flicks(frames as u64, self.rate);
        for s in &mut self.sources {
            let c = &s.clip;
            if s.failed || !c.overlaps(t0, t1) {
                continue;
            }
            let a = t0.max(c.start);
            let b = t1.min(c.end());
            let offset = flicks_to_frames(a - t0, self.rate).clamp(0, frames as i64) as usize;
            let end = flicks_to_frames(b - t0, self.rate).clamp(0, frames as i64) as usize;
            if end <= offset {
                continue;
            }
            if s.decoder.is_none() {
                let start = to_seconds(c.source_time(a));
                let opened = s.path.as_deref().ok_or(()).and_then(|p| AudioDecoder::open(p, self.rate).map_err(|_| ()));
                match opened.and_then(|mut d| d.seek(start).map(|()| d).map_err(|_| ())) {
                    Ok(d) => s.decoder = Some(d),
                    Err(()) => {
                        s.failed = true;
                        continue;
                    }
                }
            }
            let d = s.decoder.as_mut().expect("decoder opened");
            let n = end - offset;
            self.scratch.resize(n * 2, 0.0);
            let got = d.read(&mut self.scratch).unwrap_or(0);
            for (o, v) in out[offset * 2..(offset + got) * 2].iter_mut().zip(&self.scratch[..got * 2]) {
                *o += v;
            }
        }
        // Clips that are over don't need their decoders any more.
        self.sources.retain(|s| s.clip.end() > t1);
        self.pos = t1;
        out
    }
}
