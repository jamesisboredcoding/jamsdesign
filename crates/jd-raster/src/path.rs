//! Vector paths (the Pen tool and the Paths panel): Bézier curves made of anchor points, each
//! with two handles. A path is turned into pixels only when it's used: loaded as a selection,
//! filled or stroked.

use crate::selection::{Combine, Selection};

pub type Pt = (f32, f32);

/// An anchor point and its two handles (absolute positions; a handle on the anchor means
/// a corner on that side).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    pub p: Pt,
    /// Controls the curve arriving at this anchor.
    pub h_in: Pt,
    /// Controls the curve leaving this anchor.
    pub h_out: Pt,
}

impl Anchor {
    pub fn corner(p: Pt) -> Self {
        Self { p, h_in: p, h_out: p }
    }

    /// A smooth anchor whose outgoing handle is at `out` (the incoming one mirrors it).
    pub fn smooth(p: Pt, out: Pt) -> Self {
        Self { p, h_in: (2.0 * p.0 - out.0, 2.0 * p.1 - out.1), h_out: out }
    }

    pub fn moved(self, dx: f32, dy: f32) -> Self {
        let m = |q: Pt| (q.0 + dx, q.1 + dy);
        Self { p: m(self.p), h_in: m(self.h_in), h_out: m(self.h_out) }
    }
}

/// One continuous piece of a path.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SubPath {
    pub anchors: Vec<Anchor>,
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct VectorPath {
    pub name: String,
    pub subpaths: Vec<SubPath>,
}

fn cubic(a: Pt, b: Pt, c: Pt, d: Pt, t: f32) -> Pt {
    let u = 1.0 - t;
    let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (w0 * a.0 + w1 * b.0 + w2 * c.0 + w3 * d.0, w0 * a.1 + w1 * b.1 + w2 * c.1 + w3 * d.1)
}

impl SubPath {
    /// The curve as points about a pixel apart (for drawing, selections and strokes).
    pub fn flatten(&self) -> Vec<Pt> {
        let n = self.anchors.len();
        let Some(first) = self.anchors.first() else { return Vec::new() };
        let mut out = vec![first.p];
        let segments = if self.closed { n } else { n.saturating_sub(1) };
        for i in 0..segments {
            let (a, b) = (self.anchors[i], self.anchors[(i + 1) % n]);
            let straight = a.h_out == a.p && b.h_in == b.p;
            if straight {
                out.push(b.p);
                continue;
            }
            // Enough steps that each is at most ~2 px along the control polygon.
            let len = dist(a.p, a.h_out) + dist(a.h_out, b.h_in) + dist(b.h_in, b.p);
            let steps = ((len / 2.0).ceil() as usize).clamp(2, 400);
            for k in 1..=steps {
                out.push(cubic(a.p, a.h_out, b.h_in, b.p, k as f32 / steps as f32));
            }
        }
        out
    }
}

fn dist(a: Pt, b: Pt) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

impl VectorPath {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), subpaths: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.subpaths.iter().all(|s| s.anchors.is_empty())
    }

    /// The area inside the path (Photoshop's "Load Path as Selection"). Open pieces are
    /// closed with a straight line, like Photoshop.
    pub fn to_selection(&self, width: u32, height: u32) -> Selection {
        let mut sel = Selection::empty(width, height);
        for s in &self.subpaths {
            let pts = s.flatten();
            if pts.len() >= 3 {
                sel.combine(&Selection::polygon(width, height, &pts), Combine::Add);
            }
        }
        sel
    }

    /// Every piece as points along its curve (closed ones end back at the start).
    pub fn polylines(&self) -> Vec<Vec<Pt>> {
        self.subpaths
            .iter()
            .map(|s| {
                let mut pts = s.flatten();
                if s.closed && let Some(&p) = pts.first() {
                    pts.push(p);
                }
                pts
            })
            .filter(|p| p.len() >= 2)
            .collect()
    }

    /// Every point moved by `f` (canvas crops, resizes, rotations...).
    pub fn map(&mut self, f: impl Fn(Pt) -> Pt) {
        for a in self.subpaths.iter_mut().flat_map(|s| &mut s.anchors) {
            (a.p, a.h_in, a.h_out) = (f(a.p), f(a.h_in), f(a.h_out));
        }
    }

    /// Bounding box of the anchors and handles: x0, y0, x1, y1.
    pub fn bounds(&self) -> Option<[f32; 4]> {
        let mut b: Option<[f32; 4]> = None;
        for a in self.subpaths.iter().flat_map(|s| &s.anchors) {
            for q in [a.p, a.h_in, a.h_out] {
                b = Some(match b {
                    None => [q.0, q.1, q.0, q.1],
                    Some(r) => [r[0].min(q.0), r[1].min(q.1), r[2].max(q.0), r[3].max(q.1)],
                });
            }
        }
        b
    }
}

