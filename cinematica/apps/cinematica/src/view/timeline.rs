//! Timeline panel: sequence tabs, toolbar, track headers (widgets) beside a canvas with the
//! time ruler, track lanes and the playhead. Both sides share one vertical layout ([`rows`]).

use iced::alignment::Vertical;
use iced::widget::canvas::{self, Frame, Geometry, Path, Text};
use iced::widget::{Canvas, button, column, container, row, space, text};
use iced::{Color, Element, Fill, Padding, Pixels, Point, Rectangle, Renderer, Size, Theme, mouse};
use jd_ui::widget::{bold, label, panel_header};
use jd_ui::{Icon, icon, style, tokens::*};

use super::{panel, toggle_button};
use crate::app::{Editor, Message, Track, TrackKind, TrackToggle, timecode};

const HEADER_WIDTH: f32 = 150.0;
const RULER_HEIGHT: f32 = 24.0;
const DIVIDER_HEIGHT: f32 = 6.0;
const MIX_HEIGHT: f32 = 30.0;

const LANE: Color = Color::from_rgb8(0x1E, 0x1E, 0x1E);
const LANE_MIX: Color = Color::from_rgb8(0x18, 0x18, 0x18);
const ROW_LINE: Color = Color::from_rgb8(0x2B, 0x2B, 0x2B);

#[derive(Debug, Clone, Copy, PartialEq)]
enum RowKind {
    Track(usize),
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

/// Vertical layout below the ruler: video tracks, divider, audio tracks, mix.
fn rows(tracks: &[Track]) -> Vec<Row> {
    let mut out = Vec::new();
    let mut y = RULER_HEIGHT;
    let mut push = |kind, height| {
        out.push(Row { kind, y, height });
        y += height;
    };
    for (i, t) in tracks.iter().enumerate() {
        if t.kind == TrackKind::Audio && i > 0 && tracks[i - 1].kind == TrackKind::Video {
            push(RowKind::Divider, DIVIDER_HEIGHT);
        }
        push(RowKind::Track(i), t.height);
    }
    push(RowKind::Mix, MIX_HEIGHT);
    out
}

pub fn view(app: &Editor) -> Element<'_, Message> {
    let f = app.format;
    let toolbar = row![
        text(timecode(app.playhead, f.fps)).size(16).font(bold()).color(ACCENT_PRIMARY),
        space().width(10),
        toggle_button(Icon::Magnet, app.snap, "Snap in Timeline (S)", Some(Message::ToggleSnap)),
        toggle_button(Icon::Link, app.linked, "Linked Selection", Some(Message::ToggleLinked)),
        toggle_button(Icon::Marker, false, "Add Marker (M)", None),
        toggle_button(Icon::Wrench, false, "Timeline Display Settings", None),
        space::horizontal(),
        label(format!("{} × {}  ·  {} fps  ·  {} kHz", f.width, f.height, f.fps, f.sample_rate / 1000)),
    ]
    .spacing(4)
    .align_y(Vertical::Center)
    .padding(Padding::from([0, 10]))
    .height(34);

