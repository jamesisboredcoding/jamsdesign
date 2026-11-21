//! Editor state, messages and keyboard shortcuts (Elm-style, like Gallerion).

use std::path::PathBuf;

use iced::keyboard::{self, Key, key::Named};
use iced::{Size, Subscription, Task, event, window};
use jd_ui::chrome::{self, WindowChrome};

use crate::media::{self, Analysis, MediaId, MediaItem};
use crate::source::{GoTo, Shuttle, SourceMonitor};
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
    /// Imported media, in import order.
    pub media: Vec<MediaItem>,
    next_media_id: MediaId,
    pub selected_media: Option<MediaId>,
    pub source: SourceMonitor,
    /// Video tracks top to bottom (V3, V2, V1), then audio (A1, A2, A3).
    pub tracks: Vec<Track>,
    pub format: SequenceFormat,
    /// Program playhead, in seconds (becomes flicks with the timeline model).
    pub playhead: f64,
    /// Timeline zoom.
    pub px_per_second: f32,
    /// Last error, shown in the Project panel until the next import.
    pub notice: Option<String>,
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

    ImportDialog,
    Import(Vec<PathBuf>),
    Analysed(MediaId, Box<Result<Analysis, String>>),
    SelectMedia(MediaId),
    /// Double-click in the bin: show the clip in the Source monitor.
    OpenInSource(MediaId),

    /// A display frame while the Source player is busy: pick up its newest frame.
    Tick,
    /// A Source frame finished uploading (numbered so late ones are dropped).
    FrameReady(u64, Option<iced_runtime::image::Allocation>),
    TogglePlay,
    Shuttle(Shuttle),
    Step(i64),
    GoTo(GoTo),
    /// Scrub bar: fraction of the clip.
    Scrub(f32),
    MarkIn,
    MarkOut,
    ClearInOut,

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
            media: Vec::new(),
            next_media_id: 1,
            selected_media: None,
            source: SourceMonitor::new(),
            tracks,
            format: SequenceFormat { width: 1920, height: 1080, fps: 25, sample_rate: 48_000 },
            playhead: 0.0,
            px_per_second: 24.0,
            notice: None,
        };
        (app, task.map(Message::Chrome))
    }

    /// Startup: every command-line argument is a file to import.
    pub fn boot() -> (Self, Task<Message>) {
        let (mut app, task) = Self::new();
        let files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        let import = if files.is_empty() { Task::none() } else { app.update(Message::Import(files)) };
        (app, Task::batch([task, import]))
    }

    pub fn media_item(&self, id: MediaId) -> Option<&MediaItem> {
        self.media.iter().find(|m| m.id == id)
    }

    /// The clip in the Source monitor.
    pub fn source_item(&self) -> Option<&MediaItem> {
        self.source.item.and_then(|id| self.media_item(id))
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

            Message::ImportDialog => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Import")
                            .add_filter("Video, audio and images", jd_media::IMPORT_EXTENSIONS)
                            .add_filter("All files", &["*"])
                            .pick_files()
                            .await
                            .map(|files| files.iter().map(|f| f.path().to_path_buf()).collect())
                            .unwrap_or_default()
                    },
                    Message::Import,
                );
            }
            Message::Import(paths) => {
                self.notice = None;
                let mut tasks = Vec::new();
                for path in paths {
                    // Importing a file twice just selects it, like Premiere's duplicate check.
                    if let Some(existing) = self.media.iter().find(|m| m.path == path) {
                        self.selected_media = Some(existing.id);
                        continue;
                    }
                    let id = self.next_media_id;
                    self.next_media_id += 1;
                    self.media.push(MediaItem::new(id, path.clone()));
                    self.selected_media = Some(id);
                    tasks.push(
                        media::background(move || media::analyse(&path), move |r| Message::Analysed(id, Box::new(r)))
                            .map(|m| m.unwrap_or(Message::Noop)),
                    );
                }
                return Task::batch(tasks);
            }
            Message::Analysed(id, result) => {
                let Some(item) = self.media.iter_mut().find(|m| m.id == id) else { return Task::none() };
                match *result {
                    Ok(a) => {
                        item.info = Some(a.info);
                        item.thumb = a.thumb.map(media::to_handle);
                    }
                    Err(e) => {
                        self.notice = Some(format!("Couldn't import {}: {e}", item.name));
                        item.error = Some(e);
                    }
                }
            }
            Message::SelectMedia(id) => self.selected_media = Some(id),
            Message::OpenInSource(id) => {
                self.selected_media = Some(id);
                let Some(item) = self.media_item(id) else { return Task::none() };
                match item.kind() {
                    media::Kind::Video | media::Kind::Still => {
                        let (name, path) = (item.name.clone(), item.path.clone());
                        self.source.load(id, &name, path);
                        self.source_tab = 0;
                    }
                    // Audio clips show a waveform in the Source monitor (M3, with audio).
                    media::Kind::Audio => self.notice = Some("Audio clips open in the Source monitor once audio playback lands".into()),
                    media::Kind::Pending | media::Kind::Unreadable => {}
                }
            }

            Message::Tick => {
                if let Some((n, handle)) = self.source.poll() {
                    return iced_runtime::image::allocate(handle).map(move |r| Message::FrameReady(n, r.ok()));
                }
            }
            Message::FrameReady(n, frame) => {
                if let Some(frame) = frame {
                    self.source.present(n, frame);
                }
            }
            Message::TogglePlay => self.source.toggle_play(),
            Message::Shuttle(s) => self.source.shuttle(s),
            Message::Step(n) => self.source.step(n),
            Message::GoTo(g) => self.source.go_to(g),
            Message::Scrub(f) => self.source.scrub(f as f64),
            Message::MarkIn => self.source.mark_in(),
            Message::MarkOut => self.source.mark_out(),
            Message::ClearInOut => self.source.clear_in_out(),

            Message::Key(event) => return self.on_key(event),
            Message::Noop => {}
        }
        Task::none()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            keyboard::listen().map(Message::Key),
            WindowChrome::subscription().map(Message::Chrome),
            window::close_requests().map(|_| Message::Quit),
            if self.source.busy() { window::frames().map(|_| Message::Tick) } else { Subscription::none() },
            // Only fires on X11: winit has no Wayland drag-and-drop yet.
            event::listen_with(|event, _status, _id| match event {
                iced::Event::Window(window::Event::FileDropped(path)) => Some(Message::Import(vec![path])),
                _ => None,
            }),
        ])
    }

    fn on_key(&mut self, event: keyboard::Event) -> Task<Message> {
        let keyboard::Event::KeyPressed { key, modifiers, .. } = event else { return Task::none() };
        if self.chrome.open_menu.is_some() && matches!(key.as_ref(), Key::Named(Named::Escape)) {
            self.chrome.open_menu = None;
            return Task::none();
        }
        let (cmd, shift) = (modifiers.command(), modifiers.shift());
        let plain = !cmd && !modifiers.alt();
        let message = match key.as_ref() {
            Key::Named(Named::F11) => Some(Message::Chrome(chrome::Event::ToggleFullscreen)),
            Key::Named(Named::Space) if plain => Some(Message::TogglePlay),
            Key::Named(Named::ArrowLeft) if plain => Some(Message::Step(if shift { -5 } else { -1 })),
            Key::Named(Named::ArrowRight) if plain => Some(Message::Step(if shift { 5 } else { 1 })),
            Key::Named(Named::Home) => Some(Message::GoTo(GoTo::Start)),
            Key::Named(Named::End) => Some(Message::GoTo(GoTo::End)),
            Key::Character(c) => {
                let c = c.to_lowercase();
                match c.as_str() {
                    "q" if cmd => Some(Message::Quit),
                    "i" if cmd => Some(Message::ImportDialog),
                    "x" if cmd && shift => Some(Message::ClearInOut),
                    _ if !plain => None,
                    "i" if shift => Some(Message::GoTo(GoTo::In)),
                    "o" if shift => Some(Message::GoTo(GoTo::Out)),
                    _ if shift => None,
                    "i" => Some(Message::MarkIn),
                    "o" => Some(Message::MarkOut),
                    "j" => Some(Message::Shuttle(Shuttle::Reverse)),
                    "k" => Some(Message::Shuttle(Shuttle::Stop)),
                    "l" => Some(Message::Shuttle(Shuttle::Forward)),
                    "s" => Some(Message::ToggleSnap),
                    c => Tool::from_key(c).map(Message::SelectTool),
                }
            }
            _ => None,
        };
        message.map_or_else(Task::none, |m| self.update(m))
    }
}

