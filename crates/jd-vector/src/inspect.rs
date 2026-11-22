//! Code for a layer, as the Inspect tab shows it and the MCP server hands to coding agents:
//! CSS for one layer, HTML+CSS for a subtree, and an iced snippet. Values bound to variables
//! come out as CSS custom properties (`var(--name, value)`), so tokens survive into code.

use std::fmt::Write;

use crate::color::to_hex;
use crate::node::{Align, CounterAlign, Direction, Effect, LineHeight, Node, NodeId, NodeKind, Paint, PaintKind, Sizing, TextAlign};
use crate::{Document, Rgba};

fn px(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() { format!("{}px", r as i64) } else { format!("{r}px") }
}

fn css_color(c: Rgba) -> String {
    if c.a >= 0.999 {
        to_hex(c)
    } else {
        let [r, g, b] = c.to_srgb8();
        format!("rgba({r}, {g}, {b}, {:.2})", c.a)
    }
}

/// `--brand-primary` from a variable name like `Brand/Primary`.
fn var_name(name: &str) -> String {
    let mut out = String::from("--");
    let mut dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if dash && out.len() > 2 {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
            dash = false;
        } else {
            dash = true;
        }
    }
    out
}

/// A value, wrapped in `var()` when the property is bound to a variable.
fn bound(doc: &Document, n: &Node, path: &str, value: String) -> String {
    match n.props.bind.get(path).and_then(|v| doc.tokens().variables.get(v)) {
        Some(var) => format!("var({}, {value})", var_name(&var.name)),
        None => value,
    }
}

fn paint_css(p: &Paint) -> Option<String> {
    if !p.visible {
        return None;
    }
    let with_opacity = |mut c: Rgba| {
        c.a *= p.opacity;
        c
    };
    Some(match &p.kind {
        PaintKind::Solid { color } => css_color(with_opacity(*color)),
        PaintKind::LinearGradient { from, to, stops } => {
            // CSS angles: 0deg points up, clockwise.
            let angle = (to[0] - from[0]).atan2(-(to[1] - from[1])).to_degrees();
            let stops: Vec<String> = stops.iter().map(|s| format!("{} {:.0}%", css_color(with_opacity(s.color)), s.position * 100.0)).collect();
            format!("linear-gradient({angle:.0}deg, {})", stops.join(", "))
        }
        PaintKind::RadialGradient { stops, .. } => {
            let stops: Vec<String> = stops.iter().map(|s| format!("{} {:.0}%", css_color(with_opacity(s.color)), s.position * 100.0)).collect();
            format!("radial-gradient({})", stops.join(", "))
        }
        PaintKind::Image { .. } => "url(image.png) center / cover".into(),
    })
}

fn weight(style: &str) -> (u16, bool) {
    let s = style.to_lowercase().replace([' ', '-'], "");
    let italic = s.contains("italic");
    let w = [
        ("extralight", 200),
        ("ultralight", 200),
        ("semibold", 600),
        ("demibold", 600),
        ("extrabold", 800),
        ("ultrabold", 800),
        ("thin", 100),
        ("light", 300),
        ("medium", 500),
        ("bold", 700),
        ("black", 900),
        ("heavy", 900),
    ]
    .iter()
    .find(|(k, _)| s.contains(k))
    .map_or(400, |(_, w)| *w);
    (w, italic)
}

