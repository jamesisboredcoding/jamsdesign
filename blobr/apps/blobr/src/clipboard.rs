//! The system clipboard, so layers move between Blobr windows and images and text come in
//! from other apps. Through `arboard` (Wayland data-control, or X11).

use jd_vector::clip::Clip;

/// What the system clipboard held.
#[derive(Debug, Clone)]
pub enum Pasted {
    Layers(Box<Clip>),
    /// An image, already encoded as PNG, with its size.
    Image { width: u32, height: u32, png: Vec<u8> },
    Text(String),
}

/// Puts text on the clipboard. On Linux the clipboard is served by whoever set it, so a
/// thread keeps serving it until something else is copied.
pub fn put_text(text: String) {
    if cfg!(test) {
        return;
    }
    std::thread::spawn(move || {
        let Ok(mut c) = arboard::Clipboard::new() else { return };
        #[cfg(target_os = "linux")]
        {
            use arboard::SetExtLinux;
            let _ = c.set().wait().text(text);
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = c.set_text(text);
        }
    });
}

/// Puts an image (RGBA8, straight alpha) on the clipboard.
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

/// Reads the clipboard (blocking; run it off the UI thread).
pub fn read() -> Option<Pasted> {
    if cfg!(test) {
        return None;
    }
    let mut c = arboard::Clipboard::new().ok()?;
    if let Ok(text) = c.get_text() {
        if let Some(clip) = Clip::from_text(&text) {
            return Some(Pasted::Layers(Box::new(clip)));
        }
        if c.get_image().is_err() {
            return Some(Pasted::Text(text));
        }
    }
    let img = c.get_image().ok()?;
    let (width, height) = (img.width as u32, img.height as u32);
    let png = encode_png(width, height, &img.bytes)?;
    Some(Pasted::Image { width, height, png })
}

/// Straight-alpha RGBA8 to PNG.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut pm = tiny_skia::Pixmap::new(width, height)?;
    for (px, c) in pm.pixels_mut().iter_mut().zip(rgba.chunks_exact(4)) {
        *px = tiny_skia::ColorU8::from_rgba(c[0], c[1], c[2], c[3]).premultiply();
    }
    pm.encode_png().ok()
}

/// A premultiplied pixmap to straight-alpha RGBA8.
pub fn to_rgba(pm: &tiny_skia::Pixmap) -> Vec<u8> {
    pm.pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

/// Reads the clipboard on a thread.
pub async fn read_async() -> Option<Pasted> {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(read());
    });
    rx.await.ok().flatten()
}

#[cfg(test)]
mod tests {
    #[test]
    fn png_round_trip_size() {
        let png = super::encode_png(2, 1, &[255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
        let pm = tiny_skia::Pixmap::decode_png(&png).unwrap();
        assert_eq!((pm.width(), pm.height()), (2, 1));
        assert_eq!(&super::to_rgba(&pm)[..4], &[255, 0, 0, 255]);
    }
}
