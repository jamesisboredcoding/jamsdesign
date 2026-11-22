//! SVG import. `usvg` resolves the SVG (CSS, `use`, transforms, text into outlines with the
//! system fonts); each shape becomes a vector layer, groups become groups, and raster images
//! become image-filled rectangles, all in a frame the size of the SVG, as Figma imports it.
//! Gradients come across; patterns, masks and filters don't.

use jd_core::Rgba;
use jd_vector::{
    FillRule, GradientStop, NodeSpec, Op, Paint, PaintKind, Ref, StrokeAlign, StrokeCap, StrokeJoin, StrokeStyle, VectorPath,
};
use kurbo::{Affine, BezPath, Point, Shape};
use usvg::tiny_skia_path::PathSegment;

/// Ops that add the SVG under `parent` as a frame named `name` at (`x`, `y`). The frame's
/// key is `"svg"`.
pub fn import_svg(data: &[u8], parent: Ref, name: &str, x: f64, y: f64) -> Result<Vec<Op>, String> {
    let mut opt = usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_data(data, &opt).map_err(|e| format!("not a readable SVG: {e}"))?;
    let size = tree.size();
    let mut frame = NodeSpec::frame(name, size.width() as f64, size.height() as f64).at(x, y);
    frame.props.fills.clear();
    frame.props.clip = false;
    let mut ops = vec![Op::create_as("svg", parent, frame)];
    let mut n = 0;
    group(tree.root(), "svg", &mut ops, &mut n);
    if ops.len() == 1 {
        return Err("the SVG has nothing to import".into());
    }
    Ok(ops)
}

fn affine(t: usvg::Transform) -> Affine {
    Affine::new([t.sx as f64, t.ky as f64, t.kx as f64, t.sy as f64, t.tx as f64, t.ty as f64])
}

fn color(c: usvg::Color, alpha: f32) -> Rgba {
    let mut r = Rgba::from_srgb8(c.red, c.green, c.blue);
    r.a = alpha;
    r
}

fn group(g: &usvg::Group, parent: &str, ops: &mut Vec<Op>, n: &mut usize) {
    for child in g.children() {
        match child {
            usvg::Node::Group(sub) => {
                // Groups only matter when they hold several layers or change opacity.
                if sub.children().len() > 1 || sub.opacity().get() < 1.0 {
                    *n += 1;
                    let key = format!("g{n}");
                    let mut spec = NodeSpec::group(if sub.id().is_empty() { "Group" } else { sub.id() });
                    spec.props.opacity = sub.opacity().get();
                    ops.push(Op::create_as(&key, parent, spec));
                    group(sub, &key, ops, n);
                } else {
                    group(sub, parent, ops, n);
                }
            }
            usvg::Node::Path(p) => {
                if p.is_visible()
                    && let Some(spec) = path(p)
                {
                    ops.push(Op::create(parent, spec));
                }
            }
            usvg::Node::Image(img) => {
                if img.is_visible() {
                    ops.extend(image(img, parent));
                }
            }
            usvg::Node::Text(t) => group(t.flattened(), parent, ops, n),
        }
    }
}

/// A path's outline in the frame's space.
fn bez(p: &usvg::Path) -> BezPath {
    let t = affine(p.abs_transform());
    let pt = |q: usvg::tiny_skia_path::Point| t * Point::new(q.x as f64, q.y as f64);
    let mut b = BezPath::new();
    for seg in p.data().segments() {
        match seg {
            PathSegment::MoveTo(a) => b.move_to(pt(a)),
            PathSegment::LineTo(a) => b.line_to(pt(a)),
            PathSegment::QuadTo(c, a) => b.quad_to(pt(c), pt(a)),
            PathSegment::CubicTo(c1, c2, a) => b.curve_to(pt(c1), pt(c2), pt(a)),
            PathSegment::Close => b.close_path(),
        }
    }
    b
}

/// A usvg paint as ours; gradient points become fractions of `bb` (the layer's box).
fn paint(p: &usvg::Paint, opacity: f32, abs: Affine, bb: kurbo::Rect) -> Option<Paint> {
    let frac = |q: Point| [(q.x - bb.x0) / bb.width().max(1e-9), (q.y - bb.y0) / bb.height().max(1e-9)];
    let stops = |s: &[usvg::Stop]| -> Vec<GradientStop> {
        s.iter().map(|s| GradientStop { position: s.offset().get(), color: color(s.color(), s.opacity().get()) }).collect()
    };
    let kind = match p {
        usvg::Paint::Color(c) => PaintKind::Solid { color: color(*c, 1.0) },
        usvg::Paint::LinearGradient(g) => {
            let t = abs * affine(g.transform());
            PaintKind::LinearGradient {
                from: frac(t * Point::new(g.x1() as f64, g.y1() as f64)),
                to: frac(t * Point::new(g.x2() as f64, g.y2() as f64)),
                stops: stops(g.stops()),
            }
        }
        usvg::Paint::RadialGradient(g) => {
            let t = abs * affine(g.transform());
            let c = t * Point::new(g.cx() as f64, g.cy() as f64);
            let r = g.r().get() as f64;
            let edge_x = t * Point::new(g.cx() as f64 + r, g.cy() as f64);
            let edge_y = t * Point::new(g.cx() as f64, g.cy() as f64 + r);
            let [cx, cy] = frac(c);
            PaintKind::RadialGradient {
                center: [cx, cy],
                radius: [(edge_x - c).hypot() / bb.width().max(1e-9), (edge_y - c).hypot() / bb.height().max(1e-9)],
                stops: stops(g.stops()),
            }
        }
        usvg::Paint::Pattern(_) => return None,
    };
    Some(Paint { kind, opacity, ..Paint::hex("#000000") })
}

