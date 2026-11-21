//! The document viewport: a GPU layer (`jd-gpu`) drawing the pixels, and a canvas overlay on
//! top handling input and drawing cursors, selections and transform handles.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use iced::keyboard;
use iced::mouse::{self, ScrollDelta};
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::shader::{self, Viewport};
use iced::widget::{Canvas, Shader, button, column, container, mouse_area, row, stack, text};
use iced::{Color, Element, Fill, Point, Rectangle, Renderer, Theme, Vector};
use jd_gpu::{Scene, SceneLayer, View, wgpu};
use jd_raster::selection::{Combine, Segment};
use jd_raster::transform::Affine;
use jd_ui::widget::bold;
use jd_ui::{Icon, icon, style, tokens::*};

use crate::app::{Message, Pen, Photo, Shape};
use crate::tool::Tool;
use crate::transform::{Handle, Kind};

/// Per-document cache of the marching-ants outline (in document pixels) and its drawn
/// geometry (in screen pixels). Outlines are expensive to trace, so they're rebuilt only when
/// the selection changes; the geometry only when the view or a transform preview changes.
#[derive(Default)]
pub struct Ants {
    outline: RefCell<Option<(u64, Arc<Vec<Segment>>)>>,
    key: Cell<Option<(u64, [u32; 9])>>,
    cache: canvas::Cache,
}

impl Ants {
    fn outline(&self, sel: &jd_raster::selection::Selection) -> Arc<Vec<Segment>> {
        let mut slot = self.outline.borrow_mut();
        match &*slot {
            Some((id, segs)) if *id == sel.id() => segs.clone(),
            _ => {
                let segs = Arc::new(sel.outline());
                *slot = Some((sel.id(), segs.clone()));
                segs
            }
        }
    }
}

pub fn view(app: &Photo) -> Element<'_, Message> {
    let Some((origin, zoom)) = app.doc_origin() else {
        return empty_state(app);
    };
    let open = app.doc().expect("doc_origin implies a document");
    let doc = &open.doc;

    let mut scene = Scene::from_document(doc);
    scene.retain = app
        .docs
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != app.active)
        .flat_map(|(_, d)| d.doc.layers().iter().flat_map(|l| l.pixels.iter().map(|(_, _, t)| t.id())))
        .collect();

    // Transform preview: the layer (or its lifted selection) is drawn through the affine.
    let session = app.transform.as_ref().filter(|t| t.doc == open.id);
    if let Some(t) = session
        && let Some(i) = doc.layers().iter().position(|l| l.id == t.layer)
    {
        let m = t.affine().as_array();
        let tiles = |g: &jd_raster::TileGrid| g.iter().map(|(x, y, t)| ([x, y], t.clone())).collect::<Vec<_>>();
        match &t.base {
            Some(base) => {
                scene.layers[i].tiles = tiles(base);
                let floating = SceneLayer { transform: m, tiles: tiles(&t.content), ..scene.layers[i].clone() };
                scene.layers.insert(i + 1, floating);
            }
            None => scene.layers[i].transform = m,
        }
    }

    // Adjustment / filter preview replaces the layer's pixels on screen only.
    if let Some(a) = app.adjusting.as_ref().filter(|a| a.doc == open.id)
        && let Some(preview) = a.shown()
        && let Some(i) = doc.layers().iter().position(|l| l.id == a.layer)
    {
        scene.layers[i].tiles = preview.iter().map(|(x, y, t)| ([x, y], t.clone())).collect();
    }

    let pixels = Shader::new(Pixels { scene: Arc::new(scene), origin, zoom }).width(Fill).height(Fill);

    let overlay = Canvas::new(Overlay {
        origin,
        zoom,
        bounds: app.canvas_bounds,
        tool: app.effective_tool(),
        brush_size: app.brush.size,
        pen: app.pen,
        pen_paints: app.pen_pressure,
        select_mode: app.select.mode,
        selection: doc.selection.as_ref(),
        ants: &open.ants,
        transform: session.map(|t| TransformView {
            kind: t.kind,
            corners: t.corners(),
            affine: t.affine(),
            grabbing: t.is_grabbing(),
        }),
        covered: app.popup.is_some()
            || app.adjusting.is_some()
            || app.new_doc.is_some()
            || app.confirm.is_some()
            || app.dialog.is_some()
            || app.palette.is_some(),
    })
    .width(Fill)
    .height(Fill);

    // A layer dragged from the Layers panel can be dropped on the canvas of another document.
    mouse_area(stack![pixels, overlay].width(Fill).height(Fill))
        .on_enter(Message::DragOver(crate::app::DropTarget::Canvas))
        .into()
}

