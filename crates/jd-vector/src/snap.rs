//! Snapping and smart guides: while a box is dragged its edges and centre lines snap to those
//! of nearby layers, and the lines that line up are returned as guides to draw. Also the
//! distances shown between two boxes (Alt-hover in Figma). All in page coordinates.

use kurbo::{Point, Rect};

use crate::{Document, NodeId};

/// A guide or measurement line to draw, in page coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    pub from: Point,
    pub to: Point,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snap {
    /// How far to shift the box so it lines up.
    pub dx: f64,
    pub dy: f64,
    pub guides: Vec<Line>,
}

/// Which lines of the moving box take part (all three for a move; the dragged edges for a
/// resize).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edges {
    pub x: [bool; 3],
    pub y: [bool; 3],
}

impl Edges {
    pub const ALL: Edges = Edges { x: [true; 3], y: [true; 3] };
}

fn xs(r: Rect) -> [f64; 3] {
    [r.x0, (r.x0 + r.x1) / 2.0, r.x1]
}

fn ys(r: Rect) -> [f64; 3] {
    [r.y0, (r.y0 + r.y1) / 2.0, r.y1]
}

/// Boxes worth snapping to when dragging `moving`: their siblings and the parent itself
/// (unless it's the page), visible and not being dragged.
pub fn targets(doc: &Document, moving: &[NodeId]) -> Vec<Rect> {
    let mut parents: Vec<NodeId> = moving.iter().filter_map(|&id| doc.parent(id)).collect();
    parents.sort();
    parents.dedup();
    let mut out = Vec::new();
    for p in parents {
        if doc.parent(p).is_some()
            && let Some(b) = doc.world_bounds(p)
        {
            out.push(b);
        }
        for &c in doc.children(p) {
            if moving.contains(&c) || !doc.node(c).is_some_and(|n| n.props.visible) {
                continue;
            }
            if let Some(b) = doc.world_bounds(c) {
                out.push(b);
            }
        }
    }
    out
}

/// Snaps `r` to `targets` when a line is within `threshold`, on each axis separately.
pub fn snap(r: Rect, edges: Edges, targets: &[Rect], threshold: f64) -> Snap {
    let best = |mine: [f64; 3], use_: [bool; 3], theirs: &dyn Fn(Rect) -> [f64; 3]| -> Option<f64> {
        let mut best: Option<f64> = None;
        for t in targets {
            for v in theirs(*t) {
                for (m, on) in mine.iter().zip(use_) {
                    let d = v - m;
                    if on && d.abs() <= threshold && best.is_none_or(|b| d.abs() < b.abs()) {
                        best = Some(d);
                    }
                }
            }
        }
        best
    };
    let dx = best(xs(r), edges.x, &xs).unwrap_or(0.0);
    let dy = best(ys(r), edges.y, &ys).unwrap_or(0.0);
    let moved = Rect::new(r.x0 + dx, r.y0 + dy, r.x1 + dx, r.y1 + dy);
    Snap { dx, dy, guides: guides(moved, edges, targets) }
}

/// Lines where `r`'s (active) edges or centres meet those of `targets`.
pub fn guides(r: Rect, edges: Edges, targets: &[Rect]) -> Vec<Line> {
    const EPS: f64 = 1e-6;
    let mut out = Vec::new();
    for t in targets {
        for (m, on) in xs(r).into_iter().zip(edges.x) {
            if on && xs(*t).iter().any(|v| (v - m).abs() < EPS) {
                out.push(Line { from: Point::new(m, r.y0.min(t.y0)), to: Point::new(m, r.y1.max(t.y1)) });
            }
        }
        for (m, on) in ys(r).into_iter().zip(edges.y) {
            if on && ys(*t).iter().any(|v| (v - m).abs() < EPS) {
                out.push(Line { from: Point::new(r.x0.min(t.x0), m), to: Point::new(r.x1.max(t.x1), m) });
            }
        }
    }
    out
}

/// Distances from box `a` to box `b`, as Figma shows them: the gaps between them, or from `a`
/// to `b`'s inner edges when `b` contains `a`. Each line comes with its length.
pub fn measure(a: Rect, b: Rect) -> Vec<(Line, f64)> {
    let mut out = Vec::new();
    let cy = (a.y0.max(b.y0) + a.y1.min(b.y1)) / 2.0;
    let cx = (a.x0.max(b.x0) + a.x1.min(b.x1)) / 2.0;
    let my = if a.y1 < b.y0 || a.y0 > b.y1 { (a.y0 + a.y1) / 2.0 } else { cy };
    let mx = if a.x1 < b.x0 || a.x0 > b.x1 { (a.x0 + a.x1) / 2.0 } else { cx };
    let mut h = |x0: f64, x1: f64| {
        if x1 - x0 > 1e-9 {
            out.push((Line { from: Point::new(x0, my), to: Point::new(x1, my) }, x1 - x0));
        }
    };
    if b.contains_rect(a) {
        h(b.x0, a.x0);
        h(a.x1, b.x1);
    } else if a.x1 <= b.x0 {
        h(a.x1, b.x0);
    } else if b.x1 <= a.x0 {
        h(b.x1, a.x0);
    }
    let mut v = |y0: f64, y1: f64| {
        if y1 - y0 > 1e-9 {
            out.push((Line { from: Point::new(mx, y0), to: Point::new(mx, y1) }, y1 - y0));
        }
    };
    if b.contains_rect(a) {
        v(b.y0, a.y0);
        v(a.y1, b.y1);
    } else if a.y1 <= b.y0 {
        v(a.y1, b.y0);
    } else if b.y1 <= a.y0 {
        v(b.y1, a.y0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_the_nearest_line_and_reports_guides() {
        let target = Rect::new(100.0, 100.0, 200.0, 150.0);
        // Left edge 3px right of the target's left edge; centre lines far apart vertically.
        let s = snap(Rect::new(103.0, 300.0, 133.0, 320.0), Edges::ALL, &[target], 5.0);
        assert_eq!((s.dx, s.dy), (-3.0, 0.0));
        assert_eq!(s.guides, vec![Line { from: Point::new(100.0, 100.0), to: Point::new(100.0, 320.0) }]);
        // Centres: 150 vs 151.
        let s = snap(Rect::new(141.0, 300.0, 161.0, 320.0), Edges::ALL, &[target], 5.0);
        assert_eq!(s.dx, -1.0);
        // Out of range, or the edge isn't being dragged.
        assert_eq!(snap(Rect::new(110.0, 300.0, 130.0, 320.0), Edges::ALL, &[target], 5.0).dx, 0.0);
        let right_only = Edges { x: [false, false, true], y: [false; 3] };
        assert_eq!(snap(Rect::new(103.0, 300.0, 120.0, 320.0), right_only, &[target], 5.0).dx, 0.0);
    }

    #[test]
    fn measures_gaps_and_insets() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let m = measure(a, Rect::new(30.0, 0.0, 40.0, 10.0));
        assert_eq!(m.iter().map(|(_, d)| *d).collect::<Vec<_>>(), vec![20.0]);
        let inside = measure(Rect::new(10.0, 20.0, 20.0, 30.0), Rect::new(0.0, 0.0, 100.0, 50.0));
        assert_eq!(inside.iter().map(|(_, d)| *d).collect::<Vec<_>>(), vec![10.0, 80.0, 20.0, 20.0]);
    }
}
