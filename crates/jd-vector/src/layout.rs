//! Auto layout, hugging text and group bounds. Runs after every batch of ops.
//!
//! For an auto layout frame:
//! 1. children lay themselves out first, so hugging children know their size;
//! 2. the frame hugs them (if it hugs) on either axis;
//! 3. children that fill share the space left along the direction, or take the full size
//!    across it, and lay their own content out again at that size;
//! 4. children are placed by `align` and `counter_align`.
//!
//! Hidden and absolutely positioned children don't take part in the flow (absolute ones keep
//! their `x`/`y`). Rotation is ignored for layout: a rotated child takes its unrotated size.
//!
//! Only values that actually change are written, so untouched nodes keep their `Arc` and a
//! batch reports exactly what moved.

use kurbo::{Affine, Point, Rect};

use crate::doc::Document;
use crate::node::*;
use crate::text::TextMeasure;

const EPS: f64 = 1e-9;

pub fn run(doc: &mut Document, measure: &dyn TextMeasure) {
    for page in doc.pages().to_vec() {
        for child in doc.children(page).to_vec() {
            layout(doc, child, Given::NONE, measure);
        }
    }
}

/// Axes whose size the parent has already decided (filling), so the node mustn't hug them.
#[derive(Clone, Copy)]
struct Given {
    w: bool,
    h: bool,
}

impl Given {
    const NONE: Self = Self { w: false, h: false };
}

fn layout(doc: &mut Document, id: NodeId, given: Given, m: &dyn TextMeasure) {
    let Some(n) = doc.node(id) else { return };
    let (kind, props) = (n.kind.clone(), n.props.clone());
    match &kind {
        NodeKind::Text { text } => {
            let hug_w = props.sizing_h == Sizing::Hug && !given.w;
            let hug_h = props.sizing_v == Sizing::Hug && !given.h;
            if hug_w {
                let (w, h) = m.measure(&text.content, &text.style, None);
                let h = if hug_h { props.clamp_height(h) } else { props.height };
                set_size(doc, id, props.clamp_width(w), h);
            } else if hug_h {
                let (_, h) = m.measure(&text.content, &text.style, Some(props.width));
                set_size(doc, id, props.width, props.clamp_height(h));
            }
        }
        NodeKind::Frame if props.layout.is_some() => auto(doc, id, given, m),
        NodeKind::Group => {
            for c in doc.children(id).to_vec() {
                layout(doc, c, Given::NONE, m);
            }
            fit_group(doc, id);
        }
        _ => {
            for c in doc.children(id).to_vec() {
                layout(doc, c, Given::NONE, m);
            }
        }
    }
}

fn size_of(doc: &Document, id: NodeId) -> (f64, f64) {
    let p = &doc.node(id).expect("child").props;
    (p.width, p.height)
}

fn set_size(doc: &mut Document, id: NodeId, w: f64, h: f64) {
    let p = &doc.node(id).expect("node").props;
    if (p.width - w).abs() > EPS || (p.height - h).abs() > EPS {
        let p = &mut doc.node_mut(id).expect("node").props;
        p.width = w;
        p.height = h;
    }
}

fn set_pos(doc: &mut Document, id: NodeId, x: f64, y: f64) {
    let p = &doc.node(id).expect("node").props;
    if (p.x - x).abs() > EPS || (p.y - y).abs() > EPS {
        let p = &mut doc.node_mut(id).expect("node").props;
        p.x = x;
        p.y = y;
    }
}

/// Splits (width, height) into (along the direction, across it) and back.
#[derive(Clone, Copy)]
struct Axes {
    horizontal: bool,
}

impl Axes {
    fn split(self, w: f64, h: f64) -> (f64, f64) {
        if self.horizontal { (w, h) } else { (h, w) }
    }

    fn join(self, main: f64, cross: f64) -> (f64, f64) {
        self.split(main, cross)
    }

