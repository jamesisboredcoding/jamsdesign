//! The canvas: the rendered page (an image from `jd-vector-render`) with an overlay on top
//! that takes the mouse and draws selection, handles, hover, marquee and frame names.

use iced::mouse::{self, Cursor, ScrollDelta};
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text};
use iced::widget::{container, image, stack};
use iced::{Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme};
use jd_ui::tokens::*;

use crate::app::{Blobr, Drag, Handle, Message};
use crate::tool::Tool;

pub fn view(app: &Blobr) -> Element<'_, Message> {
    // The last picture, moved and scaled to where the camera is now (a newer one is on its
    // way if they differ), so panning and zooming follow the pointer at once. Drawn on a
    // canvas, not placed with `pin`: `pin` shrinks its child to the room left after the
    // offset, which squashed the picture while panning right or zooming in.
    let picture = canvas::Canvas::new(Picture { app }).width(Fill).height(Fill);
    let mut layers = stack![picture, canvas::Canvas::new(Overlay { app }).width(Fill).height(Fill)];
    if let Some(editor) = text_editor_overlay(app) {
        layers = layers.push(editor);
    }
    container(layers)
        .width(Fill)
        .height(Fill)
        .clip(true)
        .style(jd_ui::style::fill(BG_CANVAS))
        .into()
}

/// The box typed into when editing a text layer, over the layer, at its zoomed size.
fn text_editor_overlay(app: &Blobr) -> Option<Element<'_, Message>> {
    use iced::widget::text_editor::{self, Binding, KeyPress};
    let (id, content) = app.text_edit.as_ref()?;
    let r = app.screen_bounds(*id)?;
    let size = match app.doc().node(*id).map(|n| &n.kind) {
        Some(jd_vector::NodeKind::Text { text }) => text.style.size * app.camera.zoom as f32,
        _ => 16.0,
    };
    let editor = iced::widget::text_editor(content)
        .id(crate::app::TEXT_EDITOR)
        .on_action(Message::TextAction)
        .size(size.clamp(8.0, 200.0))
        .padding(2)
        .width(r.width.max(120.0) + 24.0)
        // Escape finishes; everything else edits as usual.
        .key_binding(|kp: KeyPress| match kp.key.as_ref() {
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) => Some(Binding::Custom(Message::EndTextEdit)),
            _ => Binding::from_key_press(kp),
        })
        .style(|_, _| text_editor::Style {
            background: BG_PANEL.into(),
            border: iced::Border { radius: 2.0.into(), width: 1.0, color: ACCENT_PRIMARY },
            placeholder: TEXT_MUTED,
            value: TEXT_PRIMARY,
            selection: BG_SELECTED,
        });
    Some(container(editor).padding(iced::Padding { top: (r.y - 3.0).max(0.0), left: (r.x - 3.0).max(0.0), ..iced::Padding::ZERO }).into())
}

/// The rendered page, where the camera puts it now.
struct Picture<'a> {
    app: &'a Blobr,
}

impl canvas::Program<Message> for Picture<'_> {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: Cursor) -> Vec<Geometry> {
        let app = self.app;
        let mut frame = Frame::new(renderer, bounds.size());
        // The page's colour under it, so the edges a zoom-out or pan uncovers before the
        // next render lands match the page instead of flashing the darker canvas behind.
        let [r, g, b] = app.page_background().to_srgb();
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), Color::from_rgb(r, g, b));
        if let Some(a) = &app.canvas {
            frame.draw_image(picture_bounds(app), canvas::Image::new(a.handle().clone()).filter_method(image::FilterMethod::Linear));
        }
        vec![frame.into_geometry()]
    }
}

/// Where the last picture goes on screen: rendered for `canvas_camera`, shown for `camera`.
pub(crate) fn picture_bounds(app: &Blobr) -> Rectangle {
    let (now, then) = (app.camera, app.canvas_camera);
    let z = (now.zoom / then.zoom) as f32;
    Rectangle {
        x: ((then.origin.x - now.origin.x) * now.zoom) as f32,
        y: ((then.origin.y - now.origin.y) * now.zoom) as f32,
        width: app.canvas_size.width * z,
        height: app.canvas_size.height * z,
    }
}

struct Overlay<'a> {
    app: &'a Blobr,
}

