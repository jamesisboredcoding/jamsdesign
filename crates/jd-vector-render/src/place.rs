//! For other apps placing a Blobr design (Photosplash's File > Place): the frames a `.jdb`
//! file has, and one of them as pixels.

use std::path::Path;

use jd_vector::{Document, NodeId};

use crate::{CosmicMeasure, RenderOptions, render_node};

/// A frame that can be placed: its page, name and size.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceableFrame {
    pub id: NodeId,
    pub page: String,
    pub name: String,
    pub width: f64,
    pub height: f64,
}

/// Opens a `.jdb` file, laid out with the system's fonts.
pub fn open(path: &Path) -> Result<Document, String> {
    let mut doc = jd_vector::file::load(path).map_err(|e| e.to_string())?;
    jd_vector::layout::run(&mut doc, &CosmicMeasure);
    Ok(doc)
}

/// The top-level frames (and components) of every page.
pub fn frames(doc: &Document) -> Vec<PlaceableFrame> {
    let mut out = Vec::new();
    for &p in doc.pages() {
        let page = doc.node(p).map_or(String::new(), |n| n.name.clone());
        for &c in doc.children(p) {
            let Some(n) = doc.node(c) else { continue };
            if n.kind.is_frame_like() {
                out.push(PlaceableFrame { id: c, page: page.clone(), name: n.name.clone(), width: n.props.width, height: n.props.height });
            }
        }
    }
    out
}

/// A frame drawn at `scale` (1.0 = one pixel per design pixel): width, height and RGBA8 with
/// straight (not premultiplied) alpha, transparent where the frame has no fill.
pub fn render_rgba(doc: &Document, id: NodeId, scale: f64) -> Option<(u32, u32, Vec<u8>)> {
    let pm = render_node(doc, id, RenderOptions { scale, background: None })?;
    let rgba = pm
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Some((pm.width(), pm.height(), rgba))
}

#[cfg(test)]
mod tests {
    use jd_vector::{ApproxMeasure, NodeSpec, Op, Paint};

    use super::*;

    #[test]
    fn lists_frames_and_renders_one() {
        let mut doc = Document::new();
        let page = doc.pages()[0];
        let done = doc
            .apply(&[Op::create_as("f", page, NodeSpec::frame("Poster", 40.0, 20.0).fill(Paint::hex("#4C8DFF"))), Op::create(page, NodeSpec::rect("Loose", 5.0, 5.0))], &ApproxMeasure)
            .unwrap();
        let f = frames(&doc);
        assert_eq!(f.len(), 1);
        assert_eq!((f[0].name.as_str(), f[0].width), ("Poster", 40.0));
        let (w, h, px) = render_rgba(&doc, done.id("f").unwrap(), 2.0).unwrap();
        assert_eq!((w, h), (80, 40));
        assert_eq!(&px[..4], &[0x4C, 0x8D, 0xFF, 255]);
    }
}