    fn sizing(self, p: &Props) -> (Sizing, Sizing) {
        if self.horizontal { (p.sizing_h, p.sizing_v) } else { (p.sizing_v, p.sizing_h) }
    }

    fn mins(self, p: &Props) -> (f64, f64) {
        let (w, h) = (p.min_width.unwrap_or(0.0), p.min_height.unwrap_or(0.0));
        self.split(w, h)
    }

    fn clamp(self, p: &Props, main: f64, cross: f64) -> (f64, f64) {
        let (w, h) = self.join(main, cross);
        self.split(p.clamp_width(w), p.clamp_height(h))
    }
}

fn auto(doc: &mut Document, id: NodeId, given: Given, m: &dyn TextMeasure) {
    let n = doc.node(id).expect("node");
    let props = n.props.clone();
    let l = props.layout.clone().expect("auto layout");
    let kids = n.children.clone();
    let ax = Axes { horizontal: l.direction == Direction::Horizontal };

    let in_flow = |doc: &Document, c: NodeId| {
        let p = &doc.node(c).expect("child").props;
        p.visible && !p.absolute
    };
    let flow: Vec<NodeId> = kids.iter().copied().filter(|&c| in_flow(doc, c)).collect();
    for &c in &kids {
        layout(doc, c, Given::NONE, m);
    }

    if l.wrap && ax.horizontal {
        wrap(doc, id, &props, &l, &flow, given);
        return;
    }

    let (pad_main0, pad_main1, pad_cross0, pad_cross1) = if ax.horizontal {
        (l.padding.left, l.padding.right, l.padding.top, l.padding.bottom)
    } else {
        (l.padding.top, l.padding.bottom, l.padding.left, l.padding.right)
    };
    let gaps = l.gap * flow.len().saturating_sub(1) as f64;

    let need = needs(doc, &flow, ax);
    let (own_main, own_cross) = ax.split(props.width, props.height);
    let (s_main, s_cross) = ax.sizing(&props);
    let (given_main, given_cross) = if ax.horizontal { (given.w, given.h) } else { (given.h, given.w) };
    let hug_main = s_main == Sizing::Hug && !given_main;
    let hug_cross = s_cross == Sizing::Hug && !given_cross;
    let own_size = |need: &Needs| {
        let main = if hug_main { pad_main0 + need.fixed_main + need.fill_min + gaps + pad_main1 } else { own_main };
        let cross = if hug_cross { pad_cross0 + need.cross + pad_cross1 } else { own_cross };
        ax.clamp(&props, main, cross)
    };
    let (main, cross) = own_size(&need);
    let (w, h) = ax.join(main, cross);
    set_size(doc, id, w, h);

    // Filling children.
    let inner_main = (main - pad_main0 - pad_main1).max(0.0);
    let inner_cross = (cross - pad_cross0 - pad_cross1).max(0.0);
    let share = if need.fill_count > 0 { ((inner_main - need.fixed_main - gaps) / need.fill_count as f64).max(0.0) } else { 0.0 };
    for &c in &flow {
        let cp = doc.node(c).expect("child").props.clone();
        let (s_main, s_cross) = ax.sizing(&cp);
        if s_main != Sizing::Fill && s_cross != Sizing::Fill {
            continue;
        }
        let (cur_main, cur_cross) = ax.split(cp.width, cp.height);
        let main = if s_main == Sizing::Fill { share } else { cur_main };
        let cross = if s_cross == Sizing::Fill { inner_cross } else { cur_cross };
        let (main, cross) = ax.clamp(&cp, main, cross);
        let (w, h) = ax.join(main, cross);
        set_size(doc, c, w, h);
        let (fill_main, fill_cross) = (s_main == Sizing::Fill, s_cross == Sizing::Fill);
        let given = if ax.horizontal { Given { w: fill_main, h: fill_cross } } else { Given { w: fill_cross, h: fill_main } };
        layout(doc, c, given, m);
    }

    // A filling child can change size along the other axis too (text that wraps once it has
    // a width), so a hugging frame measures again.
    let (main, cross) = own_size(&needs(doc, &flow, ax));
    let (w, h) = ax.join(main, cross);
    set_size(doc, id, w, h);
    let inner_main = (main - pad_main0 - pad_main1).max(0.0);
    let inner_cross = (cross - pad_cross0 - pad_cross1).max(0.0);

    // Placing.
    let sizes: Vec<(f64, f64)> = flow
        .iter()
        .map(|&c| {
            let (w, h) = size_of(doc, c);
            ax.split(w, h)
        })
        .collect();
    let used: f64 = sizes.iter().map(|s| s.0).sum();
    let (mut cursor, spacing) = match l.align {
        Align::Start => (pad_main0, l.gap),
        Align::Center => (pad_main0 + (inner_main - used - gaps) / 2.0, l.gap),
        Align::End => (pad_main0 + inner_main - used - gaps, l.gap),
        Align::SpaceBetween if flow.len() > 1 => (pad_main0, (inner_main - used) / (flow.len() - 1) as f64),
        Align::SpaceBetween => (pad_main0, 0.0),
    };
    for (&c, &(main, cross)) in flow.iter().zip(&sizes) {
        let cross_pos = match l.counter_align {
            CounterAlign::Start => pad_cross0,
            CounterAlign::Center => pad_cross0 + (inner_cross - cross) / 2.0,
            CounterAlign::End => pad_cross0 + inner_cross - cross,
        };
        let (x, y) = ax.join(cursor, cross_pos);
        set_pos(doc, c, x, y);
        cursor += main + spacing;
    }
}

