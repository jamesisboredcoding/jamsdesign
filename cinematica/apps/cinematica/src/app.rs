//! Editor state, messages and keyboard shortcuts (Elm-style, like Gallerion). Timeline
//! edits live in `edit.rs`.

use std::path::PathBuf;

use iced::keyboard::{self, Key, key::Named};
use iced::{Size, Subscription, Task, event, window};
use jd_timeline::{ClipId, Edge, FrameRate, History, Sequence, Time, TrackRef};
use jd_ui::chrome::{self, WindowChrome};

use crate::media::{self, Analysis, MediaId, MediaItem};
use crate::program::ProgramMonitor;
use crate::source::{GoTo, Shuttle, SourceMonitor};
use crate::tool::Tool;
use crate::view::MENU_COUNT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackToggle {
    Lock,
    Target,
    Enabled,
    Solo,
}

/// Which panel transport keys (Space, J/K/L, arrows, I/O) act on, like Premiere's panel focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Source,
    Timeline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Monitor {
    Source,
    Program,
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
    pub focus: Focus,
    /// Imported media, in import order.
    pub media: Vec<MediaItem>,
    next_media_id: MediaId,
    pub selected_media: Option<MediaId>,
    pub source: SourceMonitor,
    pub program: ProgramMonitor,
    pub sequence: Sequence,
    pub history: History<Sequence>,
    pub selection: Vec<ClipId>,
    pub playhead: Time,
    /// Sequence In/Out marks (Program monitor / timeline).
    pub seq_in: Option<Time>,
    pub seq_out: Option<Time>,
    /// Timeline zoom, and the time at the timeline's left edge.
    pub px_per_second: f32,
    pub scroll: Time,
    /// Width of the timeline's lane area (for zoom to fit and keeping the playhead in view).
    pub timeline_width: f32,
    /// Last error or hint, shown in the timeline toolbar.
    pub notice: Option<String>,
    /// Audio meter levels (dBFS, left and right), falling back slowly like a peak meter.
    pub meters: [f32; 2],
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
    Focus(Focus),
    /// A monitor's button: focus that monitor's panel, then run the message.
    OnPanel(Focus, Box<Message>),

    ImportDialog,
    Import(Vec<PathBuf>),
    Analysed(MediaId, Box<Result<Analysis, String>>),
    SelectMedia(MediaId),
    /// Double-click in the bin: show the clip in the Source monitor.
    OpenInSource(MediaId),

    /// A display frame while a monitor's player is busy: pick up its newest frame.
    Tick,
    /// A monitor frame finished uploading (numbered so late ones are dropped).
    FrameReady(Monitor, u64, Option<iced_runtime::image::Allocation>),
    // Transport: acts on the focused panel.
    TogglePlay,
    Shuttle(Shuttle),
    Step(i64),
    GoTo(GoTo),
    MarkIn,
    MarkOut,
    ClearInOut,
    /// Source scrub bar: fraction of the clip.
    Scrub(f32),
    /// Up/Down: previous (-1) or next (+1) edit point.
    EditPoint(i8),

    // Timeline.
    TimelineWidth(f32),
    SetPlayhead(Time),
    Select { id: ClipId, add: bool },
    Deselect,
    SelectAll,
    /// Track Select Forward: everything from `at` on `track` (all tracks for `None`).
    SelectForward { at: Time, track: Option<TrackRef> },
    MoveClips { ids: Vec<ClipId>, delta: Time, video_tracks: isize, audio_tracks: isize },
    TrimClips { ids: Vec<ClipId>, edge: Edge, delta: Time, ripple: bool },
    SlipClips { ids: Vec<ClipId>, delta: Time },
    Razor { at: Time, tracks: Vec<TrackRef> },
    ScrollTimeline(f32),
    ZoomTimeline { factor: f32, anchor_x: f32 },
    ZoomToFit,
    Insert,
    Overwrite,
    Lift,
    Extract,
    AddEdit { all: bool },
    DeleteSelection { ripple: bool },
    Undo,
    Redo,
    ToggleTrack(TrackRef, TrackToggle),

    Key(keyboard::Event),
    Noop,
}

