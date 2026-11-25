//! Files on disk: writing without ever leaving a damaged file, and the user's folders.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Writes a file next to `path` (with a name no other write uses), synced to disk. Hand the
/// result to [`commit`] to put it in place; until then `path` is untouched.
pub fn write_temp<E: From<std::io::Error>>(
    path: &Path,
    write: impl FnOnce(BufWriter<File>) -> Result<BufWriter<File>, E>,
) -> Result<PathBuf, E> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.saving-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let result = (|| {
        let mut out = write(BufWriter::new(File::create(&tmp)?))?;
        out.flush()?;
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        Ok(())
    })();
    match result {
        Ok(()) => Ok(tmp),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Puts a file from [`write_temp`] in place of `path` (or removes it if that fails).
pub fn commit(tmp: &Path, path: &Path) -> std::io::Result<()> {
    std::fs::rename(tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(tmp);
    })
}

/// Writes `path` safely: a crash or a full disk mid-write leaves the old file intact.
pub fn write_atomic<E: From<std::io::Error>>(
    path: &Path,
    write: impl FnOnce(BufWriter<File>) -> Result<BufWriter<File>, E>,
) -> Result<(), E> {
    let tmp = write_temp(path, write)?;
    Ok(commit(&tmp, path)?)
}

/// A user folder from the XDG variable `var` (absolute paths only), or `~/<fallback>`:
/// `user_dir("XDG_CONFIG_HOME", ".config")`.
pub fn user_dir(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)))
}

/// Puts `path` first in a recent-files `list` (once), keeping at most `max`.
pub fn remember(list: &mut Vec<PathBuf>, path: &Path, max: usize) {
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(max);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_files_are_newest_first_once_and_capped() {
        let mut l = Vec::new();
        for i in 0..30 {
            remember(&mut l, Path::new(&format!("/f{i}")), 24);
        }
        remember(&mut l, Path::new("/f10"), 24);
        assert_eq!(l.len(), 24);
        assert_eq!((l[0].as_path(), l[1].as_path()), (Path::new("/f10"), Path::new("/f29")));
        assert_eq!(l.iter().filter(|p| *p == Path::new("/f10")).count(), 1);
    }

    #[test]
    fn a_failed_write_keeps_the_old_file() {
        let dir = std::env::temp_dir().join(format!("jd-core-fs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("photo.jpg");
        write_atomic::<std::io::Error>(&path, |mut w| {
            w.write_all(b"old")?;
            Ok(w)
        })
        .unwrap();
        let failed = write_atomic(&path, |mut w| {
            w.write_all(b"half")?;
            Err(std::io::Error::other("disk full"))
        });
        assert!(failed.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temp file left behind");
        std::fs::remove_dir_all(dir).ok();
    }
}