/// Make Work Path: the outline of a selection as a path, simplified to within `tolerance`
/// pixels (Photoshop's default is 2). Sharp turns stay corners; gentle ones become smooth
/// points, so curves stay curves.
pub fn from_selection(sel: &Selection, name: impl Into<String>, tolerance: f32) -> VectorPath {
    let mut path = VectorPath::new(name);
    for ring in outline_rings(sel) {
        let pts = simplify_ring(&ring, tolerance.max(0.5));
        if pts.len() >= 3 {
            path.subpaths.push(SubPath { anchors: smooth_ring(&pts), closed: true });
        }
    }
    path
}

/// The selection's outline (pixel edges) joined into closed rings of corner points.
fn outline_rings(sel: &Selection) -> Vec<Vec<Pt>> {
    use std::collections::HashMap;
    // Every unit edge, both ways round, so rings can be walked from any end.
    let mut edges: Vec<((u32, u32), (u32, u32))> = Vec::new();
    for [x0, y0, x1, y1] in sel.outline() {
        let (dx, dy) = ((x1 as i64 - x0 as i64).signum(), (y1 as i64 - y0 as i64).signum());
        let (mut x, mut y) = (x0 as i64, y0 as i64);
        while (x, y) != (x1 as i64, y1 as i64) {
            let next = (x + dx, y + dy);
            edges.push(((x as u32, y as u32), (next.0 as u32, next.1 as u32)));
            (x, y) = next;
        }
    }
    let mut at: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (i, (a, b)) in edges.iter().enumerate() {
        at.entry(*a).or_default().push(i);
        at.entry(*b).or_default().push(i);
    }
    let mut used = vec![false; edges.len()];
    let mut rings = Vec::new();
    for start in 0..edges.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let (first, mut cur) = edges[start];
        let mut ring = vec![first];
        let mut prev_dir = (cur.0 as i64 - first.0 as i64, cur.1 as i64 - first.1 as i64);
        while cur != first {
            ring.push(cur);
            // Prefer carrying straight on, then any free edge from here.
            let free: Vec<usize> = at.get(&cur).map(|v| v.iter().copied().filter(|&e| !used[e]).collect()).unwrap_or_default();
            let other = |e: usize| if edges[e].0 == cur { edges[e].1 } else { edges[e].0 };
            let pick = free
                .iter()
                .copied()
                .find(|&e| {
                    let o = other(e);
                    (o.0 as i64 - cur.0 as i64, o.1 as i64 - cur.1 as i64) == prev_dir
                })
                .or_else(|| free.first().copied());
            let Some(e) = pick else { break };
            used[e] = true;
            let o = other(e);
            prev_dir = (o.0 as i64 - cur.0 as i64, o.1 as i64 - cur.1 as i64);
            cur = o;
        }
        if ring.len() >= 4 {
            rings.push(ring.into_iter().map(|(x, y)| (x as f32, y as f32)).collect());
        }
    }
    rings
}

/// Distance from `p` to the line through `a` and `b`.
fn line_dist(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len == 0.0 { dist(p, a) } else { ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / len }
}

/// Douglas-Peucker on an open run of points (keeps both ends).
fn simplify_run(pts: &[Pt], tol: f32, out: &mut Vec<Pt>) {
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let far = (1..pts.len() - 1).map(|i| (i, line_dist(pts[i], a, b))).max_by(|x, y| x.1.total_cmp(&y.1));
    match far {
        Some((i, d)) if d > tol => {
            simplify_run(&pts[..=i], tol, out);
            simplify_run(&pts[i..], tol, out);
        }
        _ => out.push(a),
    }
}

/// A closed ring simplified: split at the point farthest from the start, simplify both halves.
fn simplify_ring(ring: &[Pt], tol: f32) -> Vec<Pt> {
    let far = (1..ring.len()).max_by(|&i, &j| dist(ring[i], ring[0]).total_cmp(&dist(ring[j], ring[0]))).unwrap_or(0);
    let mut out = Vec::new();
    let first: Vec<Pt> = ring[..=far].to_vec();
    let mut second: Vec<Pt> = ring[far..].to_vec();
    second.push(ring[0]);
    if first.len() >= 2 {
        simplify_run(&first, tol, &mut out);
    }
    if second.len() >= 2 {
        simplify_run(&second, tol, &mut out);
    }
    out
}

