//! Blobr's MCP tools. They work on a [`Target`]: a document plus its undo history, either a
//! file opened headless by the `blobr-mcp` server ([`FileTarget`]) or the document open in
//! the running Blobr app (the app implements [`Target`] and runs the same [`call`], so both
//! modes behave alike).

use std::path::{Path, PathBuf};

use jd_mcp::{Content, Tool, ToolResult, args};
use jd_vector::{Applied, Document, Editor, NodeId, NodeKind, Op, Rgba};
use jd_vector_render::{CosmicMeasure, RenderOptions};
use kurbo::Shape;
use serde_json::{Map, Value, json};

pub mod query;

/// The long reference the `reference` tool returns.
pub const REFERENCE: &str = include_str!("reference.md");

/// Told to the model once per session.
pub const INSTRUCTIONS: &str = "Blobr is a vector design app (like Figma) for the jamsdesign suite. These tools read and \
edit Blobr documents (.jdb). When the Blobr app is running, they work on the document open in it, live, with every \
change an undo step there; otherwise on a file opened here (open_document / new_document, then save_document). \
Read the `reference` tool once before building anything: it has the node and op formats. Change documents with \
`apply_ops` (atomic batches; created nodes get keys you can refer to), look with `get_tree`, `query` and `get_node`, \
and check your work with `render`.";

pub const NEEDS_APP: &str = "this needs the running Blobr app (start Blobr; the server uses it automatically while it runs)";

/// A document being edited, with whatever else the tools need from its host.
pub trait Target {
    fn doc(&self) -> &Document;
    /// Runs `ops` as one undo step named `label`.
    fn apply(&mut self, label: &str, ops: &[Op]) -> Result<Applied, String>;
    fn undo(&mut self) -> Result<String, String>;
    fn path(&self) -> Option<PathBuf>;
    fn dirty(&self) -> bool;
    fn new_document(&mut self, path: Option<PathBuf>) -> Result<(), String>;
    fn open(&mut self, path: &Path) -> Result<(), String>;
    /// Saves to `path`, or where the document came from. Returns where it went.
    fn save(&mut self, path: Option<&Path>) -> Result<PathBuf, String>;

    /// Whether this is the running app.
    fn live(&self) -> bool {
        false
    }
    fn selection(&self) -> Result<Vec<NodeId>, String> {
        Err(NEEDS_APP.into())
    }
    fn select(&mut self, _ids: &[NodeId]) -> Result<(), String> {
        Err(NEEDS_APP.into())
    }
    /// Shows `id` on the canvas (scrolls and zooms to it).
    fn focus(&mut self, _id: NodeId) -> Result<(), String> {
        Err(NEEDS_APP.into())
    }
    /// What the canvas shows right now, as PNG.
    fn view_png(&mut self) -> Result<Vec<u8>, String> {
        Err(NEEDS_APP.into())
    }
}

/// A document opened headless, from a file or new.
pub struct FileTarget {
    pub editor: Editor,
    pub path: Option<PathBuf>,
}

impl Default for FileTarget {
    fn default() -> Self {
        Self { editor: Editor::new(Document::new()), path: None }
    }
}

impl Target for FileTarget {
    fn doc(&self) -> &Document {
        &self.editor.doc
    }

    fn apply(&mut self, label: &str, ops: &[Op]) -> Result<Applied, String> {
        self.editor.apply(label, ops, &CosmicMeasure).map_err(|e| e.to_string())
    }

    fn undo(&mut self) -> Result<String, String> {
        self.editor.undo().ok_or_else(|| "nothing to undo".into())
    }

    fn path(&self) -> Option<PathBuf> {
        self.path.clone()
    }

    fn dirty(&self) -> bool {
        self.editor.history.is_dirty()
    }

    fn new_document(&mut self, path: Option<PathBuf>) -> Result<(), String> {
        self.editor = Editor::new(Document::new());
        self.path = path;
        Ok(())
    }

    fn open(&mut self, path: &Path) -> Result<(), String> {
        let doc = jd_vector_render::place::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.editor = Editor::new(doc);
        self.path = Some(path.to_owned());
        Ok(())
    }

    fn save(&mut self, path: Option<&Path>) -> Result<PathBuf, String> {
        let path = path.map(Path::to_owned).or_else(|| self.path.clone()).ok_or("no file yet: give a path")?;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        jd_vector::file::save(&self.editor.doc, &path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.editor.history.mark_saved();
        self.path = Some(path.clone());
        Ok(path)
    }
}

fn schema(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required })
}

