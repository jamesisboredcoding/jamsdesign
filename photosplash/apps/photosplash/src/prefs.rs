//! Edit > Preferences (Ctrl+K): settings kept between sessions in
//! `$XDG_CONFIG_HOME/jamsdesign/photosplash.json`. A missing or unreadable file just means
//! the defaults; unknown or missing fields fall back to their defaults too.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::app::{Panel, Workspace};

/// What the cursor looks like while painting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BrushCursor {
    /// A circle the size of the brush (Photoshop's "Normal Brush Tip").
    #[default]
    Outline,
    /// The circle plus a crosshair in the middle.
    OutlineCrosshair,
    /// Just a crosshair (Photoshop's "Precise").
    Precise,
}

impl BrushCursor {
    pub const ALL: [BrushCursor; 3] = [BrushCursor::Outline, BrushCursor::OutlineCrosshair, BrushCursor::Precise];

    pub fn name(self) -> &'static str {
        match self {
            BrushCursor::Outline => "Brush size",
            BrushCursor::OutlineCrosshair => "Brush size with crosshair",
            BrushCursor::Precise => "Precise (crosshair)",
        }
    }
}

/// Window and panel arrangement, restored at the next start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedLayout {
    pub workspace: Workspace,
    pub dock_width: f32,
    /// Shown: Color, Properties, Layers.
    pub panels: [bool; 3],
    pub order: Vec<Panel>,
    pub heights: Vec<(Panel, f32)>,
    pub history_open: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Undo steps kept per document.
    pub history_states: u32,
    /// New documents and opened images blend in linear light (Photoshop: perceptual).
    pub linear_blending: bool,
    /// Bring back the workspace and panel layout at the next start.
    pub remember_layout: bool,
    /// The scroll wheel zooms (otherwise it scrolls; Ctrl+wheel zooms).
    pub wheel_zooms: bool,
    pub brush_cursor: BrushCursor,
    /// Pen tablet pressure changes the brush (the options bar toggles this too).
    pub pen_pressure: bool,
    /// Snapping (View > Snap) at startup.
    pub snap: bool,
    /// View > Rulers (remembered, like Photoshop).
    pub rulers: bool,
    /// Grid line spacing (document pixels) and how many parts each square is split into.
    pub grid_spacing: u32,
    pub grid_subdivisions: u32,
    /// Files opened or saved lately, newest first (the Home screen).
    pub recent: Vec<PathBuf>,
    pub layout: Option<SavedLayout>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            history_states: 100,
            linear_blending: false,
            remember_layout: true,
            wheel_zooms: true,
            brush_cursor: BrushCursor::Outline,
            pen_pressure: true,
            snap: true,
            rulers: false,
            grid_spacing: 100,
            grid_subdivisions: 4,
            recent: Vec::new(),
            layout: None,
        }
    }
}

pub const HISTORY_RANGE: std::ops::RangeInclusive<f32> = 5.0..=1000.0;

/// The app used to be called Gallerion: moves its settings, tool presets and thumbnail cache
/// to the new names, once (only when nothing exists under the new name yet).
pub fn migrate_old_names() {
    let dir = jd_core::fs::user_dir;
    let moves = [
        (dir("XDG_CONFIG_HOME", ".config"), "gallerion.json", "photosplash.json"),
        (dir("XDG_CONFIG_HOME", ".config"), "gallerion-tool-presets.json", "photosplash-tool-presets.json"),
        (dir("XDG_CACHE_HOME", ".cache"), "gallerion-thumbs", "photosplash-thumbs"),
    ];
    for (base, old, new) in moves {
        let Some(base) = base.map(|b| b.join("jamsdesign")) else { continue };
        let (old, new) = (base.join(old), base.join(new));
        if old.exists() && !new.exists() {
            let _ = std::fs::rename(old, new);
        }
    }
}

fn path() -> Option<PathBuf> {
    let base = jd_core::fs::user_dir("XDG_CONFIG_HOME", ".config")?;
    Some(base.join("jamsdesign").join("photosplash.json"))
}

/// Grid spacing limits (px) and subdivision limits, for the Preferences boxes.
pub const GRID_RANGE: std::ops::RangeInclusive<f32> = 2.0..=2000.0;
pub const SUBDIVISION_RANGE: std::ops::RangeInclusive<f32> = 1.0..=20.0;

impl Prefs {
    /// Distance between the finest grid lines (subdivisions), document pixels.
    pub fn grid_step(&self) -> f32 {
        self.grid_spacing.max(1) as f32 / self.grid_subdivisions.max(1) as f32
    }

    /// The saved preferences (defaults if there are none). Tests never read the real file.
    pub fn load() -> Self {
        if cfg!(test) {
            return Self::default();
        }
        path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Writes the preferences file (tests never write it).
    pub fn save(&self) -> Result<(), String> {
        if cfg!(test) {
            return Ok(());
        }
        let Some(p) = path() else { return Ok(()) };
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&p, json).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_defaults() {
        let p: Prefs = serde_json::from_str(r#"{"history_states": 40, "something_new": 1}"#).unwrap();
        assert_eq!(p.history_states, 40);
        assert!(p.wheel_zooms && p.remember_layout);
        let back: Prefs = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }
}
