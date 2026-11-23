//! Builds Blobr's main-window mockup from ops, components included, saves it as a `.jdb`
//! and renders the window to PNG.
//!
//! cargo run -p blobr-mcp --example blobr_window -- out.jdb out.png
//!
//! The ops are in `blobr_window.json` (written by `blobr_window.py`), in the same format an
//! agent sends to the MCP server's `apply_ops`: icon components first, then the other
//! components and the window, built from their instances.

use std::collections::HashMap;
use std::path::Path;

use blobr_mcp::{FileTarget, Target, parse_ops};
use jd_vector::NodeId;
use serde_json::Value;

pub const OPS: &str = include_str!("blobr_window.json");

/// Applies both batches to a new document; gives back the window's id.
pub fn build(target: &mut FileTarget) -> Result<NodeId, String> {
    let all: Value = serde_json::from_str(OPS).map_err(|e| e.to_string())?;
    let icons = target.apply("Icons", &parse_ops(target.doc(), batch(&all, "icons", &HashMap::new())?)?)?;
    let ids: HashMap<String, u64> = icons.keys.iter().map(|(k, id)| (format!("${k}"), id.0)).collect();
    let done = target.apply("Blobr window", &parse_ops(target.doc(), batch(&all, "window", &ids)?)?)?;
    done.id("win").ok_or_else(|| "no window".into())
}

/// One batch, with `"$key"` strings swapped for the ids the first batch gave those keys.
fn batch(all: &Value, name: &str, ids: &HashMap<String, u64>) -> Result<Vec<Value>, String> {
    fn fill(v: &mut Value, ids: &HashMap<String, u64>) -> Result<(), String> {
        match v {
            Value::String(s) if s.starts_with('$') => {
                *v = Value::from(*ids.get(s.as_str()).ok_or_else(|| format!("unknown {s}"))?);
            }
            Value::Array(a) => a.iter_mut().try_for_each(|x| fill(x, ids))?,
            Value::Object(m) => m.values_mut().try_for_each(|x| fill(x, ids))?,
            _ => {}
        }
        Ok(())
    }
    let mut ops = all.get(name).cloned().ok_or_else(|| format!("no {name} batch"))?;
    fill(&mut ops, ids)?;
    serde_json::from_value(ops).map_err(|e| e.to_string())
}

#[allow(dead_code)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [jdb, png] = args.as_slice() else {
        eprintln!("usage: blobr_window <out.jdb> <out.png>");
        std::process::exit(2);
    };
    let mut target = FileTarget::default();
    let mut run = || -> Result<(), String> {
        let win = build(&mut target)?;
        jd_vector::file::save(target.doc(), Path::new(jdb)).map_err(|e| e.to_string())?;
        let bytes = jd_vector_render::render_png(target.doc(), win, jd_vector_render::RenderOptions::default())?;
        std::fs::write(png, bytes).map_err(|e| e.to_string())
    };
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
    println!("Wrote {jdb} and {png}");
}
