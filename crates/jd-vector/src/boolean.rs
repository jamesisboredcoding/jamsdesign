//! Boolean operations and flattening. Layers become one vector layer with the combined
//! outline (union, subtract, intersect, exclude), taking the bottom layer's look, in the
//! bottom layer's parent, where the top layer was in the stack.
//!
//! The geometry is done by `linesweeper` (exact sweep-line booleans on bezier paths). Groups
//! count as their children; text counts as its box (glyph outlines live in the renderer).

use kurbo::{Affine, BezPath, PathEl, Shape};

use crate::geometry::outline;
use crate::{Document, FillRule, NodeId, NodeKind, NodeSpec, Op, VectorPath};

type Kurbo13 = kurbo13::BezPath;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    /// The bottom layer minus the others.
    Subtract,
    Intersect,
    Exclude,
}

impl BoolOp {
    pub fn name(self) -> &'static str {
        match self {
            BoolOp::Union => "Union",
            BoolOp::Subtract => "Subtract",
            BoolOp::Intersect => "Intersect",
            BoolOp::Exclude => "Exclude",
        }
    }
}

fn to13(p: &BezPath) -> Kurbo13 {
    let q = |p: kurbo::Point| kurbo13::Point::new(p.x, p.y);
    let mut out = Kurbo13::new();
    let mut open = false;
    for el in p.elements() {
        match *el {
            PathEl::MoveTo(a) => {
                // Every subpath must be closed for a boolean; open ones are closed straight.
                if open {
                    out.close_path();
                }
                out.move_to(q(a));
                open = true;
            }
            PathEl::LineTo(a) => out.line_to(q(a)),
            PathEl::QuadTo(c, a) => out.quad_to(q(c), q(a)),
            PathEl::CurveTo(c1, c2, a) => out.curve_to(q(c1), q(c2), q(a)),
            PathEl::ClosePath => {
                out.close_path();
                open = false;
            }
        }
    }
    if open {
        out.close_path();
    }
    out
}

fn from13(p: &Kurbo13) -> BezPath {
    let q = |p: kurbo13::Point| kurbo::Point::new(p.x, p.y);
    let mut out = BezPath::new();
    for el in p.elements() {
        match *el {
            kurbo13::PathEl::MoveTo(a) => out.move_to(q(a)),
            kurbo13::PathEl::LineTo(a) => out.line_to(q(a)),
            kurbo13::PathEl::QuadTo(c, a) => out.quad_to(q(c), q(a)),
            kurbo13::PathEl::CurveTo(c1, c2, a) => out.curve_to(q(c1), q(c2), q(a)),
            kurbo13::PathEl::ClosePath => out.close_path(),
        }
    }
    out
}

/// One boolean between two page-space shapes.
fn combine(a: &BezPath, b: &BezPath, op: BoolOp) -> Result<BezPath, String> {
    use linesweeper::BinaryOp;
    let op = match op {
        BoolOp::Union => BinaryOp::Union,
        BoolOp::Subtract => BinaryOp::Difference,
        BoolOp::Intersect => BinaryOp::Intersection,
        BoolOp::Exclude => BinaryOp::Xor,
    };
    let contours = linesweeper::binary_op(&to13(a), &to13(b), linesweeper::FillRule::NonZero, op).map_err(|e| e.to_string())?;
    let mut out = BezPath::new();
    for c in contours.contours() {
        out.extend(from13(&c.path));
    }
    Ok(out)
}

/// A layer's shape on its page (groups: their children's, combined).
fn shape(doc: &Document, id: NodeId) -> Result<BezPath, String> {
    let n = doc.node(id).ok_or("a selected layer no longer exists")?;
    if matches!(n.kind, NodeKind::Group) {
        let mut acc: Option<BezPath> = None;
        for &c in doc.children(id) {
            if !doc.node(c).is_some_and(|c| c.props.visible) {
                continue;
            }
            let s = shape(doc, c)?;
            acc = Some(match acc {
                Some(a) => combine(&a, &s, BoolOp::Union)?,
                None => s,
            });
        }
        return Ok(acc.unwrap_or_default());
    }
    Ok(doc.world_transform(id) * outline(n))
}

