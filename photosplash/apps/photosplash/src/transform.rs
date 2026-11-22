//! Move / Free Transform sessions: what's being transformed and how the handles drive it.
//!
//! While a session is open the document is untouched; the canvas previews it on the GPU (layer
//! transform). Committing resamples on the CPU (jd_raster::transform) as one undo step.

use iced::Point;
use jd_raster::selection::{self, Selection};
use jd_raster::transform::Affine;
use jd_raster::{Document, LayerId, TileGrid};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Move tool drag or arrow-key nudge: translation only, commits on release.
    Move,
    /// Ctrl+T: handles for scale/rotate, commits on Enter.
    Free,
}

/// What part of the transform box the pointer grabbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    Inside,
    /// 0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left.
    Corner(usize),
    /// 0 top, 1 right, 2 bottom, 3 left.
    Edge(usize),
    Rotate,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub center: (f32, f32),
    pub angle: f32,
    pub sx: f32,
    pub sy: f32,
}

#[derive(Debug, Clone)]
struct Grab {
    handle: Handle,
    start: Point,
    params: Params,
}

/// Lines that moved or resized content snaps to (document px): vertical lines at `x`,
/// horizontal lines at `y`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SnapLines {
    pub x: Vec<f32>,
    pub y: Vec<f32>,
}

impl SnapLines {
    /// The box edges/centre (or a single point) along one axis, snapped: the shift to apply and
    /// the line it snapped to, if any line is within `tol`.
    fn nearest(lines: &[f32], values: &[f32], tol: f32) -> Option<(f32, f32)> {
        let mut best: Option<(f32, f32)> = None;
        for &v in values {
            for &l in lines {
                let d = l - v;
                if d.abs() <= tol && best.is_none_or(|(b, _)| d.abs() < b.abs()) {
                    best = Some((d, l));
                }
            }
        }
        best
    }
}

pub struct Session {
    pub doc: u64,
    pub layer: LayerId,
    pub kind: Kind,
    /// The layer with the transformed part removed (when moving a selection).
    pub base: Option<TileGrid>,
    /// The pixels being transformed.
    pub content: TileGrid,
    /// The layer's mask, transformed along with a whole layer (Photoshop's linked mask).
    pub mask: Option<TileGrid>,
    /// Moving a group: every layer inside it (pixels and mask), transformed together.
    pub members: Vec<(LayerId, TileGrid, Option<TileGrid>)>,
    /// The selection at the start; it's transformed along with the pixels.
    pub selection: Option<Selection>,
    /// Content box before transforming: x0, y0, x1, y1 in document pixels.
    pub bounds: [f32; 4],
    pub params: Params,
    grab: Option<Grab>,
    /// What to snap to (set by the app when the session starts).
    pub snap: SnapLines,
    /// Snap distance in document pixels; 0 turns snapping off.
    pub snap_tolerance: f32,
    /// Lines snapped to by the last drag, drawn as guides: (vertical, position).
    pub guides: Vec<(bool, f32)>,
}

