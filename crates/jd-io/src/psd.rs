//! Opening Photoshop `.psd` files: layers with their names, positions, opacity, visibility
//! and blend modes, and layer groups.
//!
//! Through the `psd` crate, which reads 8-bit RGB and greyscale files. Not carried over:
//! layer masks, adjustment layers (they come in empty), layer effects, editable text (text
//! layers come in as their pixels) and clipping masks.

use std::collections::HashMap;

use jd_raster::{BlendMode, Document, Group, LayerId};

use crate::LoadError;

/// Photoshop's blend modes, ours where they match (and the nearest where we have none). The
/// `psd` crate doesn't export its blend mode type, so this goes by the variant's name.
fn blend(mode: impl std::fmt::Debug) -> BlendMode {
    match format!("{mode:?}").as_str() {
        "Dissolve" => BlendMode::Dissolve,
        "Darken" | "DarkerColor" => BlendMode::Darken,
        "Multiply" => BlendMode::Multiply,
        "ColorBurn" => BlendMode::ColorBurn,
        "LinearBurn" => BlendMode::LinearBurn,
        "Lighten" | "LighterColor" => BlendMode::Lighten,
        "Screen" => BlendMode::Screen,
        "ColorDodge" => BlendMode::ColorDodge,
        "LinearDodge" => BlendMode::LinearDodge,
        "Overlay" => BlendMode::Overlay,
        "SoftLight" => BlendMode::SoftLight,
        "HardLight" => BlendMode::HardLight,
        "VividLight" | "HardMix" => BlendMode::VividLight,
        "LinearLight" => BlendMode::LinearLight,
        "PinLight" => BlendMode::PinLight,
        "Difference" => BlendMode::Difference,
        "Exclusion" => BlendMode::Exclusion,
        "Subtract" => BlendMode::Subtract,
        "Divide" => BlendMode::Divide,
        "Hue" => BlendMode::Hue,
        "Saturation" => BlendMode::Saturation,
        "Color" => BlendMode::Color,
        "Luminosity" => BlendMode::Luminosity,
        _ => BlendMode::Normal,
    }
}

/// The `psd` crate reads the layer flag bit Photoshop sets for *hidden* layers and calls it
/// `visible`, so its answer is the wrong way round (psd-tools reads the same bit as hidden).
fn shown(crate_visible: bool) -> bool {
    !crate_visible
}

fn is_pass_through(mode: impl std::fmt::Debug) -> bool {
    format!("{mode:?}") == "PassThrough"
}

/// One row of the layer stack, top first: a group (by its id in the file) or a layer (by
/// its index in `Psd::layers`).
enum Item {
    Group(u32),
    Layer(usize),
}

