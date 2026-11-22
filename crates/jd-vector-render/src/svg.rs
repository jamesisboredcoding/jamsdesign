//! SVG export.
//!
//! Text is written as glyph outlines, so the file looks the same without the fonts. Inside
//! and outside strokes are a double-width stroke clipped or masked to the shape. Background
//! blur has no SVG equivalent and is left out.

use std::fmt::Write as _;

use jd_vector::{BlendMode, Document, Effect, ImageFit, Node, NodeId, NodeKind, Paint, PaintKind, StrokeAlign, StrokeCap, StrokeJoin};
use kurbo::{Affine, BezPath, Rect};

use crate::raster::{effect_visible, image_transform, is_closed, shape_of};

/// The node (or page) as a standalone SVG document, cropped to its bounds.
pub fn export_svg(doc: &Document, id: NodeId) -> Option<String> {
    let n = doc.node(id)?;
    let bounds = if n.is_page() {
        n.children.iter().filter_map(|&c| doc.world_bounds(c)).reduce(|a, b| a.union(b)).unwrap_or(Rect::new(0.0, 0.0, 1.0, 1.0))
    } else {
        doc.world_bounds(id)?
    };
    let mut w = Writer { doc, body: String::new(), defs: String::new(), next: 0 };
    if n.is_page() {
        if let NodeKind::Page { background } = n.kind {
            let _ = write!(
                w.body,
                r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#,
                num(bounds.x0),
                num(bounds.y0),
                num(bounds.width()),
                num(bounds.height()),
                hex(background)
            );
        }
        for &c in &n.children {
            w.node(c);
        }
    } else {
        let parent = n.parent.map_or(Affine::IDENTITY, |p| doc.world_transform(p));
        let _ = write!(w.body, r#"<g transform="{}">"#, matrix(parent));
        w.node(id);
        w.body.push_str("</g>");
    }
    let mut out = String::new();
    let _ = write!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{}" height="{}" viewBox="{} {} {} {}">"#,
        num(bounds.width()),
        num(bounds.height()),
        num(bounds.x0),
        num(bounds.y0),
        num(bounds.width()),
        num(bounds.height())
    );
    if !w.defs.is_empty() {
        let _ = write!(out, "<defs>{}</defs>", w.defs);
    }
    out.push_str(&w.body);
    out.push_str("</svg>");
    Some(out)
}

struct Writer<'a> {
    doc: &'a Document,
    body: String,
    defs: String,
    next: usize,
}

fn num(v: f64) -> String {
    let r = (v * 1000.0).round() / 1000.0;
    if r == 0.0 { "0".into() } else { format!("{r}") }
}

fn matrix(a: Affine) -> String {
    let c = a.as_coeffs();
    format!("matrix({} {} {} {} {} {})", num(c[0]), num(c[1]), num(c[2]), num(c[3]), num(c[4]), num(c[5]))
}

fn hex(c: jd_core::Rgba) -> String {
    format!("#{}", c.to_hex())
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;")
}

fn blend_css(b: BlendMode) -> Option<&'static str> {
    Some(match b {
        BlendMode::PassThrough | BlendMode::Normal => return None,
        BlendMode::Darken => "darken",
        BlendMode::Multiply => "multiply",
        BlendMode::ColorBurn => "color-burn",
        BlendMode::Lighten => "lighten",
        BlendMode::Screen => "screen",
        BlendMode::ColorDodge => "color-dodge",
        BlendMode::Overlay => "overlay",
        BlendMode::SoftLight => "soft-light",
        BlendMode::HardLight => "hard-light",
        BlendMode::Difference => "difference",
        BlendMode::Exclusion => "exclusion",
        BlendMode::Hue => "hue",
        BlendMode::Saturation => "saturation",
        BlendMode::Color => "color",
        BlendMode::Luminosity => "luminosity",
    })
}

