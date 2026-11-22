//! Popup menus: title-bar dropdowns, the canvas right-click menu and toolbar flyouts.

use iced::widget::{column, mouse_area, pin, space, stack};
use iced::{Element, Fill};
use jd_raster::BlendSpace;
use jd_ui::widget::{MenuEntry, menu, menu_height};

use super::chrome::MENUS;
use crate::adjust::OpKind;
use crate::app::{DialogKind, MaskOp, Message, Orient, Panel, Photo, PopupKind, SnapTo, Zoom};
use crate::tool::{self, Tool};

const WIDTH: f32 = 260.0;

/// The open popup (and its submenu), over a click-catcher that closes it.
pub fn popup(app: &Photo) -> Option<Element<'_, Message>> {
    let p = app.popup?;
    if p.kind == PopupKind::Font {
        return Some(font_list(app, p.at));
    }
    let (entries, width) = match p.kind {
        PopupKind::Context => (context(app), WIDTH),
        PopupKind::MenuBar(i) => (menu_bar(app, i), WIDTH),
        PopupKind::ToolSlot(slot) => (tool_slot(slot), 220.0),
        PopupKind::Layer(_) => (layer_menu(app), WIDTH),
        PopupKind::Workspace => (workspaces(app), 200.0),
        PopupKind::ToolPresets => (tool_presets(app), 300.0),
        PopupKind::Panel(panel) => (panel_menu(app, panel), 200.0),
        PopupKind::NewAdjustment => (new_adjustment(app), 220.0),
        PopupKind::Font => return None,
    };
    let height = menu_height(&entries);
    // Keep the menu inside the window.
    let x = p.at.x.min(app.window_size.width - width - 4.0).max(0.0);
    let y = if p.at.y + height > app.window_size.height { (p.at.y - height).max(0.0) } else { p.at.y };
    let view = menu(entries, width, p.sub, Message::PopupSub);

    let catcher = mouse_area(space().width(Fill).height(Fill))
        .on_press(Message::ClosePopup)
        .on_right_press(Message::ClosePopup);
    // Title-bar menus leave the title bar uncovered, so clicking or hovering another title
    // switches menus (like any desktop menu bar).
    let catcher: Element<'_, Message> = if matches!(p.kind, PopupKind::MenuBar(_)) {
        column![space().height(30), catcher].into()
    } else {
        catcher.into()
    };
    let mut layers = stack![catcher, pin(view.main).x(x).y(y)];
    if let Some((offset, sub)) = view.sub {
        // Open to the right, or to the left when there's no room.
        let sx = if x + 2.0 * width - 6.0 > app.window_size.width { x - width + 6.0 } else { x + width - 6.0 };
        layers = layers.push(pin(sub).x(sx.max(0.0)).y(y + offset));
    }
    Some(layers.into())
}

fn when(enabled: bool, m: Message) -> Option<Message> {
    enabled.then(|| Message::Menu(Box::new(m)))
}

fn item<'a>(label: &'a str, shortcut: &'a str, enabled: bool, m: Message) -> MenuEntry<'a, Message> {
    MenuEntry::Item(label, shortcut, when(enabled, m))
}

fn check(on: bool) -> &'static str {
    if on { "✓" } else { "" }
}

/// What the current document allows; menus grey out what doesn't apply.
struct State {
    doc: bool,
    sel: bool,
    editable: bool,
    undo: bool,
    redo: bool,
    clip: bool,
    can_merge_down: bool,
    reselect: bool,
}

fn state(app: &Photo) -> State {
    let d = app.document();
    let index = d.and_then(|d| d.active_id().and_then(|id| d.layers().iter().position(|l| l.id == id)));
    State {
        doc: d.is_some(),
        sel: d.is_some_and(|d| d.selection.is_some()),
        editable: d.and_then(|d| d.active()).is_some_and(|l| !l.locked),
        undo: app.doc().is_some_and(|d| d.history.can_undo()),
        redo: app.doc().is_some_and(|d| d.history.can_redo()),
        clip: app.clipboard.is_some(),
        can_merge_down: index.is_some_and(|i| i > 0),
        reselect: app.doc().is_some_and(|d| d.last_selection.is_some()),
    }
}

