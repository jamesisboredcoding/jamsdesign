//! Text shaping with `cosmic-text`: measuring for layout, and glyph outlines for drawing.
//!
//! Text is drawn from its glyph outlines (unhinted), not from pre-rendered glyph bitmaps, so it
//! stays sharp at any zoom or rotation, takes any paint (gradients, images), and exports to
//! SVG as plain paths.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use cosmic_text::{Attrs, Buffer, CacheKeyFlags, Family, FontSystem, Metrics, Shaping, Style, SwashCache, Weight, Wrap};
use jd_vector::{TextAlign, TextMeasure, TextStyle, TextVAlign};
use kurbo::BezPath;
use swash::zeno::Command;

struct Engine {
    fonts: FontSystem,
    glyphs: SwashCache,
}

/// Loading the system's fonts takes a moment, so it happens once.
static ENGINE: LazyLock<Mutex<Engine>> = LazyLock::new(|| {
    let mut fonts = FontSystem::new();
    let db = fonts.db_mut();
    let loaded: Vec<std::path::PathBuf> = db.faces().filter_map(|f| face_path(&f.source)).collect();
    for dir in flatpak_font_dirs(&loaded) {
        db.load_fonts_dir(dir);
    }
    Mutex::new(Engine { fonts, glyphs: SwashCache::new() })
});

fn face_path(source: &cosmic_text::fontdb::Source) -> Option<std::path::PathBuf> {
    match source {
        cosmic_text::fontdb::Source::File(p) | cosmic_text::fontdb::Source::SharedFile(p, _) => Some(p.clone()),
        _ => None,
    }
}

/// Inside a Flatpak, the host's fonts are mounted under `/run/host`. The runtime's
/// fontconfig lists them, but in case that wasn't followed (`loaded`: files already
/// loaded), these are the folders to load as well.
pub(crate) fn flatpak_font_dirs(loaded: &[std::path::PathBuf]) -> Vec<&'static std::path::Path> {
    if std::env::var_os("FLATPAK_ID").is_none() || loaded.iter().any(|p| p.starts_with("/run/host")) {
        return Vec::new();
    }
    ["/run/host/fonts", "/run/host/local-fonts", "/run/host/user-fonts"].into_iter().map(std::path::Path::new).filter(|d| d.is_dir()).collect()
}

/// Font families installed on this computer, sorted.
pub fn families() -> Vec<String> {
    let Ok(engine) = ENGINE.lock() else { return Vec::new() };
    let mut names: Vec<String> =
        engine.fonts.db().faces().filter_map(|f| f.families.first().map(|(n, _)| n.clone())).collect();
    names.sort();
    names.dedup();
    names
}

/// Weight from a style name like "Semi Bold" or "Black Italic".
pub fn weight_of(style: &str) -> u16 {
    let s = style.to_ascii_lowercase().replace(['-', '_'], " ");
    let has = |w: &str| s.contains(w);
    if has("thin") || has("hairline") {
        100
    } else if has("extra light") || has("ultra light") {
        200
    } else if has("light") {
        300
    } else if has("medium") {
        500
    } else if has("semi bold") || has("semibold") || has("demi bold") {
        600
    } else if has("extra bold") || has("ultra bold") {
        800
    } else if has("bold") {
        700
    } else if has("black") || has("heavy") {
        900
    } else {
        400
    }
}

