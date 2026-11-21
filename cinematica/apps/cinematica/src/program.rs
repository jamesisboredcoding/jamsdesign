//! The Program monitor: shows the sequence's frame at the playhead (the topmost enabled
//! video clip). Playing the sequence back comes with the compositor and audio (M3).

use std::path::PathBuf;

use iced::widget::image;
use iced_runtime::image::Allocation;
use jd_media::Player;

use crate::media::{MediaId, to_handle};

const MONITOR_SIZE: (u32, u32) = (1920, 1080);

pub struct ProgramMonitor {
    /// Panel tab label: "Program: <sequence name>".
    pub title: String,
    player: Player,
    /// Media loaded in the player.
    loaded: Option<MediaId>,
    /// What's on screen; `None` is black (no clip at the playhead).
    pub image: Option<Allocation>,
    /// The playhead is over a clip.
    pub showing: bool,
    frames_sent: u64,
    frame_shown: u64,
}

impl ProgramMonitor {
    pub fn new(sequence_name: &str) -> Self {
        Self { title: format!("Program: {sequence_name}"), player: Player::new(MONITOR_SIZE), loaded: None, image: None, showing: false, frames_sent: 0, frame_shown: 0 }
    }

    /// Shows `media` at `time` seconds, or black for `None`.
    pub fn show(&mut self, target: Option<(MediaId, PathBuf, f64)>) {
        match target {
            Some((id, path, t)) => {
                self.showing = true;
                if self.loaded != Some(id) {
                    self.loaded = Some(id);
                    self.player.load(path);
                }
                self.player.seek(t);
            }
            None => {
                self.showing = false;
                // Frames still in flight belong to the old picture: drop them.
                self.frame_shown = self.frames_sent;
                self.image = None;
            }
        }
    }

    pub fn busy(&self) -> bool {
        self.loaded.is_some() && self.player.busy()
    }

    pub fn poll(&mut self) -> Option<(u64, image::Handle)> {
        let frame = self.player.take_frame()?;
        if !self.showing {
            return None;
        }
        self.frames_sent += 1;
        Some((self.frames_sent, to_handle(frame)))
    }

    pub fn present(&mut self, number: u64, frame: Allocation) {
        if self.showing && number > self.frame_shown {
            self.frame_shown = number;
            self.image = Some(frame);
        }
    }
}
