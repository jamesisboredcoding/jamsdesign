//! Shapes and hit testing.
//!
//! [`outline`] is the one place a node's kind becomes a path, so the renderer, hit testing and
//! export all agree on what a node looks like.

use kurbo::{Arc, BezPath, Ellipse, ParamCurveNearest, Point, Rect, RoundedRect, RoundedRectRadii, Shape, Vec2};

use crate::doc::Document;
use crate::node::{CornerRadii, Node, NodeId, NodeKind};

const TOLERANCE: f64 = 0.05;

/// The node's shape in its own space (origin at its top-left corner). Frames, groups,
/// sections and text are their box. Lines and open paths aren't closed.
pub fn outline(node: &Node) -> BezPath {
    let (w, h) = (node.props.width, node.props.height);
    let rect = Rect::new(0.0, 0.0, w, h);
    match &node.kind {
        NodeKind::Page { .. } => BezPath::new(),
        NodeKind::Frame | NodeKind::Section | NodeKind::Rectangle | NodeKind::Group | NodeKind::Text { .. } => {
            rounded(rect, node.props.corner)
        }
        NodeKind::Ellipse { arc } => {
            let full = arc.sweep.abs() >= 360.0;
            if full && arc.inner <= 0.0 {
                return Ellipse::from_rect(rect).to_path(TOLERANCE);
            }
            let center = rect.center();
            let radii = Vec2::new(w / 2.0, h / 2.0);
            // Counter-clockwise on screen (y down) is a negative angle.
            let start = -(arc.start as f64).to_radians();
            let sweep = -(arc.sweep.clamp(-360.0, 360.0) as f64).to_radians();
            let outer = Arc::new(center, radii, start, sweep, 0.0);
            let mut p = BezPath::new();
            p.move_to(arc_start(&outer));
            p.extend(outer.append_iter(TOLERANCE));
            if arc.inner > 0.0 {
                let inner_r = radii * arc.inner.clamp(0.0, 1.0) as f64;
                let inner = Arc::new(center, inner_r, start + sweep, -sweep, 0.0);
                if full {
                    // A ring: two closed loops in opposite directions leave a hole.
                    p.close_path();
                    p.move_to(arc_start(&inner));
                } else {
                    p.line_to(arc_start(&inner));
                }
                p.extend(inner.append_iter(TOLERANCE));
            } else {
                p.line_to(center);
            }
            p.close_path();
            p
        }
        NodeKind::Line => {
            let mut p = BezPath::new();
            p.move_to((0.0, 0.0));
            p.line_to((w, 0.0));
            p
        }
        NodeKind::Polygon { sides } => {
            let n = (*sides).max(3) as usize;
            let pts: Vec<Point> = (0..n).map(|i| unit_point(i as f64 / n as f64, 1.0)).collect();
            fit_polygon(&pts, rect)
        }
        NodeKind::Star { points, inner } => {
            let n = (*points).max(3) as usize;
            let pts: Vec<Point> = (0..n * 2)
                .map(|i| unit_point(i as f64 / (n * 2) as f64, if i % 2 == 0 { 1.0 } else { *inner as f64 }))
                .collect();
            fit_polygon(&pts, rect)
        }
        NodeKind::Vector { path } => path.to_bez(w, h),
    }
}

fn arc_start(a: &Arc) -> Point {
    a.center + Vec2::new(a.radii.x * a.start_angle.cos(), a.radii.y * a.start_angle.sin())
}

/// A rounded rectangle, radii shrunk where they'd overlap.
pub fn rounded(rect: Rect, c: CornerRadii) -> BezPath {
    let max = rect.width().min(rect.height()) / 2.0;
    let r = |v: f64| v.clamp(0.0, max.max(0.0));
    if c == CornerRadii::default() {
        return rect.to_path(TOLERANCE);
    }
    let radii = RoundedRectRadii::new(r(c.top_left), r(c.top_right), r(c.bottom_right), r(c.bottom_left));
    RoundedRect::from_rect(rect, radii).to_path(TOLERANCE)
}

/// A point on the unit circle, starting at the top and going clockwise.
fn unit_point(t: f64, radius: f64) -> Point {
    let a = t * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2;
    Point::new(a.cos() * radius, a.sin() * radius)
}

