//! Boolean operations and flattening. Layers become one vector layer with the combined
//! outline (union, subtract, intersect, exclude), taking the bottom layer's look, in the
//! bottom layer's parent, where the top layer was in the stack.
//!
//! The geometry is done by `linesweeper` (exact sweep-line booleans on bezier paths). Groups
//! count as their children; text counts as its box (glyph outlines live in the renderer).

use kurbo::{Affine, BezPath, PathEl, Shape};

use crate::geometry::outline;
use crate::{Document, FillRule, NodeId, NodeKind, NodeSpec, Op, Ref, VectorPath};

type Kurbo13 = kurbo13::BezPath;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoolOp {
    Union,
    /// The bottom layer minus the others.
    Subtract,
    Intersect,
    Exclude,
}

impl BoolOp {
    pub const ALL: [BoolOp; 4] = [BoolOp::Union, BoolOp::Subtract, BoolOp::Intersect, BoolOp::Exclude];

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

/// A layer's shape on its page (groups: their children's, combined; boolean groups: their
/// children combined with their operation).
fn shape(doc: &Document, id: NodeId) -> Result<BezPath, String> {
    let n = doc.node(id).ok_or("a selected layer no longer exists")?;
    let op = match n.kind {
        NodeKind::Group => BoolOp::Union,
        NodeKind::Boolean { op, .. } => op,
        _ => return Ok(doc.world_transform(id) * outline(n)),
    };
    let mut acc: Option<BezPath> = None;
    for &c in doc.children(id) {
        if !doc.node(c).is_some_and(|c| c.props.visible) {
            continue;
        }
        let s = shape(doc, c)?;
        acc = Some(match acc {
            Some(a) => combine(&a, &s, op)?,
            None => s,
        });
    }
    Ok(acc.unwrap_or_default())
}

/// Ops that turn a layer's stroke into a filled vector of the same shape (key `"result"`),
/// placed above it. The layer loses its stroke, or goes if it had no fill.
pub fn outline_stroke(doc: &Document, id: NodeId) -> Result<Vec<Op>, String> {
    use crate::{StrokeAlign, StrokeCap, StrokeJoin};
    let n = doc.node(id).ok_or("that layer no longer exists")?;
    let parent = doc.parent(id).ok_or("pages have no stroke")?;
    if doc.is_inside_instance(id) {
        return Err("layers inside an instance can't be changed like that; detach it first".into());
    }
    if matches!(n.kind, NodeKind::Text { .. }) {
        return Err("text strokes can't be outlined yet".into());
    }
    let style = &n.props.stroke;
    let paints: Vec<_> = n.props.strokes.iter().filter(|p| p.visible).cloned().collect();
    if paints.is_empty() || style.weight <= 0.0 {
        return Err("this layer has no stroke to outline".into());
    }
    let shape = outline(n);
    let closed = shape.elements().iter().any(|e| matches!(e, PathEl::ClosePath));
    // Inside and outside strokes are a centred one twice as wide, cut to the shape.
    let align = if closed { style.align } else { StrokeAlign::Center };
    let w = style.weight as f64 * if align == StrokeAlign::Center { 1.0 } else { 2.0 };
    let mut ks = kurbo::Stroke::new(w)
        .with_join(match style.join {
            StrokeJoin::Miter => kurbo::Join::Miter,
            StrokeJoin::Round => kurbo::Join::Round,
            StrokeJoin::Bevel => kurbo::Join::Bevel,
        })
        .with_caps(match style.cap {
            StrokeCap::None => kurbo::Cap::Butt,
            StrokeCap::Round => kurbo::Cap::Round,
            StrokeCap::Square => kurbo::Cap::Square,
        });
    if !style.dash.is_empty() {
        ks = ks.with_dashes(0.0, style.dash.iter().map(|&d| d as f64));
    }
    let stroked = kurbo::stroke(shape.elements().iter().copied(), &ks, &kurbo::StrokeOpts::default(), 0.05);
    let stroked = match align {
        StrokeAlign::Center => combine(&stroked, &BezPath::new(), BoolOp::Union)?,
        StrokeAlign::Inside => combine(&stroked, &shape, BoolOp::Intersect)?,
        StrokeAlign::Outside => combine(&stroked, &shape, BoolOp::Subtract)?,
    };
    let in_parent = crate::Document::local_transform(n) * stroked;
    let b = in_parent.bounding_box();
    if !(b.width() > 0.0 || b.height() > 0.0) {
        return Err("the outline is empty".into());
    }
    let moved = Affine::translate(-b.origin().to_vec2()) * in_parent;
    let mut vp = VectorPath::from_bez(&moved, b.width(), b.height());
    vp.fill_rule = FillRule::EvenOdd;
    let mut spec = NodeSpec::vector(&format!("{} outline", n.name), vp).at(b.x0, b.y0);
    spec.props.fills = paints;
    spec.props.strokes = Vec::new();
    spec.props.opacity = n.props.opacity;
    let index = doc.children(parent).iter().position(|&c| c == id).map(|i| i + 1);
    let mut ops = vec![Op::Create { parent: parent.into(), node: spec, key: Some("result".into()), index }];
    if n.props.fills.iter().any(|f| f.visible) {
        ops.push(Op::set(id, crate::Patch { strokes: Some(Vec::new()), ..Default::default() }));
    } else {
        ops.push(Op::Delete { id: id.into() });
    }
    Ok(ops)
}

/// Works out every boolean group's shape from its children (after each batch, once layout has
/// fitted it to them). Innermost first, so nested ones are ready for their parents.
pub(crate) fn sync_all(doc: &mut Document) {
    let mut ids: Vec<(usize, NodeId)> = doc
        .nodes
        .values()
        .filter(|n| matches!(n.kind, NodeKind::Boolean { .. }))
        .map(|n| (std::iter::successors(doc.parent(n.id), |&p| doc.parent(p)).count(), n.id))
        .collect();
    ids.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, id) in ids {
        let page = shape(doc, id).unwrap_or_default();
        let local = doc.world_transform(id).inverse() * page;
        let n = doc.node(id).expect("listed");
        let mut path = VectorPath::from_bez(&local, n.props.width.max(1e-6), n.props.height.max(1e-6));
        path.fill_rule = FillRule::EvenOdd;
        if let NodeKind::Boolean { path: old, .. } = &n.kind
            && *old != path
            && let Some(NodeKind::Boolean { path: p, .. }) = doc.node_mut(id).map(|n| &mut n.kind)
        {
            *p = path;
        }
    }
}

