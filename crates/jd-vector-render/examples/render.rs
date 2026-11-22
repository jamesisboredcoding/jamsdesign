//! Renders a `.jdb` file (a page, or one node by name) to PNG or SVG.
//!
//! cargo run -p jd-vector-render --example render -- in.jdb out.png [scale] [node name]
//!
//! Layout runs again with the system's fonts first, so text is measured for real.

use std::path::Path;

use jd_vector::file;
use jd_vector_render::{CosmicMeasure, RenderOptions, export_svg, render_png};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output, rest @ ..] = args.as_slice() else {
        eprintln!("usage: render <in.jdb> <out.png|out.svg> [scale] [node name]");
        std::process::exit(2);
    };
    let scale: f64 = rest.first().and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let mut doc = file::load(Path::new(input)).unwrap_or_else(|e| fail(&format!("{input}: {e}")));
    jd_vector::layout::run(&mut doc, &CosmicMeasure);
    let id = match rest.get(1) {
        Some(name) => doc.find_by_name(name).unwrap_or_else(|| fail(&format!("no node named {name:?}"))),
        None => doc.pages()[0],
    };
    let bytes = if output.ends_with(".svg") {
        export_svg(&doc, id).unwrap_or_else(|| fail("nothing to export")).into_bytes()
    } else {
        render_png(&doc, id, RenderOptions { scale, ..RenderOptions::default() }).unwrap_or_else(|e| fail(&e))
    };
    std::fs::write(output, bytes).unwrap_or_else(|e| fail(&format!("{output}: {e}")));
    println!("Wrote {output}");
}

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}
