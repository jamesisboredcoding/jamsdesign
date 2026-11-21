//! cargo run --release -p jd-raster --example bench_select
use std::time::Instant;

use jd_raster::selection::{self, Selection};
use jd_raster::transform::{Affine, transform_grid};
use jd_raster::{Document, TileGrid};

fn time<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let r = f();
    println!("{name:<34} {:>9.1?}", t.elapsed());
    r
}

fn main() {
    let (w, h) = (5120u32, 2880u32);
    let px: Vec<u8> = (0..w * h).flat_map(|i| [(i % w % 256) as u8, (i / w % 256) as u8, 128, 255]).collect();
    let doc = Document::from_rgba8("b", w, h, &px);
    let layer = &doc.layers()[0].pixels;

    let all = time("select all", || Selection::all(w, h));
    time("select all: outline", || all.outline().len());
    let rect = time("rect 4000x2000", || Selection::rect(w, h, 500.0, 400.0, 4500.0, 2400.0));
    time("rect: outline", || rect.outline().len());
    let ell = time("ellipse 4000x2000", || Selection::ellipse(w, h, 500.0, 400.0, 4500.0, 2400.0));
    time("ellipse: outline", || ell.outline().len());
    let pts: Vec<(f32, f32)> = (0..2000).map(|i| { let a = i as f32 / 2000.0 * 6.283; (2560.0 + a.cos() * (1800.0 + (a * 7.0).sin() * 300.0), 1440.0 + a.sin() * 1200.0) }).collect();
    let lasso = time("lasso 2000 points", || Selection::polygon(w, h, &pts));
    time("lasso: outline", || lasso.outline().len());
    time("magic wand contiguous", || Selection::magic_wand(w, h, 10, 10, 40, true, |x, y| layer.pixel(x, y)));
    time("extract (ellipse)", || selection::extract(layer, &ell));
    let mut g: TileGrid = layer.clone();
    time("clear (ellipse)", || selection::clear(&mut g, &ell));
    time("move whole layer (integer)", || transform_grid(layer, &Affine::translate(37.0, -12.0)));
    time("rotate+scale whole layer", || transform_grid(layer, &Affine::from_parts((2560.0, 1440.0), (2600.0, 1400.0), 0.3, 0.9, 0.9)));
}