impl Editor {
    pub fn new() -> (Self, Task<Message>) {
        let (chrome, task) = WindowChrome::new(MENU_COUNT, Size::new(1600.0, 960.0));
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
            focus: Focus::Timeline,
            media: Vec::new(),
            next_media_id: 1,
            selected_media: None,
            source: SourceMonitor::new(),
            program: ProgramMonitor::new("Sequence 01"),
            sequence: Sequence::new("Sequence 01", FrameRate::new(25, 1), (1920, 1080), 3, 3),
            history: History::default(),
            selection: Vec::new(),
            playhead: 0,
            seq_in: None,
            seq_out: None,
            px_per_second: 24.0,
            scroll: 0,
            timeline_width: 900.0,
            notice: None,
            meters: [f32::NEG_INFINITY; 2],
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
            Message::Focus(f) => self.focus = f,
            Message::OnPanel(f, m) => {
                self.focus = f;
                return self.update(*m);
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
                        item.waveform = a.waveform;
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
                    media::Kind::Video | media::Kind::Still | media::Kind::Audio => {
                        let (name, path) = (item.name.clone(), item.path.clone());
                        let info = crate::source::ClipInfo {
                            has_video: item.info.as_ref().is_some_and(|i| i.video.is_some()),
                            has_audio: item.info.as_ref().is_some_and(|i| i.audio.is_some()),
                            rate: item.info.as_ref().and_then(|i| i.video.as_ref()).filter(|v| !v.still).map_or(self.sequence.rate, |v| {
                                jd_timeline::FrameRate::new(v.rate.num.max(1) as u32, v.rate.den.max(1) as u32)
                            }),
                            duration: item.duration(),
                        };
                        self.source.load(id, &name, path, info);
                        self.source_tab = 0;
                        self.focus = Focus::Source;
                    }
                    media::Kind::Pending | media::Kind::Unreadable => {}
                }
            }

            Message::Tick => {
                let mut tasks = Vec::new();
                if let Some((n, handle)) = self.source.poll() {
                    tasks.push(iced_runtime::image::allocate(handle).map(move |r| Message::FrameReady(Monitor::Source, n, r.ok())));
                }
                if self.program.is_playing() {
                    let (frame, position, ended) = self.program.poll_playback();
                    if let Some((n, handle)) = frame {
                        tasks.push(iced_runtime::image::allocate(handle).map(move |r| Message::FrameReady(Monitor::Program, n, r.ok())));
                    }
                    self.playhead = position;
                    self.follow_playhead();
                    if ended {
                        self.set_playhead(position);
                    }
                    if let Some(e) = self.program.take_error() {
                        self.notice = Some(e);
                    }
                } else if let Some((n, handle)) = self.program.poll() {
                    tasks.push(iced_runtime::image::allocate(handle).map(move |r| Message::FrameReady(Monitor::Program, n, r.ok())));
                }
                self.update_meters();
                return Task::batch(tasks);
            }
            Message::FrameReady(monitor, n, frame) => match (monitor, frame) {
                (Monitor::Source, Some(f)) => self.source.present(n, f),
                (Monitor::Program, Some(f)) => self.program.present(n, f),
                (_, None) => {}
            },
            Message::Scrub(f) => self.source.scrub(f as f64),

