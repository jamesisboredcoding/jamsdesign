//! Preferences, kept in `$XDG_CONFIG_HOME/jamsdesign/blobr.json` (inside a Flatpak that is
//! the app's own config folder). Tests never read or write it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Narrowest and widest a side panel can be dragged.
pub const PANEL_MIN: f32 = 200.0;
pub const PANEL_MAX: f32 = 480.0;

/// Choices for how often auto-save runs, in seconds.
pub const AUTOSAVE_STEPS: &[u32] = &[30, 60, 120, 300, 600];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Width of the Layers/Assets panel.
    pub layers_width: f32,
    /// Width of the Design/Prototype/Inspect panel.
    pub design_width: f32,
    /// Layers on the right and Design on the left.
    pub swap_panels: bool,
    /// Hide the side panels and toolbar (Ctrl+\), leaving the canvas.
    pub hide_panels: bool,
    /// Save files that already have a path, every `autosave_secs`.
    pub autosave: bool,
    pub autosave_secs: u32,
    /// Scroll zooms (and Ctrl+scroll pans), instead of the other way round.
    pub scroll_zooms: bool,
    /// Arrow keys move by this many pixels (Shift: ten times as far).
    pub nudge: f64,
    /// Rulers along the canvas's top and left edges (Shift+R).
    pub rulers: bool,
    /// Lines between pixels when zoomed in far (Shift+').
    pub pixel_grid: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            layers_width: 248.0,
            design_width: 272.0,
            swap_panels: false,
            hide_panels: false,
            autosave: false,
            autosave_secs: 60,
            scroll_zooms: false,
            nudge: 1.0,
            rulers: false,
            pixel_grid: true,
        }
    }
}

impl Prefs {
    /// Widths in range and a sane interval, whatever the file said.
    fn sanitized(mut self) -> Self {
        let clamp = |w: f32, d: f32| if w.is_finite() { w.clamp(PANEL_MIN, PANEL_MAX) } else { d };
        let d = Prefs::default();
        self.layers_width = clamp(self.layers_width, d.layers_width);
        self.design_width = clamp(self.design_width, d.design_width);
        self.autosave_secs = self.autosave_secs.clamp(10, 3600);
        if !(self.nudge.is_finite() && self.nudge > 0.0) {
            self.nudge = d.nudge;
        }
        self
    }
}

fn file() -> Option<PathBuf> {
    let base = jd_core::fs::user_dir("XDG_CONFIG_HOME", ".config")?;
    Some(base.join("jamsdesign").join("blobr.json"))
}

/// The saved preferences, or the defaults. Tests always get the defaults.
pub fn load() -> Prefs {
    if cfg!(test) {
        return Prefs::default();
    }
    file()
        .and_then(|f| std::fs::read(f).ok())
        .and_then(|b| serde_json::from_slice::<Prefs>(&b).ok())
        .unwrap_or_default()
        .sanitized()
}

/// Writes the preferences (never from tests).
pub fn save(prefs: &Prefs) {
    if cfg!(test) {
        return;
    }
    let Some(f) = file() else { return };
    if let Some(dir) = f.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(prefs) {
        let _ = std::fs::write(f, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_odd_values_fall_back() {
        let p: Prefs = serde_json::from_str(r#"{"layers_width": 9999, "autosave": true, "nudge": -1}"#).unwrap();
        let p = p.sanitized();
        assert_eq!(p.layers_width, PANEL_MAX);
        assert_eq!(p.design_width, Prefs::default().design_width);
        assert!(p.autosave);
        assert_eq!(p.nudge, 1.0);
    }
}
