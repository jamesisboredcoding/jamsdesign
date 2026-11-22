//! Adjustment / filter dialogs: one open at a time, previewed live on the canvas.
//!
//! The document isn't touched while a dialog is open. Previews are rendered on the thread
//! pool from the layer's original pixels; while one renders, slider changes just mark it stale,
//! and the newest settings render next (so dragging never queues up work). OK applies the
//! final result as one undo step; Cancel drops it.

use std::sync::atomic::{AtomicU64, Ordering};

use iced::Task;
use jd_raster::adjust::{self, Adjustment};
use jd_raster::filter::{self, Filter};
use jd_raster::selection::Selection;
use jd_raster::{Document, LayerId, TileGrid};

use crate::app::Message;

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Adjust(Adjustment),
    Filter(Filter),
}

impl Op {
    pub fn name(&self) -> &'static str {
        match self {
            Op::Adjust(a) => a.name(),
            Op::Filter(f) => f.name(),
        }
    }

    pub fn run(&self, grid: &TileGrid, sel: Option<&Selection>) -> TileGrid {
        match self {
            Op::Adjust(a) => adjust::apply(grid, a, sel),
            Op::Filter(f) => filter::apply(grid, f, sel),
        }
    }

    /// Default settings, as when the dialog first opens.
    pub fn default_for(kind: OpKind) -> Self {
        match kind {
            OpKind::BrightnessContrast => Op::Adjust(Adjustment::BrightnessContrast { brightness: 0.0, contrast: 0.0 }),
            OpKind::Levels => Op::Adjust(Adjustment::Levels { in_black: 0, in_white: 255, gamma: 1.0, out_black: 0, out_white: 255 }),
            OpKind::Curves => Op::Adjust(Adjustment::Curves { points: vec![(0, 0), (255, 255)] }),
            OpKind::HueSaturation => Op::Adjust(Adjustment::HueSaturation { hue: 0.0, saturation: 0.0, lightness: 0.0 }),
            OpKind::Threshold => Op::Adjust(Adjustment::Threshold { level: 128 }),
            OpKind::Posterize => Op::Adjust(Adjustment::Posterize { levels: 4 }),
            OpKind::Invert => Op::Adjust(Adjustment::Invert),
            OpKind::Desaturate => Op::Adjust(Adjustment::Desaturate),
            OpKind::GaussianBlur => Op::Filter(Filter::GaussianBlur { radius: 2.0 }),
            OpKind::UnsharpMask => Op::Filter(Filter::UnsharpMask { amount: 100.0, radius: 1.5, threshold: 0 }),
            OpKind::AddNoise => Op::Filter(Filter::AddNoise { amount: 10.0, monochrome: false, seed: 1 }),
            OpKind::Mosaic => Op::Filter(Filter::Mosaic { cell: 12 }),
            OpKind::BoxBlur => Op::Filter(Filter::BoxBlur { radius: 4.0 }),
            OpKind::HighPass => Op::Filter(Filter::HighPass { radius: 10.0 }),
            OpKind::Sharpen => Op::Filter(Filter::Sharpen),
        }
    }
}

/// Every adjustment and filter the menus offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    BrightnessContrast,
    Levels,
    Curves,
    HueSaturation,
    Threshold,
    Posterize,
    Invert,
    Desaturate,
    GaussianBlur,
    UnsharpMask,
    AddNoise,
    Mosaic,
    BoxBlur,
    HighPass,
    Sharpen,
}

impl OpKind {
    /// The adjustments (what adjustment layers can be).
    pub const ADJUSTMENTS: [OpKind; 8] = [
        OpKind::BrightnessContrast,
        OpKind::Levels,
        OpKind::Curves,
        OpKind::HueSaturation,
        OpKind::Threshold,
        OpKind::Posterize,
        OpKind::Invert,
        OpKind::Desaturate,
    ];

    /// Which dialog edits this adjustment (`None`: it has no dialog, like Auto Tone's tables).
    pub fn of(adj: &Adjustment) -> Option<OpKind> {
        Some(match adj {
            Adjustment::BrightnessContrast { .. } => OpKind::BrightnessContrast,
            Adjustment::Levels { .. } => OpKind::Levels,
            Adjustment::Curves { .. } => OpKind::Curves,
            Adjustment::HueSaturation { .. } => OpKind::HueSaturation,
            Adjustment::Threshold { .. } => OpKind::Threshold,
            Adjustment::Posterize { .. } => OpKind::Posterize,
            Adjustment::Invert => OpKind::Invert,
            Adjustment::Desaturate => OpKind::Desaturate,
            Adjustment::ChannelLut(_) => return None,
        })
    }

