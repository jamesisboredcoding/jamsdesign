//! The video editor's document model: a [`Sequence`] of video and audio tracks holding
//! clips, edited by pure functions (insert, overwrite, lift, extract, razor, move, trim,
//! ripple, slip), with snapshot undo in [`History`]. Times are exact [`time::Time`] flicks.
//! Never depends on iced or FFmpeg.

pub mod sequence;
pub mod time;

#[cfg(test)]
mod tests;

pub use jd_core::History;
pub use sequence::{Clip, ClipId, ClipSource, Edge, EditError, LinkId, MediaId, Sequence, Track, TrackKind, TrackRef};
pub use time::{FrameRate, SECOND, Time, from_seconds, to_seconds};