impl canvas::Program<Message> for Overlay<'_> {
    type State = ();

    fn update(&self, _: &mut (), event: &canvas::Event, bounds: Rectangle, cursor: Cursor) -> Option<canvas::Action<Message>> {
        let app = self.app;
        if (bounds.width - app.viewport.width).abs() > 0.5 || (bounds.height - app.viewport.height).abs() > 0.5 {
            return Some(canvas::Action::publish(Message::Viewport(bounds.size())));
        }
        let canvas::Event::Mouse(event) = event else { return None };
        let publish = |m| Some(canvas::Action::publish(m).and_capture());
        let local = |p: Point| Point::new(p.x - bounds.x, p.y - bounds.y);
        match event {
            mouse::Event::ButtonPressed(button) => {
                let p = cursor.position_in(bounds)?;
                publish(Message::CanvasPress { at: p, button: *button })
            }
            mouse::Event::CursorMoved { position } => {
                // While dragging, follow the pointer even outside the canvas.
                if app.drag.is_some() || bounds.contains(*position) {
                    publish(Message::CanvasMove(local(*position)))
                } else {
                    None
                }
            }
            mouse::Event::ButtonReleased(_) if app.drag.is_some() => {
                let p = cursor.position().map_or(Point::ORIGIN, local);
                publish(Message::CanvasRelease(p))
            }
            mouse::Event::WheelScrolled { delta } => {
                let p = cursor.position_in(bounds)?;
                let d = match *delta {
                    ScrollDelta::Lines { x, y } => iced::Vector::new(x * 60.0, y * 60.0),
                    ScrollDelta::Pixels { x, y } => iced::Vector::new(x, y),
                };
                publish(Message::CanvasScroll { delta: d, at: p })
            }
            _ => None,
        }
    }

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: Cursor) -> Vec<Geometry> {
        let app = self.app;
        let doc = app.doc();
        let mut frame = Frame::new(renderer, bounds.size());
        pixel_grid(&mut frame, app, bounds.size());

        // Names above top-level frames, like Figma.
        for &c in doc.children(app.page) {
            let Some(n) = doc.node(c) else { continue };
            if !(n.kind.is_frame_like() || matches!(n.kind, jd_vector::NodeKind::Section | jd_vector::NodeKind::ComponentSet { .. })) {
                continue;
            }
            let Some(r) = app.screen_bounds(c) else { continue };
            let selected = app.selection.contains(&c);
            frame.fill_text(Text {
                content: n.name.clone(),
                position: Point::new(r.x, r.y - 16.0),
                color: if selected { ACCENT_PRIMARY } else { TEXT_SECONDARY },
                size: 11.0.into(),
                ..Text::default()
            });
        }

        let outline = |frame: &mut Frame, id| {
            if let Some(c) = app.screen_corners(id) {
                let p = Path::new(|b| {
                    b.move_to(c[0]);
                    for q in &c[1..] {
                        b.line_to(*q);
                    }
                    b.close();
                });
                frame.stroke(&p, Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
            }
        };
        if let Some(h) = app.hover.filter(|h| !app.selection.contains(h)) {
            outline(&mut frame, h);
        }
        for &id in &app.selection {
            outline(&mut frame, id);
        }
        if let [one] = app.selection[..]
            && app.vector_edit.is_none()
            && let Some(r) = app.screen_bounds(one)
        {
            for (_, c) in app.handle_points(one) {
                let sq = Path::rectangle(Point::new(c.x - 3.5, c.y - 3.5), Size::new(7.0, 7.0));
                frame.fill(&sq, Color::WHITE);
                frame.stroke(&sq, Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
            }
            if let Some(n) = doc.node(one) {
                let label = format!("{:.0} × {:.0}", n.props.width, n.props.height);
                let w = label.chars().count() as f32 * 6.0 + 10.0;
                let at = Point::new(r.x + r.width / 2.0 - w / 2.0, r.y + r.height + 8.0);
                frame.fill(&Path::rounded_rectangle(at, Size::new(w, 16.0), 3.0.into()), ACCENT_PRIMARY);
                frame.fill_text(Text {
                    content: label,
                    position: Point::new(at.x + 5.0, at.y + 2.0),
                    color: Color::WHITE,
                    size: 10.0.into(),
                    ..Text::default()
                });
            }
        }

        // Pen: the path so far, its points, and the segment following the pointer.
        if let Some(pen) = &app.pen {
            let to_screen = |p: [f64; 2]| app.camera.to_screen(kurbo::Point::new(p[0], p[1]));
            let mut anchors = pen.anchors.clone();
            if !pen.dragging
                && let Some(c) = pen.cursor
            {
                anchors.push(jd_vector::Anchor::corner(c.x, c.y));
            }
            let bez = crate::pen::bez(&anchors, false);
            let screen = kurbo::Affine::scale(app.camera.zoom) * kurbo::Affine::translate(-app.camera.origin.to_vec2()) * bez;
            frame.stroke(&to_iced(&screen), Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
            for (i, a) in pen.anchors.iter().enumerate() {
                point(&mut frame, to_screen(a.p), i == 0 && pen.anchors.len() >= 2);
            }
            if let Some(last) = pen.anchors.last() {
                handles(&mut frame, to_screen(last.p), last.handle_in.map(to_screen), last.handle_out.map(to_screen));
            }
        }

        // Editing a vector's points: the path, every point, and the selected point's handles.
        if let Some(ve) = app.vector_edit
            && let Some(jd_vector::NodeKind::Vector { path }) = doc.node(ve.id).map(|n| &n.kind)
            && let Some(map) = app.anchors_to_screen(ve.id)
        {
            let n = doc.node(ve.id).expect("checked");
            let local = path.to_bez(n.props.width, n.props.height);
            frame.stroke(&to_iced(&(app.node_to_screen(ve.id) * local)), Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
            let to_screen = |p: [f64; 2]| {
                let q = map * kurbo::Point::new(p[0], p[1]);
                Point::new(q.x as f32, q.y as f32)
            };
            for (s, sp) in path.subpaths.iter().enumerate() {
                for (i, a) in sp.anchors.iter().enumerate() {
                    let selected = ve.selected == Some((s, i));
                    if selected {
                        handles(&mut frame, to_screen(a.p), a.handle_in.map(to_screen), a.handle_out.map(to_screen));
                    }
                    point(&mut frame, to_screen(a.p), selected);
                }
            }
        }

        // Prototype connections: a curve from each layer with a link to its destination frame.
        if app.right_tab == 1 {
            let blue = Color::from_rgb8(0x4C, 0x8D, 0xFF);
            for id in doc.descendants(app.page) {
                let Some(n) = doc.node(id) else { continue };
                for r in &n.props.reactions {
                    let (jd_vector::Action::Navigate { to } | jd_vector::Action::Overlay { to }) = r.action else { continue };
                    let (Some(a), Some(b)) = (app.screen_bounds(id), app.screen_bounds(to)) else { continue };
                    let selected = app.selection.contains(&id);
                    let from = Point::new(a.x + a.width, a.y + a.height / 2.0);
                    let end = if b.x >= from.x { Point::new(b.x, b.y + b.height.min(60.0) / 2.0) } else { Point::new(b.x + b.width, b.y + b.height.min(60.0) / 2.0) };
                    let bend = ((end.x - from.x).abs() / 2.0).max(40.0);
                    let c2x = if end.x >= from.x { end.x - bend } else { end.x + bend };
                    let curve = Path::new(|p| {
                        p.move_to(from);
                        p.bezier_curve_to(Point::new(from.x + bend, from.y), Point::new(c2x, end.y), end);
                    });
                    let color = if selected { blue } else { Color { a: 0.6, ..blue } };
                    frame.stroke(&curve, Stroke::default().with_width(if selected { 2.0 } else { 1.5 }).with_color(color));
                    frame.fill(&Path::circle(from, 3.5), color);
                    let dir = if end.x >= c2x { 1.0 } else { -1.0 };
                    let head = Path::new(|p| {
                        p.move_to(end);
                        p.line_to(Point::new(end.x - 7.0 * dir, end.y - 4.0));
                        p.line_to(Point::new(end.x - 7.0 * dir, end.y + 4.0));
                        p.close();
                    });
                    frame.fill(&head, color);
                }
            }
        }

        // Smart guides while a drag snaps, and Alt-hover distances, in Figma's red.
        let red = Color::from_rgb8(0xF2, 0x48, 0x22);
        for g in &app.guides {
            let line = Path::line(app.camera.to_screen(g.from), app.camera.to_screen(g.to));
            frame.stroke(&line, Stroke::default().with_width(1.0).with_color(red));
        }
        if (app.modifiers.alt() || app.right_tab == 2)
            && app.drag.is_none()
            && let Some(a) = app.selection_bounds()
        {
            let other = app.hover.filter(|h| !app.selection.contains(h)).or_else(|| {
                let s = *app.selection.first()?;
                doc.parent(s).filter(|&p| p != app.page)
            });
            if let Some(b) = other.and_then(|o| doc.world_bounds(o)) {
                for (l, d) in jd_vector::snap::measure(a, b) {
                    let (p, q) = (app.camera.to_screen(l.from), app.camera.to_screen(l.to));
                    frame.stroke(&Path::line(p, q), Stroke::default().with_width(1.0).with_color(red));
                    let label = format!("{}", (d * 100.0).round() / 100.0);
                    let w = label.chars().count() as f32 * 6.0 + 8.0;
                    let at = Point::new((p.x + q.x) / 2.0 - w / 2.0, (p.y + q.y) / 2.0 - 8.0);
                    frame.fill(&Path::rounded_rectangle(at, Size::new(w, 16.0), 3.0.into()), red);
                    frame.fill_text(Text { content: label, position: Point::new(at.x + 4.0, at.y + 2.0), color: Color::WHITE, size: 10.0.into(), ..Text::default() });
                }
                let r = Rectangle::new(app.camera.to_screen(kurbo::Point::new(b.x0, b.y0)), Size::new((b.width() * app.camera.zoom) as f32, (b.height() * app.camera.zoom) as f32));
                frame.stroke(&Path::rectangle(r.position(), r.size()), Stroke::default().with_width(1.0).with_color(red));
            }
        }

        match &app.drag {
            Some(Drag::Marquee { start, now }) => {
                let (a, b) = (app.camera.to_screen(*start), app.camera.to_screen(*now));
                let r = Rectangle::new(Point::new(a.x.min(b.x), a.y.min(b.y)), Size::new((a.x - b.x).abs(), (a.y - b.y).abs()));
                let p = Path::rectangle(r.position(), r.size());
                frame.fill(&p, Color { a: 0.1, ..ACCENT_PRIMARY });
                frame.stroke(&p, Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
            }
            Some(Drag::Create { tool: Tool::Line, start, now }) => {
                let (a, b) = (app.camera.to_screen(*start), app.camera.to_screen(app.line_end(*start, *now)));
                frame.stroke(&Path::line(a, b), Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
            }
            Some(Drag::Create { tool, start, now }) => {
                let (a, b) = (app.camera.to_screen(*start), app.camera.to_screen(*now));
                let r = Rectangle::new(Point::new(a.x.min(b.x), a.y.min(b.y)), Size::new((a.x - b.x).abs(), (a.y - b.y).abs()));
                let p = if *tool == Tool::Ellipse {
                    Path::new(|b| {
                        b.ellipse(iced::widget::canvas::path::arc::Elliptical {
                            center: r.center(),
                            radii: iced::Vector::new(r.width / 2.0, r.height / 2.0),
                            rotation: iced::Radians(0.0),
                            start_angle: iced::Radians(0.0),
                            end_angle: iced::Radians(std::f32::consts::TAU),
                        })
                    })
                } else {
                    Path::rectangle(r.position(), r.size())
                };
                frame.stroke(&p, Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
            }
            _ => {}
        }
        // Comment pins, numbered; the open one (or the one being written) in the accent.
        let pin = |frame: &mut Frame, at: Point, label: String, on: bool, resolved: bool| {
            let fill = if on { ACCENT_PRIMARY } else if resolved { Color::from_rgb8(0x6B, 0x6B, 0x6B) } else { Color::from_rgb8(0xE0, 0x3E, 0x7A) };
            let c = Path::circle(at, crate::app::PIN_RADIUS);
            frame.fill(&c, fill);
            frame.stroke(&c, Stroke::default().with_width(2.0).with_color(Color::WHITE));
            let w = label.chars().count() as f32 * 6.5;
            frame.fill_text(Text { content: label, position: Point::new(at.x - w / 2.0, at.y - 7.0), color: Color::WHITE, size: 11.0.into(), ..Text::default() });
        };
        for (id, n, at) in app.visible_comments() {
            let resolved = doc.comments().get(&id).is_some_and(|c| c.resolved);
            pin(&mut frame, at, n.to_string(), app.comment_open == Some(id), resolved);
        }
        if let Some(p) = app.comment_new {
            pin(&mut frame, app.camera.to_screen(p), "+".into(), true, false);
        }
        rulers(&mut frame, app, bounds.size());
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, _: &(), bounds: Rectangle, cursor: Cursor) -> mouse::Interaction {
        let app = self.app;
        match app.drag {
            Some(Drag::Pan { .. }) => return mouse::Interaction::Grabbing,
            Some(Drag::Create { .. }) => return mouse::Interaction::Crosshair,
            _ => {}
        }
        if matches!(app.drag, Some(Drag::Rotate { .. })) {
            return mouse::Interaction::Grabbing;
        }
        let Some(p) = cursor.position_in(bounds) else { return mouse::Interaction::default() };
        if app.space_held || app.tool == Tool::Hand {
            return mouse::Interaction::Grab;
        }
        if app.comment_at(p).is_some() {
            return mouse::Interaction::Pointer;
        }
        if app.tool == Tool::Zoom {
            return if app.modifiers.alt() { mouse::Interaction::ZoomOut } else { mouse::Interaction::ZoomIn };
        }
        if app.tool.creates() || app.tool == Tool::Pen || app.tool == Tool::Comment {
            return mouse::Interaction::Crosshair;
        }
        if app.handle_at(p).is_none() && app.rotate_at(p).is_some() {
            return mouse::Interaction::Grab;
        }
        match app.handle_at(p).map(|(_, h)| h) {
            Some(Handle::Left | Handle::Right) => mouse::Interaction::ResizingHorizontally,
            Some(Handle::Top | Handle::Bottom) => mouse::Interaction::ResizingVertically,
            Some(Handle::TopLeft | Handle::BottomRight) => mouse::Interaction::ResizingDiagonallyDown,
            Some(Handle::TopRight | Handle::BottomLeft) => mouse::Interaction::ResizingDiagonallyUp,
            None => mouse::Interaction::default(),
        }
    }
}

/// A kurbo path (screen coordinates) as an iced one.
fn to_iced(bez: &kurbo::BezPath) -> Path {
    let p = |q: kurbo::Point| Point::new(q.x as f32, q.y as f32);
    Path::new(|b| {
        for el in bez.elements() {
            match *el {
                kurbo::PathEl::MoveTo(a) => b.move_to(p(a)),
                kurbo::PathEl::LineTo(a) => b.line_to(p(a)),
                kurbo::PathEl::QuadTo(c, a) => b.quadratic_curve_to(p(c), p(a)),
                kurbo::PathEl::CurveTo(c1, c2, a) => b.bezier_curve_to(p(c1), p(c2), p(a)),
                kurbo::PathEl::ClosePath => b.close(),
            }
        }
    })
}

/// A vector point: white square, blue when selected.
fn point(frame: &mut Frame, at: Point, selected: bool) {
    let sq = Path::rectangle(Point::new(at.x - 3.5, at.y - 3.5), Size::new(7.0, 7.0));
    frame.fill(&sq, if selected { ACCENT_PRIMARY } else { Color::WHITE });
    frame.stroke(&sq, Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
}

/// A point's bezier handles: lines out to small circles.
fn handles(frame: &mut Frame, at: Point, hin: Option<Point>, hout: Option<Point>) {
    for h in [hin, hout].into_iter().flatten() {
        frame.stroke(&Path::line(at, h), Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
        let c = Path::circle(h, 3.5);
        frame.fill(&c, Color::WHITE);
        frame.stroke(&c, Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
    }
}

/// Zoom from which the pixel grid shows.
pub const PIXEL_GRID_ZOOM: f64 = 4.0;
/// How thick the rulers are.
pub const RULER: f32 = 18.0;

/// A line between page pixels, when zoomed in far enough to see them.
fn pixel_grid(frame: &mut Frame, app: &Blobr, size: Size) {
    let (z, o) = (app.camera.zoom, app.camera.origin);
    if !app.prefs.pixel_grid || z < PIXEL_GRID_ZOOM {
        return;
    }
    let lines = Path::new(|b| {
        let mut x = o.x.floor();
        while (x - o.x) * z <= size.width as f64 {
            let sx = ((x - o.x) * z) as f32;
            b.move_to(Point::new(sx, 0.0));
            b.line_to(Point::new(sx, size.height));
            x += 1.0;
        }
        let mut y = o.y.floor();
        while (y - o.y) * z <= size.height as f64 {
            let sy = ((y - o.y) * z) as f32;
            b.move_to(Point::new(0.0, sy));
            b.line_to(Point::new(size.width, sy));
            y += 1.0;
        }
    });
    frame.stroke(&lines, Stroke::default().with_width(1.0).with_color(Color::from_rgba(0.5, 0.5, 0.5, 0.25)));
}

/// The ruler step (page pixels between numbers): the smallest "nice" one at least 60 screen
/// pixels apart.
pub fn ruler_step(zoom: f64) -> f64 {
    let mut step = 1.0;
    while step * zoom < 60.0 {
        step *= if (step / 10f64.powf(step.log10().floor()) - 2.0).abs() < 1e-9 { 2.5 } else { 2.0 };
    }
    step
}

/// Rulers along the top and left, in page pixels, with the selection's extent marked.
fn rulers(frame: &mut Frame, app: &Blobr, size: Size) {
    if !app.prefs.rulers {
        return;
    }
    let (z, o) = (app.camera.zoom, app.camera.origin);
    frame.fill_rectangle(Point::ORIGIN, Size::new(size.width, RULER), BG_PANEL);
    frame.fill_rectangle(Point::ORIGIN, Size::new(RULER, size.height), BG_PANEL);
    // The selection's extent, in the accent.
    let sel = app.selection.iter().filter_map(|&id| app.screen_bounds(id)).reduce(|a, b| a.union(&b));
    if let Some(r) = sel {
        let band = Color { a: 0.3, ..ACCENT_PRIMARY };
        frame.fill_rectangle(Point::new(r.x.max(RULER), 0.0), Size::new((r.x + r.width).min(size.width) - r.x.max(RULER), RULER), band);
        frame.fill_rectangle(Point::new(0.0, r.y.max(RULER)), Size::new(RULER, (r.y + r.height).min(size.height) - r.y.max(RULER)), band);
    }
    let step = ruler_step(z);
    let mantissa = (step / 10f64.powf(step.log10().floor())).round();
    let minor = step / if mantissa == 2.0 { 4.0 } else if mantissa == 5.0 { 5.0 } else { 10.0 };
    let label = |v: f64| if step < 1.0 { format!("{v:.1}") } else { format!("{v:.0}") };
    let mut ticks = Vec::new();
    // Along the top.
    let mut k = (o.x / minor).floor();
    loop {
        let x = k * minor;
        let sx = ((x - o.x) * z) as f32;
        if sx > size.width {
            break;
        }
        let major = (x / step - (x / step).round()).abs() < 1e-6;
        if sx >= RULER {
            ticks.push((Point::new(sx, if major { 0.0 } else { RULER - 5.0 }), Point::new(sx, RULER)));
            if major {
                frame.fill_text(Text { content: label(x), position: Point::new(sx + 3.0, 1.0), color: TEXT_SECONDARY, size: 9.0.into(), ..Text::default() });
            }
        }
        k += 1.0;
    }
    // Down the left, numbers written one digit under another.
    let mut k = (o.y / minor).floor();
    loop {
        let y = k * minor;
        let sy = ((y - o.y) * z) as f32;
        if sy > size.height {
            break;
        }
        let major = (y / step - (y / step).round()).abs() < 1e-6;
        if sy >= RULER {
            ticks.push((Point::new(if major { 0.0 } else { RULER - 5.0 }, sy), Point::new(RULER, sy)));
            if major {
                for (i, ch) in label(y).chars().enumerate() {
                    let at = Point::new(3.0, sy + 2.0 + i as f32 * 9.0);
                    frame.fill_text(Text { content: ch.to_string(), position: at, color: TEXT_SECONDARY, size: 9.0.into(), ..Text::default() });
                }
            }
        }
        k += 1.0;
    }
    let lines = Path::new(|b| {
        for (a, c) in &ticks {
            b.move_to(*a);
            b.line_to(*c);
        }
        // Their inner edges.
        b.move_to(Point::new(RULER, RULER));
        b.line_to(Point::new(size.width, RULER));
        b.move_to(Point::new(RULER, RULER));
        b.line_to(Point::new(RULER, size.height));
    });
    frame.stroke(&lines, Stroke::default().with_width(1.0).with_color(BORDER_DEFAULT));
    frame.fill_rectangle(Point::ORIGIN, Size::new(RULER, RULER), BG_PANEL);
}

#[cfg(test)]
mod tests {
    #[test]
    fn ruler_steps_are_round_numbers_far_enough_apart() {
        for (zoom, step) in [(1.0, 100.0), (0.5, 200.0), (0.1, 1000.0), (2.0, 50.0), (8.0, 10.0), (64.0, 1.0)] {
            assert_eq!(super::ruler_step(zoom), step, "zoom {zoom}");
        }
    }
}