    /// Applied straight away, without a dialog (like Photoshop's Invert / Desaturate).
    pub fn is_instant(self) -> bool {
        matches!(self, OpKind::Invert | OpKind::Desaturate | OpKind::Sharpen)
    }
}

/// Render generations are unique across all sessions, so a late result from a cancelled dialog
/// can never be mistaken for one of the current dialog's.
fn next_generation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Filters copy the affected area as 16 bytes per pixel (twice with a selection). Past this,
/// refuse instead of risking an out-of-memory abort (which loses unsaved work).
const MAX_FILTER_PIXELS: u64 = 120_000_000;

pub struct Session {
    pub doc: u64,
    pub layer: LayerId,
    /// Working on the layer's mask rather than its pixels.
    pub mask: bool,
    pub kind: OpKind,
    pub op: Op,
    original: TileGrid,
    selection: Option<Selection>,
    /// Luminance histogram of what's being adjusted (for Levels / Curves / Threshold).
    pub histogram: [u32; 256],
    pub preview: Option<TileGrid>,
    pub preview_enabled: bool,
    /// A render is in flight; `stale` means settings changed since it started.
    pub rendering: bool,
    stale: bool,
    /// The first generation this session used; anything older belongs to another session.
    first_generation: u64,
    /// Matches render results to the settings that produced them.
    generation: u64,
    preview_generation: u64,
    /// Editing an adjustment layer (its settings before the dialog opened): changes apply to
    /// the layer live instead of rendering a preview.
    pub layer_original: Option<Adjustment>,
}

