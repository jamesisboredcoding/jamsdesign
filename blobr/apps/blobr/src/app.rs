//! Blobr's state and everything that changes it. Every document change goes through
//! `jd_vector` ops (`run` / `run_gesture`), so it's undoable and validated; the canvas is a
//! `jd-vector-render` image redrawn whenever the document, camera or window changes.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::keyboard::{self, Key, key::Named};
use iced::widget::image;
use iced::{Point, Size, Subscription, Task, Vector, mouse, window};
use iced_runtime::image::Allocation;
use jd_core::Rgba;
use jd_ui::chrome::{self, WindowChrome};
use jd_vector::{
    AutoLayout, Direction, Document, Editor, ExportFormat, NodeId, NodeKind, NodeSpec, Op, Padding, Paint, PaintKind, Patch, PropDef,
    PropValue, Sizing, TextStyle, file,
};
use jd_vector_render::{CosmicMeasure, RenderOptions, export_svg, render_png, render_region};
use kurbo::{Affine, Rect, Vec2};

use crate::tool::Tool;
use crate::view::{self, MENU_COUNT};

/// The canvas camera: the page point at the canvas's top-left corner and the zoom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub origin: kurbo::Point,
    pub zoom: f64,
}

impl Camera {
    pub fn to_page(&self, p: Point) -> kurbo::Point {
        kurbo::Point::new(self.origin.x + p.x as f64 / self.zoom, self.origin.y + p.y as f64 / self.zoom)
    }

    pub fn to_screen(&self, p: kurbo::Point) -> Point {
        Point::new(((p.x - self.origin.x) * self.zoom) as f32, ((p.y - self.origin.y) * self.zoom) as f32)
    }
}

