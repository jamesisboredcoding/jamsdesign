#!/usr/bin/env python3
"""Writes blobr_window.json: the ops that build Blobr's main-window mockup.

Two batches, in the MCP server's op format. "icons" makes the icon components; "window"
makes the other components (tool button, panel tab, layer row, section header, field,
paint row) and the window, built from their instances. "$icon/name" strings in "window" stand
for the icon components' ids, which the example fills in after the first batch.

python3 blobr_window.py  (run from this folder)
"""

import json

BG = "#1E1E1E"
PANEL = "#252525"
CANVAS = "#181818"
BORDER = "#333333"
TEXT = "#E8E8E8"
MUTED = "#8C8C8C"
ACCENT = "#4C8DFF"
FONT = "Noto Sans"


def solid(c, opacity=None):
    p = {"type": "solid", "color": c}
    if opacity is not None:
        p["opacity"] = opacity
    return [p]


def text(name, content, size=12, color=TEXT, style="Regular", **extra):
    node = {"type": "text", "name": name, "text": {"content": content, "style": {"family": FONT, "size": size, "style": style}},
            "fills": solid(color), "sizing_h": "hug", "sizing_v": "hug"}
    node.update(extra)
    return node


def row(name, gap=0, pad=0, align="start", counter="center", **extra):
    node = {"type": "frame", "name": name, "fills": [], "clip": False,
            "layout": {"direction": "horizontal", "gap": gap, "padding": pad, "align": align, "counter_align": counter},
            "sizing_h": "hug", "sizing_v": "hug"}
    node.update(extra)
    return node


def col(name, gap=0, pad=0, counter="start", **extra):
    node = row(name, gap, pad, counter=counter, **extra)
    node["layout"]["direction"] = "vertical"
    return node


ops = []


def create(parent, node, key=None):
    op = {"op": "create", "parent": parent, "node": node}
    if key:
        op["key"] = key
    ops.append(op)
    return key


def instance(parent, component, key=None, **values):
    op = {"op": "create_instance", "parent": parent, "component": component}
    if key:
        op["key"] = key
    ops.append(op)
    return op


def set_(ref, **patch):
    ops.append({"op": "set", "id": ref, "patch": patch})


# --- Batch 1: icons (16 x 16 outlines on a 24 x 24 grid) ---

ICONS = {
    "move": "M6 3 L19 12 L12.5 13.5 L9.5 20 Z",
    "frame": "M8 3 V21 M16 3 V21 M3 8 H21 M3 16 H21",
    "rectangle": "M4 5 H20 V19 H4 Z",
    "ellipse": "M12 4 C16.4 4 20 7.6 20 12 C20 16.4 16.4 20 12 20 C7.6 20 4 16.4 4 12 C4 7.6 7.6 4 12 4 Z",
    "pen": "M12 3 L18 13 L12 21 L6 13 Z M12 3 V11",
    "text": "M5 5 H19 M12 5 V20",
    "comment": "M4 5 H20 V16 H10 L5 20 V16 H4 Z",
    "hand": "M8 12 V6 M11 11 V4 M14 11 V5 M17 12 V8 M8 12 C8 18 10 20 13 20 C16 20 17 18 17 14",
    "search": "M10.5 4 C14.1 4 17 6.9 17 10.5 C17 14.1 14.1 17 10.5 17 C6.9 17 4 14.1 4 10.5 C4 6.9 6.9 4 10.5 4 Z M15.5 15.5 L20 20",
    "page": "M6 3 H14 L18 7 V21 H6 Z",
    "hash": "M9 4 L7 20 M17 4 L15 20 M4 9 H20 M4 15 H20",
    "type": "M6 6 H18 M12 6 V19",
    "component": "M12 3 L16 7 L12 11 L8 7 Z M12 13 L16 17 L12 21 L8 17 Z M7 8 L11 12 L7 16 L3 12 Z M17 8 L21 12 L17 16 L13 12 Z",
    "plus": "M12 5 V19 M5 12 H19",
    "chevron": "M8 10 L12 14 L16 10",
    "eye": "M3 12 C6 7 18 7 21 12 C18 17 6 17 3 12 Z M12 10 C13.1 10 14 10.9 14 12 C14 13.1 13.1 14 12 14 C10.9 14 10 13.1 10 12 C10 10.9 10.9 10 12 10 Z",
    "minus": "M5 12 H19",
}
for name, d in ICONS.items():
    k = f"icon/{name}"
    create(1, {"type": "component", "name": k, "x": 1700 + 24 * list(ICONS).index(name), "y": 1100, "width": 16, "height": 16,
               "fills": [], "clip": False}, key=k)
    create(k, {"type": "vector", "name": "Glyph", "svg_path": d, "svg_viewbox": [24, 24], "width": 16, "height": 16,
               "fills": [], "strokes": solid(TEXT), "stroke": {"weight": 1.4, "align": "center", "cap": "round", "join": "round"},
               "constraints": {"horizontal": "scale", "vertical": "scale"}})
