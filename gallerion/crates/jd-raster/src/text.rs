//! Text layers (the Type tool): shaping and rasterising with cosmic-text (system fonts).
//!
//! A text layer keeps its [`TextLayer`] description and its pixels are the rasterised text, so
//! everything that works on pixels (styles, thumbnails, export, saving) just works. If the
//! pixels are later edited directly, [`TextLayer::is_current`] notices and the layer is
//! treated as plain pixels from then on (Photoshop's "rasterize type").

use std::sync::{LazyLock, Mutex};

use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style, SwashCache, Weight};

use crate::selection::over_srgb8;
use crate::TileGrid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

impl Align {
    pub fn key(self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
        }
    }

    pub fn from_key(k: &str) -> Self {
        match k {
            "center" => Align::Center,
            "right" => Align::Right,
            _ => Align::Left,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextLayer {
    pub text: String,
    /// Font family name; empty means the system's default sans-serif.
    pub family: String,
    /// Font size in pixels.
    pub size: f32,
    pub color: [u8; 3],
    pub bold: bool,
    pub italic: bool,
    pub align: Align,
    /// Where the text block starts: left edge (for left alignment; the centre or right edge
    /// otherwise) and top of the first line, in document pixels.
    pub x: f32,
    pub y: f32,
    /// Tile ids of the pixels this text was last rasterised to (see [`TextLayer::is_current`]).
    pub signature: u64,
}

struct Engine {
    fonts: FontSystem,
    glyphs: SwashCache,
}

/// Loading system fonts takes a moment, so it happens once, on first use.
static ENGINE: LazyLock<Mutex<Engine>> = LazyLock::new(|| Mutex::new(Engine { fonts: FontSystem::new(), glyphs: SwashCache::new() }));

/// Installed font families, sorted (for the font menu).
pub fn families() -> Vec<String> {
    let Ok(engine) = ENGINE.lock() else { return Vec::new() };
    let mut names: Vec<String> = engine.fonts.db().faces().filter_map(|f| f.families.first().map(|(n, _)| n.clone())).collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

/// A quick fingerprint of a grid's tiles: changes whenever any pixel is written.
pub fn signature(grid: &TileGrid) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (x, y, t) in grid.iter_all() {
        for v in [x as i64 as u64, y as i64 as u64, t.id().raw()] {
            h = (h ^ v).wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// The laid-out text: line boxes and where the caret goes at the end.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Layout {
    /// Bounds of the text block: x0, y0, x1, y1 (document px).
    pub bounds: [f32; 4],
    /// Caret after the last character: x, top, height.
    pub caret: (f32, f32, f32),
}

impl TextLayer {
    pub fn new(x: f32, y: f32, family: String, size: f32, color: [u8; 3]) -> Self {
        Self { text: String::new(), family, size, color, bold: false, italic: false, align: Align::Left, x, y, signature: 0 }
    }

    /// Whether `pixels` still are this text as last rasterised (not painted over since).
    pub fn is_current(&self, pixels: &TileGrid) -> bool {
        signature(pixels) == self.signature
    }

    fn shape(&self, engine: &mut Engine) -> Buffer {
        let size = self.size.clamp(1.0, 2000.0);
        let mut buffer = Buffer::new(&mut engine.fonts, Metrics::new(size, size * 1.2));
        buffer.set_size(&mut engine.fonts, None, None);
        let mut attrs = Attrs::new();
        attrs = if self.family.is_empty() { attrs.family(Family::SansSerif) } else { attrs.family(Family::Name(&self.family)) };
        if self.bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        if self.italic {
            attrs = attrs.style(Style::Italic);
        }
        let align = match self.align {
            Align::Left => cosmic_text::Align::Left,
            Align::Center => cosmic_text::Align::Center,
            Align::Right => cosmic_text::Align::Right,
        };
        buffer.set_text(&mut engine.fonts, &self.text, &attrs, Shaping::Advanced, Some(align));
        buffer.shape_until_scroll(&mut engine.fonts, false);
        buffer
    }

    /// Horizontal offset of the block so `x` is its left edge, centre or right edge.
    fn offset_x(&self, width: f32) -> f32 {
        match self.align {
            Align::Left => self.x,
            Align::Center => self.x - width / 2.0,
            Align::Right => self.x - width,
        }
    }

    pub fn layout(&self) -> Layout {
        let Ok(mut engine) = ENGINE.lock() else { return Layout::default() };
        let buffer = self.shape(&mut engine);
        let line_h = self.size.clamp(1.0, 2000.0) * 1.2;
        let width = buffer.layout_runs().map(|r| r.line_w).fold(0.0, f32::max);
        let ox = self.offset_x(width);
        let mut height = 0.0f32;
        let mut caret = (ox, self.y, line_h);
        for run in buffer.layout_runs() {
            height = height.max(run.line_top + line_h);
            let end = run.glyphs.last().map_or(0.0, |g| g.x + g.w);
            // Lines are aligned within the widest one.
            let start = match self.align {
                Align::Left => 0.0,
                Align::Center => (width - run.line_w) / 2.0,
                Align::Right => width - run.line_w,
            };
            let x = if run.glyphs.is_empty() { start } else { end };
            caret = (ox + x, self.y + run.line_top, line_h);
        }
        if self.text.ends_with('\n') {
            let x = match self.align {
                Align::Left => 0.0,
                Align::Center => width / 2.0,
                Align::Right => width,
            };
            caret = (ox + x, self.y + height, line_h);
            height += line_h;
        }
        Layout { bounds: [ox, self.y, ox + width.max(1.0), self.y + height.max(line_h)], caret }
    }

    /// The text drawn into a new `w` x `h` grid (anything past the canvas is kept off it).
    pub fn rasterize(&self, w: u32, h: u32) -> TileGrid {
        let mut out = TileGrid::empty(w, h);
        let Ok(mut engine) = ENGINE.lock() else { return out };
        let buffer = self.shape(&mut engine);
        let width = buffer.layout_runs().map(|r| r.line_w).fold(0.0, f32::max);
        let (ox, oy) = (self.offset_x(width).round() as i64, self.y.round() as i64);
        let [r, g, b] = self.color;
        let Engine { fonts, glyphs } = &mut *engine;
        buffer.draw(fonts, glyphs, Color::rgb(r, g, b), |x, y, gw, gh, c| {
            if c.a() == 0 {
                return;
            }
            for dy in 0..gh as i64 {
                for dx in 0..gw as i64 {
                    let (px, py) = (ox + x as i64 + dx, oy + y as i64 + dy);
                    let d = out.pixel_at(px, py);
                    out.set_pixel_at(px, py, over_srgb8(d, [c.r(), c.g(), c.b(), c.a()]));
                }
            }
        });
        out
    }

    /// Re-renders the text into a layer's pixels and remembers them as current.
    pub fn render_into(&mut self, pixels: &mut TileGrid) {
        *pixels = self.rasterize(pixels.width(), pixels.height());
        self.signature = signature(pixels);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_text_where_it_is_placed_and_tracks_edits() {
        let mut t = TextLayer::new(20.0, 10.0, String::new(), 40.0, [255, 0, 0]);
        t.text = "Hi".into();
        let mut pixels = TileGrid::empty(200, 100);
        t.render_into(&mut pixels);
        let b = crate::transform::content_bounds(&pixels).expect("glyphs drawn");
        assert!(b[0] >= 20 && b[0] < 30 && b[1] >= 10 && b[3] <= 70, "{b:?}");
        let p = (b[1]..b[3]).flat_map(|y| (b[0]..b[2]).map(move |x| (x, y))).map(|(x, y)| pixels.pixel(x, y)).find(|p| p[3] == 255);
        assert_eq!(p.map(|p| [p[0], p[1], p[2]]), Some([255, 0, 0]), "solid red glyph pixels");
        assert!(t.is_current(&pixels));
        pixels.set_pixel(0, 0, [1, 1, 1, 255]);
        assert!(!t.is_current(&pixels), "painting over it makes it plain pixels");
    }

    #[test]
    fn layout_grows_with_lines_and_caret_follows() {
        let mut t = TextLayer::new(0.0, 0.0, String::new(), 20.0, [0, 0, 0]);
        t.text = "abc".into();
        let one = t.layout();
        t.text = "abc\nabcdef".into();
        let two = t.layout();
        assert!(two.bounds[3] > one.bounds[3] + 15.0, "second line");
        assert!(two.bounds[2] > one.bounds[2], "wider");
        assert!(two.caret.1 > one.caret.1 && two.caret.0 > one.caret.0);
        t.text = "abc\n".into();
        assert_eq!(t.layout().caret.0, 0.0, "new empty line starts at the left");
    }

    #[test]
    fn centred_text_is_centred_on_x() {
        let mut t = TextLayer::new(100.0, 0.0, String::new(), 30.0, [0, 0, 0]);
        t.text = "MMMM".into();
        t.align = Align::Center;
        let [x0, _, x1, _] = t.layout().bounds;
        assert!(((x0 + x1) / 2.0 - 100.0).abs() < 1.0, "{x0}..{x1}");
    }
}
