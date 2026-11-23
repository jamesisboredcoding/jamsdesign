//! Recently opened and saved files, kept in `$XDG_CONFIG_HOME/jamsdesign/blobr-recent.json`
//! (newest first) for File > Open Recent.

use std::path::{Path, PathBuf};

/// How many recent files are kept.
pub const MAX: usize = 10;

fn file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("jamsdesign").join("blobr-recent.json"))
}

/// The saved list, without files that have gone since. Tests start empty.
pub fn load() -> Vec<PathBuf> {
    if cfg!(test) {
        return Vec::new();
    }
    let Some(list) = file().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice::<Vec<PathBuf>>(&b).ok()) else {
        return Vec::new();
    };
    list.into_iter().filter(|p| p.exists()).take(MAX).collect()
}

/// Writes the list (never from tests, so they can't touch the user's).
pub fn save(list: &[PathBuf]) {
    if cfg!(test) {
        return;
    }
    let Some(f) = file() else { return };
    if let Some(dir) = f.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(list) {
        let _ = std::fs::write(f, json);
    }
}

/// Puts `path` first in `list` (once), dropping the oldest past [`MAX`].
pub fn remember(list: &mut Vec<PathBuf>, path: &Path) {
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(MAX);
}

/// The menu's label for a file: its name, and its folder (with `~` for home).
pub fn label(path: &Path) -> (String, String) {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    let dir = path.parent().map(|d| d.display().to_string()).unwrap_or_default();
    let dir = match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && dir.starts_with(&home) => format!("~{}", &dir[home.len()..]),
        _ => dir,
    };
    // Long folders keep their end, which says the most.
    let dir = if dir.chars().count() > 28 { format!("...{}", dir.chars().rev().take(25).collect::<Vec<_>>().into_iter().rev().collect::<String>()) } else { dir };
    (name, dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_once_and_capped() {
        let mut list = Vec::new();
        for i in 0..(MAX + 3) {
            remember(&mut list, Path::new(&format!("/d/{i}.jdb")));
        }
        remember(&mut list, Path::new("/d/5.jdb"));
        assert_eq!(list.len(), MAX);
        assert_eq!(list[0], Path::new("/d/5.jdb"));
        assert_eq!(list.iter().filter(|p| p.ends_with("5.jdb")).count(), 1);
    }
}
