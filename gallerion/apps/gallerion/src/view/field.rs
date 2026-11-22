//! Value boxes you can type into (Photoshop's numeric fields): the box shows the value; a
//! click turns it into a text field with the number selected. Enter applies, Esc cancels,
//! and anything else you do applies it first (see `Photo::field_edit`).

use std::ops::RangeInclusive;

use iced::widget::{mouse_area, text_input};
use iced::{Element, Length, Padding};
use jd_ui::tokens::*;
use jd_ui::widget::{field_box, value};

use crate::app::{Message, Photo};

/// Id of the text field while a value box is being edited.
pub const FIELD_INPUT: &str = "value-field";

/// The number in what a box shows ("61%" → "61", "30 px" → "30", "-12.5°" → "-12.5").
pub fn number_text(shown: &str) -> String {
    shown.chars().filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect()
}

/// Parses typed text into a value in `range`; the text is in display units (`scale` times
/// the stored value, e.g. 100 for percentages).
pub fn parse(typed: &str, scale: f32, range: &RangeInclusive<f32>) -> Option<f32> {
    let n: f32 = number_text(typed.trim()).parse().ok()?;
    n.is_finite().then(|| (n / scale).clamp(*range.start(), *range.end()))
}

/// A numeric value box. `shown` is what it displays, `range` the stored value's limits,
/// `scale` display units per stored unit, and `on_value` the message that sets a new value.
pub fn number<'a>(
    app: &'a Photo,
    key: &'static str,
    shown: String,
    width: impl Into<Length>,
    scale: f32,
    range: RangeInclusive<f32>,
    on_value: impl Fn(f32) -> Message + 'a,
) -> Element<'a, Message> {
    custom(app, key, shown, width, move |typed| parse(typed, scale, &range).map(&on_value))
}

/// Like [`number`], but the value applies with every keystroke (Esc puts it back). For boxes
/// next to controls whose messages carry the other settings too (dialog sliders, colour
/// channels, preferences): applying later would be undone by those.
pub fn number_live<'a>(
    app: &'a Photo,
    key: &'static str,
    shown: String,
    width: impl Into<Length>,
    scale: f32,
    range: RangeInclusive<f32>,
    on_value: impl Fn(f32) -> Message + 'a,
) -> Element<'a, Message> {
    custom_live(app, key, shown, width, move |typed| parse(typed, scale, &range).map(&on_value))
}

/// A value box whose typed text is turned into a message by `commit` (`None`: not valid).
pub fn custom<'a>(
    app: &'a Photo,
    key: &'static str,
    shown: String,
    width: impl Into<Length>,
    commit: impl Fn(&str) -> Option<Message> + 'a,
) -> Element<'a, Message> {
    field(app, key, shown, width.into(), false, commit)
}

/// [`custom`], applying as you type (see [`number_live`]).
pub fn custom_live<'a>(
    app: &'a Photo,
    key: &'static str,
    shown: String,
    width: impl Into<Length>,
    commit: impl Fn(&str) -> Option<Message> + 'a,
) -> Element<'a, Message> {
    field(app, key, shown, width.into(), true, commit)
}

fn field<'a>(
    app: &'a Photo,
    key: &'static str,
    shown: String,
    width: Length,
    live: bool,
    commit: impl Fn(&str) -> Option<Message> + 'a,
) -> Element<'a, Message> {
    match &app.field_edit {
        Some(edit) if edit.key == key => text_input("", &edit.text)
            .id(FIELD_INPUT)
            .size(TEXT_SIZE)
            .padding(Padding::from([2, 5]))
            .width(width)
            .on_input(move |t| {
                let pending = commit(&t).map(Box::new);
                Message::FieldInput(t, pending)
            })
            .on_submit(Message::FieldCommit)
            .into(),
        _ => {
            let initial = if key == "hex" { shown.trim_start_matches('#').to_owned() } else { number_text(&shown) };
            // A live box remembers how to put the value back (Esc).
            let press = if live {
                let revert = commit(&initial).map(Box::new);
                Message::FieldEditLive(key, initial, revert)
            } else {
                Message::FieldEdit(key, initial)
            };
            mouse_area(field_box(value(shown)).width(width))
                .on_press(press)
                .interaction(iced::mouse::Interaction::Text)
                .into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_units_and_clamps() {
        assert_eq!(parse("61%", 100.0, &(0.0..=1.0)), Some(0.61));
        assert_eq!(parse(" 250 px", 1.0, &(1.0..=500.0)), Some(250.0));
        assert_eq!(parse("900", 1.0, &(1.0..=500.0)), Some(500.0));
        assert_eq!(parse("-12.5°", 1.0, &(-180.0..=180.0)), Some(-12.5));
        assert_eq!(parse("abc", 1.0, &(0.0..=1.0)), None);
        assert_eq!(number_text("33.33%"), "33.33");
    }
}
