//! CPU rendering with tiny-skia.
//!
//! A node draws: its fills, then its children (clipped if it clips), then its strokes, the
//! same order as Figma. A node with opacity, a blend mode or effects is drawn into its own
//! layer first, which gets its effects and is then composited.
//!
//! Colour maths happens on sRGB-encoded values, like browsers, Figma and Photoshop's default,
//! so colours match what designers expect.

use std::collections::HashMap;

use jd_core::Rgba;
use jd_vector::{
    BlendMode, Document, Effect, FillRule, ImageFit, Node, NodeId, NodeKind, Paint, PaintKind, Shadow, StrokeAlign, StrokeCap,
    StrokeJoin, geometry,
};
use kurbo::{Affine, BezPath, PathEl, Rect};
use tiny_skia as sk;

#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    /// Device pixels per document pixel.
    pub scale: f64,
    /// Paint this behind everything (transparent when `None`).
    pub background: Option<Rgba>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self { scale: 1.0, background: None }
    }
}

/// Renders a node (or a whole page) cropped to its bounds.
pub fn render_node(doc: &Document, id: NodeId, opts: RenderOptions) -> Option<sk::Pixmap> {
    let n = doc.node(id)?;
    if n.is_page() {
        let mut bounds: Option<Rect> = None;
        for &c in &n.children {
            if let Some(b) = doc.world_bounds(c) {
                bounds = Some(bounds.map_or(b, |a| a.union(b)));
            }
        }
        let bg = opts.background.or(match n.kind {
            NodeKind::Page { background } => Some(background),
            _ => None,
        });
        return render_region(doc, id, bounds.unwrap_or(Rect::new(0.0, 0.0, 1.0, 1.0)), RenderOptions { background: bg, ..opts });
    }
    let bounds = doc.world_bounds(id)?;
    let (mut pm, base) = canvas(bounds, opts)?;
    let parent = n.parent.map_or(Affine::IDENTITY, |p| doc.world_transform(p));
    let mut ctx = Ctx::new(doc, pm.width(), pm.height());
    ctx.draw(&mut pm, id, base * parent, None);
    Some(pm)
}

/// Renders the part of `page` inside `view` (page coordinates), e.g. what a window shows.
pub fn render_region(doc: &Document, page: NodeId, view: Rect, opts: RenderOptions) -> Option<sk::Pixmap> {
    let (mut pm, base) = canvas(view, opts)?;
    let mut ctx = Ctx::new(doc, pm.width(), pm.height());
    for &c in doc.children(page) {
        ctx.draw(&mut pm, c, base, None);
    }
    Some(pm)
}

/// A node as PNG bytes.
pub fn render_png(doc: &Document, id: NodeId, opts: RenderOptions) -> Result<Vec<u8>, String> {
    let pm = render_node(doc, id, opts).ok_or_else(|| format!("node {id} has nothing to render"))?;
    pm.encode_png().map_err(|e| e.to_string())
}

fn canvas(rect: Rect, opts: RenderOptions) -> Option<(sk::Pixmap, Affine)> {
    let s = opts.scale.max(1e-6);
    let w = (rect.width() * s).ceil().clamp(1.0, 16384.0) as u32;
    let h = (rect.height() * s).ceil().clamp(1.0, 16384.0) as u32;
    let mut pm = sk::Pixmap::new(w, h)?;
    if let Some(bg) = opts.background {
        pm.fill(color(bg, 1.0));
    }
    Some((pm, Affine::scale(s) * Affine::translate((-rect.x0, -rect.y0))))
}

pub(crate) fn to_ts(a: Affine) -> sk::Transform {
    let [a, b, c, d, e, f] = a.as_coeffs().map(|v| v as f32);
    sk::Transform::from_row(a, b, c, d, e, f)
}

pub(crate) fn to_path(p: &BezPath) -> Option<sk::Path> {
    let mut pb = sk::PathBuilder::new();
    let f = |v: f64| v as f32;
    for el in p.elements() {
        match *el {
            PathEl::MoveTo(a) => pb.move_to(f(a.x), f(a.y)),
            PathEl::LineTo(a) => pb.line_to(f(a.x), f(a.y)),
            PathEl::QuadTo(a, b) => pb.quad_to(f(a.x), f(a.y), f(b.x), f(b.y)),
            PathEl::CurveTo(a, b, c) => pb.cubic_to(f(a.x), f(a.y), f(b.x), f(b.y), f(c.x), f(c.y)),
            PathEl::ClosePath => pb.close(),
        }
    }
    pb.finish()
}

fn color(c: Rgba, alpha: f32) -> sk::Color {
    let [r, g, b] = c.to_srgb();
    sk::Color::from_rgba(r, g, b, (c.a * alpha).clamp(0.0, 1.0)).unwrap_or(sk::Color::TRANSPARENT)
}

fn blend(b: BlendMode) -> sk::BlendMode {
    use sk::BlendMode as S;
    match b {
        BlendMode::PassThrough | BlendMode::Normal => S::SourceOver,
        BlendMode::Darken => S::Darken,
        BlendMode::Multiply => S::Multiply,
        BlendMode::ColorBurn => S::ColorBurn,
        BlendMode::Lighten => S::Lighten,
        BlendMode::Screen => S::Screen,
        BlendMode::ColorDodge => S::ColorDodge,
        BlendMode::Overlay => S::Overlay,
        BlendMode::SoftLight => S::SoftLight,
        BlendMode::HardLight => S::HardLight,
        BlendMode::Difference => S::Difference,
        BlendMode::Exclusion => S::Exclusion,
        BlendMode::Hue => S::Hue,
        BlendMode::Saturation => S::Saturation,
        BlendMode::Color => S::Color,
        BlendMode::Luminosity => S::Luminosity,
    }
}