/// The selection sorted bottom to top (by their place in the page's tree).
fn stacked(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let order = |id: NodeId| {
        let mut path = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            let p = doc.parent(c);
            path.push(p.map_or(0, |p| doc.children(p).iter().position(|&x| x == c).unwrap_or(0)));
            cur = p;
        }
        path.reverse();
        path
    };
    let mut v: Vec<NodeId> = ids.iter().copied().filter(|&id| doc.parent(id).is_some()).collect();
    v.sort_by_key(|&id| order(id));
    v.dedup();
    v
}

/// Ops that replace `ids` with one vector made from `path` (page space), styled like
/// `bottom`, placed where `top` was. Its key is `"result"`.
fn replace(doc: &Document, ids: &[NodeId], bottom: NodeId, top: NodeId, path: BezPath, name: &str, fill_rule: FillRule) -> Result<Vec<Op>, String> {
    let parent = doc.parent(bottom).ok_or("pages can't be combined")?;
    if doc.is_inside_instance(bottom) {
        return Err("layers inside an instance can't be combined; detach it first".into());
    }
    let local = doc.world_transform(parent).inverse() * path;
    let b = local.bounding_box();
    if !(b.width() > 0.0 || b.height() > 0.0) {
        return Err("the result is empty".into());
    }
    let moved = Affine::translate(-b.origin().to_vec2()) * local;
    let mut vp = VectorPath::from_bez(&moved, b.width(), b.height());
    vp.fill_rule = fill_rule;
    let mut spec = NodeSpec::vector(name, vp).at(b.x0, b.y0);
    let look = &doc.node(bottom).expect("exists").props;
    spec.props.fills = look.fills.clone();
    spec.props.strokes = look.strokes.clone();
    spec.props.stroke = look.stroke.clone();
    spec.props.effects = look.effects.clone();
    spec.props.opacity = look.opacity;
    spec.props.blend = look.blend;
    // Above the top layer if it's in the same parent, else above the bottom one.
    let anchor = if doc.parent(top) == Some(parent) { top } else { bottom };
    let index = doc.children(parent).iter().position(|&c| c == anchor).map(|i| i + 1);
    let mut ops = vec![Op::Create { parent: parent.into(), node: spec, key: Some("result".into()), index }];
    ops.extend(ids.iter().map(|&id| Op::Delete { id: id.into() }));
    Ok(ops)
}

/// Ops combining two or more layers into one vector.
pub fn boolean(doc: &Document, ids: &[NodeId], op: BoolOp) -> Result<Vec<Op>, String> {
    let sorted = stacked(doc, ids);
    if sorted.len() < 2 {
        return Err("select two or more layers".into());
    }
    let shapes: Vec<BezPath> = sorted.iter().map(|&id| shape(doc, id)).collect::<Result<_, _>>()?;
    let path = match op {
        BoolOp::Subtract => {
            let mut cut = shapes[1].clone();
            for s in &shapes[2..] {
                cut = combine(&cut, s, BoolOp::Union)?;
            }
            combine(&shapes[0], &cut, BoolOp::Subtract)?
        }
        _ => {
            let mut acc = shapes[0].clone();
            for s in &shapes[1..] {
                acc = combine(&acc, s, op)?;
            }
            acc
        }
    };
    if path.elements().is_empty() {
        return Err(format!("{} leaves nothing: the layers don't overlap", op.name()));
    }
    replace(doc, &sorted, sorted[0], sorted[sorted.len() - 1], path, op.name(), FillRule::EvenOdd)
}

