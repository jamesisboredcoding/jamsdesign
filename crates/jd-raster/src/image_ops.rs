//! Whole-document operations: crop, canvas size, image size, rotate/flip, trim, merging.

use rayon::prelude::*;

use crate::blend;
use crate::composite::{self, BlendSpace};
use crate::selection::{self, Selection};
use crate::transform::{self, content_bounds};
use crate::{BlendMode, Document, Layer, TileGrid};

/// Where the old image sits when the canvas grows or shrinks: (0, 0) top-left, (0.5, 0.5)
/// centre, (1, 1) bottom-right.
pub type Anchor = (f32, f32);

impl Document {
    fn map_layers(&mut self, w: u32, h: u32, f: impl Fn(&TileGrid) -> TileGrid + Sync) {
        let grids: Vec<(TileGrid, Option<TileGrid>)> =
            self.layers().par_iter().map(|l| (f(&l.pixels), l.mask.as_ref().map(|m| f(&m.pixels)))).collect();
        for (layer, (grid, mask)) in self.layers_mut().iter_mut().zip(grids) {
            layer.pixels = grid;
            if let (Some(m), Some(g)) = (layer.mask.as_mut(), mask) {
                m.pixels = g;
            }
        }
        self.width = w;
        self.height = h;
        self.selection = None;
    }

    /// Crops to x0, y0, x1, y1 (exclusive), clipped to the canvas.
    pub fn crop(&mut self, [x0, y0, x1, y1]: [u32; 4]) {
        let (x1, y1) = (x1.min(self.width), y1.min(self.height));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        self.map_layers(x1 - x0, y1 - y0, |g| transform::place(g, x1 - x0, y1 - y0, -(x0 as i64), -(y0 as i64)));
    }

    /// Resizes the canvas without scaling pixels. New area on a locked background layer is
    /// filled with `fill` (Photoshop uses the background colour); other layers stay transparent.
    pub fn canvas_size(&mut self, w: u32, h: u32, anchor: Anchor, fill: [u8; 3]) {
        let dx = ((w as f32 - self.width as f32) * anchor.0).round() as i64;
        let dy = ((h as f32 - self.height as f32) * anchor.1).round() as i64;
        let backgrounds: Vec<bool> = self.layers().iter().enumerate().map(|(i, l)| i == 0 && l.locked).collect();
        let grids: Vec<TileGrid> = self
            .layers()
            .par_iter()
            .zip(backgrounds.par_iter())
            .map(|(l, &bg)| {
                let moved = transform::place(&l.pixels, w, h, dx, dy);
                if !bg {
                    return moved;
                }
                let mut out = TileGrid::empty(w, h);
                selection::fill(&mut out, &Selection::all(w, h), fill);
                transform::composite_over(&mut out, &moved);
                out
            })
            .collect();
        for (layer, grid) in self.layers_mut().iter_mut().zip(grids) {
            layer.pixels = grid;
            // New canvas area reads as the mask's default.
            if let Some(m) = layer.mask.as_mut() {
                m.pixels = transform::place(&m.pixels, w, h, dx, dy);
            }
        }
        self.width = w;
        self.height = h;
        self.selection = None;
    }

    /// Scales every layer to `w`×`h` (Image Size).
    pub fn resize_image(&mut self, w: u32, h: u32) {
        self.map_layers(w, h, |g| transform::resize(g, w, h));
    }

    /// Rotates the canvas by quarter turns clockwise (1 = 90° CW, 2 = 180°, 3 = 90° CCW).
    pub fn rotate_canvas(&mut self, quarter_turns: u32) {
        let (w, h) = (self.width, self.height);
        match quarter_turns % 4 {
            1 => self.map_layers(h, w, |g| transform::remap(g, h, w, |x, y| (y, h - 1 - x))),
            2 => self.map_layers(w, h, |g| transform::remap(g, w, h, |x, y| (w - 1 - x, h - 1 - y))),
            3 => self.map_layers(h, w, |g| transform::remap(g, h, w, |x, y| (w - 1 - y, x))),
            _ => {}
        }
    }

    pub fn flip_canvas(&mut self, horizontal: bool) {
        let (w, h) = (self.width, self.height);
        self.map_layers(w, h, |g| {
            transform::remap(g, w, h, |x, y| if horizontal { (w - 1 - x, y) } else { (x, h - 1 - y) })
        });
    }

