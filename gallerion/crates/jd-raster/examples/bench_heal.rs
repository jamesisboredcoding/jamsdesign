//! Times Spot Healing on a photo-sized hole: `cargo run --release -p jd-raster --example bench_heal`
use jd_raster::heal::{Px, inpaint};

fn main() {
    for (size, hole_r) in [(200usize, 30usize), (360, 60), (600, 120)] {
        // Smooth gradient plus diagonal stripes, with a disc-shaped object in the middle.
        let tex = |x: usize, y: usize| {
            let v = 80.0 + 60.0 * (((x + y) % 10) < 5) as u8 as f32 + y as f32 * 0.1;
            [v, v * 0.8, v * 0.6, 255.0]
        };
        let c = size as i64 / 2;
        let hole: Vec<bool> = (0..size * size)
            .map(|i| {
                let (x, y) = ((i % size) as i64 - c, (i / size) as i64 - c);
                x * x + y * y <= (hole_r * hole_r) as i64
            })
            .collect();
        let img: Vec<Px> = (0..size * size).map(|i| if hole[i] { [255.0, 0.0, 0.0, 255.0] } else { tex(i % size, i / size) }).collect();
        let t = std::time::Instant::now();
        let out = inpaint(&img, &hole, size, size);
        let (mut err, mut n) = (0.0, 0.0);
        for i in 0..size * size {
            if hole[i] {
                err += (out[i][0] - tex(i % size, i / size)[0]).abs();
                n += 1.0;
            }
        }
        println!("{size}px region, {}px-radius hole: {:?}, mean error {:.1} (stripes are 60 apart)", hole_r, t.elapsed(), err / n);
    }
}
