//! Title-bar dropdowns. Items that aren't implemented yet are listed (disabled) with Figma's
//! shortcuts, so the menus double as a map of what's coming.

use iced::Element;
use jd_ui::chrome;
use jd_ui::widget::MenuEntry;
use jd_vector::ExportFormat;
use jd_vector::arrange::{Align, Order};
use jd_vector::boolean::BoolOp;

use crate::app::{Blobr, Message};

pub const MENUS: &[&str] = &["File", "Edit", "View", "Object", "Text", "Arrange", "Plugins", "Window", "Help"];
pub const MENU_COUNT: usize = MENUS.len();

const WIDTH: f32 = 260.0;

pub fn popup(app: &Blobr) -> Option<Element<'_, Message>> {
    let (i, at) = app.chrome.menu_anchor()?;
    Some(chrome::popup(entries(app, i), WIDTH, at, app.chrome.size, true, Message::CloseMenu, None, |_| Message::Noop))
}

fn item<'a>(label: &'a str, shortcut: &'a str, m: Option<Message>) -> MenuEntry<'a, Message> {
    MenuEntry::Item(label, shortcut, m.map(|m| Message::Menu(Box::new(m))))
}

/// Not implemented yet: shown disabled.
fn todo<'a>(label: &'a str, shortcut: &'a str) -> MenuEntry<'a, Message> {
    item(label, shortcut, None)
}

fn entries(app: &Blobr, menu: usize) -> Vec<MenuEntry<'_, Message>> {
    use MenuEntry::Separator;
    let sel = !app.selection.is_empty();
    let on = |cond: bool, m: Message| cond.then_some(m);
    match MENUS[menu] {
        "File" => vec![
            item("New", "Ctrl+N", Some(Message::New)),
            item("Open...", "Ctrl+O", Some(Message::Open)),
            Separator,
            item("Save", "Ctrl+S", Some(Message::Save)),
            item("Save As...", "Ctrl+Shift+S", Some(Message::SaveAs)),
            Separator,
            item("Export as PNG...", "Ctrl+Shift+E", Some(Message::Export(ExportFormat::Png))),
            item("Export as SVG...", "", Some(Message::Export(ExportFormat::Svg))),
            Separator,
            item("Quit", "Ctrl+Q", Some(Message::Quit)),
        ],
        "Edit" => vec![
            item("Undo", "Ctrl+Z", on(app.ed.history.can_undo(), Message::Undo)),
            item("Redo", "Ctrl+Shift+Z", on(app.ed.history.can_redo(), Message::Redo)),
            Separator,
            item("Cut", "Ctrl+X", on(sel, Message::Cut)),
            item("Copy", "Ctrl+C", on(sel, Message::Copy)),
            item("Paste", "Ctrl+V", Some(Message::Paste)),
            item("Copy as PNG", "Ctrl+Shift+C", on(sel, Message::CopyAsPng)),
            item("Duplicate", "Ctrl+D", on(sel, Message::Duplicate)),
            item("Delete", "Delete", on(sel, Message::Delete)),
            Separator,
            item("Select All", "Ctrl+A", Some(Message::SelectAll)),
            item("Select None", "Ctrl+Shift+A", on(sel, Message::Deselect)),
        ],
        "View" => vec![
            item("Zoom In", "Ctrl++", Some(Message::ZoomIn)),
            item("Zoom Out", "Ctrl+-", Some(Message::ZoomOut)),
            item("Zoom to 100%", "Shift+0", Some(Message::Zoom100)),
            item("Zoom to Fit", "Shift+1", Some(Message::ZoomFit)),
            item("Zoom to Selection", "Shift+2", on(sel, Message::ZoomSelection)),
            Separator,
            todo("Rulers", "Shift+R"),
            todo("Pixel Grid", "Shift+'"),
            Separator,
            item("Full Screen", "F11", Some(Message::Chrome(chrome::Event::ToggleFullscreen))),
        ],
        "Object" => vec![
            item("Group Selection", "Ctrl+G", on(sel, Message::Group)),
            item("Frame Selection", "Ctrl+Alt+G", on(sel, Message::FrameSelection)),
            item("Ungroup Selection", "Ctrl+Shift+G", on(sel, Message::Ungroup)),
            item("Rename", "Ctrl+R", app.selection.first().map(|&id| Message::StartRename(id))),
            item("Add Auto Layout", "Shift+A", on(sel, Message::ToggleAutoLayout)),
            Separator,
            item("Create Component", "Ctrl+Alt+K", on(sel, Message::CreateComponent)),
            item("Combine as Variants", "", on(app.selection.len() >= 2, Message::CombineVariants)),
            item("Detach Instance", "Ctrl+Alt+B", on(sel, Message::Detach)),
            item("Reset All Overrides", "", on(sel, Message::ResetOverrides)),
            Separator,
            MenuEntry::Submenu(
                "Boolean",
                vec![
                    item("Union", "", on(app.selection.len() >= 2, Message::Boolean(BoolOp::Union))),
                    item("Subtract", "", on(app.selection.len() >= 2, Message::Boolean(BoolOp::Subtract))),
                    item("Intersect", "", on(app.selection.len() >= 2, Message::Boolean(BoolOp::Intersect))),
                    item("Exclude", "", on(app.selection.len() >= 2, Message::Boolean(BoolOp::Exclude))),
                ],
            ),
            item("Flatten Selection", "Ctrl+E", on(sel, Message::Flatten)),
            todo("Outline Stroke", "Ctrl+Shift+O"),
        ],
        "Text" => vec![todo("Bold", "Ctrl+B"), todo("Italic", "Ctrl+I"), todo("Underline", "Ctrl+U"), Separator, todo("Text Styles...", "")],
        "Arrange" => vec![
            item("Bring to Front", "Ctrl+]", on(sel, Message::Reorder(Order::Front))),
            item("Bring Forward", "Ctrl+Shift+]", on(sel, Message::Reorder(Order::Forward))),
            item("Send Backward", "Ctrl+Shift+[", on(sel, Message::Reorder(Order::Backward))),
            item("Send to Back", "Ctrl+[", on(sel, Message::Reorder(Order::Back))),
            Separator,
            item("Align Left", "Alt+A", on(sel, Message::Align(Align::Left))),
            item("Align Horizontal Centers", "Alt+H", on(sel, Message::Align(Align::Center))),
            item("Align Right", "Alt+D", on(sel, Message::Align(Align::Right))),
            item("Align Top", "Alt+W", on(sel, Message::Align(Align::Top))),
            item("Align Vertical Centers", "Alt+V", on(sel, Message::Align(Align::Middle))),
            item("Align Bottom", "Alt+S", on(sel, Message::Align(Align::Bottom))),
            Separator,
            item("Distribute Horizontal Spacing", "Alt+Shift+H", on(app.selection.len() >= 3, Message::Distribute(true))),
            item("Distribute Vertical Spacing", "Alt+Shift+V", on(app.selection.len() >= 3, Message::Distribute(false))),
        ],
        "Plugins" => vec![todo("Manage Plugins...", "")],
        "Window" => vec![
            item("Layers", "Alt+L", Some(Message::LeftTab(0))),
            item("Assets", "Alt+I", Some(Message::LeftTab(1))),
            item("Local Variables...", "", Some(Message::ShowVariables(true))),
        ],
        "Help" => vec![todo("Keyboard Shortcuts", "Ctrl+Shift+?"), todo("About Blobr", "")],
        _ => Vec::new(),
    }
}
