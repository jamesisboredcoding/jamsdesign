//! Popup menus: title-bar dropdowns, the canvas right-click menu and toolbar flyouts.

use iced::widget::{column, mouse_area, pin, space, stack};
use iced::{Element, Fill};
use jd_raster::BlendSpace;
use jd_ui::widget::{MenuEntry, menu, menu_height};

use super::chrome::MENUS;
use crate::adjust::OpKind;
use crate::app::{DialogKind, Message, Orient, Panel, Photo, PopupKind, Zoom};
use crate::tool::{self, Tool};

const WIDTH: f32 = 260.0;

/// The open popup (and its submenu), over a click-catcher that closes it.
pub fn popup(app: &Photo) -> Option<Element<'_, Message>> {
    let p = app.popup?;
    let (entries, width) = match p.kind {
        PopupKind::Context => (context(app), WIDTH),
        PopupKind::MenuBar(i) => (menu_bar(app, i), WIDTH),
        PopupKind::ToolSlot(slot) => (tool_slot(slot), 220.0),
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
        ],
        "Image" => {
            let space = app.document().map_or(BlendSpace::Perceptual, |d| d.blend_space);
            vec![
                Submenu("Adjustments", adjustments(&s)),
                Separator,
                item("Auto Tone", "Shift+Ctrl+L", ed, Message::AutoTone),
                item("Auto Contrast", "Alt+Shift+Ctrl+L", ed, Message::AutoContrast),
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
            item("Full Screen", "F11", true, Message::ToggleFullscreen),
        ],
        "Window" => vec![
            item("Color", check(app.panels.color), true, Message::TogglePanel(Panel::Color)),
            item("History", check(app.open_strip == Some(0)), true, Message::ToggleStrip(0)),
            item("Layers", check(app.panels.layers), true, Message::TogglePanel(Panel::Layers)),
            item("Properties", check(app.panels.properties), true, Message::TogglePanel(Panel::Properties)),
        ],
        _ => vec![item("Nothing here yet", "", false, Message::Noop)],
    }
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