/// What the flowing children need along and across the direction.
struct Needs {
    /// Total size of the children that don't fill along the direction.
    fixed_main: f64,
    /// Minimum sizes of the ones that do.
    fill_min: f64,
    fill_count: usize,
    /// Largest size across the direction.
    cross: f64,
}

fn needs(doc: &Document, flow: &[NodeId], ax: Axes) -> Needs {
    let mut n = Needs { fixed_main: 0.0, fill_min: 0.0, fill_count: 0, cross: 0.0 };
    for &c in flow {
        let cp = &doc.node(c).expect("child").props;
        let (main, cross) = ax.split(cp.width, cp.height);
        let (s_main, s_cross) = ax.sizing(cp);
        let (min_main, min_cross) = ax.mins(cp);
        if s_main == Sizing::Fill {
            n.fill_count += 1;
            n.fill_min += min_main;
        } else {
            n.fixed_main += main;
        }
        n.cross = n.cross.max(if s_cross == Sizing::Fill { min_cross } else { cross });
    }
    n
}

/// Horizontal wrapping layout: rows fill up left to right, then start again below.
fn wrap(doc: &mut Document, id: NodeId, props: &Props, l: &AutoLayout, flow: &[NodeId], given: Given) {
    let pad = l.padding;
    let hug_w = props.sizing_h == Sizing::Hug && !given.w;
    let hug_h = props.sizing_v == Sizing::Hug && !given.h;
    let limit = if hug_w { f64::INFINITY } else { (props.width - pad.left - pad.right).max(0.0) };

    let mut rows: Vec<Vec<NodeId>> = Vec::new();
    let mut row_w = 0.0;
    for &c in flow {
        let (w, _) = size_of(doc, c);
        match rows.last_mut() {
            Some(row) if row_w + l.gap + w <= limit + EPS => {
                row.push(c);
                row_w += l.gap + w;
            }
            _ => {
                rows.push(vec![c]);
                row_w = w;
            }
        }
    }
    let row_width = |doc: &Document, row: &[NodeId]| -> f64 {
        row.iter().map(|&c| size_of(doc, c).0).sum::<f64>() + l.gap * row.len().saturating_sub(1) as f64
    };
    let row_height = |doc: &Document, row: &[NodeId]| row.iter().map(|&c| size_of(doc, c).1).fold(0.0, f64::max);

    let content_h: f64 =
        rows.iter().map(|r| row_height(doc, r)).sum::<f64>() + l.row_gap * rows.len().saturating_sub(1) as f64;
    let w = if hug_w { pad.left + rows.iter().map(|r| row_width(doc, r)).fold(0.0, f64::max) + pad.right } else { props.width };
    let h = if hug_h { pad.top + content_h + pad.bottom } else { props.height };
    let (w, h) = (props.clamp_width(w), props.clamp_height(h));
    set_size(doc, id, w, h);

    let inner_w = (w - pad.left - pad.right).max(0.0);
    let mut y = pad.top;
    for row in &rows {
        let (used_w, rh) = (row_width(doc, row), row_height(doc, row));
        let items: f64 = row.iter().map(|&c| size_of(doc, c).0).sum();
        let (mut x, spacing) = match l.align {
            Align::Start => (pad.left, l.gap),
            Align::Center => (pad.left + (inner_w - used_w) / 2.0, l.gap),
            Align::End => (pad.left + inner_w - used_w, l.gap),
            Align::SpaceBetween if row.len() > 1 => (pad.left, (inner_w - items) / (row.len() - 1) as f64),
            Align::SpaceBetween => (pad.left, 0.0),
        };
        for &c in row {
            let (cw, ch) = size_of(doc, c);
            let cy = match l.counter_align {
                CounterAlign::Start => y,
                CounterAlign::Center => y + (rh - ch) / 2.0,
                CounterAlign::End => y + rh - ch,
            };
            set_pos(doc, c, x, cy);
            x += cw + spacing;
        }
        y += rh + l.row_gap;
    }
}

