# Blobr

The jamsdesign suite's interface and vector design app, in the spirit of Figma: pages, frames,
shapes, text, auto layout, components and variants on an infinite canvas, with prototypes,
comments and developer handoff. Documents are `.jdb` files on your disk. AI agents can read and
edit them through Blobr's MCP server, live in the running app.

## Run

From the repository root:

```sh
cargo run -p blobr                          # the app (optionally: -- path/to/file.jdb)
cargo run -p blobr -- blobr/mockups/suite.jdb   # the suite's own mockups
cargo test -p jd-vector -p jd-vector-render -p blobr -p blobr-mcp
```

Build the example the roadmap's acceptance check uses (the Blobr main window, built from
components with about 280 ops, saved and rendered):

```sh
cargo run -p blobr-mcp --example blobr_window -- window.jdb window.png
```

## Layout

```
crates/jd-vector/          the document: scene graph, auto layout, ops, undo, .jdb (no iced)
crates/jd-vector-render/   draws documents without a window: PNG, SVG, PDF, SVG import, Place
crates/jd-figma/           one-time Figma import (figma2jdb)
crates/jd-mcp/             MCP server plumbing, shared with future suite servers
blobr/apps/blobr/          the iced app
blobr/apps/blobr-mcp/      the MCP server (stdio), headless or bridged to a running Blobr
blobr/mockups/             the suite's mockups as a .jdb
```

Every change, from the app, undo/redo or an agent, is a serializable `Op` applied in an atomic
batch. See [ROADMAP.md](ROADMAP.md) for how it was built and what each step covers.

## Features

- **Drawing:** Frame (with device sizes), Rectangle, Ellipse, Line, Pen and Text tools; move,
  resize (rotated layers too) and rotate on the canvas; snapping with red guides; Alt shows
  distances; editing a vector's points (click a segment to add one).
- **Layout:** auto layout (direction, gap, padding, alignment, wrap, hug/fill/fixed, min/max),
  constraints, groups, frame selection, align and distribute.
- **Shapes:** live boolean groups (union, subtract, intersect, exclude) that stay editable,
  Flatten, Outline Stroke.
- **Components:** components, instances with overrides, variants, text/boolean/instance-swap
  properties, the Assets panel, libraries from other `.jdb` files that are saved with the
  document and update when their file changes.
- **Tokens:** variables in collections with modes (colour, number, text, boolean), paint, text
  and effect styles.
- **Prototype:** click, hover and after-delay interactions; navigate, overlay, back and links;
  dissolve, slides and smart animate; scrolling frames; flows. Present in the window or in a
  window of its own.
- **Inspect:** measurements, colours with their variables, CSS, HTML, iced and Rust tokens.
- **Comments:** numbered pins on the canvas with replies, resolve and delete, kept in the file.
- **Files:** tabs, recent files, unsaved-changes prompts per tab; export PNG, SVG and PDF;
  import SVG; Copy as PNG; paste images and text from other apps.

## Shortcuts (Figma's)

| Keys | Does |
| --- | --- |
| V, F, R, O, L, P, T, C, H, Z | Move, Frame, Rectangle, Ellipse, Line, Pen, Text, Comment, Hand, Zoom |
| Space or middle drag, Ctrl+scroll | Pan, zoom |
| Shift+0, Shift+1, Shift+2 | 100%, zoom to fit, zoom to selection |
| Ctrl+G, Ctrl+Alt+G, Ctrl+Shift+G | Group, frame selection, ungroup |
| Shift+A | Auto layout |
| Ctrl+E, Ctrl+Shift+O | Flatten, Outline Stroke |
| Ctrl+Alt+K, Ctrl+Alt+B | Create component, detach instance |
| Ctrl+D, Ctrl+C, Ctrl+V, Ctrl+Shift+C | Duplicate, copy, paste, copy as PNG |
| Ctrl+], Ctrl+[ | Bring to front, send to back |
| Enter, Escape | Edit text or points, go up a level |
| Ctrl+N, Ctrl+O, Ctrl+S, Ctrl+W, Ctrl+Tab | New tab, open, save, close tab, next tab |
| Ctrl+Alt+Enter, Ctrl+Alt+Shift+Enter | Present, present in a new window |

## Agents (MCP)

`blobr-mcp` lets Claude Code and other MCP clients build and edit designs, with no call limit:

```sh
cargo build --release -p blobr-mcp
claude mcp add blobr -- "$PWD/target/release/blobr-mcp"
```

While Blobr is running the tools work on its active tab, and each change is an undo step there.
Otherwise they work on files of their own. Details: [apps/blobr-mcp/README.md](apps/blobr-mcp/README.md).

## Status

All roadmap steps (0 to 8) are done. Ideas for later: scripting or plugins, version history
and branching, whiteboard pages, and MCP servers for Photosplash and Cinematica.
