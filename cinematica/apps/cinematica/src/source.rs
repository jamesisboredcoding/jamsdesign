//! The Source monitor: one clip from the bin, played by a `jd_media::Player`, with In/Out
//! marks and Premiere's J/K/L shuttle.

use std::collections::HashMap;
use std::path::PathBuf;

use iced::widget::image;
use iced_runtime::image::Allocation;
use jd_media::{Player, Status};
use jd_playback::{Request, SequencePlayer};
use jd_timeline::{ClipSource, FrameRate, Sequence, from_seconds, to_seconds};

use crate::media::{MediaId, to_handle};

/// Largest frame the monitor decodes (monitors are rarely bigger than this on screen).
const MONITOR_SIZE: (u32, u32) = (1920, 1080);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shuttle {
    /// J: play backwards; pressing again doubles the speed.
    Reverse,
    /// K: stop.
    Stop,
    /// L: play forwards; pressing again doubles the speed.
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoTo {
    Start,
    End,
    In,
    Out,
}

const NO_CLIP: &str = "Source: (no clip)";

/// Fastest J/L shuttle speed.
const MAX_SHUTTLE: f64 = 8.0;

/// Speed after a J/K/L press (`None` = stop).
pub fn shuttle_speed(playing: bool, speed: f64, press: Shuttle) -> Option<f64> {
    let dir = match press {
        Shuttle::Stop => return None,
        Shuttle::Forward => 1.0,
        Shuttle::Reverse => -1.0,
    };
    if playing && speed.signum() == dir {
        Some((speed * 2.0).clamp(-MAX_SHUTTLE, MAX_SHUTTLE))
    } else {
        Some(dir)
    }
}

pub struct SourceMonitor {
    player: Player,
    pub item: Option<MediaId>,
    /// Panel tab label: "Source: <clip name>".
    pub title: String,
    /// The frame on screen. Frames are allocated (uploaded) before they replace it, so
    /// the monitor never draws an empty frame while a texture uploads.
    pub image: Option<Allocation>,
    /// Frames handed out for allocation; stale allocations arriving late are dropped.
    frames_sent: u64,
    frame_shown: u64,
    pub status: Status,
    pub in_point: Option<f64>,
    pub out_point: Option<f64>,
    /// Plays the clip with sound at 1x (the `player` covers scrubbing, stepping and other
    /// speeds, silently). The clip as a one-clip sequence, if it has audio.
    sound: SequencePlayer,
    sound_clip: Option<(Sequence, HashMap<MediaId, PathBuf>)>,
    with_sound: bool,
    /// No pictures: the monitor shows the waveform and only `sound` plays it.
    pub audio_only: bool,
}

/// What the Source monitor needs to know about a clip besides its path.
pub struct ClipInfo {
    pub has_video: bool,
    pub has_audio: bool,
    pub rate: FrameRate,
    pub duration: f64,
}

impl SourceMonitor {
    pub fn new() -> Self {
        Self::with_player(Player::new(MONITOR_SIZE))
    }

    fn with_player(player: Player) -> Self {
        Self {
            player,
            item: None,
            title: NO_CLIP.into(),
            image: None,
            frames_sent: 0,
            frame_shown: 0,
            status: Status::default(),
            in_point: None,
            out_point: None,
            sound: SequencePlayer::new(),
            sound_clip: None,
            with_sound: false,
            audio_only: false,
        }
    }

    pub fn load(&mut self, id: MediaId, name: &str, path: PathBuf, info: ClipInfo) {
        self.stop_sound();
        self.sound_clip = info.has_audio.then(|| {
            let mut seq = Sequence::new(name, info.rate, (1, 1), 1, 1);
            let part = ClipSource {
                media: id,
                name: name.into(),
                source_in: 0,
                duration: info.rate.snap(from_seconds(info.duration)).max(info.rate.frame()),
                media_duration: None,
            };
            let _ = seq.overwrite(0, info.has_video.then_some(&part), Some(&part));
            (seq, HashMap::from([(id, path.clone())]))
        });
        self.item = Some(id);
        self.title = format!("Source: {name}");
        self.image = None;
        self.in_point = None;
        self.out_point = None;
        self.audio_only = !info.has_video;
        if self.audio_only {
            // Nothing for the picture player to do: the monitor keeps time itself.
            self.status = Status {
                path: Some(path),
                duration: info.duration,
                rate: jd_media::Rate { num: info.rate.num as i32, den: info.rate.den as i32 },
                ..Status::default()
            };
        } else {
            self.status = Status { path: Some(path.clone()), ..Status::default() };
            self.player.load(path);
        }
    }

    #[cfg(test)]
    fn new_empty() -> Self {
        Self::with_player(Player::new((1, 1)))
    }

    pub fn loaded(&self) -> bool {
        self.item.is_some() && self.status.error.is_none()
    }

    /// Picks up the newest status, and the newest frame (to allocate, then [`Self::present`]).
    pub fn poll(&mut self) -> Option<(u64, image::Handle)> {
        if self.with_sound {
            return self.poll_sound();
        }
        if self.audio_only {
            return None;
        }
        self.status = self.player.status();
        let frame = self.player.take_frame()?;
        self.frames_sent += 1;
        Some((self.frames_sent, to_handle(frame)))
    }

    /// Shows an allocated frame, unless a newer one is already up.
    pub fn present(&mut self, number: u64, frame: Allocation) {
        if number > self.frame_shown {
            self.frame_shown = number;
            self.image = Some(frame);
        }
    }

    /// Keep polling (playing, or a command still being decoded).
    pub fn busy(&self) -> bool {
        self.item.is_some() && (self.with_sound || (!self.audio_only && self.player.busy()))
    }

    fn poll_sound(&mut self) -> Option<(u64, image::Handle)> {
        self.status.position = to_seconds(self.sound.position()).min(self.last_frame());
        if !self.sound.playing() {
            // Reached the end: hand back to the silent player at the same frame.
            self.with_sound = false;
            self.status.playing = false;
            if !self.audio_only {
                self.player.seek(self.status.position);
            }
        }
        let frame = self.sound.take_frame()??;
        self.frames_sent += 1;
        Some((self.frames_sent, to_handle(frame)))
    }

    fn stop_sound(&mut self) {
        if std::mem::take(&mut self.with_sound) {
            self.sound.stop();
            self.status.position = to_seconds(self.sound.position()).min(self.last_frame());
            if !self.audio_only {
                self.player.seek(self.status.position);
            }
        }
    }

    /// Peak levels of the clip's sound since the last call.
    pub fn take_peaks(&self) -> [f32; 2] {
        self.sound.take_peaks()
    }

    pub fn frame_duration(&self) -> f64 {
        1.0 / self.status.rate.as_f64().max(1.0)
    }

    /// The last frame's time (what "end" means for seeking and Out marks).
    pub fn last_frame(&self) -> f64 {
        (self.status.duration - self.frame_duration()).max(0.0)
    }

    pub fn shuttle(&mut self, press: Shuttle) {
        match shuttle_speed(self.status.playing, self.status.speed, press) {
            Some(speed) => self.play(speed),
            None => self.pause(),
        }
    }

    pub fn toggle_play(&mut self) {
        if self.status.playing { self.pause() } else { self.play(1.0) }
    }

    fn play(&mut self, speed: f64) {
        if !self.loaded() {
            return;
        }
        // Update now so a second quick press doubles the speed before the next poll.
        let from = if self.status.position >= self.last_frame() { 0.0 } else { self.status.position };
        self.stop_sound();
        self.status.playing = true;
        self.status.speed = speed;
        match &self.sound_clip {
            // Normal speed with sound (not in tests: they must stay quiet). Audio-only clips
            // always play this way (silently at other speeds).
            Some((seq, media)) if (speed == 1.0 && !cfg!(test)) || self.audio_only => {
                self.player.pause();
                self.with_sound = true;
                self.sound.play(Request {
                    sequence: seq.clone(),
                    media: media.clone(),
                    from: from_seconds(from),
                    speed: if cfg!(test) && speed == 1.0 { 2.0 } else { speed },
                    max_size: MONITOR_SIZE,
                });
            }
            _ => self.player.play(speed),
        }
    }

    fn pause(&mut self) {
        self.status.playing = false;
        self.stop_sound();
        self.player.pause();
    }

    pub fn step(&mut self, frames: i64) {
        self.stop_sound();
        if self.audio_only {
            self.status.playing = false;
            self.seek(self.status.position + frames as f64 * self.frame_duration());
            return;
        }
        if self.loaded() {
            self.status.playing = false;
            self.player.step(frames);
        }
    }

    pub fn seek(&mut self, time: f64) {
        self.stop_sound();
        if self.loaded() {
            let t = time.clamp(0.0, self.last_frame());
            self.status.position = t;
            if !self.audio_only {
                self.player.seek(t);
            }
        }
    }

    /// Dragging on the scrub bar: `fraction` of the clip. Stops playback, like Premiere.
    pub fn scrub(&mut self, fraction: f64) {
        if self.status.playing {
            self.pause();
        }
        self.seek(fraction.clamp(0.0, 1.0) * self.last_frame());
    }

    pub fn go_to(&mut self, target: GoTo) {
        let t = match target {
            GoTo::Start => Some(0.0),
            GoTo::End => Some(self.last_frame()),
            GoTo::In => self.in_point,
            GoTo::Out => self.out_point,
        };
        if let Some(t) = t {
            self.pause();
            self.seek(t);
        }
    }

    /// I: In at the current frame (an Out before it is cleared).
    pub fn mark_in(&mut self) {
        if !self.loaded() {
            return;
        }
        let t = self.status.position;
        self.in_point = Some(t);
        if self.out_point.is_some_and(|o| o < t) {
            self.out_point = None;
        }
    }

    /// O: Out at the current frame (an In after it is cleared). The Out frame is included.
    pub fn mark_out(&mut self) {
        if !self.loaded() {
            return;
        }
        let t = self.status.position;
        self.out_point = Some(t);
        if self.in_point.is_some_and(|i| i > t) {
            self.in_point = None;
        }
    }

    pub fn clear_in_out(&mut self) {
        self.in_point = None;
        self.out_point = None;
    }

    /// Length between the marks (whole clip without them), counting the Out frame.
    pub fn marked_duration(&self) -> f64 {
        let start = self.in_point.unwrap_or(0.0);
        let end = self.out_point.unwrap_or(self.last_frame());
        (end - start + self.frame_duration()).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_plays_forward_and_doubles_up_to_8x() {
        let mut speed = shuttle_speed(false, 1.0, Shuttle::Forward).unwrap();
        assert_eq!(speed, 1.0);
        for expected in [2.0, 4.0, 8.0, 8.0] {
            speed = shuttle_speed(true, speed, Shuttle::Forward).unwrap();
            assert_eq!(speed, expected);
        }
    }

    #[test]
    fn j_reverses_and_switching_direction_restarts_at_1x() {
        assert_eq!(shuttle_speed(true, 4.0, Shuttle::Reverse), Some(-1.0));
        assert_eq!(shuttle_speed(true, -1.0, Shuttle::Reverse), Some(-2.0));
        assert_eq!(shuttle_speed(true, -2.0, Shuttle::Forward), Some(1.0));
        assert_eq!(shuttle_speed(true, -2.0, Shuttle::Stop), None);
    }

    #[test]
    fn marks_keep_in_before_out() {
        let mut s = SourceMonitor::new_empty();
        s.item = Some(1);
        s.status.duration = 10.0;
        s.status.position = 4.0;
        s.mark_out();
        s.status.position = 2.0;
        s.mark_in();
        assert_eq!((s.in_point, s.out_point), (Some(2.0), Some(4.0)));
        // 2.0 to 4.0 inclusive of the Out frame at 25 fps.
        assert!((s.marked_duration() - 2.04).abs() < 1e-9);
        // An In after the Out clears the Out.
        s.status.position = 6.0;
        s.mark_in();
        assert_eq!((s.in_point, s.out_point), (Some(6.0), None));
    }
}
