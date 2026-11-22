//! `.jdb` files: a zip holding `mimetype`, `document.json` (the node tree, nested and
//! readable) and `images/<key>` for image fills. Only images some paint still uses are written.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::doc::Document;
use crate::node::{Node, NodeId, NodeKind, PaintKind, Props};

pub const MIME: &str = "application/vnd.jamsdesign.blobr";
pub const EXTENSION: &str = "jdb";
/// Bumped when the format changes in a way older versions can't read.
pub const VERSION: u32 = 1;

#[derive(Debug)]
pub enum FileError {
    Io(std::io::Error),
    Format(String),
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Format(e) => write!(f, "not a valid Blobr document: {e}"),
        }
    }
}

impl std::error::Error for FileError {}

impl From<std::io::Error> for FileError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<zip::result::ZipError> for FileError {
    fn from(e: zip::result::ZipError) -> Self {
        match e {
            zip::result::ZipError::Io(e) => Self::Io(e),
            e => Self::Format(e.to_string()),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct FileDoc {
    format: String,
    version: u32,
    next_id: u64,
    pages: Vec<FileNode>,
    #[serde(default, skip_serializing_if = "crate::tokens::Tokens::is_empty")]
    tokens: crate::tokens::Tokens,
}

/// A node and its subtree, nested, as files (and copied layers) store them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FileNode {
    pub(crate) id: NodeId,
    pub(crate) name: String,
    #[serde(flatten)]
    pub(crate) kind: NodeKind,
    #[serde(flatten)]
    pub(crate) props: Props,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) children: Vec<FileNode>,
}

pub(crate) fn to_file(doc: &Document, id: NodeId) -> FileNode {
    let n = doc.node(id).expect("listed node exists");
    FileNode {
        id,
        name: n.name.clone(),
        kind: n.kind.clone(),
        props: n.props.clone(),
        children: n.children.iter().map(|&c| to_file(doc, c)).collect(),
    }
}

/// The document as JSON (what `document.json` holds).
pub fn to_json(doc: &Document) -> String {
    let file = FileDoc {
        format: "blobr".into(),
        version: VERSION,
        next_id: doc.next_id,
        pages: doc.pages.iter().map(|&p| to_file(doc, p)).collect(),
        tokens: doc.tokens.clone(),
    };
    serde_json::to_string_pretty(&file).expect("documents always serialize")
}

/// Reads `document.json` content. Images are added separately.
pub fn from_json(json: &str) -> Result<Document, FileError> {
    let file: FileDoc = serde_json::from_str(json).map_err(|e| FileError::Format(e.to_string()))?;
    if file.format != "blobr" {
        return Err(FileError::Format(format!("format is {:?}", file.format)));
    }
    if file.version > VERSION {
        return Err(FileError::Format(format!(
            "made by a newer Blobr (format {}, this version reads up to {VERSION})",
            file.version
        )));
    }
    if file.pages.is_empty() {
        return Err(FileError::Format("no pages".into()));
    }
    let mut doc = Document::empty();
    fn add(doc: &mut Document, f: FileNode, parent: Option<NodeId>) -> Result<(), FileError> {
        let is_page = matches!(f.kind, NodeKind::Page { .. });
        if is_page != parent.is_none() {
            return Err(FileError::Format(format!("node {} is a page in the wrong place, or a non-page at the top", f.id)));
        }
        if doc.contains(f.id) {
            return Err(FileError::Format(format!("node id {} is used twice", f.id)));
        }
        if parent.is_some_and(|p| !doc.node(p).is_some_and(|n| n.kind.is_container())) {
            return Err(FileError::Format(format!("node {} is inside something that can't hold children", f.id)));
        }
        let id = f.id;
        doc.insert(Node { id, name: f.name, parent, children: f.children.iter().map(|c| c.id).collect(), kind: f.kind, props: f.props });
        for c in f.children {
            add(doc, c, Some(id))?;
        }
        Ok(())
    }
    for page in file.pages {
        let id = page.id;
        add(&mut doc, page, None)?;
        doc.pages.push(id);
    }
    doc.tokens = file.tokens;
    let max = doc.nodes.keys().chain(doc.tokens.collections.keys()).chain(doc.tokens.variables.keys()).chain(doc.tokens.styles.keys()).map(|id| id.0).max().unwrap_or(0);
    doc.next_id = file.next_id.max(max + 1);
    Ok(doc)
}

fn used_images(doc: &Document) -> BTreeSet<&str> {
    let mut used = BTreeSet::new();
    for n in doc.nodes.values() {
        for paint in n.props.fills.iter().chain(&n.props.strokes) {
            if let PaintKind::Image { image, .. } = &paint.kind {
                used.insert(image.as_str());
            }
        }
    }
    used
}

pub fn write<W: Write + Seek>(doc: &Document, out: W) -> Result<W, FileError> {
    let mut zip = ZipWriter::new(out);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("mimetype", stored)?;
    zip.write_all(MIME.as_bytes())?;
    zip.start_file("document.json", deflated)?;
    zip.write_all(to_json(doc).as_bytes())?;
    for key in used_images(doc) {
        if let Some(bytes) = doc.images.get(key) {
            // Images are already compressed.
            zip.start_file(format!("images/{key}"), stored)?;
            zip.write_all(bytes)?;
        }
    }
    Ok(zip.finish()?)
}

pub fn read<R: Read + Seek>(input: R) -> Result<Document, FileError> {
    let mut zip = ZipArchive::new(input)?;
    let mut json = String::new();
    zip.by_name("document.json")
        .map_err(|_| FileError::Format("document.json is missing".into()))?
        .read_to_string(&mut json)?;
    let mut doc = from_json(&json)?;
    let mut images = BTreeMap::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let Some(key) = entry.name().ok().and_then(|n| n.strip_prefix("images/").map(str::to_owned)) else { continue };
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        images.insert(key, Arc::new(bytes));
    }
    doc.images = images;
    Ok(doc)
}

