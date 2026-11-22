//! `.jdp`, the native Gallerion document format.
//!
//! A zip archive (like OpenRaster or .docx) containing:
//!
//! | entry                  | contents                                                    |
//! |------------------------|-------------------------------------------------------------|
//! | `mimetype`             | `application/x-jamsdesign-photo`, stored first, uncompressed |
//! | `document.json`        | size, blend space, layer list (see [`Manifest`])            |
//! | `layers/<i>/<x>_<y>`   | one tile: 256×256 RGBA8, sRGB, straight alpha, as a raw     |
//! |                        | deflate stream in a *stored* zip entry (see below)          |
//! | `masks/<i>/<x>_<y>`    | a layer mask tile, same encoding (greys; see `jd_raster::mask`) |
//! | `thumbnail.png`        | flattened preview, max 256 px, for file managers            |
//!
//! Tiles are deflated by us, in parallel, rather than by the zip writer (which compresses one
//! entry at a time): on a 12-core machine a 5120×2880 three-layer document saves in a fraction
//! of a second instead of ~3 s. `tile_encoding` in the manifest names the scheme.
//!
//! Empty tiles aren't stored. Unknown fields are ignored when reading, so newer minor versions
//! stay readable; a newer *major* `version` is refused.

use std::io::{Read, Seek, Write};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use rayon::prelude::*;
use std::path::Path;
use std::sync::Arc;

use jd_raster::mask::Mask;
use jd_raster::style::{LayerStyle, StrokePosition};
use jd_raster::text::{Align, TextLayer};
use jd_raster::{BlendMode, BlendSpace, Document, TILE_SIZE, Tile, TileGrid, composite};
use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::{LoadError, SaveError};

pub const EXTENSION: &str = "jdp";
pub const MIME: &str = "application/x-jamsdesign-photo";
const VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
    width: u32,
    height: u32,
    #[serde(default = "default_space")]
    blend_space: String,
    #[serde(default = "default_encoding")]
    tile_encoding: String,
    /// Index into `layers` (bottom to top) of the selected layer.
    #[serde(default)]
    active: Option<usize>,
    layers: Vec<LayerEntry>,
}

const TILE_ENCODING: &str = "deflate";

fn default_encoding() -> String {
    TILE_ENCODING.to_owned()
}

fn default_space() -> String {
    BlendSpace::default().key().to_owned()
}

#[derive(Debug, Serialize, Deserialize)]
struct LayerEntry {
    name: String,
    visible: bool,
    locked: bool,
    opacity: f32,
    blend: String,
    /// Grid positions of stored tiles.
    /// Negative or past the canvas for content kept off it.
    tiles: Vec<[i32; 2]>,
    /// Layer effects; absent in files without any (and in older files).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    style: Option<StyleEntry>,
    /// Editable text (Type tool); the layer's tiles hold the rendered text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<TextEntry>,
    /// Layer mask; its tiles are under `masks/<i>/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mask: Option<MaskEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MaskEntry {
    /// Value wherever no tile is stored: 255 reveals, 0 hides.
    default: u8,
    enabled: bool,
    tiles: Vec<[i32; 2]>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TextEntry {
    text: String,
    family: String,
    size: f32,
    color: [u8; 3],
    #[serde(default)]
    bold: bool,
    #[serde(default)]
    italic: bool,
    #[serde(default)]
    align: String,
    x: f32,
    y: f32,
}

/// Layer effects as stored: each effect keeps its settings even when switched off.
#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(default)]
struct StyleEntry {
    drop_shadow: Option<EffectEntry>,
    outer_glow: Option<EffectEntry>,
    stroke: Option<EffectEntry>,
    color_overlay: Option<EffectEntry>,
}

/// One effect's settings (fields an effect doesn't use are left out).
#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(default)]
struct EffectEntry {
    on: bool,
    color: [u8; 3],
    opacity: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    angle: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    distance: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spread: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    position: Option<String>,
}

impl StyleEntry {
    fn from_style(s: &LayerStyle) -> Option<Self> {
        if *s == LayerStyle::default() {
            return None;
        }
        let (d, g, k, o) = (s.drop_shadow, s.outer_glow, s.stroke, s.color_overlay);
        Some(Self {
            drop_shadow: Some(EffectEntry {
                on: d.on, color: d.color, opacity: d.opacity, angle: Some(d.angle), distance: Some(d.distance),
                spread: Some(d.spread), size: Some(d.size), position: None,
            }),
            outer_glow: Some(EffectEntry {
                on: g.on, color: g.color, opacity: g.opacity, spread: Some(g.spread), size: Some(g.size), ..Default::default()
            }),
            stroke: Some(EffectEntry {
                on: k.on, color: k.color, opacity: k.opacity, size: Some(k.size), position: Some(k.position.key().into()),
                ..Default::default()
            }),
            color_overlay: Some(EffectEntry { on: o.on, color: o.color, opacity: o.opacity, ..Default::default() }),
        })
    }