fn empty_state(app: &Photo) -> Element<'_, Message> {
    let message = if app.loading > 0 { "Opening…" } else { "No image open" };
    let open = button(text("Open…").size(TEXT_SIZE).font(bold()))
        .padding([6, 16])
        .style(style::primary)
        .on_press(Message::OpenDialog);
    let new = button(text("New…").size(TEXT_SIZE))
        .padding([6, 16])
        .style(style::ghost(true))
        .on_press(Message::NewDocDialog);
    // Still report the viewport size so "fit" is right when the first image arrives.
    let sizer = Canvas::new(Sizer { bounds: app.canvas_bounds }).width(Fill).height(Fill);
    stack![
        sizer,
        container(
            column![
                icon(Icon::Image, 48.0, TEXT_MUTED),
                text(message).size(TEXT_SIZE).color(TEXT_SECONDARY),
                row![new, open].spacing(10),
                text("Ctrl+N new · Ctrl+O open").size(TEXT_SIZE_SMALL).color(TEXT_MUTED),
            ]
            .spacing(10)
            .align_x(iced::Alignment::Center),
        )
        .center(Fill)
        .style(style::fill(BG_CANVAS)),
    ]
    .into()
}

/// Reports the canvas bounds while no document is open.
struct Sizer {
    bounds: Rectangle,
}

impl canvas::Program<Message> for Sizer {
    type State = ();

    fn update(&self, _: &mut (), _: &canvas::Event, bounds: Rectangle, _: mouse::Cursor) -> Option<canvas::Action<Message>> {
        (bounds != self.bounds).then(|| canvas::Action::publish(Message::CanvasBounds(bounds)))
    }

    fn draw(&self, _: &(), _: &Renderer, _: &Theme, _: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        Vec::new()
    }
}

// ---- GPU layer --------------------------------------------------------------------------------

struct Pixels {
    scene: Arc<Scene>,
    origin: Point,
    zoom: f32,
}

impl shader::Program<Message> for Pixels {
    type State = ();
    type Primitive = PixelsPrimitive;

    fn draw(&self, _: &(), _: mouse::Cursor, _: Rectangle) -> PixelsPrimitive {
        PixelsPrimitive {
            scene: self.scene.clone(),
            origin: self.origin,
            zoom: self.zoom,
        }
    }
}

#[derive(Debug)]
struct PixelsPrimitive {
    scene: Arc<Scene>,
    origin: Point,
    zoom: f32,
}

struct Pipeline(jd_gpu::Renderer);

impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self(jd_gpu::Renderer::new(device, format))
    }
}

fn srgb(c: Color) -> [f32; 3] {
    [c.r, c.g, c.b]
}

impl shader::Primitive for PixelsPrimitive {
    type Pipeline = Pipeline;

    fn prepare(
        &self,
        pipeline: &mut Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        let s = viewport.scale_factor();
        let view = View {
            bounds: [bounds.x * s, bounds.y * s, bounds.width * s, bounds.height * s],
            origin: [self.origin.x * s, self.origin.y * s],
            zoom: self.zoom * s,
            background: srgb(BG_CANVAS),
            checker: [[1.0; 3], [0.8; 3]],
            checker_size: 8.0 * s,
        };
        pipeline.0.prepare(device, queue, &self.scene, &view);
    }

    fn render(
        &self,
        pipeline: &Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip: &Rectangle<u32>,
    ) {
        pipeline.0.render(encoder, target, [clip.x, clip.y, clip.width, clip.height]);
    }
}

// ---- input + overlay --------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct TransformView {
    kind: Kind,
    /// Box corners after the transform, document pixels (TL, TR, BR, BL).
    corners: [Point; 4],
    affine: Affine,
    grabbing: bool,
}