fn tool(name: &'static str, read_only: bool, description: &str, input_schema: Value) -> Tool {
    Tool { name, description: description.to_owned(), input_schema, read_only }
}

/// Every tool, as clients see them.
pub fn tools() -> Vec<Tool> {
    let id = json!({ "type": "integer", "description": "A node id" });
    let ids = json!({ "type": "array", "items": { "type": "integer" }, "description": "Node ids" });
    let path = json!({ "type": "string", "description": "A .jdb file path" });
    vec![
        tool("reference", true, "The Blobr document and op reference: node types, properties, paints, auto layout, text, components, variables. Read it before building.", schema(json!({}), &[])),
        tool(
            "document_info",
            true,
            "Which document is being edited (file, unsaved changes, live in the app or headless), its pages with ids, and token counts.",
            schema(json!({}), &[]),
        ),
        tool("new_document", false, "Start a new document (one empty page). Headless only.", schema(json!({ "path": path }), &[])),
        tool("open_document", false, "Open a .jdb file. Headless only (in the app, open files there).", schema(json!({ "path": path }), &["path"])),
        tool("save_document", false, "Save the document (to `path`, or where it came from).", schema(json!({ "path": path }), &[])),
        tool(
            "get_tree",
            true,
            "An outline of a page or node: id, type, name, size, position and auto layout per line, indented. Starts at the first page by default.",
            schema(
                json!({
                    "id": id,
                    "depth": { "type": "integer", "description": "Levels to show (default 3)" },
                    "max_nodes": { "type": "integer", "description": "Lines at most (default 300)" },
                }),
                &[],
            ),
        ),
        tool(
            "query",
            true,
            "Find nodes with a CSS-like selector: types (frame, text, *), #id, attributes [name=..] [name*=..] [name^=..] [name$=..] [text*=..] [visible=false] [component=..], descendant (space) and child (>) combinators, commas. Searches every page unless `root` is given.",
            schema(
                json!({
                    "selector": { "type": "string" },
                    "root": id,
                    "limit": { "type": "integer", "description": "Results at most (default 100)" },
                }),
                &["selector"],
            ),
        ),
        tool(
            "get_node",
            true,
            "Every property of the given nodes (as JSON in the op format), with their children's ids and page bounds. Vector paths come as `svg_path`.",
            schema(json!({ "ids": ids }), &["ids"]),
        ),
        tool(
            "apply_ops",
            false,
            "Change the document: a list of ops run as one atomic batch and one undo step (see `reference`). Returns created ids (by key) and what changed. If any op fails, nothing is applied.",
            schema(
                json!({
                    "ops": { "type": "array", "items": { "type": "object" } },
                    "label": { "type": "string", "description": "Undo step name (default \"Agent edit\")" },
                }),
                &["ops"],
            ),
        ),
        tool(
            "arrange",
            false,
            "Group, ungroup, frame, align, distribute, restack or combine layers. action: group | frame (wrap in a frame) | ungroup | align (to: left, center, right, top, middle, bottom) | distribute (horizontal: bool) | order (to: front, forward, backward, back) | union | subtract | intersect | exclude | flatten.",
            schema(
                json!({
                    "action": { "type": "string" },
                    "ids": ids,
                    "to": { "type": "string" },
                    "horizontal": { "type": "boolean" },
                }),
                &["action", "ids"],
            ),
        ),
        tool(
            "import_svg",
            false,
            "Add SVG markup as editable layers (a frame the size of the SVG holding vectors, groups and text as outlines). Good for icons and illustrations.",
            schema(
                json!({
                    "svg": { "type": "string" },
                    "parent": { "type": "integer", "description": "Page or frame id (default the first page)" },
                    "x": { "type": "number" }, "y": { "type": "number" },
                    "name": { "type": "string" },
                }),
                &["svg"],
            ),
        ),
        tool(
            "render",
            true,
            "A PNG of a node (or a whole page) to check your work.",
            schema(
                json!({
                    "id": id,
                    "scale": { "type": "number", "description": "Pixels per design pixel (default: fit max_size)" },
                    "max_size": { "type": "integer", "description": "Longest side in pixels (default 1024)" },
                    "background": { "type": "string", "description": "Hex colour behind it (default transparent; pages use their colour)" },
                }),
                &["id"],
            ),
        ),
        tool(
            "export",
            false,
            "Write nodes to a file: png (one node, `scale`), svg (one node) or pdf (a page per node).",
            schema(
                json!({
                    "ids": ids,
                    "format": { "type": "string", "enum": ["png", "svg", "pdf"] },
                    "path": { "type": "string" },
                    "scale": { "type": "number" },
                }),
                &["ids", "format", "path"],
            ),
        ),
        tool("list_fonts", true, "Installed font families (for text styles).", schema(json!({ "filter": { "type": "string", "description": "Only names containing this (any case)" } }), &[])),
        tool("get_tokens", true, "The document's variable collections, variables and styles, with ids.", schema(json!({}), &[])),
        tool(
            "inspect_code",
            true,
            "A node as code: css (its declarations), html (with children) or iced (Rust); `tokens` gives the variables as Rust constants.",
            schema(json!({ "id": id, "format": { "type": "string", "enum": ["css", "html", "iced", "tokens"] } }), &["format"]),
        ),
        tool("undo", false, "Undo the last change.", schema(json!({}), &[])),
        tool("get_selection", true, "The layers selected in the app (live only).", schema(json!({}), &[])),
        tool("select", false, "Select layers in the app (live only).", schema(json!({ "ids": ids }), &["ids"])),
        tool("focus_node", false, "Scroll and zoom the app's canvas to a node (live only).", schema(json!({ "id": id }), &["id"])),
        tool("render_view", true, "A PNG of exactly what the app's canvas shows now (live only).", schema(json!({}), &[])),
    ]
}

