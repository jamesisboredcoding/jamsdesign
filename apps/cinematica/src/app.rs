//! Editor state, messages and keyboard shortcuts (Elm-style, like Gallerion).

use iced::keyboard::{self, Key, key::Named};
use iced::{Size, Subscription, Task};
use jd_ui::chrome::{self, WindowChrome};

use crate::tool::Tool;
use crate::view::MENU_COUNT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
}

/// A timeline track's header state. Clips arrive with the timeline model (M2).
#[derive(Debug, Clone)]
pub struct Track {
    pub name: String,
    pub kind: TrackKind,
    pub height: f32,
    pub locked: bool,
    /// Edits (insert, overwrite...) land on targeted tracks.
    pub targeted: bool,
    /// Video: shown. Audio: not muted.
    pub enabled: bool,
    pub solo: bool,
}

impl Track {
    fn new(kind: TrackKind, n: usize, targeted: bool) -> Self {
        let prefix = if kind == TrackKind::Video { "V" } else { "A" };
        Self {
            name: format!("{prefix}{n}"),
            kind,
            height: if n == 1 { 52.0 } else { 38.0 },
            locked: false,
            targeted,
            enabled: true,
            solo: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackToggle {
    Lock,
    Target,
    Enabled,
    Solo,
}

/// Sequence settings (fixed until the Sequence Settings dialog exists).
#[derive(Debug, Clone, Copy)]
pub struct SequenceFormat {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub sample_rate: u32,
}

pub struct Editor {
    pub chrome: WindowChrome,
    pub project_name: String,
    pub tool: Tool,
    pub source_tab: usize,
    pub project_tab: usize,
    pub sequence_tab: usize,
    pub search: String,
    pub grid_view: bool,
    pub snap: bool,
    pub linked: bool,
    /// Video tracks top to bottom (V3, V2, V1), then audio (A1, A2, A3).
    pub tracks: Vec<Track>,
    pub format: SequenceFormat,
    /// Program playhead, in seconds (becomes flicks with the timeline model).
    pub playhead: f64,
    /// Timeline zoom.
    pub px_per_second: f32,
}

#[derive(Debug, Clone)]
pub enum Message {
    Chrome(chrome::Event),
    /// A menu item: closes the dropdown, then runs the inner message.
    Menu(Box<Message>),
    CloseMenu,
    Quit,
    SelectTool(Tool),
    SourceTab(usize),
    ProjectTab(usize),
    SequenceTab(usize),
    Search(String),
    GridView(bool),
    ToggleSnap,
    ToggleLinked,
    ToggleTrack(usize, TrackToggle),
    Key(keyboard::Event),
    Noop,
}

impl Editor {
    pub fn new() -> (Self, Task<Message>) {
        let (chrome, task) = WindowChrome::new(MENU_COUNT, Size::new(1600.0, 960.0));
        let mut tracks: Vec<Track> = (1..=3).rev().map(|n| Track::new(TrackKind::Video, n, n == 1)).collect();
        tracks.extend((1..=3).map(|n| Track::new(TrackKind::Audio, n, n == 1)));
        let app = Self {
            chrome,
            project_name: "Untitled".into(),
            tool: Tool::Selection,
            source_tab: 0,
            project_tab: 0,
            sequence_tab: 0,
            search: String::new(),
            grid_view: true,
            snap: true,
            linked: true,
            tracks,
            format: SequenceFormat { width: 1920, height: 1080, fps: 25, sample_rate: 48_000 },
            playhead: 0.0,
            px_per_second: 24.0,
        };
        (app, task.map(Message::Chrome))
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Chrome(e) => return self.chrome.update(e).map(Message::Chrome),
            Message::Menu(m) => {
                self.chrome.open_menu = None;
                return self.update(*m);
            }
            Message::CloseMenu => self.chrome.open_menu = None,
            // Nothing to save yet. With projects (M4) this asks about unsaved changes first.
            Message::Quit => return self.chrome.close(),
            Message::SelectTool(t) => self.tool = t,
            Message::SourceTab(i) => self.source_tab = i,
            Message::ProjectTab(i) => self.project_tab = i,
            Message::SequenceTab(i) => self.sequence_tab = i,
            Message::Search(s) => self.search = s,
            Message::GridView(on) => self.grid_view = on,
            Message::ToggleSnap => self.snap = !self.snap,
            Message::ToggleLinked => self.linked = !self.linked,
            Message::ToggleTrack(i, what) => {
                if let Some(t) = self.tracks.get_mut(i) {
                    match what {
                        TrackToggle::Lock => t.locked = !t.locked,
                        TrackToggle::Target => t.targeted = !t.targeted,
                        TrackToggle::Enabled => t.enabled = !t.enabled,
                        TrackToggle::Solo => t.solo = !t.solo,
                    }
                }
            }
            Message::Key(event) => return self.on_key(event),
            Message::Noop => {}
        }
        Task::none()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            keyboard::listen().map(Message::Key),
            WindowChrome::subscription().map(Message::Chrome),
            iced::window::close_requests().map(|_| Message::Quit),
        ])
    }

