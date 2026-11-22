//! Arranging layers: group, frame selection, ungroup and stacking order. Each function turns a
//! selection into a batch of ops (one undo step), so apps and the MCP server share the rules.

use kurbo::{Affine, Point, Rect};

use crate::{Document, NodeId, NodeKind, NodeSpec, Op, Patch};

/// The selection sorted bottom to top under one shared parent.
fn siblings(doc: &Document, ids: &[NodeId]) -> Result<(NodeId, Vec<NodeId>), String> {
    let mut parent = None;
    for &id in ids {
        let n = doc.node(id).ok_or("a selected layer no longer exists")?;
        if matches!(n.kind, NodeKind::Page { .. }) {
            return Err("pages can't be grouped".into());
        }
        let p = n.parent.ok_or("pages can't be grouped")?;
        if parent.is_some_and(|q| q != p) {
            return Err("select layers that share a parent".into());
        }
        parent = Some(p);
    }
    let parent = parent.ok_or("nothing selected")?;
    let mut sorted: Vec<NodeId> = doc.children(parent).iter().copied().filter(|c| ids.contains(c)).collect();
    sorted.dedup();
    Ok((parent, sorted))
}

fn index_in(doc: &Document, parent: NodeId, id: NodeId) -> usize {
    doc.children(parent).iter().position(|&c| c == id).unwrap_or(0)
}

/// Ops that put `ids` (same parent) in a new group, or a new frame around them
/// (`frame`), placed where the topmost of them was. The new node's key is `"group"`.
pub fn group(doc: &Document, ids: &[NodeId], frame: bool) -> Result<Vec<Op>, String> {
    let (parent, sorted) = siblings(doc, ids)?;
    let index = sorted.iter().map(|&id| index_in(doc, parent, id)).max().unwrap_or(0) + 1;
    let n = doc.children(parent).len() + 1;
    let (spec, offset) = if frame {
        // The frame covers the layers' box in the parent's space.
        let mut b: Option<Rect> = None;
        for &id in &sorted {
            let node = doc.node(id).expect("checked");
            let t = Document::local_transform(node);
            let (w, h) = (node.props.width, node.props.height);
            for (x, y) in [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)] {
                let p = t * Point::new(x, y);
                b = Some(b.map_or(Rect::from_points(p, p), |b| b.union_pt(p)));
            }
        }
        let b = b.expect("not empty");
        let mut spec = NodeSpec::frame(&format!("Frame {n}"), b.width(), b.height()).at(b.x0, b.y0);
        spec.props.fills.clear();
        spec.props.clip = false;
        (spec, (b.x0, b.y0))
    } else {
        (NodeSpec::group(&format!("Group {n}")), (0.0, 0.0))
    };
    let mut ops = vec![Op::Create { parent: parent.into(), node: spec, key: Some("group".into()), index: Some(index) }];
    for &id in &sorted {
        ops.push(Op::Move { id: id.into(), parent: "group".into(), index: None });
        if frame {
            let p = &doc.node(id).expect("checked").props;
            ops.push(Op::set(id, Patch { x: Some(p.x - offset.0), y: Some(p.y - offset.1), ..Patch::default() }));
        }
    }
    Ok(ops)
}

/// Ops that dissolve groups and plain frames, leaving their layers where they were on the
/// page. Other layers in `ids` are left alone; returns the freed layers too.
pub fn ungroup(doc: &Document, ids: &[NodeId]) -> Result<(Vec<Op>, Vec<NodeId>), String> {
    let mut ops = Vec::new();
    let mut freed = Vec::new();
    for &id in ids {
        let Some(g) = doc.node(id) else { continue };
        if !matches!(g.kind, NodeKind::Group | NodeKind::Frame | NodeKind::Section) {
            continue;
        }
        let parent = g.parent.ok_or("pages can't be ungrouped")?;
        let at = index_in(doc, parent, id);
        let tg = Document::local_transform(g);
        for (i, &c) in doc.children(id).iter().enumerate() {
            let cn = doc.node(c).expect("child");
            let t: Affine = tg * Document::local_transform(cn);
            let [.., x, y] = t.as_coeffs();
            ops.push(Op::Move { id: c.into(), parent: parent.into(), index: Some(at + i) });
            ops.push(Op::set(c, Patch { x: Some(x), y: Some(y), rotation: Some(cn.props.rotation + g.props.rotation), ..Patch::default() }));
            freed.push(c);
        }
        ops.push(Op::Delete { id: id.into() });
    }
    if ops.is_empty() {
        return Err("select a group or frame to ungroup".into());
    }
    Ok((ops, freed))
}

/// Where to move layers in their parent's stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Front,
    Forward,
    Backward,
    Back,
}

