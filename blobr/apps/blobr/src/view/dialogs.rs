//! Small dialogs: Preferences, Keyboard Shortcuts and About.

use iced::alignment::Vertical;
use iced::widget::{button, checkbox, column, container, mouse_area, row, scrollable, space, text};
use iced::{Border, Color, Element, Fill, Padding};
use jd_ui::widget::{bold, hline};
use jd_ui::{Icon, icon, style, tokens::*};

use crate::app::{Blobr, Message};
use crate::prefs::{AUTOSAVE_STEPS, Prefs};

/// A centred card on a dimmed backdrop that swallows clicks.
fn dialog<'a>(title: &'a str, close: Message, body: Element<'a, Message>, width: f32) -> Element<'a, Message> {
    let header = row![
        text(title).size(TEXT_SIZE).font(bold()).color(TEXT_PRIMARY),
        space::horizontal(),
        button(icon(Icon::Close, 12.0, ICON)).padding(4).style(style::ghost(false)).on_press(close.clone()),
    ]
    .align_y(Vertical::Center);
    let card = container(column![header, hline(BORDER_SUBTLE), body].spacing(12)).padding(20).width(width).style(|_| container::Style {
        background: Some(BG_PANEL.into()),
        border: Border { radius: 6.0.into(), width: 1.0, color: BORDER_DEFAULT },
        ..Default::default()
    });
    iced::widget::opaque(
        mouse_area(container(card).center(Fill).padding(40).style(|_| container::Style {
            background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.45).into()),
            ..Default::default()
        }))
        .on_press(Message::Noop),
    )
}

fn section(title: &str) -> Element<'_, Message> {
    text(title).size(TEXT_SIZE_SMALL).font(bold()).color(TEXT_SECONDARY).into()
}

fn toggle<'a>(label: &'a str, on: bool, prefs: &Prefs, set: impl Fn(&mut Prefs, bool) + 'a) -> Element<'a, Message> {
    let prefs = prefs.clone();
    row![
        checkbox(on)
            .on_toggle(move |v| {
                let mut p = prefs.clone();
                set(&mut p, v);
                Message::SetPrefs(p)
            })
            .size(14),
        text(label).size(TEXT_SIZE_SMALL).color(TEXT_PRIMARY),
    ]
    .spacing(8)
    .align_y(Vertical::Center)
    .into()
}

/// A row of buttons, one per choice, the current one highlighted.
fn choices<'a, T: PartialEq + Copy + 'a>(
    current: T,
    options: &[(T, String)],
    prefs: &Prefs,
    set: impl Fn(&mut Prefs, T) + Clone + 'a,
) -> Element<'a, Message> {
    let mut r = row![].spacing(4);
    for (v, label) in options {
        let mut p = prefs.clone();
        set.clone()(&mut p, *v);
        r = r.push(button(text(label.clone()).size(TEXT_SIZE_SMALL)).padding(Padding::from([3, 8])).style(style::tab(*v == current)).on_press(Message::SetPrefs(p)));
    }
    r.into()
}

fn note(s: &str) -> Element<'_, Message> {
    text(s).size(TEXT_SIZE_SMALL).color(TEXT_MUTED).into()
}

pub fn prefs(app: &Blobr) -> Element<'_, Message> {
    let p = &app.prefs;
    let interval = |s: u32| if s < 60 { format!("{s} s") } else { format!("{} min", s / 60) };
    let mut autosave = column![
        section("Saving"),
        toggle("Save files automatically", p.autosave, p, |p, v| p.autosave = v),
    ]
    .spacing(8);
    if p.autosave {
        let steps: Vec<(u32, String)> = AUTOSAVE_STEPS.iter().map(|&s| (s, interval(s))).collect();
        autosave = autosave
            .push(row![text("Every").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY), choices(p.autosave_secs, &steps, p, |p, v| p.autosave_secs = v)].spacing(8).align_y(Vertical::Center))
            .push(note("Only files that have been saved once; new files still ask for a name."));
    }
    let nudges: Vec<(f64, String)> = [0.5, 1.0, 2.0, 5.0, 8.0].iter().map(|&n| (n, format!("{n} px"))).collect();
    let canvas = column![
        section("Canvas"),
        toggle("Scroll wheel zooms (Ctrl+scroll pans)", p.scroll_zooms, p, |p, v| p.scroll_zooms = v),
        row![text("Arrow keys move").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY), choices(p.nudge, &nudges, p, |p, v| p.nudge = v)].spacing(8).align_y(Vertical::Center),
        note("Shift+arrow moves ten times as far."),
    ]
    .spacing(8);
    let defaults = Prefs::default();
    let mut reset = p.clone();
    reset.layers_width = defaults.layers_width;
    reset.design_width = defaults.design_width;
    let panels = column![
        section("Panels"),
        toggle("Layers on the right, Design on the left", p.swap_panels, p, |p, v| p.swap_panels = v),
        toggle("Hide the toolbar and panels (Ctrl+\\)", p.hide_panels, p, |p, v| p.hide_panels = v),
        row![
            note("Drag a panel's inner edge to resize it."),
            space::horizontal(),
            button(text("Reset widths").size(TEXT_SIZE_SMALL)).padding(Padding::from([3, 8])).style(style::ghost(false)).on_press(Message::SetPrefs(reset)),
        ]
        .align_y(Vertical::Center),
    ]
    .spacing(8);
    dialog("Preferences", Message::ShowPrefs(false), column![autosave, canvas, panels].spacing(18).into(), 460.0)
}

