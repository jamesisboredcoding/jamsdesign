//! A one-time import of a Figma file into a Blobr document, from the JSON the Figma REST API
//! returns for `GET /v1/files/:key?geometry=paths` (plus the image fills' bytes, from
//! `GET /v1/files/:key/images`).
//!
//! Comes across: pages, frames (clipping, auto layout with padding/gap/alignment/wrap and
//! hug/fill sizing), groups, sections, rectangles, ellipses, lines, vectors and booleans (as
//! vector paths from Figma's geometry), text (font, size, weight, line height, spacing,
//! alignment, auto-resize), fills (solid, gradients, images), strokes, corner radii, effects,
//! opacity, rotation, visibility and components.
//!
//! Approximated: instances become frames holding what Figma shows for them (overrides
//! applied), component sets become frames of their variants, and variables and styles are
//! left out (the REST API only lists them on Enterprise plans).

use std::collections::HashMap;

use jd_core::Rgba;
use jd_vector::{
    Align, AutoLayout, CornerRadii, CounterAlign, Direction, Document, Effect, FillRule, GradientStop, ImageFit, LineHeight, NodeKind, NodeSpec, Op,
    Padding, Paint, PaintKind, Shadow, Sizing, StrokeAlign, StrokeCap, StrokeJoin, StrokeStyle, TextAlign, TextStyle, VectorPath,
};
use serde_json::Value;

/// What happened during an import, for the user.
#[derive(Debug, Default)]
pub struct Report {
    pub pages: usize,
    pub layers: usize,
    /// Things that didn't come across exactly, with a count each.
    pub notes: HashMap<String, usize>,
}

impl Report {
    fn note(&mut self, what: &str) {
        *self.notes.entry(what.to_owned()).or_default() += 1;
    }
}

/// Builds a document from a Figma file's JSON. `images` maps Figma image refs to their bytes
/// (fills whose image is missing become grey).
pub fn import(json: &str, images: &HashMap<String, Vec<u8>>) -> Result<(Document, Report), String> {
    let file: Value = serde_json::from_str(json).map_err(|e| format!("not Figma file JSON: {e}"))?;
    let document = file.get("document").ok_or("no \"document\" in the JSON (is it from GET /v1/files/:key?)")?;
    let mut doc = Document::new();
    let mut report = Report::default();
    // Image bytes go straight into the document's store; paints refer to them by key.
    let keys: HashMap<String, String> = images.iter().map(|(r, b)| (r.clone(), doc.add_image(b.clone()))).collect();
    let canvases = arr(document, "children");
    for (i, canvas) in canvases.iter().enumerate() {
        let name = str_of(canvas, "name").unwrap_or("Page");
        let mut ops = Vec::new();
        let page: jd_vector::Ref = if i == 0 {
            let first = doc.pages()[0];
            ops.push(Op::set(first, jd_vector::Patch { name: Some(name.to_owned()), background: canvas.get("backgroundColor").map(color), ..Default::default() }));
            first.into()
        } else {
            ops.push(Op::CreatePage { name: name.to_owned(), key: Some("page".into()), index: None });
            "page".into()
        };
        let mut ctx = Ctx { keys: &keys, report: &mut report, ops: Vec::new(), n: 0 };
        for child in arr(canvas, "children") {
            ctx.node(child, page.clone(), (0.0, 0.0));
        }
        ops.append(&mut ctx.ops);
        if let Some(bg) = canvas.get("backgroundColor").map(color)
            && i > 0
        {
            ops.push(Op::set("page", jd_vector::Patch { background: Some(bg), ..Default::default() }));
        }
        doc.apply(&ops, &jd_vector_render::CosmicMeasure).map_err(|e| format!("page {name:?}: {e}"))?;
        report.pages += 1;
    }
    Ok((doc, report))
}

fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn f(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

fn color(c: &Value) -> Rgba {
    let g = |k| c.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    // Figma's 0..1 channels are sRGB.
    let mut r = Rgba::from_srgb([g("r"), g("g"), g("b")]);
    r.a = c.get("a").and_then(Value::as_f64).unwrap_or(1.0) as f32;
    r
}

struct Ctx<'a> {
    keys: &'a HashMap<String, String>,
    report: &'a mut Report,
    ops: Vec<Op>,
    n: usize,
}

