//! The adjustment / filter dialog: floats over the top-right of the canvas so the live preview
//! stays visible. Everything else is blocked while it's open (zooming still works).

use iced::alignment::Vertical;
use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::{Canvas, button, column, container, mouse_area, pin, row, slider, space, stack, text};
use iced::{Color, Element, Fill, Padding, Point, Rectangle, Renderer, Size, Theme};
use jd_raster::adjust::{Adjustment, curve_table};
use jd_raster::filter::Filter;
use jd_ui::widget::{bold, label};
use jd_ui::{style, tokens::*};

use crate::adjust::Op;
use crate::app::{Message, Photo};

const WIDTH: f32 = 320.0;

pub fn view(app: &Photo) -> Option<Element<'_, Message>> {
    let s = app.adjusting.as_ref()?;
    let set = |op: Op| Message::AdjustSet(op);
    let controls: Element<'_, Message> = match s.op.clone() {
        Op::Adjust(Adjustment::BrightnessContrast { brightness, contrast }) => column![
            slider_row(app, "Brightness", format!("{brightness:.0}"), -150.0..=150.0, brightness, move |v| set(Op::Adjust(Adjustment::BrightnessContrast { brightness: v.round(), contrast }))),
            slider_row(app, "Contrast", format!("{contrast:.0}"), -50.0..=100.0, contrast, move |v| set(Op::Adjust(Adjustment::BrightnessContrast { brightness, contrast: v.round() }))),
        ]
        .spacing(10)
        .into(),
        Op::Adjust(Adjustment::Levels { in_black, in_white, gamma, out_black, out_white }) => {
            let lv = move |ib: u8, iw: u8, g: f32, ob: u8, ow: u8| set(Op::Adjust(Adjustment::Levels { in_black: ib, in_white: iw, gamma: g, out_black: ob, out_white: ow }));
            column![
                histogram(&s.histogram, Some((in_black, in_white))),
                slider_row(app, "Shadows", in_black.to_string(), 0.0..=253.0, in_black as f32, move |v| lv((v as u8).min(in_white.saturating_sub(2)), in_white, gamma, out_black, out_white)),
                // Midtones on a log scale: 1.0 sits in the middle, like Photoshop's slider.
                slider_row_mapped(app, "Midtones", format!("{gamma:.2}"), -1.0..=1.0, gamma.log10(), |g| g.clamp(0.1, 9.99).log10(), move |v| lv(in_black, in_white, 10f32.powf(v).clamp(0.1, 9.99), out_black, out_white)),
                slider_row(app, "Highlights", in_white.to_string(), 2.0..=255.0, in_white as f32, move |v| lv(in_black, (v as u8).max(in_black.saturating_add(2)), gamma, out_black, out_white)),
                label("Output Levels"),
                slider_row(app, "Black", out_black.to_string(), 0.0..=255.0, out_black as f32, move |v| lv(in_black, in_white, gamma, v as u8, out_white)),
                slider_row(app, "White", out_white.to_string(), 0.0..=255.0, out_white as f32, move |v| lv(in_black, in_white, gamma, out_black, v as u8)),
            ]
            .spacing(8)
            .into()
        }
        Op::Adjust(Adjustment::Curves { points }) => column![
            Canvas::new(CurvesEditor { points: points.clone(), histogram: s.histogram }).width(EDITOR).height(EDITOR),
            label("Click to add a point · drag to move · drag a point out to remove"),
        ]
        .spacing(8)
        .into(),
        Op::Adjust(Adjustment::HueSaturation { hue, saturation, lightness }) => column![
            slider_row(app, "Hue", format!("{hue:+.0}"), -180.0..=180.0, hue, move |v| set(Op::Adjust(Adjustment::HueSaturation { hue: v.round(), saturation, lightness }))),
            slider_row(app, "Saturation", format!("{saturation:+.0}"), -100.0..=100.0, saturation, move |v| set(Op::Adjust(Adjustment::HueSaturation { hue, saturation: v.round(), lightness }))),
            slider_row(app, "Lightness", format!("{lightness:+.0}"), -100.0..=100.0, lightness, move |v| set(Op::Adjust(Adjustment::HueSaturation { hue, saturation, lightness: v.round() }))),
        ]
        .spacing(10)
        .into(),
        Op::Adjust(Adjustment::Threshold { level }) => column![
            histogram(&s.histogram, Some((level, level))),
            slider_row(app, "Threshold Level", level.to_string(), 1.0..=255.0, level as f32, move |v| set(Op::Adjust(Adjustment::Threshold { level: v as u8 }))),
        ]
        .spacing(8)
        .into(),
        Op::Adjust(Adjustment::Posterize { levels }) => slider_row(app, "Levels", levels.to_string(), 2.0..=255.0, levels as f32, move |v| set(Op::Adjust(Adjustment::Posterize { levels: v as u8 }))),
        Op::Adjust(Adjustment::Invert | Adjustment::Desaturate) => space().into(),
        // Radii use a squared scale so small values (the common ones) get most of the slider.
        Op::Filter(Filter::GaussianBlur { radius }) => slider_row_mapped(app, "Radius", format!("{radius:.1} px"), 0.0..=1.0, radius_to_t(radius), radius_to_t, move |t| set(Op::Filter(Filter::GaussianBlur { radius: t_to_radius(t) }))),
        Op::Filter(Filter::UnsharpMask { amount, radius, threshold }) => column![
            slider_row(app, "Amount", format!("{amount:.0}%"), 1.0..=500.0, amount, move |v| set(Op::Filter(Filter::UnsharpMask { amount: v.round(), radius, threshold }))),
            slider_row_mapped(app, "Radius", format!("{radius:.1} px"), 0.0..=1.0, radius_to_t(radius), radius_to_t, move |t| set(Op::Filter(Filter::UnsharpMask { amount, radius: t_to_radius(t), threshold }))),
            slider_row(app, "Threshold", format!("{threshold} levels"), 0.0..=255.0, threshold as f32, move |v| set(Op::Filter(Filter::UnsharpMask { amount, radius, threshold: v as u8 }))),
        ]
        .spacing(10)
        .into(),
        Op::Filter(Filter::AddNoise { amount, monochrome, seed }) => column![
            slider_row(app, "Amount", format!("{amount:.1}%"), 0.0..=100.0, amount, move |v| set(Op::Filter(Filter::AddNoise { amount: (v * 10.0).round() / 10.0, monochrome, seed }))),
            row![
                toggle("Monochromatic", monochrome, set(Op::Filter(Filter::AddNoise { amount, monochrome: !monochrome, seed }))),
                toggle("New seed", false, set(Op::Filter(Filter::AddNoise { amount, monochrome, seed: seed.wrapping_add(1) }))),
            ]
            .spacing(6),
        ]
        .spacing(10)
        .into(),
        Op::Filter(Filter::BoxBlur { radius }) => slider_row(app, "Radius", format!("{radius:.0} px"), 1.0..=100.0, radius, move |v| set(Op::Filter(Filter::BoxBlur { radius: v.round() }))),
        Op::Filter(Filter::HighPass { radius }) => slider_row_mapped(app, "Radius", format!("{radius:.1} px"), 0.0..=1.0, radius_to_t(radius), radius_to_t, move |t| set(Op::Filter(Filter::HighPass { radius: t_to_radius(t) }))),
        Op::Filter(Filter::Sharpen) | Op::Adjust(Adjustment::ChannelLut(_)) => space().into(),
        Op::Filter(Filter::Mosaic { cell }) => slider_row(app, "Cell Size", format!("{cell} px"), 2.0..=200.0, cell as f32, move |v| set(Op::Filter(Filter::Mosaic { cell: v as u32 }))),
    };

    let status = if s.rendering && s.preview_enabled { "Rendering preview…" } else { "" };
    let footer = row![
        toggle("Preview", s.preview_enabled, Message::AdjustTogglePreview),
        button(text("Reset").size(TEXT_SIZE_SMALL)).padding(Padding::from([4, 10])).style(style::ghost(false)).on_press(Message::AdjustReset),
        space::horizontal(),
        button(text("Cancel").size(TEXT_SIZE)).padding(Padding::from([5, 12])).style(style::ghost(false)).on_press(Message::AdjustCancel),
        button(text("OK").size(TEXT_SIZE).font(bold())).padding(Padding::from([5, 16])).style(style::primary).on_press(Message::AdjustOk),
    ]
    .spacing(6)
    .align_y(Vertical::Center);

    let card = container(
        column![
            row![text(s.op.name()).size(14).font(bold()), space::horizontal(), text(status).size(TEXT_SIZE_SMALL).color(TEXT_MUTED)]
                .align_y(Vertical::Center),
            controls,
            footer,
        ]
        .spacing(14),
    )
    .width(WIDTH)
    .padding(16)
    .style(|_| container::Style {
        background: Some(BG_PANEL.into()),
        border: iced::Border { color: BORDER_SUBTLE, width: 1.0, radius: 8.0.into() },
        shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.5), offset: iced::Vector::new(0.0, 8.0), blur_radius: 30.0 },
        ..Default::default()
    });

    // Top-right of the canvas, clear of the panels.
    let b = app.canvas_bounds;
    let x = (b.x + b.width - WIDTH - 16.0).max(b.x);
    // Clicks anywhere else do nothing (the dialog is modal); scrolling still zooms the canvas.
    let blocker = mouse_area(space().width(Fill).height(Fill)).on_press(Message::Noop).on_right_press(Message::Noop);
    Some(stack![blocker, pin(card).x(x).y(b.y + 16.0)].into())
}