fn path(p: &usvg::Path) -> Option<NodeSpec> {
    let page = bez(p);
    let bb = page.bounding_box();
    if !(bb.width() > 0.0 || bb.height() > 0.0) {
        return None;
    }
    let local = Affine::translate(-bb.origin().to_vec2()) * page;
    let mut vp = VectorPath::from_bez(&local, bb.width(), bb.height());
    let abs = affine(p.abs_transform());
    let name = if p.id().is_empty() { "Vector" } else { p.id() };
    let mut spec = NodeSpec::vector(name, vp.clone()).at(bb.x0, bb.y0);
    spec.props.fills.clear();
    spec.props.strokes.clear();
    if let Some(f) = p.fill() {
        if f.rule() == usvg::FillRule::EvenOdd {
            vp.fill_rule = FillRule::EvenOdd;
            spec = NodeSpec { kind: jd_vector::NodeKind::Vector { path: vp }, ..spec };
        }
        spec.props.fills.extend(paint(f.paint(), f.opacity().get(), abs, bb));
    }
    if let Some(s) = p.stroke() {
        spec.props.strokes.extend(paint(s.paint(), s.opacity().get(), abs, bb));
        // Strokes scale with the transform (its average scale).
        let [a, b, c, d, ..] = abs.as_coeffs();
        let scale = ((a * a + b * b).sqrt() + (c * c + d * d).sqrt()) / 2.0;
        spec.props.stroke = StrokeStyle {
            weight: (s.width().get() as f64 * scale) as f32,
            // SVG strokes straddle the outline.
            align: StrokeAlign::Center,
            dash: s.dasharray().map(|d| d.iter().map(|v| (*v as f64 * scale) as f32).collect()).unwrap_or_default(),
            cap: match s.linecap() {
                usvg::LineCap::Butt => StrokeCap::None,
                usvg::LineCap::Round => StrokeCap::Round,
                usvg::LineCap::Square => StrokeCap::Square,
            },
            join: match s.linejoin() {
                usvg::LineJoin::Round => StrokeJoin::Round,
                usvg::LineJoin::Bevel => StrokeJoin::Bevel,
                _ => StrokeJoin::Miter,
            },
        };
    }
    (!spec.props.fills.is_empty() || !spec.props.strokes.is_empty()).then_some(spec)
}

/// A raster image as a pasted, image-filled rectangle (that's how image bytes get into a
/// document within a batch).
fn image(img: &usvg::Image, parent: &str) -> Option<Op> {
    let bytes = match img.kind() {
        usvg::ImageKind::PNG(d) | usvg::ImageKind::JPEG(d) | usvg::ImageKind::GIF(d) | usvg::ImageKind::WEBP(d) => d.as_ref().clone(),
        usvg::ImageKind::SVG(_) => return None,
    };
    let b = img.abs_bounding_box();
    let clip = jd_vector::clip::Clip::image(if img.id().is_empty() { "Image" } else { img.id() }, b.width() as f64, b.height() as f64, bytes);
    Some(Op::Paste { parent: parent.into(), clip, index: None, key: None, offset: [b.x() as f64, b.y() as f64] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jd_vector::{ApproxMeasure, Document, NodeKind};

    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="60">
        <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/></linearGradient></defs>
        <g id="shapes" opacity="0.5">
            <rect id="box" x="10" y="10" width="30" height="20" fill="url(#g)"/>
            <circle cx="70" cy="30" r="20" fill="#4C8DFF" stroke="#000" stroke-width="2" transform="translate(5 0)"/>
        </g>
    </svg>"##;

    #[test]
    fn svg_shapes_groups_and_gradients_come_in() {
        let mut doc = Document::new();
        let page = doc.pages()[0];
        let ops = import_svg(SVG.as_bytes(), page.into(), "icon", 0.0, 0.0).unwrap();
        let done = doc.apply(&ops, &ApproxMeasure).unwrap();
        let frame = done.id("svg").unwrap();
        let f = doc.node(frame).unwrap();
        assert_eq!((f.props.width, f.props.height), (100.0, 60.0));
        let g = doc.children(frame)[0];
        assert_eq!(doc.node(g).unwrap().name, "shapes");
        assert_eq!(doc.node(g).unwrap().props.opacity, 0.5);
        let kids = doc.children(g);
        let rect = doc.node(kids[0]).unwrap();
        let rb = doc.world_bounds(kids[0]).unwrap();
        assert_eq!((rb.x0, rb.y0, rb.width(), rb.height()), (10.0, 10.0, 30.0, 20.0), "the rect is where it was on the page");
        assert!(matches!(&rect.props.fills[0].kind, PaintKind::LinearGradient { from, to, .. } if from[0].abs() < 1e-6 && (to[0] - 1.0).abs() < 1e-6));
        let circle = doc.node(kids[1]).unwrap();
        assert!(matches!(circle.kind, NodeKind::Vector { .. }));
        let cb = doc.world_bounds(kids[1]).unwrap();
        assert!((cb.x0 - 55.0).abs() < 0.01, "translated: {}", cb.x0);
        assert_eq!(circle.props.stroke.weight, 2.0);
        assert_eq!(circle.props.stroke.align, StrokeAlign::Center);
    }
}