fn text(s: impl Into<String>) -> ToolResult {
    Ok(vec![Content::Text(s.into())])
}

fn node_id(doc: &Document, args: &Value, key: &str) -> Result<NodeId, String> {
    let id = NodeId(args::get::<u64>(args, key)?);
    if doc.contains(id) { Ok(id) } else { Err(format!("there is no node {id}")) }
}

fn node_ids(doc: &Document, args: &Value) -> Result<Vec<NodeId>, String> {
    let ids: Vec<u64> = args::get(args, "ids")?;
    ids.into_iter()
        .map(|i| {
            let id = NodeId(i);
            if doc.contains(id) { Ok(id) } else { Err(format!("there is no node {id}")) }
        })
        .collect()
}

/// Runs one tool.
pub fn call(target: &mut dyn Target, name: &str, args: Value) -> ToolResult {
    match name {
        "reference" => text(REFERENCE),
        "document_info" => {
            let doc = target.doc();
            let pages: Vec<Value> = doc
                .pages()
                .iter()
                .map(|&p| json!({ "id": p.0, "name": doc.node(p).map_or("", |n| n.name.as_str()), "children": doc.children(p).len() }))
                .collect();
            let t = doc.tokens();
            let mut info = json!({
                "mode": if target.live() { "live (the Blobr app)" } else { "headless file" },
                "path": target.path().map(|p| p.display().to_string()),
                "unsaved_changes": target.dirty(),
                "pages": pages,
                "collections": t.collections.len(),
                "variables": t.variables.len(),
                "styles": t.styles.len(),
            });
            if let Ok(sel) = target.selection() {
                info["selection"] = json!(sel.iter().map(|i| i.0).collect::<Vec<_>>());
            }
            text(serde_json::to_string_pretty(&info).expect("json"))
        }
        "new_document" => {
            if target.live() {
                return Err("in the app, start new documents there".into());
            }
            target.new_document(args::opt::<PathBuf>(&args, "path")?)?;
            text(format!("New document; page 1 has id {}.", target.doc().pages()[0].0))
        }
        "open_document" => {
            if target.live() {
                return Err("in the app, open files there (File > Open); these tools follow the open document".into());
            }
            let path: PathBuf = args::get(&args, "path")?;
            target.open(&path)?;
            call(target, "document_info", json!({}))
        }
        "save_document" => {
            let path = args::opt::<PathBuf>(&args, "path")?;
            let saved = target.save(path.as_deref())?;
            text(format!("Saved {}", saved.display()))
        }
        "get_tree" => {
            let doc = target.doc();
            let root = match args.get("id") {
                Some(Value::Null) | None => doc.pages()[0],
                _ => node_id(doc, &args, "id")?,
            };
            let depth = args::opt::<usize>(&args, "depth")?.unwrap_or(3);
            let max = args::opt::<usize>(&args, "max_nodes")?.unwrap_or(300);
            let mut out = String::new();
            let mut lines = 0;
            outline(doc, root, 0, depth, max, &mut lines, &mut out);
            text(out)
        }
        "query" => {
            let doc = target.doc();
            let sel = query::Selector::parse(&args::get::<String>(&args, "selector")?)?;
            let limit = args::opt::<usize>(&args, "limit")?.unwrap_or(100);
            let roots = match args.get("root") {
                Some(Value::Null) | None => doc.pages().to_vec(),
                _ => vec![node_id(doc, &args, "root")?],
            };
            let found: Vec<NodeId> = roots.iter().flat_map(|&r| sel.find(doc, r)).collect();
            let mut out: String = found.iter().take(limit).map(|&id| format!("{}\n", line(doc, id))).collect();
            if found.len() > limit {
                out.push_str(&format!("... and {} more (raise `limit`)\n", found.len() - limit));
            }
            if found.is_empty() {
                out = "No matches.".into();
            }
            text(out)
        }
        "get_node" => {
            let doc = target.doc();
            let nodes: Vec<Value> = node_ids(doc, &args)?.into_iter().map(|id| node_json(doc, id)).collect();
            text(serde_json::to_string_pretty(&nodes).expect("json"))
        }
        "apply_ops" => {
            let raw: Vec<Value> = args::get(&args, "ops")?;
            let label = args::opt::<String>(&args, "label")?.unwrap_or_else(|| "Agent edit".into());
            let ops = parse_ops(target.doc(), raw)?;
            let done = target.apply(&label, &ops)?;
            text(applied(&done))
        }
        "arrange" => arrange(target, &args),
        "import_svg" => {
            let doc = target.doc();
            let svg: String = args::get(&args, "svg")?;
            let parent = match args.get("parent") {
                Some(Value::Null) | None => doc.pages()[0],
                _ => node_id(doc, &args, "parent")?,
            };
            let x = args::opt::<f64>(&args, "x")?.unwrap_or(0.0);
            let y = args::opt::<f64>(&args, "y")?.unwrap_or(0.0);
            let name = args::opt::<String>(&args, "name")?.unwrap_or_else(|| "SVG".into());
            let ops = jd_vector_render::import_svg(svg.as_bytes(), parent.into(), &name, x, y)?;
            let done = target.apply("Import SVG", &ops)?;
            let frame = done.id("svg").map_or(0, |i| i.0);
            text(format!("Imported as frame {frame} ({} layers).", done.created.len()))
        }
        "render" => {
            let doc = target.doc();
            let id = node_id(doc, &args, "id")?;
            let background = match args::opt::<String>(&args, "background")? {
                Some(h) => Some(jd_vector::color::from_hex(&h).ok_or_else(|| format!("bad colour {h:?}"))?),
                None => None,
            };
            Ok(vec![Content::Png(render(doc, id, args::opt(&args, "scale")?, args::opt(&args, "max_size")?, background)?)])
        }
        "export" => {
            let doc = target.doc();
            let ids = node_ids(doc, &args)?;
            let path: PathBuf = args::get(&args, "path")?;
            let format: String = args::get(&args, "format")?;
            let bytes = match format.as_str() {
                "pdf" => jd_vector_render::export_pdf(doc, &ids)?,
                "svg" => jd_vector_render::export_svg(doc, *ids.first().ok_or("no ids")?).ok_or("nothing to export")?.into_bytes(),
                "png" => {
                    let scale = args::opt::<f64>(&args, "scale")?.unwrap_or(1.0);
                    jd_vector_render::render_png(doc, *ids.first().ok_or("no ids")?, RenderOptions { scale, background: None })?
                }
                f => return Err(format!("unknown format {f:?}")),
            };
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            text(format!("Wrote {} ({} bytes)", path.display(), bytes.len()))
        }
        "list_fonts" => {
            let filter = args::opt::<String>(&args, "filter")?.map(|f| f.to_lowercase());
            let mut fams = jd_vector_render::families();
            fams.retain(|f| filter.as_ref().is_none_or(|q| f.to_lowercase().contains(q)));
            fams.sort();
            fams.dedup();
            text(if fams.is_empty() { "No matching fonts.".into() } else { fams.join("\n") })
        }
        "get_tokens" => text(serde_json::to_string_pretty(target.doc().tokens()).expect("json")),
        "inspect_code" => {
            let doc = target.doc();
            let format: String = args::get(&args, "format")?;
            if format == "tokens" {
                return text(jd_vector::inspect::rust_tokens(doc));
            }
            let id = node_id(doc, &args, "id")?;
            text(match format.as_str() {
                "css" => jd_vector::inspect::css(doc, id).join("\n"),
                "html" => jd_vector::inspect::html(doc, id),
                "iced" => jd_vector::inspect::iced(doc, id),
                f => return Err(format!("unknown format {f:?}")),
            })
        }
        "undo" => text(format!("Undid \"{}\"", target.undo()?)),
        "get_selection" => {
            let sel = target.selection()?;
            let doc = target.doc();
            text(if sel.is_empty() { "Nothing selected.".to_owned() } else { sel.iter().map(|&i| format!("{}\n", line(doc, i))).collect::<String>() })
        }
        "select" => {
            let ids = node_ids(target.doc(), &args)?;
            target.select(&ids)?;
            text(format!("Selected {} layer(s).", ids.len()))
        }
        "focus_node" => {
            let id = node_id(target.doc(), &args, "id")?;
            target.focus(id)?;
            text("Done.")
        }
        "render_view" => Ok(vec![Content::Png(target.view_png()?)]),
        _ => Err(format!("unknown tool {name:?}")),
    }
}

