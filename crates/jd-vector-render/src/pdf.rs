//! PDF export: each layer (usually a frame) becomes a page, drawn from our SVG export by
//! `svg2pdf`, so the PDF is vector (text as outlines) and matches the SVG.

use std::collections::HashMap;

use jd_vector::{Document, NodeId};
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref};

/// A PDF with one page per layer in `ids`, each page the layer's size (1 px = 1 pt).
pub fn export_pdf(doc: &Document, ids: &[NodeId]) -> Result<Vec<u8>, String> {
    let mut alloc = Ref::new(1);
    let catalog = alloc.bump();
    let tree_id = alloc.bump();
    let mut pdf = Pdf::new();
    let mut pages = Vec::new();
    let opt = svg2pdf::usvg::Options::default();
    for &id in ids {
        let svg = crate::export_svg(doc, id).ok_or_else(|| format!("layer {id} has nothing to export"))?;
        let tree = svg2pdf::usvg::Tree::from_str(&svg, &opt).map_err(|e| e.to_string())?;
        let (chunk, svg_ref) = svg2pdf::to_chunk(&tree, svg2pdf::ConversionOptions::default()).map_err(|e| e.to_string())?;
        // Give the chunk's objects numbers after ours.
        let mut map = HashMap::new();
        let chunk = chunk.renumber(|old| *map.entry(old).or_insert_with(|| alloc.bump()));
        let svg_ref = map[&svg_ref];
        let (w, h) = (tree.size().width(), tree.size().height());
        let page_id = alloc.bump();
        let content_id = alloc.bump();
        let name = Name(b"S1");
        let mut page = pdf.page(page_id);
        page.media_box(Rect::new(0.0, 0.0, w, h));
        page.parent(tree_id);
        page.contents(content_id);
        page.resources().x_objects().pair(name, svg_ref);
        page.finish();
        let mut content = Content::new();
        content.transform([w, 0.0, 0.0, h, 0.0, 0.0]).x_object(name);
        pdf.stream(content_id, &content.finish());
        pdf.extend(&chunk);
        pages.push(page_id);
    }
    if pages.is_empty() {
        return Err("nothing to export".into());
    }
    pdf.catalog(catalog).pages(tree_id);
    pdf.pages(tree_id).count(pages.len() as i32).kids(pages);
    Ok(pdf.finish())
}

#[cfg(test)]
mod tests {
    use jd_vector::{ApproxMeasure, NodeSpec, Op, Paint};

    use super::*;

    #[test]
    fn one_page_per_frame() {
        let mut doc = Document::new();
        let page = doc.pages()[0];
        let done = doc
            .apply(
                &[
                    Op::create_as("a", page, NodeSpec::frame("A", 200.0, 100.0)),
                    Op::create("a", NodeSpec::rect("R", 50.0, 50.0).fill(Paint::hex("#4C8DFF"))),
                    Op::create_as("b", page, NodeSpec::frame("B", 300.0, 300.0).at(300.0, 0.0)),
                ],
                &ApproxMeasure,
            )
            .unwrap();
        let pdf = export_pdf(&doc, &[done.id("a").unwrap(), done.id("b").unwrap()]).unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(pdf.starts_with(b"%PDF-"));
        assert!(text.contains("/Count 2"), "two pages");
        assert!(text.contains("/MediaBox [0 0 200 100]") && text.contains("/MediaBox [0 0 300 300]"));
    }
}
