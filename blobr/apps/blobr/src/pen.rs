//! The Pen tool's path in progress, and editing a vector's points: which point or handle is
//! under the pointer, moving it, and fitting the layer's box to the path afterwards.

use jd_vector::{Anchor, Node, NodeKind, Patch, Subpath, VectorPath};
use kurbo::{Affine, BezPath, Point, Shape, Vec2};

/// A path being drawn with the Pen, in page coordinates.
#[derive(Debug, Clone, Default)]
pub struct PenPath {
    pub anchors: Vec<Anchor>,
    /// The mouse is down on the last anchor: dragging pulls out its handles.
    pub dragging: bool,
    /// The pointer, for the segment that follows it.
    pub cursor: Option<Point>,
}

/// A vector layer whose points are being edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VectorEdit {
    pub id: jd_vector::NodeId,
    /// (subpath, anchor) of the selected point.
    pub selected: Option<(usize, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Point,
    In,
    Out,
}

fn pt(p: [f64; 2]) -> Point {
    Point::new(p[0], p[1])
}

/// The path through `anchors` (in their own coordinates).
pub fn bez(anchors: &[Anchor], closed: bool) -> BezPath {
    VectorPath { subpaths: vec![Subpath { anchors: anchors.to_vec(), closed }], fill_rule: Default::default(), base: [0.0, 0.0] }.to_bez(0.0, 0.0)
}

/// A vector layer for a page-space path: position and size in the parent (`to_local` maps page
/// to parent space) and the path in the layer's own space.
pub fn vector_from(anchors: &[Anchor], closed: bool, to_local: Affine) -> (f64, f64, VectorPath) {
    let local = to_local * bez(anchors, closed);
    let b = local.bounding_box();
    let moved = Affine::translate(-b.origin().to_vec2()) * local;
    (b.x0, b.y0, VectorPath::from_bez(&moved, b.width(), b.height()))
}

/// How the layer's own space scales the path's anchors (drawn at `base`).
pub fn base_scale(n: &Node, path: &VectorPath) -> Affine {
    let s = |size: f64, base: f64| if base > 0.0 { size / base } else { 1.0 };
    Affine::scale_non_uniform(s(n.props.width, path.base[0]), s(n.props.height, path.base[1]))
}

/// Patch that makes a vector layer's box fit its path again (after points moved outside it).
pub fn refit(n: &Node) -> Option<Patch> {
    let NodeKind::Vector { path } = &n.kind else { return None };
    let local = path.to_bez(n.props.width, n.props.height);
    let b = local.bounding_box();
    let same = |a: f64, b: f64| (a - b).abs() < 1e-6;
    if same(b.x0, 0.0) && same(b.y0, 0.0) && same(b.width(), n.props.width) && same(b.height(), n.props.height) {
        return None;
    }
    let moved = Affine::translate(-b.origin().to_vec2()) * local;
    let origin = jd_vector::Document::local_transform(n) * b.origin();
    Some(Patch {
        path: Some(VectorPath { fill_rule: path.fill_rule, ..VectorPath::from_bez(&moved, b.width(), b.height()) }),
        x: Some(origin.x),
        y: Some(origin.y),
        width: Some(b.width()),
        height: Some(b.height()),
        ..Patch::default()
    })
}

/// The point or handle within `radius` of `at`; `map` takes anchor space to screen. Handles of
/// the selected point come first (they sit on top).
pub fn hit(path: &VectorPath, map: Affine, at: Point, selected: Option<(usize, usize)>, radius: f64) -> Option<(usize, usize, Part)> {
    let near = |p: [f64; 2]| (map * pt(p) - at).hypot() <= radius;
    if let Some((s, i)) = selected
        && let Some(a) = path.subpaths.get(s).and_then(|sp| sp.anchors.get(i))
    {
        if a.handle_in.is_some_and(near) {
            return Some((s, i, Part::In));
        }
        if a.handle_out.is_some_and(near) {
            return Some((s, i, Part::Out));
        }
    }
    for (s, sp) in path.subpaths.iter().enumerate() {
        for (i, a) in sp.anchors.iter().enumerate() {
            if near(a.p) {
                return Some((s, i, Part::Point));
            }
        }
    }
    None
}