icons = ops
ops = []

# --- Batch 2: components ---

create(1, {"type": "frame", "name": "Components", "x": 0, "y": 1040, "width": 1640, "height": 260, "fills": solid("#141414"),
           "corner": 12}, key="lib")
create("lib", text("Heading", "Components", 14, MUTED, "Bold", x=24, y=16))


def icon_slot(parent, prop, default, color=TEXT):
    """A nested icon instance whose component is the `prop` property of the component."""
    instance(parent, default, key=f"{parent}.{prop}")
    set_(f"{parent}.{prop}", refs={"swap": prop})


def tool_button(key, state, x):
    create("lib", {"type": "component", "name": f"State={state}", "x": x, "y": 56, "width": 32, "height": 32, "corner": 8,
                   "fills": solid(ACCENT) if state == "Active" else [],
                   "layout": {"direction": "horizontal", "align": "center", "counter_align": "center"}}, key=key)
    icon_slot(key, "Icon", "$icon/move")
    set_(key, properties={"Icon": {"type": "instance_swap", "default": "$icon/move"}})


tool_button("tool.default", "Default", 24)
tool_button("tool.active", "Active", 64)
ops.append({"op": "combine_as_variants", "ids": ["tool.default", "tool.active"], "name": "Tool Button", "key": "Tool Button"})
set_("Tool Button", x=24, y=48)


def panel_tab(key, state, x):
    create("lib", {"type": "component", "name": f"State={state}", "x": x, "y": 56, "height": 26, "corner": 4,
                   "fills": solid("#2E2E2E") if state == "Active" else [],
                   "strokes": solid(ACCENT, 0.6) if state == "Active" else [], "stroke": {"weight": 1, "align": "inside"},
                   "layout": {"direction": "horizontal", "padding": {"top": 0, "right": 10, "bottom": 0, "left": 10},
                              "counter_align": "center"}, "sizing_h": "hug"}, key=key)
    create(key, text("Label", "Layers", 12, TEXT if state == "Active" else MUTED, refs={"text": "Label"}))
    set_(key, properties={"Label": {"type": "text", "default": "Layers"}})


panel_tab("tab.active", "Active", 160)
panel_tab("tab.inactive", "Inactive", 240)
ops.append({"op": "combine_as_variants", "ids": ["tab.active", "tab.inactive"], "name": "Panel Tab", "key": "Panel Tab"})
set_("Panel Tab", x=140, y=48)


def layer_row(key, state, y):
    create("lib", {"type": "component", "name": f"State={state}", "x": 360, "y": y, "width": 248, "height": 26,
                   "fills": solid(ACCENT, 0.25) if state == "Selected" else [],
                   "layout": {"direction": "horizontal", "gap": 6, "counter_align": "center",
                              "padding": {"top": 0, "right": 12, "bottom": 0, "left": 12}}}, key=key)
    create(key, {"type": "frame", "name": "Indent", "width": 12, "height": 16, "fills": [], "refs": {"visible": "Nested"}})
    icon_slot(key, "Icon", "$icon/hash")
    create(key, text("Label", "Layer", 12, refs={"text": "Label"}))
    set_(key, properties={"Label": {"type": "text", "default": "Layer"}, "Icon": {"type": "instance_swap", "default": "$icon/hash"},
                          "Nested": {"type": "boolean", "default": False}})


layer_row("layer.default", "Default", 56)
layer_row("layer.selected", "Selected", 90)
ops.append({"op": "combine_as_variants", "ids": ["layer.default", "layer.selected"], "name": "Layer Row", "key": "Layer Row"})
set_("Layer Row", x=340, y=48)

create("lib", {"type": "component", "name": "Section Header", "x": 660, "y": 56, "width": 276, "height": 36, "fills": [],
               "layout": {"direction": "horizontal", "align": "space_between", "counter_align": "center",
                          "padding": {"top": 0, "right": 12, "bottom": 0, "left": 12}}}, key="Section Header")