            Message::TogglePlay
            | Message::Shuttle(_)
            | Message::Step(_)
            | Message::GoTo(_)
            | Message::MarkIn
            | Message::MarkOut
            | Message::ClearInOut
            | Message::EditPoint(_)
                if self.focus == Focus::Source =>
            {
                match message {
                    Message::TogglePlay => self.source.toggle_play(),
                    Message::Shuttle(s) => self.source.shuttle(s),
                    Message::Step(n) => self.source.step(n),
                    Message::GoTo(g) => self.source.go_to(g),
                    Message::MarkIn => self.source.mark_in(),
                    Message::MarkOut => self.source.mark_out(),
                    Message::ClearInOut => self.source.clear_in_out(),
                    _ => {}
                }
            }
            Message::TogglePlay => {
                if self.program.is_playing() {
                    self.stop_playback();
                } else {
                    self.play_sequence(1.0);
                }
            }
            Message::Shuttle(press) => {
                match crate::source::shuttle_speed(self.program.is_playing(), self.program.speed, press) {
                    Some(speed) => self.play_sequence(speed),
                    None => self.stop_playback(),
                }
            }
            Message::Step(n) => self.set_playhead(self.playhead + self.sequence.rate.from_frames(n)),
            Message::GoTo(g) => {
                let t = match g {
                    GoTo::Start => Some(0),
                    GoTo::End => Some(self.sequence.duration()),
                    GoTo::In => self.seq_in,
                    GoTo::Out => self.seq_out,
                };
                if let Some(t) = t {
                    self.set_playhead(t);
                }
            }
            Message::MarkIn => {
                self.seq_in = Some(self.playhead);
                if self.seq_out.is_some_and(|o| o <= self.playhead) {
                    self.seq_out = None;
                }
            }
            Message::MarkOut => {
                // The Out mark sits after the frame under the playhead, so it's included.
                let out = self.playhead + self.sequence.rate.frame();
                self.seq_out = Some(out);
                if self.seq_in.is_some_and(|i| i >= out) {
                    self.seq_in = None;
                }
            }
            Message::ClearInOut => {
                self.seq_in = None;
                self.seq_out = None;
            }
            Message::EditPoint(dir) => {
                let pts = self.sequence.edit_points();
                let t = if dir < 0 {
                    pts.iter().rev().find(|&&p| p < self.playhead).copied()
                } else {
                    pts.iter().find(|&&p| p > self.playhead).copied()
                };
                if let Some(t) = t {
                    self.set_playhead(t);
                }
            }

            Message::Key(event) => return self.on_key(event),
            Message::Noop => {}
            timeline => self.timeline_message(timeline),
        }
        Task::none()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            keyboard::listen().map(Message::Key),
            WindowChrome::subscription().map(Message::Chrome),
            window::close_requests().map(|_| Message::Quit),
            if self.source.busy() || self.program.busy() || self.meters.iter().any(|m| m.is_finite()) {
                window::frames().map(|_| Message::Tick)
            } else {
                Subscription::none()
            },
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
            Key::Named(Named::ArrowUp) if plain => Some(Message::EditPoint(-1)),
            Key::Named(Named::ArrowDown) if plain => Some(Message::EditPoint(1)),
            Key::Named(Named::Home) => Some(Message::GoTo(GoTo::Start)),
            Key::Named(Named::End) => Some(Message::GoTo(GoTo::End)),
            Key::Named(Named::Delete | Named::Backspace) if !cmd => Some(Message::DeleteSelection { ripple: shift }),
            Key::Character(c) => {
                let c = c.to_lowercase();
                match c.as_str() {
                    "q" if cmd => Some(Message::Quit),
                    "i" if cmd => Some(Message::ImportDialog),
                    "x" if cmd && shift => Some(Message::ClearInOut),
                    "z" if cmd && shift => Some(Message::Redo),
                    "z" if cmd => Some(Message::Undo),
                    "y" if cmd => Some(Message::Redo),
                    "k" if cmd => Some(Message::AddEdit { all: shift }),
                    "a" if cmd && shift => Some(Message::Deselect),
                    "a" if cmd => Some(Message::SelectAll),
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
                    "," => Some(Message::Insert),
                    "." => Some(Message::Overwrite),
                    ";" => Some(Message::Lift),
                    "'" => Some(Message::Extract),
                    "=" | "+" => Some(Message::ZoomTimeline { factor: 1.5, anchor_x: f32::NAN }),
                    "-" => Some(Message::ZoomTimeline { factor: 1.0 / 1.5, anchor_x: f32::NAN }),
                    "\\" => Some(Message::ZoomToFit),
                    c => Tool::from_key(c).map(Message::SelectTool),
                }
            }
            _ => None,
        };
        message.map_or_else(Task::none, |m| self.update(m))
    }
}

