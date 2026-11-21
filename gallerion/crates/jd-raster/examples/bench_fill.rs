//! cargo run --release -p jd-raster --example bench_fill
use jd_raster::Document;
use jd_raster::selection::{self, Selection};

fn main() {
    for (w, h) in [(2480u32, 3508u32), (10000, 10000)] {
        let mut doc = Document::new("f", w, h);
        let id = doc.active_id().unwrap();
        let t = std::time::Instant::now();
        let sel = Selection::all(w, h);
        selection::fill(&mut doc.layer_mut(id).unwrap().pixels, &sel, [255, 255, 255]);
        println!("{w}x{h}: {:?}", t.elapsed());
    }
}