/// The path a node is filled and stroked with, in its own space.
pub(crate) fn shape_of(n: &Node) -> (BezPath, sk::FillRule) {
    match &n.kind {
        NodeKind::Text { text } => {
            (crate::text::text_outline(&text.content, &text.style, n.props.width, n.props.height), sk::FillRule::Winding)
        }
        NodeKind::Vector { path } => (
            geometry::outline(n),
            if path.fill_rule == FillRule::EvenOdd { sk::FillRule::EvenOdd } else { sk::FillRule::Winding },
        ),
        NodeKind::Group | NodeKind::Page { .. } => (BezPath::new(), sk::FillRule::Winding),
        _ => (geometry::outline(n), sk::FillRule::Winding),
    }
}

/// Whether a path is closed (strokes can only be inside/outside a closed shape).
pub(crate) fn is_closed(p: &BezPath) -> bool {
    p.elements().iter().any(|e| matches!(e, PathEl::ClosePath))
}

struct Ctx<'a> {
    doc: &'a Document,
    images: HashMap<String, Option<sk::Pixmap>>,
    /// Size of the pixmap being drawn into (the canvas, or a node's own layer).
    w: u32,
    h: u32,
    /// Where that pixmap sits on the canvas (non-zero inside a node's layer).
    offset: kurbo::Vec2,
    /// What each node (with everything in it) covers, in canvas pixels; `None` if nothing.
    vis: HashMap<NodeId, Option<Rect>>,
}

impl<'a> Ctx<'a> {
    fn new(doc: &'a Document, w: u32, h: u32) -> Self {
        Self { doc, images: HashMap::new(), w, h, offset: kurbo::Vec2::ZERO, vis: HashMap::new() }
    }

