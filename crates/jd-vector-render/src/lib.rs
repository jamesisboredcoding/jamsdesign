//! Draws `jd-vector` documents without a window: a CPU rasterizer (tiny-skia), PNG and SVG
//! export, and text shaping with the system's fonts (cosmic-text), including the
//! [`CosmicMeasure`] that layout should use once fonts are loaded.
//!
//! No iced here, so export, thumbnails, tests and (later) MCP renders all work headless.

pub mod pdf;
pub mod raster;
pub mod svg;
mod svgin;
pub mod text;

pub use raster::{RenderOptions, render_node, render_png, render_region};
pub use pdf::export_pdf;
pub use svg::export_svg;
pub use svgin::import_svg;
pub use text::{CosmicMeasure, families, text_outline};

use jd_vector::{Document, ExportFormat, NodeId};

/// The files a node's export settings ask for: (file name, bytes). File names are the node's
/// name (with `/` and other unsafe characters replaced) plus each setting's suffix.
pub fn export_files(doc: &Document, id: NodeId) -> Result<Vec<(String, Vec<u8>)>, String> {
    let n = doc.node(id).ok_or_else(|| format!("node {id} doesn't exist"))?;
    let base: String = n.name.chars().map(|c| if c.is_alphanumeric() || " -_.@".contains(c) { c } else { '_' }).collect();
    n.props
        .exports
        .iter()
        .map(|e| {
            let bytes = match e.format {
                ExportFormat::Png => render_png(doc, id, RenderOptions { scale: e.scale, ..RenderOptions::default() })?,
                ExportFormat::Svg => export_svg(doc, id).ok_or("nothing to export")?.into_bytes(),
            };
            Ok((format!("{base}{}.{}", e.suffix, e.format.extension()), bytes))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use jd_vector::*;

    #[test]
    fn export_settings_name_and_scale_files() {
        let mut doc = Document::new();
        let p = doc.pages()[0];
        let exports = vec![
            ExportSetting { format: ExportFormat::Png, scale: 1.0, suffix: String::new() },
            ExportSetting { format: ExportFormat::Png, scale: 2.0, suffix: "@2x".into() },
            ExportSetting { format: ExportFormat::Svg, scale: 1.0, suffix: String::new() },
        ];
        let done = doc
            .apply(&[Op::create(p, NodeSpec::rect("Icons/Star", 10.0, 10.0).with(|p| p.exports = exports))], &ApproxMeasure)
            .unwrap();
        let files = super::export_files(&doc, done.created[0]).unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(names, ["Icons_Star.png", "Icons_Star@2x.png", "Icons_Star.svg"]);
        let size = |png: &[u8]| u32::from_be_bytes(png[16..20].try_into().unwrap());
        assert_eq!((size(&files[0].1), size(&files[1].1)), (10, 20));
    }
}
