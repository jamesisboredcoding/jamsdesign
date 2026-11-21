//! Popup menus: title-bar dropdowns, the canvas right-click menu and toolbar flyouts.

use iced::widget::{column, mouse_area, pin, space, stack};
use iced::{Element, Fill};
use jd_raster::BlendSpace;
use jd_ui::widget::{MenuEntry, menu};

use super::chrome::MENUS;
use crate::adjust::OpKind;
use crate::app::{Message, Photo, PopupKind, Zoom};
use crate::tool::{self, Tool};

const WIDTH: f32 = 250.0;

/// The open popup, if any, over a click-catcher that closes it.
pub fn popup(app: &Photo) -> Option<Element<'_, Message>> {
    let p = app.popup?;
    let (entries, width) = match p.kind {
        PopupKind::Context => (context(app), WIDTH),
        PopupKind::MenuBar(i) => (menu_bar(app, i), WIDTH),
        PopupKind::ToolSlot(slot) => (tool_slot(app, slot), 220.0),
    };
    let items = entries.iter().filter(|e| !matches!(e, MenuEntry::Separator)).count() as f32;
    let seps = entries.len() as f32 - items;
    let height = items * 27.0 + seps * 9.0 + 8.0;
    // Keep the menu inside the window.
    let x = p.at.x.min(app.window_size.width - width - 4.0).max(0.0);
    let y = if p.at.y + height > app.window_size.height { (p.at.y - height).max(0.0) } else { p.at.y };

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
    Some(stack![catcher, pin(menu(entries, width)).x(x).y(y)].into())
}

fn when(enabled: bool, m: Message) -> Option<Message> {
    enabled.then(|| Message::Menu(Box::new(m)))
}

fn item<'a>(label: &'a str, shortcut: &'a str, enabled: bool, m: Message) -> MenuEntry<'a, Message> {
    MenuEntry::Item(label, shortcut, when(enabled, m))
}

/// Right-click on the canvas, like Photoshop's: what you can do with the selection, or the layer.
fn context(app: &Photo) -> Vec<MenuEntry<'_, Message>> {
    use MenuEntry::Separator;
    let Some(doc) = app.document() else { return vec![item("No document open", "", false, Message::Noop)] };
    if app.transform.is_some() {
        return vec![
            item("Commit Transform", "Enter", true, Message::CommitTransform),
            item("Cancel Transform", "Esc", true, Message::CancelTransform),
        ];
    }
    let editable = doc.active().is_some_and(|l| !l.locked);
    if doc.selection.is_some() {
        vec![
            item("Deselect", "Ctrl+D", true, Message::Deselect),
            item("Select Inverse", "Shift+Ctrl+I", true, Message::InvertSelection),
            Separator,
            item("Layer via Copy", "Ctrl+J", true, Message::LayerVia { cut: false }),
            item("Layer via Cut", "Shift+Ctrl+J", editable, Message::LayerVia { cut: true }),
            item("New Layer", "Shift+Ctrl+N", true, Message::AddLayer),
            Separator,
            item("Free Transform", "Ctrl+T", editable, Message::FreeTransform),
            Separator,
            item("Fill with Foreground", "Alt+Backspace", editable, Message::Fill(true)),
            item("Fill with Background", "Ctrl+Backspace", editable, Message::Fill(false)),
            item("Clear", "Delete", editable, Message::ClearSelected),
        ]
    } else {
        vec![
            item("Select All", "Ctrl+A", true, Message::SelectAll),
            Separator,
            item("New Layer", "Shift+Ctrl+N", true, Message::AddLayer),
            item("Duplicate Layer", "Ctrl+J", true, Message::DuplicateLayer),
            Separator,
            item("Free Transform", "Ctrl+T", editable, Message::FreeTransform),
            Separator,
            item("Fill with Foreground", "Alt+Backspace", editable, Message::Fill(true)),
            item("Fill with Background", "Ctrl+Backspace", editable, Message::Fill(false)),
        ]
    }
}

