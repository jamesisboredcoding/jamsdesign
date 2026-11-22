//! Lists what a `.psd` opens as: `cargo run -p jd-io --example psd_info -- file.psd`
fn main() {
    let path = std::env::args().nth(1).expect("usage: psd_info file.psd");
    let t = std::time::Instant::now();
    match jd_io::open(std::path::Path::new(&path)) {
        Ok(doc) => {
            println!("{} x {}, {} layers, opened in {:?}", doc.width, doc.height, doc.layers().len(), t.elapsed());
            for l in doc.layers().iter().rev() {
                let indent = "  ".repeat(doc.depth(l.id));
                let kind = if l.is_group() { "group" } else { "layer" };
                let bounds = jd_raster::transform::content_bounds(&l.pixels);
                println!("{indent}{kind} {:?} visible={} opacity={:.2} blend={:?} bounds={bounds:?}", l.name, l.visible, l.opacity, l.blend);
            }
        }
        Err(e) => println!("error: {e}"),
    }
}