/// Ops turning layers into vectors with the same outline (several at once: one, combined).
pub fn flatten(doc: &Document, ids: &[NodeId]) -> Result<Vec<Op>, String> {
    let sorted = stacked(doc, ids);
    match sorted.as_slice() {
        [] => Err("nothing to flatten".into()),
        [one] => {
            let n = doc.node(*one).expect("exists");
            if matches!(n.kind, NodeKind::Vector { .. }) {
                return Ok(Vec::new());
            }
            // Keeps open paths (lines) open: no boolean needed for one layer.
            let path = shape(doc, *one)?;
            let rule = if matches!(n.kind, NodeKind::Group) { FillRule::EvenOdd } else { FillRule::NonZero };
            replace(doc, &sorted, *one, *one, path, &n.name, rule)
        }
        many => {
            let ops = boolean(doc, many, BoolOp::Union)?;
            Ok(ops)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApproxMeasure, NodeSpec, Paint};

    fn two_squares() -> (Document, NodeId, Vec<NodeId>) {
        let mut doc = Document::new();
        let page = doc.pages()[0];
        let done = doc
            .apply(
                &[
                    Op::create(page, NodeSpec::rect("A", 100.0, 100.0).at(0.0, 0.0).fill(Paint::hex("#FF0000"))),
                    Op::create(page, NodeSpec::rect("B", 100.0, 100.0).at(50.0, 50.0)),
                ],
                &ApproxMeasure,
            )
            .unwrap();
        (doc, page, done.created)
    }

    fn area(doc: &Document, id: NodeId) -> f64 {
        let n = doc.node(id).unwrap();
        // Sum each contour's area (holes don't occur in these cases).
        let NodeKind::Vector { path } = &n.kind else { panic!("a vector") };
        let bez = path.to_bez(n.props.width, n.props.height);
        let mut total = 0.0;
        let mut cur = BezPath::new();
        for el in bez.elements() {
            if let PathEl::MoveTo(_) = el
                && !cur.elements().is_empty()
            {
                total += cur.area().abs();
                cur = BezPath::new();
            }
            cur.push(*el);
        }
        total + cur.area().abs()
    }

    #[test]
    fn union_subtract_intersect_exclude() {
        for (op, want, bounds) in [
            (BoolOp::Union, 17500.0, (0.0, 0.0, 150.0, 150.0)),
            (BoolOp::Subtract, 7500.0, (0.0, 0.0, 100.0, 100.0)),
            (BoolOp::Intersect, 2500.0, (50.0, 50.0, 50.0, 50.0)),
            // Two L-shapes; the even-odd rule leaves the middle empty.
            (BoolOp::Exclude, 15000.0, (0.0, 0.0, 150.0, 150.0)),
        ] {
            let (mut doc, page, ids) = two_squares();
            let ops = boolean(&doc, &[ids[1], ids[0]], op).unwrap();
            let r = doc.apply(&ops, &ApproxMeasure).unwrap().id("result").unwrap();
            assert_eq!(doc.children(page), &[r], "{op:?} replaces both");
            let n = doc.node(r).unwrap();
            assert_eq!((n.props.x, n.props.y, n.props.width, n.props.height), bounds, "{op:?}");
            assert!((area(&doc, r) - want).abs() < 1.0, "{op:?}: area {}", area(&doc, r));
            assert_eq!(n.props.fills, vec![Paint::hex("#FF0000")], "the bottom layer's look");
            assert_eq!(n.name, op.name());
        }
    }

    #[test]
    fn flatten_keeps_the_shape_and_place() {
        let mut doc = Document::new();
        let page = doc.pages()[0];
        let e = doc.apply(&[Op::create(page, NodeSpec::ellipse("E", 40.0, 20.0).at(10.0, 10.0))], &ApproxMeasure).unwrap().created[0];
        let ops = flatten(&doc, &[e]).unwrap();
        let v = doc.apply(&ops, &ApproxMeasure).unwrap().id("result").unwrap();
        let n = doc.node(v).unwrap();
        assert_eq!(n.name, "E");
        assert!((n.props.x - 10.0).abs() < 1e-6 && (n.props.width - 40.0).abs() < 1e-6);
        assert!(boolean(&doc, &[v], BoolOp::Union).is_err(), "one layer isn't a boolean");
    }
}