fn radius_to_t(r: f32) -> f32 {
    ((r - 0.1) / 99.9).max(0.0).sqrt()
}

fn t_to_radius(t: f32) -> f32 {
    ((0.1 + 99.9 * t * t) * 10.0).round() / 10.0
}

fn slider_row<'a>(
    app: &'a Photo,
    name: &'static str,
    shown: String,
    range: std::ops::RangeInclusive<f32>,
    v: f32,
    on_change: impl Fn(f32) -> Message + Clone + 'a,
) -> Element<'a, Message> {
    slider_row_mapped(app, name, shown, range, v, |x| x, on_change)
}

/// A slider whose value box shows a different unit than the slider moves in: `map` turns a
/// typed value into slider units (e.g. a radius in px into the slider's squared scale).
fn slider_row_mapped<'a>(
    app: &'a Photo,
    name: &'static str,
    shown: String,
    range: std::ops::RangeInclusive<f32>,
    v: f32,
    map: fn(f32) -> f32,
    on_change: impl Fn(f32) -> Message + Clone + 'a,
) -> Element<'a, Message> {
    let (lo, hi) = (*range.start(), *range.end());
    let typed = on_change.clone();
    let field = super::field::custom_live(app, name, shown, 80, move |t| {
        let n: f32 = super::field::number_text(t).parse().ok()?;
        n.is_finite().then(|| typed(map(n).clamp(lo, hi)))
    });
    column![
        row![label(name).width(Fill), field].align_y(Vertical::Center),
        slider(range, v, on_change).step(0.001_f32).style(style::slider),
    ]
    .spacing(4)
    .into()
}

