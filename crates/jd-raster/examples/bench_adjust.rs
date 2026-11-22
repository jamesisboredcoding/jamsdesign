//! cargo run --release -p jd-raster --example bench_adjust
use std::time::Instant;

use jd_raster::TileGrid;
use jd_raster::adjust::{self, Adjustment};
use jd_raster::filter::{self, Filter};

fn main() {
    let (w, h) = (5120u32, 2880u32);
    let px: Vec<u8> = (0..w * h).flat_map(|i| [(i % w % 256) as u8, (i / w % 256) as u8, 128, 255]).collect();
    let g = TileGrid::from_rgba8(w, h, &px);
    let t = |name: &str, f: &dyn Fn() -> TileGrid| {
        let s = Instant::now();
        f();
        println!("{name:<26} {:>9.1?}", s.elapsed());
    };
    t("levels", &|| adjust::apply(&g, &Adjustment::Levels { in_black: 20, in_white: 230, gamma: 1.3, out_black: 0, out_white: 255 }, None));
    t("curves", &|| adjust::apply(&g, &Adjustment::Curves { points: vec![(0, 0), (90, 60), (200, 230), (255, 255)] }, None));
    t("hue/saturation", &|| adjust::apply(&g, &Adjustment::HueSaturation { hue: 40.0, saturation: 20.0, lightness: 0.0 }, None));
    t("gaussian blur r=2", &|| filter::apply(&g, &Filter::GaussianBlur { radius: 2.0 }, None));
    t("gaussian blur r=20", &|| filter::apply(&g, &Filter::GaussianBlur { radius: 20.0 }, None));
    t("unsharp r=3", &|| filter::apply(&g, &Filter::UnsharpMask { amount: 100.0, radius: 3.0, threshold: 2 }, None));
    t("noise", &|| filter::apply(&g, &Filter::AddNoise { amount: 10.0, monochrome: false, seed: 1 }, None));
    t("mosaic 16", &|| filter::apply(&g, &Filter::Mosaic { cell: 16 }, None));
}