/// Every shortcut, grouped, as shown in the Keyboard Shortcuts dialog.
pub const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Tools",
        &[
            ("V", "Move"),
            ("F", "Frame"),
            ("R", "Rectangle"),
            ("O", "Ellipse"),
            ("L", "Line"),
            ("P", "Pen"),
            ("T", "Text"),
            ("C", "Comment"),
            ("H, or hold Space", "Hand"),
            ("Z", "Zoom"),
        ],
    ),
    (
        "View",
        &[
            ("Ctrl+scroll, Ctrl++, Ctrl+-", "Zoom"),
            ("Shift+0", "Zoom to 100%"),
            ("Shift+1", "Zoom to fit"),
            ("Shift+2", "Zoom to selection"),
            ("Ctrl+F", "Find layers"),
            ("Ctrl+\\", "Hide or show panels"),
            ("F11", "Full screen"),
            ("Ctrl+Alt+Enter", "Present"),
            ("Ctrl+Alt+Shift+Enter", "Present in a new window"),
        ],
    ),
    (
        "Edit",
        &[
            ("Ctrl+Z, Ctrl+Shift+Z", "Undo, redo"),
            ("Ctrl+C, Ctrl+X, Ctrl+V", "Copy, cut, paste"),
            ("Ctrl+Shift+C", "Copy as PNG"),
            ("Ctrl+D", "Duplicate"),
            ("Delete", "Delete"),
            ("Ctrl+A, Ctrl+Shift+A", "Select all, none"),
            ("Ctrl+R", "Rename"),
            ("Arrows, Shift+arrows", "Nudge"),
            ("Enter, Escape", "Edit text or points, go up a level"),
        ],
    ),
    (
        "Object",
        &[
            ("Ctrl+G, Ctrl+Shift+G", "Group, ungroup"),
            ("Ctrl+Alt+G", "Frame selection"),
            ("Shift+A", "Auto layout"),
            ("Ctrl+E", "Flatten"),
            ("Ctrl+Shift+O", "Outline stroke"),
            ("Ctrl+Alt+K", "Create component"),
            ("Ctrl+Alt+B", "Detach instance"),
            ("Ctrl+], Ctrl+[", "Bring to front, send to back"),
            ("Alt+A, H, D", "Align left, centre, right"),
            ("Alt+W, V, S", "Align top, middle, bottom"),
            ("Alt+Shift+H, V", "Distribute"),
        ],
    ),
    (
        "File",
        &[
            ("Ctrl+N, Ctrl+O", "New tab, open"),
            ("Ctrl+S, Ctrl+Shift+S", "Save, save as"),
            ("Ctrl+W, Ctrl+Tab", "Close tab, next tab"),
            ("Ctrl+Shift+E", "Export PNG"),
            ("Ctrl+,", "Preferences"),
            ("Ctrl+Q", "Quit"),
        ],
    ),
];

pub fn shortcuts(_: &Blobr) -> Element<'_, Message> {
    let mut body = column![].spacing(14);
    for (group, keys) in SHORTCUTS {
        let mut list = column![section(group)].spacing(4);
        for (k, what) in *keys {
            list = list.push(
                row![text(*what).size(TEXT_SIZE_SMALL).color(TEXT_PRIMARY).width(Fill), text(*k).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY)]
                    .spacing(12),
            );
        }
        body = body.push(list);
    }
    dialog("Keyboard Shortcuts", Message::ShowShortcuts(false), container(scrollable(body.padding(Padding::ZERO.right(12)))).height(520).into(), 480.0)
}

pub fn about(_: &Blobr) -> Element<'_, Message> {
    let body = column![
        text(format!("{} {}", crate::APP_NAME, env!("CARGO_PKG_VERSION"))).size(TEXT_SIZE).color(TEXT_PRIMARY),
        note("The jamsdesign suite's interface and vector design app."),
        note("Files are .jdb documents; agents can edit them over MCP (blobr-mcp)."),
    ]
    .spacing(8);
    dialog("About Blobr", Message::ShowAbout(false), body.into(), 380.0)
}