    /// The pixels `id` can touch, in the current target's space (`t`: its transform there).
    /// Shadows, blurs and outside strokes count; children count unless clipped away.
    fn vis(&mut self, id: NodeId, t: Affine) -> Option<Rect> {
        if let Some(r) = self.vis.get(&id) {
            return r.map(|r| r - self.offset);
        }
        let doc = self.doc;
        let n = doc.node(id)?;
        let r = if !n.props.visible {
            None
        } else {
            let scale = t.determinant().abs().sqrt();
            let own = match &n.kind {
                NodeKind::Group => None,
                NodeKind::Text { .. } => {
                    let (shape, _) = shape_of(n);
                    let b = kurbo::Shape::bounding_box(&shape);
                    (b.area() > 0.0).then(|| t.transform_rect_bbox(b.union(Rect::new(0.0, 0.0, n.props.width, n.props.height))))
                }
                _ => Some(t.transform_rect_bbox(Rect::new(0.0, 0.0, n.props.width, n.props.height))),
            };
            let clips = n.props.clip && (n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section));
            let mut all = own;
            if n.kind.is_container() && !clips {
                for &c in &n.children {
                    let Some(cn) = doc.node(c) else { continue };
                    if let Some(cb) = self.vis(c, t * Document::local_transform(cn)) {
                        all = Some(all.map_or(cb, |a| a.union(cb)));
                    }
                }
            }
            all.map(|a| a.inflate(1.0, 1.0) + margin(n, scale))
        };
        self.vis.insert(id, r.map(|r| r + self.offset));
        r
    }

    /// Whether everything inside a clipping frame stays within it, so clipping would change
    /// nothing. Only for frames drawn without rotation; rounded corners count as cut off.
    fn children_fit(&mut self, n: &Node, t: Affine) -> bool {
        let [a, b, c, d, ..] = t.as_coeffs();
        if b.abs() > 1e-9 || c.abs() > 1e-9 || a <= 0.0 || d <= 0.0 {
            return false;
        }
        let k = n.props.corner;
        let r = k.top_left.max(k.top_right).max(k.bottom_right).max(k.bottom_left) * a.max(d);
        let inner = t.transform_rect_bbox(Rect::new(0.0, 0.0, n.props.width, n.props.height)).inset(-r);
        n.children.iter().all(|&ch| {
            let Some(cn) = self.doc.node(ch) else { return true };
            match self.vis(ch, t * Document::local_transform(cn)) {
                None => true,
                Some(v) => {
                    // vis is padded by a pixel; allow for that.
                    let v = v.inset(-1.0);
                    v.x0 >= inner.x0 && v.y0 >= inner.y0 && v.x1 <= inner.x1 && v.y1 <= inner.y1
                }
            }
        })
    }

    fn full_mask(&self) -> sk::Mask {
        let mut m = sk::Mask::new(self.w, self.h).expect("canvas size is valid");
        m.data_mut().fill(255);
        m
    }

    /// `clip` narrowed to a path.
    fn clip_to(&self, clip: Option<&sk::Mask>, path: &sk::Path, rule: sk::FillRule, t: sk::Transform) -> sk::Mask {
        let mut m = clip.cloned().unwrap_or_else(|| self.full_mask());
        m.intersect_path(path, rule, true, t);
        m
    }

    /// `clip` narrowed to everything outside a path.
    fn clip_outside(&self, clip: Option<&sk::Mask>, path: &sk::Path, rule: sk::FillRule, t: sk::Transform) -> sk::Mask {
        let mut m = sk::Mask::new(self.w, self.h).expect("canvas size is valid");
        m.fill_path(path, rule, true, t);
        m.invert();
        if let Some(c) = clip {
            for (a, b) in m.data_mut().iter_mut().zip(c.data()) {
                *a = ((*a as u16 * *b as u16 + 127) / 255) as u8;
            }
        }
        m
    }

    fn draw(&mut self, target: &mut sk::Pixmap, id: NodeId, parent: Affine, clip: Option<&sk::Mask>) {
        let doc = self.doc;
        let Some(n) = doc.node(id) else { return };
        if !n.props.visible {
            return;
        }
        let t = parent * Document::local_transform(n);
        // Off screen (with everything it draws): skip it.
        let Some(v) = self.vis(id, t) else { return };
        let screen = Rect::new(0.0, 0.0, self.w as f64, self.h as f64);
        if !overlaps(v, screen) {
            return;
        }
        let scale = t.determinant().abs().sqrt();
        let effects: Vec<&Effect> = n.props.effects.iter().filter(|e| effect_visible(e)).collect();
        for e in &effects {
            if let Effect::BackgroundBlur { radius, .. } = e {
                self.backdrop_blur(target, n, t, clip, *radius as f64 * scale / 2.0);
            }
        }
        let layered = effects.iter().any(|e| !matches!(e, Effect::BackgroundBlur { .. }));
        let isolate = n.props.opacity < 1.0 || !matches!(n.props.blend, BlendMode::Normal | BlendMode::PassThrough) || layered;
        if !isolate {
            self.content(target, n, t, clip);
            return;
        }
        // Its own layer covers only what it draws (shadows included), not the whole canvas.
        let r = v.intersect(screen);
        let (x0, y0) = (r.x0.floor(), r.y0.floor());
        let (lw, lh) = ((r.x1.ceil() - x0).max(1.0) as u32, (r.y1.ceil() - y0).max(1.0) as u32);
        let Some(mut layer) = sk::Pixmap::new(lw, lh) else { return };
        let saved = (self.w, self.h, self.offset);
        (self.w, self.h) = (lw, lh);
        self.offset += kurbo::Vec2::new(x0, y0);
        self.content(&mut layer, n, Affine::translate((-x0, -y0)) * t, None);
        (self.w, self.h, self.offset) = saved;
        apply_effects(&mut layer, &effects, scale);
        let paint = sk::PixmapPaint { opacity: n.props.opacity, blend_mode: blend(n.props.blend), quality: sk::FilterQuality::Nearest };
        target.draw_pixmap(x0 as i32, y0 as i32, layer.as_ref(), &paint, sk::Transform::identity(), clip);
    }

    fn content(&mut self, target: &mut sk::Pixmap, n: &'a Node, t: Affine, clip: Option<&sk::Mask>) {
        let (w, h) = (n.props.width, n.props.height);
        let ts = to_ts(t);
        let (shape, rule) = shape_of(n);
        let path = to_path(&shape);

        if let Some(p) = &path {
            for paint in n.props.fills.iter().filter(|p| p.visible) {
                self.load(paint);
                if let PaintKind::Image { fit: ImageFit::Fit, .. } = paint.kind {
                    // Fit leaves empty space around the image: fill only where the image is.
                    let Some(r) = self.image_rect(paint, w, h) else { continue };
                    let Some(rp) = to_path(&kurbo::Shape::to_path(&r, 0.1)) else { continue };
                    let mask = self.clip_to(clip, p, rule, ts);
                    if let Some(shader) = self.shader(paint, w, h) {
                        let sp = sk::Paint { shader, blend_mode: blend(paint.blend), anti_alias: true, ..sk::Paint::default() };
                        target.fill_path(&rp, &sp, sk::FillRule::Winding, ts, Some(&mask));
                    }
                    continue;
                }
                if let Some(shader) = self.shader(paint, w, h) {
                    let sp = sk::Paint { shader, blend_mode: blend(paint.blend), anti_alias: true, ..sk::Paint::default() };
                    target.fill_path(p, &sp, rule, ts, clip);
                }
            }
        }

        if n.kind.is_container() {
            let clips = n.props.clip && (n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section));
            // Clipping costs a canvas-sized mask; skip it when everything inside fits anyway.
            let clips = clips && !self.children_fit(n, t);
            // Or draw the children into a layer just the frame's size, clipped by a mask that
            // size, when that looks the same: nothing inside blends with what's underneath.
            // (Big frames are cheaper with the canvas-sized mask than a big layer to blend.)
            let screen = Rect::new(0.0, 0.0, self.w as f64, self.h as f64);
            let r = t.transform_rect_bbox(Rect::new(0.0, 0.0, w, h)).intersect(screen);
            if clips
                && let Some(p) = &path
                && r.area() < screen.area() / 4.0
                && n.children.iter().all(|&c| plain(self.doc, c))
            {
                if r.width() > 0.0 && r.height() > 0.0 {
                    let (x0, y0) = (r.x0.floor(), r.y0.floor());
                    let (lw, lh) = ((r.x1.ceil() - x0).max(1.0) as u32, (r.y1.ceil() - y0).max(1.0) as u32);
                    if let (Some(mut layer), Some(mut mask)) = (sk::Pixmap::new(lw, lh), sk::Mask::new(lw, lh)) {
                        let lt = Affine::translate((-x0, -y0)) * t;
                        mask.fill_path(p, rule, true, to_ts(lt));
                        let saved = (self.w, self.h, self.offset);
                        (self.w, self.h) = (lw, lh);
                        self.offset += kurbo::Vec2::new(x0, y0);
                        for &c in &n.children {
                            self.draw(&mut layer, c, lt, Some(&mask));
                        }
                        (self.w, self.h, self.offset) = saved;
                        target.draw_pixmap(x0 as i32, y0 as i32, layer.as_ref(), &sk::PixmapPaint::default(), sk::Transform::identity(), clip);
                    }
                }
                self.stroke(target, n, &shape, &path, rule, ts, clip);
                return;
            }
            let own = match (&path, clips) {
                (Some(p), true) => Some(self.clip_to(clip, p, rule, ts)),
                _ => None,
            };
            let child_clip = own.as_ref().or(clip);
            for &c in &n.children {
                self.draw(target, c, t, child_clip);
            }
        }

        self.stroke(target, n, &shape, &path, rule, ts, clip);
    }

    /// The node's strokes, on top of its fills and children.
    #[allow(clippy::too_many_arguments)]
    fn stroke(&mut self, target: &mut sk::Pixmap, n: &'a Node, shape: &BezPath, path: &Option<sk::Path>, rule: sk::FillRule, ts: sk::Transform, clip: Option<&sk::Mask>) {
        let (w, h) = (n.props.width, n.props.height);
        let st = &n.props.stroke;
        if let Some(p) = path
            && st.weight > 0.0
            && n.props.strokes.iter().any(|s| s.visible)
        {
            let mut align = if is_closed(shape) && !matches!(n.kind, NodeKind::Line) { st.align } else { StrokeAlign::Center };
            // Boxes and ellipses: an inside or outside stroke is a centred stroke on a
            // smaller or bigger copy of the shape, which needs no mask.
            let mut p = p.clone();
            // (Not for dashes: the copy's path starts elsewhere, which would move them.)
            if align != StrokeAlign::Center
                && st.dash.len() < 2
                && let Some(moved) = offset_shape(n, if align == StrokeAlign::Inside { -(st.weight as f64) / 2.0 } else { st.weight as f64 / 2.0 })
                && let Some(mp) = to_path(&moved)
            {
                p = mp;
                align = StrokeAlign::Center;
            }
            let p = &p;
            let width = st.weight * if align == StrokeAlign::Center { 1.0 } else { 2.0 };
            let mask = match align {
                StrokeAlign::Center => None,
                StrokeAlign::Inside => Some(self.clip_to(clip, p, rule, ts)),
                StrokeAlign::Outside => Some(self.clip_outside(clip, p, rule, ts)),
            };
            let mask = mask.as_ref().or(clip);
            let stroke = sk::Stroke {
                width,
                miter_limit: 4.0,
                line_cap: match st.cap {
                    StrokeCap::None => sk::LineCap::Butt,
                    StrokeCap::Round => sk::LineCap::Round,
                    StrokeCap::Square => sk::LineCap::Square,
                },
                line_join: match st.join {
                    StrokeJoin::Miter => sk::LineJoin::Miter,
                    StrokeJoin::Round => sk::LineJoin::Round,
                    StrokeJoin::Bevel => sk::LineJoin::Bevel,
                },
                dash: if st.dash.len() >= 2 { sk::StrokeDash::new(even(&st.dash), 0.0) } else { None },
            };
            for paint in n.props.strokes.iter().filter(|p| p.visible) {
                self.load(paint);
                if let Some(shader) = self.shader(paint, w, h) {
                    let sp = sk::Paint { shader, blend_mode: blend(paint.blend), anti_alias: true, ..sk::Paint::default() };
                    target.stroke_path(p, &sp, &stroke, ts, mask);
                }
            }
        }
    }

    /// Blurs what's already drawn underneath the node's shape.
    fn backdrop_blur(&self, target: &mut sk::Pixmap, n: &Node, t: Affine, clip: Option<&sk::Mask>, sigma: f64) {
        let (shape, rule) = shape_of(n);
        let Some(p) = to_path(&shape) else { return };
        let mut blurred = target.clone();
        blur_rgba(&mut blurred, sigma);
        let mask = self.clip_to(clip, &p, rule, to_ts(t));
        let paint = sk::PixmapPaint { opacity: 1.0, blend_mode: sk::BlendMode::Source, quality: sk::FilterQuality::Nearest };
        target.draw_pixmap(0, 0, blurred.as_ref(), &paint, sk::Transform::identity(), Some(&mask));
    }

    fn load(&mut self, paint: &Paint) {
        let PaintKind::Image { image, .. } = &paint.kind else { return };
        if self.images.contains_key(image) {
            return;
        }
        let decoded = self.doc.image(image).and_then(|bytes| decode(bytes));
        self.images.insert(image.clone(), decoded);
    }

    /// Where an image paint's image sits in the node's box.
    fn image_rect(&self, paint: &Paint, w: f64, h: f64) -> Option<Rect> {
        let PaintKind::Image { image, fit } = &paint.kind else { return None };
        let pm = self.images.get(image)?.as_ref()?;
        let local = image_transform(*fit, pm.width() as f64, pm.height() as f64, w, h);
        Some(local.transform_rect_bbox(Rect::new(0.0, 0.0, pm.width() as f64, pm.height() as f64)))
    }

    /// A paint's shader in the node's own space (tiny-skia applies the path's transform to the
    /// shader as well, so it mustn't be applied here too).
    fn shader(&self, paint: &Paint, w: f64, h: f64) -> Option<sk::Shader<'_>> {
        let op = paint.opacity;
        let at = |p: [f64; 2]| sk::Point::from_xy((p[0] * w) as f32, (p[1] * h) as f32);
        let stops = |s: &[jd_vector::GradientStop]| -> Vec<sk::GradientStop> {
            s.iter().map(|g| sk::GradientStop::new(g.position, color(g.color, op))).collect()
        };
        match &paint.kind {
            PaintKind::Solid { color: c } => Some(sk::Shader::SolidColor(color(*c, op))),
            PaintKind::LinearGradient { from, to, stops: s } => {
                sk::LinearGradient::new(at(*from), at(*to), stops(s), sk::SpreadMode::Pad, sk::Transform::identity())
            }
            PaintKind::RadialGradient { center, radius, stops: s } => {
                let c = (center[0] * w, center[1] * h);
                let (rx, ry) = (radius[0] * w, radius[1] * h);
                if rx <= 0.0 || ry <= 0.0 {
                    return None;
                }
                let squash = Affine::translate(c) * Affine::scale_non_uniform(1.0, ry / rx) * Affine::translate((-c.0, -c.1));
                let cp = sk::Point::from_xy(c.0 as f32, c.1 as f32);
                sk::RadialGradient::new(cp, cp, rx as f32, stops(s), sk::SpreadMode::Pad, to_ts(squash))
            }
            PaintKind::Image { image, fit } => {
                let pm = self.images.get(image)?.as_ref()?;
                let local = image_transform(*fit, pm.width() as f64, pm.height() as f64, w, h);
                let spread = if *fit == ImageFit::Tile { sk::SpreadMode::Repeat } else { sk::SpreadMode::Pad };
                Some(sk::Pattern::new(pm.as_ref(), spread, sk::FilterQuality::Bicubic, op, to_ts(local)))
            }
        }
    }
}

