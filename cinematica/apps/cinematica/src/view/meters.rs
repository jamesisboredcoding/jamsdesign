//! Audio meters: stereo peak meters with a dB scale (silent until playback exists, M3).

use iced::widget::canvas::{self, Frame, Geometry, Text, gradient};
use iced::widget::{Canvas, column, container, row, text};
use iced::{Color, Element, Fill, Padding, Pixels, Point, Rectangle, Renderer, Size, Theme, mouse};
use jd_ui::widget::bold;
use jd_ui::tokens::*;

use super::panel;
use crate::app::{Editor, Message};

/// Lowest level shown, in dB.
const FLOOR_DB: f32 = -54.0;

pub fn view(app: &Editor) -> Element<'_, Message> {
    let solo = |_: u8| {
        container(text("S").size(8).font(bold()).color(TEXT_SECONDARY))
            .center(14)
            .style(|_| container::Style { background: Some(BG_HOVER.into()), border: iced::border::rounded(2), ..Default::default() })
    };
    panel(
        column![
            Canvas::new(Meters { levels_db: app.meters }).width(Fill).height(Fill),
            row![solo(0), solo(1)].spacing(4),
        ]
        .spacing(6)
        .align_x(iced::Alignment::Center)
        .padding(Padding::from([8, 6])),
    )
    .width(50)
    .height(Fill)
    .into()
}

struct Meters {
    /// Current left/right peak, in dBFS.
    levels_db: [f32; 2],
}

/// 0.0 at the floor, 1.0 at 0 dBFS.
fn meter_fraction(db: f32) -> f32 {
    ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

impl canvas::Program<Message> for Meters {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let h = bounds.height;
        for db in (0..=9).map(|i| -6.0 * i as f32) {
            let y = (1.0 - meter_fraction(db)) * (h - 9.0);
            frame.fill_text(Text {
                content: format!("{db:.0}"),
                position: Point::new(0.0, y),
                color: TEXT_MUTED,
                size: Pixels(7.0),
                ..Text::default()
            });
        }
        let fill = gradient::Linear::new(Point::ORIGIN, Point::new(0.0, h))
            .add_stop(0.0, DANGER)
            .add_stop(0.08, WARNING)
            .add_stop(0.25, Color::from_rgb8(0x6C, 0xC4, 0x6A))
            .add_stop(1.0, Color::from_rgb8(0x2F, 0x8A, 0x4A));
        for (i, &db) in self.levels_db.iter().enumerate() {
            let x = bounds.width - 18.0 + i as f32 * 9.0;
            frame.fill_rectangle(Point::new(x, 0.0), Size::new(7.0, h), BG_CANVAS);
            let lit = meter_fraction(db) * h;
            if lit > 0.0 {
                frame.fill_rectangle(Point::new(x, 0.0), Size::new(7.0, h), fill);
                frame.fill_rectangle(Point::new(x, 0.0), Size::new(7.0, h - lit), BG_CANVAS);
            }
        }
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_scale_maps_floor_to_empty_and_zero_db_to_full() {
        assert_eq!(meter_fraction(f32::NEG_INFINITY), 0.0);
        assert_eq!(meter_fraction(FLOOR_DB), 0.0);
        assert_eq!(meter_fraction(0.0), 1.0);
        assert_eq!(meter_fraction(6.0), 1.0);
        assert!((meter_fraction(-27.0) - 0.5).abs() < 1e-6);
    }
}