/// `HH:MM:SS:FF` at a whole-number frame rate, from seconds (media times).
pub fn timecode(seconds: f64, fps: u32) -> String {
    let frames = (seconds * fps as f64).round().max(0.0) as u64;
    let fps = fps.max(1) as u64;
    let (f, s) = (frames % fps, frames / fps);
    format!("{:02}:{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60, f)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn key_with(c: &str, modifiers: keyboard::Modifiers) -> Message {
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

    pub fn key(c: &str) -> Message {
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

    /// A 2 s, 25 fps test clip with audio, made with the ffmpeg CLI (None, and the test
    /// skips, without it).
    pub fn fixture() -> Option<PathBuf> {
        // Tests run in parallel threads of one process: make the file once.
        static FIXTURE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
        FIXTURE.get_or_init(make_fixture).clone()
    }

    fn make_fixture() -> Option<PathBuf> {
        let dir = std::env::temp_dir().join("cinematica-tests");
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join("testsrc_2s_av.mp4");
        if path.exists() {
            return Some(path);
        }
        let tmp = dir.join(format!("tmp-{}.mp4", std::process::id()));
        let ok = std::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=2:size=320x240:rate=25"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2:sample_rate=48000"])
            .args(["-c:v", "libx264", "-g", "25", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
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

    /// Polls the monitors like the UI's frame subscription does.
    pub fn settle(app: &mut Editor, what: &str, done: impl Fn(&Editor) -> bool) {
        let start = std::time::Instant::now();
        loop {
            let _ = app.update(Message::Tick);
            if !app.source.busy() && !app.program.busy() && done(app) {
                return;
            }
            assert!(start.elapsed().as_secs() < 10, "timed out waiting for {what}: {:?}", app.source.status);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// The fixture imported, analysed and open in the Source monitor.
    pub fn with_clip() -> Option<(Editor, MediaId)> {
        let path = fixture()?;
        let (mut app, _) = Editor::new();
        let _ = app.update(Message::Import(vec![path.clone()]));
        let id = app.media[0].id;
        let _ = app.update(Message::Analysed(id, Box::new(media::analyse(&path))));
        let _ = app.update(Message::OpenInSource(id));
        settle(&mut app, "load", |a| a.source.status.duration > 0.0);
        Some((app, id))
    }

    #[test]
    fn imported_clip_plays_in_the_source_monitor() {
        let Some((mut app, id)) = with_clip() else { return };
        assert_eq!(app.media_item(id).unwrap().kind(), media::Kind::Video);
        assert!(app.media_item(id).unwrap().thumb.is_some(), "videos get a thumbnail");
        assert_eq!(app.source.title, "Source: testsrc_2s_av.mp4");
        assert_eq!(app.focus, Focus::Source);

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

        let _ = app.update(Message::Scrub(0.5));
        settle(&mut app, "scrub", |a| matches!((a.source.status.position * 25.0).round() as i64, 24 | 25));
    }

    #[test]
    fn transport_keys_without_a_clip_do_nothing() {
        let (mut app, _) = Editor::new();
        app.focus = Focus::Source;
        for k in ["l", "j", "k", "i", "o"] {
            let _ = app.update(key(k));
        }
        assert!(!app.source.status.playing);
        assert_eq!((app.source.in_point, app.source.out_point), (None, None));
    }
}