fn toggle<'a>(name: &'a str, on: bool, msg: Message) -> Element<'a, Message> {
    button(text(name).size(TEXT_SIZE_SMALL).color(if on { TEXT_PRIMARY } else { TEXT_SECONDARY }))
        .padding(Padding::from([4, 10]))
        .style(style::ghost(on))
        .on_press(msg)
        .into()
}

/// Luminance histogram, with optional markers (black and white points).
fn histogram<'a>(h: &[u32; 256], markers: Option<(u8, u8)>) -> Element<'a, Message> {
    Canvas::new(Histogram { h: *h, markers }).width(Fill).height(90).into()
}

struct Histogram {
    h: [u32; 256],
    markers: Option<(u8, u8)>,
}

/// Scale for histogram bars: ignore the top spikes so the rest stays readable.
fn histogram_scale(h: &[u32; 256]) -> f32 {
    let mut sorted = *h;
    sorted.sort_unstable();
    sorted[250].max(1) as f32
}

fn draw_histogram(frame: &mut Frame, h: &[u32; 256], area: Rectangle, color: Color) {
    let scale = histogram_scale(h);
    let bars = Path::new(|b| {
        for (i, &v) in h.iter().enumerate() {
            let height = (v as f32 / scale).min(1.0) * area.height;
            let x = area.x + i as f32 / 256.0 * area.width;
            b.rectangle(Point::new(x, area.y + area.height - height), Size::new(area.width / 256.0 + 0.5, height));
        }
    });
    frame.fill(&bars, color);
}

impl canvas::Program<Message> for Histogram {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), BG_INPUT);
        let area = Rectangle::new(Point::ORIGIN, bounds.size());
        draw_histogram(&mut frame, &self.h, area, TEXT_SECONDARY);
        if let Some((a, b)) = self.markers {
            for m in [a, b] {
                let x = m as f32 / 255.0 * bounds.width;
                frame.stroke(&Path::line(Point::new(x, 0.0), Point::new(x, bounds.height)), Stroke::default().with_width(1.0).with_color(ACCENT_PRIMARY));
            }
        }
        vec![frame.into_geometry()]
    }
}

const EDITOR: f32 = 288.0;
const PAD: f32 = 8.0;

/// Curves editor: a 0..255 square with the histogram behind the curve.
struct CurvesEditor {
    points: Vec<(u8, u8)>,
    histogram: [u32; 256],
}

#[derive(Default)]
struct CurveDrag {
    /// Index of the point being dragged, in sorted order.
    dragging: Option<usize>,
}

impl CurvesEditor {
    fn sorted(&self) -> Vec<(u8, u8)> {
        let mut p = self.points.clone();
        p.sort_by_key(|p| p.0);
        p
    }

    fn to_value(&self, p: Point, size: Size) -> (f32, f32) {
        let span = size.width - 2.0 * PAD;
        ((p.x - PAD) / span * 255.0, (1.0 - (p.y - PAD) / span) * 255.0)
    }