impl Writer<'_> {
    fn id(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    fn node(&mut self, id: NodeId) {
        let doc = self.doc;
        let Some(n) = doc.node(id) else { return };
        if !n.props.visible {
            return;
        }
        let mut attrs = format!(r#" transform="{}""#, matrix(Document::local_transform(n)));
        if n.props.opacity < 1.0 {
            let _ = write!(attrs, r#" opacity="{}""#, num(n.props.opacity as f64));
        }
        if let Some(b) = blend_css(n.props.blend) {
            let _ = write!(attrs, r#" style="mix-blend-mode:{b}""#);
        }
        if let Some(f) = self.filter(n) {
            let _ = write!(attrs, r#" filter="url(#{f})""#);
        }
        let _ = write!(self.body, r#"<g id="{}"{attrs}>"#, escape(&format!("{}-{}", n.name, n.id)));
        self.content(n);
        self.body.push_str("</g>");
    }

    fn content(&mut self, n: &Node) {
        let (shape, rule) = shape_of(n);
        let d = shape.to_svg();
        let rule = if rule == tiny_skia::FillRule::EvenOdd { r#" fill-rule="evenodd""# } else { "" };
        if !shape.is_empty() {
            for paint in n.props.fills.iter().filter(|p| p.visible) {
                if let Some(fill) = self.paint(paint, n) {
                    let _ = write!(self.body, r#"<path d="{d}" {fill}{rule}/>"#);
                }
            }
        }
        if n.kind.is_container() {
            let clips = n.props.clip && (n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section));
            if clips {
                let c = self.clip_path(&d);
                let _ = write!(self.body, r#"<g clip-path="url(#{c})">"#);
            }
            for &c in &n.children {
                self.node(c);
            }
            if clips {
                self.body.push_str("</g>");
            }
        }
        self.strokes(n, &shape, &d);
    }

    fn clip_path(&mut self, d: &str) -> String {
        let c = self.id("clip");
        let _ = write!(self.defs, r#"<clipPath id="{c}"><path d="{d}"/></clipPath>"#);
        c
    }

    fn strokes(&mut self, n: &Node, shape: &BezPath, d: &str) {
        let st = &n.props.stroke;
        if shape.is_empty() || st.weight <= 0.0 || !n.props.strokes.iter().any(|p| p.visible) {
            return;
        }
        let align = if is_closed(shape) && !matches!(n.kind, NodeKind::Line) { st.align } else { StrokeAlign::Center };
        let width = st.weight as f64 * if align == StrokeAlign::Center { 1.0 } else { 2.0 };
        let mut extra = format!(r#" fill="none" stroke-width="{}""#, num(width));
        match st.cap {
            StrokeCap::None => {}
            StrokeCap::Round => extra.push_str(r#" stroke-linecap="round""#),
            StrokeCap::Square => extra.push_str(r#" stroke-linecap="square""#),
        }
        match st.join {
            StrokeJoin::Miter => {}
            StrokeJoin::Round => extra.push_str(r#" stroke-linejoin="round""#),
            StrokeJoin::Bevel => extra.push_str(r#" stroke-linejoin="bevel""#),
        }
        if st.dash.len() >= 2 {
            let dash: Vec<String> = st.dash.iter().map(|v| num(*v as f64)).collect();
            let _ = write!(extra, r#" stroke-dasharray="{}""#, dash.join(" "));
        }
        match align {
            StrokeAlign::Center => {}
            StrokeAlign::Inside => {
                let c = self.clip_path(d);
                let _ = write!(extra, r#" clip-path="url(#{c})""#);
            }
            StrokeAlign::Outside => {
                let m = self.id("mask");
                let big = 1e6;
                let _ = write!(
                    self.defs,
                    r#"<mask id="{m}" maskUnits="userSpaceOnUse" x="-{big}" y="-{big}" width="{}" height="{}"><rect x="-{big}" y="-{big}" width="{}" height="{}" fill="white"/><path d="{d}" fill="black"/></mask>"#,
                    2.0 * big,
                    2.0 * big,
                    2.0 * big,
                    2.0 * big
                );
                let _ = write!(extra, r#" mask="url(#{m})""#);
            }
        }
        for paint in n.props.strokes.iter().filter(|p| p.visible) {
            if let Some(stroke) = self.paint(paint, n) {
                let stroke = stroke.replacen("fill=", "stroke=", 1).replacen("fill-opacity=", "stroke-opacity=", 1);
                let _ = write!(self.body, r#"<path d="{d}" {stroke}{extra}/>"#);
            }
        }
    }

    /// `fill="..."` (and opacity) attributes for a paint, adding gradients and patterns to defs.
    fn paint(&mut self, paint: &Paint, n: &Node) -> Option<String> {
        let (w, h) = (n.props.width, n.props.height);
        let op = paint.opacity;
        let blend = blend_css(paint.blend).map(|b| format!(r#" style="mix-blend-mode:{b}""#)).unwrap_or_default();
        let stops = |s: &[jd_vector::GradientStop]| -> String {
            s.iter()
                .map(|g| {
                    format!(
                        r#"<stop offset="{}" stop-color="{}" stop-opacity="{}"/>"#,
                        num(g.position as f64),
                        hex(g.color),
                        num((g.color.a * op) as f64)
                    )
                })
                .collect()
        };
        let id = match &paint.kind {
            PaintKind::Solid { color } => {
                let a = color.a * op;
                let opacity = if a < 1.0 { format!(r#" fill-opacity="{}""#, num(a as f64)) } else { String::new() };
                return Some(format!(r#"fill="{}"{opacity}{blend}"#, hex(*color)));
            }
            PaintKind::LinearGradient { from, to, stops: s } => {
                let g = self.id("grad");
                let _ = write!(
                    self.defs,
                    r#"<linearGradient id="{g}" gradientUnits="userSpaceOnUse" x1="{}" y1="{}" x2="{}" y2="{}">{}</linearGradient>"#,
                    num(from[0] * w),
                    num(from[1] * h),
                    num(to[0] * w),
                    num(to[1] * h),
                    stops(s)
                );
                g
            }
            PaintKind::RadialGradient { center, radius, stops: s } => {
                let (cx, cy, rx, ry) = (center[0] * w, center[1] * h, radius[0] * w, radius[1] * h);
                if rx <= 0.0 || ry <= 0.0 {
                    return None;
                }
                let g = self.id("grad");
                let squash = Affine::translate((cx, cy)) * Affine::scale_non_uniform(1.0, ry / rx) * Affine::translate((-cx, -cy));
                let _ = write!(
                    self.defs,
                    r#"<radialGradient id="{g}" gradientUnits="userSpaceOnUse" cx="{}" cy="{}" r="{}" gradientTransform="{}">{}</radialGradient>"#,
                    num(cx),
                    num(cy),
                    num(rx),
                    matrix(squash),
                    stops(s)
                );
                g
            }
            PaintKind::Image { image, fit } => {
                let bytes = self.doc.image(image)?;
                let reader = image::ImageReader::new(std::io::Cursor::new(bytes.as_slice())).with_guessed_format().ok()?;
                let mime = reader.format().map_or("image/png", |f| f.to_mime_type());
                let (iw, ih) = reader.into_dimensions().ok()?;
                let (iw, ih) = (iw as f64, ih as f64);
                let p = self.id("img");
                let local = image_transform(*fit, iw, ih, w, h);
                let (pw, ph, inner) = if *fit == ImageFit::Tile {
                    (iw, ih, String::new())
                } else {
                    (w.max(1.0), h.max(1.0), format!(r#" transform="{}""#, matrix(local)))
                };
                let _ = write!(
                    self.defs,
                    r#"<pattern id="{p}" patternUnits="userSpaceOnUse" width="{}" height="{}"><image width="{}" height="{}" preserveAspectRatio="none"{inner} opacity="{}" xlink:href="data:{mime};base64,{}"/></pattern>"#,
                    num(pw),
                    num(ph),
                    num(iw),
                    num(ih),
                    num(op as f64),
                    base64(bytes)
                );
                p
            }
        };
        Some(format!(r#"fill="url(#{id})"{blend}"#))
    }

    /// A filter for the node's shadows and layer blur.
    fn filter(&mut self, n: &Node) -> Option<String> {
        let effects: Vec<&Effect> =
            n.props.effects.iter().filter(|e| effect_visible(e) && !matches!(e, Effect::BackgroundBlur { .. })).collect();
        if effects.is_empty() {
            return None;
        }
        let f = self.id("fx");
        let mut s = format!(
            r#"<filter id="{f}" x="-50%" y="-50%" width="200%" height="200%" color-interpolation-filters="sRGB">"#
        );
        let mut merge = Vec::new();
        for (i, e) in effects.iter().enumerate() {
            if let Effect::DropShadow(sh) = e {
                let _ = write!(
                    s,
                    r#"<feMorphology in="SourceAlpha" operator="{}" radius="{}" result="sp{i}"/><feOffset in="sp{i}" dx="{}" dy="{}" result="o{i}"/><feGaussianBlur in="o{i}" stdDeviation="{}" result="b{i}"/><feFlood flood-color="{}" flood-opacity="{}"/><feComposite in2="b{i}" operator="in" result="d{i}"/>"#,
                    if sh.spread < 0.0 { "erode" } else { "dilate" },
                    num(sh.spread.abs() as f64),
                    num(sh.x as f64),
                    num(sh.y as f64),
                    num(sh.blur as f64 / 2.0),
                    hex(sh.color),
                    num(sh.color.a as f64)
                );
                merge.push(format!("d{i}"));
            }
        }
        merge.push("SourceGraphic".into());
        s.push_str(r#"<feMerge result="r0">"#);
        for m in &merge {
            let _ = write!(s, r#"<feMergeNode in="{m}"/>"#);
        }
        s.push_str("</feMerge>");
        let mut last = "r0".to_string();
        for (i, e) in effects.iter().enumerate() {
            match e {
                Effect::InnerShadow(sh) => {
                    let _ = write!(
                        s,
                        r#"<feOffset in="SourceAlpha" dx="{}" dy="{}" result="io{i}"/><feGaussianBlur in="io{i}" stdDeviation="{}" result="ib{i}"/><feComposite in="SourceAlpha" in2="ib{i}" operator="arithmetic" k2="1" k3="-1" result="ic{i}"/><feFlood flood-color="{}" flood-opacity="{}"/><feComposite in2="ic{i}" operator="in" result="is{i}"/><feComposite in="is{i}" in2="{last}" operator="over" result="r{}"/>"#,
                        num(sh.x as f64),
                        num(sh.y as f64),
                        num(sh.blur as f64 / 2.0),
                        hex(sh.color),
                        num(sh.color.a as f64),
                        i + 1
                    );
                    last = format!("r{}", i + 1);
                }
                Effect::LayerBlur { radius, .. } => {
                    let _ = write!(s, r#"<feGaussianBlur in="{last}" stdDeviation="{}" result="lb{i}"/>"#, num(*radius as f64 / 2.0));
                    last = format!("lb{i}");
                }
                _ => {}
            }
        }
        s.push_str("</filter>");
        self.defs.push_str(&s);
        Some(f)
    }
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let v = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(v >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use jd_vector::*;

    use super::*;
    use crate::raster::{RenderOptions, render_node};

    #[test]
    fn base64_matches_known_values() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }

    /// Renders the SVG export with resvg and compares it with the raster renderer.
    fn mean_difference(doc: &Document, id: NodeId) -> f64 {
        let ours = render_node(doc, id, RenderOptions::default()).unwrap();
        let svg = export_svg(doc, id).unwrap();
        let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).unwrap_or_else(|e| panic!("{e}\n{svg}"));
        let mut theirs = tiny_skia::Pixmap::new(ours.width(), ours.height()).unwrap();
        resvg::render(&tree, tiny_skia::Transform::identity(), &mut theirs.as_mut());
        // JD_DUMP=<dir> saves both renders and the SVG, to see where they differ.
        if let Ok(dir) = std::env::var("JD_DUMP") {
            let dir = std::path::Path::new(&dir);
            ours.save_png(dir.join("raster.png")).unwrap();
            theirs.save_png(dir.join("svg.png")).unwrap();
            std::fs::write(dir.join("export.svg"), &svg).unwrap();
        }
        let total: u64 = ours.data().iter().zip(theirs.data()).map(|(a, b)| (*a as i64 - *b as i64).unsigned_abs()).sum();
        total as f64 / ours.data().len() as f64
    }

    #[test]
    fn svg_export_matches_the_raster_renderer() {
        let mut doc = Document::new();
        let p = doc.pages()[0];
        let grad = Paint {
            kind: PaintKind::LinearGradient {
                from: [0.0, 0.0],
                to: [1.0, 1.0],
                stops: vec![
                    GradientStop { position: 0.0, color: Rgba::from_srgb8(0x4C, 0x8D, 0xFF) },
                    GradientStop { position: 1.0, color: Rgba::from_srgb8(0xE0, 0x1B, 0x5A) },
                ],
            },
            ..Paint::hex("#000000")
        };
        let shadow = Effect::DropShadow(Shadow { color: Rgba::from_srgb8(0, 0, 0), x: 0.0, y: 6.0, blur: 8.0, spread: 0.0, visible: true });
        let mut png = Vec::new();
        image::RgbaImage::from_fn(8, 4, |x, y| image::Rgba([(x * 32) as u8, (y * 64) as u8, 200, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let photo = Paint { kind: PaintKind::Image { image: doc.add_image(png), fit: ImageFit::Fill }, ..Paint::hex("#000000") };
        let radial = Paint {
            kind: PaintKind::RadialGradient {
                center: [0.5, 0.5],
                radius: [0.5, 0.5],
                stops: vec![
                    GradientStop { position: 0.0, color: Rgba::from_srgb8(255, 255, 255) },
                    GradientStop { position: 1.0, color: Rgba::from_srgb8(0x26, 0xA2, 0x69) },
                ],
            },
            ..Paint::hex("#000000")
        };
        let inner = Effect::InnerShadow(Shadow { color: Rgba::from_srgb8(0, 0, 0), x: 2.0, y: 2.0, blur: 2.0, spread: 0.0, visible: true });
        let text = TextStyle { family: String::new(), size: 14.0, style: "Bold".into(), ..TextStyle::default() };
        let done = doc
            .apply(
                &[
                    Op::create_as("f", p, NodeSpec::frame("Card", 200.0, 160.0).fill(Paint::hex("#FBF7F2")).radius(12.0)),
                    Op::create("f", NodeSpec::rect("Photo", 40.0, 30.0).at(140.0, 90.0).fill(photo)),
                    Op::create("f", NodeSpec::rect("Tilted", 30.0, 20.0).at(70.0, 100.0).fill(radial).with(|p| p.rotation = 30.0)),
                    Op::create("f", NodeSpec::rect("Banner", 160.0, 60.0).at(20.0, 20.0).radius(8.0).fill(grad)),
                    Op::create(
                        "f",
                        NodeSpec::ellipse("Dot", 40.0, 40.0)
                            .at(20.0, 100.0)
                            .fill(Paint::hex("#FFC53D"))
                            .stroke(Paint::hex("#1C1B22"), 3.0)
                            .with(|p| p.stroke.align = StrokeAlign::Outside),
                    ),
                    Op::create(
                        "f",
                        NodeSpec::rect("Raised", 80.0, 30.0).at(90.0, 110.0).fill(Paint::hex("#FFFFFF")).with(move |p| p.effects = vec![shadow]),
                    ),
                    Op::create("f", NodeSpec::rect("Spill", 50.0, 50.0).at(180.0, 140.0).fill(Paint::hex("#8B5CF6")).with(|p| p.opacity = 0.6)),
                    Op::create("f", NodeSpec::text("Label", "Blobr", text).at(110.0, 30.0).fill(Paint::hex("#FFFFFF"))),
                    Op::create("f", NodeSpec::rect("Well", 30.0, 16.0).at(25.0, 30.0).fill(Paint::hex("#FFFFFF")).with(move |p| p.effects = vec![inner])),
                ],
                &ApproxMeasure,
            )
            .unwrap();
        let diff = mean_difference(&doc, done.id("f").unwrap());
        assert!(diff < 2.0, "average channel difference {diff}");
    }
}
