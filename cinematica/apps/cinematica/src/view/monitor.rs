//! Source and Program monitors: viewer, scrub bar, timecode row and transport controls.

use iced::alignment::Vertical;
use iced::widget::canvas::{self, Frame, Geometry, Path};
use iced::widget::{Canvas, column, container, image, row, space, text};
use iced::{ContentFit, Element, Fill, Padding, Point, Rectangle, Renderer, Size, Theme, mouse};
use jd_ui::widget::{bold, field_box, hover_tip, icon_button, panel_header, value, vline};
use jd_ui::{Icon, icon, style, tokens::*};

use super::panel;
use crate::app::{Editor, Focus, Message, timecode};
use jd_timeline::Time;
use crate::source::GoTo;

pub fn source(app: &Editor) -> Element<'_, Message> {
    let s = &app.source;
    let tabs = [s.title.as_str(), "Effect Controls", "Audio Clip Mixer", "Metadata"];
    let body: Element<'_, Message> = match app.source_tab {
        0 => source_body(app),
        1 => placeholder("Select a clip in the timeline to edit its Motion, Opacity and effects"),
        2 => placeholder("Clip volume and pan per track appear here"),
        _ => match app.source_item().and_then(|m| m.info.as_ref().map(|i| (m, i))) {
            Some((m, i)) => metadata(m, i),
            None => placeholder("File details of the source clip appear here"),
        },
    };
    panel(column![panel_header(&tabs, app.source_tab, Message::SourceTab), body])
        .width(Fill)
        .height(Fill)
        .into()
}

fn source_body(app: &Editor) -> Element<'_, Message> {
    let s = &app.source;
    let fps = app.source_item().map_or(app.sequence.rate.timecode_base(), |m| m.fps(app.sequence.rate.timecode_base()));
    let viewer: Element<'_, Message> = match (&s.status.error, &s.image, s.item) {
        (None, _, Some(_)) if s.audio_only => {
            let duration = s.status.duration.max(1e-6);
            let frac = |t: f64| (t / duration) as f32;
            Canvas::new(Waveform {
                peaks: app.source_item().and_then(|m| m.waveform.clone()),
                head: frac(s.status.position),
                in_point: s.in_point.map(frac),
                out_point: s.out_point.map(frac),
            })
            .width(Fill)
            .height(Fill)
            .into()
        }
        (Some(e), _, _) => placeholder_owned(format!("Can't play this clip: {e}")),
        (None, Some(img), _) => image(img.handle().clone()).content_fit(ContentFit::Contain).width(Fill).height(Fill).into(),
        (None, None, Some(_)) => placeholder("Loading..."),
        (None, None, None) => placeholder("Double-click a clip in the Project panel to view it here"),
    };
    let last = s.last_frame() as f32;
    let fraction = |t: f64| if last > 0.0 { t as f32 / last } else { 0.0 };
    let scrub = ScrubBar {
        head: s.item.map(|_| fraction(s.status.position)),
        in_point: s.in_point.map(fraction),
        out_point: s.out_point.map(fraction),
        interactive: s.loaded(),
    };
    let on = s.loaded();
    let msg = |m: Message| on.then(|| Message::OnPanel(Focus::Source, Box::new(m)));
    let play_icon = if s.status.playing { Icon::Pause } else { Icon::Play };
    let buttons = vec![
        Btn(Icon::MarkIn, "Mark In (I)", msg(Message::MarkIn)),
        Btn(Icon::MarkOut, "Mark Out (O)", msg(Message::MarkOut)),
        Btn::DIVIDER,
        Btn(Icon::GoToIn, "Go to In (Shift+I)", msg(Message::GoTo(GoTo::In))),
        Btn(Icon::StepBack, "Step Back 1 Frame (Left)", msg(Message::Step(-1))),
        Btn(play_icon, "Play/Stop (Space)", msg(Message::TogglePlay)),
        Btn(Icon::StepForward, "Step Forward 1 Frame (Right)", msg(Message::Step(1))),
        Btn(Icon::GoToOut, "Go to Out (Shift+O)", msg(Message::GoTo(GoTo::Out))),
        Btn::DIVIDER,
        Btn(Icon::Insert, "Insert (,)", msg(Message::Insert)),
        Btn(Icon::Overwrite, "Overwrite (.)", msg(Message::Overwrite)),
        Btn(Icon::Camera, "Export Frame", None),
    ];
    let speed = (s.status.playing && s.status.speed != 1.0).then(|| format!("{:+}x", s.status.speed));
    monitor_body(
        viewer,
        scrub,
        timecode(s.status.position, fps),
        speed,
        timecode(if s.item.is_some() { s.marked_duration() } else { 0.0 }, fps),
        buttons,
    )
}

