//! Plugins: programs in `$XDG_DATA_HOME/jamsdesign/blobr/plugins/` (any language). The Plugins
//! menu lists the executable ones; running one hands it `BLOBR_SOCKET`, the live socket agents
//! use, so it edits the open document with the same tools (see `blobr/PLUGINS.md`).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn dir() -> Option<PathBuf> {
    Some(jd_core::fs::user_dir("XDG_DATA_HOME", ".local/share")?.join("jamsdesign").join("blobr").join("plugins"))
}

/// The executable files in `dir`, by name.
pub fn list(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
        .collect();
    found.sort();
    found
}

/// "make-buttons.py" -> "make buttons".
pub fn name(path: &Path) -> String {
    path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().replace(['-', '_'], " "))
}

/// Runs a plugin to the end. An error is the last line it printed to stderr, or its exit code.
pub fn run(path: &Path, socket: &Path) -> Result<(), String> {
    let out = Command::new(path)
        .env("BLOBR_SOCKET", socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("couldn't start it: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(err.lines().rev().find(|l| !l.trim().is_empty()).map_or_else(|| format!("it stopped with {}", out.status), |l| l.trim().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_runs_executables() {
        let dir = std::env::temp_dir().join(format!("blobr-plugins-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = dir.join("say-hi.sh");
        let bad = dir.join("broken");
        std::fs::write(&ok, "#!/bin/sh\ntest -n \"$BLOBR_SOCKET\"\n").unwrap();
        std::fs::write(&bad, "#!/bin/sh\necho 'no frame selected' >&2\nexit 3\n").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a plugin").unwrap();
        for p in [&ok, &bad] {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(list(&dir), vec![bad.clone(), ok.clone()]);
        assert_eq!(name(&ok), "say hi");
        let sock = Path::new("/tmp/x.sock");
        assert_eq!(run(&ok, sock), Ok(()));
        assert_eq!(run(&bad, sock), Err("no frame selected".into()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