/// Which side or corner of the selection box a resize drags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl Handle {
    pub const ALL: [Handle; 8] =
        [Handle::TopLeft, Handle::Top, Handle::TopRight, Handle::Right, Handle::BottomRight, Handle::Bottom, Handle::BottomLeft, Handle::Left];

    /// Where on a box (0..1 on each axis).
    pub fn anchor(self) -> (f64, f64) {
        match self {
            Handle::TopLeft => (0.0, 0.0),
            Handle::Top => (0.5, 0.0),
            Handle::TopRight => (1.0, 0.0),
            Handle::Right => (1.0, 0.5),
            Handle::BottomRight => (1.0, 1.0),
            Handle::Bottom => (0.5, 1.0),
            Handle::BottomLeft => (0.0, 1.0),
            Handle::Left => (0.0, 0.5),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Drag {
    Pan { start: Point, origin: kurbo::Point },
    /// Each node with its starting x/y and the map from page to its parent's space.
    /// `started` once the pointer has gone past the drag threshold (a click doesn't move).
    Move { start: kurbo::Point, items: Vec<(NodeId, f64, f64, Affine)>, started: bool },
    Resize { id: NodeId, handle: Handle, start: kurbo::Point, rect: Rect, to_parent: Affine, started: bool },
    Marquee { start: kurbo::Point, now: kurbo::Point },
    Create { tool: Tool, start: kurbo::Point, now: kurbo::Point },
}

/// A Design-panel value box being typed into.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Field {
    Name,
    X,
    Y,
    W,
    H,
    Rotation,
    Radius,
    Opacity,
    Gap,
    PadH,
    PadV,
    Text,
    FontSize,
    FontStyle,
    Fill(usize),
    Stroke(usize),
    StrokeWeight,
    Prop(String),
    PageBackground,
}

#[derive(Debug, Clone)]
pub enum Message {
    Chrome(chrome::Event),
    Menu(Box<Message>),
    CloseMenu,
    Noop,
    Quit,
    Key(keyboard::Event),
    ScaleFactor(f32),
    Viewport(Size),
    /// A screen refresh while dragging (see `update`).
    Frame,
    /// A canvas render finished uploading.
    CanvasReady(u64, Option<Allocation>),
    Tool(Tool),
    CanvasPress { at: Point, button: mouse::Button },
    CanvasMove(Point),
    CanvasRelease(Point),
    CanvasScroll { delta: Vector, at: Point },
    Select(NodeId, bool),
    SelectPage(NodeId),
    AddPage,
    ToggleExpand(NodeId),
    ToggleVisible(NodeId),
    ToggleLock(NodeId),
    FieldInput(Field, String),
    FieldSubmit(Field),
    SetSizing { horizontal: bool, sizing: Sizing },
    ToggleAutoLayout,
    SetDirection(Direction),
    AddFill,
    RemoveFill(usize),
    AddStroke,
    RemoveStroke(usize),
    SetBool(String, bool),
    SetVariant(String, String),
    Undo,
    Redo,
    Delete,
    Duplicate,
    SelectAll,
    Deselect,
    CreateComponent,
    Detach,
    ResetOverrides,
    ZoomIn,
    ZoomOut,
    ZoomFit,
    Zoom100,
    ZoomSelection,
    New,
    Open,
    Opened(Option<PathBuf>),
    Save,
    SaveAs,
    SavedAs(Option<PathBuf>),
    Export(ExportFormat),
    Exported(Option<PathBuf>, Vec<u8>),
}

/// The status bar's message.
#[derive(Debug, Clone, Default)]
pub struct Status {
    pub text: String,
    pub error: bool,
}

pub struct Blobr {
    pub chrome: WindowChrome,
    pub ed: Editor,
    pub path: Option<PathBuf>,
    pub page: NodeId,
    pub selection: Vec<NodeId>,
    pub hover: Option<NodeId>,
    pub tool: Tool,
    pub camera: Camera,
    pub viewport: Size,
    pub scale_factor: f32,
    /// The canvas image on screen. A new render replaces it only once uploaded to the GPU
    /// (`CanvasReady`), so zooming and panning never show an empty frame.
    pub canvas: Option<Allocation>,
    rendered: Option<(u64, Camera, Size, f32, NodeId)>,
    /// Number of the latest render, so a late upload can't replace a newer one.
    render_seq: u64,
    shown_seq: u64,
    version: u64,
    pub drag: Option<Drag>,
    pub space_held: bool,
    pub modifiers: keyboard::Modifiers,
    pub expanded: BTreeSet<NodeId>,
    pub status: Status,
    pub drafts: HashMap<Field, String>,
    last_click: Option<(Instant, Point)>,
    fit_pending: bool,
}

const MIN_ZOOM: f64 = 0.02;
/// Screen pixels the pointer must travel before a press becomes a drag.
const DRAG_THRESHOLD: f64 = 3.0;
const MAX_ZOOM: f64 = 256.0;

impl Blobr {
    pub fn boot() -> (Self, Task<Message>) {
        let (chrome, task) = WindowChrome::new(MENU_COUNT, Size::new(1600.0, 960.0));
        let doc = Document::new();
        let page = doc.pages()[0];
        let mut app = Self {
            chrome,
            ed: Editor::new(doc),
            path: None,
            page,
            selection: Vec::new(),
            hover: None,
            tool: Tool::Move,
            camera: Camera { origin: kurbo::Point::new(-100.0, -100.0), zoom: 1.0 },
            viewport: Size::ZERO,
            scale_factor: 1.0,
            canvas: None,
            rendered: None,
            render_seq: 0,
            shown_seq: 0,
            version: 0,
            drag: None,
            space_held: false,
            modifiers: keyboard::Modifiers::empty(),
            expanded: BTreeSet::new(),
            status: Status { text: "Press F and drag to draw a frame".into(), error: false },
            drafts: HashMap::new(),
            last_click: None,
            fit_pending: false,
        };
        app.viewport = view::canvas_size(app.chrome.size);
        // Every command-line argument is a file to open (the suite's apps take no flags).
        let open = std::env::args().nth(1).map(PathBuf::from);
        if let Some(path) = open {
            app.open_path(path);
        }
        let render = app.refresh();
        (app, Task::batch([task.map(Message::Chrome), render]))
    }

    pub fn doc(&self) -> &Document {
        &self.ed.doc
    }

    pub fn file_name(&self) -> String {
        let name = self.path.as_ref().and_then(|p| p.file_name()).map_or("Untitled".into(), |n| n.to_string_lossy().into_owned());
        if self.ed.history.is_dirty() { format!("{name} *") } else { name }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            keyboard::listen().map(Message::Key),
            WindowChrome::subscription().map(Message::Chrome),
            window::close_requests().map(|_| Message::Quit),
            if self.drag.is_some() { window::frames().map(|_| Message::Frame) } else { Subscription::none() },
        ])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let before = self.selection.clone();
        let task = self.handle(message);
        // Half-typed values belong to the layer they were typed for.
        if self.selection != before {
            self.drafts.clear();
        }
        // While dragging, mouse moves can outpace the screen: render once per frame instead.
        if self.drag.is_none() { Task::batch([task, self.refresh()]) } else { task }
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Chrome(chrome::Event::Opened(id)) => {
                let task = self.chrome.update(chrome::Event::Opened(id)).map(Message::Chrome);
                let scale = id.map_or_else(Task::none, |id| window::scale_factor(id).map(Message::ScaleFactor));
                return Task::batch([task, scale]);
            }
            Message::Chrome(e) => {
                // The canvas overlay reports its real size (Message::Viewport).
                return self.chrome.update(e).map(Message::Chrome);
            }
            Message::Menu(m) => {
                self.chrome.open_menu = None;
                return self.handle(*m);
            }
            Message::CloseMenu => self.chrome.open_menu = None,
            Message::Noop => {}
            Message::Frame => return self.refresh(),
            Message::CanvasReady(seq, allocation) => {
                if seq > self.shown_seq
                    && let Some(a) = allocation
                {
                    self.shown_seq = seq;
                    self.canvas = Some(a);
                }
            }
            Message::Quit => return self.chrome.close(),
            Message::Key(e) => return self.on_key(e),
            Message::ScaleFactor(s) => self.scale_factor = s.max(0.5),
            Message::Viewport(size) => {
                self.viewport = size;
                if std::mem::take(&mut self.fit_pending) {
                    self.zoom_fit(false);
                }
            }
            Message::Tool(t) => {
                self.tool = t;
                self.drag = None;
            }
            Message::CanvasPress { at, button } => self.press(at, button),
            Message::CanvasMove(at) => self.drag_to(at),
            Message::CanvasRelease(at) => self.release(at),
            Message::CanvasScroll { delta, at } => self.scroll(delta, at),
            Message::Select(id, add) => self.select(id, add),
            Message::SelectPage(p) => {
                if p != self.page {
                    self.page = p;
                    self.selection.clear();
                    self.zoom_fit(false);
                }
            }
            Message::AddPage => {
                let n = self.doc().pages().len() + 1;
                let ops = [Op::CreatePage { name: format!("Page {n}"), key: None, index: None }];
                if let Some(done) = self.run("Add page", &ops) {
                    self.page = done.created[0];
                    self.selection.clear();
                }
            }
            Message::ToggleExpand(id) => {
                if !self.expanded.remove(&id) {
                    self.expanded.insert(id);
                }
            }
            Message::ToggleVisible(id) => {
                let v = self.doc().node(id).is_some_and(|n| n.props.visible);
                self.run("Show/hide", &[Op::set(id, Patch { visible: Some(!v), ..Patch::default() })]);
            }
            Message::ToggleLock(id) => {
                let l = self.doc().node(id).is_some_and(|n| n.props.locked);
                self.run("Lock/unlock", &[Op::set(id, Patch { locked: Some(!l), ..Patch::default() })]);
            }
            Message::FieldInput(f, s) => {
                self.drafts.insert(f, s);
            }
            Message::FieldSubmit(f) => self.submit(f),
            Message::SetSizing { horizontal, sizing } => {
                let patch = if horizontal { Patch { sizing_h: Some(sizing), ..Patch::default() } } else { Patch { sizing_v: Some(sizing), ..Patch::default() } };
                self.patch_selection("Change sizing", patch);
            }
            Message::ToggleAutoLayout => self.toggle_auto_layout(),
            Message::SetDirection(d) => {
                let ops: Vec<Op> = self
                    .selection
                    .iter()
                    .filter_map(|&id| {
                        let l = self.doc().node(id)?.props.layout.clone()?;
                        Some(Op::set(id, Patch { layout: Some(Some(AutoLayout { direction: d, ..l })), ..Patch::default() }))
                    })
                    .collect();
                self.run("Auto layout direction", &ops);
            }
            Message::AddFill => self.edit_paints(true, |p| p.push(Paint::hex("#D9D9D9"))),
            Message::RemoveFill(i) => self.edit_paints(true, |p| {
                if i < p.len() {
                    p.remove(i);
                }
            }),
            Message::AddStroke => self.edit_paints(false, |p| p.push(Paint::hex("#000000"))),
            Message::RemoveStroke(i) => self.edit_paints(false, |p| {
                if i < p.len() {
                    p.remove(i);
                }
            }),
            Message::SetBool(name, b) => {
                self.patch_selection("Set property", Patch { values: Some([(name, PropValue::Bool(b))].into()), ..Patch::default() })
            }
            Message::SetVariant(name, v) => {
                self.patch_selection("Switch variant", Patch { values: Some([(name, PropValue::Text(v))].into()), ..Patch::default() })
            }
            Message::Undo => {
                if let Some(label) = self.ed.undo() {
                    self.after_history(format!("Undid {label}"));
                }
            }
            Message::Redo => {
                if let Some(label) = self.ed.redo() {
                    self.after_history(format!("Redid {label}"));
                }
            }
            Message::Delete => {
                let ops: Vec<Op> = self.selection.iter().map(|&id| Op::Delete { id: id.into() }).collect();
                if !ops.is_empty() && self.run("Delete", &ops).is_some() {
                    self.selection.clear();
                }
            }
            Message::Duplicate => {
                let ops: Vec<Op> = self.selection.iter().map(|&id| Op::Duplicate { id: id.into(), key: Some(id.to_string()) }).collect();
                if let Some(done) = self.run("Duplicate", &ops) {
                    self.selection = self.selection.iter().filter_map(|id| done.id(&id.to_string())).collect();
                }
            }
            Message::SelectAll => {
                let scope = self.selection.first().and_then(|&s| self.doc().parent(s)).unwrap_or(self.page);
                self.selection = self.doc().children(scope).to_vec();
            }
            Message::Deselect => self.selection.clear(),
            Message::CreateComponent => {
                let ops: Vec<Op> = self.selection.iter().map(|&id| Op::MakeComponent { id: id.into() }).collect();
                self.run("Create component", &ops);
            }
            Message::Detach => {
                let ops: Vec<Op> = self.selection.iter().map(|&id| Op::Detach { id: id.into() }).collect();
                self.run("Detach instance", &ops);
            }
            Message::ResetOverrides => {
                let ops: Vec<Op> = self.selection.iter().map(|&id| Op::ResetOverrides { id: id.into() }).collect();
                self.run("Reset overrides", &ops);
            }
            Message::ZoomIn => self.zoom_at(2.0, None),
            Message::ZoomOut => self.zoom_at(0.5, None),
            Message::ZoomFit => self.zoom_fit(false),
            Message::ZoomSelection => self.zoom_fit(true),
            Message::Zoom100 => self.zoom_at(1.0 / self.camera.zoom, None),
            Message::New => {
                // Unsaved-changes prompts come with dialogs; until then, refuse when dirty.
                if self.ed.history.is_dirty() {
                    self.error("Save first: there are unsaved changes (prompts come later)".into());
                } else {
                    self.load(Document::new(), None);
                }
            }
            Message::Open => {
                return Task::perform(
                    rfd::AsyncFileDialog::new().add_filter("Blobr document", &[file::EXTENSION]).pick_file(),
                    |f| Message::Opened(f.map(|f| f.path().to_path_buf())),
                );
            }
            Message::Opened(Some(path)) => self.open_path(path),
            Message::Opened(None) | Message::SavedAs(None) | Message::Exported(None, _) => {}
            Message::Save => match self.path.clone() {
                Some(path) => self.save_to(path),
                None => return self.handle(Message::SaveAs),
            },
            Message::SaveAs => {
                let name = self.path.as_ref().and_then(|p| p.file_name()).map_or("Untitled.jdb".into(), |n| n.to_string_lossy().into_owned());
                return Task::perform(
                    rfd::AsyncFileDialog::new().add_filter("Blobr document", &[file::EXTENSION]).set_file_name(name).save_file(),
                    |f| Message::SavedAs(f.map(|f| f.path().to_path_buf())),
                );
            }
            Message::SavedAs(Some(mut path)) => {
                if path.extension().is_none() {
                    path.set_extension(file::EXTENSION);
                }
                self.save_to(path);
            }
            Message::Export(format) => {
                let target = self.selection.first().copied().unwrap_or(self.page);
                let bytes = match format {
                    ExportFormat::Png => render_png(self.doc(), target, RenderOptions { scale: 2.0, ..RenderOptions::default() }),
                    ExportFormat::Svg => export_svg(self.doc(), target).map(String::into_bytes).ok_or_else(|| "nothing to export".to_string()),
                };
                let bytes = match bytes {
                    Ok(b) => b,
                    Err(e) => {
                        self.error(e);
                        return Task::none();
                    }
                };
                let name = format!("{}.{}", self.doc().node(target).map_or("Export".into(), |n| n.name.clone()), format.extension());
                return Task::perform(rfd::AsyncFileDialog::new().set_file_name(name).save_file(), move |f| {
                    Message::Exported(f.map(|f| f.path().to_path_buf()), bytes)
                });
            }
            Message::Exported(Some(path), bytes) => match std::fs::write(&path, bytes) {
                Ok(()) => self.info(format!("Exported {}", path.display())),
                Err(e) => self.error(format!("Couldn't export {}: {e}", path.display())),
            },
        }
        Task::none()
    }

    fn on_key(&mut self, event: keyboard::Event) -> Task<Message> {
        match event {
            keyboard::Event::ModifiersChanged(m) => {
                self.modifiers = m;
                return Task::none();
            }
            keyboard::Event::KeyReleased { key: Key::Named(Named::Space), .. } => {
                self.space_held = false;
                return Task::none();
            }
            keyboard::Event::KeyPressed { .. } => {}
            _ => return Task::none(),
        }
        let keyboard::Event::KeyPressed { key, modifiers, .. } = event else { return Task::none() };
        if self.chrome.open_menu.is_some() && matches!(key.as_ref(), Key::Named(Named::Escape)) {
            self.chrome.open_menu = None;
            return Task::none();
        }
        let (cmd, shift, alt) = (modifiers.command(), modifiers.shift(), modifiers.alt());
        let plain = !cmd && !alt;
        let nudge = if shift { 10.0 } else { 1.0 };
        let message = match key.as_ref() {
            Key::Named(Named::Space) if plain => {
                self.space_held = true;
                None
            }
            Key::Named(Named::F11) => Some(Message::Chrome(chrome::Event::ToggleFullscreen)),
            Key::Named(Named::Escape) => {
                // Up one level, like Figma.
                let parent = self.selection.first().and_then(|&s| self.doc().parent(s)).filter(|&p| p != self.page);
                self.selection = parent.into_iter().collect();
                None
            }
            Key::Named(Named::Delete | Named::Backspace) if plain => Some(Message::Delete),
            Key::Named(Named::ArrowLeft) if plain => return self.nudge(-nudge, 0.0),
            Key::Named(Named::ArrowRight) if plain => return self.nudge(nudge, 0.0),
            Key::Named(Named::ArrowUp) if plain => return self.nudge(0.0, -nudge),
            Key::Named(Named::ArrowDown) if plain => return self.nudge(0.0, nudge),
            Key::Character(c) => {
                let c = c.to_lowercase();
                match (c.as_str(), cmd, shift, alt) {
                    ("z", true, false, false) => Some(Message::Undo),
                    ("z", true, true, false) | ("y", true, false, false) => Some(Message::Redo),
                    ("d", true, false, false) => Some(Message::Duplicate),
                    ("a", true, false, false) => Some(Message::SelectAll),
                    ("a", true, true, false) => Some(Message::Deselect),
                    ("q", true, false, false) => Some(Message::Quit),
                    ("s", true, false, false) => Some(Message::Save),
                    ("s", true, true, false) => Some(Message::SaveAs),
                    ("o", true, false, false) => Some(Message::Open),
                    ("n", true, false, false) => Some(Message::New),
                    ("e", true, true, false) => Some(Message::Export(ExportFormat::Png)),
                    ("k", true, false, true) => Some(Message::CreateComponent),
                    ("b", true, false, true) => Some(Message::Detach),
                    ("a", false, true, false) => Some(Message::ToggleAutoLayout),
                    ("=" | "+", true, _, false) => Some(Message::ZoomIn),
                    ("-", true, _, false) => Some(Message::ZoomOut),
                    ("0", false, true, false) | (")", false, true, false) => Some(Message::Zoom100),
                    ("1", false, true, false) | ("!", false, true, false) => Some(Message::ZoomFit),
                    ("2", false, true, false) | ("@", false, true, false) => Some(Message::ZoomSelection),
                    (k, false, false, false) => Tool::from_key(k).map(Message::Tool),
                    _ => None,
                }
            }
            _ => None,
        };
        message.map_or_else(Task::none, |m| self.handle(m))
    }

    // ----- Document changes -------------------------------------------------------------

    /// Applies ops as one undo step. Errors go to the status bar.
    pub fn run(&mut self, label: &str, ops: &[Op]) -> Option<jd_vector::Applied> {
        if ops.is_empty() {
            return None;
        }
        match self.ed.apply(label, ops, &CosmicMeasure) {
            Ok(done) => {
                self.after_change();
                self.status = Status::default();
                Some(done)
            }
            Err(e) => {
                self.error(e.to_string());
                None
            }
        }
    }

    /// Applies ops as part of a drag: the whole drag is one undo step.
    fn run_gesture(&mut self, label: &str, ops: &[Op]) {
        match self.ed.apply_gesture(label, "drag", ops, &CosmicMeasure) {
            Ok(_) => self.after_change(),
            Err(e) => self.error(e.to_string()),
        }
    }

    fn after_change(&mut self) {
        self.version += 1;
        let doc = &self.ed.doc;
        self.selection.retain(|&id| doc.contains(id));
        if !doc.contains(self.page) {
            self.page = doc.pages()[0];
        }
        if self.hover.is_some_and(|h| !doc.contains(h)) {
            self.hover = None;
        }
    }

    fn after_history(&mut self, msg: String) {
        self.after_change();
        self.drafts.clear();
        self.info(msg);
    }

    fn info(&mut self, text: String) {
        self.status = Status { text, error: false };
    }

    fn error(&mut self, text: String) {
        self.status = Status { text, error: true };
    }

    fn patch_selection(&mut self, label: &str, patch: Patch) {
        let ops: Vec<Op> = self.selection.iter().map(|&id| Op::set(id, patch.clone())).collect();
        self.run(label, &ops);
    }

    fn edit_paints(&mut self, fills: bool, f: impl Fn(&mut Vec<Paint>)) {
        let ops: Vec<Op> = self
            .selection
            .iter()
            .filter_map(|&id| {
                let p = &self.doc().node(id)?.props;
                let mut list = if fills { p.fills.clone() } else { p.strokes.clone() };
                f(&mut list);
                Some(Op::set(id, if fills { Patch { fills: Some(list), ..Patch::default() } } else { Patch { strokes: Some(list), ..Patch::default() } }))
            })
            .collect();
        self.run(if fills { "Change fill" } else { "Change stroke" }, &ops);
    }

    fn toggle_auto_layout(&mut self) {
        let ops: Vec<Op> = self
            .selection
            .iter()
            .filter_map(|&id| {
                let n = self.doc().node(id)?;
                if !n.kind.is_frame_like() {
                    return None;
                }
                Some(Op::set(
                    id,
                    match n.props.layout {
                        Some(_) => Patch { layout: Some(None), ..Patch::default() },
                        None => Patch {
                            layout: Some(Some(AutoLayout { gap: 10.0, padding: Padding::all(10.0), ..AutoLayout::default() })),
                            sizing_h: Some(Sizing::Hug),
                            sizing_v: Some(Sizing::Hug),
                            ..Patch::default()
                        },
                    },
                ))
            })
            .collect();
        if ops.is_empty() {
            self.error("Auto layout needs a frame, component or instance selected".into());
        } else {
            self.run("Auto layout", &ops);
        }
    }

    fn nudge(&mut self, dx: f64, dy: f64) -> Task<Message> {
        let ops: Vec<Op> = self
            .selection
            .iter()
            .filter_map(|&id| {
                let p = &self.doc().node(id)?.props;
                Some(Op::set(id, Patch { x: Some(p.x + dx), y: Some(p.y + dy), ..Patch::default() }))
            })
            .collect();
        self.run("Nudge", &ops);
        Task::none()
    }

    /// Applies a typed Design-panel value to the selection.
    fn submit(&mut self, f: Field) {
        let Some(input) = self.drafts.remove(&f) else { return };
        let input = input.trim().to_owned();
        let num = || input.trim_end_matches(['%', '°']).parse::<f64>().ok();
        let Some(&first) = self.selection.first() else {
            if f == Field::PageBackground
                && let Some(c) = Rgba::from_hex(&input)
            {
                self.run("Page background", &[Op::set(self.page, Patch { background: Some(c), ..Patch::default() })]);
            }
            return;
        };
        let node = self.doc().node(first).cloned();
        let Some(node) = node else { return };
        let bad = |app: &mut Self| app.error(format!("{input:?} isn't a valid value"));
        let patch = match &f {
            Field::Name => Patch { name: Some(input.clone()), ..Patch::default() },
            Field::X => match num() {
                Some(v) => Patch { x: Some(v), ..Patch::default() },
                None => return bad(self),
            },
            Field::Y => match num() {
                Some(v) => Patch { y: Some(v), ..Patch::default() },
                None => return bad(self),
            },
            Field::W | Field::H => {
                let Some(v) = num().filter(|v| *v >= 0.0) else { return bad(self) };
                let ops: Vec<Op> = self
                    .selection
                    .iter()
                    .filter_map(|&id| {
                        let p = &self.doc().node(id)?.props;
                        let (w, h) = if f == Field::W { (v, p.height) } else { (p.width, v) };
                        Some(Op::Resize { id: id.into(), width: w, height: h })
                    })
                    .collect();
                self.run("Resize", &ops);
                return;
            }
            Field::Rotation => match num() {
                Some(v) => Patch { rotation: Some(v), ..Patch::default() },
                None => return bad(self),
            },
            Field::Radius => match num().filter(|v| *v >= 0.0) {
                Some(v) => Patch { corner: Some(jd_vector::CornerRadii::all(v)), ..Patch::default() },
                None => return bad(self),
            },
            Field::Opacity => match num() {
                Some(v) => Patch { opacity: Some((v / 100.0).clamp(0.0, 1.0) as f32), ..Patch::default() },
                None => return bad(self),
            },
            Field::Gap | Field::PadH | Field::PadV => {
                let Some(v) = num() else { return bad(self) };
                let Some(mut l) = node.props.layout.clone() else { return };
                match f {
                    Field::Gap => l.gap = v,
                    Field::PadH => (l.padding.left, l.padding.right) = (v, v),
                    _ => (l.padding.top, l.padding.bottom) = (v, v),
                }
                Patch { layout: Some(Some(l)), ..Patch::default() }
            }
            Field::Text => Patch { text: Some(input.replace("\\n", "\n")), ..Patch::default() },
            Field::FontSize | Field::FontStyle => {
                let NodeKind::Text { text } = &node.kind else { return };
                let mut style = text.style.clone();
                if f == Field::FontSize {
                    let Some(v) = num().filter(|v| *v > 0.0) else { return bad(self) };
                    style.size = v as f32;
                } else {
                    style.style = input.clone();
                }
                Patch { text_style: Some(style), ..Patch::default() }
            }
            Field::Fill(i) | Field::Stroke(i) => {
                let Some(c) = Rgba::from_hex(&input) else { return bad(self) };
                let fills = matches!(f, Field::Fill(_));
                let i = *i;
                self.edit_paints(fills, move |list| {
                    if let Some(p) = list.get_mut(i) {
                        p.kind = PaintKind::Solid { color: c };
                    }
                });
                return;
            }
            Field::StrokeWeight => match num().filter(|v| *v >= 0.0) {
                Some(v) => Patch { stroke: Some(jd_vector::StrokeStyle { weight: v as f32, ..node.props.stroke.clone() }), ..Patch::default() },
                None => return bad(self),
            },
            Field::Prop(name) => Patch { values: Some([(name.clone(), PropValue::Text(input.clone()))].into()), ..Patch::default() },
            Field::PageBackground => return,
        };
        self.patch_selection("Edit", patch);
    }

    // ----- Files ------------------------------------------------------------------------

    fn open_path(&mut self, path: PathBuf) {
        match file::load(&path) {
            Ok(mut doc) => {
                // Lay out again with the real fonts (files may come from elsewhere).
                jd_vector::layout::run(&mut doc, &CosmicMeasure);
                self.load(doc, Some(path));
            }
            Err(e) => self.error(format!("Couldn't open {}: {e}", path.display())),
        }
    }

    fn load(&mut self, doc: Document, path: Option<PathBuf>) {
        self.page = doc.pages()[0];
        self.ed = Editor::new(doc);
        self.path = path;
        self.selection.clear();
        self.expanded.clear();
        self.drafts.clear();
        self.hover = None;
        self.version += 1;
        self.fit_pending = true;
        self.zoom_fit(false);
        self.info(match &self.path {
            Some(p) => format!("Opened {}", p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())),
            None => "New document".into(),
        });
    }

    fn save_to(&mut self, path: PathBuf) {
        match file::save(&self.ed.doc, &path) {
            Ok(()) => {
                self.ed.history.mark_saved();
                self.info(format!("Saved {}", path.display()));
                self.path = Some(path);
            }
            Err(e) => self.error(format!("Couldn't save {}: {e}", path.display())),
        }
    }

    // ----- Camera -----------------------------------------------------------------------

    fn zoom_at(&mut self, factor: f64, at: Option<Point>) {
        let at = at.unwrap_or(Point::new(self.viewport.width / 2.0, self.viewport.height / 2.0));
        let anchor = self.camera.to_page(at);
        let zoom = (self.camera.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.camera = Camera { origin: kurbo::Point::new(anchor.x - at.x as f64 / zoom, anchor.y - at.y as f64 / zoom), zoom };
    }

    /// Fits the page's content (or the selection) in the view.
    fn zoom_fit(&mut self, selection: bool) {
        let doc = &self.ed.doc;
        let ids: Vec<NodeId> = if selection && !self.selection.is_empty() { self.selection.clone() } else { doc.children(self.page).to_vec() };
        let Some(b) = ids.iter().filter_map(|&id| doc.world_bounds(id)).reduce(|a, b| a.union(b)) else {
            self.camera = Camera { origin: kurbo::Point::new(-100.0, -100.0), zoom: 1.0 };
            return;
        };
        let (vw, vh) = (self.viewport.width as f64, self.viewport.height as f64);
        if vw < 10.0 || vh < 10.0 {
            return;
        }
        let zoom = ((vw - 80.0) / b.width().max(1.0)).min((vh - 80.0) / b.height().max(1.0)).clamp(MIN_ZOOM, if selection { MAX_ZOOM } else { 1.0 });
        let c = b.center();
        self.camera = Camera { origin: kurbo::Point::new(c.x - vw / 2.0 / zoom, c.y - vh / 2.0 / zoom), zoom };
    }

    fn scroll(&mut self, delta: Vector, at: Point) {
        if self.modifiers.command() {
            self.zoom_at((delta.y as f64 / 300.0).exp(), Some(at));
        } else {
            let (dx, dy) = if self.modifiers.shift() { (delta.y, delta.x) } else { (delta.x, delta.y) };
            let z = self.camera.zoom;
            self.camera.origin = kurbo::Point::new(self.camera.origin.x - dx as f64 / z, self.camera.origin.y - dy as f64 / z);
        }
    }

    /// Re-renders the canvas image if anything it shows changed.
    fn refresh(&mut self) -> Task<Message> {
        let key = (self.version, self.camera, self.viewport, self.scale_factor, self.page);
        if self.rendered == Some(key) {
            return Task::none();
        }
        let (w, h) = (self.viewport.width as f64, self.viewport.height as f64);
        if w < 1.0 || h < 1.0 {
            return Task::none();
        }
        let view = Rect::new(self.camera.origin.x, self.camera.origin.y, self.camera.origin.x + w / self.camera.zoom, self.camera.origin.y + h / self.camera.zoom);
        let background = match self.doc().node(self.page).map(|n| &n.kind) {
            Some(NodeKind::Page { background }) => *background,
            _ => Rgba::from_srgb8(0x1E, 0x1E, 0x1E),
        };
        let opts = RenderOptions { scale: self.camera.zoom * self.scale_factor as f64, background: Some(background) };
        let Some(pm) = render_region(self.doc(), self.page, view, opts) else { return Task::none() };
        // Opaque (the page background covers everything), so premultiplied data is fine.
        let handle = image::Handle::from_rgba(pm.width(), pm.height(), pm.take());
        self.rendered = Some(key);
        self.render_seq += 1;
        let seq = self.render_seq;
        iced_runtime::image::allocate(handle).map(move |r| Message::CanvasReady(seq, r.ok()))
    }

    /// Whether the canvas has been rendered for the current view (for tests).
    #[cfg(test)]
    pub fn rendered(&self) -> bool {
        self.rendered.is_some()
    }

    // ----- Canvas input -----------------------------------------------------------------

    /// The selection box of `id` on screen (axis-aligned page bounds).
    pub fn screen_bounds(&self, id: NodeId) -> Option<iced::Rectangle> {
        let b = self.doc().world_bounds(id)?;
        let a = self.camera.to_screen(kurbo::Point::new(b.x0, b.y0));
        let c = self.camera.to_screen(kurbo::Point::new(b.x1, b.y1));
        Some(iced::Rectangle::new(a, Size::new(c.x - a.x, c.y - a.y)))
    }

    pub fn handle_at(&self, at: Point) -> Option<(NodeId, Handle)> {
        let [id] = self.selection[..] else { return None };
        let n = self.doc().node(id)?;
        // Rotated layers get rotated handles later; for now they resize from the panel.
        if n.props.rotation != 0.0 {
            return None;
        }
        let r = self.screen_bounds(id)?;
        Handle::ALL.into_iter().find_map(|h| {
            let (ax, ay) = h.anchor();
            let p = Point::new(r.x + r.width * ax as f32, r.y + r.height * ay as f32);
            ((p.x - at.x).abs() <= 6.0 && (p.y - at.y).abs() <= 6.0).then_some((id, h))
        })
    }

    /// What a click at `at` selects (see [`pick`]).
    fn pick_at(&self, at: Point, deep: bool, double: bool) -> Option<NodeId> {
        let p = self.camera.to_page(at);
        let hits = self.doc().nodes_at(self.page, p, 4.0 / self.camera.zoom);
        pick(self.doc(), self.page, &hits, &self.selection, deep, double)
    }

    fn select(&mut self, id: NodeId, add: bool) {
        if add {
            if let Some(i) = self.selection.iter().position(|&s| s == id) {
                self.selection.remove(i);
            } else {
                self.selection.push(id);
            }
        } else {
            self.selection = vec![id];
        }
        self.drafts.clear();
        // Show the layer in the Layers panel.
        let mut cur = self.doc().parent(id);
        while let Some(p) = cur.filter(|&p| p != self.page) {
            self.expanded.insert(p);
            cur = self.doc().parent(p);
        }
    }

    fn press(&mut self, at: Point, button: mouse::Button) {
        let page = self.camera.to_page(at);
        if button == mouse::Button::Middle || self.space_held || self.tool == Tool::Hand {
            self.drag = Some(Drag::Pan { start: at, origin: self.camera.origin });
            return;
        }
        if button != mouse::Button::Left {
            return;
        }
        let now = Instant::now();
        let double = self.last_click.is_some_and(|(t, p)| now - t < Duration::from_millis(400) && (p.x - at.x).abs() < 4.0 && (p.y - at.y).abs() < 4.0);
        self.last_click = Some((now, at));
        if self.tool.creates() {
            self.drag = Some(Drag::Create { tool: self.tool, start: page, now: page });
            return;
        }
        if let Some((id, handle)) = self.handle_at(at) {
            let n = self.doc().node(id).expect("selected");
            let to_parent = n.parent.map_or(Affine::IDENTITY, |p| self.doc().world_transform(p).inverse());
            let rect = Rect::new(n.props.x, n.props.y, n.props.x + n.props.width, n.props.y + n.props.height);
            self.drag = Some(Drag::Resize { id, handle, start: page, rect, to_parent, started: false });
            return;
        }
        let shift = self.modifiers.shift();
        match self.pick_at(at, self.modifiers.command(), double) {
            Some(id) => {
                if shift {
                    self.select(id, true);
                } else if !self.selection.contains(&id) {
                    self.select(id, false);
                }
                let items = self.movable();
                self.drag = Some(Drag::Move { start: page, items, started: false });
            }
            None => {
                if !shift {
                    self.selection.clear();
                }
                self.drag = Some(Drag::Marquee { start: page, now: page });
            }
        }
    }

    /// The selected nodes a drag can move (auto layout places the others).
    fn movable(&self) -> Vec<(NodeId, f64, f64, Affine)> {
        let doc = self.doc();
        self.selection
            .iter()
            .filter_map(|&id| {
                let n = doc.node(id)?;
                let parent = n.parent?;
                let p = doc.node(parent)?;
                if p.props.layout.is_some() && !n.props.absolute {
                    return None;
                }
                Some((id, n.props.x, n.props.y, doc.world_transform(parent).inverse()))
            })
            .collect()
    }

    fn drag_to(&mut self, at: Point) {
        let page = self.camera.to_page(at);
        if let Some(Drag::Move { start, started, .. } | Drag::Resize { start, started, .. }) = &mut self.drag
            && !*started
        {
            if (page - *start).hypot() * self.camera.zoom < DRAG_THRESHOLD {
                return;
            }
            *started = true;
        }
        let Some(drag) = self.drag.clone() else {
            let hover = if self.tool == Tool::Move { self.pick_at(at, self.modifiers.command(), false) } else { None };
            self.hover = hover;
            return;
        };
        match drag {
            Drag::Pan { start, origin } => {
                let z = self.camera.zoom;
                self.camera.origin = kurbo::Point::new(origin.x - (at.x - start.x) as f64 / z, origin.y - (at.y - start.y) as f64 / z);
            }
            Drag::Move { start, items, .. } => {
                if items.is_empty() {
                    if !self.selection.is_empty() {
                        self.error("Auto layout places this layer; change its order in the Layers panel".into());
                    }
                    return;
                }
                let d = page - start;
                let ops: Vec<Op> = items
                    .iter()
                    .map(|(id, x, y, to_parent)| {
                        let ld = to_parent.as_coeffs();
                        // Only the linear part applies to a displacement.
                        let dx = ld[0] * d.x + ld[2] * d.y;
                        let dy = ld[1] * d.x + ld[3] * d.y;
                        Op::set(*id, Patch { x: Some((x + dx).round()), y: Some((y + dy).round()), ..Patch::default() })
                    })
                    .collect();
                self.run_gesture("Move", &ops);
            }
            Drag::Resize { id, handle, start, rect, to_parent, .. } => {
                let c = to_parent.as_coeffs();
                let d = page - start;
                let d = Vec2::new(c[0] * d.x + c[2] * d.y, c[1] * d.x + c[3] * d.y);
                let r = resize_rect(rect, handle, d, self.modifiers.shift());
                let ops = [
                    Op::Resize { id: id.into(), width: r.width().round(), height: r.height().round() },
                    Op::set(id, Patch { x: Some(r.x0.round()), y: Some(r.y0.round()), ..Patch::default() }),
                ];
                self.run_gesture("Resize", &ops);
            }
            Drag::Marquee { start, .. } => self.drag = Some(Drag::Marquee { start, now: page }),
            Drag::Create { tool, start, .. } => self.drag = Some(Drag::Create { tool, start, now: page }),
        }
    }

    fn release(&mut self, at: Point) {
        let page = self.camera.to_page(at);
        let Some(drag) = self.drag.take() else { return };
        match drag {
            Drag::Move { .. } | Drag::Resize { .. } => self.ed.end_gesture(),
            Drag::Pan { .. } => {}
            Drag::Marquee { start, .. } => {
                let r = Rect::from_points(start, page);
                if r.width() * self.camera.zoom < 3.0 && r.height() * self.camera.zoom < 3.0 {
                    return;
                }
                // Inside a top-level frame, select its children; otherwise top-level nodes.
                let scope = self
                    .doc()
                    .children(self.page)
                    .iter()
                    .rev()
                    .copied()
                    .find(|&c| self.doc().node(c).is_some_and(|n| n.kind.is_container()) && self.doc().world_bounds(c).is_some_and(|b| b.contains(start)))
                    .unwrap_or(self.page);
                for id in self.doc().children_in_rect(scope, r, false) {
                    if !self.selection.contains(&id) && !self.doc().node(id).is_some_and(|n| n.props.locked) {
                        self.selection.push(id);
                    }
                }
            }
            Drag::Create { tool, start, .. } => self.create(tool, start, page),
        }
    }

    fn create(&mut self, tool: Tool, start: kurbo::Point, end: kurbo::Point) {
        let doc = self.doc();
        let mut r = Rect::from_points(start, end);
        let clicked = r.width() * self.camera.zoom < 3.0 && r.height() * self.camera.zoom < 3.0;
        if clicked {
            r = Rect::from_origin_size(start, (100.0, 100.0));
        }
        // Into the frame under the start point, like Figma.
        let parent = doc
            .nodes_at(self.page, start, 0.0)
            .into_iter()
            .find(|&id| doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Frame | NodeKind::Section | NodeKind::Component { .. })) && !doc.is_inside_instance(id))
            .unwrap_or(self.page);
        let to_local = doc.world_transform(parent).inverse();
        let at = to_local * kurbo::Point::new(r.x0, r.y0);
        let (w, h) = (r.width().round(), r.height().round());
        let n = doc.children(parent).len() + 1;
        let spec = match tool {
            Tool::Frame => NodeSpec::frame(&format!("Frame {n}"), w, h),
            Tool::Rectangle => NodeSpec::rect(&format!("Rectangle {n}"), w, h),
            Tool::Ellipse => NodeSpec::ellipse(&format!("Ellipse {n}"), w, h),
            Tool::Text => NodeSpec::text("Text", "Text", TextStyle { size: 16.0, ..TextStyle::default() }),
            Tool::Move | Tool::Hand => return,
        }
        .at(at.x.round(), at.y.round());
        if let Some(done) = self.run(&format!("Add {}", tool.tip().split(' ').next().unwrap_or("layer")), &[Op::create(parent, spec)]) {
            self.select(done.created[0], false);
            self.tool = Tool::Move;
            if tool == Tool::Text {
                self.info("Type the text in the Design panel".into());
            }
        }
    }

    // ----- For the panels ---------------------------------------------------------------

    /// The current text of a value box: what's being typed, or the value.
    pub fn draft(&self, f: &Field, value: impl FnOnce() -> String) -> String {
        self.drafts.get(f).cloned().unwrap_or_else(value)
    }

    /// Component properties of the selected instance: (name, definition, current value).
    pub fn instance_props(&self, id: NodeId) -> Vec<(String, PropDef, PropValue)> {
        let doc = self.doc();
        let Some(NodeKind::Instance { component, values, .. }) = doc.node(id).map(|n| &n.kind) else { return Vec::new() };
        let Some(NodeKind::Component { properties, .. }) = doc.node(*component).map(|n| &n.kind) else { return Vec::new() };
        properties
            .iter()
            .map(|(k, d)| (k.clone(), d.clone(), values.get(k).cloned().unwrap_or_else(|| d.default_value())))
            .collect()
    }
}