struct Overlay<'a> {
    origin: Point,
    zoom: f32,
    bounds: Rectangle,
    tool: Tool,
    brush_size: f32,
    /// Pen hovering or drawing (from the tablet), for the brush outline.
    pen: Option<Pen>,
    /// The pen paints with pressure through `Message::Tablet` (so ignore its pointer twin).
    pen_paints: bool,
    select_mode: Combine,
    selection: Option<&'a jd_raster::selection::Selection>,
    ants: &'a Ants,
    transform: Option<TransformView>,
    /// A dialog or menu is open over the canvas: show the normal arrow (the canvas would
    /// otherwise hide the cursor under it for brush tools) and no brush outline.
    covered: bool,
}

/// A marquee or lasso being dragged.
#[derive(Debug, Clone)]
struct SelectDrag {
    start: Point,
    current: Point,
    combine: Combine,
    lasso: Vec<(f32, f32)>,
}

/// A polygonal lasso being clicked out.
#[derive(Debug, Clone)]
struct PolyDrag {
    points: Vec<Point>,
    combine: Combine,
    /// Where the rubber-band line to the pointer ends (document px).
    hover: Point,
}

#[derive(Default)]
struct DragState {
    poly: Option<PolyDrag>,
    /// Time and place of the last left click, for double-click detection.
    last_click: Option<(std::time::Instant, Point)>,
    /// Last cursor position while panning.
    panning: Option<Point>,
    painting: bool,
    picking: bool,
    moving: bool,
    transforming: bool,
    select: Option<SelectDrag>,
    modifiers: keyboard::Modifiers,
}

/// Handle size in screen pixels.
const HANDLE: f32 = 8.0;

impl Overlay<'_> {
    fn to_doc(&self, p: Point) -> Point {
        Point::new((p.x - self.origin.x) / self.zoom, (p.y - self.origin.y) / self.zoom)
    }

    fn to_screen(&self, p: Point) -> Point {
        Point::new(self.origin.x + p.x * self.zoom, self.origin.y + p.y * self.zoom)
    }

    /// Selection mode for a new shape: Shift adds, Alt subtracts, both intersect; otherwise
    /// the options-bar mode.
    fn combine(&self, m: keyboard::Modifiers) -> Combine {
        match (m.shift(), m.alt()) {
            (true, true) => Combine::Intersect,
            (true, false) => Combine::Add,
            (false, true) => Combine::Subtract,
            _ => self.select_mode,
        }
    }

    /// Marquee box from drag start to current, with Shift = square and Alt = from centre
    /// (unless those keys chose the combine mode at the start of the drag).
    fn marquee_box(&self, d: &SelectDrag, m: keyboard::Modifiers) -> [f32; 4] {
        let square = m.shift() && d.combine != Combine::Add && d.combine != Combine::Intersect;
        let centred = m.alt() && d.combine != Combine::Subtract && d.combine != Combine::Intersect;
        let (mut dx, mut dy) = (d.current.x - d.start.x, d.current.y - d.start.y);
        if square {
            let s = dx.abs().max(dy.abs());
            dx = s.copysign(dx);
            dy = s.copysign(dy);
        }
        if centred {
            [d.start.x - dx, d.start.y - dy, d.start.x + dx, d.start.y + dy]
        } else {
            [d.start.x, d.start.y, d.start.x + dx, d.start.y + dy]
        }
    }

    fn hit_transform(&self, t: &TransformView, p: Point) -> Handle {
        let c: Vec<Point> = t.corners.iter().map(|&c| self.to_screen(c)).collect();
        let near = |a: Point, b: Point| (a.x - b.x).abs() <= HANDLE && (a.y - b.y).abs() <= HANDLE;
        for (i, &corner) in c.iter().enumerate() {
            if near(p, corner) {
                return Handle::Corner(i);
            }
        }
        for i in 0..4 {
            let mid = Point::new((c[i].x + c[(i + 1) % 4].x) / 2.0, (c[i].y + c[(i + 1) % 4].y) / 2.0);
            if near(p, mid) {
                return Handle::Edge(i);
            }
        }
        if inside_quad(&c, p) { Handle::Inside } else { Handle::Rotate }
    }
}