fn menu_bar(app: &Photo, i: usize) -> Vec<MenuEntry<'_, Message>> {
    use MenuEntry::Separator;
    let doc = app.document();
    let has_doc = doc.is_some();
    let has_sel = doc.is_some_and(|d| d.selection.is_some());
    let editable = doc.and_then(|d| d.active()).is_some_and(|l| !l.locked);
    let undo = app.doc().is_some_and(|d| d.history.can_undo());
    let redo = app.doc().is_some_and(|d| d.history.can_redo());
    match MENUS.get(i).copied().unwrap_or("") {
        "File" => vec![
            item("New...", "Ctrl+N", true, Message::NewDocDialog),
            item("Open...", "Ctrl+O", true, Message::OpenDialog),
            Separator,
            item("Close", "Ctrl+W", has_doc, Message::RequestCloseDoc(app.active)),
            item("Save", "Ctrl+S", has_doc, Message::Save),
            item("Save As...", "Shift+Ctrl+S", has_doc, Message::SaveAs),
            item("Export...", "Alt+Shift+Ctrl+W", has_doc, Message::Export),
            Separator,
            item("Quit", "Ctrl+Q", true, Message::RequestQuit),
        ],
        "Edit" => vec![
            item("Undo", "Ctrl+Z", undo, Message::Undo),
            item("Redo", "Shift+Ctrl+Z", redo, Message::Redo),
            Separator,
            item("Cut", "Ctrl+X", has_doc && editable, Message::Cut),
            item("Copy", "Ctrl+C", has_doc, Message::Copy),
            item("Paste", "Ctrl+V", has_doc && app.clipboard.is_some(), Message::Paste),
            item("Clear", "Delete", has_sel && editable, Message::ClearSelected),
            Separator,
            item("Fill with Foreground", "Alt+Backspace", has_doc && editable, Message::Fill(true)),
            item("Fill with Background", "Ctrl+Backspace", has_doc && editable, Message::Fill(false)),
            Separator,
            item("Free Transform", "Ctrl+T", has_doc && editable, Message::FreeTransform),
        ],
        "Image" => {
            let space = doc.map_or(BlendSpace::Perceptual, |d| d.blend_space);
            let op = |label, keys, kind| item(label, keys, has_doc && editable, Message::OpenOp(kind));
            vec![
                op("Brightness/Contrast...", "", OpKind::BrightnessContrast),
                op("Levels...", "Ctrl+L", OpKind::Levels),
                op("Curves...", "Ctrl+M", OpKind::Curves),
                op("Hue/Saturation...", "Ctrl+U", OpKind::HueSaturation),
                op("Desaturate", "Shift+Ctrl+U", OpKind::Desaturate),
                Separator,
                op("Invert", "Ctrl+I", OpKind::Invert),
                op("Threshold...", "", OpKind::Threshold),
                op("Posterize...", "", OpKind::Posterize),
                Separator,
                item("Blend Perceptually (like Photoshop)", if space == BlendSpace::Perceptual { "✓" } else { "" }, has_doc, Message::SetBlendSpace(BlendSpace::Perceptual)),
                item("Blend in Linear Light", if space == BlendSpace::Linear { "✓" } else { "" }, has_doc, Message::SetBlendSpace(BlendSpace::Linear)),
            ]
        }
        "Layer" => vec![
            item("New Layer", "Shift+Ctrl+N", has_doc, Message::AddLayer),
            item("Duplicate Layer", "Ctrl+J", has_doc, Message::DuplicateLayer),
            item("Delete Layer", "", has_doc, Message::DeleteLayer),
            Separator,
            item("Layer via Copy", "Ctrl+J", has_sel, Message::LayerVia { cut: false }),
            item("Layer via Cut", "Shift+Ctrl+J", has_sel && editable, Message::LayerVia { cut: true }),
            Separator,
            item("Bring Forward", "Ctrl+]", has_doc, Message::MoveLayer(1)),
            item("Send Backward", "Ctrl+[", has_doc, Message::MoveLayer(-1)),
            Separator,
            item(if editable || !has_doc { "Lock Layer" } else { "Unlock Layer" }, "", has_doc, Message::ToggleLock),
        ],
        "Select" => vec![
            item("All", "Ctrl+A", has_doc, Message::SelectAll),
            item("Deselect", "Ctrl+D", has_sel, Message::Deselect),
            item("Inverse", "Shift+Ctrl+I", has_doc, Message::InvertSelection),
        ],
        "View" => vec![
            item("Zoom In", "Ctrl++", has_doc, Message::ZoomIn),
            item("Zoom Out", "Ctrl+-", has_doc, Message::ZoomOut),
            item("Fit on Screen", "Ctrl+0", has_doc, Message::ZoomTo(Zoom::Fit)),
            item("100%", "Ctrl+1", has_doc, Message::ZoomTo(Zoom::Fixed(1.0))),
            Separator,
            item("Full Screen", "F11", true, Message::ToggleFullscreen),
        ],
        "Filter" => {
            let op = |label, kind| item(label, "", has_doc && editable, Message::OpenOp(kind));
            vec![
                op("Gaussian Blur...", OpKind::GaussianBlur),
                op("Unsharp Mask...", OpKind::UnsharpMask),
                Separator,
                op("Add Noise...", OpKind::AddNoise),
                op("Mosaic...", OpKind::Mosaic),
            ]
        }
        "Window" => vec![item("History", "", true, Message::ToggleStrip(0))],
        _ => vec![item("Nothing here yet", "", false, Message::Noop)],
    }
}

/// A toolbar slot's alternates, Photoshop flyout style.
fn tool_slot(_app: &Photo, slot: usize) -> Vec<MenuEntry<'_, Message>> {
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