/// What clicking selects, given the hits under the pointer (topmost, deepest first):
/// - with `deep` (Ctrl), the deepest layer;
/// - otherwise a child of a top-level frame (frames act as artboards), or the top-level node;
/// - staying at the level of the current selection: a sibling of a selected layer is picked
///   directly;
/// - a double-click goes one level deeper than the current selection.
pub fn pick(doc: &Document, page: NodeId, hits: &[NodeId], selection: &[NodeId], deep: bool, double: bool) -> Option<NodeId> {
    let &h = hits.first()?;
    if deep {
        return Some(h);
    }
    let mut chain = vec![h];
    let mut cur = doc.parent(h);
    while let Some(c) = cur.filter(|&c| c != page) {
        chain.push(c);
        cur = doc.parent(c);
    }
    // chain: deepest first, top-level last.
    if double && let Some(i) = chain.iter().position(|c| selection.contains(c)) {
        return Some(chain[i.saturating_sub(1)]);
    }
    for (i, &c) in chain.iter().enumerate() {
        let parent = doc.parent(c);
        if selection.iter().any(|&s| s != c && doc.parent(s) == parent) || selection.contains(&c) {
            return Some(chain[i]);
        }
    }
    let top = *chain.last()?;
    let artboard = doc.node(top).is_some_and(|n| n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section));
    if artboard && chain.len() >= 2 { Some(chain[chain.len() - 2]) } else { Some(top) }
}