/// CSS declarations for one layer (one per line, `prop: value;`).
pub fn css(doc: &Document, id: NodeId) -> Vec<String> {
    let Some(n) = doc.node(id) else { return Vec::new() };
    let p = &n.props;
    let parent = n.parent.and_then(|q| doc.node(q));
    let in_flow = parent.is_some_and(|q| q.props.layout.is_some()) && !p.absolute;
    let parent_dir = parent.and_then(|q| q.props.layout.as_ref()).map(|l| l.direction);
    let mut out = Vec::new();
    let mut push = |k: &str, v: String| out.push(format!("{k}: {v};"));

    if !in_flow && parent.is_some_and(|q| !matches!(q.kind, NodeKind::Page { .. })) {
        push("position", "absolute".into());
        push("left", bound(doc, n, "x", px(p.x)));
        push("top", bound(doc, n, "y", px(p.y)));
    }
    let size = |push: &mut dyn FnMut(&str, String), axis: &str, sizing: Sizing, v: f64, horizontal: bool| match sizing {
        Sizing::Fixed => push(axis, bound(doc, n, axis, px(v))),
        Sizing::Hug => {}
        Sizing::Fill => {
            let along = parent_dir == Some(if horizontal { Direction::Horizontal } else { Direction::Vertical });
            if along { push("flex", "1 0 0".into()) } else { push("align-self", "stretch".into()) }
        }
    };
    size(&mut push, "width", p.sizing_h, p.width, true);
    size(&mut push, "height", p.sizing_v, p.height, false);

    if let Some(l) = &p.layout {
        push("display", "flex".into());
        push("flex-direction", if l.direction == Direction::Horizontal { "row" } else { "column" }.into());
        if l.wrap {
            push("flex-wrap", "wrap".into());
        }
        push(
            "justify-content",
            match l.align {
                Align::Start => "flex-start",
                Align::Center => "center",
                Align::End => "flex-end",
                Align::SpaceBetween => "space-between",
            }
            .into(),
        );
        push(
            "align-items",
            match l.counter_align {
                CounterAlign::Start => "flex-start",
                CounterAlign::Center => "center",
                CounterAlign::End => "flex-end",
            }
            .into(),
        );
        if l.align != Align::SpaceBetween && l.gap != 0.0 {
            push("gap", bound(doc, n, "gap", px(l.gap)));
        }
        let pd = l.padding;
        if pd.top != 0.0 || pd.right != 0.0 || pd.bottom != 0.0 || pd.left != 0.0 {
            let v = if pd.top == pd.bottom && pd.left == pd.right {
                if pd.top == pd.left { px(pd.top) } else { format!("{} {}", px(pd.top), px(pd.left)) }
            } else {
                format!("{} {} {} {}", px(pd.top), px(pd.right), px(pd.bottom), px(pd.left))
            };
            push("padding", bound(doc, n, "padding", v));
        }
    } else if n.kind.is_container() && !doc.children(id).is_empty() {
        // Children are placed absolutely inside.
        push("position", "relative".into());
    }

    let is_text = matches!(n.kind, NodeKind::Text { .. });
    let fills: Vec<(usize, String)> = p.fills.iter().enumerate().filter_map(|(i, f)| paint_css(f).map(|c| (i, c))).collect();
    if let Some((i, top)) = fills.last() {
        let v = bound(doc, n, &format!("fills/{i}"), top.clone());
        if is_text {
            push("color", v);
        } else if fills.len() == 1 && !top.contains("gradient") && !top.starts_with("url") {
            push("background", v);
        } else {
            // CSS lists backgrounds top first; Blobr's last fill is on top.
            let layers: Vec<String> = fills.iter().rev().map(|(_, c)| if c.contains('(') && !c.starts_with("rgba") { c.clone() } else { format!("linear-gradient({c}, {c})") }).collect();
            push("background", layers.join(", "));
        }
    }
    if let Some(stroke) = p.strokes.iter().rev().find_map(paint_css)
        && p.stroke.weight > 0.0
    {
        push("border", format!("{} solid {}", bound(doc, n, "stroke_weight", px(p.stroke.weight as f64)), stroke));
    }
    let c = p.corner;
    if c.top_left != 0.0 || c.top_right != 0.0 || c.bottom_right != 0.0 || c.bottom_left != 0.0 {
        let v = if c.top_left == c.top_right && c.top_left == c.bottom_right && c.top_left == c.bottom_left {
            px(c.top_left)
        } else {
            format!("{} {} {} {}", px(c.top_left), px(c.top_right), px(c.bottom_right), px(c.bottom_left))
        };
        push("border-radius", bound(doc, n, "corner", v));
    } else if matches!(n.kind, NodeKind::Ellipse { .. }) {
        push("border-radius", "50%".into());
    }
    let shadows: Vec<String> = p
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::DropShadow(s) if s.visible => Some(format!("{} {} {} {} {}", px(s.x as f64), px(s.y as f64), px(s.blur as f64), px(s.spread as f64), css_color(s.color))),
            Effect::InnerShadow(s) if s.visible => Some(format!("inset {} {} {} {} {}", px(s.x as f64), px(s.y as f64), px(s.blur as f64), px(s.spread as f64), css_color(s.color))),
            _ => None,
        })
        .collect();
    if !shadows.is_empty() {
        push(if is_text { "text-shadow" } else { "box-shadow" }, shadows.join(", "));
    }
    for e in &p.effects {
        match e {
            Effect::LayerBlur { radius, visible: true } => push("filter", format!("blur({})", px(*radius as f64 / 2.0))),
            Effect::BackgroundBlur { radius, visible: true } => push("backdrop-filter", format!("blur({})", px(*radius as f64 / 2.0))),
            _ => {}
        }
    }
    if p.opacity < 1.0 {
        push("opacity", bound(doc, n, "opacity", format!("{:.2}", p.opacity)));
    }
    if p.rotation != 0.0 {
        // Blobr turns counter-clockwise for positive angles; CSS turns clockwise.
        push("transform", format!("rotate({}deg)", (-p.rotation * 100.0).round() / 100.0));
        push("transform-origin", "top left".into());
    }
    if p.clip && n.kind.is_container() {
        push("overflow", "hidden".into());
    }
    if let NodeKind::Text { text } = &n.kind {
        let st = &text.style;
        if !st.family.is_empty() {
            push("font-family", format!("\"{}\", sans-serif", st.family));
        }
        push("font-size", bound(doc, n, "font_size", px(st.size as f64)));
        let (w, italic) = weight(&st.style);
        if w != 400 {
            push("font-weight", w.to_string());
        }
        if italic {
            push("font-style", "italic".into());
        }
        match st.line_height {
            LineHeight::Auto => {}
            LineHeight::Px(v) => push("line-height", px(v as f64)),
            LineHeight::Percent(v) => push("line-height", format!("{}%", v)),
        }
        if st.letter_spacing != 0.0 {
            push("letter-spacing", px(st.letter_spacing as f64));
        }
        match st.align {
            TextAlign::Left => {}
            TextAlign::Center => push("text-align", "center".into()),
            TextAlign::Right => push("text-align", "right".into()),
            TextAlign::Justified => push("text-align", "justify".into()),
        }
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn class_name(n: &Node) -> String {
    let mut s = var_name(&n.name);
    s.drain(..2);
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert_str(0, "layer-");
    }
    s
}