/// Maps image pixels into a `w` x `h` box.
pub(crate) fn image_transform(fit: ImageFit, iw: f64, ih: f64, w: f64, h: f64) -> Affine {
    if iw <= 0.0 || ih <= 0.0 {
        return Affine::IDENTITY;
    }
    let centred = |s: f64| Affine::translate(((w - iw * s) / 2.0, (h - ih * s) / 2.0)) * Affine::scale(s);
    match fit {
        ImageFit::Fill => centred((w / iw).max(h / ih)),
        ImageFit::Fit => centred((w / iw).min(h / ih)),
        ImageFit::Stretch => Affine::scale_non_uniform(w / iw, h / ih),
        ImageFit::Tile => Affine::IDENTITY,
    }
}

/// A dash pattern needs an even number of entries.
/// Whether a node and everything in it blend normally and don't blur what's under them
/// (so drawing them into a separate layer first looks the same).
fn plain(doc: &Document, id: NodeId) -> bool {
    let Some(n) = doc.node(id) else { return true };
    matches!(n.props.blend, BlendMode::Normal | BlendMode::PassThrough)
        && n.props.fills.iter().chain(&n.props.strokes).all(|p| matches!(p.blend, BlendMode::Normal | BlendMode::PassThrough))
        && !n.props.effects.iter().any(|e| matches!(e, Effect::BackgroundBlur { .. }) && effect_visible(e))
        && n.children.iter().all(|&c| plain(doc, c))
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

/// How far past its box a node can draw, in pixels: outside strokes, shadows, blurs.
fn margin(n: &Node, scale: f64) -> kurbo::Insets {
    let mut m: f64 = 0.0;
    if n.props.strokes.iter().any(|s| s.visible) {
        let w = n.props.stroke.weight as f64;
        // Ends of open paths, caps and miters can reach past half the width.
        m = m.max(match n.props.stroke.align {
            StrokeAlign::Inside => 0.0,
            _ => w * 2.0,
        });
    }
    for e in n.props.effects.iter().filter(|e| effect_visible(e)) {
        let reach = match e {
            Effect::DropShadow(s) => (s.x.abs().max(s.y.abs()) + s.blur * 1.5 + s.spread.max(0.0)) as f64,
            Effect::LayerBlur { radius, .. } => *radius as f64 * 1.5,
            _ => 0.0,
        };
        m = m.max(reach);
    }
    kurbo::Insets::uniform(m * scale)
}

/// A box or ellipse grown (or shrunk, if `d` < 0) by `d` on every side, with its corners
/// following; `None` for other shapes.
fn offset_shape(n: &Node, d: f64) -> Option<BezPath> {
    let (w, h) = (n.props.width, n.props.height);
    let r = Rect::new(-d, -d, w + d, h + d);
    if r.width() <= 0.0 || r.height() <= 0.0 {
        return None;
    }
    match &n.kind {
        NodeKind::Frame | NodeKind::Rectangle | NodeKind::Section | NodeKind::Component { .. } | NodeKind::ComponentSet { .. } | NodeKind::Instance { .. } => {
            let c = n.props.corner;
            let rad = |v: f64| if v > 0.0 { (v + d).max(0.0) } else { 0.0 };
            let radii = kurbo::RoundedRectRadii::new(rad(c.top_left), rad(c.top_right), rad(c.bottom_right), rad(c.bottom_left));
            Some(kurbo::Shape::to_path(&kurbo::RoundedRect::from_rect(r, radii), 0.1))
        }
        NodeKind::Ellipse { arc } if arc.sweep.abs() >= 360.0 && arc.inner <= 0.0 => Some(kurbo::Shape::to_path(&kurbo::Ellipse::from_rect(r), 0.1)),
        _ => None,
    }
}

fn even(d: &[f32]) -> Vec<f32> {
    if d.len() % 2 == 0 { d.to_vec() } else { d.iter().chain(d).copied().collect() }
}

pub(crate) fn effect_visible(e: &Effect) -> bool {
    match e {
        Effect::DropShadow(s) | Effect::InnerShadow(s) => s.visible,
        Effect::LayerBlur { visible, .. } | Effect::BackgroundBlur { visible, .. } => *visible,
    }
}

fn decode(bytes: &[u8]) -> Option<sk::Pixmap> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let mut data = img.into_raw();
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as u16;
        for c in &mut px[..3] {
            *c = ((*c as u16 * a + 127) / 255) as u8;
        }
    }
    sk::Pixmap::from_vec(data, sk::IntSize::from_wh(w, h)?)
}