fn adjustments<'a>(s: &State) -> Vec<MenuEntry<'a, Message>> {
    let op = |label, keys, kind| item(label, keys, s.doc && s.editable, Message::OpenOp(kind));
    vec![
        op("Brightness/Contrast...", "", OpKind::BrightnessContrast),
        op("Levels...", "Ctrl+L", OpKind::Levels),
        op("Curves...", "Ctrl+M", OpKind::Curves),
        MenuEntry::Separator,
        op("Hue/Saturation...", "Ctrl+U", OpKind::HueSaturation),
        op("Desaturate", "Shift+Ctrl+U", OpKind::Desaturate),
        MenuEntry::Separator,
        op("Invert", "Ctrl+I", OpKind::Invert),
        op("Threshold...", "", OpKind::Threshold),
        op("Posterize...", "", OpKind::Posterize),
    ]
}

fn transform_entries<'a>(s: &State) -> Vec<MenuEntry<'a, Message>> {
    let ok = s.doc && s.editable;
    vec![
        item("Rotate 180°", "", ok, Message::OrientLayer(Orient::Rotate180)),
        item("Rotate 90° Clockwise", "", ok, Message::OrientLayer(Orient::Rotate90Cw)),
        item("Rotate 90° Counter Clockwise", "", ok, Message::OrientLayer(Orient::Rotate90Ccw)),
        MenuEntry::Separator,
        item("Flip Horizontal", "", ok, Message::OrientLayer(Orient::FlipH)),
        item("Flip Vertical", "", ok, Message::OrientLayer(Orient::FlipV)),
    ]
}

/// Right-click on the canvas, like Photoshop's: what you can do with the selection, or the layer.
fn context(app: &Photo) -> Vec<MenuEntry<'_, Message>> {
    use MenuEntry::Separator;
    let s = state(app);
    if !s.doc {
        return vec![item("No document open", "", false, Message::Noop)];
    }
    if app.transform.is_some() {
        return vec![
            item("Commit Transform", "Enter", true, Message::CommitTransform),
            item("Cancel Transform", "Esc", true, Message::CancelTransform),
        ];
    }
    if s.sel {
        vec![
            item("Deselect", "Ctrl+D", true, Message::Deselect),
            item("Select Inverse", "Shift+Ctrl+I", true, Message::InvertSelection),
            item("Feather...", "Shift+F6", true, Message::ShowDialog(DialogKind::Feather)),
            Separator,
            item("Layer via Copy", "Ctrl+J", true, Message::LayerVia { cut: false }),
            item("Layer via Cut", "Shift+Ctrl+J", s.editable, Message::LayerVia { cut: true }),
            item("New Layer", "Shift+Ctrl+N", true, Message::AddLayer),
            Separator,
            item("Free Transform", "Ctrl+T", s.editable, Message::FreeTransform),
            MenuEntry::Submenu("Transform", transform_entries(&s)),
            Separator,
            item("Fill with Foreground", "Alt+Backspace", s.editable, Message::Fill(true)),
            item("Fill with Background", "Ctrl+Backspace", s.editable, Message::Fill(false)),
            item("Stroke...", "", s.editable, Message::ShowDialog(DialogKind::Stroke)),
            item("Clear", "Delete", s.editable, Message::ClearSelected),
        ]
    } else {
        vec![
            item("Select All", "Ctrl+A", true, Message::SelectAll),
            item("Reselect", "Shift+Ctrl+D", s.reselect, Message::Reselect),
            Separator,
            item("New Layer", "Shift+Ctrl+N", true, Message::AddLayer),
            item("Duplicate Layer", "Ctrl+J", true, Message::DuplicateLayer),
            item("Merge Down", "Ctrl+E", s.can_merge_down, Message::MergeDown),
            Separator,
            item("Free Transform", "Ctrl+T", s.editable, Message::FreeTransform),
            MenuEntry::Submenu("Transform", transform_entries(&s)),
            Separator,
            item("Fill with Foreground", "Alt+Backspace", s.editable, Message::Fill(true)),
            item("Fill with Background", "Ctrl+Backspace", s.editable, Message::Fill(false)),
        ]
    }
}