pub fn program(app: &Editor) -> Element<'_, Message> {
    let seq = &app.sequence;
    let msg = |m: Message| Some(Message::OnPanel(Focus::Timeline, Box::new(m)));
    let buttons = vec![
        Btn(Icon::MarkIn, "Mark In (I)", msg(Message::MarkIn)),
        Btn(Icon::MarkOut, "Mark Out (O)", msg(Message::MarkOut)),
        Btn::DIVIDER,
        Btn(Icon::GoToIn, "Go to In (Shift+I)", msg(Message::GoTo(GoTo::In))),
        Btn(Icon::StepBack, "Step Back 1 Frame (Left)", msg(Message::Step(-1))),
        Btn(Icon::Play, "Play/Stop (Space)", msg(Message::TogglePlay)),
        Btn(Icon::StepForward, "Step Forward 1 Frame (Right)", msg(Message::Step(1))),
        Btn(Icon::GoToOut, "Go to Out (Shift+O)", msg(Message::GoTo(GoTo::Out))),
        Btn::DIVIDER,
        Btn(Icon::Lift, "Lift (;)", msg(Message::Lift)),
        Btn(Icon::Extract, "Extract (')", msg(Message::Extract)),
        Btn(Icon::Camera, "Export Frame", None),
    ];
    let viewer: Element<'_, Message> = match (&app.program.image, app.program.showing) {
        (Some(img), true) => image(img.handle().clone()).content_fit(ContentFit::Contain).width(Fill).height(Fill).into(),
        (_, true) => placeholder(""),
        (_, false) if seq.duration() == 0 => placeholder("The sequence plays here"),
        (_, false) => placeholder(""),
    };
    let duration = seq.duration();
    let head = (duration > 0).then(|| (app.playhead as f64 / duration as f64) as f32);
    let marked = match (app.seq_in, app.seq_out) {
        (None, None) => duration,
        (i, o) => o.unwrap_or(duration) - i.unwrap_or(0),
    };
    let frac = |t: Time| if duration > 0 { Some(t as f32 / duration as f32) } else { None };
    let body = monitor_body(
        viewer,
        ScrubBar {
            head: head.or(Some(0.0)),
            in_point: app.seq_in.and_then(frac),
            out_point: app.seq_out.and_then(frac),
            interactive: false,
        },
        seq.rate.timecode(app.playhead),
        None,
        seq.rate.timecode(marked.max(0)),
        buttons,
    );
    let tabs = [app.program.title.as_str()];
    panel(column![panel_header(&tabs, 0, |_| Message::Noop), body])
        .width(Fill)
        .height(Fill)
        .into()
}

fn placeholder<'a>(hint: &'a str) -> Element<'a, Message> {
    container(text(hint).size(TEXT_SIZE_SMALL).color(TEXT_MUTED)).center(Fill).style(style::fill(BG_CANVAS)).into()
}

fn placeholder_owned<'a>(hint: String) -> Element<'a, Message> {
    container(text(hint).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY)).center(Fill).padding(20).style(style::fill(BG_CANVAS)).into()
}