    let layout = rows(&app.tracks);
    let mut headers = column![container(space()).width(Fill).height(RULER_HEIGHT).style(style::fill(BG_PANEL_HEADER))];
    for r in &layout {
        headers = headers.push(match r.kind {
            RowKind::Track(i) => track_header(i, &app.tracks[i], r.height),
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

    let lanes = Canvas::new(Lanes { layout, playhead: app.playhead as f32, px_per_second: app.px_per_second, fps: f.fps })
        .width(Fill)
        .height(Fill);
    let body = row![
        container(headers).width(HEADER_WIDTH).height(Fill).style(style::fill(Color::from_rgb8(0x1A, 0x1A, 0x1A))),
        container(space()).width(1).height(Fill).style(style::fill(BORDER_DEFAULT)),
        lanes,
    ];

    panel(column![panel_header(&["Sequence 01"], app.sequence_tab, Message::SequenceTab), toolbar, body])
        .width(Fill)
        .height(Fill)
        .into()
}

/// Small on/off button showing text (track target, mute, solo).
fn pill<'a>(s: &'a str, width: f32, on: bool, on_color: Color, msg: Message) -> Element<'a, Message> {
    button(container(text(s).size(9).font(bold()).color(if on { BG_INPUT } else { TEXT_SECONDARY })).center(Fill))
        .width(width)
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

fn track_header(i: usize, t: &Track, height: f32) -> Element<'_, Message> {
    let toggle = move |what| Message::ToggleTrack(i, what);
    // Targeted tracks are white-on-blue, like Premiere's track targeting.
    let target = button(
        container(text(&t.name).size(9).font(bold()).color(if t.targeted { Color::WHITE } else { TEXT_SECONDARY }))
            .center(Fill),
    )
    .width(24)
    .height(16)
    .padding(0)
    .style({
        let on = t.targeted;
        move |_, status| button::Style {
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
        }
    })
    .on_press(toggle(TrackToggle::Target));

    let mut r = row![small_icon(Icon::Lock, t.locked, toggle(TrackToggle::Lock)), target]
        .spacing(4)
        .align_y(Vertical::Center);
    if height >= 50.0 {
        let name = match t.kind {
            TrackKind::Video => format!("Video {}", &t.name[1..]),
            TrackKind::Audio => format!("Audio {}", &t.name[1..]),
        };
        r = r.push(text(name).size(10).color(TEXT_SECONDARY));
    }
    r = r.push(space::horizontal());
    match t.kind {
        TrackKind::Video => r = r.push(small_icon(Icon::Eye, t.enabled, toggle(TrackToggle::Enabled))),
        TrackKind::Audio => {
            r = r
                .push(pill("M", 16.0, !t.enabled, WARNING, toggle(TrackToggle::Enabled)))
                .push(pill("S", 16.0, t.solo, SUCCESS, toggle(TrackToggle::Solo)))
                .push(icon(Icon::Mic, 12.0, TEXT_MUTED));
        }
    }
    container(r)
        .padding(Padding::from([0, 6]))
        .width(Fill)
        .height(height - 1.0)
        .align_y(Vertical::Center)
        .style(style::fill(if t.locked { Color::from_rgb8(0x2A, 0x24, 0x24) } else { BG_PANEL_HEADER }))
        .into()
}

struct Lanes {
    layout: Vec<Row>,
    playhead: f32,
    px_per_second: f32,
    fps: u32,
}

/// Seconds between ruler labels: the smallest "nice" step at least `min_px` apart.
fn label_step(px_per_second: f32, min_px: f32) -> f32 {
    const STEPS: &[f32] = &[1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0];
    STEPS.iter().copied().find(|s| s * px_per_second >= min_px).unwrap_or(600.0)
}

impl canvas::Program<Message> for Lanes {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let w = bounds.width;
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), Color::from_rgb8(0x1A, 0x1A, 0x1A));

        // Lanes.
        for r in &self.layout {
            let (color, line) = match r.kind {
                RowKind::Track(_) => (LANE, true),
                RowKind::Divider => (BORDER_DEFAULT, false),
                RowKind::Mix => (LANE_MIX, false),
            };
            frame.fill_rectangle(Point::new(0.0, r.y), Size::new(w, r.height), color);
            if line {
                frame.fill_rectangle(Point::new(0.0, r.y + r.height - 1.0), Size::new(w, 1.0), ROW_LINE);
            }
        }
        if let Some(first) = self.layout.first() {
            let video_end = self.layout.iter().find(|r| r.kind == RowKind::Divider).map_or(first.y, |r| r.y);
            frame.fill_text(Text {
                content: "Drag clips here from the Project or Source panel".into(),
                position: Point::new(w / 2.0, (first.y + video_end) / 2.0),
                color: TEXT_MUTED,
                size: Pixels(TEXT_SIZE_SMALL),
                align_x: iced::alignment::Horizontal::Center.into(),
                align_y: Vertical::Center,
                ..Text::default()
            });
        }

        // Ruler: a tick per second (half-seconds when zoomed in), labels at a readable step.
        frame.fill_rectangle(Point::ORIGIN, Size::new(w, RULER_HEIGHT), BG_PANEL_HEADER);
        let pps = self.px_per_second;
        let step = label_step(pps, 90.0);
        let minor = if pps >= 40.0 { 0.5 } else if pps >= 8.0 { 1.0 } else { step / 5.0 };
        let mut ticks = canvas::path::Builder::new();
        let count = (w / (minor * pps)).ceil() as i64;
        for n in 0..=count {
            let t = n as f32 * minor;
            let x = (t * pps).round() + 0.5;
            let major = (t / step).fract().abs() < 1e-4;
            ticks.move_to(Point::new(x, if major { 12.0 } else { 19.0 }));
            ticks.line_to(Point::new(x, RULER_HEIGHT));
            if major {
                frame.fill_text(Text {
                    content: timecode(t as f64, self.fps),
                    position: Point::new(x + 3.0, 2.0),
                    color: TEXT_SECONDARY,
                    size: Pixels(8.0),
                    ..Text::default()
                });
            }
        }
        frame.stroke(&ticks.build(), canvas::Stroke::default().with_width(1.0).with_color(TEXT_MUTED));

        // Playhead.
        let end = self.layout.last().map_or(bounds.height, |r| r.y + r.height);
        let x = (self.playhead * pps).round();
        frame.fill_rectangle(Point::new(x, 8.0), Size::new(1.0, end - 8.0), ACCENT_PRIMARY);
        frame.fill(&Path::rounded_rectangle(Point::new(x - 5.0, 2.0), Size::new(11.0, 12.0), 2.0.into()), ACCENT_PRIMARY);

        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_put_a_divider_between_video_and_audio_and_mix_last() {
        let (app, _) = Editor::new();
        let r = rows(&app.tracks);
        let kinds: Vec<_> = r.iter().map(|r| r.kind).collect();
        use RowKind::*;
        assert_eq!(kinds, [Track(0), Track(1), Track(2), Divider, Track(3), Track(4), Track(5), Mix]);
        assert_eq!(r[0].y, RULER_HEIGHT);
        // Rows are stacked without gaps.
        for pair in r.windows(2) {
            assert_eq!(pair[0].y + pair[0].height, pair[1].y);
        }
    }

    #[test]
    fn ruler_labels_stay_readable_when_zoomed_out() {
        assert_eq!(label_step(24.0, 90.0), 5.0);
        assert_eq!(label_step(100.0, 90.0), 1.0);
        assert_eq!(label_step(1.0, 90.0), 120.0);
    }
}