fn menu_bar(app: &Photo, i: usize) -> Vec<MenuEntry<'_, Message>> {
    use MenuEntry::{Separator, Submenu};
    let s = state(app);
    let (doc, sel, ed) = (s.doc, s.sel, s.doc && s.editable);
    match MENUS.get(i).copied().unwrap_or("") {
        "File" => vec![
            item("New...", "Ctrl+N", true, Message::NewDocDialog),
            item("Open...", "Ctrl+O", true, Message::OpenDialog),
            item("Place...", "", doc, Message::Place),
            Separator,
            item("Close", "Ctrl+W", doc, Message::RequestCloseDoc(app.active)),
            item("Save", "Ctrl+S", doc, Message::Save),
            item("Save As...", "Shift+Ctrl+S", doc, Message::SaveAs),
            item("Export As...", "Alt+Shift+Ctrl+W", doc, Message::Export),
            Separator,
            item("Quit", "Ctrl+Q", true, Message::RequestQuit),
        ],
        "Edit" => vec![
            item("Undo", "Ctrl+Z", s.undo, Message::Undo),
            item("Redo", "Shift+Ctrl+Z", s.redo, Message::Redo),
            Separator,
            item("Cut", "Ctrl+X", ed, Message::Cut),
            item("Copy", "Ctrl+C", doc, Message::Copy),
            item("Copy Merged", "Shift+Ctrl+C", doc, Message::CopyMerged),
            item("Paste", "Ctrl+V", doc && s.clip, Message::Paste),
            item("Clear", "Delete", sel && s.editable, Message::ClearSelected),
            Separator,
            item("Fill with Foreground", "Alt+Backspace", ed, Message::Fill(true)),
            item("Fill with Background", "Ctrl+Backspace", ed, Message::Fill(false)),
            item("Stroke...", "", sel && s.editable, Message::ShowDialog(DialogKind::Stroke)),
            Separator,
            item("Free Transform", "Ctrl+T", ed, Message::FreeTransform),
            Submenu("Transform", transform_entries(&s)),
            Separator,
            item("Preferences...", "Ctrl+K", true, Message::OpenPreferences),
        ],
        "Image" => {
            let space = app.document().map_or(BlendSpace::Perceptual, |d| d.blend_space);
            vec![
                Submenu("Adjustments", adjustments(&s)),
                Separator,
                item("Auto Tone", "Shift+Ctrl+L", ed, Message::AutoTone),
                item("Auto Contrast", "Alt+Shift+Ctrl+L", ed, Message::AutoContrast),
                Separator,
                item(
                    if app.removing_background { "Remove Background (working…)" } else { "Remove Background" },
                    "",
                    doc && !app.removing_background,
                    Message::RemoveBackground,
                ),
                Separator,
                item("Image Size...", "Alt+Ctrl+I", doc, Message::ShowDialog(DialogKind::ImageSize)),
                item("Canvas Size...", "Alt+Ctrl+C", doc, Message::ShowDialog(DialogKind::CanvasSize)),
                Submenu(
                    "Image Rotation",
                    vec![
                        item("180°", "", doc, Message::RotateCanvas(2)),
                        item("90° Clockwise", "", doc, Message::RotateCanvas(1)),
                        item("90° Counter Clockwise", "", doc, Message::RotateCanvas(3)),
                        Separator,
                        item("Flip Canvas Horizontal", "", doc, Message::FlipCanvas(true)),
                        item("Flip Canvas Vertical", "", doc, Message::FlipCanvas(false)),
                    ],
                ),
                item("Crop", "", sel, Message::CropToSelection),
                item("Trim", "", doc, Message::Trim),
                Separator,
                item("Duplicate", "", doc, Message::DuplicateImage),
                Separator,
                Submenu(
                    "Blending",
                    vec![
                        item("Perceptual (like Photoshop)", check(space == BlendSpace::Perceptual), doc, Message::SetBlendSpace(BlendSpace::Perceptual)),
                        item("Linear Light", check(space == BlendSpace::Linear), doc, Message::SetBlendSpace(BlendSpace::Linear)),
                    ],
                ),
            ]
        }
        "Layer" => vec![
            item("New Layer", "Shift+Ctrl+N", doc, Message::AddLayer),
            item("Duplicate Layer", "Ctrl+J", doc, Message::DuplicateLayer),
            item("Delete Layer", "", doc, Message::DeleteLayer),
            Separator,
            item("Layer via Copy", "Ctrl+J", sel, Message::LayerVia { cut: false }),
            item("Layer via Cut", "Shift+Ctrl+J", sel && s.editable, Message::LayerVia { cut: true }),
            Separator,
            Submenu(
                "Arrange",
                vec![
                    item("Bring to Front", "Shift+Ctrl+]", doc, Message::ArrangeEnd(true)),
                    item("Bring Forward", "Ctrl+]", doc, Message::MoveLayer(1)),
                    item("Send Backward", "Ctrl+[", doc, Message::MoveLayer(-1)),
                    item("Send to Back", "Shift+Ctrl+[", doc, Message::ArrangeEnd(false)),
                ],
            ),
            item(if s.editable || !doc { "Lock Layer" } else { "Unlock Layer" }, "", doc, Message::ToggleLock),
            Separator,
            Submenu(
                "Layer Style",
                vec![
                    item("Blending Options...", "", s.editable, Message::ShowDialog(DialogKind::BlendingOptions)),
                    item("Clear Layer Style", "", s.editable, Message::ClearLayerStyle),
                    item("Rasterize Layer Style", "", s.editable, Message::RasterizeLayerStyle),
                ],
            ),
            Submenu("Layer Mask", mask_menu(app, doc, sel)),
            Submenu("New Adjustment Layer", new_adjustment(app)),
            Separator,
            item("Merge Down", "Ctrl+E", s.can_merge_down, Message::MergeDown),
            item("Merge Visible", "Shift+Ctrl+E", doc, Message::MergeVisible),
            item("Flatten Image", "", doc, Message::Flatten),
        ],
        "Select" => vec![
            item("All", "Ctrl+A", doc, Message::SelectAll),
            item("Deselect", "Ctrl+D", sel, Message::Deselect),
            item("Reselect", "Shift+Ctrl+D", s.reselect, Message::Reselect),
            item("Inverse", "Shift+Ctrl+I", doc, Message::InvertSelection),
            Separator,
            Submenu(
                "Modify",
                vec![
                    item("Expand...", "", sel, Message::ShowDialog(DialogKind::Expand)),
                    item("Contract...", "", sel, Message::ShowDialog(DialogKind::Contract)),
                    item("Feather...", "Shift+F6", sel, Message::ShowDialog(DialogKind::Feather)),
                ],
            ),
        ],
        "Filter" => {
            let f = |label, kind| item(label, "", ed, Message::OpenOp(kind));
            let last = app.last_filter.as_ref().map_or("Last Filter", |op| op.name());
            vec![
                item(last, "Alt+Ctrl+F", ed && app.last_filter.is_some(), Message::LastFilter),
                Separator,
                Submenu("Blur", vec![f("Gaussian Blur...", OpKind::GaussianBlur), f("Box Blur...", OpKind::BoxBlur)]),
                Submenu("Sharpen", vec![f("Sharpen", OpKind::Sharpen), f("Unsharp Mask...", OpKind::UnsharpMask)]),
                Submenu("Noise", vec![f("Add Noise...", OpKind::AddNoise)]),
                Submenu("Pixelate", vec![f("Mosaic...", OpKind::Mosaic)]),
                Submenu("Other", vec![f("High Pass...", OpKind::HighPass)]),
            ]
        }
        "View" => vec![
            item("Zoom In", "Ctrl++", doc, Message::ZoomIn),
            item("Zoom Out", "Ctrl+-", doc, Message::ZoomOut),
            item("Fit on Screen", "Ctrl+0", doc, Message::ZoomTo(Zoom::Fit)),
            item("100%", "Ctrl+1", doc, Message::ZoomTo(Zoom::Fixed(1.0))),
            Separator,
            item("Snap", check(app.snap.on), true, Message::ToggleSnap),
            MenuEntry::Submenu(
                "Snap To",
                vec![
                    item("Layers", check(app.snap.layers), true, Message::SnapTo(SnapTo::Layers)),
                    item("Document Bounds", check(app.snap.bounds), true, Message::SnapTo(SnapTo::Bounds)),
                    Separator,
                    item("All", "", true, Message::SnapTo(SnapTo::All)),
                    item("None", "", true, Message::SnapTo(SnapTo::None)),
                ],
            ),
            Separator,
            item("Full Screen", "F11", true, Message::ToggleFullscreen),
        ],
        "Help" => vec![item("About Gallerion", "", true, Message::About)],
        "Window" => vec![
            item("Color", check(app.panels.color), true, Message::TogglePanel(Panel::Color)),
            item("History", check(app.open_strip == Some(0)), true, Message::ToggleStrip(0)),
            item("Layers", check(app.panels.layers), true, Message::TogglePanel(Panel::Layers)),
            item("Properties", check(app.panels.properties), true, Message::TogglePanel(Panel::Properties)),
        ],
        _ => vec![item("Nothing here yet", "", false, Message::Noop)],
    }
}