/// One line about a node: `#12 frame "Card" 320x180 at 40,40`.
fn line(doc: &Document, id: NodeId) -> String {
    let Some(n) = doc.node(id) else { return format!("#{id} (gone)") };
    let p = &n.props;
    let mut s = format!("#{} {} {:?}", id.0, n.kind.type_name(), n.name);
    if !n.is_page() {
        s.push_str(&format!(" {}x{} at {},{}", num(p.width), num(p.height), num(p.x), num(p.y)));
    }
    if let Some(l) = &p.layout {
        let dir = if l.direction == jd_vector::Direction::Vertical { "vertical" } else { "horizontal" };
        s.push_str(&format!(" [auto {dir} gap {}]", num(l.gap)));
    }
    match &n.kind {
        NodeKind::Text { text } => {
            let t: String = text.content.chars().take(60).collect();
            s.push_str(&format!(" text={t:?}"));
        }
        NodeKind::Instance { component, .. } => {
            s.push_str(&format!(" of {:?}", doc.node(*component).map_or("?", |c| c.name.as_str())));
        }
        _ => {}
    }
    if !p.visible {
        s.push_str(" hidden");
    }
    if p.locked {
        s.push_str(" locked");
    }
    s
}

fn num(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{r}") }
}

fn outline(doc: &Document, id: NodeId, level: usize, depth: usize, max: usize, lines: &mut usize, out: &mut String) {
    if *lines >= max {
        return;
    }
    *lines += 1;
    out.push_str(&"  ".repeat(level));
    out.push_str(&line(doc, id));
    out.push('\n');
    let kids = doc.children(id);
    if kids.is_empty() {
        return;
    }
    if level + 1 >= depth {
        out.push_str(&format!("{}  ({} children; get_tree with id {} to see them)\n", "  ".repeat(level), kids.len(), id.0));
        return;
    }
    // Top of the stack first, as layer panels show them.
    for &c in kids.iter().rev() {
        if *lines >= max {
            out.push_str(&format!("{}... (max_nodes reached)\n", "  ".repeat(level + 1)));
            return;
        }
        outline(doc, c, level + 1, depth, max, lines, out);
    }
}