/// Ops that put `ids` into a new live boolean group (key `"result"`) where the top one was,
/// in the bottom one's parent. The group takes the bottom layer's look.
pub fn boolean_group(doc: &Document, ids: &[NodeId], op: BoolOp) -> Result<Vec<Op>, String> {
    let sorted = stacked(doc, ids);
    let (Some(&bottom), Some(&top)) = (sorted.first(), sorted.last()) else { return Err("select two or more layers".into()) };
    if sorted.len() < 2 {
        return Err("select two or more layers".into());
    }
    let parent = doc.parent(bottom).ok_or("pages can't be combined")?;
    if sorted.iter().any(|&id| doc.is_inside_instance(id)) {
        return Err("layers inside an instance can't be combined; detach it first".into());
    }
    let look = &doc.node(bottom).expect("exists").props;
    let mut spec = NodeSpec::new(op.name(), NodeKind::Boolean { op, path: VectorPath::empty() });
    spec.props.fills = look.fills.clone();
    spec.props.strokes = look.strokes.clone();
    spec.props.stroke = look.stroke.clone();
    spec.props.effects = look.effects.clone();
    let anchor = if doc.parent(top) == Some(parent) { top } else { bottom };
    let index = doc.children(parent).iter().position(|&c| c == anchor).map(|i| i + 1);
    let mut ops = vec![Op::Create { parent: parent.into(), node: spec, key: Some("result".into()), index }];
    for &id in &sorted {
        // Keep each layer where it is on the page: the group starts at the parent's origin and
        // fits itself to them afterwards.
        let world = doc.world_transform(id);
        let local = doc.world_transform(parent).inverse() * world;
        let [.., x, y] = local.as_coeffs();
        ops.push(Op::Move { id: id.into(), parent: Ref::Key("result".into()), index: None });
        ops.push(Op::set(id, crate::Patch { x: Some(x), y: Some(y), ..Default::default() }));
    }
    Ok(ops)
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

    #[test]
    fn live_boolean_groups_follow_their_children() {
        let (mut doc, page, ids) = two_squares();
        let done = doc.apply(&boolean_group(&doc, &ids, BoolOp::Union).unwrap(), &ApproxMeasure).unwrap();
        let g = done.id("result").unwrap();
        let n = doc.node(g).unwrap();
        assert_eq!(doc.children(g), ids.as_slice(), "the layers move inside, bottom first");
        assert_eq!(doc.parent(g), Some(page));
        assert_eq!((n.props.x, n.props.y, n.props.width, n.props.height), (0.0, 0.0, 150.0, 150.0));
        assert_eq!(n.props.fills, doc.node(ids[0]).unwrap().props.fills, "the bottom layer's look");
        let area = |doc: &Document| crate::geometry::outline(doc.node(g).unwrap()).area().abs();
        assert!((area(&doc) - 17_500.0).abs() < 1.0, "union of two overlapping squares: {}", area(&doc));
        // Pull the top square away: the result follows.
        doc.apply(&[Op::set(ids[1], crate::Patch { x: Some(300.0), ..Default::default() })], &ApproxMeasure).unwrap();
        assert!((area(&doc) - 20_000.0).abs() < 1.0);
        assert_eq!(doc.node(g).unwrap().props.width, 400.0, "refitted");
        // Another operation on the same layers.
        doc.apply(&[Op::set(ids[1], crate::Patch { x: Some(50.0), ..Default::default() })], &ApproxMeasure).unwrap();
        let mut patch = crate::Patch::default();
        patch.bool_op = Some(BoolOp::Intersect);
        doc.apply(&[Op::set(g, patch)], &ApproxMeasure).unwrap();
        assert!((area(&doc) - 2_500.0).abs() < 1.0);
        // Saved and loaded, it is still live.
        let back = crate::file::from_json(&crate::file::to_json(&doc)).unwrap();
        assert!(matches!(back.node(g).unwrap().kind, NodeKind::Boolean { op: BoolOp::Intersect, .. }));
    }

    #[test]
    fn outline_stroke_makes_a_filled_ring() {
        let mut doc = Document::new();
        let page = doc.pages()[0];
        let mut spec = NodeSpec::rect("Box", 100.0, 100.0).at(10.0, 10.0);
        spec.props.fills = Vec::new();
        spec.props.strokes = vec![Paint::hex("#0000FF")];
        spec.props.stroke.weight = 10.0;
        spec.props.stroke.align = crate::StrokeAlign::Inside;
        let id = doc.apply(&[Op::create(page, spec)], &ApproxMeasure).unwrap().created[0];
        let done = doc.apply(&outline_stroke(&doc, id).unwrap(), &ApproxMeasure).unwrap();
        let v = done.id("result").unwrap();
        assert!(doc.node(id).is_none(), "no fill: the layer is replaced");
        let n = doc.node(v).unwrap();
        assert_eq!((n.props.x, n.props.y, n.props.width, n.props.height), (10.0, 10.0, 100.0, 100.0), "inside: within the box");
        assert_eq!(n.props.fills, vec![Paint::hex("#0000FF")]);
        assert!(n.props.strokes.is_empty());
        // A 10 px ring inside a 100 px square: 100^2 - 80^2.
        let a = crate::geometry::outline(n);
        let mut ring = 0.0;
        let mut cur = BezPath::new();
        for el in a.elements().iter().chain([PathEl::MoveTo(kurbo::Point::ZERO)].iter()) {
            if matches!(el, PathEl::MoveTo(_)) && !cur.elements().is_empty() {
                ring += cur.area().abs();
                cur = BezPath::new();
            }
            cur.push(*el);
        }
        let outer_minus_inner = 100.0_f64 * 100.0 - 80.0 * 80.0;
        let outer_plus_inner = 100.0_f64 * 100.0 + 80.0 * 80.0;
        assert!((ring - outer_plus_inner).abs() < 2.0 || (ring - outer_minus_inner).abs() < 2.0, "{ring}");
    }
}