create("Section Header", text("Title", "Section", 12, TEXT, "Bold", refs={"text": "Title"}))
instance("Section Header", "$icon/plus", key="sh.plus")
set_("sh.plus", refs={"visible": "Add"})
set_("Section Header", properties={"Title": {"type": "text", "default": "Section"}, "Add": {"type": "boolean", "default": True}})

create("lib", {"type": "component", "name": "Field", "x": 660, "y": 110, "width": 120, "height": 28, "corner": 4,
               "fills": solid("#2A2A2A"),
               "layout": {"direction": "horizontal", "gap": 8, "counter_align": "center",
                          "padding": {"top": 0, "right": 8, "bottom": 0, "left": 8}}}, key="Field")
create("Field", text("Label", "W", 11, MUTED, refs={"text": "Label"}))
create("Field", text("Value", "100", 12, TEXT, refs={"text": "Value"}))
set_("Field", properties={"Label": {"type": "text", "default": "W"}, "Value": {"type": "text", "default": "100"}})

create("lib", {"type": "component", "name": "Paint Row", "x": 960, "y": 56, "width": 252, "height": 28, "fills": [],
               "layout": {"direction": "horizontal", "gap": 8, "counter_align": "center"}}, key="Paint Row")
create("Paint Row", {"type": "rectangle", "name": "Swatch", "width": 16, "height": 16, "corner": 3, "fills": solid(ACCENT), "strokes": []})
create("Paint Row", text("Hex", "4C8DFF", 12, refs={"text": "Hex"}, sizing_h="fill"))
create("Paint Row", text("Opacity", "100%", 12, MUTED))
instance("Paint Row", "$icon/eye")
instance("Paint Row", "$icon/minus")
set_("Paint Row", properties={"Hex": {"type": "text", "default": "4C8DFF"}})

# --- The window ---

create(1, {"type": "frame", "name": "Blobr - Main Window", "x": 0, "y": 0, "width": 1600, "height": 960, "corner": 10,
           "fills": solid(BG), "clip": True, "layout": {"direction": "vertical"}}, key="win")

# Menu bar.
create("win", row("Menu bar", gap=18, pad={"top": 0, "right": 12, "bottom": 0, "left": 12}, sizing_h="fill", sizing_v="fixed",
                  height=32, fills=solid("#1A1A1A")), key="menu")
create("menu", {"type": "rectangle", "name": "Logo", "width": 20, "height": 20, "corner": 5, "fills": solid("#7B61FF"), "strokes": []})
for m in ["File", "Edit", "View", "Object", "Text", "Arrange", "Plugins", "Window", "Help"]:
    create("menu", text(m, m, 13))
title = text("Title", "Onboarding Flow.jdb - Blobr", 12, MUTED, sizing_h="fill")
title["text"]["style"]["align"] = "center"
create("menu", title)
create("menu", row("Export", gap=6, pad={"top": 4, "right": 10, "bottom": 4, "left": 10}, corner=5, fills=solid(ACCENT)), key="export")
create("export", text("Label", "Export", 12, "#FFFFFF", "Bold"))
for c in ["#3A3A3A", "#3A3A3A", "#E5484D"]:
    create("menu", {"type": "ellipse", "name": "Window control", "width": 12, "height": 12, "fills": solid(c), "strokes": []})

# Body: toolbar, left panel, canvas, right panel.
create("win", row("Body", sizing_h="fill", sizing_v="fill", counter="start"), key="body")

create("body", col("Toolbar", gap=6, pad={"top": 8, "right": 8, "bottom": 8, "left": 8}, counter="center",
                   sizing_v="fill", fills=solid("#1A1A1A")), key="tools")
for i, name in enumerate(["move", "frame", "rectangle", "ellipse", "pen", "text", "comment", "hand", "search"]):
    k = f"tool{i}"
    instance("tools", "tool.active" if i == 0 else "tool.default", key=k)
    set_(k, values={"Icon": f"$icon/{name}"})

create("body", col("Left panel", width=248, sizing_h="fixed", sizing_v="fill", fills=solid(PANEL),
                   strokes=solid(BORDER), stroke={"weight": 1, "align": "inside"}), key="left")
create("left", row("Tabs", gap=4, pad=8, sizing_h="fill"), key="ltabs")
for k, label, state in [("lt1", "Layers", "Active"), ("lt2", "Assets", "Inactive")]:
    instance("ltabs", "tab.active" if state == "Active" else "tab.inactive", key=k)
    set_(k, values={"Label": label})