/// `HH:MM:SS:FF` at a whole-number frame rate.
pub fn timecode(seconds: f64, fps: u32) -> String {
    let frames = (seconds * fps as f64).round().max(0.0) as u64;
    let fps = fps.max(1) as u64;
    let (f, s) = (frames % fps, frames / fps);
    format!("{:02}:{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60, f)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_with(c: &str, modifiers: keyboard::Modifiers) -> Message {
        Message::Key(keyboard::Event::KeyPressed {
            key: Key::Character(c.into()),
            modified_key: Key::Character(c.into()),
            physical_key: keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
            location: keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })
    }

    fn key(c: &str) -> Message {
        key_with(c, keyboard::Modifiers::empty())
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
        // Ctrl+C is not the Razor tool.
        let _ = app.update(key_with("c", keyboard::Modifiers::CTRL));
        assert_eq!(app.tool, Tool::Selection);
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

    #[test]
    fn importing_adds_pending_items_and_skips_duplicates() {
        let (mut app, _) = Editor::new();
        let _ = app.update(Message::Import(vec!["/a/one.mp4".into(), "/a/two.wav".into()]));
        let _ = app.update(Message::Import(vec!["/a/one.mp4".into()]));
        let names: Vec<_> = app.media.iter().map(|m| (m.name.as_str(), m.kind())).collect();
        assert_eq!(names, [("one.mp4", media::Kind::Pending), ("two.wav", media::Kind::Pending)]);
        assert_eq!(app.selected_media, Some(app.media[0].id));
        let _ = app.update(Message::Analysed(app.media[1].id, Box::new(Err("bad".into()))));
        assert_eq!(app.media[1].kind(), media::Kind::Unreadable);
        assert!(app.notice.as_deref().unwrap().contains("two.wav"));
    }

    /// A 2 s, 25 fps test clip made with the ffmpeg CLI (None, and the test skips, without it).
    fn fixture() -> Option<PathBuf> {
        let dir = std::env::temp_dir().join("cinematica-tests");
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join("testsrc_2s.mp4");
        if path.exists() {
            return Some(path);
        }
        let tmp = dir.join(format!("tmp-{}.mp4", std::process::id()));
        let ok = std::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=2:size=320x240:rate=25"])
            .args(["-c:v", "libx264", "-g", "25", "-pix_fmt", "yuv420p"])
            .arg(&tmp)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            eprintln!("skipping: couldn't make a test video with the ffmpeg CLI");
            return None;
        }
        std::fs::rename(&tmp, &path).ok()?;
        Some(path)
    }

    /// Polls the Source monitor like the UI's frame subscription does.
    fn settle(app: &mut Editor, what: &str, done: impl Fn(&Editor) -> bool) {
        let start = std::time::Instant::now();
        loop {
            let _ = app.update(Message::Tick);
            if !app.source.busy() && done(app) {
                return;
            }
            assert!(start.elapsed().as_secs() < 10, "timed out waiting for {what}: {:?}", app.source.status);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn imported_clip_plays_in_the_source_monitor() {
        let Some(path) = fixture() else { return };
        let (mut app, _) = Editor::new();
        let _ = app.update(Message::Import(vec![path.clone()]));
        let id = app.media[0].id;
        let analysis = media::analyse(&path).unwrap();
        assert!(analysis.thumb.is_some(), "videos get a thumbnail");
        let _ = app.update(Message::Analysed(id, Box::new(Ok(analysis))));
        assert_eq!(app.media[0].kind(), media::Kind::Video);
        assert_eq!(timecode(app.media[0].duration(), app.media[0].fps(25)), "00:00:02:00");

        let _ = app.update(Message::OpenInSource(id));
        assert_eq!(app.source.title, "Source: testsrc_2s.mp4");
        settle(&mut app, "load", |a| a.source.status.duration > 0.0);

        // Right arrow x3, mark In; End, mark Out.
        for _ in 0..3 {
            let _ = app.update(Message::Step(1));
        }
        settle(&mut app, "steps", |a| (a.source.status.position * 25.0).round() == 3.0);
        let _ = app.update(Message::MarkIn);
        let _ = app.update(Message::GoTo(GoTo::End));
        settle(&mut app, "end", |a| (a.source.status.position * 25.0).round() == 49.0);
        let _ = app.update(Message::MarkOut);
        assert!((app.source.marked_duration() - 47.0 / 25.0).abs() < 1e-6);

        // L from the end restarts from the start and plays through to the end again.
        let _ = app.update(key("l"));
        assert!(app.source.status.playing);
        let _ = app.update(key("l"));
        assert_eq!(app.source.status.speed, 2.0, "second L doubles the speed");
        settle(&mut app, "playback", |a| !a.source.status.playing);
        assert_eq!((app.source.status.position * 25.0).round(), 49.0);

        // Scrub to the middle.
        let _ = app.update(Message::Scrub(0.5));
        settle(&mut app, "scrub", |a| (a.source.status.position * 25.0).round() == 24.0 || (a.source.status.position * 25.0).round() == 25.0);
    }

    #[test]
    fn transport_keys_without_a_clip_do_nothing() {
        let (mut app, _) = Editor::new();
        for k in ["l", "j", "k", "i", "o"] {
            let _ = app.update(key(k));
        }
        assert!(!app.source.status.playing);
        assert_eq!((app.source.in_point, app.source.out_point), (None, None));
    }
}