/// Shrinks or grows a group to its children's bounds, keeping the children where they are on
/// the page.
fn fit_group(doc: &mut Document, id: NodeId) {
    let kids = doc.children(id).to_vec();
    let mut bounds: Option<Rect> = None;
    for &c in &kids {
        let n = doc.node(c).expect("child");
        if !n.props.visible {
            continue;
        }
        let t = Document::local_transform(n);
        let (w, h) = (n.props.width, n.props.height);
        for (x, y) in [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)] {
            let p = t * Point::new(x, y);
            bounds = Some(bounds.map_or(Rect::from_points(p, p), |b| b.union_pt(p)));
        }
    }
    let Some(b) = bounds else { return };
    if b.x0.abs() > EPS || b.y0.abs() > EPS {
        for &c in &kids {
            let p = &doc.node(c).expect("child").props;
            let (x, y) = (p.x - b.x0, p.y - b.y0);
            set_pos(doc, c, x, y);
        }
        let g = &doc.node(id).expect("group").props;
        // The group's own rotation turns the offset into its parent's space.
        let offset = Affine::rotate(-g.rotation.to_radians()) * Point::new(b.x0, b.y0);
        let (x, y) = (g.x + offset.x, g.y + offset.y);
        set_pos(doc, id, x, y);
    }
    set_size(doc, id, b.width(), b.height());
}

#[cfg(test)]
mod tests {
    use crate::ops::{NodeSpec, Op, Patch};
    use crate::*;

    fn row(name: &str, l: AutoLayout) -> NodeSpec {
        NodeSpec::frame(name, 0.0, 0.0).auto_layout(l)
    }

    fn build(ops: Vec<Op>) -> (Document, Applied) {
        let mut doc = Document::new();
        let done = doc.apply(&ops, &ApproxMeasure).unwrap();
        (doc, done)
    }

    fn rect_of(doc: &Document, done: &Applied, key: &str) -> (f64, f64, f64, f64) {
        let p = &doc.node(done.id(key).unwrap()).unwrap().props;
        (p.x, p.y, p.width, p.height)
    }

    fn page() -> Ref {
        Ref::Id(NodeId(1))
    }