/// The Type tool's font list: every name drawn in its own font, with a filter on top.
fn font_list(app: &Photo, at: iced::Point) -> Element<'_, Message> {
    use iced::widget::{button, container, scrollable, text, text_input};
    use jd_ui::tokens::*;
    let filter = app.font_filter.to_lowercase();
    let mut list = column![].spacing(1);
    let mut shown = 0;
    for &name in app.fonts.iter().filter(|f| filter.is_empty() || f.to_lowercase().contains(&filter)) {
        let current = name == app.type_opts.family;
        list = list.push(
            button(text(name).size(16).font(iced::Font::with_name(name)))
                .width(Fill)
                .padding(iced::Padding::from([4, 10]))
                .style(jd_ui::style::row(current))
                .on_press(Message::Menu(Box::new(Message::TypeFamily(name.to_owned())))),
        );
        shown += 1;
    }
    if shown == 0 {
        list = list.push(container(text("No matching fonts").size(TEXT_SIZE).color(TEXT_MUTED)).padding(10));
    }
    let card = container(
        column![
            text_input("Filter fonts", &app.font_filter)
                .id(crate::view::FONT_FILTER)
                .size(TEXT_SIZE)
                .padding(iced::Padding::from([6, 8]))
                .on_input(Message::FontFilter),
            scrollable(list).height(380),
        ]
        .spacing(6)
        .width(300),
    )
    .padding(6)
    .style(|_| container::Style {
        background: Some(BG_PANEL_HEADER.into()),
        border: iced::Border { color: BORDER_SUBTLE, width: 1.0, radius: 6.0.into() },
        shadow: iced::Shadow { color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.5), offset: iced::Vector::new(0.0, 6.0), blur_radius: 18.0 },
        ..Default::default()
    });
    let x = at.x.min(app.window_size.width - 316.0).max(0.0);
    stack![
        mouse_area(space().width(Fill).height(Fill)).on_press(Message::ClosePopup).on_right_press(Message::ClosePopup),
        pin(card).x(x).y(at.y),
    ]
    .into()
}

