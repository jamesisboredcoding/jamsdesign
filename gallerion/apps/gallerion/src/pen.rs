//! The Pen and Path Selection tools: drawing and editing the active vector path.
//!
//! Like Photoshop: with the Pen, a click adds a corner point, a click-drag adds a smooth point
//! (dragging out its handles), and clicking the first point closes the shape; Enter or Esc
//! finishes an open one. The Path Selection tool (or Ctrl with the Pen) drags points, handles
//! (Alt: just that handle, making a corner) or, from the curve itself, the whole shape.

use iced::Point;
use jd_raster::Document;
use jd_raster::path::{Anchor, SubPath, VectorPath};

/// What a press on the canvas grabbed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Grab {
    /// A point just added with the Pen: dragging pulls out its handles.
    NewAnchor { sub: usize, idx: usize },
    /// Closing a shape on its first point: dragging shapes the closing curve.
    Close { sub: usize },
    Anchor { sub: usize, idx: usize, last: Point },
    Handle { sub: usize, idx: usize, out: bool, alone: bool },
    Whole { sub: usize, last: Point },
}

/// Pen state between messages.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PenState {
    /// The open piece being drawn (index into the active path's pieces).
    pub open: Option<usize>,
    pub grab: Option<Grab>,
}

fn near(a: (f32, f32), b: Point, tol: f32) -> bool {
    (a.0 - b.x).abs() <= tol && (a.1 - b.y).abs() <= tol
}

/// Distance from `p` to the segment a-b.
fn seg_dist(a: (f32, f32), b: (f32, f32), p: Point) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.x - a.0) * dx + (p.y - a.1) * dy) / len2).clamp(0.0, 1.0) };
    let (x, y) = (a.0 + t * dx, a.1 + t * dy);
    ((p.x - x).powi(2) + (p.y - y).powi(2)).sqrt()
}

/// The active path, created as "Work Path" if there's none (like Photoshop).
pub fn active_path(doc: &mut Document) -> &mut VectorPath {
    let i = match doc.active_path.filter(|&i| i < doc.paths.len()) {
        Some(i) => i,
        None => {
            doc.paths.push(VectorPath::new("Work Path"));
            doc.paths.len() - 1
        }
    };
    doc.active_path = Some(i);
    &mut doc.paths[i]
}

/// What an editing press at `at` (document pixels) grabs on `path`; `tol` is the hit size.
pub fn hit(path: &VectorPath, at: Point, tol: f32, alt: bool) -> Option<Grab> {
    for (sub, s) in path.subpaths.iter().enumerate() {
        for (idx, a) in s.anchors.iter().enumerate() {
            if a.h_out != a.p && near(a.h_out, at, tol) {
                return Some(Grab::Handle { sub, idx, out: true, alone: alt });
            }
            if a.h_in != a.p && near(a.h_in, at, tol) {
                return Some(Grab::Handle { sub, idx, out: false, alone: alt });
            }
        }
    }
    for (sub, s) in path.subpaths.iter().enumerate() {
        for (idx, a) in s.anchors.iter().enumerate() {
            if near(a.p, at, tol) {
                return Some(Grab::Anchor { sub, idx, last: at });
            }
        }
    }
    for (sub, s) in path.subpaths.iter().enumerate() {
        let pts = s.flatten();
        let closing = if s.closed { pts.first().copied() } else { None };
        let on_curve = pts.windows(2).any(|w| seg_dist(w[0], w[1], at) <= tol)
            || closing.zip(pts.last()).is_some_and(|(a, &b)| seg_dist(b, a, at) <= tol);
        if on_curve {
            return Some(Grab::Whole { sub, last: at });
        }
    }
    None
}

/// A Pen click: adds a point to the open piece (or starts one), or closes it on its first
/// point. Returns what dragging from here does.
pub fn pen_click(doc: &mut Document, state: &mut PenState, at: Point, tol: f32) -> Grab {
    let open = state.open;
    let path = active_path(doc);
    if let Some(sub) = open.filter(|&s| s < path.subpaths.len()) {
        let s = &mut path.subpaths[sub];
        if s.anchors.len() >= 2 && s.anchors.first().is_some_and(|a| near(a.p, at, tol)) {
            s.closed = true;
            state.open = None;
            return Grab::Close { sub };
        }
        s.anchors.push(Anchor::corner((at.x, at.y)));
        return Grab::NewAnchor { sub, idx: s.anchors.len() - 1 };
    }
    path.subpaths.push(SubPath { anchors: vec![Anchor::corner((at.x, at.y))], closed: false });
    let sub = path.subpaths.len() - 1;
    state.open = Some(sub);
    Grab::NewAnchor { sub, idx: 0 }
}