/// A node in the op format, plus where it is.
fn node_json(doc: &Document, id: NodeId) -> Value {
    let n = doc.node(id).expect("checked");
    let mut m = Map::new();
    m.insert("id".into(), json!(id.0));
    m.insert("name".into(), json!(n.name));
    m.insert("parent".into(), json!(n.parent.map(|p| p.0)));
    if let Value::Object(kind) = serde_json::to_value(&n.kind).expect("json") {
        m.extend(kind);
    }
    if let NodeKind::Vector { path } = &n.kind {
        m.remove("path");
        m.insert("svg_path".into(), json!(path.to_bez(n.props.width, n.props.height).to_svg()));
        if path.fill_rule == jd_vector::FillRule::EvenOdd {
            m.insert("fill_rule".into(), json!("even_odd"));
        }
    }
    if !n.is_page()
        && let Value::Object(props) = serde_json::to_value(&n.props).expect("json")
    {
        m.extend(props);
    }
    if let Some(b) = doc.world_bounds(id)
        && !n.is_page()
    {
        m.insert("page_bounds".into(), json!({ "x": b.x0, "y": b.y0, "width": b.width(), "height": b.height() }));
    }
    m.insert("children".into(), json!(n.children.iter().map(|c| c.0).collect::<Vec<_>>()));
    Value::Object(m)
}

