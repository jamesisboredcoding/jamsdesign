//! Picks the frame to show at a sequence time, decoding forward when possible and seeking
//! only on cuts, jumps and backwards playback.

use std::collections::HashMap;
use std::path::PathBuf;

use jd_media::{Frame, VideoDecoder};
use jd_timeline::{ClipId, MediaId, Sequence, Time, to_seconds};

pub struct VideoState {
    max_size: (u32, u32),
    /// The clip being shown, its decoder, and the source time of the decoder's frame.
    current: Option<(ClipId, VideoDecoder, Option<f64>)>,
    /// Something is on screen (false after showing black).
    showing: bool,
}

impl VideoState {
    pub fn new(max_size: (u32, u32)) -> Self {
        Self { max_size, current: None, showing: false }
    }

    /// The frame for time `t` if it differs from what's on screen: `Some(None)` is black.
    pub fn show(&mut self, seq: &Sequence, media: &HashMap<MediaId, PathBuf>, t: Time) -> Result<Option<Option<Frame>>, String> {
        let Some(clip) = seq.top_video_at(t) else {
            self.current = None;
            return Ok(std::mem::take(&mut self.showing).then_some(None));
        };
        if self.current.as_ref().is_none_or(|(id, ..)| *id != clip.id) {
            let path = media.get(&clip.media).ok_or("media missing")?;
            let d = VideoDecoder::open(path, self.max_size).map_err(|e| e.to_string())?;
            self.current = Some((clip.id, d, None));
        }
        let (_, d, at) = self.current.as_mut().expect("decoder opened");
        let still = d.is_still();
        let s = if still { 0.0 } else { to_seconds(clip.source_time(t)) };
        let fd = 1.0 / d.rate().as_f64().max(1.0);
        if let Some(cur) = *at
            && (still || (cur <= s + fd * 0.5 && s < cur + fd * 0.5))
            && self.showing
        {
            return Ok(None);
        }
        let landed = match *at {
            // Close ahead: decode forward (cheap), converting only the frame we show.
            Some(cur) if !still && s > cur && s - cur < 1.0 => {
                let mut last = Some(cur);
                loop {
                    match d.next_time().map_err(|e| e.to_string())? {
                        Some(time) if time + fd * 0.5 <= s => last = Some(time),
                        Some(time) => break Some(time),
                        None => break last,
                    }
                }
            }
            _ => d.seek(s).map_err(|e| e.to_string())?,
        };
        *at = landed;
        let frame = d.frame().map_err(|e| e.to_string())?;
        self.showing = frame.is_some();
        Ok(Some(frame))
    }
}