/// A box after dragging one of its handles by `d` (parent space); `keep_ratio` with Shift.
pub fn resize_rect(r: Rect, h: Handle, d: Vec2, keep_ratio: bool) -> Rect {
    let (ax, ay) = h.anchor();
    let (mut x0, mut y0, mut x1, mut y1) = (r.x0, r.y0, r.x1, r.y1);
    if ax == 0.0 {
        x0 += d.x;
    } else if ax == 1.0 {
        x1 += d.x;
    }
    if ay == 0.0 {
        y0 += d.y;
    } else if ay == 1.0 {
        y1 += d.y;
    }
    let mut out = Rect::new(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
    if keep_ratio && r.height() > 0.0 && ax != 0.5 && ay != 0.5 {
        let ratio = r.width() / r.height();
        let w = out.width().max(out.height() * ratio);
        let h = w / ratio;
        out = Rect::from_origin_size((if ax == 0.0 { out.x1 - w } else { out.x0 }, if ay == 0.0 { out.y1 - h } else { out.y0 }), (w, h));
    }
    out
}

#[cfg(test)]
mod tests {
    use jd_vector::{ApproxMeasure, NodeSpec};

    use super::*;

    fn app() -> Blobr {
        let (mut app, _) = Blobr::boot();
        app.viewport = Size::new(800.0, 600.0);
        app.camera = Camera { origin: kurbo::Point::new(0.0, 0.0), zoom: 1.0 };
        app
    }

    #[test]
    fn drawing_a_frame_then_a_rectangle_inside_it() {
        let mut app = app();
        let _ = app.update(Message::Tool(Tool::Frame));
        let _ = app.update(Message::CanvasPress { at: Point::new(10.0, 10.0), button: mouse::Button::Left });
        let _ = app.update(Message::CanvasMove(Point::new(210.0, 160.0)));
        let _ = app.update(Message::CanvasRelease(Point::new(210.0, 160.0)));
        let frame = app.selection[0];
        let f = app.doc().node(frame).unwrap();
        assert_eq!((f.props.x, f.props.y, f.props.width, f.props.height), (10.0, 10.0, 200.0, 150.0));
        assert_eq!(app.tool, Tool::Move, "back to Move after drawing");

        let _ = app.update(Message::Tool(Tool::Rectangle));
        let _ = app.update(Message::CanvasPress { at: Point::new(30.0, 30.0), button: mouse::Button::Left });
        let _ = app.update(Message::CanvasRelease(Point::new(30.0, 30.0)));
        let rect = app.selection[0];
        assert_eq!(app.doc().parent(rect), Some(frame), "drawn into the frame under the pointer");
        assert_eq!(app.doc().node(rect).unwrap().props.x, 20.0, "positioned in the frame's space");
        assert!(app.rendered());
    }

    #[test]
    fn a_drag_moves_and_is_one_undo_step() {
        let mut app = app();
        let p = app.page;
        let r = app.run("Add", &[Op::create(p, NodeSpec::rect("R", 50.0, 50.0).at(100.0, 100.0))]).unwrap().created[0];
        let _ = app.update(Message::CanvasPress { at: Point::new(120.0, 120.0), button: mouse::Button::Left });
        assert_eq!(app.selection, vec![r]);
        for i in 1..=10 {
            let _ = app.update(Message::CanvasMove(Point::new(120.0 + i as f32 * 3.0, 120.0)));
        }
        let _ = app.update(Message::CanvasRelease(Point::new(150.0, 120.0)));
        assert_eq!(app.doc().node(r).unwrap().props.x, 130.0);
        let _ = app.update(Message::Undo);
        assert_eq!(app.doc().node(r).unwrap().props.x, 100.0, "the whole drag undid at once");
    }

    #[test]
    fn clicks_pick_artboard_children_and_double_click_goes_deeper() {
        let mut doc = Document::new();
        let page = doc.pages()[0];
        let done = doc
            .apply(
                &[
                    Op::create_as("screen", page, NodeSpec::frame("Screen", 300.0, 300.0)),
                    Op::create_as("card", "screen", NodeSpec::frame("Card", 200.0, 200.0).at(10.0, 10.0)),
                    Op::create_as("dot", "card", NodeSpec::ellipse("Dot", 20.0, 20.0).at(10.0, 10.0)),
                ],
                &ApproxMeasure,
            )
            .unwrap();
        let id = |k| done.id(k).unwrap();
        let hits = doc.nodes_at(page, kurbo::Point::new(25.0, 25.0), 0.0);
        assert_eq!(pick(&doc, page, &hits, &[], false, false), Some(id("card")));
        assert_eq!(pick(&doc, page, &hits, &[], true, false), Some(id("dot")), "Ctrl picks the deepest");
        assert_eq!(pick(&doc, page, &hits, &[id("card")], false, true), Some(id("dot")), "double-click goes in");
        assert_eq!(pick(&doc, page, &hits, &[id("dot")], false, false), Some(id("dot")), "stays at that level");
    }

    #[test]
    fn resize_handles_and_typed_values() {
        let r = resize_rect(Rect::new(0.0, 0.0, 100.0, 50.0), Handle::TopLeft, Vec2::new(10.0, 10.0), false);
        assert_eq!(r, Rect::new(10.0, 10.0, 100.0, 50.0));
        let flipped = resize_rect(Rect::new(0.0, 0.0, 100.0, 50.0), Handle::Right, Vec2::new(-150.0, 0.0), false);
        assert_eq!(flipped, Rect::new(-50.0, 0.0, 0.0, 50.0));

        let mut app = app();
        let p = app.page;
        let r = app.run("Add", &[Op::create(p, NodeSpec::rect("R", 50.0, 50.0))]).unwrap().created[0];
        let _ = app.update(Message::Select(r, false));
        let _ = app.update(Message::FieldInput(Field::W, "120".into()));
        let _ = app.update(Message::FieldSubmit(Field::W));
        let _ = app.update(Message::FieldInput(Field::Fill(0), "#4C8DFF".into()));
        let _ = app.update(Message::FieldSubmit(Field::Fill(0)));
        let n = app.doc().node(r).unwrap();
        assert_eq!(n.props.width, 120.0);
        assert_eq!(n.props.fills, vec![Paint::hex("#4C8DFF")]);
        let _ = app.update(Message::FieldInput(Field::X, "abc".into()));
        let _ = app.update(Message::FieldSubmit(Field::X));
        assert!(app.status.error);
    }

    #[test]
    fn half_typed_values_and_click_jitter_dont_leak() {
        let mut app = app();
        let p = app.page;
        let r = app.run("Add", &[Op::create(p, NodeSpec::rect("R", 50.0, 50.0).at(100.0, 100.0))]).unwrap().created[0];
        let _ = app.update(Message::Select(r, false));
        let _ = app.update(Message::FieldInput(Field::W, "999".into()));
        let _ = app.update(Message::Deselect);
        assert!(app.drafts.is_empty(), "a draft for one layer must not show on the next");

        // A click with a pixel of jitter selects without moving or adding an undo step.
        let steps = app.ed.history.labels().count();
        let _ = app.update(Message::CanvasPress { at: Point::new(120.0, 120.0), button: mouse::Button::Left });
        let _ = app.update(Message::CanvasMove(Point::new(121.0, 120.0)));
        let _ = app.update(Message::CanvasRelease(Point::new(121.0, 120.0)));
        assert_eq!(app.doc().node(r).unwrap().props.x, 100.0);
        assert_eq!(app.ed.history.labels().count(), steps);
        assert_eq!(app.selection, vec![r]);
    }

    #[test]
    fn errors_go_to_the_status_bar_and_save_round_trips() {
        let mut app = app();
        let p = app.page;
        let done = app.run("Add", &[Op::create(p, NodeSpec::rect("R", 10.0, 10.0))]).unwrap();
        // Rectangles can't hold children: the op fails, nothing changes, the status says why.
        assert!(app.run("Bad", &[Op::create(done.created[0], NodeSpec::rect("X", 1.0, 1.0))]).is_none());
        assert!(app.status.error && app.status.text.contains("can't hold"), "{}", app.status.text);

        let dir = std::env::temp_dir().join(format!("blobr-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jdb");
        let _ = app.update(Message::SavedAs(Some(path.clone())));
        assert!(!app.ed.history.is_dirty());
        let mut other = self::app();
        let _ = other.update(Message::Opened(Some(path)));
        assert_eq!(other.doc().len(), app.doc().len());
        let _ = std::fs::remove_dir_all(dir);
    }
}
