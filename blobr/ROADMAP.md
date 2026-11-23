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
- [x] Step 2: components, variants and tokens in the core
  - [x] Components with text, boolean and instance-swap properties (`refs` on their layers)
  - [x] Instances: full copies rebuilt after every batch, stable layer ids, overrides on any
        layer (kept when the component changes), nested instances and swaps, reset, detach,
        duplicate; adding/removing/moving layers inside an instance is refused, as in Figma
  - [x] Variants: component sets named `Size=Large, State=Default`, switched through
        property values, overrides carried across by layer name
  - [x] A component can't end up containing itself (checked on create, move and swap)
  - [x] Variables in collections with modes (colour, number, text, bool, aliases), bound to
        properties by path (`fills/0`, `gap`, `padding/top`, ...), modes chosen per frame;
        typing a value unbinds it
  - [x] Paint, text and effect styles
  - [ ] Acceptance: an example builds the Blobr main-window mockup, components included, as
        a `.jdb` and renders it to PNG
- [ ] Step 3: the app, viewing (`cargo run -p blobr -- file.jdb`)
  - [x] Window, menus, toolbar and panels laid out like the mockup, built from `jd-ui` (theme
        tokens, icons, panel tabs) so it matches Photosplash and Cinematica
  - [x] Open/save/save as `.jdb`, PNG/SVG export of the selection
  - [x] Unsaved-changes prompt (Save / Don't Save / Cancel) before quit, new and open
  - [ ] Document tabs, recent files
  - [x] Canvas: tiny-skia render of the visible region at screen resolution, redrawn only when
        the document, camera or window changes; zoom (Ctrl+scroll, Shift+0/1/2), pan (scroll,
        Space, middle drag, Hand), frame labels, hover and selection outlines
  - [ ] Faster redraws: a full-viewport render is ~30 ms at 2x in debug (drags render once
        per frame); cache shaped text and the untouched layers, GPU later if needed
  - [x] Pages list (add, switch), layer tree (expand, select, hide, lock); clicks pick a
        frame's children, Ctrl picks the deepest layer, double-click goes one level deeper
  - [x] Design panel showing (and editing) the selection: name, position, size, rotation,
        radius, sizing, auto layout, opacity, text, fills, strokes, instance properties and
        variants; errors from refused ops go to the status bar
  - [x] Rename in the layer tree (double-click or Ctrl+R), drag layers to restack them or
        drop them into frames and groups
  - [x] Multi-selection editing (shared values, "Mixed" where they differ)
- [ ] Step 4: editing tools (started: move/resize by drag as one undo step, Frame, Rectangle,
      Ellipse and Text by drag, marquee, nudge, duplicate, delete; snapping to siblings and
      the parent with red guides, Shift keeps to one axis, Ctrl turns snapping off; Alt shows
      distances to the hovered layer or the parent; group, frame selection, ungroup and
      stacking order, align and distribute, as `jd_vector::arrange` op builders; copy, cut and
      paste through the system clipboard (`jd_vector::clip`, `Op::Paste`), images and text
      pasted from other apps, Copy as PNG; typing text on the canvas; rotating from just
      outside a corner (Shift: 15 degree steps); the Pen tool (click for corners, drag for
      curves, click the first point to close) and editing a vector's points and handles
      (double-click or Enter), the box refitting afterwards; booleans (union, subtract,
      intersect, exclude via `linesweeper`) and flatten, as `jd_vector::boolean`)
  - [ ] Live boolean groups (editable afterwards, as in Figma), outline stroke, resize
        handles on rotated layers, adding points to an existing path
  - Move, resize and rotate with handles; Frame (with device presets), Rectangle, Ellipse,
    Line, Pen (bezier editing) and Text tools; Hand and Zoom
  - Snapping, smart guides and spacing measurements (as in the mockup)
  - Multi-select, align and distribute, group/ungroup, frame selection, wrap in auto layout
  - Editable Design panel (every value box typed into, like Photosplash's)
  - Copy/paste within Blobr and to/from other apps as SVG/PNG; Comment tool with local pins
  - Boolean operations (union, subtract, intersect, exclude), flatten, outline stroke
- [ ] Step 5: components and assets in the app
  - [x] Create component, instances, detach, reset overrides, variant switching from the
        Design panel; combine as variants (plain names become `Property 1=...`), swap an
        instance's component
  - [x] Component properties (text, boolean) with defaults; layers inside a component link
        to them (text, visibility, swap); instances set them
  - [x] Assets panel: the document's components with thumbnails and search; click to place
  - [x] Local variables dialog: collections, a column per mode, add/rename/delete variables
        (colour, number, text, boolean) and modes
  - [x] Fills bound to colour variables, frames pick a mode per collection; paint and text
        styles created from a layer, applied, renamed and deleted
  - [x] Number variables bound from the panel (size, radius, opacity, gap, padding, font
        size, stroke); effects (drop/inner shadow, layer/background blur) and effect styles;
        instance-swap properties
  - [x] Libraries: add another .jdb, place its components (copied in once, onto a
        "Library components" page). Libraries are remembered for the session only
  - [ ] Libraries that update when their file changes; remembered per document
- [x] Step 6: prototype and inspect
  - [x] Prototype tab: interactions (on click, while hovering, after a delay) that navigate,
        open an overlay, go back or open a link; instant, dissolve and slide transitions;
        connection arrows on the canvas; links between pasted frames point at the copies
  - [x] Presenting full-window (Ctrl+Alt+Enter or the play button): click hotspots, Escape
        leaves, arrows step through frames. In the main window rather than a second one
  - [x] Inspect tab: measurements, colours (with their variables), type, and code: CSS,
        HTML for a subtree, an iced snippet, and the variables as Rust constants in the form
        `jd-ui`'s `theme.rs` uses (`jd_vector::inspect`, for the MCP server too); hovering
        shows distances while the tab is open
  - [ ] Smart animate, scroll behaviour, prototype flows saved per page, a separate window
- [x] Step 7: interop and moving off Figma
  - [x] SVG import (File > Import SVG, or paste SVG markup): shapes, groups, gradients,
        strokes, images and text as outlines, through usvg (`jd_vector_render::import_svg`)
  - [x] SVG export; PDF export with a page per frame (vector, via svg2pdf)
  - [x] Figma import: `jd-figma` turns the REST API's file JSON into a document (frames,
        auto layout, shapes, vectors from Figma's geometry, text, fills incl. images,
        strokes, effects, components); `cargo run -p jd-figma --bin figma2jdb -- <file URL>
        out.jdb` with `FIGMA_TOKEN` set fetches it (2 API calls + image downloads). Instances
        come in as frames, component sets as frames of variants, no variables/styles
  - [x] The suite's mockups are in `blobr/mockups/suite.jdb` (with the Figma JSON they came
        from, for offline re-imports); design work no longer depends on Figma
  - [x] `jd_vector_render::place`: a `.jdb` file's frames and one as RGBA, for Photosplash
  - [x] Photosplash File > Place uses it (Photosplash's side); keep `place::open`, `frames`
        and `render_rgba` stable, or tell the Photosplash session first

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