/// HTML for a layer and everything in it, styled with a class per layer, plus the stylesheet
/// (with the variables the layers use, at their values in the first mode).
pub fn html(doc: &Document, id: NodeId) -> String {
    let mut body = String::new();
    let mut styles = String::new();
    let mut used: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    fn walk(doc: &Document, id: NodeId, depth: usize, body: &mut String, styles: &mut String, used: &mut std::collections::BTreeMap<String, String>, seen: &mut std::collections::BTreeMap<String, usize>) {
        let Some(n) = doc.node(id) else { return };
        if !n.props.visible {
            return;
        }
        let base = class_name(n);
        let count = seen.entry(base.clone()).or_insert(0);
        *count += 1;
        let class = if *count == 1 { base } else { format!("{base}-{count}") };
        let _ = writeln!(styles, ".{class} {{");
        for d in css(doc, id) {
            let _ = writeln!(styles, "  {d}");
        }
        let _ = writeln!(styles, "}}");
        for (path, var) in &n.props.bind {
            if let (Some(v), Some(value)) = (doc.tokens().variables.get(var), doc.resolve_variable(*var, id)) {
                let _ = path;
                let shown = match value {
                    crate::VarValue::Color(c) => css_color(c),
                    crate::VarValue::Number(x) => px(x),
                    crate::VarValue::Text(t) => format!("\"{t}\""),
                    crate::VarValue::Bool(b) => b.to_string(),
                    crate::VarValue::Alias(_) => continue,
                };
                used.insert(var_name(&v.name), shown);
            }
        }
        let pad = "  ".repeat(depth);
        match &n.kind {
            NodeKind::Text { text } => {
                let _ = writeln!(body, "{pad}<p class=\"{class}\">{}</p>", escape(&text.content).replace('\n', "<br>"));
            }
            NodeKind::Vector { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. } | NodeKind::Line => {
                let _ = writeln!(body, "{pad}<div class=\"{class}\"><!-- {}: export as SVG --></div>", n.kind.type_name());
            }
            _ if doc.children(id).is_empty() => {
                let _ = writeln!(body, "{pad}<div class=\"{class}\"></div>");
            }
            _ => {
                let _ = writeln!(body, "{pad}<div class=\"{class}\">");
                for &c in doc.children(id) {
                    walk(doc, c, depth + 1, body, styles, used, seen);
                }
                let _ = writeln!(body, "{pad}</div>");
            }
        }
    }
    walk(doc, id, 0, &mut body, &mut styles, &mut used, &mut Default::default());
    let mut root = String::new();
    if !used.is_empty() {
        root.push_str(":root {\n");
        for (k, v) in &used {
            let _ = writeln!(root, "  {k}: {v};");
        }
        root.push_str("}\n");
    }
    format!("<style>\n{root}{styles}</style>\n{body}")
}

