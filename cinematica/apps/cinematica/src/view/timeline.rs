//! Timeline panel: sequence tabs, toolbar, track headers (widgets) beside a canvas that draws
//! the ruler, clips and playhead and handles every tool. Drags are previewed in the canvas
//! and become one edit (one undo step) on release.

use iced::alignment::Vertical;
use iced::keyboard;
use iced::mouse::{self, ScrollDelta};
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text};
use iced::widget::{Canvas, button, column, container, row, space, text};
use iced::{Color, Element, Fill, Padding, Pixels, Point, Rectangle, Renderer, Size, Theme};
use jd_timeline::{Clip, ClipId, Edge, Sequence, Time, TrackKind, TrackRef, from_seconds, to_seconds};
use jd_ui::widget::{bold, label, panel_header};
use jd_ui::{Icon, icon, style, tokens::*};

use super::{panel, toggle_button};
use crate::app::{Editor, Message, TrackToggle};
use crate::tool::Tool;

const HEADER_WIDTH: f32 = 150.0;
const RULER_HEIGHT: f32 = 24.0;
const DIVIDER_HEIGHT: f32 = 6.0;
const MIX_HEIGHT: f32 = 30.0;
/// How close (px) the pointer must be to a clip edge to trim it.
const EDGE_GRAB: f32 = 6.0;
/// How close (px) things snap.
const SNAP_PX: f32 = 8.0;

const LANE: Color = Color::from_rgb8(0x1E, 0x1E, 0x1E);
const LANE_MIX: Color = Color::from_rgb8(0x18, 0x18, 0x18);
const ROW_LINE: Color = Color::from_rgb8(0x2B, 0x2B, 0x2B);
const VIDEO_CLIP: Color = Color::from_rgb8(0x6A, 0x6F, 0xB5);
const AUDIO_CLIP: Color = Color::from_rgb8(0x3F, 0x8F, 0x6B);
const SNAP_LINE: Color = Color::from_rgb8(0xF2, 0xD0, 0x4A);