/// Reads a `.psd` into a document.
pub fn read(name: String, bytes: &[u8]) -> Result<Document, LoadError> {
    // The parser can panic on files it doesn't expect; that's a bad file, not a crash.
    let parsed = std::panic::catch_unwind(|| psd::Psd::from_bytes(bytes))
        .map_err(|_| LoadError::Format("this Photoshop file uses features that can't be read yet".into()))?;
    let file = parsed.map_err(|e| LoadError::Format(format!("couldn't read the Photoshop file: {e}")))?;
    let (w, h) = (file.width(), file.height());
    if w == 0 || h == 0 {
        return Err(LoadError::Format("the Photoshop file has no size".into()));
    }
    let mut doc = Document::blank(name, w, h);

    // A file saved with only a background has no layers, just the flattened image.
    if file.layers().is_empty() {
        let rgba = std::panic::catch_unwind(|| file.rgba()).map_err(|_| LoadError::Format("couldn't read the image data".into()))?;
        let id = doc.add_layer("Background");
        if let Some(l) = doc.layer_mut(id) {
            l.pixels = jd_raster::TileGrid::from_rgba8(w, h, &rgba);
            l.locked = true;
        }
        return Ok(doc);
    }

    // Top-first order: before each layer, the headers of the groups that start there (in the
    // order they were opened, which is top to bottom).
    let mut starts: HashMap<usize, Vec<u32>> = HashMap::new();
    // (Where a group's layers start in `layers()`: the crate only hands out the slice.)
    let base = file.layers().as_ptr() as usize;
    let size = std::mem::size_of::<psd::PsdLayer>().max(1);
    for id in file.groups().keys() {
        let Some(sub) = file.get_group_sub_layers(id) else { continue };
        let start = (sub.as_ptr() as usize).saturating_sub(base) / size;
        starts.entry(start.min(file.layers().len())).or_default().push(*id);
    }
    for ids in starts.values_mut() {
        ids.sort_unstable();
    }
    let mut items = Vec::new();
    for i in 0..=file.layers().len() {
        items.extend(starts.get(&i).into_iter().flatten().map(|&id| Item::Group(id)));
        if i < file.layers().len() {
            items.push(Item::Layer(i));
        }
    }

    // Added bottom to top (each new layer goes above the last).
    let mut ids: HashMap<u32, LayerId> = HashMap::new();
    let mut parents: Vec<(LayerId, Option<u32>)> = Vec::new();
    for item in items.iter().rev() {
        match *item {
            Item::Group(gid) => {
                let g = &file.groups()[&gid];
                let id = doc.add_layer(g.name());
                if let Some(l) = doc.layer_mut(id) {
                    l.group = Some(Group { pass_through: is_pass_through(g.blend_mode()), open: true });
                    l.blend = blend(g.blend_mode());
                    l.opacity = g.opacity() as f32 / 255.0;
                    l.visible = shown(g.visible());
                }
                ids.insert(gid, id);
                parents.push((id, g.parent_id()));
            }
            Item::Layer(i) => {
                let layer = &file.layers()[i];
                let rgba = std::panic::catch_unwind(|| layer.rgba())
                    .map_err(|_| LoadError::Format(format!("couldn't read the pixels of layer \"{}\"", layer.name())))?;
                let id = doc.add_layer(layer.name());
                if let Some(l) = doc.layer_mut(id) {
                    l.pixels = jd_raster::TileGrid::from_rgba8(w, h, &rgba);
                    l.blend = blend(layer.blend_mode());
                    l.opacity = layer.opacity() as f32 / 255.0;
                    l.visible = shown(layer.visible());
                }
                parents.push((id, layer.parent_id()));
            }
        }
    }
    for (id, parent) in parents {
        let parent = parent.and_then(|p| ids.get(&p).copied());
        if let Some(l) = doc.layer_mut(id) {
            l.parent = parent;
        }
    }
    // Photoshop's "Background" (the bottom layer of that name) is locked.
    if let Some(bottom) = doc.layers().first().map(|l| l.id)
        && let Some(l) = doc.layer_mut(bottom)
        && l.name == "Background"
        && l.group.is_none()
        && l.parent.is_none()
    {
        l.locked = true;
    }
    // The top layer is selected.
    if let Some(top) = doc.layers().last().map(|l| l.id) {
        doc.set_active(top);
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_and_nested_groups_keep_their_order() {
        let bytes = include_bytes!("../tests/fixtures/green-1x1-one-group-with-two-subgroups.psd");
        let doc = read("t.psd".into(), bytes).unwrap();
        // Top to bottom, as the Layers panel shows it.
        let names: Vec<(String, usize)> =
            doc.layers().iter().rev().map(|l| (l.name.clone(), doc.depth(l.id))).collect();
        // As the crate's own fixture documents it (Photoshop's panel, top first).
        let expect = [
            ("outside group", 0),
            ("first group inside", 1),
            ("First Layer", 2),
            ("second group inside", 1),
            ("sub sub group", 2),
            ("Second Layer", 3),
            ("Third Layer", 2),
            ("third group inside", 1),
            ("Fourth Layer", 1),
            ("Firth Layer", 0),
            ("outside group 2", 0),
            ("Sixth Layer", 1),
        ];
        // (Printed in full if the order is off.)
        let got: Vec<(&str, usize)> = names.iter().map(|(n, d)| (n.as_str(), *d)).collect();
        for (want, have) in expect.iter().zip(&got) {
            assert_eq!(want, have, "{got:?}");
        }
        assert_eq!(got.len(), expect.len());
        assert_eq!(doc.layers().len(), 12);
        // Every group's contents sit right below it.
        for (i, l) in doc.layers().iter().enumerate() {
            if l.is_group() {
                let block = doc.block(i);
                assert!(doc.layers()[block.clone()].iter().filter(|c| c.id != l.id).all(|c| doc.is_within(c.id, l.id)), "{got:?}");
            }
        }
    }

    #[test]
    fn pixels_opacity_and_flatten() {
        let bytes = include_bytes!("../tests/fixtures/two-layers-red-green-1x1.psd");
        let doc = read("t.psd".into(), bytes).unwrap();
        assert_eq!((doc.width, doc.height, doc.layers().len()), (1, 1, 2));
        let flat = jd_raster::composite::flatten(&doc);
        let info: Vec<_> = doc.layers().iter().map(|l| (l.name.clone(), l.opacity, l.visible, l.blend, l.pixels.pixel(0, 0))).collect();
        assert_eq!(flat[3], 255, "{info:?}");
        // Green below, red on top (as the fixture's notes say), both showing: red.
        assert_eq!(&flat[..4], &[255, 0, 0, 255], "{info:?}");
        assert_eq!(doc.layers()[1].name, "Red");
        assert!(read("x.psd".into(), b"not a psd").is_err());
    }
}
