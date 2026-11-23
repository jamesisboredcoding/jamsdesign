//! Canvas render timing on the suite mockups: `cargo run --release -p jd-vector-render --example bench`.
use std::time::Instant;
fn main() {
    let mut doc = jd_vector::file::load(std::path::Path::new("blobr/mockups/suite.jdb")).unwrap();
    jd_vector::layout::run(&mut doc, &jd_vector_render::CosmicMeasure);
    let page = doc.pages()[0];
    let opts = |s| jd_vector_render::RenderOptions { scale: s, background: Some(jd_vector::Rgba::from_srgb8(30, 30, 30)) };
    for (name, view, scale) in [("whole page, zoomed out", kurbo::Rect::new(-100.0, -100.0, 5100.0, 1700.0), 0.3), ("one window at 100%", kurbo::Rect::new(3400.0, 0.0, 4800.0, 900.0), 1.0)] {
        let _ = jd_vector_render::render_region(&doc, page, view, opts(scale));
        let t = Instant::now();
        for _ in 0..5 { let _ = jd_vector_render::render_region(&doc, page, view, opts(scale)); }
        println!("{name}: {:.1} ms per frame", t.elapsed().as_secs_f64() * 1000.0 / 5.0);
    }
}