/// Shadows and layer blur, applied to a node's own layer. `scale` is device pixels per
/// document pixel.
fn apply_effects(layer: &mut sk::Pixmap, effects: &[&Effect], scale: f64) {
    let (w, h) = (layer.width() as usize, layer.height() as usize);
    let alpha: Vec<f32> = layer.data().chunks_exact(4).map(|p| p[3] as f32 / 255.0).collect();

    for e in effects {
        if let Effect::InnerShadow(s) = e {
            // Light from outside the shape, shifted in and blurred, kept inside the shape.
            let mut a: Vec<f32> = alpha.iter().map(|v| 1.0 - v).collect();
            a = shift(&a, w, h, s.x as f64 * scale, s.y as f64 * scale, 1.0);
            blur_plane(&mut a, w, h, s.blur as f64 * scale / 2.0, 1.0);
            let shade = shade_pixmap(&a, w, h, s, Some(&alpha));
            layer.draw_pixmap(0, 0, shade.as_ref(), &sk::PixmapPaint::default(), sk::Transform::identity(), None);
        }
    }

    let drops: Vec<&Shadow> = effects.iter().filter_map(|e| if let Effect::DropShadow(s) = e { Some(s) } else { None }).collect();
    if !drops.is_empty() {
        let mut under = sk::Pixmap::new(w as u32, h as u32).expect("same size as the layer");
        for s in drops {
            let mut a = shift(&alpha, w, h, s.x as f64 * scale, s.y as f64 * scale, 0.0);
            let spread = (s.spread as f64 * scale).round() as i64;
            if spread != 0 {
                a = spread_plane(&a, w, h, spread);
            }
            blur_plane(&mut a, w, h, s.blur as f64 * scale / 2.0, 0.0);
            let shade = shade_pixmap(&a, w, h, s, None);
            under.draw_pixmap(0, 0, shade.as_ref(), &sk::PixmapPaint::default(), sk::Transform::identity(), None);
        }
        under.draw_pixmap(0, 0, layer.as_ref(), &sk::PixmapPaint::default(), sk::Transform::identity(), None);
        *layer = under;
    }

    for e in effects {
        if let Effect::LayerBlur { radius, .. } = e {
            blur_rgba(layer, *radius as f64 * scale / 2.0);
        }
    }
}