impl Session {
    /// Starts transforming the active layer (or the selected part of it). `None` if there's
    /// nothing to transform.
    pub fn begin(doc_id: u64, doc: &Document, kind: Kind) -> Result<Self, &'static str> {
        let layer = doc.active().ok_or("No layer selected")?;
        if layer.locked {
            return Err("The layer is locked");
        }
        if layer.lock_position {
            return Err("The layer's position is locked");
        }
        if doc.selection.is_some() && doc.editing_mask() {
            return Err("Moving part of a layer mask isn't supported yet (deselect to move the whole layer)");
        }
        if layer.is_group() {
            if doc.selection.is_some() {
                return Err("Deselect to move a group (it moves everything inside it)");
            }
            let index = doc.index_of_layer(layer.id).ok_or("No layer selected")?;
            let members: Vec<(LayerId, TileGrid, Option<TileGrid>)> = doc.layers()[doc.block(index)]
                .iter()
                .filter(|l| !l.is_group() && l.adjustment.is_none())
                .map(|l| (l.id, l.pixels.clone(), l.mask.as_ref().map(|m| m.pixels.clone())))
                .collect();
            let [x0, y0, x1, y1] = members
                .iter()
                .filter_map(|(_, p, _)| jd_raster::transform::content_bounds_all(p))
                .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
                .ok_or("Nothing to transform here")?;
            let bounds = [x0 as f32, y0 as f32, x1 as f32, y1 as f32];
            return Ok(Self {
                doc: doc_id,
                layer: layer.id,
                kind,
                mask: None,
                members,
                base: None,
                content: TileGrid::empty(doc.width, doc.height),
                selection: None,
                bounds,
                params: Params { center: centre(bounds), angle: 0.0, sx: 1.0, sy: 1.0 },
                grab: None,
                snap: SnapLines::default(),
                snap_tolerance: 0.0,
                guides: Vec::new(),
            });
        }
        let (base, content) = match &doc.selection {
            Some(sel) => {
                let content = selection::extract(&layer.pixels, sel);
                let mut base = layer.pixels.clone();
                selection::clear(&mut base, sel);
                (Some(base), content)
            }
            None => (None, layer.pixels.clone()),
        };
        // Includes content off the canvas, so it can be moved back into view.
        let [x0, y0, x1, y1] = jd_raster::transform::content_bounds_all(&content).ok_or("Nothing to transform here")?;
        let bounds = [x0 as f32, y0 as f32, x1 as f32, y1 as f32];
        Ok(Self {
            doc: doc_id,
            layer: layer.id,
            kind,
            mask: if doc.selection.is_none() { layer.mask.as_ref().map(|m| m.pixels.clone()) } else { None },
            members: Vec::new(),
            base,
            content,
            selection: doc.selection.clone(),
            bounds,
            params: Params { center: centre(bounds), angle: 0.0, sx: 1.0, sy: 1.0 },
            grab: None,
            snap: SnapLines::default(),
            snap_tolerance: 0.0,
            guides: Vec::new(),
        })
    }

    pub fn affine(&self) -> Affine {
        let p = self.params;
        Affine::from_parts(centre(self.bounds), p.center, p.angle, p.sx, p.sy)
    }

    /// Box corners after the transform, in document pixels (TL, TR, BR, BL).
    pub fn corners(&self) -> [Point; 4] {
        let [x0, y0, x1, y1] = self.bounds;
        let m = self.affine();
        [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| {
            let (a, b) = m.apply(x, y);
            Point::new(a, b)
        })
    }

    pub fn is_identity(&self) -> bool {
        self.affine() == Affine::IDENTITY || self.affine().integer_translation() == Some((0, 0))
    }

    /// Flips and/or turns by quarter turns clockwise (Edit > Transform). The box is snapped so
    /// its corners land on whole pixels, which keeps the result pixel-exact.
    pub fn orient(&mut self, flip_h: bool, flip_v: bool, quarter_turns: u32) {
        let (w, h) = (self.bounds[2] - self.bounds[0], self.bounds[3] - self.bounds[1]);
        let (cx, cy) = centre(self.bounds);
        let q = quarter_turns % 4;
        let (bw, bh) = if q % 2 == 1 { (h, w) } else { (w, h) };
        self.params = Params {
            center: ((cx - bw / 2.0).round() + bw / 2.0, (cy - bh / 2.0).round() + bh / 2.0),
            angle: q as f32 * std::f32::consts::FRAC_PI_2,
            sx: if flip_h { -1.0 } else { 1.0 },
            sy: if flip_v { -1.0 } else { 1.0 },
        };
    }

    /// Nudges by whole pixels.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.params.center.0 += dx;
        self.params.center.1 += dy;
    }

    pub fn grab(&mut self, handle: Handle, at: Point) {
        self.grab = Some(Grab { handle, start: at, params: self.params });
    }

    pub fn release(&mut self) {
        self.grab = None;
        self.guides.clear();
    }

    pub fn is_grabbing(&self) -> bool {
        self.grab.is_some()
    }

    /// Drags the grabbed handle to `at` (document pixels). `shift`: free aspect for corners /
    /// 15° steps for rotation. `alt`: scale around the centre.
    pub fn drag(&mut self, mut at: Point, shift: bool, alt: bool) {
        let Some(g) = self.grab.clone() else { return };
        self.guides.clear();
        let tol = self.snap_tolerance;
        let p0 = g.params;
        let half = ((self.bounds[2] - self.bounds[0]) / 2.0, (self.bounds[3] - self.bounds[1]) / 2.0);
        let (sin, cos) = p0.angle.sin_cos();
        // World vector -> box-local (unrotated) frame, and back.
        let to_local = |v: (f32, f32)| (v.0 * cos + v.1 * sin, -v.0 * sin + v.1 * cos);
        let to_world = |v: (f32, f32)| (v.0 * cos - v.1 * sin, v.0 * sin + v.1 * cos);

        match g.handle {
            Handle::Inside => {
                let mut dx = at.x - g.start.x;
                let mut dy = at.y - g.start.y;
                if self.kind == Kind::Move {
                    // Plain moves stay on whole pixels, so they're lossless.
                    dx = dx.round();
                    dy = dy.round();
                }
                self.params.center = (p0.center.0 + dx, p0.center.1 + dy);
                if tol > 0.0 {
                    // Snap the box's edges or centre (whichever is closest) on each axis.
                    let c = self.corners();
                    let (x0, x1) = (c.iter().map(|p| p.x).fold(f32::MAX, f32::min), c.iter().map(|p| p.x).fold(f32::MIN, f32::max));
                    let (y0, y1) = (c.iter().map(|p| p.y).fold(f32::MAX, f32::min), c.iter().map(|p| p.y).fold(f32::MIN, f32::max));
                    if let Some((d, line)) = SnapLines::nearest(&self.snap.x, &[x0, (x0 + x1) / 2.0, x1], tol) {
                        self.params.center.0 += d;
                        self.guides.push((true, line));
                    }
                    if let Some((d, line)) = SnapLines::nearest(&self.snap.y, &[y0, (y0 + y1) / 2.0, y1], tol) {
                        self.params.center.1 += d;
                        self.guides.push((false, line));
                    }
                    if self.kind == Kind::Move {
                        self.params.center.0 = p0.center.0 + (self.params.center.0 - p0.center.0).round();
                        self.params.center.1 = p0.center.1 + (self.params.center.1 - p0.center.1).round();
                    }
                }
            }
            Handle::Rotate => {
                let (cx, cy) = p0.center;
                let a0 = (g.start.y - cy).atan2(g.start.x - cx);
                let a1 = (at.y - cy).atan2(at.x - cx);
                let mut angle = p0.angle + (a1 - a0);
                if shift {
                    let step = std::f32::consts::PI / 12.0;
                    angle = (angle / step).round() * step;
                }
                self.params.angle = angle;
            }
            Handle::Corner(_) | Handle::Edge(_) => {
                // Which sides move: unit signs in box space; 0 means that axis is fixed.
                let (ux, uy): (f32, f32) = match g.handle {
                    Handle::Corner(0) => (-1.0, -1.0),
                    Handle::Corner(1) => (1.0, -1.0),
                    Handle::Corner(2) => (1.0, 1.0),
                    Handle::Corner(_) => (-1.0, 1.0),
                    Handle::Edge(0) => (0.0, -1.0),
                    Handle::Edge(1) => (1.0, 0.0),
                    Handle::Edge(2) => (0.0, 1.0),
                    Handle::Edge(_) => (-1.0, 0.0),
                    Handle::Inside | Handle::Rotate => unreachable!("handled above"),
                };
                // Snap the dragged side to nearby lines (only while the box is upright).
                if tol > 0.0 && (p0.angle % std::f32::consts::FRAC_PI_2).abs() < 1e-4 {
                    if ux != 0.0
                        && let Some((d, line)) = SnapLines::nearest(&self.snap.x, &[at.x], tol)
                    {
                        at.x += d;
                        self.guides.push((true, line));
                    }
                    if uy != 0.0
                        && let Some((d, line)) = SnapLines::nearest(&self.snap.y, &[at.y], tol)
                    {
                        at.y += d;
                        self.guides.push((false, line));
                    }
                }
                let q = to_local((at.x - p0.center.0, at.y - p0.center.1));
                // Fixed point (opposite side, or the centre with Alt), box-local.
                let anchor = if alt { (0.0, 0.0) } else { (-ux * half.0 * p0.sx, -uy * half.1 * p0.sy) };
                let span = if alt { 1.0 } else { 2.0 };
                let mut sx = if ux != 0.0 { (q.0 - anchor.0) / (span * ux * half.0) } else { p0.sx };
                let mut sy = if uy != 0.0 { (q.1 - anchor.1) / (span * uy * half.1) } else { p0.sy };
                // Corners keep proportions unless Shift (Photoshop's current default).
                if ux != 0.0 && uy != 0.0 && !shift {
                    let (rx, ry) = (sx / p0.sx, sy / p0.sy);
                    let k = if (rx - 1.0).abs() > (ry - 1.0).abs() { rx } else { ry };
                    sx = p0.sx * k;
                    sy = p0.sy * k;
                }
                let clamp = |s: f32| if s.abs() < 0.001 { 0.001f32.copysign(s) } else { s };
                sx = clamp(sx);
                sy = clamp(sy);
                // New centre: midway between the anchor and the moved side, back in world space.
                let moved = (anchor.0 + span * ux * half.0 * sx, anchor.1 + span * uy * half.1 * sy);
                let centre_local = if alt {
                    (0.0, 0.0)
                } else {
                    (
                        if ux != 0.0 { (anchor.0 + moved.0) / 2.0 } else { 0.0 },
                        if uy != 0.0 { (anchor.1 + moved.1) / 2.0 } else { 0.0 },
                    )
                };
                let c = to_world(centre_local);
                self.params = Params { center: (p0.center.0 + c.0, p0.center.1 + c.1), angle: p0.angle, sx, sy };
            }
        }
    }

    /// Applies the transform to the document's layer (and selection).
    pub fn commit(&self, doc: &mut Document) {
        let m = self.affine();
        if !self.members.is_empty() {
            for (id, pixels, mask) in &self.members {
                let Some(l) = doc.layer_mut(*id) else { continue };
                l.pixels = jd_raster::transform::transform_grid(pixels, &m);
                if let (Some(lm), Some(grid)) = (l.mask.as_mut(), mask) {
                    lm.pixels = jd_raster::transform::transform_grid(grid, &m);
                }
            }
            return;
        }
        let moved = jd_raster::transform::transform_grid(&self.content, &m);
        let Some(layer) = doc.layer_mut(self.layer) else { return };
        layer.pixels = match &self.base {
            Some(base) => {
                let mut out = base.clone();
                jd_raster::transform::composite_over(&mut out, &moved);
                out
            }
            None => moved,
        };
        if let (Some(mask), Some(m_grid)) = (layer.mask.as_mut(), &self.mask) {
            mask.pixels = jd_raster::transform::transform_grid(m_grid, &m);
        }
        doc.selection = self
            .selection
            .as_ref()
            .map(|s| jd_raster::transform::transform_selection(s, &m))
            .filter(|s| !s.is_empty());
    }
}