/// Right-click on a layer row.
/// Layer > Layer Mask.
fn mask_menu<'a>(app: &Photo, doc: bool, sel: bool) -> Vec<MenuEntry<'a, Message>> {
    let mask = app.document().and_then(|d| d.active()).and_then(|l| l.mask.as_ref());
    let (add, has) = (doc && mask.is_none(), mask.is_some());
    vec![
        item("Reveal All", "", add, Message::LayerMask(MaskOp::RevealAll)),
        item("Hide All", "", add, Message::LayerMask(MaskOp::HideAll)),
        item("Reveal Selection", "", add && sel, Message::LayerMask(MaskOp::RevealSelection)),
        item("Hide Selection", "", add && sel, Message::LayerMask(MaskOp::HideSelection)),
        MenuEntry::Separator,
        item("Delete", "", has, Message::LayerMask(MaskOp::Delete)),
        item("Apply", "", has, Message::LayerMask(MaskOp::Apply)),
        item(if mask.is_some_and(|m| !m.enabled) { "Enable" } else { "Disable" }, "", has, Message::LayerMask(MaskOp::Toggle)),
    ]
}

fn layer_menu(app: &Photo) -> Vec<MenuEntry<'_, Message>> {
    use MenuEntry::{Separator, Submenu};
    let s = state(app);
    let layer = app.document().and_then(|d| d.active());
    let (visible, locked) = layer.map_or((true, false), |l| (l.visible, l.locked));
    let has_style = layer.is_some_and(|l| !l.style.is_empty());
    let id = layer.map(|l| l.id);
    // Other open documents, for Duplicate Into.
    let others: Vec<MenuEntry<'_, Message>> = app
        .docs
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != app.active)
        .map(|(i, d)| item(d.doc.name.as_str(), "", true, Message::DuplicateInto(i)))
        .collect();
    let mut entries = vec![
        item("Blending Options...", "", s.doc && !locked, Message::ShowDialog(DialogKind::BlendingOptions)),
    ];
    if let Some(l) = layer.filter(|l| l.adjustment.is_some()) {
        entries.push(item("Edit Adjustment...", "", app.adjusting.is_none(), Message::EditAdjustment(l.id)));
    }
    entries.extend(vec![
        item("Clear Layer Style", "", has_style && !locked, Message::ClearLayerStyle),
        item("Rasterize Layer Style", "", has_style && !locked, Message::RasterizeLayerStyle),
        Separator,
        item("Select Pixels", "", s.doc, Message::SelectPixels),
        Separator,
    ]);
    let mask = layer.and_then(|l| l.mask.as_ref());
    entries.extend(match mask {
        None => vec![item("Add Layer Mask", "", s.doc, Message::AddMask)],
        Some(m) => vec![
            item(if m.enabled { "Disable Layer Mask" } else { "Enable Layer Mask" }, "", true, Message::LayerMask(MaskOp::Toggle)),
            item("Delete Layer Mask", "", true, Message::LayerMask(MaskOp::Delete)),
            item("Apply Layer Mask", "", true, Message::LayerMask(MaskOp::Apply)),
        ],
    });
    entries.extend([
        Separator,
        item("Duplicate Layer", "Ctrl+J", s.doc, Message::DuplicateLayer),
    ]);
    entries.push(if others.is_empty() {
        item("Duplicate Into...", "", false, Message::Noop)
    } else {
        Submenu("Duplicate Into", others)
    });
    entries.extend([
        item("Delete Layer", "", s.doc && !locked, Message::DeleteLayer),
        Separator,
        item("Rename Layer", "", id.is_some(), id.map_or(Message::Noop, Message::StartRename)),
        item(if visible { "Hide Layer" } else { "Show Layer" }, "", id.is_some(), id.map_or(Message::Noop, Message::ToggleVisible)),
        item(if locked { "Unlock Layer" } else { "Lock Layer" }, "", id.is_some(), Message::ToggleLock),
        Separator,
        item("Merge Down", "Ctrl+E", s.can_merge_down, Message::MergeDown),
        item("Merge Visible", "Shift+Ctrl+E", s.doc, Message::MergeVisible),
        item("Flatten Image", "", s.doc, Message::Flatten),
    ]);
    entries
}

