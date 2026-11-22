# Blobr roadmap

Blobr is the jamsdesign suite's interface and vector design app, in the spirit of Figma: pages,
frames, shapes, text, auto layout, components and variants on an infinite canvas. The Figma
mockup is "Blobr - Main Window" (node 6:393) in Figma file `y2CmiTTVmgx3mksdEJlvpD`; the logo is
`img/blobr.svg`.

**Why MCP comes first.** Blobr exists partly because designing in Figma through its MCP server
hits the Starter plan's call cap after a handful of edits. Blobr's MCP server runs locally, so
there is no cap, and an agent should be able to design in Blobr before the GUI can edit
anything. The order below follows from that: headless core, then headless MCP, then the app.

## Principles

- **One edit path.** Every change is a serializable `Op` applied to the document in a
  transaction. The GUI, undo/redo, file loading and MCP all go through it. There is no
  MCP-only backdoor, so whatever an agent can do a user can undo, and the reverse.
- **Headless first.** The document model, layout and rendering live in crates with no iced
  dependency (the suite's layering rule), so MCP can open, edit and render `.jdb` files
  without a window.
- **Atomic batches.** An MCP write is one transaction: it applies completely or not at all,
  and it is one undo step. No half-built frames to clean up after an error.
- **Cheap reads, cheap renders.** Reads take depth and field limits, so a large file doesn't
  flood the agent's context. Rendering a node to PNG is fast and unlimited, because screenshots
  are how an agent checks its own work.
- **Every write returns IDs.** Created and changed node IDs come back from each call. IDs are
  stable across saves.
- **Shared plumbing.** The MCP framework lives in a shared crate so Gallerion (see the
  Gallerion MCP idea) and Cinematica can expose their own tools later without rewriting it.

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

- [ ] Step 0: headless core (`jd-vector`)
  - Document > pages > node tree. Node kinds: Frame, Group, Section, Rectangle, Ellipse,
    Line, Polygon/Star, Vector (bezier path), Text, Component, Instance
  - Properties: position, size, rotation, constraints, opacity, blend mode, corner radii
    (per corner), clip content, visibility, lock; fills (solid, linear/radial gradient,
    image) and strokes (weight, align, dash); effects (drop shadow, inner shadow, layer blur)
  - Auto layout: horizontal/vertical, gap, padding, align, hug/fill/fixed sizing, wrap,
    absolutely positioned children, min/max sizes
  - Text with `cosmic-text` (already used by `jd-raster`): font family/style, size, line
    height, letter spacing, alignment, auto width/height/fixed
  - `Op` enum + transactions + undo history; stable node IDs
  - `.jdb` file format, versioned from day one
  - Spike: `.jdb` encoding (plain JSON vs a zip of JSON + images); check current crates
  - Spike: geometry and path library for beziers, bounds, hit testing and booleans; check
    current crates against the vendored/offline build setup (`git-fetch-with-cli`)
- [ ] Step 1: headless rendering and export
  - CPU renderer: node or page to PNG at any scale, used by MCP renders and export
  - Export PNG and SVG per node, with export settings saved on the node
  - Golden-image tests for shapes, strokes, gradients, effects, text and auto layout
  - Spike: rendering backend for both headless and on-screen (one renderer for both if
    possible, otherwise a parity test like `jd-gpu`'s `blend_parity`). `jd-gpu` composites
    raster tiles, so vectors need their own path. Check current crates
- [ ] Step 2: MCP server v1, headless (`jd-mcp` + `blobr-mcp`)
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
  - Acceptance: Claude rebuilds the Gallerion main-window mockup in a `.jdb` file over MCP,
    checking itself with `render`, with no call limit
- [ ] Step 3: components, variants and tokens in the core and MCP
  - Components, instances, overrides, component properties (text, boolean, instance swap)
    and variant sets
  - Variables/tokens (colour, number, string) with modes (for example dark and light),
    bindable to properties; colour and text styles
  - MCP tools: `create_component`, `create_instance`, `set_properties`, variable and style
    tools
  - Acceptance: Claude rebuilds the Blobr mockup itself, including its components
- [ ] Step 4: the app, viewing
  - Window, menus, toolbar and panels laid out like the mockup, built from `jd-ui` (theme
    tokens, icons, panel tabs) so it matches Gallerion and Cinematica
  - Open/save `.jdb`, document tabs, recent files, unsaved-changes prompts
  - GPU canvas: zoom, pan, pixel-snapped rendering of frames and their labels
  - Pages list, layer tree (expand, select, rename, hide, lock), click to select, Design panel
    showing the selection's properties
- [ ] Step 5: live MCP bridge
  - A running Blobr listens on a local socket (under `$XDG_RUNTIME_DIR`); `blobr-mcp`
    connects to it when it's running and falls back to headless file mode when it isn't
  - Agent edits appear live, land in History as named undo steps, and can be undone
  - Tools that need the app: `get_selection`, `select`, `focus_node` (scroll the canvas to
    a node), render exactly what is on screen
  - A small status bar indicator while an agent is connected
- [ ] Step 6: editing tools
  - Move, resize and rotate with handles; Frame (with device presets), Rectangle, Ellipse,
    Line, Pen (bezier editing) and Text tools; Hand and Zoom
  - Snapping, smart guides and spacing measurements (as in the mockup)
  - Multi-select, align and distribute, group/ungroup, frame selection, wrap in auto layout
  - Editable Design panel (every value box typed into, like Gallerion's)
  - Copy/paste within Blobr and to/from other apps as SVG/PNG; Comment tool with local pins
  - Boolean operations (union, subtract, intersect, exclude), flatten, outline stroke
- [ ] Step 7: components and assets in the app
  - Create component, instances, detach, reset overrides, variant switching from the
    Design panel (as in the mockup)
  - Assets panel; styles and variables panels with modes
- [ ] Step 8: prototype and inspect
  - Prototype tab: connections between frames (on click, navigate, overlay, back), simple
    transitions, a presentation window
  - Inspect tab: measurements and code for the selection (CSS, and Rust constants in the
    form `jd-ui`'s `theme.rs` uses)
- [ ] Step 9: interop and moving off Figma
  - Full SVG import and export, PDF export
  - Spike: one-time import of Figma files through the Figma REST API's JSON (needs a
    personal token, and it's a handful of calls rather than one per edit)
  - Move the suite's mockups (Gallerion, Cinematica, Blobr, the logos) into `.jdb` files in
    the repo, so design work no longer depends on Figma
  - Place a `.jdb` frame into Gallerion as a layer

## Later

- Plugins or scripting inside the app, if Step 2's scripting spike says it's worth it
- Gallerion and Cinematica MCP servers on `jd-mcp` (each app's actions are already
  messages, so tools can map onto them)
- Version history per file, and branching
- Whiteboard pages (sticky notes, connectors)