/// The path with a point (and its handles) or one handle moved to `to` (anchor space). Moving
/// a handle turns the opposite one to match, keeping its length, so curves stay smooth.
pub fn moved(path: &VectorPath, s: usize, i: usize, part: Part, to: [f64; 2]) -> VectorPath {
    let mut path = path.clone();
    let Some(a) = path.subpaths.get_mut(s).and_then(|sp| sp.anchors.get_mut(i)) else { return path };
    let shift = |h: &mut Option<[f64; 2]>, d: Vec2| {
        if let Some(h) = h {
            *h = [h[0] + d.x, h[1] + d.y];
        }
    };
    match part {
        Part::Point => {
            let d = pt(to) - pt(a.p);
            a.p = to;
            shift(&mut a.handle_in, d);
            shift(&mut a.handle_out, d);
        }
        Part::In | Part::Out => {
            let (this, other) = if part == Part::In { (&mut a.handle_in, &mut a.handle_out) } else { (&mut a.handle_out, &mut a.handle_in) };
            *this = Some(to);
            if let Some(o) = other {
                let len = (pt(*o) - pt(a.p)).hypot();
                let dir = pt(a.p) - pt(to);
                if dir.hypot() > 1e-9 {
                    let m = pt(a.p) + dir.normalize() * len;
                    *o = [m.x, m.y];
                }
            }
        }
    }
    path
}

/// Where a click lands on one of the path's segments, if within `radius` of one on screen
/// (`map`: anchor space to screen): the path with a point added there, and that point's
/// (subpath, index). Curves are split so they keep their shape; straight segments stay
/// straight.
pub fn insert_at(path: &VectorPath, map: Affine, at: Point, radius: f64) -> Option<(VectorPath, (usize, usize))> {
    use kurbo::{CubicBez, ParamCurve, ParamCurveNearest};
    let mut best: Option<(f64, usize, usize, f64)> = None;
    for (s, sp) in path.subpaths.iter().enumerate() {
        let n = sp.anchors.len();
        let segs = if sp.closed { n } else { n.saturating_sub(1) };
        for i in 0..segs {
            let (a, b) = (&sp.anchors[i], &sp.anchors[(i + 1) % n]);
            let c = CubicBez::new(pt(a.p), pt(a.handle_out.unwrap_or(a.p)), pt(b.handle_in.unwrap_or(b.p)), pt(b.p));
            // Straight segments measure along the line, so the new point lands under the click.
            let near = if a.handle_out.is_none() && b.handle_in.is_none() {
                kurbo::Line::new(map * c.p0, map * c.p3).nearest(at, 0.01)
            } else {
                CubicBez::new(map * c.p0, map * c.p1, map * c.p2, map * c.p3).nearest(at, 0.01)
            };
            let d = near.distance_sq.sqrt();
            if d <= radius && best.is_none_or(|b| d < b.0) {
                best = Some((d, s, i, near.t));
            }
        }
    }
    let (_, s, i, t) = best?;
    // Not on top of an existing point.
    if !(0.02..=0.98).contains(&t) {
        return None;
    }
    let mut out = path.clone();
    let sp = &mut out.subpaths[s];
    let n = sp.anchors.len();
    let j = (i + 1) % n;
    let (a, b) = (sp.anchors[i].clone(), sp.anchors[j].clone());
    let new = if a.handle_out.is_none() && b.handle_in.is_none() {
        let p = pt(a.p).lerp(pt(b.p), t);
        Anchor::corner(p.x, p.y)
    } else {
        let c = CubicBez::new(pt(a.p), pt(a.handle_out.unwrap_or(a.p)), pt(b.handle_in.unwrap_or(b.p)), pt(b.p));
        let (l, r) = (c.subsegment(0.0..t), c.subsegment(t..1.0));
        sp.anchors[i].handle_out = Some([l.p1.x, l.p1.y]);
        sp.anchors[j].handle_in = Some([r.p2.x, r.p2.y]);
        let p = c.eval(t);
        Anchor { p: [p.x, p.y], handle_in: Some([l.p2.x, l.p2.y]), handle_out: Some([r.p1.x, r.p1.y]) }
    };
    sp.anchors.insert(i + 1, new);
    Some((out, (s, i + 1)))
}