/// New adjustment layer kinds (Layer > New Adjustment Layer, the Layers panel button).
fn new_adjustment<'a>(app: &Photo) -> Vec<MenuEntry<'a, Message>> {
    let ok = app.document().is_some() && app.adjusting.is_none();
    crate::adjust::OpKind::ADJUSTMENTS
        .iter()
        .map(|&k| {
            let name = match crate::adjust::Op::default_for(k) {
                crate::adjust::Op::Adjust(a) => a.name(),
                crate::adjust::Op::Filter(f) => f.name(),
            };
            item(if k.is_instant() { name } else { DIALOG_NAMES.iter().find(|(n, _)| *n == name).map_or(name, |(_, d)| d) }, "", ok, Message::NewAdjustmentLayer(k))
        })
        .collect()
}

/// Adjustments that open a dialog get "..." in menus.
const DIALOG_NAMES: [(&str, &str); 6] = [
    ("Brightness/Contrast", "Brightness/Contrast..."),
    ("Levels", "Levels..."),
    ("Curves", "Curves..."),
    ("Hue/Saturation", "Hue/Saturation..."),
    ("Threshold", "Threshold..."),
    ("Posterize", "Posterize..."),
];

/// A dock panel's ≡ menu.
fn panel_menu(app: &Photo, panel: Panel) -> Vec<MenuEntry<'_, Message>> {
    let shown = app.shown_panels();
    let i = shown.iter().position(|p| *p == panel).unwrap_or(0);
    vec![
        item("Move Up", "", i > 0, Message::PanelMoveBy(panel, -1)),
        item("Move Down", "", i + 1 < shown.len(), Message::PanelMoveBy(panel, 1)),
        MenuEntry::Separator,
        item("Reset Panel Layout", "", true, Message::ResetPanels),
        item("Close", "", true, Message::TogglePanel(panel)),
    ]
}