/// A shadow colour with coverage `a` (times `within`, if given).
fn shade_pixmap(a: &[f32], w: usize, h: usize, s: &Shadow, within: Option<&[f32]>) -> sk::Pixmap {
    let mut pm = sk::Pixmap::new(w as u32, h as u32).expect("same size as the layer");
    let c = color(s.color, 1.0).premultiply();
    let rgba = [c.red(), c.green(), c.blue(), c.alpha()];
    for (i, px) in pm.data_mut().chunks_exact_mut(4).enumerate() {
        let k = (a[i] * within.map_or(1.0, |m| m[i])).clamp(0.0, 1.0);
        for ch in 0..4 {
            px[ch] = (rgba[ch] * k * 255.0).round() as u8;
        }
    }
    pm
}

/// Moves a plane by (dx, dy) pixels, filling what's uncovered with `edge`.
fn shift(a: &[f32], w: usize, h: usize, dx: f64, dy: f64, edge: f32) -> Vec<f32> {
    let (dx, dy) = (dx.round() as i64, dy.round() as i64);
    if dx == 0 && dy == 0 {
        return a.to_vec();
    }
    let mut out = vec![edge; a.len()];
    for y in 0..h as i64 {
        let sy = y - dy;
        if sy < 0 || sy >= h as i64 {
            continue;
        }
        for x in 0..w as i64 {
            let sx = x - dx;
            if sx >= 0 && sx < w as i64 {
                out[(y as usize) * w + x as usize] = a[(sy as usize) * w + sx as usize];
            }
        }
    }
    out
}

/// Grows (or, negative, shrinks) coverage by `r` pixels: a square max (min) filter.
fn spread_plane(a: &[f32], w: usize, h: usize, r: i64) -> Vec<f32> {
    let grow = r > 0;
    let r = r.unsigned_abs() as usize;
    let pick = |x: f32, y: f32| if grow { x.max(y) } else { x.min(y) };
    let mut tmp = a.to_vec();
    for y in 0..h {
        for x in 0..w {
            let (lo, hi) = (x.saturating_sub(r), (x + r).min(w - 1));
            tmp[y * w + x] = (lo..=hi).map(|i| a[y * w + i]).reduce(pick).unwrap_or(0.0);
        }
    }
    let mut out = tmp.clone();
    for y in 0..h {
        let (lo, hi) = (y.saturating_sub(r), (y + r).min(h - 1));
        for x in 0..w {
            out[y * w + x] = (lo..=hi).map(|j| tmp[j * w + x]).reduce(pick).unwrap_or(0.0);
        }
    }
    out
}