fn inside_quad(c: &[Point], p: Point) -> bool {
    let mut inside = false;
    for i in 0..c.len() {
        let (a, b) = (c[i], c[(i + 1) % c.len()]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}

impl canvas::Program<Message> for Overlay<'_> {
    type State = DragState;

    fn update(
        &self,
        state: &mut DragState,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if bounds != self.bounds {
            return Some(canvas::Action::publish(Message::CanvasBounds(bounds)));
        }
        if let canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) = event {
            state.modifiers = *m;
            return state.select.is_some().then(canvas::Action::request_redraw);
        }
        // Polygonal lasso: Enter closes, Esc cancels, Backspace removes the last point.
        if let (Some(poly), canvas::Event::Keyboard(keyboard::Event::KeyPressed { key, .. })) = (&mut state.poly, event) {
            use keyboard::key::Named;
            match key.as_ref() {
                keyboard::Key::Named(Named::Enter) => {
                    let poly = state.poly.take().expect("checked");
                    return Some(finish_polygon(poly).map_or_else(canvas::Action::request_redraw, |m| canvas::Action::publish(m)).and_capture());
                }
                keyboard::Key::Named(Named::Escape) => {
                    state.poly = None;
                    return Some(canvas::Action::request_redraw().and_capture());
                }
                keyboard::Key::Named(Named::Backspace | Named::Delete) => {
                    poly.points.pop();
                    if poly.points.is_empty() {
                        state.poly = None;
                    }
                    return Some(canvas::Action::request_redraw().and_capture());
                }
                _ => {}
            }
        }
        let local = cursor.position_in(bounds);
        let canvas::Event::Mouse(event) = event else { return None };
        let publish = |m| Some(canvas::Action::publish(m).and_capture());
        let m = state.modifiers;

        match event {
            mouse::Event::WheelScrolled { delta } => {
                let anchor = local?;
                let dy = match delta {
                    ScrollDelta::Lines { y, .. } | ScrollDelta::Pixels { y, .. } => *y,
                };
                if dy == 0.0 { None } else { publish(Message::ZoomAt { zoom_in: dy > 0.0, anchor }) }
            }
            mouse::Event::ButtonPressed(button) => {
                let p = local?;
                let left = *button == mouse::Button::Left;
                if *button == mouse::Button::Middle || (left && self.tool == Tool::Hand) {
                    state.panning = Some(p);
                    return Some(canvas::Action::request_redraw().and_capture());
                }
                if self.tool == Tool::Zoom && (left || *button == mouse::Button::Right) {
                    return publish(Message::ZoomAt { zoom_in: left, anchor: p });
                }
                if *button == mouse::Button::Right {
                    state.poly = None;
                    let at = cursor.position()?;
                    return publish(Message::OpenPopup(crate::app::PopupKind::Context, at));
                }
                if !left {
                    return None;
                }
                let doc = self.to_doc(p);
                // Free Transform handles take every click while open.
                if let Some(t) = &self.transform
                    && t.kind == Kind::Free
                {
                    state.transforming = true;
                    return publish(Message::TransformGrab(self.hit_transform(t, p), doc));
                }
                match self.tool {
                    Tool::Move => {
                        state.moving = true;
                        publish(Message::MoveBegin(doc))
                    }
                    Tool::PolygonalLasso => {
                        let now = std::time::Instant::now();
                        let double = state.last_click.is_some_and(|(t, at)| {
                            now.duration_since(t).as_millis() < 400 && (at.x - p.x).abs() < 5.0 && (at.y - p.y).abs() < 5.0
                        });
                        state.last_click = Some((now, p));
                        match &mut state.poly {
                            None => {
                                state.poly = Some(PolyDrag { points: vec![doc], combine: self.combine(m), hover: doc });
                                Some(canvas::Action::request_redraw().and_capture())
                            }
                            Some(poly) => {
                                let first = self.to_screen(poly.points[0]);
                                let near_start = (first.x - p.x).abs() <= 6.0 && (first.y - p.y).abs() <= 6.0;
                                if double || (near_start && poly.points.len() >= 3) {
                                    let poly = state.poly.take().expect("checked");
                                    return Some(finish_polygon(poly).map_or_else(canvas::Action::request_redraw, |m| canvas::Action::publish(m)).and_capture());
                                }
                                poly.points.push(doc);
                                Some(canvas::Action::request_redraw().and_capture())
                            }
                        }
                    }
                    Tool::Marquee | Tool::EllipticalMarquee | Tool::Lasso => {
                        state.select = Some(SelectDrag {
                            start: doc,
                            current: doc,
                            combine: self.combine(m),
                            lasso: vec![(doc.x, doc.y)],
                        });
                        Some(canvas::Action::request_redraw().and_capture())
                    }
                    Tool::Wand => publish(Message::Wand { at: doc, combine: self.combine(m) }),
                    // The pen paints through its own path (with pressure); its pointer twin mustn't.
                    _ if self.pen_paints && crate::tablet::pen_near() => None,
                    Tool::Eyedropper => {
                        state.picking = true;
                        publish(Message::PickColor(doc))
                    }
                    t if t.paints() && m.alt() => {
                        state.picking = true;
                        publish(Message::PickColor(doc))
                    }
                    t if t.paints() => {
                        state.painting = true;
                        publish(Message::StrokeBegin { at: doc, pressure: 1.0, erase: t == Tool::Eraser, line: m.shift() })
                    }
                    _ => None,
                }
            }
            mouse::Event::ButtonReleased(_) => {
                if state.panning.take().is_some() {
                    return Some(canvas::Action::request_redraw().and_capture());
                }
                state.picking = false;
                if std::mem::take(&mut state.painting) {
                    return publish(Message::StrokeEnd);
                }
                if std::mem::take(&mut state.moving) {
                    return publish(Message::MoveEnd);
                }
                if std::mem::take(&mut state.transforming) {
                    return publish(Message::TransformRelease);
                }
                if let Some(d) = state.select.take() {
                    let moved = (d.current.x - d.start.x).abs() * self.zoom > 2.0
                        || (d.current.y - d.start.y).abs() * self.zoom > 2.0;
                    if !moved {
                        // A plain click deselects, like Photoshop.
                        return (d.combine == Combine::Replace).then(|| canvas::Action::publish(Message::Deselect).and_capture());
                    }
                    let shape = if self.tool == Tool::Lasso {
                        Shape::Polygon(d.lasso)
                    } else if self.tool == Tool::EllipticalMarquee {
                        Shape::Ellipse(self.marquee_box(&d, m))
                    } else {
                        Shape::Rect(self.marquee_box(&d, m))
                    };
                    return publish(Message::SelectShape { shape, combine: d.combine });
                }
                None
            }
            mouse::Event::CursorMoved { .. } => {
                let p = cursor.position().map(|p| p - Vector::new(bounds.x, bounds.y));
                if let (Some(last), Some(p)) = (state.panning, p) {
                    state.panning = Some(p);
                    return publish(Message::PanBy(p - last));
                }
                let Some(p) = p else { return None };
                let doc = self.to_doc(p);
                if let Some(poly) = &mut state.poly {
                    poly.hover = doc;
                    return Some(canvas::Action::request_redraw());
                }
                if state.painting {
                    return publish(Message::StrokeTo { at: doc, pressure: 1.0 });
                }
                if state.moving {
                    return publish(Message::MoveTo(doc));
                }
                if state.transforming {
                    return publish(Message::TransformDrag { at: doc, shift: m.shift(), alt: m.alt() });
                }
                if let Some(d) = &mut state.select {
                    d.current = doc;
                    if let Some(&(lx, ly)) = d.lasso.last()
                        && ((doc.x - lx).abs() + (doc.y - ly).abs()) * self.zoom >= 2.0
                    {
                        d.lasso.push((doc.x, doc.y));
                    }
                    return Some(canvas::Action::request_redraw().and_capture());
                }
                if state.picking && local.is_some() {
                    return publish(Message::PickColor(doc));
                }
                Some(canvas::Action::publish(Message::CursorMoved(local.map(|p| self.to_doc(p)))))
            }
            mouse::Event::CursorLeft => Some(canvas::Action::publish(Message::CursorMoved(None))),
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &DragState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut layers = Vec::new();

        // Marching ants (cached; follows a transform preview).
        if let Some(sel) = self.selection {
            let m = self.transform.map_or(Affine::IDENTITY, |t| t.affine);
            let key = (
                sel.id(),
                [self.origin.x, self.origin.y, self.zoom, m.a, m.b, m.c, m.d, m.e, m.f].map(f32::to_bits),
            );
            if self.ants.key.get() != Some(key) {
                self.ants.cache.clear();
                self.ants.key.set(Some(key));
            }
            let segs = self.ants.outline(sel);
            layers.push(self.ants.cache.draw(renderer, bounds.size(), |frame| {
                let lines: Vec<(Point, Point)> = segs
                    .iter()
                    .map(|s| {
                        let (x0, y0) = m.apply(s[0] as f32, s[1] as f32);
                        let (x1, y1) = m.apply(s[2] as f32, s[3] as f32);
                        (self.to_screen(Point::new(x0, y0)), self.to_screen(Point::new(x1, y1)))
                    })
                    .collect();
                ants(frame, &lines);
            }));
        }

        let mut frame = Frame::new(renderer, bounds.size());

        // Marquee / lasso being dragged, polygonal lasso being clicked out.
        if let Some(d) = &state.select {
            let points: Vec<Point> = if self.tool == Tool::Lasso {
                d.lasso.iter().map(|&(x, y)| self.to_screen(Point::new(x, y))).collect()
            } else {
                let [x0, y0, x1, y1] = self.marquee_box(d, state.modifiers);
                let a = self.to_screen(Point::new(x0.min(x1), y0.min(y1)));
                let b = self.to_screen(Point::new(x0.max(x1), y0.max(y1)));
                if self.tool == Tool::EllipticalMarquee {
                    let (cx, cy, rx, ry) = ((a.x + b.x) / 2.0, (a.y + b.y) / 2.0, (b.x - a.x) / 2.0, (b.y - a.y) / 2.0);
                    (0..96)
                        .map(|i| {
                            let t = i as f32 / 96.0 * std::f32::consts::TAU;
                            Point::new(cx + rx * t.cos(), cy + ry * t.sin())
                        })
                        .collect()
                } else {
                    vec![a, Point::new(b.x, a.y), b, Point::new(a.x, b.y)]
                }
            };
            ants(&mut frame, &closed(&points));
        }
        if let Some(poly) = &state.poly {
            let mut pts: Vec<Point> = poly.points.iter().map(|&p| self.to_screen(p)).collect();
            pts.push(self.to_screen(poly.hover));
            let mut lines: Vec<(Point, Point)> = pts.windows(2).map(|w| (w[0], w[1])).collect();
            if pts.len() > 2 {
                // Faint closing edge so it's clear what Enter would select.
                let close = Path::line(pts[pts.len() - 1], pts[0]);
                frame.stroke(&close, Stroke::default().with_width(1.0).with_color(Color::from_rgba(1.0, 1.0, 1.0, 0.4)));
            }
            ants(&mut frame, &std::mem::take(&mut lines));
            let start = Path::circle(pts[0], 4.0);
            frame.stroke(&start, Stroke::default().with_width(1.5).with_color(ACCENT_PRIMARY));
        }

        // Free Transform box and handles.
        if let Some(t) = &self.transform
            && t.kind == Kind::Free
        {
            let c: Vec<Point> = t.corners.iter().map(|&c| self.to_screen(c)).collect();
            let outline = Path::new(|b| {
                b.move_to(c[0]);
                for &p in &c[1..] {
                    b.line_to(p);
                }
                b.close();
            });
            frame.stroke(&outline, Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
            let mut handles = c.clone();
            for i in 0..4 {
                handles.push(Point::new((c[i].x + c[(i + 1) % 4].x) / 2.0, (c[i].y + c[(i + 1) % 4].y) / 2.0));
            }
            for h in handles {
                let r = Path::rectangle(h - Vector::new(HANDLE / 2.0, HANDLE / 2.0), iced::Size::new(HANDLE, HANDLE));
                frame.fill(&r, Color::WHITE);
                frame.stroke(&r, Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
            }
        }

        // Brush outline.
        let at = match self.pen {
            Some(pen) => Some(pen.position),
            None => cursor.position_in(bounds),
        };
        if self.tool.uses_brush()
            && self.transform.is_none()
            && !self.covered
            && let Some(p) = at
        {
            let r = (self.brush_size * self.zoom / 2.0).max(1.5);
            let circle = Path::circle(p, r);
            frame.stroke(&circle, Stroke::default().with_width(2.0).with_color(Color::from_rgba(0.0, 0.0, 0.0, 0.5)));
            frame.stroke(&circle, Stroke::default().with_width(1.0).with_color(Color::WHITE));
            if r < 4.0 {
                let c = Path::new(|b| {
                    b.move_to(p - Vector::new(5.0, 0.0));
                    b.line_to(p + Vector::new(5.0, 0.0));
                    b.move_to(p - Vector::new(0.0, 5.0));
                    b.line_to(p + Vector::new(0.0, 5.0));
                });
                frame.stroke(&c, Stroke::default().with_width(1.0).with_color(Color::WHITE));
            }
        }
        layers.push(frame.into_geometry());
        layers
    }

    fn mouse_interaction(&self, state: &DragState, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if state.panning.is_some() {
            return mouse::Interaction::Grabbing;
        }
        if self.covered {
            // Idle (not None) so the stack stops here and shows the arrow.
            return mouse::Interaction::Idle;
        }
        let Some(p) = cursor.position_in(bounds) else { return mouse::Interaction::default() };
        if let Some(t) = &self.transform
            && t.kind == Kind::Free
        {
            if t.grabbing {
                return mouse::Interaction::Grabbing;
            }
            return match self.hit_transform(t, p) {
                Handle::Inside => mouse::Interaction::Move,
                Handle::Corner(i) if i % 2 == 0 => mouse::Interaction::ResizingDiagonallyDown,
                Handle::Corner(_) => mouse::Interaction::ResizingDiagonallyUp,
                Handle::Edge(i) if i % 2 == 0 => mouse::Interaction::ResizingVertically,
                Handle::Edge(_) => mouse::Interaction::ResizingHorizontally,
                Handle::Rotate => mouse::Interaction::Grab,
            };
        }
        match self.tool {
            t if t.uses_brush() => mouse::Interaction::Hidden,
            Tool::Hand => mouse::Interaction::Grab,
            Tool::Zoom => mouse::Interaction::ZoomIn,
            Tool::Type => mouse::Interaction::Text,
            Tool::Move => mouse::Interaction::Move,
            _ => mouse::Interaction::Crosshair,
        }
    }
}

/// Edges of a closed polygon.
fn closed(points: &[Point]) -> Vec<(Point, Point)> {
    (0..points.len()).map(|i| (points[i], points[(i + 1) % points.len()])).collect()
}

/// A finished polygonal lasso becomes a selection (needs at least 3 points).
fn finish_polygon(poly: PolyDrag) -> Option<Message> {
    (poly.points.len() >= 3).then(|| Message::SelectShape {
        shape: Shape::Polygon(poly.points.iter().map(|p| (p.x, p.y)).collect()),
        combine: poly.combine,
    })
}

/// Black-and-white "marching ants": a white line with black dashes on top, visible on any
/// image. Dashes are cut here rather than with the renderer's dash option, which joins up
/// separate outline pieces (it drew stray lines across selections). The dash phase comes
/// from screen position, so neighbouring segments continue the same pattern.
fn ants(frame: &mut Frame, lines: &[(Point, Point)]) {
    const ON: f32 = 4.0;
    const PERIOD: f32 = 8.0;
    let solid = Path::new(|b| {
        for &(a, c) in lines {
            b.move_to(a);
            b.line_to(c);
        }
    });
    frame.stroke(&solid, Stroke::default().with_width(1.0).with_color(Color::WHITE));
    let dashes = Path::new(|b| {
        for &(a, c) in lines {
            let (dx, dy) = (c.x - a.x, c.y - a.y);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 0.01 {
                continue;
            }
            let (ux, uy) = (dx / len, dy / len);
            let mut t = -((a.x + a.y).rem_euclid(PERIOD));
            while t < len {
                let (s0, s1) = (t.max(0.0), (t + ON).min(len));
                if s1 > s0 {
                    b.move_to(Point::new(a.x + ux * s0, a.y + uy * s0));
                    b.line_to(Point::new(a.x + ux * s1, a.y + uy * s1));
                }
                t += PERIOD;
            }
        }
    });
    frame.stroke(&dashes, Stroke::default().with_width(1.0).with_color(Color::BLACK));
}
