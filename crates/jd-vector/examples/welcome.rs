//! Builds the "Welcome" screen from the Blobr mockup as a `.jdb` file, using only ops.
//!
//! cargo run -p jd-vector --example welcome -- welcome.jdb

use jd_vector::*;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "welcome.jdb".into());
    let mut ed = Editor::new(Document::new());
    let page = ed.doc.pages()[0];

    let ink = "#1C1B22";
    let sub = "#6B6872";
    let style = |size: f32, weight: &str| TextStyle { size, style: weight.into(), ..TextStyle::default() };
    let text = |name: &str, s: &str, st: TextStyle, color: &str| NodeSpec::text(name, s, st).fill(Paint::hex(color));

    let blob = VectorPath::from_svg(
        "M110 46c54-34 132-12 150 44s-12 124-76 140-130-4-142-60S60 78 110 46z",
        345.0,
        300.0,
    )
    .expect("valid path");

    let ops = vec![
        Op::create_as("screen", page, NodeSpec::frame("Welcome", 393.0, 852.0).fill(Paint::hex("#FBF7F2"))),
        Op::create_as(
            "status",
            "screen",
            NodeSpec::frame("Status bar", 393.0, 44.0).no_fill().auto_layout(AutoLayout {
                padding: Padding::hv(32.0, 0.0),
                align: Align::SpaceBetween,
                counter_align: CounterAlign::Center,
                ..AutoLayout::default()
            })
            .sizing(Sizing::Fixed, Sizing::Fixed),
        ),
        Op::create("status", text("Time", "9:41", style(15.0, "Semi Bold"), ink)),
        Op::create("status", NodeSpec::rect("Battery", 24.0, 12.0).radius(3.0).no_fill().stroke(Paint::hex(ink), 1.5)),
        Op::create(
            "screen",
            NodeSpec::vector("Hero blob", blob).at(24.0, 96.0).fill(Paint::hex("#FF7A59")).with(|p| p.strokes.clear()),
        ),
        Op::create_as(
            "content",
            "screen",
            NodeSpec::frame("Content", 345.0, 0.0)
                .no_fill()
                .at(24.0, 436.0)
                .auto_layout(AutoLayout { direction: Direction::Vertical, gap: 16.0, ..AutoLayout::default() })
                .sizing(Sizing::Fixed, Sizing::Hug),
        ),
        Op::create_as("dots", "content", NodeSpec::frame("Pager dots", 0.0, 0.0).no_fill().auto_layout(AutoLayout { gap: 6.0, ..AutoLayout::default() })),
        Op::create("dots", NodeSpec::rect("Dot", 22.0, 8.0).radius(4.0).fill(Paint::hex(ink))),
        Op::create("dots", NodeSpec::rect("Dot", 8.0, 8.0).radius(4.0).fill(Paint::hex("#D6CFC6"))),
        Op::create("dots", NodeSpec::rect("Dot", 8.0, 8.0).radius(4.0).fill(Paint::hex("#D6CFC6"))),
        Op::create("content", text("Title", "Make something\nwonderful", style(36.0, "Bold"), ink)),
        Op::create(
            "content",
            text("Subtitle", "Sketch, plan and ship ideas together, on a canvas that never runs out.", style(17.0, "Regular"), sub)
                .sizing(Sizing::Fill, Sizing::Hug),
        ),
        Op::create_as(
            "button",
            "screen",
            NodeSpec::frame("Get started", 345.0, 52.0)
                .at(24.0, 668.0)
                .radius(14.0)
                .fill(Paint::hex("#4C8DFF"))
                .auto_layout(AutoLayout { align: Align::Center, counter_align: CounterAlign::Center, ..AutoLayout::default() })
                .sizing(Sizing::Fixed, Sizing::Fixed),
        ),
        Op::create("button", text("Label", "Get started", style(16.0, "Semi Bold"), "#FFFFFF")),
    ];

    let done = match ed.apply("Build Welcome screen", &ops, &ApproxMeasure) {
        Ok(done) => done,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = file::save(&ed.doc, std::path::Path::new(&out)) {
        eprintln!("couldn't save {out}: {e}");
        std::process::exit(1);
    }

    println!("Saved {out}: {} nodes, {} created in one undo step.", ed.doc.len(), done.created.len());
    for key in ["content", "button"] {
        let id = done.id(key).expect("created");
        let b = ed.doc.world_bounds(id).expect("exists");
        println!("  {key}: {} x {} at ({}, {})", b.width(), b.height(), b.x0, b.y0);
    }
}