impl Session {
    pub fn begin(doc_id: u64, doc: &Document, kind: OpKind) -> Result<Self, &'static str> {
        let layer = doc.active().ok_or("No layer selected")?;
        if layer.locked {
            return Err("The layer is locked");
        }
        if layer.is_group() && !doc.editing_mask() {
            return Err("Groups have no pixels: select a layer inside the group");
        }
        if layer.adjustment.is_some() && !doc.editing_mask() {
            return Err("Adjustment layers have no pixels: double-click the thumbnail to change the adjustment");
        }
        if layer.lock_pixels {
            return Err("The layer's pixels are locked");
        }
        if !layer.visible {
            return Err("The layer is hidden");
        }
        if matches!(Op::default_for(kind), Op::Filter(_)) && doc.width as u64 * doc.height as u64 > MAX_FILTER_PIXELS {
            return Err("This image is too large for filters (over 120 megapixels)");
        }
        let mask = doc.editing_mask();
        let histogram = adjust::histogram(layer.target(mask), doc.selection.as_ref());
        let generation = next_generation();
        Ok(Self {
            doc: doc_id,
            layer: layer.id,
            mask,
            kind,
            op: Op::default_for(kind),
            original: layer.target(mask).clone(),
            selection: doc.selection.clone(),
            histogram,
            preview: None,
            preview_enabled: true,
            rendering: false,
            stale: true,
            generation,
            first_generation: generation,
            preview_generation: 0,
            layer_original: None,
        })
    }

    /// Opens the dialog for an adjustment layer's settings.
    pub fn begin_layer(doc_id: u64, doc: &Document, id: LayerId) -> Result<Self, &'static str> {
        let index = doc.layers().iter().position(|l| l.id == id).ok_or("No layer selected")?;
        let adj = doc.layers()[index].adjustment.clone().ok_or("Not an adjustment layer")?;
        let kind = OpKind::of(&adj).ok_or("This adjustment has no settings to change")?;
        // Levels / Curves show the histogram of what the layer adjusts: everything below.
        let histogram = if index == 0 {
            [0; 256]
        } else {
            let mut below = doc.clone();
            for l in &doc.layers()[index..] {
                below.remove_layer(l.id);
            }
            let flat = jd_raster::composite::flatten(&below);
            adjust::histogram(&TileGrid::from_rgba8(doc.width, doc.height, &flat), None)
        };
        let generation = next_generation();
        Ok(Self {
            doc: doc_id,
            layer: id,
            mask: false,
            kind,
            op: Op::Adjust(adj.clone()),
            original: TileGrid::empty(1, 1),
            selection: None,
            histogram,
            preview: None,
            preview_enabled: true,
            rendering: false,
            stale: false,
            generation,
            first_generation: generation,
            preview_generation: 0,
            layer_original: Some(adj),
        })
    }

    /// The adjustment an adjustment layer should show now (`None`: not editing a layer).
    pub fn layer_adjustment(&self) -> Option<Adjustment> {
        let original = self.layer_original.as_ref()?;
        match (&self.op, self.preview_enabled) {
            (Op::Adjust(a), true) => Some(a.clone()),
            _ => Some(original.clone()),
        }
    }

    pub fn set_op(&mut self, op: Op) {
        if op != self.op {
            self.op = op;
            self.stale = true;
            self.generation = next_generation();
        }
    }

    pub fn reset(&mut self) {
        self.set_op(Op::default_for(self.kind));
    }

    /// Starts a preview render if one is due and none is running.
    pub fn schedule(&mut self) -> Task<Message> {
        if self.rendering || !self.stale || !self.preview_enabled || self.layer_original.is_some() {
            return Task::none();
        }
        self.rendering = true;
        self.stale = false;
        let (op, grid, sel, generation) = (self.op.clone(), self.original.clone(), self.selection.clone(), self.generation);
        Task::perform(async move { op.run(&grid, sel.as_ref()) }, move |result| Message::AdjustPreviewReady(generation, Box::new(result)))
    }

    /// A render finished. Keeps it if it's for the current settings, then renders again if
    /// settings moved on meanwhile.
    pub fn preview_ready(&mut self, generation: u64, grid: TileGrid) -> Task<Message> {
        // Not ours (an earlier, cancelled dialog's render): ignore it entirely.
        if !self.rendering || generation < self.first_generation {
            return Task::none();
        }
        self.rendering = false;
        if generation == self.generation {
            self.preview = Some(grid);
            self.preview_generation = generation;
        } else {
            self.stale = true;
        }
        self.schedule()
    }

    pub fn toggle_preview(&mut self) -> Task<Message> {
        self.preview_enabled = !self.preview_enabled;
        if self.preview_enabled && self.preview_generation != self.generation {
            self.stale = true;
        }
        self.schedule()
    }

    /// What the canvas should show for the layer right now, if not its own pixels.
    pub fn shown(&self) -> Option<&TileGrid> {
        self.preview.as_ref().filter(|_| self.preview_enabled)
    }

    /// The final pixels: the preview if it's current, otherwise rendered now.
    pub fn result(&self) -> TileGrid {
        match &self.preview {
            Some(p) if self.preview_generation == self.generation => p.clone(),
            _ => self.op.run(&self.original, self.selection.as_ref()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Document {
        let mut d = Document::from_rgba8("t", 4, 4, &[100, 100, 100, 255].repeat(16));
        let id = d.active_id().unwrap();
        d.layer_mut(id).unwrap().locked = false;
        d
    }

    #[test]
    fn stale_results_are_dropped_and_rerendered() {
        let mut s = Session::begin(1, &doc(), OpKind::Levels).unwrap();
        let _ = s.schedule();
        assert!(s.rendering);
        // Settings change while rendering: the result for the old settings isn't shown.
        s.set_op(Op::Adjust(Adjustment::Invert));
        let old = s.first_generation;
        let _ = s.preview_ready(old, TileGrid::empty(4, 4));
        assert!(s.preview.is_none());
        assert!(s.rendering, "re-rendered with the new settings");
        let _ = s.preview_ready(s.generation, TileGrid::empty(4, 4));
        assert!(s.preview.is_some() && !s.rendering);
    }

    #[test]
    fn results_from_another_session_are_ignored() {
        let mut blur = Session::begin(1, &doc(), OpKind::GaussianBlur).unwrap();
        let _ = blur.schedule();
        let stale = blur.generation;
        let mut levels = Session::begin(1, &doc(), OpKind::Levels).unwrap();
        let _ = levels.schedule();
        let _ = levels.preview_ready(stale, TileGrid::empty(99, 99));
        assert!(levels.preview.is_none());
        assert!(levels.rendering, "still waiting for its own render");
    }

    #[test]
    fn result_renders_if_preview_is_behind() {
        let mut s = Session::begin(1, &doc(), OpKind::Levels).unwrap();
        s.set_op(Op::Adjust(Adjustment::Invert));
        assert_eq!(s.result().pixel(0, 0), [155, 155, 155, 255]);
    }

    #[test]
    fn locked_layers_refuse() {
        let d = Document::from_rgba8("t", 2, 2, &[0; 16]);
        assert!(Session::begin(1, &d, OpKind::Curves).is_err());
    }
}