pub fn track_height(r: TrackRef) -> f32 {
    if r.index == 0 { 52.0 } else { 38.0 }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum RowKind {
    Track(TrackRef),
    /// The gap between the video and audio tracks.
    Divider,
    Mix,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Row {
    kind: RowKind,
    y: f32,
    height: f32,
}

/// Vertical layout below the ruler: video tracks top-down (V3, V2, V1), divider, audio
/// tracks (A1, A2, A3), mix.
fn rows(seq: &Sequence) -> Vec<Row> {
    let mut out = Vec::new();
    let mut y = RULER_HEIGHT;
    let mut push = |kind, height| {
        out.push(Row { kind, y, height });
        y += height;
    };
    for i in (0..seq.video.len()).rev() {
        push(RowKind::Track(TrackRef::video(i)), track_height(TrackRef::video(i)));
    }
    push(RowKind::Divider, DIVIDER_HEIGHT);
    for i in 0..seq.audio.len() {
        push(RowKind::Track(TrackRef::audio(i)), track_height(TrackRef::audio(i)));
    }
    push(RowKind::Mix, MIX_HEIGHT);
    out
}

pub fn view(app: &Editor) -> Element<'_, Message> {
    let seq = &app.sequence;
    let status: Element<'_, Message> = match &app.notice {
        Some(n) => text(n).size(TEXT_SIZE_SMALL).color(WARNING).into(),
        None => label(format!(
            "{} × {}  ·  {:.2} fps  ·  {} kHz",
            seq.width,
            seq.height,
            seq.rate.as_f64(),
            seq.sample_rate / 1000
        ))
        .into(),
    };
    let toolbar = row![
        text(seq.rate.timecode(app.playhead)).size(16).font(bold()).color(ACCENT_PRIMARY),
        space().width(10),
        toggle_button(Icon::Magnet, app.snap, "Snap in Timeline (S)", Some(Message::ToggleSnap)),
        toggle_button(Icon::Link, app.linked, "Linked Selection", Some(Message::ToggleLinked)),
        toggle_button(Icon::Marker, false, "Add Marker (M)", None),
        toggle_button(Icon::Wrench, false, "Timeline Display Settings", None),
        space::horizontal(),
        status,
    ]
    .spacing(4)
    .align_y(Vertical::Center)
    .padding(Padding::from([0, 10]))
    .height(34);

    let layout = rows(seq);
    let mut headers = column![container(space()).width(Fill).height(RULER_HEIGHT).style(style::fill(BG_PANEL_HEADER))];
    for r in &layout {
        headers = headers.push(match r.kind {
            RowKind::Track(t) => track_header(seq, t, r.height),
            RowKind::Divider => container(space()).width(Fill).height(r.height).style(style::fill(BORDER_DEFAULT)).into(),
            RowKind::Mix => container(
                row![text("Mix").size(10).font(bold()), space::horizontal(), text("0.0").size(10).color(ACCENT_PRIMARY)]
                    .align_y(Vertical::Center),
            )
            .padding(Padding::from([0, 8]))
            .width(Fill)
            .height(r.height - 1.0)
            .align_y(Vertical::Center)
            .style(style::fill(BG_PANEL_HEADER))
            .into(),
        });
        // Track and mix headers are 1px short; the row line completes their height.
        if r.kind != RowKind::Divider {
            headers = headers.push(container(space()).width(Fill).height(1).style(style::fill(ROW_LINE)));
        }
    }

    let body = row![
        container(headers).width(HEADER_WIDTH).height(Fill).style(style::fill(Color::from_rgb8(0x1A, 0x1A, 0x1A))),
        container(space()).width(1).height(Fill).style(style::fill(BORDER_DEFAULT)),
        Canvas::new(Lanes { app, layout }).width(Fill).height(Fill),
    ];

    panel(column![panel_header(&[seq.name.as_str()], app.sequence_tab, Message::SequenceTab), toolbar, body])
        .width(Fill)
        .height(Fill)
        .into()
}

/// Small on/off button showing text (mute, solo).
fn pill<'a>(s: &'a str, on: bool, on_color: Color, msg: Message) -> Element<'a, Message> {
    button(container(text(s).size(9).font(bold()).color(if on { BG_INPUT } else { TEXT_SECONDARY })).center(Fill))
        .width(16)
        .height(16)
        .padding(0)
        .style(move |_, status| button::Style {
            background: Some(
                match (on, status) {
                    (true, _) => on_color,
                    (false, button::Status::Hovered) => Color::from_rgb8(0x48, 0x48, 0x48),
                    _ => BG_HOVER,
                }
                .into(),
            ),
            border: iced::border::rounded(2),
            ..Default::default()
        })
        .on_press(msg)
        .into()
}

fn small_icon<'a>(i: Icon, on: bool, msg: Message) -> Element<'a, Message> {
    button(icon(i, 12.0, if on { ICON } else { TEXT_MUTED }))
        .padding(2)
        .style(style::ghost(false))
        .on_press(msg)
        .into()
}

