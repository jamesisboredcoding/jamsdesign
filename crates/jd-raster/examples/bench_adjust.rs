//! Times adjustment layers on a 15 MP, 3-layer document: the first render, a slider change
//! (only the adjustment changes) and a brush dab below it (one tile changes).
//! `cargo run --release -p jd-raster --example bench_adjust`
use std::time::Instant;

use jd_raster::Document;
use jd_raster::adjust::Adjustment;
use jd_raster::mask::Mask;

fn main() {
    let (w, h) = (4800u32, 3200u32);
    let px: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, (i % 241) as u8, (i % 239) as u8, 255]).collect();
    let mut doc = Document::from_rgba8("bench", w, h, &px);
    let top = doc.add_layer("top");
    doc.layer_mut(top).unwrap().pixels = jd_raster::TileGrid::from_rgba8(w, h, &[30, 60, 90, 128].repeat((w * h) as usize));
    let id = doc.add_layer("Levels");
    let l = doc.layer_mut(id).unwrap();
    l.adjustment = Some(Adjustment::Levels { in_black: 10, in_white: 240, gamma: 1.2, out_black: 0, out_white: 255 });
    l.mask = Some(Mask::new(w, h, 255));

    let t = Instant::now();
    let _ = doc.rendered_layers();
    println!("first render: {:?}", t.elapsed());
    let t = Instant::now();
    let _ = doc.rendered_layers();
    println!("unchanged (cache): {:?}", t.elapsed());
    doc.layer_mut(id).unwrap().adjustment = Some(Adjustment::Levels { in_black: 20, in_white: 240, gamma: 1.2, out_black: 0, out_white: 255 });
    let t = Instant::now();
    let _ = doc.rendered_layers();
    println!("slider change: {:?}", t.elapsed());
    doc.layer_mut(top).unwrap().pixels.set_pixel(100, 100, [255, 0, 0, 255]);
    let t = Instant::now();
    let _ = doc.rendered_layers();
    println!("one tile painted below: {:?}", t.elapsed());
}
