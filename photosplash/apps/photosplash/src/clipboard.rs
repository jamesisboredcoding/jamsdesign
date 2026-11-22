//! The system clipboard (other apps): copied pixels go out as an image; pasting takes an
//! image, image files copied in a file manager, or text (while typing).
//!
//! Through `arboard` (Wayland data-control, or X11). Reading and writing can block for a
//! moment, so they run off the UI thread.

use std::path::PathBuf;

use jd_raster::TileGrid;

/// What the system clipboard held when pasting.
#[derive(Debug, Clone)]
pub enum Pasted {
    /// RGBA8 pixels (straight alpha) and a fingerprint (to recognise our own copy).
    Image { width: u32, height: u32, rgba: Vec<u8>, stamp: u64 },
    /// Image files copied in a file manager.
    Files(Vec<PathBuf>),
    Text(String),
}

/// A fingerprint of an image's pixels.
pub fn stamp(width: u32, height: u32, rgba: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ ((width as u64) << 32 | height as u64);
    // Every 7th byte is plenty to tell images apart, and fast on big ones.
    for b in rgba.iter().step_by(7) {
        h = (h ^ *b as u64).wrapping_mul(0x0100_0000_0100_01b3);
    }
    h
}

/// The visible part of `grid` (on the canvas) as RGBA8, cropped to its content.
pub fn grid_image(grid: &TileGrid) -> Option<(u32, u32, Vec<u8>)> {
    let [x0, y0, x1, y1] = jd_raster::transform::content_bounds(grid)?;
    let (w, h) = (x1 - x0, y1 - y0);
    let full = grid.to_rgba8();
    let stride = grid.width() as usize * 4;
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in y0..y1 {
        let row = y as usize * stride;
        out.extend_from_slice(&full[row + x0 as usize * 4..row + x1 as usize * 4]);
    }
    Some((w, h, out))
}

/// Puts an image on the system clipboard. On Linux the clipboard is served by whoever set it,
/// so a thread keeps serving it until something else is copied.
pub fn put_image(width: u32, height: u32, rgba: Vec<u8>) {
    if cfg!(test) {
        return;
    }
    std::thread::spawn(move || {
        let Ok(mut c) = arboard::Clipboard::new() else { return };
        let img = arboard::ImageData { width: width as usize, height: height as usize, bytes: rgba.into() };
        #[cfg(target_os = "linux")]
        {
            use arboard::SetExtLinux;
            let _ = c.set().wait().image(img);
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = c.set_image(img);
        }
    });
}

/// Reads the system clipboard: an image if there is one, else image files, else text.
pub fn read() -> Option<Pasted> {
    if cfg!(test) {
        return None;
    }
    let mut c = arboard::Clipboard::new().ok()?;
    if let Ok(img) = c.get_image() {
        let (width, height) = (img.width as u32, img.height as u32);
        let rgba = img.bytes.into_owned();
        let stamp = stamp(width, height, &rgba);
        return Some(Pasted::Image { width, height, rgba, stamp });
    }
    let text = c.get_text().ok()?;
    // A file manager's copy: one file:// URI per line.
    let files: Vec<PathBuf> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("file://"))
        .map(|p| PathBuf::from(percent_decode(p)))
        .filter(|p| openable(p))
        .collect();
    if !files.is_empty() {
        return Some(Pasted::Files(files));
    }
    Some(Pasted::Text(text))
}

/// An image or document Photosplash opens (by extension).
fn openable(p: &std::path::Path) -> bool {
    let ext = p.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_default();
    jd_io::OPEN_EXTENSIONS.contains(&ext.as_str()) || ext == jd_io::jdp::EXTENSION
}

/// `%20` and friends in file URIs.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_to_content_and_decodes_uris() {
        let mut g = TileGrid::empty(300, 300);
        g.set_pixel(10, 20, [1, 2, 3, 255]);
        g.set_pixel(12, 21, [4, 5, 6, 255]);
        let (w, h, px) = grid_image(&g).unwrap();
        assert_eq!((w, h), (3, 2));
        assert_eq!(&px[..4], &[1, 2, 3, 255]);
        assert_eq!(percent_decode("/home/me/My%20Pics/a.png"), "/home/me/My Pics/a.png");
        assert_ne!(stamp(3, 2, &px), stamp(2, 3, &px));
    }
}