/// Box sizes whose three passes approximate a Gaussian of `sigma`.
fn box_radii(sigma: f64) -> [usize; 3] {
    let n = 3.0;
    let ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut wl = ideal.floor() as i64;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let m = ((12.0 * sigma * sigma - n * (wl * wl) as f64 - 4.0 * n * wl as f64 - 3.0 * n) / (-4.0 * wl as f64 - 4.0)).round();
    [0, 1, 2].map(|i| (((if (i as f64) < m { wl } else { wu }) - 1) / 2).max(0) as usize)
}

/// Gaussian-ish blur of one plane; pixels past the edge count as `edge`.
fn blur_plane(a: &mut [f32], w: usize, h: usize, sigma: f64, edge: f32) {
    if sigma < 0.3 || w == 0 || h == 0 {
        return;
    }
    let mut line = Vec::new();
    for r in box_radii(sigma) {
        if r == 0 {
            continue;
        }
        for y in 0..h {
            line.clear();
            line.extend_from_slice(&a[y * w..(y + 1) * w]);
            box_1d(&line, &mut a[y * w..(y + 1) * w], r, edge);
        }
        for x in 0..w {
            line.clear();
            line.extend((0..h).map(|y| a[y * w + x]));
            let mut col = vec![0.0; h];
            box_1d(&line, &mut col, r, edge);
            for (y, v) in col.into_iter().enumerate() {
                a[y * w + x] = v;
            }
        }
    }
}

fn box_1d(src: &[f32], dst: &mut [f32], r: usize, edge: f32) {
    let n = src.len() as i64;
    let at = |i: i64| if i < 0 || i >= n { edge } else { src[i as usize] };
    let span = (2 * r + 1) as f32;
    let mut sum: f32 = (-(r as i64)..=r as i64).map(at).sum();
    for i in 0..n {
        dst[i as usize] = sum / span;
        sum += at(i + r as i64 + 1) - at(i - r as i64);
    }
}

/// Blurs all four (premultiplied) channels.
fn blur_rgba(pm: &mut sk::Pixmap, sigma: f64) {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    for ch in 0..4 {
        let mut plane: Vec<f32> = pm.data().chunks_exact(4).map(|p| p[ch] as f32).collect();
        blur_plane(&mut plane, w, h, sigma, 0.0);
        for (px, v) in pm.data_mut().chunks_exact_mut(4).zip(plane) {
            px[ch] = v.round().clamp(0.0, 255.0) as u8;
        }
    }
    // Rounding can leave a colour channel above alpha, which isn't valid premultiplied colour.
    for px in pm.data_mut().chunks_exact_mut(4) {
        let a = px[3];
        for c in &mut px[..3] {
            *c = (*c).min(a);
        }
    }
}

#[cfg(test)]
mod tests {
    use jd_vector::*;

    use super::*;

    fn px(pm: &sk::Pixmap, x: u32, y: u32) -> [u8; 4] {
        let c = pm.pixel(x, y).unwrap().demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    }

