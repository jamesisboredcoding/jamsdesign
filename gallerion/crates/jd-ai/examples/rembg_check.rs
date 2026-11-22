//! Times background removal on a synthetic image (a disc on a textured background):
//! `cargo run --release -p jd-ai --example rembg_check -- <isnet|silueta> [model dir]`
fn main() {
    let mut args = std::env::args().skip(1);
    let model = if args.next().as_deref() == Some("silueta") { jd_ai::SILUETA } else { jd_ai::ISNET };
    if let Some(dir) = args.next() {
        // SAFETY: single-threaded at this point.
        unsafe { std::env::set_var("XDG_DATA_HOME", dir) };
    }
    let (w, h) = (1200usize, 800usize);
    let mut img = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f32 - 600.0, y as f32 - 420.0);
            let inside = dx * dx / 220.0f32.powi(2) + dy * dy / 260.0f32.powi(2) < 1.0;
            let p = if inside {
                [220, 60 + (y / 4 % 40) as u8, 40, 255]
            } else {
                let v = 150 + ((x / 30 + y / 30) % 2) as u8 * 30;
                [v - 20, v, v + 10, 255]
            };
            img[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&p);
        }
    }
    for round in 0..2 {
        let t = std::time::Instant::now();
        let mask = jd_ai::foreground_mask(&model, &img, w, h).expect("mask");
        println!(
            "{} run {round}: {:?}  centre {}  corner {}  edge-of-disc {}",
            model.name,
            t.elapsed(),
            mask[420 * w + 600],
            mask[20 * w + 20],
            mask[420 * w + 600 + 215]
        );
    }
}