/// Ops that restack `ids` among their siblings, keeping their order among themselves.
/// Layers with different parents are restacked in each parent separately.
pub fn reorder(doc: &Document, ids: &[NodeId], order: Order) -> Vec<Op> {
    let mut parents: Vec<NodeId> = ids.iter().filter_map(|&id| doc.parent(id)).collect();
    parents.sort();
    parents.dedup();
    let mut ops = Vec::new();
    for parent in parents {
        let kids = doc.children(parent);
        let picked: Vec<(usize, NodeId)> = kids.iter().copied().enumerate().filter(|(_, c)| ids.contains(c)).collect();
        let last = kids.len() - 1;
        let mv = |id: NodeId, index: usize| Op::Move { id: id.into(), parent: parent.into(), index: Some(index) };
        match order {
            Order::Front => ops.extend(picked.iter().map(|&(_, id)| Op::Move { id: id.into(), parent: parent.into(), index: None })),
            Order::Back => ops.extend(picked.iter().enumerate().map(|(i, &(_, id))| mv(id, i))),
            Order::Forward => {
                // Top first, each one step up unless the one above is selected and stuck.
                let mut ceiling = last + 1;
                for &(i, id) in picked.iter().rev() {
                    let to = (i + 1).min(ceiling - 1);
                    if to != i {
                        ops.push(mv(id, to));
                    }
                    ceiling = to;
                }
            }
            Order::Backward => {
                let mut floor: isize = -1;
                for &(i, id) in &picked {
                    let to = (i as isize - 1).max(floor + 1) as usize;
                    if to != i {
                        ops.push(mv(id, to));
                    }
                    floor = to as isize;
                }
            }
        }
    }
    ops
}

/// Which edge or centre line to line layers up on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
    Top,
    Middle,
    Bottom,
}

/// Layers that can be placed freely (auto layout places the rest), with their page boxes.
fn placeable(doc: &Document, ids: &[NodeId]) -> Vec<(NodeId, Rect)> {
    ids.iter()
        .filter_map(|&id| {
            let n = doc.node(id)?;
            let parent = doc.node(n.parent?)?;
            if parent.props.layout.is_some() && !n.props.absolute {
                return None;
            }
            Some((id, doc.world_bounds(id)?))
        })
        .collect()
}

/// An op moving `id` by `(dx, dy)` on its page.
fn shift(doc: &Document, id: NodeId, dx: f64, dy: f64) -> Option<Op> {
    if dx.abs() < 1e-9 && dy.abs() < 1e-9 {
        return None;
    }
    let n = doc.node(id)?;
    let c = doc.world_transform(n.parent?).inverse().as_coeffs();
    let (lx, ly) = (c[0] * dx + c[2] * dy, c[1] * dx + c[3] * dy);
    let r = |v: f64| (v * 100.0).round() / 100.0;
    Some(Op::set(id, Patch { x: Some(r(n.props.x + lx)), y: Some(r(n.props.y + ly)), ..Patch::default() }))
}

/// Ops lining up `ids` with each other, or one layer with its parent frame.
pub fn align(doc: &Document, ids: &[NodeId], to: Align) -> Vec<Op> {
    let items = placeable(doc, ids);
    let reference = match items.as_slice() {
        [] => return Vec::new(),
        [(id, _)] => match doc.parent(*id).filter(|&p| doc.parent(p).is_some()).and_then(|p| doc.world_bounds(p)) {
            Some(b) => b,
            None => return Vec::new(),
        },
        many => many.iter().map(|(_, b)| *b).reduce(|a, b| a.union(b)).expect("not empty"),
    };
    items
        .iter()
        .filter_map(|&(id, b)| {
            let (dx, dy) = match to {
                Align::Left => (reference.x0 - b.x0, 0.0),
                Align::Center => (reference.center().x - b.center().x, 0.0),
                Align::Right => (reference.x1 - b.x1, 0.0),
                Align::Top => (0.0, reference.y0 - b.y0),
                Align::Middle => (0.0, reference.center().y - b.center().y),
                Align::Bottom => (0.0, reference.y1 - b.y1),
            };
            shift(doc, id, dx, dy)
        })
        .collect()
}