fn track_header(seq: &Sequence, r: TrackRef, height: f32) -> Element<'_, Message> {
    let t = seq.track(r);
    let toggle = move |what| Message::ToggleTrack(r, what);
    // Targeted tracks are white-on-blue, like Premiere's track targeting.
    let on = t.targeted;
    let target = button(
        container(text(r.name()).size(9).font(bold()).color(if on { Color::WHITE } else { TEXT_SECONDARY })).center(Fill),
    )
    .width(24)
    .height(16)
    .padding(0)
    .style(move |_, status| button::Style {
        background: Some(
            match (on, status) {
                (true, _) => ACCENT_PRIMARY,
                (false, button::Status::Hovered) => Color::from_rgb8(0x48, 0x48, 0x48),
                _ => BG_HOVER,
            }
            .into(),
        ),
        border: iced::border::rounded(2),
        ..Default::default()
    })
    .on_press(toggle(TrackToggle::Target));

    let mut header = row![small_icon(Icon::Lock, t.locked, toggle(TrackToggle::Lock)), target]
        .spacing(4)
        .align_y(Vertical::Center);
    if height >= 50.0 {
        let name = match r.kind {
            TrackKind::Video => format!("Video {}", r.index + 1),
            TrackKind::Audio => format!("Audio {}", r.index + 1),
        };
        header = header.push(text(name).size(10).color(TEXT_SECONDARY));
    }
    header = header.push(space::horizontal());
    match r.kind {
        TrackKind::Video => header = header.push(small_icon(Icon::Eye, t.enabled, toggle(TrackToggle::Enabled))),
        TrackKind::Audio => {
            header = header
                .push(pill("M", !t.enabled, WARNING, toggle(TrackToggle::Enabled)))
                .push(pill("S", t.solo, SUCCESS, toggle(TrackToggle::Solo)))
                .push(icon(Icon::Mic, 12.0, TEXT_MUTED));
        }
    }
    container(header)
        .padding(Padding::from([0, 6]))
        .width(Fill)
        .height(height - 1.0)
        .align_y(Vertical::Center)
        .style(style::fill(if t.locked { Color::from_rgb8(0x2A, 0x24, 0x24) } else { BG_PANEL_HEADER }))
        .into()
}

/// Seconds between ruler labels: the smallest "nice" step at least `min_px` apart.
fn label_step(px_per_second: f32, min_px: f32) -> f32 {
    const STEPS: &[f32] = &[0.2, 0.4, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0];
    STEPS.iter().copied().find(|s| s * px_per_second >= min_px).unwrap_or(3600.0)
}

#[derive(Debug, Clone)]
enum Drag {
    Scrub,
    Pan { x0: f32, scroll0: Time },
    Move {
        ids: Vec<ClipId>,
        x0: f32,
        /// Track grabbed, and the grabbed clip's span (for snapping both edges).
        from: TrackRef,
        span: (Time, Time),
        delta: Time,
        video_tracks: isize,
        audio_tracks: isize,
    },
    Trim { ids: Vec<ClipId>, edge: Edge, x0: f32, at: Time, delta: Time, ripple: bool },
    Slip { ids: Vec<ClipId>, x0: f32, delta: Time },
}

#[derive(Default)]
struct State {
    drag: Option<Drag>,
    modifiers: keyboard::Modifiers,
    /// Where the current drag snapped, for the guide line.
    snapped: Option<Time>,
}

struct Lanes<'a> {
    app: &'a Editor,
    layout: Vec<Row>,
}

enum Hit<'a> {
    Ruler,
    Clip { track: TrackRef, clip: &'a Clip, edge: Option<Edge> },
    Empty { track: Option<TrackRef> },
}

impl<'a> Lanes<'a> {
    fn x_of(&self, t: Time) -> f32 {
        to_seconds(t - self.app.scroll) as f32 * self.app.px_per_second
    }

    fn time_at(&self, x: f32) -> Time {
        self.app.scroll + from_seconds((x / self.app.px_per_second) as f64)
    }

    /// A horizontal pointer movement as a whole number of frames.
    fn dt(&self, dx: f32) -> Time {
        self.app.sequence.rate.snap(from_seconds((dx / self.app.px_per_second) as f64))
    }

    fn snap_threshold(&self) -> Time {
        from_seconds((SNAP_PX / self.app.px_per_second) as f64)
    }

    fn row_at(&self, y: f32) -> Option<&Row> {
        self.layout.iter().find(|r| y >= r.y && y < r.y + r.height)
    }

