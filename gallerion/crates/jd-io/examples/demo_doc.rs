//! Builds a layered demo document from two images, for trying blend modes.
//!
//! cargo run -p jd-io --example demo_doc -- out.jdp base.jpg top.jpg

use std::path::PathBuf;

use jd_raster::{BlendMode, TileGrid};

fn main() {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [out, base, top] = args.as_slice() else {
        eprintln!("usage: demo_doc OUT.jdp BASE_IMAGE TOP_IMAGE");
        std::process::exit(2);
    };
    let mut doc = jd_io::open(base).expect("open base image");
    doc.name = "demo.jdp".into();
    let (w, h) = (doc.width, doc.height);

    let top = image::open(top).expect("open top image").resize_to_fill(w, h, image::imageops::FilterType::Triangle);
    let id = doc.add_layer("Texture (Multiply 70%)");
    let layer = doc.layer_mut(id).unwrap();
    layer.pixels = TileGrid::from_rgba8(w, h, top.to_rgba8().as_raw());
    layer.blend = BlendMode::Multiply;
    layer.opacity = 0.7;

    // A soft horizontal colour gradient that fades out towards the bottom.
    let gradient: Vec<u8> = (0..w * h)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            let t = x as f32 / w as f32;
            let a = (1.0 - y as f32 / h as f32) * 255.0;
            [(255.0 * (1.0 - t)) as u8, 60, (255.0 * t) as u8, a as u8]
        })
        .collect();
    let id = doc.add_layer("Tint (Color)");
    let layer = doc.layer_mut(id).unwrap();
    layer.pixels = TileGrid::from_rgba8(w, h, &gradient);
    layer.blend = BlendMode::Color;
    layer.opacity = 0.6;

    jd_io::jdp::save(&doc, out).expect("save");
    println!("wrote {}", out.display());
}