/// Turns the friendlier JSON the reference allows into ops: `svg_path` for vectors, plain
/// numbers for `corner` and `padding`. Each op is read on its own, so errors say which.
pub fn parse_ops(doc: &Document, raw: Vec<Value>) -> Result<Vec<Op>, String> {
    raw.into_iter()
        .enumerate()
        .map(|(i, mut v)| {
            let kind = v.get("op").and_then(Value::as_str).unwrap_or("?").to_owned();
            let fail = |m: String| format!("op {i} ({kind}): {m}. Nothing was applied.");
            match kind.as_str() {
                "create" => {
                    if let Some(node) = v.get_mut("node") {
                        friendly(node, None).map_err(fail)?;
                    }
                }
                "set" => {
                    let size = v.get("id").and_then(Value::as_u64).and_then(|i| doc.node(NodeId(i))).map(|n| (n.props.width, n.props.height));
                    if let Some(patch) = v.get_mut("patch") {
                        friendly(patch, size).map_err(fail)?;
                    }
                }
                _ => {}
            }
            serde_json::from_value::<Op>(v).map_err(|e| fail(e.to_string()))
        })
        .collect()
}

fn friendly(obj: &mut Value, existing_size: Option<(f64, f64)>) -> Result<(), String> {
    let Some(m) = obj.as_object_mut() else { return Ok(()) };
    if let Some(r) = m.get("corner").and_then(Value::as_f64) {
        m.insert("corner".into(), json!({ "top_left": r, "top_right": r, "bottom_right": r, "bottom_left": r }));
    }
    if let Some(layout) = m.get_mut("layout").and_then(Value::as_object_mut)
        && let Some(p) = layout.get("padding").and_then(Value::as_f64)
    {
        layout.insert("padding".into(), json!({ "top": p, "right": p, "bottom": p, "left": p }));
    }
    if let Some(d) = m.remove("svg_path") {
        let d = d.as_str().ok_or("svg_path must be a string")?.to_owned();
        let w = m.get("width").and_then(Value::as_f64).or(existing_size.map(|s| s.0));
        let h = m.get("height").and_then(Value::as_f64).or(existing_size.map(|s| s.1));
        let (bw, bh) = match m.remove("svg_viewbox") {
            Some(vb) => {
                let [w, h]: [f64; 2] = serde_json::from_value(vb).map_err(|_| "svg_viewbox must be [width, height]".to_string())?;
                (w, h)
            }
            None => match (w, h) {
                (Some(w), Some(h)) => (w, h),
                // No size given: the path's own box.
                _ => {
                    let b = kurbo::BezPath::from_svg(&d).map_err(|e| format!("bad svg_path: {e:?}"))?.bounding_box();
                    (b.x1.max(1.0), b.y1.max(1.0))
                }
            },
        };
        let rule = m.remove("fill_rule");
        let mut path = jd_vector::VectorPath::from_svg(&d, bw, bh)?;
        if rule.as_ref().and_then(Value::as_str) == Some("even_odd") {
            path.fill_rule = jd_vector::FillRule::EvenOdd;
        }
        m.entry("width").or_insert(json!(bw));
        m.entry("height").or_insert(json!(bh));
        m.insert("path".into(), serde_json::to_value(&path).expect("json"));
    }
    Ok(())
}

fn applied(done: &Applied) -> String {
    let ids = |v: &[NodeId]| v.iter().map(|i| i.0.to_string()).collect::<Vec<_>>().join(", ");
    let mut s = String::from("Applied.");
    if !done.keys.is_empty() {
        let keys: Vec<String> = done.keys.iter().map(|(k, v)| format!("{k}={}", v.0)).collect();
        s.push_str(&format!("\nKeys: {}", keys.join(", ")));
    }
    if !done.created.is_empty() {
        s.push_str(&format!("\nCreated: {}", ids(&done.created)));
    }
    if !done.changed.is_empty() {
        let shown: Vec<NodeId> = done.changed.iter().copied().take(50).collect();
        s.push_str(&format!("\nChanged: {}{}", ids(&shown), if done.changed.len() > 50 { ", ..." } else { "" }));
    }
    if !done.removed.is_empty() {
        s.push_str(&format!("\nRemoved: {}", ids(&done.removed)));
    }
    s
}