    /// Crops away fully transparent borders (of all visible layers). Returns whether it changed.
    pub fn trim(&mut self) -> bool {
        let bounds = self
            .layers()
            .iter()
            .filter(|l| l.visible)
            .filter_map(|l| content_bounds(&l.rendered()))
            .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
        match bounds {
            Some(b) if b != [0, 0, self.width, self.height] => {
                self.crop(b);
                true
            }
            _ => false,
        }
    }

    /// Merges the active layer into the one below (Ctrl+E). The result keeps the lower
    /// layer's name, opacity and blend mode, like Photoshop.
    pub fn merge_down(&mut self) -> bool {
        let Some(i) = self.active_id().and_then(|id| self.layers().iter().position(|l| l.id == id)) else { return false };
        if i == 0 {
            return false;
        }
        let (lower, upper) = (&self.layers()[i - 1], &self.layers()[i]);
        let base = Layer { opacity: 1.0, blend: BlendMode::Normal, visible: true, ..lower.clone() };
        let top = Layer { visible: upper.visible, ..upper.clone() };
        let merged = merge(&[&base, &top], self.blend_space, self.width, self.height, None);
        let (lower_id, upper_id) = (lower.id, upper.id);
        if let Some(l) = self.layer_mut(lower_id) {
            l.pixels = merged;
            l.style = Default::default();
            l.mask = None;
        }
        self.remove_layer(upper_id);
        self.set_active(lower_id);
        true
    }

    /// Merges all visible layers into one (Shift+Ctrl+E); hidden layers stay.
    pub fn merge_visible(&mut self) -> bool {
        let visible: Vec<&Layer> = self.layers().iter().filter(|l| l.visible).collect();
        if visible.len() < 2 {
            return false;
        }
        let merged = merge(&visible, self.blend_space, self.width, self.height, None);
        let keep = visible[0].id;
        let drop: Vec<_> = visible[1..].iter().map(|l| l.id).collect();
        for id in drop {
            self.remove_layer(id);
        }
        if let Some(l) = self.layer_mut(keep) {
            l.pixels = merged;
            l.style = Default::default();
            l.mask = None;
            l.opacity = 1.0;
            l.blend = BlendMode::Normal;
        }
        self.set_active(keep);
        true
    }

    /// Flattens to one locked "Background" over white; hidden layers are discarded.
    pub fn flatten(&mut self) {
        let visible: Vec<&Layer> = self.layers().iter().filter(|l| l.visible).collect();
        let merged = merge(&visible, self.blend_space, self.width, self.height, Some([255, 255, 255]));
        let (name, w, h) = (self.name.clone(), self.width, self.height);
        let mut flat = Document::new(name, w, h);
        flat.blend_space = self.blend_space;
        let bg = flat.layers()[0].id;
        if let Some(l) = flat.layer_mut(bg) {
            l.pixels = merged;
        }
        *self = flat;
    }
}