instance("left", "Section Header", key="pages.h")
set_("pages.h", values={"Title": "Pages"}, sizing_h="fill")
for i, (label, sel) in enumerate([("Cover", False), ("Onboarding", True), ("Components", False), ("Archive", False)]):
    k = f"page{i}"
    instance("left", "layer.selected" if sel else "layer.default", key=k)
    set_(k, values={"Label": label, "Icon": "$icon/page"}, sizing_h="fill")
instance("left", "Section Header", key="layers.h")
set_("layers.h", values={"Title": "Layers", "Add": False}, sizing_h="fill")
LAYERS = [("Sign up", "hash", False, False), ("Interests", "hash", False, False), ("Welcome", "hash", False, False),
          ("Get started", "component", True, True), ("Already have an account?", "type", True, False),
          ("Subtitle", "type", True, False), ("Title", "type", True, False), ("Pager dots", "hash", True, False),
          ("Hero blobs", "hash", True, False), ("Status bar", "component", True, False)]
for i, (label, icon, nested, sel) in enumerate(LAYERS):
    k = f"layer{i}"
    instance("left", "layer.selected" if sel else "layer.default", key=k)
    set_(k, values={"Label": label, "Icon": f"$icon/{icon}", "Nested": nested}, sizing_h="fill")

# Canvas with the three onboarding screens.
create("body", {"type": "frame", "name": "Canvas", "fills": solid(CANVAS), "sizing_h": "fill", "sizing_v": "fill", "clip": True},
       key="canvas")
create("canvas", row("Tabs", gap=2, pad={"top": 4, "right": 0, "bottom": 0, "left": 4}, x=0, y=0, fills=[]), key="dtabs")
for k, label, state in [("dt1", "Onboarding Flow.jdb @ 50%", "Active"), ("dt2", "Design System.jdb @ 100%", "Inactive")]:
    instance("dtabs", "tab.active" if state == "Active" else "tab.inactive", key=k)
    set_(k, values={"Label": label})

SCREENS = [
    ("Welcome", ["Make something", "wonderful"], "Sketch, plan and ship ideas together, on a canvas that never runs out.", "Get started"),
    ("Interests", ["What do you like", "making?"], "Step 2 of 3", "Continue"),
    ("Sign up", ["Create your account"], "Step 3 of 3", "Create account"),
]
for i, (name, title, sub, cta) in enumerate(SCREENS):
    x = 150 + i * 256
    sk = f"s{i}"
    create("canvas", text(f"{name} label", name, 11, ACCENT if i == 0 else MUTED, x=x, y=226))
    create("canvas", {"type": "frame", "name": name, "x": x, "y": 244, "width": 196, "height": 426, "fills": solid("#FAF6F0"),
                      "layout": {"direction": "vertical", "gap": 8, "padding": {"top": 12, "right": 12, "bottom": 16, "left": 12}}},
           key=sk)
    create(sk, text("Status bar", "9:41", 7, "#1A1A1A", "Bold"))
    if i == 0:
        create(sk, {"type": "frame", "name": "Hero blobs", "width": 172, "height": 170, "fills": [], "sizing_h": "fill"}, key="hero")
        create("hero", {"type": "ellipse", "name": "Coral", "x": 18, "y": 28, "width": 100, "height": 96, "fills": solid("#FF7A59"), "strokes": []})
        create("hero", {"type": "ellipse", "name": "Violet", "x": 80, "y": 60, "width": 66, "height": 66, "fills": solid("#8B5CF6"), "strokes": []})
        create("hero", {"type": "ellipse", "name": "Sun", "x": 10, "y": 98, "width": 36, "height": 36, "fills": solid("#F5C542"), "strokes": []})
    else:
        create(sk, text("Step", sub, 7, ACCENT))
    create(sk, text("Title", "\n".join(title), 15, "#141414", "Bold"))
    if i == 0:
        create(sk, text("Subtitle", sub, 7.5, "#6B6B6B", sizing_h="fill"))
    if i == 1:
        create(sk, {"type": "frame", "name": "Chips", "fills": [], "sizing_h": "fill", "sizing_v": "hug",
                    "layout": {"direction": "horizontal", "wrap": True, "gap": 4, "row_gap": 4}}, key="chips")
        for j, c in enumerate(["Illustration", "UI design", "Posters", "Typography", "Motion", "3D", "Photography", "Branding", "Icons"]):
            on = c in ("UI design", "Posters", "Motion")
            create("chips", row(c, pad={"top": 3, "right": 7, "bottom": 3, "left": 7}, corner=10,
                                fills=solid("#1A1A1A" if on else "#FFFFFF"), strokes=solid("#DDDDDD"),
                                stroke={"weight": 0.5, "align": "inside"}), key=f"chip{j}")
            create(f"chip{j}", text("Label", c, 6.5, "#FFFFFF" if on else "#333333"))
    if i == 2:
        for j, (lab, val) in enumerate([("Name", "Robin Ash"), ("Email", "robin@example.com"), ("Password", "At least 8 characters")]):
            create(sk, text(f"{lab} label", lab, 6, "#6B6B6B"))
            create(sk, row(f"{lab} field", pad={"top": 6, "right": 6, "bottom": 6, "left": 6}, sizing_h="fill", corner=4,
                           fills=solid("#FFFFFF"), strokes=solid(ACCENT if j == 1 else "#E2E2E2"), stroke={"weight": 1, "align": "inside"}),
                   key=f"f{j}")
            create(f"f{j}", text("Value", val, 7, "#1A1A1A" if j < 2 else "#AAAAAA"))
    create(sk, {"type": "frame", "name": "Spacer", "fills": [], "sizing_h": "fill", "sizing_v": "fill", "height": 1})
    create(sk, row(cta, pad=8, align="center", sizing_h="fill", corner=6, fills=solid(ACCENT)), key=f"cta{i}")
    create(f"cta{i}", text("Label", cta, 8, "#FFFFFF", "Bold"))