/// Anchors through `pts` (a closed ring): corners where it turns sharply (over 50°),
/// smooth points with tangent handles elsewhere.
fn smooth_ring(pts: &[Pt]) -> Vec<Anchor> {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let (prev, p, next) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            let (ax, ay) = (p.0 - prev.0, p.1 - prev.1);
            let (bx, by) = (next.0 - p.0, next.1 - p.1);
            let turn = (ax * by - ay * bx).atan2(ax * bx + ay * by).abs();
            if turn > 50f32.to_radians() {
                return Anchor::corner(p);
            }
            // Tangent along prev -> next; each handle a third of its own segment long.
            let (tx, ty) = (next.0 - prev.0, next.1 - prev.1);
            let tl = (tx * tx + ty * ty).sqrt().max(1e-6);
            let (ux, uy) = (tx / tl, ty / tl);
            let (lin, lout) = (dist(prev, p) / 3.0, dist(p, next) / 3.0);
            Anchor { p, h_in: (p.0 - ux * lin, p.1 - uy * lin), h_out: (p.0 + ux * lout, p.1 + uy * lout) }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_and_circle_selections() {
        let mut p = VectorPath::new("p");
        p.subpaths.push(SubPath {
            anchors: [(10.0, 10.0), (50.0, 10.0), (50.0, 30.0), (10.0, 30.0)].map(Anchor::corner).to_vec(),
            closed: true,
        });
        let s = p.to_selection(100, 100);
        assert_eq!((s.get(20, 20), s.get(60, 20), s.get(20, 40)), (255, 0, 0));
        // A circle from four smooth anchors (handle length 0.5523 r).
        let (c, r, k) = ((70.0, 70.0), 20.0, 20.0 * 0.5523);
        let circle = SubPath {
            anchors: vec![
                Anchor::smooth((c.0 + r, c.1), (c.0 + r, c.1 + k)),
                Anchor::smooth((c.0, c.1 + r), (c.0 - k, c.1 + r)),
                Anchor::smooth((c.0 - r, c.1), (c.0 - r, c.1 - k)),
                Anchor::smooth((c.0, c.1 - r), (c.0 + k, c.1 - r)),
            ],
            closed: true,
        };
        p.subpaths.push(circle);
        let s = p.to_selection(100, 100);
        assert_eq!(s.get(70, 70), 255);
        assert_eq!(s.get(70, 52), 255, "inside near the top");
        assert_eq!(s.get(54, 54), 0, "outside the corner of its box");
        assert_eq!(s.get(20, 20), 255, "both pieces");
    }

    #[test]
    fn work_path_from_selections() {
        // A rectangle: four corners, and loading it back gives the same pixels.
        let sel = Selection::rect(60, 60, 10.0, 10.0, 40.0, 30.0);
        let p = from_selection(&sel, "Work Path", 2.0);
        assert_eq!(p.subpaths.len(), 1);
        assert_eq!(p.subpaths[0].anchors.len(), 4);
        assert!(p.subpaths[0].anchors.iter().all(|a| a.h_in == a.p && a.h_out == a.p));
        let back = p.to_selection(60, 60);
        assert_eq!((back.get(20, 20), back.get(5, 5), back.get(39, 29), back.get(40, 30)), (255, 0, 255, 0));

        // An ellipse: a handful of smooth points that still hug the edge.
        let e = Selection::ellipse(100, 100, 20.0, 20.0, 80.0, 80.0);
        let p = from_selection(&e, "Work Path", 2.0);
        let s = &p.subpaths[0];
        assert!(s.anchors.len() >= 6 && s.anchors.len() <= 40, "{}", s.anchors.len());
        assert!(s.anchors.iter().any(|a| a.h_out != a.p), "curves stay curves");
        let back = p.to_selection(100, 100);
        assert_eq!((back.get(50, 50), back.get(50, 23), back.get(24, 24)), (255, 255, 0));

        // Two separate areas: two pieces.
        let mut two = Selection::rect(60, 60, 0.0, 0.0, 10.0, 10.0);
        two.combine(&Selection::rect(60, 60, 30.0, 30.0, 50.0, 50.0), Combine::Add);
        assert_eq!(from_selection(&two, "p", 2.0).subpaths.len(), 2);
    }

    #[test]
    fn flatten_follows_the_curve_and_open_paths_dont_close() {
        let s = SubPath { anchors: vec![Anchor::smooth((0.0, 0.0), (0.0, 20.0)), Anchor::smooth((40.0, 0.0), (40.0, -20.0))], closed: false };
        let pts = s.flatten();
        assert_eq!(pts.first(), Some(&(0.0, 0.0)));
        assert_eq!(*pts.last().unwrap(), (40.0, 0.0));
        let low = pts.iter().map(|p| p.1).fold(f32::MIN, f32::max);
        assert!(low > 10.0, "bulges down towards the handles: {low}");
    }
}