/// Saves through a temporary file next to `path`, so a failed save never damages the old one.
pub fn save(doc: &Document, path: &Path) -> Result<(), FileError> {
    let tmp = path.with_extension(format!("{EXTENSION}.saving"));
    let result = (|| {
        let file = std::fs::File::create(&tmp)?;
        write(doc, std::io::BufWriter::new(file))?.flush()?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

pub fn load(path: &Path) -> Result<Document, FileError> {
    read(std::io::BufReader::new(std::fs::File::open(path)?))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::*;

    fn sample() -> Document {
        let mut doc = Document::new();
        let p = doc.pages()[0];
        let png = doc.add_image(vec![0x89, b'P', b'N', b'G', 1, 2, 3]);
        let _unused = doc.add_image(vec![9, 9, 9]);
        doc.apply(
            &[
                Op::CreatePage { name: "Components".into(), key: None, index: None },
                Op::create_as("f", p, NodeSpec::frame("Welcome", 393.0, 852.0)),
                Op::create(
                    "f",
                    NodeSpec::rect("Photo", 100.0, 80.0).fill(Paint {
                        kind: PaintKind::Image { image: png, fit: ImageFit::Fill },
                        ..Paint::hex("#000000")
                    }),
                ),
                Op::create("f", NodeSpec::text("Title", "Make something", TextStyle { size: 36.0, ..TextStyle::default() })),
                Op::create(
                    "f",
                    NodeSpec::vector("Blob", VectorPath::from_svg("M10 0C20 0 20 20 10 20S0 0 10 0z", 20.0, 20.0).unwrap()),
                ),
            ],
            &ApproxMeasure,
        )
        .unwrap();
        doc
    }

    fn same_tree(a: &Document, b: &Document) {
        assert_eq!(a.pages(), b.pages());
        assert_eq!(a.len(), b.len());
        for &p in a.pages() {
            for id in std::iter::once(p).chain(a.descendants(p)) {
                assert_eq!(a.node(id), b.node(id), "node {id}");
            }
        }
    }

    #[test]
    fn documents_round_trip_through_jdb() {
        let doc = sample();
        let bytes = write(&doc, Cursor::new(Vec::new())).unwrap().into_inner();
        let back = read(Cursor::new(bytes)).unwrap();
        same_tree(&doc, &back);
        assert_eq!(back.images.len(), 1, "the unused image was dropped");
        // New ids never collide with loaded ones.
        let mut back = back;
        let p = back.pages()[0];
        let done = back.apply(&[Op::create(p, NodeSpec::rect("New", 1.0, 1.0))], &ApproxMeasure).unwrap();
        assert!(!doc.contains(done.created[0]));
    }

    #[test]
    fn json_is_nested_and_skips_defaults() {
        let json = to_json(&sample());
        assert!(json.contains("\"type\": \"frame\""));
        assert!(json.contains("\"children\""));
        assert!(!json.contains("\"visible\""), "default visibility isn't written");
    }

    #[test]
    fn bad_files_are_rejected_with_a_reason() {
        let err = from_json(r#"{"format":"blobr","version":99,"next_id":1,"pages":[]}"#).unwrap_err();
        assert!(err.to_string().contains("newer Blobr"), "{err}");
        let dup = r#"{"format":"blobr","version":1,"next_id":3,"pages":[
            {"id":1,"name":"P","type":"page","x":0,"y":0,"width":0,"height":0,"children":[
              {"id":1,"name":"R","type":"rectangle","x":0,"y":0,"width":1,"height":1}]}]}"#;
        assert!(from_json(dup).unwrap_err().to_string().contains("used twice"));
    }
}