/// Ops spacing three or more layers evenly between the outermost two.
pub fn distribute(doc: &Document, ids: &[NodeId], horizontal: bool) -> Vec<Op> {
    let mut items = placeable(doc, ids);
    if items.len() < 3 {
        return Vec::new();
    }
    let lo = |b: &Rect| if horizontal { b.x0 } else { b.y0 };
    let len = |b: &Rect| if horizontal { b.width() } else { b.height() };
    items.sort_by(|a, b| lo(&a.1).total_cmp(&lo(&b.1)));
    let first = lo(&items[0].1);
    let end = items.iter().map(|(_, b)| lo(b) + len(b)).fold(f64::MIN, f64::max);
    let total: f64 = items.iter().map(|(_, b)| len(b)).sum();
    let gap = (end - first - total) / (items.len() - 1) as f64;
    let mut at = first;
    let mut ops = Vec::new();
    for (id, b) in &items {
        let d = at - lo(b);
        ops.extend(if horizontal { shift(doc, *id, d, 0.0) } else { shift(doc, *id, 0.0, d) });
        at += len(b) + gap;
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ApproxMeasure;

    fn setup() -> (Document, NodeId, Vec<NodeId>) {
        let mut doc = Document::new();
        let page = doc.pages()[0];
        let done = doc
            .apply(
                &[
                    Op::create(page, NodeSpec::rect("A", 10.0, 10.0).at(100.0, 100.0)),
                    Op::create(page, NodeSpec::rect("B", 10.0, 10.0).at(150.0, 120.0)),
                    Op::create(page, NodeSpec::rect("C", 10.0, 10.0).at(0.0, 0.0)),
                ],
                &ApproxMeasure,
            )
            .unwrap();
        (doc, page, done.created)
    }

    fn names(doc: &Document, parent: NodeId) -> Vec<String> {
        doc.children(parent).iter().map(|&c| doc.node(c).unwrap().name.clone()).collect()
    }

    #[test]
    fn grouping_keeps_positions_and_stack_place_and_ungroup_undoes_it() {
        let (mut doc, page, ids) = setup();
        let ops = group(&doc, &[ids[1], ids[0]], false).unwrap();
        let g = doc.apply(&ops, &ApproxMeasure).unwrap().id("group").unwrap();
        assert_eq!(names(&doc, page), ["Group 4", "C"]);
        assert_eq!(names(&doc, g), ["A", "B"]);
        let gb = doc.world_bounds(g).unwrap();
        assert_eq!((gb.x0, gb.y0, gb.x1, gb.y1), (100.0, 100.0, 160.0, 130.0));
        assert_eq!(doc.world_bounds(ids[1]).unwrap().x0, 150.0, "layers stay put on the page");

        let (ops, freed) = ungroup(&doc, &[g]).unwrap();
        doc.apply(&ops, &ApproxMeasure).unwrap();
        assert_eq!(names(&doc, page), ["A", "B", "C"]);
        assert_eq!(freed, vec![ids[0], ids[1]]);
        assert_eq!(doc.world_bounds(ids[1]).unwrap().x0, 150.0);
        assert!(!doc.contains(g));
    }

    #[test]
    fn frame_selection_wraps_the_layers_box() {
        let (mut doc, _, ids) = setup();
        let ops = group(&doc, &[ids[0], ids[1]], true).unwrap();
        let f = doc.apply(&ops, &ApproxMeasure).unwrap().id("group").unwrap();
        let n = doc.node(f).unwrap();
        assert_eq!((n.props.x, n.props.y, n.props.width, n.props.height), (100.0, 100.0, 60.0, 30.0));
        assert_eq!(doc.node(ids[0]).unwrap().props.x, 0.0);
        assert_eq!(doc.world_bounds(ids[1]).unwrap().x0, 150.0);
        assert!(group(&doc, &[ids[0], ids[2]], false).is_err(), "different parents");
    }

    #[test]
    fn align_and_distribute() {
        let (mut doc, _, ids) = setup();
        let ops = align(&doc, &[ids[0], ids[1], ids[2]], Align::Right);
        doc.apply(&ops, &ApproxMeasure).unwrap();
        assert!(ids.iter().all(|&id| doc.world_bounds(id).unwrap().x1 == 160.0));
        let ops = align(&doc, &[ids[0], ids[1], ids[2]], Align::Top);
        doc.apply(&ops, &ApproxMeasure).unwrap();
        assert!(ids.iter().all(|&id| doc.world_bounds(id).unwrap().y0 == 0.0));
        assert!(align(&doc, &[ids[0]], Align::Left).is_empty(), "a top-level layer has no frame to align to");

        // Spread along x: A at 0, C at 100, B at 20 => B moves to 50 (gaps of 40).
        let set = |doc: &mut Document, id, x| doc.apply(&[Op::set(id, Patch { x: Some(x), ..Patch::default() })], &ApproxMeasure).unwrap();
        set(&mut doc, ids[0], 0.0);
        set(&mut doc, ids[1], 20.0);
        set(&mut doc, ids[2], 100.0);
        let ops = distribute(&doc, &ids, true);
        doc.apply(&ops, &ApproxMeasure).unwrap();
        assert_eq!(doc.node(ids[1]).unwrap().props.x, 50.0);
    }

    #[test]
    fn restacking() {
        let (mut doc, page, ids) = setup();
        let run = |doc: &mut Document, sel: &[NodeId], o| {
            let ops = reorder(doc, sel, o);
            doc.apply(&ops, &ApproxMeasure).unwrap();
            names(doc, page)
        };
        assert_eq!(run(&mut doc, &[ids[0]], Order::Front), ["B", "C", "A"]);
        assert_eq!(run(&mut doc, &[ids[0]], Order::Back), ["A", "B", "C"]);
        assert_eq!(run(&mut doc, &[ids[0]], Order::Forward), ["B", "A", "C"]);
        assert_eq!(run(&mut doc, &[ids[0], ids[2]], Order::Forward), ["B", "A", "C"], "C is on top already, A stays under it");
        assert_eq!(run(&mut doc, &[ids[2]], Order::Backward), ["B", "C", "A"]);
        assert_eq!(run(&mut doc, &[ids[1], ids[2]], Order::Back), ["B", "C", "A"]);
    }
}