    fn near(a: [u8; 4], b: [u8; 4], tol: i32) -> bool {
        a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= tol)
    }

    /// A 100x100 white frame at (0, 0) with whatever children `ops` add to key "f".
    fn frame_with(ops: Vec<Op>) -> (Document, NodeId) {
        let mut doc = Document::new();
        let p = doc.pages()[0];
        let mut all = vec![Op::create_as("f", p, NodeSpec::frame("F", 100.0, 100.0))];
        all.extend(ops);
        let done = doc.apply(&all, &ApproxMeasure).unwrap();
        (doc, done.id("f").unwrap())
    }

    #[test]
    fn fills_clip_and_scale() {
        let (doc, f) = frame_with(vec![
            Op::create("f", NodeSpec::rect("Red", 50.0, 50.0).at(25.0, 25.0).fill(Paint::hex("#FF0000"))),
            Op::create("f", NodeSpec::rect("Spill", 50.0, 50.0).at(80.0, 80.0).fill(Paint::hex("#0000FF"))),
        ]);
        let pm = render_node(&doc, f, RenderOptions::default()).unwrap();
        assert_eq!((pm.width(), pm.height()), (100, 100));
        assert_eq!(px(&pm, 50, 50), [255, 0, 0, 255]);
        assert_eq!(px(&pm, 5, 5), [255, 255, 255, 255]);
        assert_eq!(px(&pm, 90, 90), [0, 0, 255, 255]);
        let big = render_node(&doc, f, RenderOptions { scale: 2.0, ..RenderOptions::default() }).unwrap();
        assert_eq!((big.width(), big.height()), (200, 200));
        assert_eq!(px(&big, 100, 100), [255, 0, 0, 255]);
        // The blue square spills past the frame but the frame clips it.
        let page = render_node(&doc, doc.pages()[0], RenderOptions::default()).unwrap();
        assert_eq!((page.width(), page.height()), (100, 100));
    }

    #[test]
    fn opacity_and_blend_modes() {
        let (doc, f) = frame_with(vec![
            Op::create("f", NodeSpec::rect("Half", 50.0, 100.0).fill(Paint::hex("#000000")).with(|p| p.opacity = 0.5)),
            Op::create(
                "f",
                NodeSpec::rect("Multiply", 50.0, 100.0).at(50.0, 0.0).fill(Paint::hex("#FF8000")).with(|p| p.blend = BlendMode::Multiply),
            ),
        ]);
        let pm = render_node(&doc, f, RenderOptions::default()).unwrap();
        assert!(near(px(&pm, 25, 50), [128, 128, 128, 255], 1));
        assert_eq!(px(&pm, 75, 50), [255, 128, 0, 255], "multiply over white keeps the colour");
    }

    #[test]
    fn gradients_run_from_start_to_end() {
        let grad = Paint {
            kind: PaintKind::LinearGradient {
                from: [0.0, 0.5],
                to: [1.0, 0.5],
                stops: vec![
                    GradientStop { position: 0.0, color: Rgba::from_srgb8(0, 0, 0) },
                    GradientStop { position: 1.0, color: Rgba::from_srgb8(255, 255, 255) },
                ],
            },
            ..Paint::hex("#000000")
        };
        let (doc, f) = frame_with(vec![Op::create("f", NodeSpec::rect("G", 100.0, 100.0).fill(grad))]);
        let pm = render_node(&doc, f, RenderOptions::default()).unwrap();
        assert!(px(&pm, 2, 50)[0] < 10);
        assert!(px(&pm, 97, 50)[0] > 245);
        assert!(near(px(&pm, 50, 50), [128, 128, 128, 255], 3));
    }

    #[test]
    fn strokes_sit_inside_centred_or_outside() {
        let ring = |align| {
            NodeSpec::rect("R", 40.0, 40.0).at(30.0, 30.0).no_fill().stroke(Paint::hex("#000000"), 4.0).with(move |p| p.stroke.align = align)
        };
        for (align, inside, outside) in [(StrokeAlign::Inside, true, false), (StrokeAlign::Outside, false, true), (StrokeAlign::Center, true, true)]
        {
            let (doc, f) = frame_with(vec![Op::create("f", ring(align))]);
            let pm = render_node(&doc, f, RenderOptions::default()).unwrap();
            // Just inside and just outside the left edge at x = 30.
            assert_eq!(px(&pm, 31, 50)[0] < 50, inside, "{align:?} inside");
            assert_eq!(px(&pm, 28, 50)[0] < 50, outside, "{align:?} outside");
        }
    }

    #[test]
    fn drop_shadows_fall_below_and_blur_softens() {
        let shadow = Effect::DropShadow(Shadow { color: Rgba::from_srgb8(0, 0, 0), x: 0.0, y: 10.0, blur: 0.0, spread: 0.0, visible: true });
        let (doc, f) = frame_with(vec![Op::create(
            "f",
            NodeSpec::rect("Card", 40.0, 40.0).at(30.0, 20.0).fill(Paint::hex("#FFFFFF")).with(move |p| p.effects = vec![shadow]),
        )]);
        let pm = render_node(&doc, f, RenderOptions::default()).unwrap();
        assert_eq!(px(&pm, 50, 40), [255, 255, 255, 255], "the card itself");
        assert_eq!(px(&pm, 50, 65), [0, 0, 0, 255], "shadow below the card");
        assert_eq!(px(&pm, 50, 75), [255, 255, 255, 255], "shadow ends 10 px down");

        let soft = Effect::LayerBlur { radius: 8.0, visible: true };
        let (doc, f) = frame_with(vec![Op::create(
            "f",
            NodeSpec::rect("Blurred", 40.0, 40.0).at(30.0, 30.0).fill(Paint::hex("#000000")).with(move |p| p.effects = vec![soft]),
        )]);
        let pm = render_node(&doc, f, RenderOptions::default()).unwrap();
        let edge = px(&pm, 30, 50)[0];
        assert!(edge > 60 && edge < 200, "edge is soft: {edge}");
    }

    #[test]
    fn images_fill_their_box() {
        let mut doc = Document::new();
        let p = doc.pages()[0];
        // A 2x1 PNG: red then blue.
        let mut png = Vec::new();
        image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255])
            .unwrap()
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let key = doc.add_image(png);
        let img = Paint { kind: PaintKind::Image { image: key, fit: ImageFit::Stretch }, ..Paint::hex("#000000") };
        let done = doc.apply(&[Op::create(p, NodeSpec::rect("Photo", 100.0, 50.0).fill(img))], &ApproxMeasure).unwrap();
        let pm = render_node(&doc, done.created[0], RenderOptions::default()).unwrap();
        let (l, r) = (px(&pm, 10, 25), px(&pm, 90, 25));
        assert!(l[0] > 200 && l[2] < 60, "{l:?}");
        assert!(r[2] > 200 && r[0] < 60, "{r:?}");
    }

    #[test]
    fn text_draws_glyphs_and_png_encodes() {
        let style = TextStyle { family: String::new(), size: 40.0, ..TextStyle::default() };
        let mut doc = Document::new();
        let p = doc.pages()[0];
        let done = doc
            .apply(&[Op::create(p, NodeSpec::text("T", "Hello", style).fill(Paint::hex("#000000")))], &crate::CosmicMeasure)
            .unwrap();
        let t = done.created[0];
        let pm = render_node(&doc, t, RenderOptions { background: Some(Rgba::WHITE), ..RenderOptions::default() }).unwrap();
        let dark = pm.pixels().iter().filter(|c| c.demultiply().red() < 100).count();
        assert!(dark > 100, "glyphs were drawn ({dark} dark pixels)");
        let png = render_png(&doc, t, RenderOptions::default()).unwrap();
        assert_eq!(&png[..4], b"\x89PNG");
    }
}