fn arrange(target: &mut dyn Target, args: &Value) -> ToolResult {
    use jd_vector::arrange::{Align, Order};
    use jd_vector::boolean::BoolOp;
    let doc = target.doc();
    let ids = node_ids(doc, args)?;
    let action: String = args::get(args, "action")?;
    let to = args::opt::<String>(args, "to")?;
    let (label, ops) = match action.as_str() {
        "group" => ("Group", jd_vector::arrange::group(doc, &ids, false)?),
        "frame" => ("Frame selection", jd_vector::arrange::group(doc, &ids, true)?),
        "ungroup" => ("Ungroup", jd_vector::arrange::ungroup(doc, &ids)?.0),
        "align" => {
            let a = match to.as_deref() {
                Some("left") => Align::Left,
                Some("center") => Align::Center,
                Some("right") => Align::Right,
                Some("top") => Align::Top,
                Some("middle") => Align::Middle,
                Some("bottom") => Align::Bottom,
                _ => return Err("align needs `to`: left, center, right, top, middle or bottom".into()),
            };
            ("Align", jd_vector::arrange::align(doc, &ids, a))
        }
        "distribute" => ("Distribute", jd_vector::arrange::distribute(doc, &ids, args::opt(args, "horizontal")?.unwrap_or(true))),
        "order" => {
            let o = match to.as_deref() {
                Some("front") => Order::Front,
                Some("forward") => Order::Forward,
                Some("backward") => Order::Backward,
                Some("back") => Order::Back,
                _ => return Err("order needs `to`: front, forward, backward or back".into()),
            };
            ("Reorder", jd_vector::arrange::reorder(doc, &ids, o))
        }
        "union" => ("Union", jd_vector::boolean::boolean(doc, &ids, BoolOp::Union)?),
        "subtract" => ("Subtract", jd_vector::boolean::boolean(doc, &ids, BoolOp::Subtract)?),
        "intersect" => ("Intersect", jd_vector::boolean::boolean(doc, &ids, BoolOp::Intersect)?),
        "exclude" => ("Exclude", jd_vector::boolean::boolean(doc, &ids, BoolOp::Exclude)?),
        "flatten" => ("Flatten", jd_vector::boolean::flatten(doc, &ids)?),
        a => return Err(format!("unknown action {a:?}")),
    };
    if ops.is_empty() {
        return text("Nothing to change.");
    }
    let done = target.apply(label, &ops)?;
    text(applied(&done))
}