fn rust_color(c: Rgba) -> String {
    let [r, g, b] = c.to_srgb8();
    if c.a >= 0.999 { format!("Color::from_rgb8(0x{r:02X}, 0x{g:02X}, 0x{b:02X})") } else { format!("Color::from_rgba8(0x{r:02X}, 0x{g:02X}, 0x{b:02X}, {:.2})", c.a) }
}

/// An iced 0.14 snippet for one layer (its children as a placeholder).
pub fn iced(doc: &Document, id: NodeId) -> String {
    let Some(n) = doc.node(id) else { return String::new() };
    let p = &n.props;
    let len = |s: Sizing, v: f64| match s {
        Sizing::Fixed => format!("{v}"),
        Sizing::Hug => "Length::Shrink".into(),
        Sizing::Fill => "Length::Fill".into(),
    };
    let solid = p.fills.iter().rev().find_map(|f| match f.kind {
        PaintKind::Solid { color } if f.visible => Some(color),
        _ => None,
    });
    if let NodeKind::Text { text } = &n.kind {
        let mut s = format!("text({:?})\n    .size({})", text.content, text.style.size);
        if let Some(c) = solid {
            let _ = write!(s, "\n    .color({})", rust_color(c));
        }
        return s;
    }
    let inner = match &p.layout {
        Some(l) => {
            let kind = if l.direction == Direction::Horizontal { "row!" } else { "column!" };
            let mut s = format!("{kind}[/* {} children */]\n        .spacing({})", doc.children(id).len(), l.gap);
            let pd = l.padding;
            if pd != Default::default() {
                let _ = write!(s, "\n        .padding(Padding {{ top: {}.0, right: {}.0, bottom: {}.0, left: {}.0 }})", pd.top, pd.right, pd.bottom, pd.left);
            }
            s
        }
        None => "space()".into(),
    };
    let mut s = format!("container(\n    {inner}\n)\n.width({})\n.height({})", len(p.sizing_h, p.width), len(p.sizing_v, p.height));
    let stroke = p.strokes.iter().rev().find_map(|f| match f.kind {
        PaintKind::Solid { color } if f.visible => Some(color),
        _ => None,
    });
    let _ = write!(s, "\n.style(|_| container::Style {{");
    if let Some(c) = solid {
        let _ = write!(s, "\n    background: Some({}.into()),", rust_color(c));
    }
    let r = p.corner.top_left;
    if r != 0.0 || stroke.is_some() {
        let width = if stroke.is_some() { p.stroke.weight } else { 0.0 };
        let color = stroke.map_or("Color::TRANSPARENT".into(), rust_color);
        let _ = write!(s, "\n    border: Border {{ radius: {r}.into(), width: {width}, color: {color} }},");
    }
    let _ = write!(s, "\n    ..Default::default()\n}})");
    s
}

/// `BG_APP` from a variable name like `bg/app`.
fn const_name(name: &str) -> String {
    let s = var_name(name);
    let s = s.trim_start_matches('-').replace('-', "_").to_uppercase();
    if s.starts_with(|c: char| c.is_ascii_digit()) { format!("V_{s}") } else { s }
}

