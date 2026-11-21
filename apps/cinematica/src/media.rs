//! The project's media bin: imported files, analysed in the background.

use std::path::{Path, PathBuf};

use iced::Task;
use iced::widget::image;
use jd_media::{Frame, MediaInfo};

pub type MediaId = u64;

/// Thumbnail size: the Project panel shows them at 116×65, rendered at 2x for HiDPI.
pub const THUMB_SIZE: (u32, u32) = (232, 130);

/// What the timeline treats a still image's length as (Premiere's default is 5 s).
pub const STILL_DURATION: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Not analysed yet.
    Pending,
    Video,
    Audio,
    Still,
    /// FFmpeg couldn't read it.
    Unreadable,
}

#[derive(Debug, Clone)]
pub struct MediaItem {
    pub id: MediaId,
    pub path: PathBuf,
    pub name: String,
    pub info: Option<MediaInfo>,
    pub thumb: Option<image::Handle>,
    pub error: Option<String>,
}

impl MediaItem {
    pub fn new(id: MediaId, path: PathBuf) -> Self {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        Self { id, path, name, info: None, thumb: None, error: None }
    }

    pub fn kind(&self) -> Kind {
        match (&self.info, &self.error) {
            (_, Some(_)) => Kind::Unreadable,
            (None, None) => Kind::Pending,
            (Some(i), None) => match &i.video {
                Some(v) if v.still => Kind::Still,
                Some(_) => Kind::Video,
                None if i.audio.is_some() => Kind::Audio,
                None => Kind::Unreadable,
            },
        }
    }

    /// Length used for display and editing.
    pub fn duration(&self) -> f64 {
        match self.kind() {
            Kind::Still => STILL_DURATION,
            _ => self.info.as_ref().map_or(0.0, |i| i.duration),
        }
    }

    /// Whole frames per second for timecode (the sequence rate for audio and stills).
    pub fn fps(&self, fallback: u32) -> u32 {
        self.info.as_ref().and_then(|i| i.video.as_ref()).filter(|v| !v.still).map_or(fallback, |v| v.rate.rounded())
    }

    pub fn matches(&self, search: &str) -> bool {
        search.is_empty() || self.name.to_lowercase().contains(&search.to_lowercase())
    }
}

/// Probe result plus a thumbnail (for anything with pictures).
#[derive(Debug, Clone)]
pub struct Analysis {
    pub info: MediaInfo,
    pub thumb: Option<Frame>,
}

pub fn analyse(path: &Path) -> Result<Analysis, String> {
    let info = jd_media::probe(path).map_err(|e| e.to_string())?;
    if info.video.is_none() && info.audio.is_none() {
        return Err("no video or audio in this file".into());
    }
    let thumb = info
        .video
        .as_ref()
        .and_then(|_| jd_media::thumbnail(path, (info.duration * 0.1).min(1.0), THUMB_SIZE).ok());
    Ok(Analysis { info, thumb })
}

pub fn to_handle(frame: Frame) -> image::Handle {
    image::Handle::from_rgba(frame.width, frame.height, frame.rgba)
}

/// Runs blocking work on its own thread and delivers the result as a message.
pub fn background<T: Send + 'static, M: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl Fn(T) -> M + Send + 'static,
) -> Task<Option<M>> {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    Task::perform(rx, move |r| r.ok().map(&done))
}