    #[test]
    fn hugging_row_adds_up_children_gaps_and_padding() {
        let l = AutoLayout { gap: 8.0, padding: Padding::hv(24.0, 16.0), counter_align: CounterAlign::Center, ..AutoLayout::default() };
        let (doc, done) = build(vec![
            Op::create_as("row", page(), row("Row", l)),
            Op::create_as("a", "row", NodeSpec::rect("A", 20.0, 10.0)),
            Op::create_as("b", "row", NodeSpec::rect("B", 30.0, 30.0)),
        ]);
        assert_eq!(rect_of(&doc, &done, "row"), (0.0, 0.0, 24.0 + 20.0 + 8.0 + 30.0 + 24.0, 16.0 + 30.0 + 16.0));
        assert_eq!(rect_of(&doc, &done, "a"), (24.0, 26.0, 20.0, 10.0), "centred across");
        assert_eq!(rect_of(&doc, &done, "b"), (52.0, 16.0, 30.0, 30.0));
    }

    #[test]
    fn fill_children_share_what_is_left() {
        let l = AutoLayout { gap: 10.0, ..AutoLayout::default() };
        let (doc, done) = build(vec![
            Op::create_as("row", page(), row("Row", l).sizing(Sizing::Fixed, Sizing::Fixed).size(300.0, 40.0)),
            Op::create_as("fixed", "row", NodeSpec::rect("Fixed", 80.0, 20.0)),
            Op::create_as("f1", "row", NodeSpec::rect("F1", 0.0, 20.0).sizing(Sizing::Fill, Sizing::Fill)),
            Op::create_as("f2", "row", NodeSpec::rect("F2", 0.0, 20.0).sizing(Sizing::Fill, Sizing::Fixed)),
        ]);
        // 300 - 80 - 2 gaps of 10 = 200, split in two.
        assert_eq!(rect_of(&doc, &done, "f1"), (90.0, 0.0, 100.0, 40.0));
        assert_eq!(rect_of(&doc, &done, "f2"), (200.0, 0.0, 100.0, 20.0));
    }

    #[test]
    fn vertical_space_between_and_end_alignment() {
        let l = AutoLayout {
            direction: Direction::Vertical,
            align: Align::SpaceBetween,
            counter_align: CounterAlign::End,
            ..AutoLayout::default()
        };
        let (doc, done) = build(vec![
            Op::create_as("col", page(), row("Col", l).sizing(Sizing::Hug, Sizing::Fixed).size(0.0, 100.0)),
            Op::create_as("a", "col", NodeSpec::rect("A", 10.0, 10.0)),
            Op::create_as("b", "col", NodeSpec::rect("B", 40.0, 20.0)),
        ]);
        assert_eq!(rect_of(&doc, &done, "col").2, 40.0);
        assert_eq!(rect_of(&doc, &done, "a"), (30.0, 0.0, 10.0, 10.0));
        assert_eq!(rect_of(&doc, &done, "b"), (0.0, 80.0, 40.0, 20.0));
    }

    #[test]
    fn hidden_and_absolute_children_stay_out_of_the_flow() {
        let (doc, done) = build(vec![
            Op::create_as("row", page(), row("Row", AutoLayout::default())),
            Op::create_as("a", "row", NodeSpec::rect("A", 10.0, 10.0)),
            Op::create_as("hidden", "row", NodeSpec::rect("Hidden", 50.0, 50.0).with(|p| p.visible = false)),
            Op::create_as("abs", "row", NodeSpec::rect("Abs", 50.0, 50.0).at(100.0, 100.0).with(|p| p.absolute = true)),
            Op::create_as("b", "row", NodeSpec::rect("B", 10.0, 10.0)),
        ]);
        assert_eq!(rect_of(&doc, &done, "row"), (0.0, 0.0, 20.0, 10.0));
        assert_eq!(rect_of(&doc, &done, "b").0, 10.0);
        assert_eq!(rect_of(&doc, &done, "abs"), (100.0, 100.0, 50.0, 50.0));
    }