    fn on_key(&mut self, event: keyboard::Event) -> Task<Message> {
        let keyboard::Event::KeyPressed { key, modifiers, .. } = event else { return Task::none() };
        if self.chrome.open_menu.is_some() && matches!(key.as_ref(), Key::Named(Named::Escape)) {
            self.chrome.open_menu = None;
            return Task::none();
        }
        let message = match key.as_ref() {
            Key::Named(Named::F11) => Some(Message::Chrome(chrome::Event::ToggleFullscreen)),
            Key::Character("q") if modifiers.command() => Some(Message::Quit),
            Key::Character("s") if modifiers.is_empty() => Some(Message::ToggleSnap),
            Key::Character(c) if modifiers.is_empty() => Tool::from_key(c).map(Message::SelectTool),
            _ => None,
        };
        message.map_or_else(Task::none, |m| self.update(m))
    }
}

/// `HH:MM:SS:FF` at a whole-number frame rate.
pub fn timecode(seconds: f64, fps: u32) -> String {
    let frames = (seconds * fps as f64).round().max(0.0) as u64;
    let fps = fps as u64;
    let (f, s) = (frames % fps, frames / fps);
    format!("{:02}:{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60, f)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: &str) -> Message {
        Message::Key(keyboard::Event::KeyPressed {
            key: Key::Character(c.into()),
            modified_key: Key::Character(c.into()),
            physical_key: keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        })
    }

    #[test]
    fn timecode_formats_hours_minutes_seconds_frames() {
        assert_eq!(timecode(0.0, 25), "00:00:00:00");
        assert_eq!(timecode(12.32, 25), "00:00:12:08");
        assert_eq!(timecode(3723.5, 30), "01:02:03:15");
    }

    #[test]
    fn tool_shortcuts_follow_premiere() {
        let (mut app, _) = Editor::new();
        for (k, tool) in [("c", Tool::Razor), ("b", Tool::RippleEdit), ("y", Tool::Slip), ("v", Tool::Selection)] {
            let _ = app.update(key(k));
            assert_eq!(app.tool, tool);
        }
    }

    #[test]
    fn s_toggles_snapping() {
        let (mut app, _) = Editor::new();
        assert!(app.snap);
        let _ = app.update(key("s"));
        assert!(!app.snap);
    }

    #[test]
    fn tracks_are_v3_to_v1_then_a1_to_a3_with_v1_a1_targeted() {
        let (app, _) = Editor::new();
        let names: Vec<_> = app.tracks.iter().map(|t| (t.name.as_str(), t.targeted)).collect();
        assert_eq!(
            names,
            [("V3", false), ("V2", false), ("V1", true), ("A1", true), ("A2", false), ("A3", false)]
        );
    }
}
