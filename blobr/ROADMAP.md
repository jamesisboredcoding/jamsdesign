# Blobr roadmap

Blobr is the jamsdesign suite's interface and vector design app, in the spirit of Figma: pages,
frames, shapes, text, auto layout, components and variants on an infinite canvas. The Figma
mockup is "Blobr - Main Window" (node 6:393) in Figma file `y2CmiTTVmgx3mksdEJlvpD`; the logo is
`img/blobr.svg`.

**MCP is the final step, but it's designed in from the start.** Blobr exists partly because
designing in Figma through its MCP server hits the Starter plan's call cap after a handful of
edits. Blobr's MCP server will run locally with no cap. It comes last: the core and the app come
first. Everything below is built so MCP can be added on top without reworking it (one edit
path, headless core, atomic batches, cheap renders).

## Principles

- **One edit path.** Every change is a serializable `Op` applied to the document in a
  transaction. The GUI, undo/redo, file loading and MCP all go through it. There is no
  MCP-only backdoor, so whatever an agent can do a user can undo, and the reverse.
- **Headless first.** The document model, layout and rendering live in crates with no iced
  dependency (the suite's layering rule), so MCP can open, edit and render `.jdb` files
  without a window.
- **Atomic batches.** A batch of ops (a GUI gesture, later an MCP write) is one transaction:
  it applies completely or not at all, and it is one undo step. No half-built frames to clean up after an error.
- **Cheap renders.** Rendering any node to PNG without a window is fast, for export, thumbnails,
  tests, and later for agents checking their own work.
- **Stable IDs.** Applying ops returns the created and changed node IDs, and IDs stay the same
  across saves.
- **Shared plumbing.** The MCP framework will live in a shared crate so Photosplash (see the
  Photosplash MCP idea) and Cinematica can expose their own tools later without rewriting it.

## Planned layout

```
crates/jd-vector/    scene graph, geometry, auto layout, ops, undo, .jdb (no iced)
crates/jd-mcp/       shared MCP server plumbing: transport, tool registry, schemas (no iced)
blobr/apps/blobr/    the iced app
blobr/apps/blobr-mcp the MCP server binary (stdio), headless or bridged to a running Blobr
```

`blobr-mcp` is a separate binary rather than a `--mcp` flag, because suite apps treat every
CLI argument as a file to open. Names are provisional until Step 0.

## Status

- [x] Step 0: headless core (`crates/jd-vector`)
  - [x] Document > pages > node tree. Kinds: Frame, Group (fits its children), Section,
        Rectangle, Ellipse (arc), Line, Polygon, Star, Vector (bezier path, SVG path import),
        Text. Component and Instance come in Step 2
  - [x] Properties: position, size, rotation, constraints, opacity, blend mode, per-corner
        radii, clip, visibility, lock; fills and strokes (solid, linear/radial gradient,
        image), stroke weight/align/dash/cap/join; drop and inner shadow, layer and
        background blur
  - [x] Auto layout: horizontal/vertical, gap, padding, align (incl. space between),
        hug/fill/fixed, wrap with row gap, absolute children, min/max sizes
  - [x] `Op` batches: atomic, validated, keys to link ops within a batch, report
        created/changed/removed ids; `Resize` follows constraints; undo history and an
        `Editor` that makes each batch one undo step
  - [x] `.jdb` format, version 1: zip of `mimetype`, nested readable `document.json` and
        `images/` (unused images dropped). Decided: JSON in a zip, like `.jdp`
  - [x] Decided: `kurbo` 0.11 for paths and transforms (already in the lockfile)
  - [x] Text: one style per node, a `TextMeasure` trait for layout; the real measurer
        (`CosmicMeasure`, system fonts) lives in `jd-vector-render`. Mixed styles within one
        text node come later
  - [x] Shape outlines for every kind (`geometry::outline`), hit testing (topmost node under
        a point, respecting clipping and rotation, with slop for lines) and marquee tests
  - Boolean operations moved to Step 4 (editing tools)
- [x] Step 1: headless rendering and export (`crates/jd-vector-render`)
  - [x] CPU renderer with tiny-skia: fills (solid, linear/radial gradients, images in
        fill/fit/stretch/tile), strokes inside/centre/outside with dashes, caps and joins,
        clipping, opacity, all blend modes, drop and inner shadows (with spread), layer and
        background blur, any scale
  - [x] Text drawn from unhinted glyph outlines (cosmic-text): sharp at any zoom or angle,
        takes any paint, exports to SVG as paths
  - [x] PNG and SVG export of a node or page; `examples/render.rs` renders any `.jdb`
  - [x] Tests: pixel checks per feature, plus SVG export rendered by resvg compared with our
        raster renderer
  - [x] Decided: tiny-skia (already in the lockfile) for headless rendering. On-screen
        rendering is decided with the app in Step 3 (tiny-skia into a texture first, GPU
        later if needed)
  - [x] Export settings saved on nodes (PNG/SVG, scale, suffix) and `export_files`
- [ ] Step 2: components, variants and tokens in the core
  - Components, instances, overrides, component properties (text, boolean, instance swap)
    and variant sets
  - Variables/tokens (colour, number, string) with modes (for example dark and light),
    bindable to properties; colour and text styles
  - Acceptance: an example builds the Blobr main-window mockup, components included, as a
    `.jdb` and renders it to PNG
- [ ] Step 3: the app, viewing
  - Window, menus, toolbar and panels laid out like the mockup, built from `jd-ui` (theme
    tokens, icons, panel tabs) so it matches Photosplash and Cinematica
  - Open/save `.jdb`, document tabs, recent files, unsaved-changes prompts
  - GPU canvas: zoom, pan, pixel-snapped rendering of frames and their labels
  - Pages list, layer tree (expand, select, rename, hide, lock), click to select, Design panel
    showing the selection's properties
- [ ] Step 4: editing tools
  - Move, resize and rotate with handles; Frame (with device presets), Rectangle, Ellipse,
    Line, Pen (bezier editing) and Text tools; Hand and Zoom
  - Snapping, smart guides and spacing measurements (as in the mockup)
  - Multi-select, align and distribute, group/ungroup, frame selection, wrap in auto layout
  - Editable Design panel (every value box typed into, like Photosplash's)
  - Copy/paste within Blobr and to/from other apps as SVG/PNG; Comment tool with local pins
  - Boolean operations (union, subtract, intersect, exclude), flatten, outline stroke
- [ ] Step 5: components and assets in the app
  - Create component, instances, detach, reset overrides, variant switching from the
    Design panel (as in the mockup)
  - Assets panel; styles and variables panels with modes
- [ ] Step 6: prototype and inspect
  - Prototype tab: connections between frames (on click, navigate, overlay, back), simple
    transitions, a presentation window
  - Inspect tab: measurements and code for the selection (CSS, and Rust constants in the
    form `jd-ui`'s `theme.rs` uses)
- [ ] Step 7: interop and moving off Figma
  - Full SVG import and export, PDF export
  - Spike: one-time import of Figma files through the Figma REST API's JSON (needs a
    personal token, and it's a handful of calls rather than one per edit)
  - Move the suite's mockups (Photosplash, Cinematica, Blobr, the logos) into `.jdb` files in
    the repo, so design work no longer depends on Figma
  - Place a `.jdb` frame into Photosplash as a layer

- [ ] Step 8: MCP (`jd-mcp` + `blobr-mcp`), the final touch
  - Transport: stdio, so it can be added to Claude Code with a single command
  - Tools: `new_document`, `open_document`, `save_document`, `list_pages`, `get_tree`
    (depth and field limits), `query` (selector syntax: type, name, attributes,
    descendants), `get_node` (full properties for given IDs), `apply_ops` (atomic batch,
    returns IDs), `import_svg` (SVG string to editable vectors, for icons), `render`
    (node to PNG, scale or max size), `export`, `list_fonts`
  - Clear errors: which op failed, why, and a note that nothing was applied
  - Spike: structured JSON ops only, or also an embedded script surface like Figma's
    `use_figma`. Start with JSON ops; add scripting only if batches prove too clumsy
  - Spike: Rust MCP SDK crate vs a small hand-written JSON-RPC layer; check what's current
  - Acceptance: Claude rebuilds the Photosplash main-window mockup in a `.jdb` file over MCP,
    checking itself with `render`, with no call limit
  - Live bridge:
    - A running Blobr listens on a local socket (under `$XDG_RUNTIME_DIR`); `blobr-mcp`
      connects to it when it's running and falls back to headless file mode when it isn't
    - Agent edits appear live, land in History as named undo steps, and can be undone
    - Tools that need the app: `get_selection`, `select`, `focus_node` (scroll the canvas to
      a node), render exactly what is on screen
    - A small status bar indicator while an agent is connected

## Later

- Plugins or scripting inside the app, if Step 8's scripting spike says it's worth it
- Photosplash and Cinematica MCP servers on `jd-mcp` (each app's actions are already
  messages, so tools can map onto them)
- Version history per file, and branching
- Whiteboard pages (sticky notes, connectors)
