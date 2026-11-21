//! Times opening and re-saving a .jdp: cargo run --release -p jd-io --example bench_jdp -- doc.jdp
fn main() {
    let path = std::path::PathBuf::from(std::env::args_os().nth(1).expect("path to a .jdp"));
    let t = std::time::Instant::now();
    let doc = jd_io::open(&path).expect("open");
    println!("open: {:?}", t.elapsed());
    let out = path.with_extension("resaved.jdp");
    let t = std::time::Instant::now();
    jd_io::jdp::save(&doc, &out).expect("save");
    println!("save: {:?} ({} MB)", t.elapsed(), std::fs::metadata(&out).unwrap().len() / 1_000_000);
    std::fs::remove_file(out).ok();
    let png = path.with_extension("export.png");
    let t = std::time::Instant::now();
    jd_io::export(&doc, &png).expect("export");
    println!("export png: {:?}", t.elapsed());
    std::fs::remove_file(png).ok();
}
