//! Measuring text for layout.
//!
//! Hugging text and auto layout need to know how big a text block is. The real measurer
//! (shaping with `cosmic-text`, the same as the renderer) arrives with rendering; layout only
//! needs this trait, so the model doesn't depend on fonts being installed.

use crate::TextStyle;

pub trait TextMeasure {
    /// Size of `content` in `style`, wrapped to `max_width` when given.
    fn measure(&self, content: &str, style: &TextStyle, max_width: Option<f64>) -> (f64, f64);
}

/// A rough, font-free measurer: every character is 0.6 em wide (plus letter spacing), lines
/// wrap at spaces. Good enough for tests and for laying out before fonts load.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApproxMeasure;

impl TextMeasure for ApproxMeasure {
    fn measure(&self, content: &str, style: &TextStyle, max_width: Option<f64>) -> (f64, f64) {
        let advance = style.size as f64 * 0.6 + style.letter_spacing as f64;
        let width_of = |s: &str| s.chars().count() as f64 * advance;
        let mut widest: f64 = 0.0;
        let mut lines = 0usize;
        for para in content.split('\n') {
            match max_width {
                None => {
                    widest = widest.max(width_of(para));
                    lines += 1;
                }
                Some(max) => {
                    let mut line = String::new();
                    for word in para.split(' ') {
                        let candidate = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
                        if !line.is_empty() && width_of(&candidate) > max {
                            widest = widest.max(width_of(&line));
                            lines += 1;
                            line = word.to_owned();
                        } else {
                            line = candidate;
                        }
                    }
                    widest = widest.max(width_of(&line));
                    lines += 1;
                }
            }
        }
        (widest, lines.max(1) as f64 * style.line_px())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approx_wraps_at_spaces() {
        let style = TextStyle { size: 10.0, ..TextStyle::default() };
        // 6 px per character, 12 px per line.
        assert_eq!(ApproxMeasure.measure("hello world", &style, None), (66.0, 12.0));
        assert_eq!(ApproxMeasure.measure("hello world", &style, Some(40.0)), (30.0, 24.0));
        assert_eq!(ApproxMeasure.measure("a\nbb", &style, None), (12.0, 24.0));
    }
}