fn shape(engine: &mut Engine, content: &str, style: &TextStyle, width: Option<f32>) -> Buffer {
    let size = style.size.clamp(1.0, 4000.0);
    let line = (style.line_px() as f32).max(1.0);
    let mut buffer = Buffer::new(&mut engine.fonts, Metrics::new(size, line));
    buffer.set_wrap(&mut engine.fonts, Wrap::WordOrGlyph);
    buffer.set_size(&mut engine.fonts, width, None);
    let lower = style.style.to_ascii_lowercase();
    let mut attrs = Attrs::new()
        .family(if style.family.is_empty() { Family::SansSerif } else { Family::Name(&style.family) })
        .weight(Weight(weight_of(&style.style)))
        .cache_key_flags(CacheKeyFlags::DISABLE_HINTING);
    if lower.contains("italic") || lower.contains("oblique") {
        attrs = attrs.style(Style::Italic);
    }
    if style.letter_spacing != 0.0 {
        // cosmic-text counts letter spacing in ems.
        attrs = attrs.letter_spacing(style.letter_spacing / size);
    }
    buffer.set_text(&mut engine.fonts, content, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(&mut engine.fonts, false);
    buffer
}

/// Width of the widest line and the height of all lines.
fn extent(buffer: &Buffer, style: &TextStyle) -> (f64, f64) {
    let mut w: f32 = 0.0;
    let mut h: f32 = 0.0;
    for run in buffer.layout_runs() {
        w = w.max(run.line_w);
        h = h.max(run.line_top + run.line_height);
    }
    (w as f64, (h as f64).max(style.line_px()))
}

/// Measures with the system's fonts. What Blobr uses for layout once fonts are available.
#[derive(Debug, Clone, Copy, Default)]
pub struct CosmicMeasure;

impl TextMeasure for CosmicMeasure {
    fn measure(&self, content: &str, style: &TextStyle, max_width: Option<f64>) -> (f64, f64) {
        let Ok(mut engine) = ENGINE.lock() else { return (0.0, style.line_px()) };
        let buffer = shape(&mut engine, content, style, max_width.map(|w| w as f32));
        let (w, h) = extent(&buffer, style);
        // Round up so wrapping the text again at this width never moves a word down.
        ((w * 100.0).ceil() / 100.0, h)
    }
}

/// Outlines already made, so redrawing (every pan and zoom step) doesn't shape text again.
static OUTLINES: LazyLock<Mutex<HashMap<String, BezPath>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Every glyph of the text as one path, laid out in a `width` x `height` box (the text node's
/// own space): wrapped at the width, aligned horizontally and vertically.
pub fn text_outline(content: &str, style: &TextStyle, width: f64, height: f64) -> BezPath {
    let key = format!("{width}x{height}\u{0}{style:?}\u{0}{content}");
    if let Ok(cache) = OUTLINES.lock()
        && let Some(p) = cache.get(&key)
    {
        return p.clone();
    }
    let path = shape_outline(content, style, width, height);
    if let Ok(mut cache) = OUTLINES.lock() {
        // Edits make new keys; start over now and then rather than grow forever.
        if cache.len() > 20_000 {
            cache.clear();
        }
        cache.insert(key, path.clone());
    }
    path
}

fn shape_outline(content: &str, style: &TextStyle, width: f64, height: f64) -> BezPath {
    let mut path = BezPath::new();
    let Ok(mut engine) = ENGINE.lock() else { return path };
    // A hair of slack, so text measured at exactly this width doesn't wrap.
    let buffer = shape(&mut engine, content, style, Some(width as f32 + 0.01));
    let (_, text_h) = extent(&buffer, style);
    let oy = match style.valign {
        TextVAlign::Top => 0.0,
        TextVAlign::Center => (height - text_h) / 2.0,
        TextVAlign::Bottom => height - text_h,
    };
    let Engine { fonts, glyphs } = &mut *engine;
    let mut lines = Vec::new();
    for run in buffer.layout_runs() {
        let ox = match style.align {
            TextAlign::Left | TextAlign::Justified => 0.0,
            TextAlign::Center => (width - run.line_w as f64) / 2.0,
            TextAlign::Right => width - run.line_w as f64,
        };
        if style.underline && run.line_w > 0.0 {
            lines.push((ox, oy + run.line_y as f64, run.line_w as f64));
        }
        for g in run.glyphs {
            let key = g.physical((0.0, 0.0), 1.0).cache_key;
            let Some(cmds) = glyphs.get_outline_commands(fonts, key) else { continue };
            let gx = ox + (g.x + g.font_size * g.x_offset) as f64;
            let gy = oy + (run.line_y + g.y - g.font_size * g.y_offset) as f64;
            // Font outlines point up; the page points down.
            let p = |v: swash::zeno::Vector| (gx + v.x as f64, gy - v.y as f64);
            for c in cmds {
                match *c {
                    Command::MoveTo(a) => path.move_to(p(a)),
                    Command::LineTo(a) => path.line_to(p(a)),
                    Command::QuadTo(a, b) => path.quad_to(p(a), p(b)),
                    Command::CurveTo(a, b, d) => path.curve_to(p(a), p(b), p(d)),
                    Command::Close => path.close_path(),
                }
            }
        }
    }
    if !lines.is_empty() {
        // Wound like the glyphs, so where it crosses a descender the nonzero fill keeps both.
        use kurbo::Shape;
        let ccw = path.area() >= 0.0;
        let size = style.size.clamp(1.0, 4000.0) as f64;
        let (gap, thick) = (size * 0.1, (size / 14.0).max(0.5));
        for (x, baseline, w) in lines {
            let (top, bottom) = (baseline + gap, baseline + gap + thick);
            let corners = if ccw { [(x, top), (x, bottom), (x + w, bottom), (x + w, top)] } else { [(x, top), (x + w, top), (x + w, bottom), (x, bottom)] };
            path.move_to(corners[0]);
            for c in &corners[1..] {
                path.line_to(*c);
            }
            path.close_path();
        }
    }
    path
}

/// Whether `family` has a face of this weight (within 100) and slant. A family that isn't
/// installed counts as yes: there's nothing to check against.
// shortcut: a variable font shows as one face, so its other weights read as missing; check its
// weight axis if that comes up.
pub fn has_face(family: &str, weight: u16, italic: bool) -> bool {
    let Ok(engine) = ENGINE.lock() else { return true };
    let faces: Vec<_> = engine.fonts.db().faces().filter(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(family))).collect();
    faces.is_empty()
        || faces.iter().any(|f| f.weight.0.abs_diff(weight) <= 100 && (f.style != cosmic_text::fontdb::Style::Normal) == italic)
}

#[cfg(test)]
mod tests {
    use kurbo::Shape;

    use super::*;

    fn style(size: f32) -> TextStyle {
        TextStyle { family: String::new(), size, ..TextStyle::default() }
    }

    #[test]
    fn underlines_add_a_bar_below_the_text() {
        let s = style(20.0);
        let (w, h) = CosmicMeasure.measure("Hi", &s, None);
        let plain = text_outline("Hi", &s, w, h).bounding_box();
        let under = text_outline("Hi", &TextStyle { underline: true, ..s }, w, h).bounding_box();
        if plain.is_zero_area() {
            return; // no fonts on this machine
        }
        assert!(under.y1 > plain.y1);
    }

    #[test]
    fn style_names_map_to_weights() {
        assert_eq!(weight_of("Regular"), 400);
        assert_eq!(weight_of("Semi Bold"), 600);
        assert_eq!(weight_of("Extra Bold Italic"), 800);
        assert_eq!(weight_of("Bold"), 700);
        assert_eq!(weight_of("Light"), 300);
    }

    #[test]
    fn measuring_grows_with_text_and_wraps() {
        let m = CosmicMeasure;
        let (w1, h1) = m.measure("Hello", &style(20.0), None);
        let (w2, _) = m.measure("Hello there", &style(20.0), None);
        assert!(w1 > 0.0 && w2 > w1, "{w1} {w2}");
        assert_eq!(h1, 24.0, "auto line height is 1.2 em");
        let (w_there, _) = m.measure("there", &style(20.0), None);
        let (_, wrapped) = m.measure("Hello there", &style(20.0), Some(w1.max(w_there) + 1.0));
        assert_eq!(wrapped, 48.0);
        // Measured width is wide enough to lay the text out again on one line.
        let (_, again) = m.measure("Hello there", &style(20.0), Some(w2));
        assert_eq!(again, 24.0);
    }

    #[test]
    fn outlines_sit_inside_the_box_and_follow_alignment() {
        let s = style(20.0);
        let (w, h) = CosmicMeasure.measure("Hi", &s, None);
        let left = text_outline("Hi", &s, 200.0, h).bounding_box();
        assert!(left.x0 >= 0.0 && left.x1 <= w + 0.5 && left.y0 >= 0.0 && left.y1 <= h, "{left:?}");
        let right = text_outline("Hi", &TextStyle { align: TextAlign::Right, ..s.clone() }, 200.0, h).bounding_box();
        assert!((right.x0 - (left.x0 + 200.0 - w)).abs() < 0.6, "{right:?} vs {left:?}");
        let bottom = text_outline("Hi", &TextStyle { valign: TextVAlign::Bottom, ..s }, 200.0, h + 30.0).bounding_box();
        assert!((bottom.y0 - (left.y0 + 30.0)).abs() < 0.01);
    }
}
