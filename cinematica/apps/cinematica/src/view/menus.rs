//! Title-bar dropdowns. Items that aren't implemented yet are listed (disabled) with
//! Premiere's shortcuts, so the menus double as a map of what's coming.

use iced::Element;
use jd_ui::chrome;
use jd_ui::widget::MenuEntry;

use crate::app::{Editor, Message};
use crate::source::GoTo;

pub const MENUS: &[&str] = &["File", "Edit", "Clip", "Sequence", "Markers", "Graphics", "View", "Window", "Help"];
pub const MENU_COUNT: usize = MENUS.len();

const WIDTH: f32 = 250.0;

pub fn popup(app: &Editor) -> Option<Element<'_, Message>> {
    let (i, at) = app.chrome.menu_anchor()?;
    Some(chrome::popup(entries(app, i), WIDTH, at, app.chrome.size, true, Message::CloseMenu))
}

fn item<'a>(label: &'a str, shortcut: &'a str, m: Option<Message>) -> MenuEntry<'a, Message> {
    MenuEntry::Item(label, shortcut, m.map(|m| Message::Menu(Box::new(m))))
}

/// Not implemented yet: shown disabled.
fn todo<'a>(label: &'a str, shortcut: &'a str) -> MenuEntry<'a, Message> {
    item(label, shortcut, None)
}

fn entries(app: &Editor, menu: usize) -> Vec<MenuEntry<'_, Message>> {
    use MenuEntry::Separator;
    // Marks apply to the Source monitor until the Program monitor plays sequences (M3).
    let source = app.source.loaded();
    match MENUS[menu] {
        "File" => vec![
            todo("New Project...", "Ctrl+Alt+N"),
            todo("Open Project...", "Ctrl+O"),
            todo("Save", "Ctrl+S"),
            todo("Save As...", "Ctrl+Shift+S"),
            Separator,
            item("Import...", "Ctrl+I", Some(Message::ImportDialog)),
            todo("Export Media...", "Ctrl+M"),
            Separator,
            item("Quit", "Ctrl+Q", Some(Message::Quit)),
        ],
        "Edit" => vec![
            todo("Undo", "Ctrl+Z"),
            todo("Redo", "Ctrl+Shift+Z"),
            Separator,
            todo("Cut", "Ctrl+X"),
            todo("Copy", "Ctrl+C"),
            todo("Paste", "Ctrl+V"),
            todo("Clear", "Delete"),
            todo("Ripple Delete", "Shift+Delete"),
            Separator,
            todo("Select All", "Ctrl+A"),
            todo("Deselect All", "Ctrl+Shift+A"),
        ],
        "Clip" => vec![
            todo("Speed/Duration...", "Ctrl+R"),
            todo("Enable", "Shift+E"),
            todo("Link", "Ctrl+L"),
            todo("Group", "Ctrl+G"),
            todo("Nest...", ""),
        ],
        "Sequence" => vec![
            todo("Sequence Settings...", ""),
            Separator,
            todo("Add Edit", "Ctrl+K"),
            todo("Lift", ";"),
            todo("Extract", "'"),
            Separator,
            item(if app.snap { "Snap in Timeline: On" } else { "Snap in Timeline: Off" }, "S", Some(Message::ToggleSnap)),
            item(
                if app.linked { "Linked Selection: On" } else { "Linked Selection: Off" },
                "",
                Some(Message::ToggleLinked),
            ),
        ],
        "Markers" => vec![
            item("Mark In", "I", source.then_some(Message::MarkIn)),
            item("Mark Out", "O", source.then_some(Message::MarkOut)),
            item("Go to In", "Shift+I", source.then_some(Message::GoTo(GoTo::In))),
            item("Go to Out", "Shift+O", source.then_some(Message::GoTo(GoTo::Out))),
            item("Clear In and Out", "Ctrl+Shift+X", source.then_some(Message::ClearInOut)),
            Separator,
            todo("Add Marker", "M"),
            todo("Go to Next Marker", "Shift+M"),
        ],
        "Graphics" => vec![todo("New Layer: Text", "Ctrl+T")],
        "View" => vec![
            todo("Zoom In", "="),
            todo("Zoom Out", "-"),
            Separator,
            item(
                if app.chrome.fullscreen { "Exit Fullscreen" } else { "Fullscreen" },
                "F11",
                Some(Message::Chrome(chrome::Event::ToggleFullscreen)),
            ),
        ],
        "Window" => vec![item("Workspace: Editing", "", Some(Message::Noop))],
        _ => vec![todo("Keyboard Shortcuts...", ""), todo("About Cinematica", "")],
    }
}