/// Where a node sits in its (Blobr) parent and how it's turned: from `relativeTransform`
/// when given, else from bounding boxes. `origin` is the page position of the nearest
/// ancestor that isn't a group (groups don't have their own coordinates in Figma).
fn placement(v: &Value, origin: (f64, f64)) -> (f64, f64, f64, f64, f64) {
    let bb = v.get("absoluteBoundingBox");
    let size = v.get("size");
    let (w, h) = match (size, bb) {
        (Some(s), _) => (f(s, "x").unwrap_or(0.0), f(s, "y").unwrap_or(0.0)),
        (None, Some(b)) => (f(b, "width").unwrap_or(0.0), f(b, "height").unwrap_or(0.0)),
        _ => (0.0, 0.0),
    };
    let rt = v.get("relativeTransform").and_then(Value::as_array);
    if let Some([r0, r1]) = rt.map(Vec::as_slice) {
        let g = |row: &Value, i: usize| row.get(i).and_then(Value::as_f64).unwrap_or(0.0);
        let (a, b) = (g(r0, 0), g(r1, 0));
        let (tx, ty) = (g(r0, 2), g(r1, 2));
        // Blobr turns counter-clockwise for positive angles, as Figma's UI does.
        let rot = -b.atan2(a).to_degrees();
        return (tx, ty, w, h, (rot * 100.0).round() / 100.0);
    }
    let (x, y) = bb.map_or((0.0, 0.0), |b| (f(b, "x").unwrap_or(0.0) - origin.0, f(b, "y").unwrap_or(0.0) - origin.1));
    (x, y, w, h, 0.0)
}