# Right panel.
create("body", col("Right panel", width=276, sizing_h="fixed", sizing_v="fill", fills=solid(PANEL),
                   strokes=solid(BORDER), stroke={"weight": 1, "align": "inside"}), key="right")
create("right", row("Tabs", gap=4, pad=8, sizing_h="fill"), key="rtabs")
for k, label, state in [("rt1", "Design", "Active"), ("rt2", "Prototype", "Inactive"), ("rt3", "Inspect", "Inactive")]:
    instance("rtabs", "tab.active" if state == "Active" else "tab.inactive", key=k)
    set_(k, values={"Label": label})
create("right", row("Selection", gap=8, pad={"top": 6, "right": 12, "bottom": 10, "left": 12}, sizing_h="fill"), key="selrow")
instance("selrow", "$icon/component", key="selicon")
create("selrow", text("Name", "Button / Primary", 12, "#A88BFF", "Bold"))


def section(title, add=True):
    k = f"sec.{title}"
    instance("right", "Section Header", key=k)
    set_(k, values={"Title": title, "Add": add}, sizing_h="fill")


def fields(key, pairs):
    create("right", row(key, gap=8, pad={"top": 2, "right": 12, "bottom": 4, "left": 12}, sizing_h="fill"), key=key)
    for j, (lab, val) in enumerate(pairs):
        k = f"{key}.{j}"
        instance(key, "Field", key=k)
        set_(k, values={"Label": lab, "Value": val}, sizing_h="fill")


section("Layout", add=False)
fields("xy", [("X", "24"), ("Y", "668")])
fields("wh", [("W", "345"), ("H", "52")])
fields("rot", [("°", "0"), ("R", "14")])
section("Auto layout", add=False)
fields("gap", [("Gap", "8"), ("Pad", "24, 16")])
fields("sizing", [("W", "Fill"), ("H", "Hug")])
section("Fill")
create("right", row("Fills", pad={"top": 0, "right": 12, "bottom": 4, "left": 12}, sizing_h="fill"), key="fills")
instance("fills", "Paint Row", key="paint")
set_("paint", sizing_h="fill")
section("Stroke")
section("Effects")
section("Export")

# Status bar.
create("win", row("Status bar", gap=16, pad={"top": 0, "right": 12, "bottom": 0, "left": 60}, sizing_h="fill", sizing_v="fixed",
                  height=28, fills=solid("#1A1A1A")), key="status")
for label in ["50%", "Welcome · Phone 393 × 852", "3 frames · 1 selected"]:
    create("status", text(label, label, 11, MUTED))
create("status", {"type": "frame", "name": "Spacer", "fills": [], "sizing_h": "fill", "height": 1})
create("status", text("Saved", "Saved", 11, MUTED))

with open("blobr_window.json", "w") as f:
    json.dump({"icons": icons, "window": ops}, f, indent=1, ensure_ascii=False)
    f.write("\n")
print(f"{len(icons)} + {len(ops)} ops")