    fn to_screen(&self, (x, y): (u8, u8), size: Size) -> Point {
        let span = size.width - 2.0 * PAD;
        Point::new(PAD + x as f32 / 255.0 * span, PAD + (1.0 - y as f32 / 255.0) * span)
    }
}

impl canvas::Program<Message> for CurvesEditor {
    type State = CurveDrag;

    fn update(&self, state: &mut CurveDrag, event: &canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<Message>> {
        let canvas::Event::Mouse(event) = event else { return None };
        let size = bounds.size();
        let pts = self.sorted();
        let publish = |pts: Vec<(u8, u8)>| Some(canvas::Action::publish(Message::AdjustSet(Op::Adjust(Adjustment::Curves { points: pts }))).and_capture());
        match event {
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                let p = cursor.position_in(bounds)?;
                if let Some(i) = pts.iter().position(|&q| {
                    let s = self.to_screen(q, size);
                    (s.x - p.x).abs() <= 7.0 && (s.y - p.y).abs() <= 7.0
                }) {
                    state.dragging = Some(i);
                    return Some(canvas::Action::request_redraw().and_capture());
                }
                let (x, y) = self.to_value(p, size);
                let new = (x.round().clamp(0.0, 255.0) as u8, y.round().clamp(0.0, 255.0) as u8);
                if pts.iter().any(|q| q.0 == new.0) {
                    return None;
                }
                let mut next = pts.clone();
                next.push(new);
                next.sort_by_key(|p| p.0);
                state.dragging = next.iter().position(|q| *q == new);
                publish(next)
            }
            mouse::Event::CursorMoved { .. } => {
                let i = state.dragging?;
                let p = cursor.position()?;
                let local = Point::new(p.x - bounds.x, p.y - bounds.y);
                let (x, y) = self.to_value(local, size);
                let mut next = pts.clone();
                let endpoint = i == 0 || i == next.len() - 1;
                // Dragged well outside: remove it (not the two end points).
                if !endpoint && (!(-20.0..=275.0).contains(&x) || !(-20.0..=275.0).contains(&y)) {
                    next.remove(i);
                    state.dragging = None;
                    return publish(next);
                }
                // Keep points in order: stay between the neighbours.
                let lo = if i > 0 { next[i - 1].0 as f32 + 1.0 } else { 0.0 };
                let hi = if i + 1 < next.len() { next[i + 1].0 as f32 - 1.0 } else { 255.0 };
                next[i] = (x.round().clamp(lo, hi) as u8, y.round().clamp(0.0, 255.0) as u8);
                publish(next)
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) => {
                state.dragging.take().map(|_| canvas::Action::request_redraw().and_capture())
            }
            _ => None,
        }
    }

    fn draw(&self, _: &CurveDrag, renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let size = bounds.size();
        let mut frame = Frame::new(renderer, size);
        let span = size.width - 2.0 * PAD;
        let area = Rectangle::new(Point::new(PAD, PAD), Size::new(span, span));
        frame.fill_rectangle(area.position(), area.size(), BG_INPUT);
        draw_histogram(&mut frame, &self.histogram, area, Color::from_rgba(1.0, 1.0, 1.0, 0.12));
        let grid = Path::new(|b| {
            for i in 1..4 {
                let t = PAD + span * i as f32 / 4.0;
                b.move_to(Point::new(t, PAD));
                b.line_to(Point::new(t, PAD + span));
                b.move_to(Point::new(PAD, t));
                b.line_to(Point::new(PAD + span, t));
            }
        });
        frame.stroke(&grid, Stroke::default().with_width(1.0).with_color(BORDER_SUBTLE));
        frame.stroke(&Path::line(Point::new(PAD, PAD + span), Point::new(PAD + span, PAD)), Stroke::default().with_width(1.0).with_color(Color::from_rgba(1.0, 1.0, 1.0, 0.15)));
        let table = curve_table(&self.points);
        let curve = Path::new(|b| {
            for (i, &v) in table.iter().enumerate() {
                let p = self.to_screen((i as u8, v), size);
                if i == 0 { b.move_to(p) } else { b.line_to(p) }
            }
        });
        frame.stroke(&curve, Stroke::default().with_width(2.0).with_color(TEXT_PRIMARY));
        for q in self.sorted() {
            let p = self.to_screen(q, size);
            let r = Path::rectangle(Point::new(p.x - 4.0, p.y - 4.0), Size::new(8.0, 8.0));
            frame.fill(&r, BG_PANEL);
            frame.stroke(&r, Stroke::default().with_width(1.5).with_color(TEXT_PRIMARY));
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, _: &CurveDrag, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if cursor.position_in(bounds).is_some() { mouse::Interaction::Crosshair } else { mouse::Interaction::default() }
    }
}
