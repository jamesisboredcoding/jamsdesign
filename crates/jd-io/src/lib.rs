//! Files in and out: opening images and `.jdp` documents, saving `.jdp`, exporting flattened
//! images.
//!
//! Current limits (colour management milestone):
//! - images are converted to 8-bit RGBA (16-bit and float images lose precision)
//! - embedded ICC profiles are ignored; pixels are assumed to be sRGB

pub mod jdp;
pub mod psd;

use std::fmt;
use std::io::{BufReader, Read, Write};
use std::path::Path;

use image::{DynamicImage, ImageDecoder, ImageReader};
use jd_raster::Document;

/// File extensions we can open, for file dialogs.
pub const OPEN_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "tif", "tiff", "gif", "bmp", "tga", "qoi", "exr", "hdr", "ico",
    "pnm", "pbm", "pgm", "ppm", "dds", "ff", "psd",
];

/// File extensions we can export to.
pub const EXPORT_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "tif", "tiff", "bmp", "tga", "qoi"];

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Decode(image::ImageError),
    /// A `.jdp` that's damaged or from an incompatible version.
    Format(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "couldn't read file: {e}"),
            Self::Decode(e) => write!(f, "couldn't decode image: {e}"),
            Self::Format(e) => write!(f, "not a valid Photosplash document: {e}"),
        }
    }
}

#[derive(Debug)]
pub enum SaveError {
    Io(std::io::Error),
    Zip(zip::result::ZipError),
    Encode(image::ImageError),
    UnknownFormat(String),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "couldn't write file: {e}"),
            Self::Zip(e) => write!(f, "couldn't write document: {e}"),
            Self::Encode(e) => write!(f, "couldn't encode image: {e}"),
            Self::UnknownFormat(ext) => write!(f, "can't export to .{ext} files"),
        }
    }
}

impl std::error::Error for SaveError {}

impl From<std::io::Error> for SaveError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<zip::result::ZipError> for SaveError {
    fn from(e: zip::result::ZipError) -> Self {
        Self::Zip(e)
    }
}

impl From<image::ImageError> for SaveError {
    fn from(e: image::ImageError) -> Self {
        Self::Encode(e)
    }
}

impl std::error::Error for LoadError {}

impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<image::ImageError> for LoadError {
    fn from(e: image::ImageError) -> Self {
        Self::Decode(e)
    }
}

/// Whether `path` is (or will be) a native document rather than a plain image.
pub fn is_document(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(jdp::EXTENSION))
}

/// Opens a `.jdp` document, or decodes an image file into a single-layer document (EXIF
/// orientation applied, so phone photos come out the right way up). Blocking; call it off
/// the UI thread.
pub fn open(path: &Path) -> Result<Document, LoadError> {
    let name = path
        .file_name()
        .map_or_else(|| "Untitled".to_owned(), |n| n.to_string_lossy().into_owned());
    let mut head = [0u8; 128];
    let n = std::fs::File::open(path)?.read(&mut head)?;
    if jdp::sniff(&head[..n]) {
        return jdp::read(name, BufReader::new(std::fs::File::open(path)?));
    }
    if head[..n].starts_with(b"8BPS") {
        return psd::read(name, &std::fs::read(path)?);
    }
    Ok(from_image(name, decode(path)?))
}

/// Flattens visible layers and writes an image; the format comes from the extension.
/// JPEG has no transparency, so transparent areas become white. Blocking.
pub fn export(doc: &Document, path: &Path) -> Result<(), SaveError> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let format = image::ImageFormat::from_extension(&ext)
        .filter(|_| EXPORT_EXTENSIONS.contains(&ext.as_str()))
        .ok_or_else(|| SaveError::UnknownFormat(ext.clone()))?;
    let rgba = image::RgbaImage::from_raw(doc.width, doc.height, jd_raster::composite::flatten(doc))
        .expect("flatten returns width*height*4 bytes");
    let image = match format {
        image::ImageFormat::Jpeg | image::ImageFormat::Bmp => {
            let rgb = image::RgbImage::from_fn(doc.width, doc.height, |x, y| {
                let [r, g, b, a] = rgba.get_pixel(x, y).0;
                let over_white = |c: u8| ((c as u32 * a as u32 + 255 * (255 - a as u32)) / 255) as u8;
                image::Rgb([over_white(r), over_white(g), over_white(b)])
            });
            DynamicImage::ImageRgb8(rgb)
        }
        _ => DynamicImage::ImageRgba8(rgba),
    };
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    if format == image::ImageFormat::Jpeg {
        image.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92))?;
    } else {
        image.write_to(&mut out, format)?;
    }
    out.flush()?;
    Ok(())
}

fn decode(path: &Path) -> Result<DynamicImage, LoadError> {
    let file = std::fs::File::open(path)?;
    let mut reader = ImageReader::new(BufReader::new(file)).with_guessed_format()?;
    // Default limits refuse big panoramas; this is a desktop editor, let the OS say no instead.
    reader.no_limits();
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image)
}

pub fn from_image(name: impl Into<String>, image: DynamicImage) -> Document {
    let rgba = image.into_rgba8();
    let (w, h) = rgba.dimensions();
    Document::from_rgba8(name, w, h, rgba.as_raw())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn opens_a_png() {
        let dir = std::env::temp_dir().join(format!("jd-io-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("check.png");
        let mut img = RgbaImage::new(300, 260);
        img.put_pixel(299, 259, Rgba([1, 2, 3, 255]));
        img.save(&path).unwrap();

        let doc = open(&path).unwrap();
        assert_eq!((doc.width, doc.height), (300, 260));
        assert_eq!(doc.name, "check.png");
        assert_eq!(doc.layers()[0].pixels.pixel(299, 259), [1, 2, 3, 255]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn open_detects_jdp_by_content_and_export_flattens() {
        let dir = std::env::temp_dir().join(format!("jd-io-jdp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let doc = Document::from_rgba8("x", 2, 1, &[255, 0, 0, 255, 0, 0, 255, 128]);
        // Wrong extension on purpose: content sniffing should still find it.
        let path = dir.join("doc.bin");
        jdp::save(&doc, &path).unwrap();
        let back = open(&path).unwrap();
        assert_eq!(back.layers()[0].pixels.pixel(1, 0), [0, 0, 255, 128]);

        let png = dir.join("out.png");
        export(&back, &png).unwrap();
        let img = image::open(&png).unwrap().into_rgba8();
        assert_eq!(img.get_pixel(1, 0).0, [0, 0, 255, 128]);

        let jpg = dir.join("out.jpg");
        export(&back, &jpg).unwrap();
        assert!(matches!(export(&back, &dir.join("out.xyz")), Err(SaveError::UnknownFormat(_))));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn reports_missing_file() {
        assert!(matches!(open(Path::new("/definitely/not/here.png")), Err(LoadError::Io(_))));
    }
}
