//! cargo run --release -p jd-raster --example bench_paint
use jd_raster::Document;
use jd_raster::paint::{BrushParams, Stroke};

fn main() {
    let (w, h) = (5120, 2880);
    let mut doc = Document::from_rgba8("b", w, h, &[200, 200, 200, 255].repeat((w * h) as usize));
    doc.add_layer("paint");
    for size in [10.0, 100.0, 400.0] {
        let params = BrushParams { size, hardness: 0.5, flow: 0.6, ..Default::default() };
        let mut s = Stroke::begin(&doc, params).unwrap();
        let t = std::time::Instant::now();
        let n = 200;
        for i in 0..n {
            // ~10 px between input events, like a fast mouse/pen.
            s.add_point(&mut doc, 500.0 + i as f32 * 10.0, 1400.0 + (i as f32 * 0.1).sin() * 300.0, 0.8);
        }
        println!("size {size:>4}: {:?} per input event", t.elapsed() / n);
    }
}