    fn to_style(&self) -> LayerStyle {
        let mut s = LayerStyle::default();
        let unit = |v: f32| v.clamp(0.0, 1.0);
        let px = |v: f32| v.clamp(0.0, 250.0);
        if let Some(e) = &self.drop_shadow {
            let d = &mut s.drop_shadow;
            (d.on, d.color, d.opacity) = (e.on, e.color, unit(e.opacity));
            d.angle = e.angle.unwrap_or(d.angle);
            d.distance = px(e.distance.unwrap_or(d.distance));
            d.spread = unit(e.spread.unwrap_or(d.spread));
            d.size = px(e.size.unwrap_or(d.size));
        }
        if let Some(e) = &self.outer_glow {
            let g = &mut s.outer_glow;
            (g.on, g.color, g.opacity) = (e.on, e.color, unit(e.opacity));
            g.spread = unit(e.spread.unwrap_or(g.spread));
            g.size = px(e.size.unwrap_or(g.size));
        }
        if let Some(e) = &self.stroke {
            let k = &mut s.stroke;
            (k.on, k.color, k.opacity) = (e.on, e.color, unit(e.opacity));
            k.size = px(e.size.unwrap_or(k.size));
            k.position = e.position.as_deref().map_or(k.position, StrokePosition::from_key);
        }
        if let Some(e) = &self.color_overlay {
            let o = &mut s.color_overlay;
            (o.on, o.color, o.opacity) = (e.on, e.color, unit(e.opacity));
        }
        s
    }
}

fn tile_path(layer: usize, x: i32, y: i32) -> String {
    format!("layers/{layer}/{x}_{y}")
}

fn mask_tile_path(layer: usize, x: i32, y: i32) -> String {
    format!("masks/{layer}/{x}_{y}")
}

fn grid_path(mask: bool, layer: usize, x: i32, y: i32) -> String {
    if mask { mask_tile_path(layer, x, y) } else { tile_path(layer, x, y) }
}

/// True if `bytes` start like a .jdp (zip with our mimetype first).
pub fn sniff(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") && bytes.windows(MIME.len()).take(128).any(|w| w == MIME.as_bytes())
}

pub fn write<W: Write + Seek>(doc: &Document, out: W) -> Result<W, SaveError> {
    let mut zip = ZipWriter::new(out);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zip.start_file("mimetype", stored)?;
    zip.write_all(MIME.as_bytes())?;

    let active = doc.active_id();
    let mut manifest = Manifest {
        format: "jamsdesign-photo".into(),
        version: VERSION,
        width: doc.width,
        height: doc.height,
        blend_space: doc.blend_space.key().into(),
        tile_encoding: TILE_ENCODING.into(),
        active: doc.layers().iter().position(|l| Some(l.id) == active),
        layers: Vec::new(),
    };
    // Compress every tile in parallel, then write them in order.
    let jobs: Vec<(String, &[u8])> = doc
        .layers()
        .iter()
        .enumerate()
        .flat_map(|(i, l)| {
            let pixels = l.pixels.iter_all().map(move |(x, y, t)| (tile_path(i, x, y), t.data()));
            let mask = l.mask.iter().flat_map(move |m| m.pixels.iter_all().map(move |(x, y, t)| (mask_tile_path(i, x, y), t.data())));
            pixels.chain(mask)
        })
        .collect();
    let compressed: Vec<std::io::Result<Vec<u8>>> = jobs
        .par_iter()
        .map(|(_, data)| {
            let mut enc = DeflateEncoder::new(Vec::with_capacity(data.len() / 3), Compression::default());
            enc.write_all(data)?;
            enc.finish()
        })
        .collect();
    for ((path, _), bytes) in jobs.iter().zip(compressed) {
        zip.start_file(path.as_str(), stored)?;
        zip.write_all(&bytes?)?;
    }

    for layer in doc.layers() {
        let tiles = layer.pixels.iter_all().map(|(x, y, _)| [x, y]).collect();
        manifest.layers.push(LayerEntry {
            name: layer.name.clone(),
            visible: layer.visible,
            locked: layer.locked,
            opacity: layer.opacity,
            blend: layer.blend.key().into(),
            tiles,
            style: StyleEntry::from_style(&layer.style),
            // Only text that still matches the pixels (not painted over) stays editable.
            text: layer.text.as_ref().filter(|t| t.is_current(&layer.pixels)).map(|t| TextEntry {
                text: t.text.clone(),
                family: t.family.clone(),
                size: t.size,
                color: t.color,
                bold: t.bold,
                italic: t.italic,
                align: t.align.key().into(),
                x: t.x,
                y: t.y,
            }),
            mask: layer.mask.as_ref().map(|m| MaskEntry {
                default: m.default,
                enabled: m.enabled,
                tiles: m.pixels.iter_all().map(|(x, y, _)| [x, y]).collect(),
            }),
        });
    }

    zip.start_file("document.json", deflated)?;
    serde_json::to_writer_pretty(&mut zip, &manifest).map_err(std::io::Error::from)?;

    let (tw, th, px) = composite::thumbnail(doc, 256, None);
    let mut png = Vec::new();
    image::RgbaImage::from_raw(tw, th, px)
        .expect("thumbnail buffer matches its size")
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
    zip.start_file("thumbnail.png", stored)?;
    zip.write_all(&png)?;

    Ok(zip.finish()?)
}

pub fn read<R: Read + Seek>(name: impl Into<String>, input: R) -> Result<Document, LoadError> {
    let mut zip = ZipArchive::new(input).map_err(|e| LoadError::Format(e.to_string()))?;
    let manifest: Manifest = {
        let entry = zip
            .by_name("document.json")
            .map_err(|_| LoadError::Format("missing document.json".into()))?;
        serde_json::from_reader(entry).map_err(|e| LoadError::Format(format!("document.json: {e}")))?
    };
    if manifest.version > VERSION {
        return Err(LoadError::Format(format!(
            "made by a newer version of Gallerion (format v{}, this build reads v{VERSION})",
            manifest.version
        )));
    }
    if manifest.width == 0 || manifest.height == 0 {
        return Err(LoadError::Format("document has zero size".into()));
    }

    if manifest.tile_encoding != TILE_ENCODING {
        return Err(LoadError::Format(format!("unknown tile encoding {:?}", manifest.tile_encoding)));
    }

    // Read compressed tiles in order (zip access is sequential), inflate them in parallel.
    let mut raw = Vec::new();
    for (i, entry) in manifest.layers.iter().enumerate() {
        let mask_tiles = entry.mask.iter().flat_map(|m| m.tiles.iter().map(|t| (true, t)));
        for (is_mask, &[x, y]) in entry.tiles.iter().map(|t| (false, t)).chain(mask_tiles) {
            let mut bytes = Vec::new();
            let path = grid_path(is_mask, i, x, y);
            zip.by_name(&path).map_err(|e| LoadError::Format(format!("{path}: {e}")))?.read_to_end(&mut bytes)?;
            raw.push((i, is_mask, x, y, bytes));
        }
    }
    let expected = (TILE_SIZE * TILE_SIZE * 4) as usize;
    let tiles: Vec<Result<(usize, bool, i32, i32, Tile), LoadError>> = raw
        .into_par_iter()
        .map(|(i, is_mask, x, y, bytes)| {
            let mut data = Vec::with_capacity(expected);
            DeflateDecoder::new(bytes.as_slice()).read_to_end(&mut data)?;
            if data.len() != expected {
                return Err(LoadError::Format(format!("tile {i}/{x}_{y} has the wrong size")));
            }
            Ok((i, is_mask, x, y, Tile::new(data.into_boxed_slice())))
        })
        .collect();
    let empty = || TileGrid::empty(manifest.width, manifest.height);
    let mut grids: Vec<TileGrid> = manifest.layers.iter().map(|_| empty()).collect();
    let mut masks: Vec<Option<Mask>> = manifest
        .layers
        .iter()
        .map(|e| e.mask.as_ref().map(|m| Mask { pixels: empty(), default: m.default, enabled: m.enabled }))
        .collect();
    for tile in tiles {
        let (i, is_mask, x, y, tile) = tile?;
        match (is_mask, masks[i].as_mut()) {
            (true, Some(m)) => m.pixels.set_tile_at(x, y, Some(Arc::new(tile))),
            _ => grids[i].set_tile_at(x, y, Some(Arc::new(tile))),
        }
    }

    let mut doc = Document::blank(name, manifest.width, manifest.height);
    doc.blend_space = BlendSpace::from_key(&manifest.blend_space).unwrap_or_default();
    let mut ids = Vec::new();
    for ((entry, grid), mask) in manifest.layers.iter().zip(grids).zip(masks) {
        let id = doc.add_layer(entry.name.clone());
        let layer = doc.layer_mut(id).expect("just added");
        layer.visible = entry.visible;
        layer.locked = entry.locked;
        layer.opacity = entry.opacity.clamp(0.0, 1.0);
        layer.blend = BlendMode::from_key(&entry.blend).unwrap_or_default();
        layer.pixels = grid;
        layer.mask = mask;
        if let Some(style) = &entry.style {
            layer.style = style.to_style();
        }
        if let Some(t) = &entry.text {
            let mut text = TextLayer::new(t.x, t.y, t.family.clone(), t.size.clamp(1.0, 2000.0), t.color);
            (text.text, text.bold, text.italic, text.align) = (t.text.clone(), t.bold, t.italic, Align::from_key(&t.align));
            // The stored tiles are the rendered text, so it's current as loaded.
            text.signature = jd_raster::text::signature(&layer.pixels);
            layer.text = Some(text);
        }
        ids.push(id);
    }
    if ids.is_empty() {
        doc.add_layer("Background");
    }
    if let Some(&id) = manifest.active.and_then(|i| ids.get(i)) {
        doc.set_active(id);
    }
    Ok(doc)
}

/// Writes `doc` to `path` safely: into a temporary file next to it, synced, then renamed over
/// the target. A crash mid-save leaves the old file intact.
pub fn save(doc: &Document, path: &Path) -> Result<(), SaveError> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{file_name}.saving-{}", std::process::id()));
    let result = (|| {
        let file = std::fs::File::create(&tmp)?;
        let file = write(doc, std::io::BufWriter::new(file))?;
        let file = file.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        std::fs::remove_file(&tmp).ok();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Document {
        let px: Vec<u8> = (0..300 * 270).flat_map(|i| [(i % 256) as u8, 40, 200, 255]).collect();
        let mut doc = Document::from_rgba8("sample", 300, 270, &px);
        doc.blend_space = BlendSpace::Linear;
        let top = doc.add_layer("Glow");
        let l = doc.layer_mut(top).unwrap();
        l.pixels = TileGrid::from_rgba8(300, 270, &[255, 128, 0, 128].repeat(300 * 270));
        l.blend = BlendMode::Screen;
        l.opacity = 0.6;
        l.visible = false;
        let bg = doc.layers()[0].id;
        doc.set_active(bg);
        doc
    }

    #[test]
    fn roundtrip_keeps_text_editable() {
        let mut doc = Document::blank("t", 200, 80);
        let id = doc.add_layer("Hello");
        let mut t = TextLayer::new(10.0, 10.0, String::new(), 30.0, [9, 9, 9]);
        t.text = "Hello".into();
        t.italic = true;
        let l = doc.layer_mut(id).unwrap();
        t.render_into(&mut l.pixels);
        l.text = Some(t);
        let bytes = write(&doc, std::io::Cursor::new(Vec::new())).unwrap().into_inner();
        let back = read("t", std::io::Cursor::new(bytes)).unwrap();
        let l = back.layers().last().unwrap();
        let t = l.text.as_ref().expect("text kept");
        assert_eq!((t.text.as_str(), t.italic, t.size), ("Hello", true, 30.0));
        assert!(t.is_current(&l.pixels));
    }

    #[test]
    fn roundtrip_keeps_layer_styles() {
        let mut doc = Document::blank("t", 50, 50);
        let id = doc.add_layer("fx");
        let mut style = LayerStyle::default();
        style.stroke.on = true;
        style.stroke.size = 7.0;
        style.stroke.position = StrokePosition::Center;
        style.drop_shadow.angle = 45.0;
        doc.layer_mut(id).unwrap().style = style;
        let bytes = write(&doc, std::io::Cursor::new(Vec::new())).unwrap().into_inner();
        let back = read("t", std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(back.layers().last().unwrap().style, style);
    }

    #[test]
    fn roundtrip_keeps_layer_masks() {
        let mut doc = Document::blank("t", 300, 60);
        let id = doc.add_layer("masked");
        let l = doc.layer_mut(id).unwrap();
        let mut mask = Mask::new(300, 60, 255);
        mask.pixels.set_pixel(280, 10, [40, 40, 40, 255]);
        mask.pixels.set_pixel_at(-20, 5, [7, 7, 7, 255]);
        mask.enabled = false;
        l.mask = Some(mask);
        let bytes = write(&doc, std::io::Cursor::new(Vec::new())).unwrap().into_inner();
        let back = read("t", std::io::Cursor::new(bytes)).unwrap();
        let m = back.layers().last().unwrap().mask.as_ref().expect("mask kept");
        assert_eq!((m.default, m.enabled), (255, false));
        assert_eq!((m.value_at(280, 10), m.value_at(0, 0), m.value_at(-20, 5)), (40, 255, 7));
    }

    #[test]
    fn roundtrip_keeps_content_off_the_canvas() {
        let mut doc = Document::blank("t", 100, 100);
        let id = doc.add_layer("moved");
        {
            let px = &mut doc.layer_mut(id).unwrap().pixels;
            px.set_pixel_at(-300, 5, [9, 8, 7, 255]);
            px.set_pixel_at(150, 40, [1, 2, 3, 200]);
        }
        let bytes = write(&doc, std::io::Cursor::new(Vec::new())).unwrap().into_inner();
        let back = read("t", std::io::Cursor::new(bytes)).unwrap();
        let px = &back.layers().last().unwrap().pixels;
        assert_eq!(px.pixel_at(-300, 5), [9, 8, 7, 255]);
        assert_eq!(px.pixel_at(150, 40), [1, 2, 3, 200]);
    }

    #[test]
    fn roundtrip_preserves_everything() {
        let doc = sample();
        let bytes = write(&doc, std::io::Cursor::new(Vec::new())).unwrap().into_inner();
        assert!(sniff(&bytes));
        let back = read("sample", std::io::Cursor::new(bytes)).unwrap();

        assert_eq!((back.width, back.height, back.blend_space), (300, 270, BlendSpace::Linear));
        assert_eq!(back.layers().len(), 2);
        assert_eq!(back.active().unwrap().name, "Background");
        let (a, b) = (&doc.layers()[1], &back.layers()[1]);
        assert_eq!((a.name.as_str(), a.blend, a.opacity, a.visible), (b.name.as_str(), b.blend, b.opacity, b.visible));
        for (x, y) in [(0, 0), (299, 269), (256, 13)] {
            for i in 0..2 {
                assert_eq!(doc.layers()[i].pixels.pixel(x, y), back.layers()[i].pixels.pixel(x, y));
            }
        }
    }

    #[test]
    fn refuses_newer_major_version() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut buf);
            zip.start_file("document.json", SimpleFileOptions::default()).unwrap();
            zip.write_all(br#"{"format":"jamsdesign-photo","version":99,"width":1,"height":1,"layers":[]}"#).unwrap();
            zip.finish().unwrap();
        }
        buf.set_position(0);
        let err = read("x", buf).unwrap_err().to_string();
        assert!(err.contains("newer version"), "{err}");
    }

    #[test]
    fn save_is_atomic_and_leaves_no_temp_files() {
        let dir = std::env::temp_dir().join(format!("jdp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("doc.jdp");
        save(&sample(), &path).unwrap();
        save(&sample(), &path).unwrap();
        let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(entries, ["doc.jdp"]);
        std::fs::remove_dir_all(dir).ok();
    }
}