impl Ctx<'_> {
    fn key(&mut self) -> String {
        self.n += 1;
        format!("n{}", self.n)
    }

    fn paints(&mut self, list: &[Value]) -> Vec<Paint> {
        list.iter().filter_map(|p| self.paint(p)).collect()
    }

    fn paint(&mut self, p: &Value) -> Option<Paint> {
        let opacity = f(p, "opacity").unwrap_or(1.0) as f32;
        let visible = p.get("visible").and_then(Value::as_bool).unwrap_or(true);
        let stops = || -> Vec<GradientStop> {
            arr(p, "gradientStops").iter().map(|s| GradientStop { position: f(s, "position").unwrap_or(0.0) as f32, color: s.get("color").map_or(Rgba::BLACK, color) }).collect()
        };
        let handle = |i: usize| -> [f64; 2] {
            arr(p, "gradientHandlePositions").get(i).map_or([0.0, 0.0], |h| [f(h, "x").unwrap_or(0.0), f(h, "y").unwrap_or(0.0)])
        };
        let kind = match str_of(p, "type")? {
            "SOLID" => PaintKind::Solid { color: p.get("color").map_or(Rgba::BLACK, color) },
            "GRADIENT_LINEAR" => PaintKind::LinearGradient { from: handle(0), to: handle(1), stops: stops() },
            "GRADIENT_RADIAL" | "GRADIENT_DIAMOND" | "GRADIENT_ANGULAR" => {
                let (c, a, b) = (handle(0), handle(1), handle(2));
                let len = |q: [f64; 2]| ((q[0] - c[0]).powi(2) + (q[1] - c[1]).powi(2)).sqrt();
                if str_of(p, "type") != Some("GRADIENT_RADIAL") {
                    self.report.note("diamond/angular gradients drawn as radial");
                }
                PaintKind::RadialGradient { center: c, radius: [len(a), len(b)], stops: stops() }
            }
            "IMAGE" => match str_of(p, "imageRef").and_then(|r| self.keys.get(r)) {
                Some(key) => PaintKind::Image {
                    image: key.clone(),
                    fit: match str_of(p, "scaleMode") {
                        Some("FIT") => ImageFit::Fit,
                        Some("STRETCH") => ImageFit::Stretch,
                        Some("TILE") => ImageFit::Tile,
                        _ => ImageFit::Fill,
                    },
                },
                None => {
                    self.report.note("image fills without their image (grey)");
                    PaintKind::Solid { color: Rgba::from_srgb8(0xD9, 0xD9, 0xD9) }
                }
            },
            other => {
                self.report.note(&format!("{other} paints left out"));
                return None;
            }
        };
        Some(Paint { kind, opacity, visible, ..Paint::hex("#000000") })
    }

    fn effects(&mut self, v: &Value) -> Vec<Effect> {
        arr(v, "effects")
            .iter()
            .filter_map(|e| {
                let visible = e.get("visible").and_then(Value::as_bool).unwrap_or(true);
                let radius = f(e, "radius").unwrap_or(0.0) as f32;
                let shadow = || Shadow {
                    color: e.get("color").map_or(Rgba::BLACK, color),
                    x: e.get("offset").and_then(|o| f(o, "x")).unwrap_or(0.0) as f32,
                    y: e.get("offset").and_then(|o| f(o, "y")).unwrap_or(0.0) as f32,
                    blur: radius,
                    spread: f(e, "spread").unwrap_or(0.0) as f32,
                    visible,
                };
                Some(match str_of(e, "type")? {
                    "DROP_SHADOW" => Effect::DropShadow(shadow()),
                    "INNER_SHADOW" => Effect::InnerShadow(shadow()),
                    "LAYER_BLUR" => Effect::LayerBlur { radius, visible },
                    "BACKGROUND_BLUR" => Effect::BackgroundBlur { radius, visible },
                    _ => return None,
                })
            })
            .collect()
    }

    /// Adds `v` (and what's in it) under `parent`.
    fn node(&mut self, v: &Value, parent: jd_vector::Ref, origin: (f64, f64)) {
        let Some(t) = str_of(v, "type") else { return };
        let name = str_of(v, "name").unwrap_or(t).to_owned();
        let (x, y, w, h, rot) = placement(v, origin);
        // How far left a widened text box moves to keep its text in place.
        let mut widened: Option<f64> = None;
        // A boolean with its parts comes in live (editable, like Figma's); one without (an
        // export that left them out) comes in as its result shape.
        let live_bool = t == "BOOLEAN_OPERATION" && !arr(v, "children").is_empty();
        // Groups and live booleans place their children in their own parent's space.
        let groupish = t == "GROUP" || live_bool;
        let mut spec = match t {
            "FRAME" | "COMPONENT" | "INSTANCE" | "COMPONENT_SET" => {
                if t == "INSTANCE" {
                    self.report.note("instances brought in as frames");
                }
                if t == "COMPONENT_SET" {
                    self.report.note("component sets brought in as frames of their variants");
                }
                NodeSpec::frame(&name, w, h)
            }
            "SECTION" => NodeSpec::new(&name, NodeKind::Section).size(w, h),
            "GROUP" => NodeSpec::group(&name),
            "BOOLEAN_OPERATION" if live_bool => {
                use jd_vector::boolean::BoolOp;
                let op = match str_of(v, "booleanOperation") {
                    Some("SUBTRACT") => BoolOp::Subtract,
                    Some("INTERSECT") => BoolOp::Intersect,
                    Some("EXCLUDE") => BoolOp::Exclude,
                    _ => BoolOp::Union,
                };
                NodeSpec::new(&name, NodeKind::Boolean { op, path: VectorPath::empty() })
            }
            "RECTANGLE" => NodeSpec::rect(&name, w, h),
            "ELLIPSE" => NodeSpec::ellipse(&name, w, h),
            "LINE" => NodeSpec::new(&name, NodeKind::Line).size(w, 0.0),
            "TEXT" => {
                let s = v.get("style").cloned().unwrap_or(Value::Null);
                let weight = f(&s, "fontWeight").unwrap_or(400.0);
                let italic = s.get("italic").and_then(Value::as_bool).unwrap_or(false);
                let style = [(100.0, "Thin"), (200.0, "ExtraLight"), (300.0, "Light"), (400.0, "Regular"), (500.0, "Medium"), (600.0, "SemiBold"), (700.0, "Bold"), (800.0, "ExtraBold"), (900.0, "Black")]
                    .iter()
                    .min_by_key(|(wt, _)| (wt - weight).abs() as i64)
                    .map_or("Regular", |(_, n)| n);
                let style = match (style, italic) {
                    ("Regular", true) => "Italic".to_owned(),
                    (s, true) => format!("{s} Italic"),
                    (s, false) => s.to_owned(),
                };
                let line_height = match str_of(&s, "lineHeightUnit") {
                    Some("PIXELS") => f(&s, "lineHeightPx").map_or(LineHeight::Auto, |v| LineHeight::Px(v as f32)),
                    Some("FONT_SIZE_%") => f(&s, "lineHeightPercentFontSize").map_or(LineHeight::Auto, |v| LineHeight::Percent(v as f32)),
                    _ => LineHeight::Auto,
                };
                let ts = TextStyle {
                    family: str_of(&s, "fontFamily").unwrap_or("Inter").to_owned(),
                    style,
                    size: f(&s, "fontSize").unwrap_or(14.0) as f32,
                    line_height,
                    letter_spacing: f(&s, "letterSpacing").unwrap_or(0.0) as f32,
                    align: match str_of(&s, "textAlignHorizontal") {
                        Some("CENTER") => TextAlign::Center,
                        Some("RIGHT") => TextAlign::Right,
                        Some("JUSTIFIED") => TextAlign::Justified,
                        _ => TextAlign::Left,
                    },
                    ..TextStyle::default()
                };
                let mut spec = NodeSpec::text(&name, str_of(v, "characters").unwrap_or(""), ts).size(w, h);
                let (sh, sv) = match str_of(v, "textAutoResize") {
                    Some("WIDTH_AND_HEIGHT") => (Sizing::Hug, Sizing::Hug),
                    Some("HEIGHT") => (Sizing::Fixed, Sizing::Hug),
                    _ => (Sizing::Fixed, Sizing::Fixed),
                };
                // One line in Figma stays one line here: if this machine's font for it runs
                // wider than Figma's box, widen the box by the difference (keeping centred or
                // right-aligned text where it was).
                if let NodeKind::Text { text } = &spec.kind
                    && sh == Sizing::Fixed
                    && !text.content.contains('\n')
                    && h < text.style.size as f64 * 1.6
                {
                    use jd_vector::TextMeasure;
                    let (mw, _) = jd_vector_render::CosmicMeasure.measure(&text.content, &text.style, None);
                    let extra = mw - w;
                    if extra > 0.01 {
                        let shift = match text.style.align {
                            TextAlign::Center => extra / 2.0,
                            TextAlign::Right => extra,
                            _ => 0.0,
                        };
                        spec.props.width = mw.ceil();
                        widened = Some(shift);
                        self.report.note("one-line text boxes widened to fit this machine's fonts");
                    }
                }
                spec.props.sizing_h = sh;
                spec.props.sizing_v = sv;
                spec
            }
            "VECTOR" | "BOOLEAN_OPERATION" | "STAR" | "REGULAR_POLYGON" => {
                // Figma's own outline of the shape (its fill geometry), or of its stroke for
                // stroke-only shapes (then the stroke paints fill that outline).
                let fills = arr(v, "fillGeometry");
                let (geo, stroke_outline) = if !fills.is_empty() { (fills, false) } else { (arr(v, "strokeGeometry"), true) };
                if geo.is_empty() {
                    self.report.note("vectors without geometry (export with ?geometry=paths)");
                    return;
                }
                let d: Vec<&str> = geo.iter().filter_map(|g| str_of(g, "path")).collect();
                let Ok(mut path) = VectorPath::from_svg(&d.join(" "), w, h) else {
                    self.report.note("vectors with unreadable paths");
                    return;
                };
                if geo.iter().any(|g| str_of(g, "windingRule") == Some("EVENODD")) {
                    path.fill_rule = FillRule::EvenOdd;
                }
                let mut spec = NodeSpec::vector(&name, path).size(w, h);
                if stroke_outline {
                    spec.props.fills = self.paints(arr(v, "strokes"));
                    spec.props.strokes.clear();
                    spec = spec.at(x, y);
                    spec.props.rotation = rot;
                    return self.finish(v, spec, parent, t, origin, false);
                }
                spec
            }
            _ => {
                self.report.note(&format!("{t} layers left out"));
                return;
            }
        };
        // A group's children are placed in the group's parent's space: the group starts at
        // the origin and then fits around them.
        if groupish {
            spec = spec.at(0.0, 0.0);
        } else {
            spec = spec.at(x - widened.unwrap_or(0.0), y);
            spec.props.rotation = rot;
        }
        self.finish(v, spec, parent, t, origin, true);
    }

    fn finish(&mut self, v: &Value, mut spec: NodeSpec, parent: jd_vector::Ref, t: &str, origin: (f64, f64), style: bool) {
        let p = &mut spec.props;
        p.visible = v.get("visible").and_then(Value::as_bool).unwrap_or(true);
        p.locked = v.get("locked").and_then(Value::as_bool).unwrap_or(false);
        p.opacity = f(v, "opacity").unwrap_or(1.0) as f32;
        if style {
            p.fills = self.paints(arr(v, "fills"));
            p.strokes = self.paints(arr(v, "strokes"));
            if !p.strokes.is_empty() || f(v, "strokeWeight").is_some() {
                p.stroke = StrokeStyle {
                    weight: f(v, "strokeWeight").unwrap_or(1.0) as f32,
                    align: match str_of(v, "strokeAlign") {
                        Some("CENTER") => StrokeAlign::Center,
                        Some("OUTSIDE") => StrokeAlign::Outside,
                        _ => StrokeAlign::Inside,
                    },
                    dash: arr(v, "strokeDashes").iter().filter_map(Value::as_f64).map(|d| d as f32).collect(),
                    cap: match str_of(v, "strokeCap") {
                        Some("ROUND") => StrokeCap::Round,
                        Some("SQUARE") => StrokeCap::Square,
                        _ => StrokeCap::None,
                    },
                    join: match str_of(v, "strokeJoin") {
                        Some("ROUND") => StrokeJoin::Round,
                        Some("BEVEL") => StrokeJoin::Bevel,
                        _ => StrokeJoin::Miter,
                    },
                };
            }
        }
        let p = &mut spec.props;
        if let Some(r) = v.get("rectangleCornerRadii").and_then(Value::as_array) {
            let g = |i: usize| r.get(i).and_then(Value::as_f64).unwrap_or(0.0);
            p.corner = CornerRadii { top_left: g(0), top_right: g(1), bottom_right: g(2), bottom_left: g(3) };
        } else if let Some(r) = f(v, "cornerRadius") {
            p.corner = CornerRadii::all(r);
        }
        if matches!(t, "FRAME" | "COMPONENT" | "INSTANCE" | "COMPONENT_SET") {
            p.clip = v.get("clipsContent").and_then(Value::as_bool).unwrap_or(false);
        }
        let effects = self.effects(v);
        let p = &mut spec.props;
        p.effects = effects;

        // Auto layout on the frame, and how it sizes in its own parent's.
        let mode = str_of(v, "layoutMode").unwrap_or("NONE");
        if mode != "NONE" {
            p.layout = Some(AutoLayout {
                direction: if mode == "VERTICAL" { Direction::Vertical } else { Direction::Horizontal },
                gap: f(v, "itemSpacing").unwrap_or(0.0),
                padding: Padding {
                    top: f(v, "paddingTop").unwrap_or(0.0),
                    right: f(v, "paddingRight").unwrap_or(0.0),
                    bottom: f(v, "paddingBottom").unwrap_or(0.0),
                    left: f(v, "paddingLeft").unwrap_or(0.0),
                },
                align: match str_of(v, "primaryAxisAlignItems") {
                    Some("CENTER") => Align::Center,
                    Some("MAX") => Align::End,
                    Some("SPACE_BETWEEN") => Align::SpaceBetween,
                    _ => Align::Start,
                },
                counter_align: match str_of(v, "counterAxisAlignItems") {
                    Some("CENTER") => CounterAlign::Center,
                    Some("MAX") => CounterAlign::End,
                    _ => CounterAlign::Start,
                },
                wrap: str_of(v, "layoutWrap") == Some("WRAP"),
                row_gap: f(v, "counterAxisSpacing").unwrap_or(0.0),
            });
            // Older files only say AUTO/FIXED per axis.
            let auto = |k: &str| str_of(v, k) == Some("AUTO");
            let (primary_hug, counter_hug) = (auto("primaryAxisSizingMode"), auto("counterAxisSizingMode"));
            let (hh, vh) = if mode == "VERTICAL" { (counter_hug, primary_hug) } else { (primary_hug, counter_hug) };
            if hh {
                p.sizing_h = Sizing::Hug;
            }
            if vh {
                p.sizing_v = Sizing::Hug;
            }
        }
        let sizing = |s: Option<&str>| match s {
            Some("HUG") => Some(Sizing::Hug),
            Some("FILL") => Some(Sizing::Fill),
            Some("FIXED") => Some(Sizing::Fixed),
            _ => None,
        };
        if let Some(s) = sizing(str_of(v, "layoutSizingHorizontal")) {
            p.sizing_h = s;
        }
        if let Some(s) = sizing(str_of(v, "layoutSizingVertical")) {
            p.sizing_v = s;
        }
        p.absolute = str_of(v, "layoutPositioning") == Some("ABSOLUTE");

        let key = self.key();
        self.ops.push(Op::create_as(&key, parent, spec));
        self.report.layers += 1;
        if t == "COMPONENT" {
            self.ops.push(Op::MakeComponent { id: key.as_str().into() });
        }
        if matches!(t, "VECTOR" | "STAR" | "REGULAR_POLYGON") || (t == "BOOLEAN_OPERATION" && arr(v, "children").is_empty()) {
            return;
        }
        // Children of groups place themselves relative to the group's parent.
        let child_origin = if t == "GROUP" || t == "BOOLEAN_OPERATION" {
            origin
        } else {
            v.get("absoluteBoundingBox").map_or(origin, |b| (f(b, "x").unwrap_or(0.0), f(b, "y").unwrap_or(0.0)))
        };
        for c in arr(v, "children") {
            self.node(c, key.as_str().into(), child_origin);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"{
      "name": "Sample",
      "document": { "id": "0:0", "type": "DOCUMENT", "children": [
        { "id": "0:1", "type": "CANVAS", "name": "Screens", "backgroundColor": {"r":0.12,"g":0.12,"b":0.12,"a":1}, "children": [
          { "id": "1:1", "type": "FRAME", "name": "Card", "clipsContent": true,
            "absoluteBoundingBox": {"x": 100, "y": 50, "width": 240, "height": 120},
            "fills": [{"type":"SOLID","color":{"r":1,"g":1,"b":1,"a":1}}],
            "cornerRadius": 12,
            "layoutMode": "VERTICAL", "itemSpacing": 8, "paddingTop": 16, "paddingRight": 16, "paddingBottom": 16, "paddingLeft": 16,
            "primaryAxisSizingMode": "AUTO", "counterAxisSizingMode": "FIXED",
            "effects": [{"type":"DROP_SHADOW","visible":true,"color":{"r":0,"g":0,"b":0,"a":0.25},"offset":{"x":0,"y":4},"radius":8,"spread":0}],
            "children": [
              { "id": "1:2", "type": "TEXT", "name": "Title", "characters": "Hello",
                "absoluteBoundingBox": {"x": 116, "y": 66, "width": 40, "height": 22},
                "style": {"fontFamily": "Inter", "fontWeight": 600, "fontSize": 18, "lineHeightUnit": "INTRINSIC_%"},
                "textAutoResize": "WIDTH_AND_HEIGHT",
                "fills": [{"type":"SOLID","color":{"r":0.1,"g":0.1,"b":0.1,"a":1}}] },
              { "id": "1:3", "type": "VECTOR", "name": "Check",
                "absoluteBoundingBox": {"x": 116, "y": 96, "width": 20, "height": 20},
                "size": {"x": 20, "y": 20},
                "relativeTransform": [[1,0,16],[0,1,46]],
                "fills": [{"type":"SOLID","color":{"r":0.3,"g":0.55,"b":1,"a":1}}],
                "fillGeometry": [{"path": "M0 10L8 18L20 2L18 0L8 14L2 8Z", "windingRule": "NONZERO"}] }
            ] },
          { "id": "2:1", "type": "GROUP", "name": "Pair", "absoluteBoundingBox": {"x": 400, "y": 400, "width": 60, "height": 20}, "children": [
            { "id": "2:2", "type": "RECTANGLE", "name": "A", "absoluteBoundingBox": {"x": 400, "y": 400, "width": 20, "height": 20}, "fills": [{"type":"IMAGE","imageRef":"abc","scaleMode":"FILL"}] },
            { "id": "2:3", "type": "ELLIPSE", "name": "B", "absoluteBoundingBox": {"x": 440, "y": 400, "width": 20, "height": 20}, "fills": [] }
          ] },
          { "id": "3:1", "type": "BOOLEAN_OPERATION", "name": "Ring", "booleanOperation": "SUBTRACT",
            "absoluteBoundingBox": {"x": 600, "y": 100, "width": 40, "height": 40},
            "fills": [{"type":"SOLID","color":{"r":1,"g":0,"b":0,"a":1}}],
            "fillGeometry": [{"path": "M0 0L40 0L40 40L0 40Z", "windingRule": "EVENODD"}],
            "children": [
              { "id": "3:2", "type": "ELLIPSE", "name": "Outer", "absoluteBoundingBox": {"x": 600, "y": 100, "width": 40, "height": 40}, "fills": [] },
              { "id": "3:3", "type": "ELLIPSE", "name": "Inner", "absoluteBoundingBox": {"x": 610, "y": 110, "width": 20, "height": 20}, "fills": [] }
            ] }
        ] }
      ] }
    }"#;

    #[test]
    fn frames_text_vectors_groups_and_images_come_across() {
        let png = tiny_png();
        let (doc, report) = import(FILE, &[("abc".to_owned(), png)].into()).unwrap();
        assert_eq!(report.pages, 1);
        assert_eq!(report.layers, 9);
        let page = doc.pages()[0];
        assert_eq!(doc.node(page).unwrap().name, "Screens");
        let top = doc.children(page);
        let card = doc.node(top[0]).unwrap();
        assert_eq!((card.name.as_str(), card.props.x, card.props.y, card.props.width), ("Card", 100.0, 50.0, 240.0));
        assert!(card.props.clip && card.props.corner.top_left == 12.0);
        let l = card.props.layout.as_ref().unwrap();
        assert_eq!((l.direction, l.gap, l.padding.top), (Direction::Vertical, 8.0, 16.0));
        assert_eq!(card.props.sizing_v, Sizing::Hug);
        assert!(matches!(&card.props.effects[0], Effect::DropShadow(s) if s.y == 4.0 && (s.color.a - 0.25).abs() < 1e-6));
        let title = doc.node(doc.children(top[0])[0]).unwrap();
        assert!(matches!(&title.kind, NodeKind::Text { text } if text.content == "Hello" && text.style.style == "SemiBold" && text.style.size == 18.0));
        let check = doc.node(doc.children(top[0])[1]).unwrap();
        assert!(matches!(&check.kind, NodeKind::Vector { path } if path.base == [20.0, 20.0]));

        let pair = top[1];
        assert!(matches!(doc.node(pair).unwrap().kind, NodeKind::Group));
        let b = doc.world_bounds(pair).unwrap();
        assert_eq!((b.x0, b.y0, b.width()), (400.0, 400.0, 60.0), "group fits its children where Figma had them");
        let a = doc.node(doc.children(pair)[0]).unwrap();
        assert!(matches!(&a.props.fills[0].kind, PaintKind::Image { image, .. } if doc.image(image).is_some()));
        assert!(report.notes.is_empty(), "{:?}", report.notes);

        // Booleans come in live, with their parts where Figma had them.
        let ring = top[2];
        let n = doc.node(ring).unwrap();
        assert!(matches!(&n.kind, NodeKind::Boolean { op: jd_vector::boolean::BoolOp::Subtract, path } if !path.subpaths.is_empty()), "{:?}", n.kind);
        assert_eq!(doc.children(ring).len(), 2);
        let b = doc.world_bounds(ring).unwrap();
        assert_eq!((b.x0, b.y0, b.width()), (600.0, 100.0, 40.0));
        assert!(matches!(&n.props.fills[0].kind, PaintKind::Solid { color } if color.r > 0.9));
    }

    fn tiny_png() -> Vec<u8> {
        // A 1x1 PNG.
        vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1F, 0x15, 0xC4,
            0x89, 0, 0, 0, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D,
            0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ]
    }
}