fn metadata<'a>(m: &'a crate::media::MediaItem, i: &'a jd_media::MediaInfo) -> Element<'a, Message> {
    let line = |k: &'a str, v: String| row![text(k).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(110), value(v)];
    let mut col = column![line("File", m.path.display().to_string()), line("Duration", format!("{:.2} s", m.duration()))].spacing(6);
    if let Some(v) = &i.video {
        col = col
            .push(line("Video", format!("{} × {}, {}", v.width, v.height, v.codec)))
            .push(line("Frame rate", format!("{:.3} fps", v.rate.as_f64())));
    }
    if let Some(a) = &i.audio {
        col = col.push(line("Audio", format!("{} Hz, {} ch, {}", a.sample_rate, a.channels, a.codec)));
    }
    container(col).padding(16).width(Fill).height(Fill).into()
}

/// A transport button (icon, tooltip, message; no message = disabled).
struct Btn(Icon, &'static str, Option<Message>);

impl Btn {
    const DIVIDER: Btn = Btn(Icon::Menu, "", None);

    fn is_divider(&self) -> bool {
        self.1.is_empty()
    }
}

fn monitor_body<'a>(
    viewer: Element<'a, Message>,
    scrub: ScrubBar,
    current: String,
    speed: Option<String>,
    duration: String,
    buttons: Vec<Btn>,
) -> Element<'a, Message> {
    let viewer = container(viewer).width(Fill).height(Fill).style(style::fill(BG_CANVAS));

    let dropdown = |s: &'a str| {
        field_box(row![value(s), icon(Icon::Chevron, 10.0, TEXT_SECONDARY)].spacing(4).align_y(Vertical::Center))
    };
    let mut left = row![text(current).size(15).font(bold()).color(ACCENT_PRIMARY)].spacing(8).align_y(Vertical::Center);
    if let Some(s) = speed {
        left = left.push(text(s).size(TEXT_SIZE_SMALL).color(WARNING));
    }
    let timecode_row = row![
        left,
        space::horizontal(),
        dropdown("Fit"),
        dropdown("Full"),
        space::horizontal(),
        text(duration).size(TEXT_SIZE).color(TEXT_SECONDARY),
    ]
    .spacing(8)
    .align_y(Vertical::Center)
    .padding(Padding::from([0, 12]))
    .height(30);

    let mut transport = row![].spacing(4).align_y(Vertical::Center);
    for b in buttons {
        transport = transport.push(if b.is_divider() {
            vline(18.0, BORDER_SUBTLE)
        } else {
            let Btn(i, tip, msg) = b;
            hover_tip(icon_button(i, 16.0, false, msg).padding(6), tip, iced::widget::tooltip::Position::Top)
        });
    }

    column![
        viewer,
        Canvas::new(scrub).width(Fill).height(22),
        timecode_row,
        container(transport).center_x(Fill).height(38).align_y(Vertical::Center),
    ]
    .into()
}

/// Fractions of the clip (0..=1) for the playhead and marks.
struct ScrubBar {
    head: Option<f32>,
    in_point: Option<f32>,
    out_point: Option<f32>,
    interactive: bool,
}

const SCRUB_MARGIN: f32 = 12.0;

impl ScrubBar {
    fn fraction_at(bounds: Rectangle, x: f32) -> f32 {
        ((x - SCRUB_MARGIN) / (bounds.width - 2.0 * SCRUB_MARGIN).max(1.0)).clamp(0.0, 1.0)
    }
}

impl canvas::Program<Message> for ScrubBar {
    /// Dragging the playhead.
    type State = bool;

