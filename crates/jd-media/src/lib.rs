//! Video and audio media for the suite, on top of the system FFmpeg (via `ffmpeg-next`).
//!
//! - [`probe`]: what's in a file (duration, video and audio streams).
//! - [`VideoDecoder`]: frame-accurate seeking and in-order decoding to RGBA8.
//! - [`Player`]: a background thread that plays, scrubs and steps a file, for monitors.
//!
//! Times are seconds from the start of the media (the stream's start offset is removed).
//! Never depends on iced (see the workspace layering rule).

mod audio;
mod decode;
mod output;
mod player;
mod probe;

pub use audio::{AudioDecoder, waveform};
pub use decode::{Frame, VideoDecoder, thumbnail};
pub use output::AudioOutput;
pub use player::{Player, Status};
pub use probe::{AudioInfo, MediaInfo, Rate, VideoInfo, probe};

use std::sync::Once;

/// File extensions offered by import dialogs. FFmpeg decides what actually opens.
pub const IMPORT_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "m4v", "mkv", "webm", "avi", "mts", "m2ts", "mxf", "mpg", "mpeg", "ts", "flv", "wmv", "ogv",
    "mp3", "wav", "flac", "aac", "m4a", "ogg", "opus", "aiff", "aif", "png", "jpg", "jpeg", "tif", "tiff", "webp",
];

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    Ffmpeg(String),
    /// The file has no stream of the kind that was asked for.
    NoVideo,
    NoAudio,
    /// The audio device couldn't be opened.
    Audio(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Ffmpeg(e) => write!(f, "{e}"),
            Error::NoVideo => write!(f, "no video stream"),
            Error::NoAudio => write!(f, "no audio stream"),
            Error::Audio(e) => write!(f, "audio output: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<ffmpeg_next::Error> for Error {
    fn from(e: ffmpeg_next::Error) -> Self {
        Error::Ffmpeg(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Initialises FFmpeg once (safe to call from any thread, any number of times) and keeps
/// its console logging to real errors.
pub fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        ffmpeg_next::init().expect("FFmpeg failed to initialise");
        ffmpeg_next::util::log::set_level(ffmpeg_next::util::log::Level::Error);
    });
}