/// A node as PNG, at `scale` or fitted into `max_size` (default 1024) pixels. Pages render
/// everything on them, on their background.
pub fn render(doc: &Document, id: NodeId, scale: Option<f64>, max_size: Option<u32>, background: Option<Rgba>) -> Result<Vec<u8>, String> {
    let n = doc.node(id).ok_or_else(|| format!("there is no node {id}"))?;
    let bounds = if let NodeKind::Page { .. } = n.kind {
        doc.children(id).iter().filter_map(|&c| doc.world_bounds(c)).reduce(|a, b| a.union(b)).ok_or("the page is empty")?
    } else {
        doc.world_bounds(id).ok_or("nothing to render")?
    };
    let longest = bounds.width().max(bounds.height()).max(1.0);
    let max = max_size.unwrap_or(1024).clamp(16, 4096) as f64;
    let scale = scale.unwrap_or(max / longest).min(max / longest);
    match n.kind {
        NodeKind::Page { background: bg } => {
            let pm = jd_vector_render::render_region(doc, id, bounds.inflate(40.0, 40.0), RenderOptions { scale, background: Some(background.unwrap_or(bg)) })
                .ok_or("nothing to render")?;
            pm.encode_png().map_err(|e| e.to_string())
        }
        _ => jd_vector_render::render_png(doc, id, RenderOptions { scale, background }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(t: &mut FileTarget, name: &str, args: Value) -> String {
        match call(t, name, args) {
            Ok(c) => c
                .into_iter()
                .map(|c| match c {
                    Content::Text(s) => s,
                    Content::Png(b) => format!("<png {} bytes>", b.len()),
                })
                .collect(),
            Err(e) => format!("ERROR: {e}"),
        }
    }

    /// Every JSON example in the reference is a valid batch, and applies to a new document.
    #[test]
    fn reference_examples_work() {
        let blocks: Vec<&str> = REFERENCE.split("```json").skip(1).map(|b| b.split("```").next().unwrap()).collect();
        assert!(!blocks.is_empty());
        for b in blocks {
            let mut t = FileTarget::default();
            let raw: Vec<Value> = serde_json::from_str(b).unwrap_or_else(|e| panic!("{e}: {b}"));
            let out = run(&mut t, "apply_ops", json!({ "ops": raw }));
            assert!(out.starts_with("Applied."), "{out}");
        }
    }

    #[test]
    fn build_query_render_save_and_undo() {
        let dir = std::env::temp_dir().join(format!("blobr-mcp-test-{}", std::process::id()));
        let file = dir.join("card.jdb");
        let mut t = FileTarget::default();
        assert!(run(&mut t, "reference", json!({})).contains("apply_ops"));
        let out = run(
            &mut t,
            "apply_ops",
            json!({ "label": "Card", "ops": [
                { "op": "create", "parent": 1, "key": "card", "node": { "type": "frame", "name": "Card", "width": 200, "height": 100, "corner": 12,
                  "layout": { "direction": "horizontal", "gap": 8, "padding": 16 }, "sizing_h": "hug" } },
                { "op": "create", "parent": "card", "key": "icon", "node": { "type": "vector", "name": "Play", "svg_path": "M0 0 L24 12 L0 24 Z",
                  "fills": [{ "type": "solid", "color": "#4C8DFF" }] } },
                { "op": "create", "parent": "card", "node": { "type": "text", "name": "Label", "text": { "content": "Play" }, "sizing_h": "hug", "sizing_v": "hug" } },
            ]}),
        );
        assert!(out.contains("Keys: card="), "{out}");
        let card = t.doc().find_by_name("Card").unwrap();
        let n = t.doc().node(card).unwrap();
        assert_eq!((n.props.corner.top_left, n.props.layout.as_ref().unwrap().padding.left), (12.0, 16.0));
        let play = t.doc().find_by_name("Play").unwrap();
        assert_eq!((t.doc().node(play).unwrap().props.width, t.doc().node(play).unwrap().props.height), (24.0, 24.0), "size from the path");

        let tree = run(&mut t, "get_tree", json!({}));
        assert!(tree.contains("frame \"Card\"") && tree.contains("[auto horizontal gap 8]") && tree.contains("text=\"Play\""), "{tree}");
        assert!(run(&mut t, "query", json!({ "selector": "frame[name=Card] > vector" })).contains("\"Play\""));
        let node = run(&mut t, "get_node", json!({ "ids": [play.0] }));
        assert!(node.contains("\"svg_path\"") && node.contains("page_bounds"), "{node}");
        assert!(run(&mut t, "render", json!({ "id": card.0 })).starts_with("<png"));
        assert!(run(&mut t, "render", json!({ "id": 1 })).starts_with("<png"), "a page renders");

        let bad = run(&mut t, "apply_ops", json!({ "ops": [{ "op": "set", "id": card.0, "patch": { "x": 5 } }, { "op": "delete", "id": 9999 }] }));
        assert!(bad.starts_with("ERROR: op 1 (delete)") && bad.contains("Nothing was applied"), "{bad}");
        assert_eq!(t.doc().node(card).unwrap().props.x, 0.0);
        let typo = run(&mut t, "apply_ops", json!({ "ops": [{ "op": "crate", "parent": 1 }] }));
        assert!(typo.starts_with("ERROR: op 0 (crate)"), "{typo}");

        assert!(run(&mut t, "save_document", json!({})).contains("give a path"));
        assert!(run(&mut t, "save_document", json!({ "path": file })).starts_with("Saved"));
        assert!(run(&mut t, "document_info", json!({})).contains("\"unsaved_changes\": false"));
        assert_eq!(run(&mut t, "undo", json!({})), "Undid \"Card\"");
        assert!(t.doc().find_by_name("Card").is_none());
        let info = run(&mut t, "open_document", json!({ "path": file }));
        assert!(info.contains("\"children\": 1"), "{info}");
        assert!(run(&mut t, "arrange", json!({ "action": "align", "ids": [card.0], "to": "nowhere" })).starts_with("ERROR"));
        assert!(run(&mut t, "focus_node", json!({ "id": card.0 })).contains("running Blobr app"));
        assert!(run(&mut t, "export", json!({ "ids": [card.0], "format": "pdf", "path": dir.join("card.pdf") })).starts_with("Wrote"));
        assert!(run(&mut t, "inspect_code", json!({ "id": card.0, "format": "css" })).contains("gap"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_tool_has_an_object_schema() {
        for t in tools() {
            assert_eq!(t.input_schema["type"], "object", "{}", t.name);
        }
    }
}