/// The options bar's tool preset picker.
fn tool_presets(app: &Photo) -> Vec<MenuEntry<'_, Message>> {
    use crate::app::{BUILTIN_BRUSHES, ToolPreset};
    let mut entries = Vec::new();
    if app.tool.uses_brush() {
        let b = &app.brush;
        for (name, size, hardness) in BUILTIN_BRUSHES {
            let p = ToolPreset { name: name.to_owned(), size, hardness, opacity: 1.0, flow: 1.0 };
            let on = (b.size, b.hardness, b.opacity, b.flow) == (size, hardness, 1.0, 1.0);
            entries.push(item(name, check(on), true, Message::ApplyBrushPreset(p)));
        }
        if !app.presets.is_empty() {
            entries.push(MenuEntry::Separator);
            for p in &app.presets {
                let on = (b.size, b.hardness, b.opacity, b.flow) == (p.size, p.hardness, p.opacity, p.flow);
                entries.push(item(p.name.as_str(), check(on), true, Message::ApplyBrushPreset(p.clone())));
            }
        }
        entries.push(MenuEntry::Separator);
        entries.push(item("New Tool Preset (current settings)", "", true, Message::SaveBrushPreset));
        entries.push(item("Delete Saved Presets", "", !app.presets.is_empty(), Message::DeleteBrushPresets));
        entries.push(MenuEntry::Separator);
    }
    entries.push(item("Reset Tool", "", true, Message::ResetTool));
    entries.push(item("Reset All Tools", "", true, Message::ResetAllTools));
    entries
}

fn workspaces(app: &Photo) -> Vec<MenuEntry<'_, Message>> {
    crate::app::Workspace::ALL
        .iter()
        .map(|&w| item(w.name(), check(app.workspace == w), true, Message::SetWorkspace(w)))
        .collect()
}

