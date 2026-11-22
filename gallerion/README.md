# jamsdesign

A Linux-first design suite. First app: **Gallerion**, the photo editor.

Design reference: Figma file "jamsdesign Photo Editor" (colour tokens there mirror `crates/jd-ui/src/theme.rs`).

## Run

```sh
cargo run -p gallerion         # the photo editor (optionally: -- path/to/image.jpg ...)
cargo run -p tablet-spike      # draw with your pen: width = pressure, bars = pressure / tilt
cargo test --workspace
```

## Layout

The suite is one Cargo workspace at the repository root (`Cargo.toml`, `Cargo.lock`,
`target/`, `vendor/`). Shared crates live in the root `crates/`; this folder holds
Gallerion's apps. Paths below are from the repository root.

```
crates/
  jd-core     colour (linear Rgba, sRGB, HSV), no UI deps
  jd-raster   document model: layers, 256px copy-on-write tiles, undo history, the blend-mode
              maths, a CPU compositor (export, thumbnails, reference for the GPU) and the
              brush engine (paint.rs), no UI deps
  jd-io       open images (EXIF orientation applied), .jdp documents, export
  jd-gpu      wgpu renderer: all 24 blend modes, perceptual or linear blending, mips,
              checkerboard. tests/blend_parity.rs checks it against the CPU compositor
  jd-ui       iced theme tokens, icons (icons/*.svg), shared widgets
  jd-ai       on-device background removal (ONNX Runtime)
gallerion/apps/
  gallerion/    Gallerion, the photo editor (iced 0.14)
  tablet-spike/ standalone pen test window (octotablet), handy for checking a tablet
```

Rule: `jd-core`, `jd-raster`, `jd-io` and `jd-gpu` never depend on iced. The iced glue for
the GPU canvas lives in `gallerion/apps/gallerion/src/view/canvas.rs`. Shared crates are used
by Cinematica too: a change to one must keep both apps building and `cargo test --workspace`
passing.

## Shortcuts (Photoshop-compatible)