/// Stretches polygon points so their bounds fill `rect`.
fn fit_polygon(pts: &[Point], rect: Rect) -> BezPath {
    let mut b = Rect::from_points(pts[0], pts[0]);
    for p in pts {
        b = b.union_pt(*p);
    }
    let sx = if b.width() > 0.0 { rect.width() / b.width() } else { 0.0 };
    let sy = if b.height() > 0.0 { rect.height() / b.height() } else { 0.0 };
    let mut path = BezPath::new();
    for (i, p) in pts.iter().enumerate() {
        let q = Point::new((p.x - b.x0) * sx, (p.y - b.y0) * sy);
        if i == 0 { path.move_to(q) } else { path.line_to(q) }
    }
    path.close_path();
    path
}

/// Whether `node` draws something solid that a click inside its outline should hit, as
/// opposed to only a line (strokes without fills, lines, open paths).
fn has_area(node: &Node) -> bool {
    match &node.kind {
        NodeKind::Line => false,
        NodeKind::Vector { path } => path.subpaths.iter().any(|s| s.closed) && node.props.fills.iter().any(|f| f.visible),
        NodeKind::Frame | NodeKind::Section | NodeKind::Group | NodeKind::Text { .. } => true,
        _ => node.props.fills.iter().any(|f| f.visible),
    }
}

/// Whether `local` (in the node's own space) hits the node, with `slop` extra reach for lines.
fn hits(node: &Node, local: Point, slop: f64) -> bool {
    let shape = outline(node);
    // kurbo can miscount a point exactly level with a segment joint, so try a hair lower too.
    if has_area(node) && (shape.contains(local) || shape.contains(local + Vec2::new(0.0, 1e-7))) {
        return true;
    }
    let reach = slop + node.props.stroke.weight as f64 / 2.0;
    shape.segments().any(|seg| seg.nearest(local, 1e-3).distance_sq <= reach * reach)
}

impl Document {
    /// Visible nodes under `point` (page coordinates), topmost first. A child comes before
    /// its parent, so the first entry is the deepest node drawn on top. Children outside a
    /// clipping frame don't count. `slop` is how close (in page units) a click must be to a
    /// line to hit it.
    pub fn nodes_at(&self, page: NodeId, point: Point, slop: f64) -> Vec<NodeId> {
        let mut out = Vec::new();
        for &c in self.children(page).iter().rev() {
            self.collect_at(c, point, slop, &mut out);
        }
        out
    }

    fn collect_at(&self, id: NodeId, point: Point, slop: f64, out: &mut Vec<NodeId>) {
        let Some(n) = self.node(id) else { return };
        if !n.props.visible {
            return;
        }
        let local = self.world_transform(id).inverse() * point;
        let inside_box = Rect::new(0.0, 0.0, n.props.width, n.props.height).contains(local);
        if !(n.props.clip && !inside_box) {
            for &c in n.children.iter().rev() {
                self.collect_at(c, point, slop, out);
            }
        }
        if hits(n, local, slop) {
            out.push(id);
        }
    }