/// Every enabled menu command as (path, shortcut, message), for the command search.
fn commands(app: &Photo) -> Vec<(String, String, Message)> {
    fn walk(prefix: &str, entries: Vec<MenuEntry<'_, Message>>, out: &mut Vec<(String, String, Message)>) {
        for e in entries {
            match e {
                MenuEntry::Item(label, keys, Some(Message::Menu(m))) | MenuEntry::IconItem(_, label, keys, Some(Message::Menu(m))) => {
                    out.push((format!("{prefix} > {}", label.trim_end_matches('.')), keys.to_owned(), *m));
                }
                MenuEntry::Submenu(label, children) => walk(&format!("{prefix} > {label}"), children, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for (i, title) in MENUS.iter().enumerate() {
        walk(title, menu_bar(app, i), &mut out);
    }
    out
}

/// Command search: type to filter every menu command, Enter (or click) runs the top match.
pub fn palette(app: &Photo) -> Option<Element<'_, Message>> {
    use iced::widget::{button, container, row, text, text_input};
    use jd_ui::tokens::*;
    let query = app.palette.as_ref()?;
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_owned).collect();
    let matches: Vec<(String, String, Message)> = commands(app)
        .into_iter()
        .filter(|(path, ..)| {
            let p = path.to_lowercase();
            words.iter().all(|w| p.contains(w.as_str()))
        })
        .take(12)
        .collect();
    let first = matches.first().map(|(_, _, m)| Message::Menu(Box::new(m.clone())));
    let input = text_input("Search commands, e.g. \"gaussian\" or \"canvas size\"", query)
        .id(crate::view::PALETTE_INPUT)
        .size(TEXT_SIZE + 1.0)
        .padding(iced::Padding::from([8, 10]))
        .on_input(Message::PaletteInput)
        .on_submit_maybe(first);
    let mut list = column![].spacing(1);
    if matches.is_empty() {
        list = list.push(container(text("No matching commands").size(TEXT_SIZE).color(TEXT_MUTED)).padding(10));
    }
    for (i, (path, keys, m)) in matches.into_iter().enumerate() {
        list = list.push(
            button(row![text(path).size(TEXT_SIZE).width(Fill), text(keys).size(TEXT_SIZE_SMALL).color(TEXT_MUTED)].spacing(10))
                .width(Fill)
                .padding(iced::Padding::from([6, 10]))
                .style(jd_ui::style::row(i == 0))
                .on_press(Message::Menu(Box::new(m))),
        );
    }
    let card = container(column![input, list].spacing(6).width(520))
        .padding(8)
        .style(|_| container::Style {
            background: Some(BG_PANEL_HEADER.into()),
            border: iced::Border { color: BORDER_SUBTLE, width: 1.0, radius: 8.0.into() },
            shadow: iced::Shadow { color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.5), offset: iced::Vector::new(0.0, 8.0), blur_radius: 24.0 },
            ..Default::default()
        });
    let x = ((app.window_size.width - 536.0) / 2.0).max(0.0);
    Some(
        stack![
            mouse_area(space().width(Fill).height(Fill)).on_press(Message::ClosePalette).on_right_press(Message::ClosePalette),
            pin(card).x(x).y(60.0),
        ]
        .into(),
    )
}

/// A toolbar slot's alternates, Photoshop flyout style.
fn tool_slot<'a>(slot: usize) -> Vec<MenuEntry<'a, Message>> {
    tool::slots()
        .nth(slot)
        .unwrap_or(&[])
        .iter()
        .map(|&t: &Tool| MenuEntry::IconItem(t.icon(), t.name(), key_label(t), when(true, Message::SelectTool(t))))
        .collect()
}

fn key_label(t: Tool) -> &'static str {
    const KEYS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let i = (t.shortcut().to_ascii_uppercase() as u8).wrapping_sub(b'A') as usize;
    KEYS.get(i..i + 1).unwrap_or("")
}