/// The path without point (s, i); None when nothing would be left.
pub fn without(path: &VectorPath, s: usize, i: usize) -> Option<VectorPath> {
    let mut path = path.clone();
    let sp = path.subpaths.get_mut(s)?;
    if i < sp.anchors.len() {
        sp.anchors.remove(i);
    }
    if sp.anchors.len() < 2 {
        path.subpaths.remove(s);
    }
    (!path.subpaths.is_empty()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pen_path_becomes_a_fitted_vector() {
        let anchors = [Anchor::corner(100.0, 100.0), Anchor::corner(200.0, 150.0), Anchor::corner(100.0, 200.0)];
        let (x, y, path) = vector_from(&anchors, true, Affine::translate((-50.0, 0.0)));
        assert_eq!((x, y), (50.0, 100.0));
        assert_eq!(path.base, [100.0, 100.0]);
        assert!(path.subpaths[0].closed);
        assert_eq!(path.subpaths[0].anchors.len(), 3, "closing doesn't duplicate the first point");
        assert_eq!(path.subpaths[0].anchors[1].p, [100.0, 50.0]);
    }

    #[test]
    fn moving_points_and_mirrored_handles() {
        let a = Anchor { p: [10.0, 10.0], handle_in: Some([0.0, 10.0]), handle_out: Some([20.0, 10.0]) };
        let path = VectorPath { subpaths: vec![Subpath { anchors: vec![Anchor::corner(0.0, 0.0), a], closed: false }], fill_rule: Default::default(), base: [20.0, 10.0] };
        let p = moved(&path, 0, 1, Part::Point, [15.0, 10.0]);
        assert_eq!(p.subpaths[0].anchors[1].handle_out, Some([25.0, 10.0]), "handles travel with the point");
        let p = moved(&path, 0, 1, Part::Out, [10.0, 30.0]);
        let h = p.subpaths[0].anchors[1].handle_in.unwrap();
        assert!((h[0] - 10.0).abs() < 1e-9 && (h[1] + 0.0).abs() < 1e-9, "opposite handle turns to match, same length: {h:?}");
        assert_eq!(hit(&path, Affine::IDENTITY, Point::new(19.0, 11.0), Some((0, 1)), 3.0), Some((0, 1, Part::Out)));
        assert_eq!(hit(&path, Affine::IDENTITY, Point::new(19.0, 11.0), None, 3.0), None, "handles only for the selected point");
        assert!(without(&path, 0, 0).is_none(), "one point left is no path");
    }

    #[test]
    fn clicking_a_segment_adds_a_point_that_keeps_the_shape() {
        let line = VectorPath { subpaths: vec![Subpath { anchors: vec![Anchor::corner(0.0, 0.0), Anchor::corner(100.0, 0.0)], closed: false }], fill_rule: Default::default(), base: [100.0, 1.0] };
        let (p, at) = insert_at(&line, Affine::IDENTITY, Point::new(25.0, 3.0), 6.0).unwrap();
        assert_eq!(at, (0, 1));
        assert_eq!(p.subpaths[0].anchors[1], Anchor::corner(25.0, 0.0), "straight stays straight");
        assert!(insert_at(&line, Affine::IDENTITY, Point::new(25.0, 20.0), 6.0).is_none(), "too far");

        let curve = VectorPath {
            subpaths: vec![Subpath {
                anchors: vec![Anchor { p: [0.0, 0.0], handle_in: None, handle_out: Some([0.0, 50.0]) }, Anchor { p: [100.0, 0.0], handle_in: Some([100.0, 50.0]), handle_out: None }],
                closed: false,
            }],
            fill_rule: Default::default(),
            base: [100.0, 40.0],
        };
        let before = curve.to_bez(100.0, 40.0);
        let (p, _) = insert_at(&curve, Affine::IDENTITY, Point::new(50.0, 37.5), 6.0).unwrap();
        let after = p.to_bez(100.0, 40.0);
        assert_eq!(p.subpaths[0].anchors.len(), 3);
        assert!((before.bounding_box().y1 - after.bounding_box().y1).abs() < 0.01, "same curve");
    }
}
