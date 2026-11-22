//! Free Transform commit speed on a 15 MP layer: `cargo run --release -p jd-raster --example bench_transform`
use jd_raster::TileGrid;
use jd_raster::transform::{Affine, transform_grid};

fn main() {
    let (w, h) = (4752u32, 3168u32);
    let px: Vec<u8> = (0..w * h).flat_map(|i| [(i % 251) as u8, (i % 241) as u8, (i % 239) as u8, 255]).collect();
    let src = TileGrid::from_rgba8(w, h, &px);
    let c = (w as f32 / 2.0, h as f32 / 2.0);
    for (name, m) in [
        ("rotate 10 degrees", Affine::from_parts(c, c, 0.1745, 1.0, 1.0)),
        ("enlarge 1.5x", Affine::from_parts(c, c, 0.0, 1.5, 1.5)),
        ("shrink to 30%", Affine::from_parts(c, c, 0.0, 0.3, 0.3)),
    ] {
        let t = std::time::Instant::now();
        let out = transform_grid(&src, &m);
        println!("{name}: {:?} ({} tiles)", t.elapsed(), out.iter().count());
    }
}