Tools: V M L W C I J B S E G O P T A U H Z · X swap colours · D reset colours ·
`[` `]` brush size · Ctrl+= / Ctrl+- zoom · Ctrl+0 fit · Ctrl+1 100% · Ctrl+Shift+N new layer ·
Delete/Backspace removes layer · Ctrl+J duplicate layer · Ctrl+] / Ctrl+[ move layer up/down ·
double-click a layer name to rename · the padlock in the Layers panel locks/unlocks
(opened images start with a locked Background, like Photoshop) · Ctrl+Z undo · Ctrl+Shift+Z or Ctrl+Y redo · Ctrl+S save ·
Ctrl+Shift+S save as · Ctrl+Alt+Shift+W export · Ctrl+Q quit · Ctrl+O open · Ctrl+N new ·
Ctrl+A select all · Ctrl+D deselect · Ctrl+Shift+I inverse · Shift + a tool key cycles that
slot's tools (Shift+M rect/ellipse marquee, Shift+L lasso/polygonal lasso) · right-click a
toolbar slot for its tools · right-click the canvas for a context menu (Free Transform etc.) ·
Shift/Alt/Shift+Alt while starting a selection: add/subtract/intersect · Delete clears the
selection (no selection: deletes the layer) · Alt+Backspace / Ctrl+Backspace fill with
foreground / background · Ctrl+C / Ctrl+X / Ctrl+V · Ctrl+J / Ctrl+Shift+J layer via copy / cut ·
Ctrl+T free transform (Enter commits, Esc cancels) · arrows nudge with the Move tool ·
Ctrl+L levels · Ctrl+M curves · Ctrl+U hue/saturation · Ctrl+I invert · Shift+Ctrl+U desaturate ·
Shift+Ctrl+L auto tone · Alt+Shift+Ctrl+L auto contrast · Alt+Ctrl+I image size · Alt+Ctrl+C
canvas size · Ctrl+E merge down · Shift+Ctrl+E merge visible · Shift+Ctrl+C copy merged ·
Shift+Ctrl+D reselect · Shift+F6 feather · Alt+Ctrl+F last filter · Shift+Ctrl+] / [ bring to
front / send to back · Ctrl+W close tab · Ctrl+Tab next tab · F11 fullscreen ·
scroll wheel zooms at the cursor · Space+drag, middle-drag or Hand tool pans · Zoom tool:
left-click in, right-click out.

## Status

- [x] Step 0: workspace, theme from Figma tokens, full window layout, tool/colour/layer state, tablet spike
- [x] 1: open images (dialog, CLI args), tabs, GPU canvas, zoom/pan, custom title bar
- [x] 2: blend modes on GPU, undo/redo + History panel, .jdp format, export, layer
      duplicate/reorder/rename/thumbnails, unsaved-changes prompts
- [x] 3: brush + eraser with pen pressure (size/opacity), flow/opacity/hardness/smoothing,
      Shift+click lines, eyedropper (and Alt+click), pen eraser end, tool tooltips
- [x] 4: marquee (rect/ellipse), lasso, magic wand, add/subtract/intersect, marching ants,
      clear/fill/copy/cut/paste/layer via copy, Move tool + nudge, Free Transform (Ctrl+T),
      New Document dialog with presets
- [x] 5: Image > Adjustments (Brightness/Contrast, Levels, Curves, Hue/Saturation, Threshold,
      Posterize, Invert, Desaturate) and Filters (Gaussian Blur, Unsharp Mask, Add Noise,
      Mosaic) with live preview, selection-aware
- [x] 5b: menus with submenus; Place, Copy Merged, Stroke, layer Transform (flip/rotate), Auto
      Tone/Contrast, Image Size, Canvas Size, Image Rotation, Crop, Trim, Duplicate, Merge
      Down/Visible, Flatten, Arrange, Reselect, Expand/Contract/Feather, Box Blur, Sharpen,
      High Pass, Last Filter, Window panel toggles; drag layers to reorder or onto another
      document's tab/canvas to copy them
- [x] 5c: content moved off the canvas is kept (and saved); layer right-click menu (Select
      Pixels, Duplicate Into, ...); crossed-out eye for hidden layers; command search (search
      icon or Ctrl+F); workspaces (Essentials, Photography, Painting, Minimal); Clone Stamp
      and Healing Brush (Alt+click source), Dodge/Burn, Gradient (linear/radial), Rectangle
      and Ellipse shapes, Crop tool
- [x] 5d: Spot Healing Brush (paint over something to remove it: patch-based fill like
      Content-Aware), Healing Brush with seamless (Poisson) blending, View > Snap / Snap To
      (layers, document bounds) with smart guides for Move and Free Transform, Layer >
      Blending Options (mode, opacity, live preview), layer drag with exact drop slots
- [x] 5e: layer styles (Layer > Layer Style, right-click a layer, or fx): Drop Shadow, Outer
      Glow, Stroke (outside/inside/center) and Color Overlay, non-destructive and saved in
      .jdp; Clear and Rasterize Layer Style; merges bake effects in
- [x] 6a: Type tool (T): click to add text, click a text layer to edit it, system fonts,
      size, bold/italic, left/center/right; typing is one undo step (Esc cancels, Ctrl+Enter
      or another tool commits); text layers stay editable when moved and are saved in .jdp
- [x] 6b: Image > Remove Background (on-device AI, IS-Net via ONNX Runtime; the 179 MB model
      downloads once after asking); font list drawn in each font; Adjustments panel; Channels
      panel (load a channel as a selection); resizable dock; draggable dialogs
- [x] 6c: layer masks (Layer > Layer Mask, the mask button, right-click a layer): Reveal/Hide
      All or Selection, Delete, Apply, Disable (Shift+click the mask thumbnail). Click a
      thumbnail to pick what edits go to (outlined); every brush, fill, gradient, filter and
      adjustment works on the mask in greys, and the eraser paints the background colour.
      Ctrl+click a thumbnail loads it as a selection. Masks move with their layer and are
      saved in .jdp. Remove Background now adds a mask instead of deleting pixels
- [x] 6d: every value box (options bar, Layers opacity, Color hex/RGB, zoom, Free Transform
      W/H/Angle, adjustment dialogs) can be clicked and typed into: Enter applies, Esc
      cancels, doing anything else applies it; tool preset picker (built-in round brushes,
      saved presets, Reset Tool / Reset All Tools); dock panels resize by the strip below
      them and move by dragging their tab strip (or their ≡ menu); Lock Image Pixels and
      Lock Position
- [ ] 6: PSD import, Flatpak, Pen tool

## App icon

The logo is the suite artwork at `../img/gallerion.svg` (embedded at build time). It shows in
the title bar and as the window icon. On Wayland, KDE takes the taskbar icon from a desktop
entry matching the app id `dev.jamsdesign.Gallerion`: run `packaging/install-desktop.sh
[binary]` once to install it for your user (defaults to the workspace's `../target/release/gallerion`).

## Selections and transforms

Selections live on the document (`jd-raster/src/selection.rs`: 256px tiles of 8-bit coverage,
anti-aliased shapes) and are part of undo, like Photoshop. They aren't saved in .jdp files.
Move/Free Transform previews on the GPU (a per-layer affine) and resamples on commit
(`jd-raster/src/transform.rs`: exact for whole-pixel moves, bilinear otherwise).

## The .jdp format

A zip: `mimetype`, `document.json` (size, blend space, layers), one entry per non-empty
256px tile (raw RGBA8 deflated in parallel), and `thumbnail.png`. Saves are atomic (temp file
then rename). See `crates/jd-io/src/jdp.rs`. A 5120×2880 three-layer document saves in about
0.4 s and opens in under 0.1 s on a 12-core machine.

Try it: `cargo run -p jd-io --example demo_doc -- demo.jdp base.jpg top.jpg`

## Known issues

- Tablet: Gallerion reads the pen through octotablet on a background thread attached to its
  own Wayland window (iced has no pen support). Wayland only; crates.io `octotablet 0.1.0`
  crashes on KDE, so the GitHub master is pinned. Reading the tablet makes the compositor
  stop sending the pen as a mouse, so we replay it as synthetic pointer input through a
  patched `iced_winit` (`vendor/iced_winit`, changes marked `JAMSDESIGN PATCH`). Re-vendor
  and re-apply the patch when upgrading iced. `GALLERION_PEN=0` turns all of this off.
- Pressure reaches the brush-engine tools (Brush, Eraser, Clone Stamp, Healing, Dodge/Burn);
  every other tool uses the pen as a plain pointer.
- Off-canvas content survives moves, transforms, paste, Place, Crop, Canvas Size and saving
  (Crop works like Photoshop with "Delete Cropped Pixels" off), but canvas rotate/flip,
  Image Size, merges and flatten only see the canvas and drop it. Shapes are
  raster (not editable vector layers). Spot Healing runs when the stroke ends and blocks
  the UI briefly on big strokes (about 0.5 s for a 240 px blob). Snapping has no grid or
  guides yet.
- Layer styles: effects are rendered on the CPU and cached, so painting on a large styled
  layer is slower; effects hide while the layer is being moved or transformed and come back
  on commit. Shadow and glow use normal blending (Photoshop defaults to Multiply/Screen).
  Not yet: Bevel, Inner Shadow/Glow, Satin, Gradient/Pattern Overlay.
- Remove Background works on the active layer and hides the background with a layer mask
  (it replaces a mask the layer already had).
- Layer masks: no Alt+click to view the mask alone, no Ctrl+\\ / Ctrl+2 shortcuts to switch
  target, and masks are always linked (moving a selection inside a mask isn't supported).
  It takes about 1.5 s per image on a 12-core CPU; complex hair/fur edges are softer than
  remove.bg. The download has no progress bar yet.
- Type: point text only (no text boxes or wrapping), the caret is always at the end (no
  arrow keys or selecting part of the text), one style per layer. Painting on, filtering or
  free-transforming a text layer turns it into plain pixels without asking. Pasting into a document of a different size lands at the top-left
  (Photoshop centres it). Move and Ctrl+T refuse on a locked layer (unlock it first).
- The clipboard is internal to Gallerion (iced only exposes text clipboard access).
- Marching ants don't animate. Transforms use bilinear resampling (Photoshop uses bicubic by
  default), so large downscales can alias.
- Adjustments are destructive (applied to the layer); adjustment layers come later. Levels and
  Curves work on all channels together (no per-channel R/G/B yet). Brightness/Contrast is close
  to, not identical to, Photoshop's. Large blurs take about 0.4 s per update on a 15 MP layer,
  and OK waits for the final render if the preview hasn't caught up. Filters refuse images over
  120 megapixels (they need a large working copy). Adjustment dialogs are modal, including the
  title bar.
- Dissolve as a brush mode paints like Normal for now (it's handled as a layer mode only).
- Drag-and-drop to open doesn't work on Wayland: winit 0.30 has no Wayland drag-and-drop
  support (it only works under X11). Fix later via our own Wayland data-device listener or
  a winit upgrade.
- No Inter font bundled yet; iced uses the system sans.
- Images are converted to 8-bit sRGB on load and ICC profiles are ignored. 16-bit/float and
  colour management are their own upcoming step.
- Burn/dodge blend modes can differ from the CPU export by a few levels on a handful of
  near-black/near-white pixels (GPU stores the composite as half floats).
- GPU tiles are uploaded for the active document only and aren't memory-budgeted yet.
- The window has no system decorations (custom title bar). Resize from the 5px window edges;
  drag the title bar to move; double-click it to maximise.
