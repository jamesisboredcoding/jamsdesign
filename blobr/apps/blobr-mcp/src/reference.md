# Blobr document reference

A Blobr document (`.jdb`) is a tree: pages hold frames, groups, sections and shapes; frames
hold more of the same. Every node has a numeric id. Coordinates are pixels, relative to the
parent (a page's children use page coordinates). Children are listed bottom to top: the last
child is drawn on top.

Change documents with `apply_ops`: a list of ops that runs as one atomic batch (and one undo
step). If any op fails, nothing changes and the error names the op. Give created nodes a
`key` to refer to them later in the same batch (anywhere an id goes, a string is a key):

```json
[
  {"op": "create", "parent": 1, "key": "card", "node": {"type": "frame", "name": "Card", "x": 40, "y": 40, "width": 320, "height": 180,
    "fills": [{"type": "solid", "color": "#FFFFFF"}], "corner": {"top_left": 16, "top_right": 16, "bottom_right": 16, "bottom_left": 16},
    "effects": [{"type": "drop_shadow", "color": "#00000040", "y": 4, "blur": 16}],
    "layout": {"direction": "vertical", "gap": 8, "padding": {"top": 24, "right": 24, "bottom": 24, "left": 24}}, "sizing_v": "hug"}},
  {"op": "create", "parent": "card", "node": {"type": "text", "name": "Title", "text": {"content": "Hello", "style": {"family": "Inter", "style": "Bold", "size": 24}},
    "sizing_h": "hug", "sizing_v": "hug", "fills": [{"type": "solid", "color": "#1A1A1A"}]}},
  {"op": "create", "parent": "card", "node": {"type": "rectangle", "name": "Divider", "height": 1, "sizing_h": "fill", "fills": [{"type": "solid", "color": "#E5E5E5"}]}}
]
```

## Node types

Nodes start with what the matching drawing tool gives them (100 x 100; frames white and
clipping, text black, vectors a 1 px black outline unless you give them `fills`); anything
you set wins, and `"fills": []` / `"strokes": []` remove them.

`frame`, `group` (fits its children; no fill), `section`,
`rectangle`, `ellipse` (`arc: {start, sweep, inner}` in degrees / ratio), `line` (from (0,0)
to (width,0); rotate for other angles), `polygon` (`sides`), `star` (`points`, `inner`),
`vector` (see Paths), `text` (`text: {content, style}`), `component`, `component_set`,
`instance` (create with `create_instance`).

## Properties (all optional when creating; defaults shown)

- `x`, `y`, `width` (100), `height` (100), `rotation` (degrees, counter-clockwise), `visible`
  (true), `locked`, `opacity` (0 to 1), `blend` (`normal`, `multiply`, `screen`, ...)
- `fills`, `strokes`: lists of paints, bottom to top:
  - `{"type": "solid", "color": "#RRGGBB" or "#RRGGBBAA"}`
  - `{"type": "linear_gradient", "from": [0, 0], "to": [1, 1], "stops": [{"position": 0, "color": "#FF5A5F"}, {"position": 1, "color": "#7B61FF"}]}`
    (points are fractions of the node's box)
  - `{"type": "radial_gradient", "center": [0.5, 0.5], "radius": [0.5, 0.5], "stops": [...]}`
  - any paint can have `opacity`, `visible`, `blend`
- `stroke`: `{"weight": 1, "align": "inside"|"center"|"outside", "dash": [4, 2], "cap": "none"|"round"|"square", "join": "miter"|"round"|"bevel"}`
  (only drawn when `strokes` has a paint)
- `corner`: `{"top_left", "top_right", "bottom_right", "bottom_left"}`
- `effects`: `{"type": "drop_shadow"|"inner_shadow", "color", "x", "y", "blur", "spread"}`,
  `{"type": "layer_blur"|"background_blur", "radius"}`
- `clip` (frames: hide what's outside)
- Auto layout (frames): `layout: {"direction": "horizontal"|"vertical", "gap", "padding": {top, right, bottom, left}, "align": "start"|"center"|"end"|"space_between", "counter_align": "start"|"center"|"end", "wrap", "row_gap"}`
  - `sizing_h` / `sizing_v`: `"fixed"` (default), `"hug"` (fit content: needs auto layout or
    text), `"fill"` (take the free space: needs an auto layout parent)
  - `absolute: true` keeps a child out of the flow at its `x`/`y`
  - `min_width`, `max_width`, `min_height`, `max_height`
- `constraints` (children of frames without auto layout): `{"horizontal": "start"|"end"|"both"|"center"|"scale", "vertical": ...}`
- Text: `text: {"content": "...", "style": {"family": "Inter", "style": "Regular", "size": 14, "line_height": "auto" | {"px": 20} | {"percent": 140}, "letter_spacing": 0, "align": "left"|"center"|"right"|"justified", "valign": "top"|"center"|"bottom"}}`.
  The text colour is its `fills`. A text box with `sizing_h: "hug"` grows to fit; with a
  fixed width and `sizing_v: "hug"` it wraps. Use `list_fonts` to see installed families;
  `style` is the face name (`Regular`, `Medium`, `SemiBold`, `Bold`, `Italic`, ...).

## Paths

Vectors take SVG path data: `{"type": "vector", "svg_path": "M0 0 L24 12 L0 24 Z", "width": 24, "height": 24}`.
The path is drawn in a `width` x `height` box (or give `"svg_viewbox": [w, h]` when the
path's coordinates use another box, and the node's size scales it). Whole SVGs (icons,
illustrations) are easier with the `import_svg` tool.

## Ops

- `create`: `{"op": "create", "parent": ref, "node": {...}, "key"?, "index"?}` (`index` counts
  from the bottom; default on top)
- `set`: `{"op": "set", "id": ref, "patch": {...}}`: any property above, plus `name`, `text`
  (a string: the characters), `text_style`, `svg_path` (vectors), `sides`, `points`,
  `inner`, `arc`, `background` (pages). `null` removes optional ones (`"layout": null`).
  Setting `width`/`height` makes that axis fixed.
- `resize`: `{"op": "resize", "id", "width", "height"}`: like dragging a handle (children
  follow their constraints)
- `move`: `{"op": "move", "id", "parent", "index"?}`: reparent or restack (keeps x/y numbers)
- `duplicate`: `{"op": "duplicate", "id", "key"?}`
- `delete`: `{"op": "delete", "id"}`
- `create_page`: `{"op": "create_page", "name", "key"?, "index"?}`
- Components:
  - `{"op": "make_component", "id"}` turns a frame into a component
  - `{"op": "create_instance", "parent", "component", "x", "y", "key"?}`
  - `{"op": "combine_as_variants", "ids": [...], "name", "key"?}` (components named like
    `Size=Large, State=Default`)
  - `{"op": "detach", "id"}`, `{"op": "reset_overrides", "id"}`
  - On a component: `set` with `properties: {"Label": {"type": "text", "default": "Button"}, "Icon": {"type": "boolean", "default": true}}`,
    and on layers inside it `refs: {"text": "Label"}` / `{"visible": "Icon"}`
  - On an instance: `set` with `values: {"Label": "Buy now", "Size": "Large"}` (a variant
    property switches variant). Edits to layers inside an instance are kept as overrides.
- Variables and styles:
  - `{"op": "create_collection", "name", "modes": ["Light", "Dark"], "key"?}`
  - `{"op": "create_variable", "collection", "name": "color/bg", "values": {"Light": {"color": "#FFFFFF"}, "Dark": {"color": "#111111"}}, "key"?}`
    (values: `{"color": "#..."}`, `{"number": 8}`, `{"text": "..."}`, `{"bool": true}`, `{"alias": id}`)
  - Bind with `set` + `bind: {"fills/0": variable, "gap": variable, "corner": variable}`
    (paths: x, y, width, height, rotation, opacity, visible, text, stroke_weight, font_size,
    gap, row_gap, corner[/top_left...], padding[/top...], fills/N, strokes/N); pick a mode
    for a subtree with `modes: {collection: "Dark"}`
  - `{"op": "create_style", "name", "style": {"type": "paint", "paints": [...]}|{"type": "text", "text": {...}}|{"type": "effect", "effects": [...]}, "key"?}`,
    apply with `set` + `styles: {"fill"|"stroke"|"text"|"effect": style}`
  - `set_collection`, `set_variable` (`name`, `mode`, `value`), `set_style`, `delete_token`
- Prototype: `set` + `reactions: [{"trigger": {"type": "click"}, "action": {"type": "navigate", "to": frame}, "transition": "dissolve", "duration": 0.3}]`

## Tips

- Build a screen in a few batches (a frame and its sections, then the contents), and check
  it with `render` after each. Read ids back from the result's `keys`.
- Prefer auto layout for anything with rows or columns: children then position themselves,
  and text changes reflow.
- `get_tree` for an outline, `query` to find nodes (`frame[name=Card] > text`), `get_node`
  for every property of a few nodes.
- `arrange` covers grouping, alignment, distribution, stacking order and boolean operations.
