//! Recent files for the Home screen: the list lives in Preferences; each file's thumbnail is
//! cached as a small PNG in `$XDG_CACHE_HOME/jamsdesign/gallerion-thumbs/` when it's opened
//! or saved (so the Home screen never has to open big files to show them).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use jd_raster::Document;

/// How many recent files are kept.
pub const MAX: usize = 24;

/// Thumbnail size (longest side, px).
const THUMB: u32 = 320;

/// Puts `path` first in `list` (once), dropping the oldest past [`MAX`].
pub fn remember(list: &mut Vec<PathBuf>, path: &Path) {
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(MAX);
}

fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("jamsdesign").join("gallerion-thumbs"))
}

/// Where `path`'s thumbnail is cached (named by a hash of the path).
fn thumb_file(path: &Path) -> Option<PathBuf> {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.as_os_str().as_encoded_bytes() {
        h = (h ^ *b as u64).wrapping_mul(0x0100_0000_0100_01b3);
    }
    Some(cache_dir()?.join(format!("{h:016x}.png")))
}

/// Writes `doc`'s thumbnail for `path` on its own thread; finishes when it's written.
pub async fn store_thumb(path: PathBuf, doc: Document) {
    if cfg!(test) {
        return;
    }
    let (done, written) = iced::futures::channel::oneshot::channel::<()>();
    std::thread::spawn(move || {
        write_thumb(&path, &doc);
        let _ = done.send(());
    });
    let _ = written.await;
}

fn write_thumb(path: &Path, doc: &Document) {
    {
        let Some(file) = thumb_file(path) else { return };
        let (w, h, rgba) = jd_raster::composite::thumbnail(doc, THUMB, None);
        let Some(img) = image::RgbaImage::from_raw(w, h, rgba) else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Written beside, then renamed, so a half-written file is never read.
        let tmp = file.with_extension("tmp.png");
        if img.save(&tmp).is_ok() {
            let _ = std::fs::rename(&tmp, &file);
        }
    }
}

/// What the Home screen shows for one recent file.
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    /// RGBA8 thumbnail, if one was cached.
    pub thumb: Option<(u32, u32, Vec<u8>)>,
    pub modified: Option<SystemTime>,
}

/// Reads the thumbnails and dates of `paths`, leaving out files that no longer exist.
pub fn load(paths: Vec<PathBuf>) -> Vec<Entry> {
    paths
        .into_iter()
        .filter_map(|path| {
            let meta = std::fs::metadata(&path).ok()?;
            let thumb = thumb_file(&path).and_then(|f| image::open(f).ok()).map(|i| {
                let i = i.to_rgba8();
                (i.width(), i.height(), i.into_raw())
            });
            Some(Entry { path, thumb, modified: meta.modified().ok() })
        })
        .collect()
}

/// "just now", "5 minutes ago", "yesterday", "3 days ago"...
pub fn ago(t: SystemTime, now: SystemTime) -> String {
    let s = now.duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
    let plural = |n: u64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match s {
        0..60 => "just now".into(),
        60..3600 => plural(s / 60, "minute"),
        3600..86_400 => plural(s / 3600, "hour"),
        86_400..172_800 => "yesterday".into(),
        172_800..2_592_000 => plural(s / 86_400, "day"),
        2_592_000..31_536_000 => plural(s / 2_592_000, "month"),
        _ => plural(s / 31_536_000, "year"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn remembers_newest_first_without_repeats() {
        let mut l = Vec::new();
        for i in 0..30 {
            remember(&mut l, Path::new(&format!("/f{i}")));
        }
        remember(&mut l, Path::new("/f10"));
        assert_eq!(l.len(), MAX);
        assert_eq!(l[0], Path::new("/f10"));
        assert_eq!(l[1], Path::new("/f29"));
        assert_eq!(l.iter().filter(|p| *p == Path::new("/f10")).count(), 1);

        let now = SystemTime::now();
        assert_eq!(ago(now - Duration::from_secs(90), now), "1 minute ago");
        assert_eq!(ago(now - Duration::from_secs(100_000), now), "yesterday");
        assert_eq!(ago(now - Duration::from_secs(3 * 86_400), now), "3 days ago");
    }
}
