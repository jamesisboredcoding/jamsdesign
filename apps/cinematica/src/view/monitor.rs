//! Source and Program monitors: viewer, scrub bar, timecode row and transport controls.

use iced::alignment::Vertical;
use iced::widget::canvas::{self, Frame, Geometry, Path};
use iced::widget::{Canvas, column, container, row, space, text};
use iced::{Element, Fill, Padding, Point, Rectangle, Renderer, Size, Theme, mouse};
use jd_ui::widget::{bold, field_box, icon_button, panel_header, value, vline};
use jd_ui::{Icon, icon, style, tokens::*};

use super::panel;
use crate::app::{Editor, Message, timecode};

const SOURCE_TABS: &[&str] = &["Source: (no clip)", "Effect Controls", "Audio Clip Mixer", "Metadata"];

pub fn source(app: &Editor) -> Element<'_, Message> {
    let body: Element<'_, Message> = match app.source_tab {
        0 => monitor_body(
            "Double-click a clip in the Project panel to view it here",
            None,
            timecode(0.0, app.format.fps),
            timecode(0.0, app.format.fps),
            &[Icon::Insert, Icon::Overwrite],
        ),
        1 => placeholder("Select a clip in the timeline to edit its Motion, Opacity and effects"),
        2 => placeholder("Clip volume and pan per track appear here"),
        _ => placeholder("File details of the source clip appear here"),
    };
    panel(column![panel_header(SOURCE_TABS, app.source_tab, Message::SourceTab), body])
        .width(Fill)
        .height(Fill)
        .into()
}

pub fn program(app: &Editor) -> Element<'_, Message> {
    let body = monitor_body(
        "The sequence plays here",
        Some(0.0),
        timecode(app.playhead, app.format.fps),
        timecode(0.0, app.format.fps),
        &[Icon::Lift, Icon::Extract],
    );
    panel(column![panel_header(&["Program: Sequence 01"], 0, |_| Message::Noop), body])
        .width(Fill)
        .height(Fill)
        .into()
}

fn placeholder<'a>(hint: &'a str) -> Element<'a, Message> {
    container(text(hint).size(TEXT_SIZE_SMALL).color(TEXT_MUTED)).center(Fill).into()
}

/// `head`: playhead position as a fraction of the duration, if there's anything to show.
fn monitor_body<'a>(
    hint: &'a str,
    head: Option<f32>,
    current: String,
    duration: String,
    edit_buttons: &[Icon],
) -> Element<'a, Message> {
    let viewer = container(text(hint).size(TEXT_SIZE_SMALL).color(TEXT_MUTED))
        .center(Fill)
        .style(style::fill(BG_CANVAS));

    let dropdown = |s: &'a str| {
        field_box(row![value(s), icon(Icon::Chevron, 10.0, TEXT_SECONDARY)].spacing(4).align_y(Vertical::Center))
    };
    let timecode_row = row![
        text(current).size(15).font(bold()).color(ACCENT_PRIMARY),
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

    // Transport works once media plays (M1); until then the buttons show dimmed.
    let btn = |i: Icon| -> Element<'a, Message> { icon_button(i, 16.0, false, None).padding(6).into() };
    let mut transport = row![btn(Icon::MarkIn), btn(Icon::MarkOut), vline(18.0, BORDER_SUBTLE)]
        .spacing(4)
        .align_y(Vertical::Center);
    for i in [Icon::GoToIn, Icon::StepBack, Icon::Play, Icon::StepForward, Icon::GoToOut] {
        transport = transport.push(btn(i));
    }
    transport = transport.push(vline(18.0, BORDER_SUBTLE));
    for &i in edit_buttons {
        transport = transport.push(btn(i));
    }
    transport = transport.push(btn(Icon::Camera));

    column![
        viewer,
        Canvas::new(ScrubBar { head }).width(Fill).height(22),
        timecode_row,
        container(transport).center_x(Fill).height(38).align_y(Vertical::Center),
    ]
    .into()
}

struct ScrubBar {
    head: Option<f32>,
}

impl canvas::Program<Message> for ScrubBar {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (left, width) = (12.0, (bounds.width - 24.0).max(0.0));
        let mid = (bounds.height / 2.0).round();
        frame.fill(&Path::rounded_rectangle(Point::new(left, mid - 1.0), Size::new(width, 2.0), 1.0.into()), BORDER_SUBTLE);
        if let Some(h) = self.head {
            let x = (left + width * h.clamp(0.0, 1.0)).round();
            frame.fill_rectangle(Point::new(x, 3.0), Size::new(1.0, bounds.height - 6.0), ACCENT_PRIMARY);
            frame.fill(&Path::rounded_rectangle(Point::new(x - 5.0, 2.0), Size::new(11.0, 6.0), 2.0.into()), ACCENT_PRIMARY);
        }
        vec![frame.into_geometry()]
    }
}