    #[test]
    fn text_hugs_and_wraps_when_it_fills() {
        let style = TextStyle { size: 10.0, ..TextStyle::default() };
        let l = AutoLayout { direction: Direction::Vertical, padding: Padding::all(5.0), ..AutoLayout::default() };
        let (doc, done) = build(vec![
            Op::create_as("card", page(), row("Card", l).sizing(Sizing::Fixed, Sizing::Hug).size(50.0, 0.0)),
            Op::create_as("title", "card", NodeSpec::text("Title", "Hi", style.clone())),
            Op::create_as("body", "card", NodeSpec::text("Body", "hello world", style).sizing(Sizing::Fill, Sizing::Hug)),
        ]);
        assert_eq!(rect_of(&doc, &done, "title"), (5.0, 5.0, 12.0, 12.0));
        // 40 px wide: "hello world" wraps onto two 12 px lines.
        assert_eq!(rect_of(&doc, &done, "body"), (5.0, 17.0, 40.0, 24.0));
        assert_eq!(rect_of(&doc, &done, "card").3, 5.0 + 12.0 + 24.0 + 5.0);
    }

    #[test]
    fn nested_hugging_frames_grow_outwards() {
        let (mut doc, done) = build(vec![
            Op::create_as("outer", page(), row("Outer", AutoLayout { padding: Padding::all(10.0), ..AutoLayout::default() })),
            Op::create_as("inner", "outer", row("Inner", AutoLayout { padding: Padding::all(5.0), ..AutoLayout::default() })),
            Op::create_as("box", "inner", NodeSpec::rect("Box", 20.0, 20.0)),
        ]);
        assert_eq!(rect_of(&doc, &done, "outer").2, 50.0);
        let grown = doc
            .apply(&[Op::set(done.id("box").unwrap(), Patch { width: Some(40.0), ..Patch::default() })], &ApproxMeasure)
            .unwrap();
        assert_eq!(rect_of(&doc, &done, "outer").2, 70.0);
        let outer = done.id("outer").unwrap();
        assert!(grown.changed.contains(&outer), "layout changes are reported");
    }

    #[test]
    fn wrapping_rows_break_when_full() {
        let l = AutoLayout { gap: 10.0, row_gap: 5.0, wrap: true, ..AutoLayout::default() };
        let (doc, done) = build(vec![
            Op::create_as("chips", page(), row("Chips", l).sizing(Sizing::Fixed, Sizing::Hug).size(100.0, 0.0)),
            Op::create_as("a", "chips", NodeSpec::rect("A", 40.0, 20.0)),
            Op::create_as("b", "chips", NodeSpec::rect("B", 40.0, 20.0)),
            Op::create_as("c", "chips", NodeSpec::rect("C", 40.0, 30.0)),
        ]);
        assert_eq!(rect_of(&doc, &done, "b").0, 50.0);
        assert_eq!(rect_of(&doc, &done, "c"), (0.0, 25.0, 40.0, 30.0));
        assert_eq!(rect_of(&doc, &done, "chips").3, 20.0 + 5.0 + 30.0);
    }

    #[test]
    fn groups_fit_their_children() {
        let (doc, done) = build(vec![
            Op::create_as("g", page(), NodeSpec::group("G").at(100.0, 100.0)),
            Op::create_as("a", "g", NodeSpec::rect("A", 10.0, 10.0).at(20.0, 30.0)),
            Op::create_as("b", "g", NodeSpec::rect("B", 10.0, 10.0).at(50.0, 40.0)),
        ]);
        assert_eq!(rect_of(&doc, &done, "g"), (120.0, 130.0, 40.0, 20.0));
        assert_eq!(rect_of(&doc, &done, "a"), (0.0, 0.0, 10.0, 10.0));
        let world = doc.world_bounds(done.id("b").unwrap()).unwrap();
        assert_eq!((world.x0, world.y0), (150.0, 140.0), "children didn't move on the page");
    }
}