    /// Children of `parent` whose page bounds meet `rect` (or lie fully inside it, with
    /// `contained`), bottom to top. For marquee selection.
    pub fn children_in_rect(&self, parent: NodeId, rect: Rect, contained: bool) -> Vec<NodeId> {
        self.children(parent)
            .iter()
            .copied()
            .filter(|&c| {
                let Some(n) = self.node(c) else { return false };
                let Some(b) = self.world_bounds(c) else { return false };
                n.props.visible
                    && if contained {
                        b.x0 >= rect.x0 && b.y0 >= rect.y0 && b.x1 <= rect.x1 && b.y1 <= rect.y1
                    } else {
                        rect.intersect(b).area() > 0.0 || (b.area() == 0.0 && rect.intersect(b) == b)
                    }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{NodeSpec, Op};
    use crate::*;

    fn bbox(p: &BezPath) -> (f64, f64, f64, f64) {
        let b = p.bounding_box();
        let r = |v: f64| (v * 1000.0).round() / 1000.0;
        (r(b.x0), r(b.y0), r(b.x1), r(b.y1))
    }

    fn node(kind: NodeKind, w: f64, h: f64) -> Node {
        Node { id: NodeId(1), name: String::new(), parent: None, children: Vec::new(), kind, props: Props { width: w, height: h, ..Props::default() } }
    }

    #[test]
    fn polygons_and_stars_fill_their_box() {
        assert_eq!(bbox(&outline(&node(NodeKind::Polygon { sides: 3 }, 100.0, 80.0))), (0.0, 0.0, 100.0, 80.0));
        assert_eq!(bbox(&outline(&node(NodeKind::Star { points: 5, inner: 0.4 }, 50.0, 50.0))), (0.0, 0.0, 50.0, 50.0));
    }

    #[test]
    fn rings_have_holes_and_pies_have_slices() {
        let ring = outline(&node(NodeKind::Ellipse { arc: EllipseArc { inner: 0.5, ..EllipseArc::default() } }, 100.0, 100.0));
        // Off the exact height where curve segments join (kurbo's winding is unreliable there).
        assert!(ring.contains(Point::new(10.0, 47.0)));
        assert!(!ring.contains(Point::new(50.0, 47.0)), "the middle is a hole");
        // A quarter pie from 3 o'clock counter-clockwise: the top-right quarter.
        let pie = outline(&node(NodeKind::Ellipse { arc: EllipseArc { start: 0.0, sweep: 90.0, inner: 0.0 } }, 100.0, 100.0));
        assert!(pie.contains(Point::new(70.0, 30.0)));
        assert!(!pie.contains(Point::new(30.0, 70.0)));
    }

    #[test]
    fn corner_radii_are_clamped() {
        let n = Node { props: Props { corner: CornerRadii::all(500.0), ..node(NodeKind::Rectangle, 40.0, 20.0).props }, ..node(NodeKind::Rectangle, 0.0, 0.0) };
        let p = outline(&n);
        assert_eq!(bbox(&p), (0.0, 0.0, 40.0, 20.0));
        assert!(!p.contains(Point::new(1.0, 1.0)), "the corner is rounded off");
    }

    #[test]
    fn hit_testing_finds_the_topmost_deepest_node() {
        let mut doc = Document::new();
        let p = doc.pages()[0];
        let done = doc
            .apply(
                &[
                    Op::create_as("frame", p, NodeSpec::frame("F", 100.0, 100.0).at(10.0, 10.0)),
                    Op::create_as("under", "frame", NodeSpec::rect("Under", 50.0, 50.0)),
                    Op::create_as("over", "frame", NodeSpec::ellipse("Over", 50.0, 50.0).at(25.0, 25.0)),
                    Op::create_as("outside", "frame", NodeSpec::rect("Outside", 20.0, 20.0).at(150.0, 0.0)),
                    Op::create_as("line", p, NodeSpec::new("Line", NodeKind::Line).size(100.0, 0.0).at(0.0, 200.0)),
                ],
                &ApproxMeasure,
            )
            .unwrap();
        let id = |k| done.id(k).unwrap();
        assert_eq!(doc.nodes_at(p, Point::new(50.0, 50.0), 0.0), vec![id("over"), id("under"), id("frame")]);
        assert_eq!(doc.nodes_at(p, Point::new(15.0, 15.0), 0.0), vec![id("under"), id("frame")], "outside the circle");
        assert!(doc.nodes_at(p, Point::new(165.0, 15.0), 0.0).is_empty(), "clipped by the frame");
        assert_eq!(doc.nodes_at(p, Point::new(50.0, 202.0), 2.0), vec![id("line")]);
        assert!(doc.nodes_at(p, Point::new(50.0, 210.0), 2.0).is_empty());
    }

    #[test]
    fn rotated_nodes_are_hit_where_they_are_drawn() {
        let mut doc = Document::new();
        let p = doc.pages()[0];
        // A 100x10 bar rotated 90 degrees counter-clockwise points up from its origin.
        let done = doc
            .apply(&[Op::create(p, NodeSpec::rect("Bar", 100.0, 10.0).at(50.0, 150.0).with(|p| p.rotation = 90.0))], &ApproxMeasure)
            .unwrap();
        let bar = done.created[0];
        assert_eq!(doc.nodes_at(p, Point::new(55.0, 100.0), 0.0), vec![bar]);
        assert!(doc.nodes_at(p, Point::new(100.0, 155.0), 0.0).is_empty());
        let b = doc.world_bounds(bar).unwrap();
        assert_eq!((b.x0.round(), b.y0.round(), b.x1.round(), b.y1.round()), (50.0, 50.0, 60.0, 150.0));
    }

    #[test]
    fn marquee_selects_touching_or_contained_children() {
        let mut doc = Document::new();
        let p = doc.pages()[0];
        let done = doc
            .apply(&[Op::create(p, NodeSpec::rect("A", 10.0, 10.0)), Op::create(p, NodeSpec::rect("B", 10.0, 10.0).at(20.0, 0.0))], &ApproxMeasure)
            .unwrap();
        let r = Rect::new(5.0, 0.0, 35.0, 10.0);
        assert_eq!(doc.children_in_rect(p, r, false), done.created);
        assert_eq!(doc.children_in_rect(p, r, true), vec![done.created[1]]);
    }
}