    fn update(
        &self,
        dragging: &mut bool,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if !self.interactive {
            return None;
        }
        let canvas::Event::Mouse(event) = event else { return None };
        match event {
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                let p = cursor.position_in(bounds)?;
                *dragging = true;
                Some(canvas::Action::publish(Message::Scrub(Self::fraction_at(bounds, p.x))).and_capture())
            }
            mouse::Event::CursorMoved { position } if *dragging => {
                Some(canvas::Action::publish(Message::Scrub(Self::fraction_at(bounds, position.x - bounds.x))).and_capture())
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) if *dragging => {
                *dragging = false;
                Some(canvas::Action::capture())
            }
            _ => None,
        }
    }

    fn draw(&self, _: &bool, renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (left, width) = (SCRUB_MARGIN, (bounds.width - 2.0 * SCRUB_MARGIN).max(0.0));
        let x_of = |f: f32| (left + width * f.clamp(0.0, 1.0)).round();
        let mid = (bounds.height / 2.0).round();
        frame.fill(&Path::rounded_rectangle(Point::new(left, mid - 1.0), Size::new(width, 2.0), 1.0.into()), BORDER_SUBTLE);

        // In/Out: the range between them, and a bracket at each.
        if self.in_point.is_some() || self.out_point.is_some() {
            let (a, b) = (x_of(self.in_point.unwrap_or(0.0)), x_of(self.out_point.unwrap_or(1.0)));
            let range = iced::Color { a: 0.45, ..ACCENT_PRIMARY };
            frame.fill_rectangle(Point::new(a, mid - 2.0), Size::new((b - a).max(1.0), 4.0), range);
        }
        for x in [self.in_point, self.out_point].into_iter().flatten().map(x_of) {
            frame.fill_rectangle(Point::new(x - 1.0, 5.0), Size::new(2.0, bounds.height - 10.0), TEXT_SECONDARY);
        }
        if let Some(h) = self.head {
            let x = x_of(h);
            frame.fill_rectangle(Point::new(x, 3.0), Size::new(1.0, bounds.height - 6.0), ACCENT_PRIMARY);
            frame.fill(&Path::rounded_rectangle(Point::new(x - 5.0, 2.0), Size::new(11.0, 6.0), 2.0.into()), ACCENT_PRIMARY);
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, dragging: &bool, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if self.interactive && (*dragging || cursor.is_over(bounds)) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

/// An audio-only clip in the Source monitor: its whole waveform, marks and playhead.
struct Waveform {
    peaks: Option<std::sync::Arc<Vec<f32>>>,
    head: f32,
    in_point: Option<f32>,
    out_point: Option<f32>,
}

impl canvas::Program<Message> for Waveform {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), BG_CANVAS);
        let (left, width) = (SCRUB_MARGIN, (bounds.width - 2.0 * SCRUB_MARGIN).max(1.0));
        let mid = bounds.height / 2.0;
        let half = bounds.height * 0.4;
        if self.in_point.is_some() || self.out_point.is_some() {
            let a = left + width * self.in_point.unwrap_or(0.0);
            let b = left + width * self.out_point.unwrap_or(1.0);
            frame.fill_rectangle(Point::new(a, 0.0), Size::new((b - a).max(1.0), bounds.height), iced::Color { a: 0.12, ..ACCENT_PRIMARY });
        }
        match &self.peaks {
            Some(peaks) if !peaks.is_empty() => {
                let mut lines = canvas::path::Builder::new();
                let n = peaks.len() as f32;
                for px in 0..width as usize {
                    let a = (px as f32 / width * n) as usize;
                    let b = (((px + 1) as f32 / width * n) as usize).max(a + 1).min(peaks.len());
                    let peak = peaks[a.min(peaks.len() - 1)..b].iter().fold(0.0f32, |m, v| m.max(*v));
                    let y = (peak.sqrt() * half).max(0.5);
                    let x = left + px as f32 + 0.5;
                    lines.move_to(Point::new(x, mid - y));
                    lines.line_to(Point::new(x, mid + y));
                }
                frame.stroke(&lines.build(), canvas::Stroke::default().with_width(1.0).with_color(iced::Color::from_rgb8(0x5F, 0xC2, 0x96)));
            }
            _ => {
                frame.fill_rectangle(Point::new(left, mid), Size::new(width, 1.0), BORDER_SUBTLE);
            }
        }
        let x = (left + width * self.head.clamp(0.0, 1.0)).round();
        frame.fill_rectangle(Point::new(x, 0.0), Size::new(1.0, bounds.height), ACCENT_PRIMARY);
        vec![frame.into_geometry()]
    }
}