fn centre(b: [f32; 4]) -> (f32, f32) {
    ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_with_square() -> Document {
        let mut doc = Document::new("t", 200, 200);
        let id = doc.active_id().unwrap();
        let l = doc.layer_mut(id).unwrap();
        l.locked = false;
        for y in 50..100 {
            for x in 50..100 {
                l.pixels.set_pixel(x, y, [10, 200, 10, 255]);
            }
        }
        doc
    }

    #[test]
    fn content_bounds_drive_the_box() {
        let s = Session::begin(1, &doc_with_square(), Kind::Free).unwrap();
        assert_eq!(s.bounds, [50.0, 50.0, 100.0, 100.0]);
        assert_eq!(s.corners()[2], Point::new(100.0, 100.0));
    }

    #[test]
    fn corner_drag_is_proportional_and_anchored_opposite() {
        let mut s = Session::begin(1, &doc_with_square(), Kind::Free).unwrap();
        s.grab(Handle::Corner(2), Point::new(100.0, 100.0));
        s.drag(Point::new(150.0, 120.0), false, false);
        let c = s.corners();
        assert!((c[0].x - 50.0).abs() < 1e-3 && (c[0].y - 50.0).abs() < 1e-3, "{c:?}");
        assert!((s.params.sx - s.params.sy).abs() < 1e-4);
        assert!((s.params.sx - 2.0).abs() < 1e-3);
        // Shift frees the aspect ratio.
        s.drag(Point::new(150.0, 120.0), true, false);
        assert!((s.params.sx - 2.0).abs() < 1e-3 && (s.params.sy - 1.4).abs() < 1e-3);
    }

    #[test]
    fn alt_scales_around_centre_and_edges_move_one_axis() {
        let mut s = Session::begin(1, &doc_with_square(), Kind::Free).unwrap();
        s.grab(Handle::Edge(1), Point::new(100.0, 75.0));
        s.drag(Point::new(125.0, 75.0), false, true);
        assert_eq!(s.params.center, (75.0, 75.0));
        assert!((s.params.sx - 2.0).abs() < 1e-3 && s.params.sy == 1.0);
    }

    #[test]
    fn rotate_snaps_with_shift() {
        let mut s = Session::begin(1, &doc_with_square(), Kind::Free).unwrap();
        s.grab(Handle::Rotate, Point::new(150.0, 75.0));
        s.drag(Point::new(75.0, 151.0), true, false);
        assert!((s.params.angle - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
    }

    #[test]
    fn move_with_selection_lifts_and_drops_pixels() {
        let mut doc = doc_with_square();
        doc.selection = Some(Selection::rect(200, 200, 50.0, 50.0, 75.0, 100.0));
        let mut s = Session::begin(1, &doc, Kind::Move).unwrap();
        s.grab(Handle::Inside, Point::new(60.0, 60.0));
        s.drag(Point::new(110.4, 60.0), false, false);
        s.commit(&mut doc);
        let px = |x, y| doc.layers()[0].pixels.pixel(x, y);
        assert_eq!(px(55, 60)[3], 0, "hole left behind");
        assert_eq!(px(80, 60), [10, 200, 10, 255], "unselected half stays");
        assert_eq!(px(105, 60), [10, 200, 10, 255], "moved by 50");
        assert_eq!(doc.selection.as_ref().unwrap().bounds(), Some([100, 50, 125, 100]));
    }

    #[test]
    fn orient_flips_and_rotates_exactly() {
        let mut doc = Document::new("t", 200, 200);
        let id = doc.active_id().unwrap();
        let l = doc.layer_mut(id).unwrap();
        l.locked = false;
        // Away from the edges, so the turned shape stays on the canvas.
        for x in 80..110 {
            l.pixels.set_pixel(x, 100, [255, 0, 0, 255]);
        }
        l.pixels.set_pixel(80, 101, [0, 0, 255, 255]);
        let mut s = Session::begin(1, &doc, Kind::Free).unwrap();
        s.orient(false, false, 1);
        s.commit(&mut doc);
        let px = |d: &Document, x, y| d.layers()[0].pixels.pixel(x, y);
        // A 30×2 box rotated 90° CW becomes 2×30, exact colours, no blur.
        assert_eq!(jd_raster::transform::content_bounds(&doc.layers()[0].pixels).map(|b| (b[2] - b[0], b[3] - b[1])), Some((2, 30)));
        let reds = (0..200).flat_map(|y| (0..200).map(move |x| (x, y))).filter(|&(x, y)| px(&doc, x, y) == [255, 0, 0, 255]).count();
        assert_eq!(reds, 30);
    }

    #[test]
    fn locked_and_empty_layers_refuse() {
        let doc = Document::new("t", 50, 50);
        assert!(Session::begin(1, &doc, Kind::Move).is_err());
        let mut doc = doc;
        let id = doc.active_id().unwrap();
        doc.layer_mut(id).unwrap().locked = false;
        assert_eq!(Session::begin(1, &doc, Kind::Move).err(), Some("Nothing to transform here"));
    }
}