    fn hit(&self, p: Point) -> Hit<'a> {
        if p.y < RULER_HEIGHT {
            return Hit::Ruler;
        }
        let Some(Row { kind: RowKind::Track(r), .. }) = self.row_at(p.y).copied() else { return Hit::Empty { track: None } };
        let track = self.app.sequence.track(r);
        // Near an end trims it (the inner third at most, so short clips can still be moved).
        for c in &track.clips {
            let (xs, xe) = (self.x_of(c.start), self.x_of(c.end()));
            if p.x < xs || p.x > xe {
                continue;
            }
            let grab = EDGE_GRAB.min((xe - xs) / 3.0);
            let edge = if p.x >= xe - grab {
                Some(Edge::Out)
            } else if p.x <= xs + grab {
                Some(Edge::In)
            } else {
                None
            };
            return Hit::Clip { track: r, clip: c, edge };
        }
        Hit::Empty { track: Some(r) }
    }

    /// Snaps `t` (or a span's edges) to clip edges and the playhead, if snapping is on.
    /// Returns the adjustment to add and where it snapped.
    fn snap(&self, times: &[Time], ignore: &[ClipId], shift_held: bool) -> (Time, Option<Time>) {
        // Shift during a drag temporarily flips snapping, like Premiere.
        if self.app.snap == shift_held {
            return (0, None);
        }
        let threshold = self.snap_threshold();
        let extra = [self.app.playhead];
        let mut best: Option<(Time, Time)> = None;
        for &t in times {
            if let Some(p) = self.app.sequence.snap(t, threshold, ignore, &extra) {
                let adj = p - t;
                if best.is_none_or(|(b, _)| adj.abs() < b.abs()) {
                    best = Some((adj, p));
                }
            }
        }
        best.map_or((0, None), |(adj, p)| (adj, Some(p)))
    }

    /// The selection a press on `clip` acts on.
    fn grabbed(&self, clip: &Clip, add: bool) -> Vec<ClipId> {
        let sel = &self.app.selection;
        if sel.contains(&clip.id) && !add {
            sel.clone()
        } else if add {
            let mut s = sel.clone();
            s.extend(self.app.with_links(&[clip.id]).into_iter().filter(|i| !sel.contains(i)));
            s
        } else {
            self.app.with_links(&[clip.id])
        }
    }

    fn update_drag(&self, state: &mut State, p: Point) {
        let shift = state.modifiers.shift();
        let mut snapped = None;
        match &mut state.drag {
            Some(Drag::Move { ids, x0, from, span, delta, video_tracks, audio_tracks }) => {
                let raw = self.dt(p.x - *x0).max(-span.0);
                let (adj, at) = self.snap(&[span.0 + raw, span.1 + raw], ids, shift);
                *delta = raw + adj;
                snapped = at;
                if let Some(Row { kind: RowKind::Track(r), .. }) = self.row_at(p.y)
                    && r.kind == from.kind
                {
                    let d = r.index as isize - from.index as isize;
                    match r.kind {
                        TrackKind::Video => *video_tracks = d,
                        TrackKind::Audio => *audio_tracks = d,
                    }
                }
            }
            Some(Drag::Trim { ids, x0, at, delta, .. }) => {
                let raw = self.dt(p.x - *x0);
                let (adj, s) = self.snap(&[*at + raw], ids, shift);
                *delta = raw + adj;
                snapped = s;
            }
            Some(Drag::Slip { x0, delta, .. }) => *delta = -self.dt(p.x - *x0),
            _ => {}
        }
        state.snapped = snapped;
    }

    /// The edit a finished drag makes.
    fn finish(&self, drag: Drag) -> Option<Message> {
        match drag {
            Drag::Move { ids, delta, video_tracks, audio_tracks, .. } => (delta != 0 || video_tracks != 0 || audio_tracks != 0)
                .then_some(Message::MoveClips { ids, delta, video_tracks, audio_tracks }),
            Drag::Trim { ids, edge, delta, ripple, .. } => (delta != 0).then_some(Message::TrimClips { ids, edge, delta, ripple }),
            Drag::Slip { ids, delta, .. } => (delta != 0).then_some(Message::SlipClips { ids, delta }),
            Drag::Scrub | Drag::Pan { .. } => None,
        }
    }

    /// The sequence as it would look after the drag in progress (for drawing).
    fn preview(&self, drag: &Option<Drag>) -> Option<Sequence> {
        let mut s = self.app.sequence.clone();
        match drag.as_ref()? {
            Drag::Move { ids, delta, video_tracks, audio_tracks, .. } => {
                s.move_clips(ids, *delta, *video_tracks, *audio_tracks).ok()?;
            }
            Drag::Trim { ids, edge, delta, ripple, .. } => {
                if *ripple { s.ripple_trim(ids, *edge, *delta) } else { s.trim(ids, *edge, *delta) };
            }
            Drag::Slip { ids, delta, .. } => {
                s.slip(ids, *delta);
            }
            _ => return None,
        }
        Some(s)
    }
}