/// Applies a drag of `grab` to `at`.
pub fn drag(path: &mut VectorPath, grab: &mut Grab, at: Point, shift: bool) {
    let at_t = (at.x, at.y);
    match grab {
        Grab::NewAnchor { sub, idx } => {
            if let Some(a) = path.subpaths.get_mut(*sub).and_then(|s| s.anchors.get_mut(*idx)) {
                *a = Anchor::smooth(a.p, constrain(a.p, at_t, shift));
            }
        }
        Grab::Close { sub } => {
            if let Some(a) = path.subpaths.get_mut(*sub).and_then(|s| s.anchors.first_mut()) {
                *a = Anchor::smooth(a.p, constrain(a.p, at_t, shift));
            }
        }
        Grab::Anchor { sub, idx, last } => {
            let (dx, dy) = (at.x - last.x, at.y - last.y);
            if let Some(a) = path.subpaths.get_mut(*sub).and_then(|s| s.anchors.get_mut(*idx)) {
                *a = a.moved(dx, dy);
            }
            *last = at;
        }
        Grab::Handle { sub, idx, out, alone } => {
            if let Some(a) = path.subpaths.get_mut(*sub).and_then(|s| s.anchors.get_mut(*idx)) {
                let h = constrain(a.p, at_t, shift);
                let (moved, other) = if *out { (&mut a.h_out, &mut a.h_in) } else { (&mut a.h_in, &mut a.h_out) };
                *moved = h;
                // A smooth point stays smooth: the other handle turns to stay opposite (its
                // length is kept). Alt breaks it into a corner.
                if !*alone && *other != a.p {
                    let len = ((other.0 - a.p.0).powi(2) + (other.1 - a.p.1).powi(2)).sqrt();
                    let (vx, vy) = (a.p.0 - h.0, a.p.1 - h.1);
                    let n = (vx * vx + vy * vy).sqrt();
                    if n > 0.0 {
                        *other = (a.p.0 + vx / n * len, a.p.1 + vy / n * len);
                    }
                }
            }
        }
        Grab::Whole { sub, last } => {
            let (dx, dy) = (at.x - last.x, at.y - last.y);
            if let Some(s) = path.subpaths.get_mut(*sub) {
                for a in &mut s.anchors {
                    *a = a.moved(dx, dy);
                }
            }
            *last = at;
        }
    }
}

/// Shift keeps a handle at 45° steps, like Photoshop.
fn constrain(origin: (f32, f32), p: (f32, f32), shift: bool) -> (f32, f32) {
    if !shift {
        return p;
    }
    let (dx, dy) = (p.0 - origin.0, p.1 - origin.1);
    let len = (dx * dx + dy * dy).sqrt();
    let step = std::f32::consts::FRAC_PI_4;
    let a = (dy.atan2(dx) / step).round() * step;
    (origin.0 + a.cos() * len, origin.1 + a.sin() * len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_drag_close_and_edit() {
        let mut doc = Document::blank("t", 100, 100);
        let mut st = PenState::default();
        let tol = 4.0;
        // Corner, smooth (dragged), corner, then click the first point to close.
        pen_click(&mut doc, &mut st, Point::new(10.0, 10.0), tol);
        let mut g = pen_click(&mut doc, &mut st, Point::new(50.0, 10.0), tol);
        drag(&mut doc.paths[0], &mut g, Point::new(60.0, 10.0), false);
        pen_click(&mut doc, &mut st, Point::new(50.0, 50.0), tol);
        assert_eq!(pen_click(&mut doc, &mut st, Point::new(11.0, 9.0), tol), Grab::Close { sub: 0 });
        assert_eq!(doc.active_path, Some(0));
        assert_eq!(doc.paths[0].name, "Work Path");
        let s = &doc.paths[0].subpaths[0];
        assert!(s.closed && st.open.is_none());
        assert_eq!(s.anchors[1].h_in, (40.0, 10.0), "mirrored handle");

        // Edit: grab the out handle of point 1 and turn it; the other follows.
        let mut g = hit(&doc.paths[0], Point::new(60.0, 10.0), tol, false).unwrap();
        assert!(matches!(g, Grab::Handle { idx: 1, out: true, .. }));
        drag(&mut doc.paths[0], &mut g, Point::new(50.0, 20.0), false);
        assert_eq!(doc.paths[0].subpaths[0].anchors[1].h_in, (50.0, 0.0));
        // Grab a point and move it.
        let mut g = hit(&doc.paths[0], Point::new(50.0, 50.0), tol, false).unwrap();
        drag(&mut doc.paths[0], &mut g, Point::new(55.0, 52.0), false);
        assert_eq!(doc.paths[0].subpaths[0].anchors[2].p, (55.0, 52.0));
        // Grab the curve itself: the whole shape moves.
        let mut g = hit(&doc.paths[0], Point::new(30.0, 10.0), tol, false).unwrap();
        assert!(matches!(g, Grab::Whole { .. }));
        drag(&mut doc.paths[0], &mut g, Point::new(30.0, 15.0), false);
        assert_eq!(doc.paths[0].subpaths[0].anchors[0].p, (10.0, 15.0));
    }
}