/// Composites `layers` (bottom to top, honouring visibility, opacity and blend modes) into
/// one grid. With `matte`, the result is flattened over that colour (opaque).
pub fn merge(layers: &[&Layer], space: BlendSpace, w: u32, h: u32, matte: Option<[u8; 3]>) -> TileGrid {
    // Effects are baked in, like Photoshop's merge.
    let grids: Vec<TileGrid> = layers.iter().map(|l| l.rendered().into_owned()).collect();
    let bounds = layers
        .iter()
        .zip(&grids)
        .filter(|(l, _)| l.visible)
        .filter_map(|(_, g)| content_bounds(g))
        .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
    let [x0, y0, x1, y1] = if matte.is_some() { [0, 0, w, h] } else { bounds.unwrap_or([0, 0, 0, 0]) };
    let mut out = TileGrid::empty(w, h);
    if x0 >= x1 || y0 >= y1 {
        return out;
    }
    let rw = (x1 - x0) as usize;
    let rows: Vec<Vec<u8>> = (y0..y1)
        .into_par_iter()
        .map(|y| {
            let mut row = vec![0u8; rw * 4];
            for (i, x) in (x0..x1).enumerate() {
                let mut acc = match matte {
                    Some(c) => composite::to_working([c[0], c[1], c[2], 255], space),
                    None => [0.0; 4],
                };
                for (li, layer) in layers.iter().enumerate() {
                    if !layer.visible || layer.opacity <= 0.0 {
                        continue;
                    }
                    let px = grids[li].pixel(x, y);
                    if px[3] == 0 {
                        continue;
                    }
                    let mut s = composite::to_working(px, space);
                    for c in &mut s {
                        *c *= layer.opacity;
                    }
                    if layer.blend == BlendMode::Dissolve {
                        if blend::dissolve_noise(x, y, composite::dissolve_seed(li)) >= s[3] {
                            continue;
                        }
                        let a = s[3];
                        s = [s[0] / a, s[1] / a, s[2] / a, 1.0];
                    }
                    acc = blend::composite(layer.blend, acc, s);
                }
                row[i * 4..i * 4 + 4].copy_from_slice(&composite::to_srgb8(acc, space));
            }
            row
        })
        .collect();
    out.write_region(x0, y0, rw as u32, y1 - y0, &rows.concat());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Document {
        let px: Vec<u8> = (0..40 * 20).flat_map(|i| [(i % 40) as u8, (i / 40) as u8, 0, 255]).collect();
        Document::from_rgba8("t", 40, 20, &px)
    }

    #[test]
    fn crop_rotate_flip() {
        let mut d = doc();
        d.crop([10, 5, 30, 15]);
        assert_eq!((d.width, d.height), (20, 10));
        assert_eq!(d.layers()[0].pixels.pixel(0, 0), [10, 5, 0, 255]);

        let mut r = doc();
        r.rotate_canvas(1);
        assert_eq!((r.width, r.height), (20, 40));
        // After 90° CW the old bottom-left (0, 19) is the new top-left.
        assert_eq!(r.layers()[0].pixels.pixel(0, 0), [0, 19, 0, 255]);
        r.rotate_canvas(3);
        assert_eq!(r.layers()[0].pixels.pixel(7, 3), doc().layers()[0].pixels.pixel(7, 3));

        let mut f = doc();
        f.flip_canvas(true);
        assert_eq!(f.layers()[0].pixels.pixel(0, 2), [39, 2, 0, 255]);
    }

    #[test]
    fn canvas_size_fills_background_and_anchors() {
        let mut d = doc();
        d.canvas_size(60, 20, (0.5, 0.5), [200, 200, 200]);
        assert_eq!(d.layers()[0].pixels.pixel(0, 0), [200, 200, 200, 255]);
        assert_eq!(d.layers()[0].pixels.pixel(10, 0), [0, 0, 0, 255]);
    }

    #[test]
    fn trim_and_merges() {
        let mut d = Document::new("t", 50, 50);
        let top = d.add_layer("a");
        d.layer_mut(top).unwrap().pixels.set_pixel(10, 20, [255, 0, 0, 255]);
        assert!(d.trim());
        assert_eq!((d.width, d.height), (1, 1));

        let mut m = Document::from_rgba8("m", 2, 1, &[100, 100, 100, 255, 100, 100, 100, 255]);
        let up = m.add_layer("up");
        let l = m.layer_mut(up).unwrap();
        l.pixels.set_pixel(0, 0, [200, 200, 200, 255]);
        l.blend = BlendMode::Multiply;
        let before = composite::flatten(&m);
        assert!(m.merge_down());
        assert_eq!(m.layers().len(), 1);
        assert_eq!(composite::flatten(&m), before, "merging doesn't change the image");

        let mut f = Document::new("f", 2, 1);
        let x = f.add_layer("x");
        f.layer_mut(x).unwrap().pixels.set_pixel(0, 0, [0, 0, 255, 128]);
        f.flatten();
        assert_eq!(f.layers().len(), 1);
        assert_eq!(f.layers()[0].pixels.pixel(1, 0), [255, 255, 255, 255], "flattened over white");
        assert!(f.layers()[0].locked);
    }

    #[test]
    fn resize_image_scales_dimensions() {
        let mut d = doc();
        d.resize_image(20, 10);
        assert_eq!((d.width, d.height, d.layers()[0].pixels.width()), (20, 10, 20));
    }
}