impl canvas::Program<Message> for Lanes<'_> {
    type State = State;

    fn update(&self, state: &mut State, event: &canvas::Event, bounds: Rectangle, cursor: mouse::Cursor) -> Option<canvas::Action<Message>> {
        if (bounds.width - self.app.timeline_width).abs() > 0.5 {
            return Some(canvas::Action::publish(Message::TimelineWidth(bounds.width)));
        }
        if let canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) = event {
            state.modifiers = *m;
            return None;
        }
        let canvas::Event::Mouse(event) = event else { return None };
        let publish = |m| Some(canvas::Action::publish(m).and_capture());
        let local = cursor.position_in(bounds);

        match event {
            mouse::Event::WheelScrolled { delta } => {
                local?;
                let (dx, dy) = match *delta {
                    ScrollDelta::Lines { x, y } => (x * 60.0, y * 60.0),
                    ScrollDelta::Pixels { x, y } => (x, y),
                };
                let m = state.modifiers;
                if m.control() || m.alt() {
                    let factor = if dy > 0.0 { 1.25 } else { 0.8 };
                    return publish(Message::ZoomTimeline { factor, anchor_x: local.map_or(f32::NAN, |p| p.x) });
                }
                let px = if dx.abs() > dy.abs() { -dx } else { -dy };
                return publish(Message::ScrollTimeline(px));
            }
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                let p = local?;
                let m = state.modifiers;
                let hit = self.hit(p);
                let tool = self.app.tool;
                if matches!(hit, Hit::Ruler) {
                    state.drag = Some(Drag::Scrub);
                    return publish(Message::SetPlayhead(self.time_at(p.x)));
                }
                if tool == Tool::Hand {
                    state.drag = Some(Drag::Pan { x0: p.x, scroll0: self.app.scroll });
                    return Some(canvas::Action::publish(Message::Focus(crate::app::Focus::Timeline)).and_capture());
                }
                match (tool, hit) {
                    (Tool::Razor, Hit::Clip { track, .. }) => {
                        let t = self.app.sequence.rate.snap(self.time_at(p.x));
                        let (adj, _) = self.snap(&[t], &[], m.shift());
                        let at = t + adj;
                        let tracks = if m.shift() {
                            self.app.sequence.track_refs()
                        } else {
                            // The track clicked, plus tracks of linked clips under the cut.
                            let mut refs = vec![track];
                            if let Some(c) = self.app.sequence.track(track).clip_at(at) {
                                for id in self.app.with_links(&[c.id]) {
                                    if let Some((r, _)) = self.app.sequence.find(id)
                                        && !refs.contains(&r)
                                    {
                                        refs.push(r);
                                    }
                                }
                            }
                            refs
                        };
                        publish(Message::Razor { at, tracks })
                    }
                    (Tool::TrackSelectForward, Hit::Clip { track, .. } | Hit::Empty { track: Some(track) }) => {
                        publish(Message::SelectForward { at: self.time_at(p.x), track: m.shift().then_some(track) })
                    }
                    (Tool::Selection | Tool::RippleEdit, Hit::Clip { clip, edge: Some(edge), .. }) => {
                        let ids = if m.alt() { vec![clip.id] } else { self.app.with_links(&[clip.id]) };
                        let at = if edge == Edge::In { clip.start } else { clip.end() };
                        state.drag = Some(Drag::Trim { ids, edge, x0: p.x, at, delta: 0, ripple: tool == Tool::RippleEdit });
                        Some(canvas::Action::request_redraw().and_capture())
                    }
                    (Tool::Selection | Tool::RippleEdit, Hit::Clip { track, clip, edge: None }) => {
                        let ids = if m.alt() { vec![clip.id] } else { self.grabbed(clip, m.shift()) };
                        if tool == Tool::Selection {
                            state.drag = Some(Drag::Move {
                                ids,
                                x0: p.x,
                                from: track,
                                span: (clip.start, clip.end()),
                                delta: 0,
                                video_tracks: 0,
                                audio_tracks: 0,
                            });
                        }
                        publish(Message::Select { id: clip.id, add: m.shift() })
                    }
                    (Tool::Slip, Hit::Clip { clip, .. }) => {
                        state.drag = Some(Drag::Slip { ids: self.app.with_links(&[clip.id]), x0: p.x, delta: 0 });
                        publish(Message::Select { id: clip.id, add: false })
                    }
                    (_, Hit::Empty { .. }) if !m.shift() => publish(Message::Deselect),
                    _ => Some(canvas::Action::publish(Message::Focus(crate::app::Focus::Timeline)).and_capture()),
                }
            }
            mouse::Event::CursorMoved { position } => {
                let p = Point::new(position.x - bounds.x, position.y - bounds.y);
                match &state.drag {
                    Some(Drag::Scrub) => publish(Message::SetPlayhead(self.time_at(p.x))),
                    Some(Drag::Pan { x0, scroll0 }) => {
                        let target = (*scroll0 - from_seconds(((p.x - x0) / self.app.px_per_second) as f64)).max(0);
                        let px = to_seconds(target - self.app.scroll) as f32 * self.app.px_per_second;
                        publish(Message::ScrollTimeline(px))
                    }
                    Some(_) => {
                        self.update_drag(state, p);
                        Some(canvas::Action::request_redraw().and_capture())
                    }
                    // Redraw for the hover cursor only when it changes is handled by iced.
                    None => None,
                }
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) => {
                let drag = state.drag.take()?;
                state.snapped = None;
                match self.finish(drag) {
                    Some(m) => publish(m),
                    None => Some(canvas::Action::request_redraw().and_capture()),
                }
            }
            _ => None,
        }
    }

    fn draw(&self, state: &State, renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let w = bounds.width;
        let app = self.app;
        let preview = self.preview(&state.drag);
        let seq = preview.as_ref().unwrap_or(&app.sequence);
        let moving: &[ClipId] = match &state.drag {
            Some(Drag::Move { ids, .. } | Drag::Trim { ids, .. } | Drag::Slip { ids, .. }) => ids,
            _ => &[],
        };
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), Color::from_rgb8(0x1A, 0x1A, 0x1A));

        // Lanes.
        for r in &self.layout {
            let color = match r.kind {
                RowKind::Track(t) if seq.track(t).locked => Color::from_rgb8(0x22, 0x1E, 0x1E),
                RowKind::Track(_) => LANE,
                RowKind::Divider => BORDER_DEFAULT,
                RowKind::Mix => LANE_MIX,
            };
            frame.fill_rectangle(Point::new(0.0, r.y), Size::new(w, r.height), color);
            if matches!(r.kind, RowKind::Track(_)) {
                frame.fill_rectangle(Point::new(0.0, r.y + r.height - 1.0), Size::new(w, 1.0), ROW_LINE);
            }
        }

        // Clips.
        let empty = seq.video.iter().chain(&seq.audio).all(|t| t.clips.is_empty());
        for r in &self.layout {
            let RowKind::Track(tr) = r.kind else { continue };
            let track = seq.track(tr);
            for c in &track.clips {
                let (x0, x1) = (self.x_of(c.start), self.x_of(c.end()));
                if x1 < 0.0 || x0 > w {
                    continue;
                }
                let selected = app.selection.contains(&c.id);
                let base = if tr.kind == TrackKind::Video { VIDEO_CLIP } else { AUDIO_CLIP };
                let mut fill = if track.enabled { base } else { Color { a: 0.4, ..base } };
                if moving.contains(&c.id) {
                    fill = Color { a: fill.a * 0.8, ..fill };
                }
                let rect = Path::rounded_rectangle(Point::new(x0.max(-4.0), r.y + 1.0), Size::new((x1.min(w + 4.0) - x0.max(-4.0)).max(1.0) - 1.0, r.height - 3.0), 2.0.into());
                frame.fill(&rect, fill);
                // Label band and name, cut to the clip's width.
                let band_w = (x1.min(w) - x0.max(0.0) - 1.0).max(0.0);
                frame.fill_rectangle(Point::new(x0.max(0.0), r.y + 1.0), Size::new(band_w, 13.0), Color::from_rgba(0.0, 0.0, 0.0, 0.22));
                let chars = ((band_w - 6.0) / 5.6).floor().max(0.0) as usize;
                if chars > 0 {
                    let mut name: String = c.name.chars().take(chars).collect();
                    if name.chars().count() < c.name.chars().count() && chars > 2 {
                        name = name.chars().take(chars - 1).collect::<String>() + ".";
                    }
                    frame.fill_text(Text {
                        content: name,
                        position: Point::new(x0.max(0.0) + 4.0, r.y + 2.5),
                        color: Color::WHITE,
                        size: Pixels(9.5),
                        ..Text::default()
                    });
                }
                if selected {
                    frame.stroke(&rect, Stroke::default().with_width(1.5).with_color(Color::WHITE));
                }
            }
        }
        if empty {
            let v1 = self.layout.iter().find(|r| r.kind == RowKind::Track(TrackRef::video(0)));
            if let Some(r) = v1 {
                frame.fill_text(Text {
                    content: "Press , (insert) or . (overwrite) to edit the Source clip in at the playhead".into(),
                    position: Point::new(w / 2.0, r.y + r.height / 2.0),
                    color: TEXT_MUTED,
                    size: Pixels(TEXT_SIZE_SMALL),
                    align_x: iced::alignment::Horizontal::Center.into(),
                    align_y: Vertical::Center,
                    ..Text::default()
                });
            }
        }

        // Ruler: In/Out range, ticks, labels.
        frame.fill_rectangle(Point::ORIGIN, Size::new(w, RULER_HEIGHT), BG_PANEL_HEADER);
        if app.seq_in.is_some() || app.seq_out.is_some() {
            let a = self.x_of(app.seq_in.unwrap_or(0)).max(0.0);
            let b = self.x_of(app.seq_out.unwrap_or(seq.duration())).min(w);
            let end = self.layout.last().map_or(bounds.height, |r| r.y + r.height);
            frame.fill_rectangle(Point::new(a, 0.0), Size::new((b - a).max(1.0), RULER_HEIGHT), Color { a: 0.35, ..ACCENT_PRIMARY });
            frame.fill_rectangle(Point::new(a, RULER_HEIGHT), Size::new((b - a).max(1.0), end - RULER_HEIGHT), Color { a: 0.07, ..ACCENT_PRIMARY });
        }
        let pps = app.px_per_second;
        let step = label_step(pps, 90.0);
        let minor = step / 5.0;
        let first = (to_seconds(app.scroll) as f32 / minor).floor() as i64;
        let count = (w / (minor * pps)).ceil() as i64 + 1;
        let mut ticks = canvas::path::Builder::new();
        for n in first..=first + count {
            let t = n as f32 * minor;
            let x = self.x_of(from_seconds(t as f64)).round() + 0.5;
            let major = n % 5 == 0;
            ticks.move_to(Point::new(x, if major { 12.0 } else { 19.0 }));
            ticks.line_to(Point::new(x, RULER_HEIGHT));
            if major {
                frame.fill_text(Text {
                    content: seq.rate.timecode(from_seconds(t as f64)),
                    position: Point::new(x + 3.0, 2.0),
                    color: TEXT_SECONDARY,
                    size: Pixels(8.0),
                    ..Text::default()
                });
            }
        }
        frame.stroke(&ticks.build(), Stroke::default().with_width(1.0).with_color(TEXT_MUTED));

        let end = self.layout.last().map_or(bounds.height, |r| r.y + r.height);
        if let Some(t) = state.snapped {
            let x = self.x_of(t).round();
            frame.fill_rectangle(Point::new(x, RULER_HEIGHT), Size::new(1.0, end - RULER_HEIGHT), SNAP_LINE);
        }

        // Playhead.
        let x = self.x_of(app.playhead).round();
        frame.fill_rectangle(Point::new(x, 8.0), Size::new(1.0, end - 8.0), ACCENT_PRIMARY);
        frame.fill(&Path::rounded_rectangle(Point::new(x - 5.0, 2.0), Size::new(11.0, 12.0), 2.0.into()), ACCENT_PRIMARY);

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, state: &State, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        match &state.drag {
            Some(Drag::Pan { .. }) => return mouse::Interaction::Grabbing,
            Some(Drag::Trim { .. } | Drag::Slip { .. }) => return mouse::Interaction::ResizingHorizontally,
            Some(Drag::Move { .. }) => return mouse::Interaction::Grabbing,
            _ => {}
        }
        let Some(p) = cursor.position_in(bounds) else { return mouse::Interaction::default() };
        match (self.app.tool, self.hit(p)) {
            (_, Hit::Ruler) => mouse::Interaction::Pointer,
            (Tool::Hand, _) => mouse::Interaction::Grab,
            (Tool::Razor, Hit::Clip { .. }) => mouse::Interaction::Crosshair,
            (Tool::Selection | Tool::RippleEdit, Hit::Clip { edge: Some(_), .. }) => mouse::Interaction::ResizingHorizontally,
            (Tool::Slip, Hit::Clip { .. }) => mouse::Interaction::ResizingHorizontally,
            _ => mouse::Interaction::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_stack_v3_to_v1_divider_a1_to_a3_then_mix() {
        let seq = Sequence::new("s", jd_timeline::FrameRate::new(25, 1), (1920, 1080), 3, 3);
        let r = rows(&seq);
        let kinds: Vec<_> = r.iter().map(|r| r.kind).collect();
        use RowKind::*;
        assert_eq!(
            kinds,
            [
                Track(TrackRef::video(2)),
                Track(TrackRef::video(1)),
                Track(TrackRef::video(0)),
                Divider,
                Track(TrackRef::audio(0)),
                Track(TrackRef::audio(1)),
                Track(TrackRef::audio(2)),
                Mix
            ]
        );
        assert_eq!(r[0].y, RULER_HEIGHT);
        for pair in r.windows(2) {
            assert_eq!(pair[0].y + pair[0].height, pair[1].y);
        }
    }

    #[test]
    fn ruler_labels_stay_readable_at_any_zoom() {
        assert_eq!(label_step(24.0, 90.0), 5.0);
        assert_eq!(label_step(100.0, 90.0), 1.0);
        assert_eq!(label_step(1.0, 90.0), 120.0);
        assert_eq!(label_step(1000.0, 90.0), 0.2);
    }
}