/// The document's colour and number variables as Rust constants, the way `jd-ui`'s
/// `theme.rs` writes its tokens (first mode's values; one module per collection).
pub fn rust_tokens(doc: &Document) -> String {
    let t = doc.tokens();
    let mut out = String::new();
    for (&cid, c) in &t.collections {
        let module = const_name(&c.name).to_lowercase();
        let _ = writeln!(out, "/// {} ({} mode).\npub mod {module} {{\n    use iced::Color;\n", c.name, c.modes[0]);
        for v in t.variables.values().filter(|v| v.collection == cid) {
            // Follow aliases to a value.
            let mut value = v.values.get(&c.modes[0]).cloned();
            for _ in 0..16 {
                match value {
                    Some(crate::VarValue::Alias(a)) => value = t.variables.get(&a).and_then(|a| a.values.values().next().cloned()),
                    _ => break,
                }
            }
            match value {
                Some(crate::VarValue::Color(col)) => {
                    let _ = writeln!(out, "    pub const {}: Color = {};", const_name(&v.name), rust_color(col));
                }
                Some(crate::VarValue::Number(n)) => {
                    let _ = writeln!(out, "    pub const {}: f32 = {n:?};", const_name(&v.name));
                }
                _ => {}
            }
        }
        out.push_str("}\n\n");
    }
    out.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApproxMeasure, AutoLayout, Op, Padding, Paint, TextStyle, VarValue};
    use crate::{NodeSpec, Patch};

    #[test]
    fn css_html_and_iced_for_an_auto_layout_card() {
        let mut doc = Document::new();
        let page = doc.pages()[0];
        let mut card = NodeSpec::frame("Card", 200.0, 100.0).fill(Paint::hex("#FFFFFF"));
        card.props.layout = Some(AutoLayout { gap: 8.0, padding: Padding::all(16.0), direction: Direction::Vertical, ..AutoLayout::default() });
        card.props.corner = crate::CornerRadii::all(12.0);
        let done = doc
            .apply(
                &[
                    Op::create_as("card", page, card),
                    Op::create_as("t", "card", NodeSpec::text("Title", "Hi <you>", TextStyle { style: "Semi Bold".into(), size: 18.0, ..TextStyle::default() })),
                    Op::CreateCollection { name: "Theme".into(), modes: vec!["Light".into()], key: Some("c".into()) },
                    Op::CreateVariable { collection: "c".into(), name: "Brand/Primary".into(), values: [("Light".into(), VarValue::Color(Rgba::from_hex("4C8DFF").unwrap()))].into(), key: Some("v".into()) },
                ],
                &ApproxMeasure,
            )
            .unwrap();
        let (card, t, v) = (done.id("card").unwrap(), done.id("t").unwrap(), done.id("v").unwrap());
        doc.apply(&[Op::set(t, Patch { bind: Some([("fills/0".into(), Some(v))].into()), ..Patch::default() })], &ApproxMeasure).unwrap();

        let c = css(&doc, card);
        for want in ["display: flex;", "flex-direction: column;", "gap: 8px;", "padding: 16px;", "border-radius: 12px;", "background: #FFFFFF;"] {
            assert!(c.contains(&want.to_string()), "missing {want}: {c:?}");
        }
        assert!(!c.iter().any(|d| d.starts_with("position")), "top-level frames aren't positioned");
        let tc = css(&doc, t);
        assert!(tc.contains(&"color: var(--brand-primary, #4C8DFF);".to_string()), "{tc:?}");
        assert!(tc.contains(&"font-weight: 600;".to_string()));

        let h = html(&doc, card);
        assert!(h.contains("--brand-primary: #4C8DFF;"), "{h}");
        assert!(h.contains("<p class=\"title\">Hi &lt;you&gt;</p>"), "{h}");
        let r = rust_tokens(&doc);
        assert!(r.contains("pub mod theme {") && r.contains("pub const BRAND_PRIMARY: Color = Color::from_rgb8(0x4C, 0x8D, 0xFF);"), "{r}");
        let i = iced(&doc, card);
        assert!(i.contains("column![") && i.contains(".spacing(8)") && i.contains("Color::from_rgb8(0xFF, 0xFF, 0xFF)"), "{i}");
    }
}
