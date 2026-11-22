//! Blobr's state and everything that changes it. Every document change goes through
//! `jd_vector` ops (`run` / `run_gesture`), so it's undoable and validated; the canvas is a
//! `jd-vector-render` image redrawn whenever the document, camera or window changes.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::keyboard::{self, Key, key::Named};
use iced::widget::{image, operation, text_editor};
use iced::{Point, Size, Subscription, Task, Vector, mouse, window};
use iced_runtime::image::Allocation;
use jd_core::Rgba;
use jd_ui::chrome::{self, WindowChrome};
use jd_vector::arrange::{self, Align, Order};
use jd_vector::boolean::{self, BoolOp};
use jd_vector::clip::Clip;
use jd_vector::snap::{self, Edges, Line};
use jd_vector::{
    AutoLayout, Direction, Document, Editor, ExportFormat, NodeId, NodeKind, NodeSpec, Op, Padding, Paint, PaintKind, Patch, PropDef,
    PropValue, Sizing, StyleKind, TextStyle, VarValue, file,
};
use jd_vector_render::{CosmicMeasure, RenderOptions, export_svg, render_png, render_region};
use kurbo::{Affine, Rect, Vec2};

use crate::clipboard::{self, Pasted};
use crate::pen::{self, Part, PenPath, VectorEdit};
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
    /// `bounds` is the selection's page box at the start, `targets` what it can snap to.
    Move { start: kurbo::Point, items: Vec<(NodeId, f64, f64, Affine)>, started: bool, bounds: Rect, targets: Vec<Rect> },
    Resize { id: NodeId, handle: Handle, start: kurbo::Point, rect: Rect, to_parent: Affine, started: bool, targets: Vec<Rect> },
    Marquee { start: kurbo::Point, now: kurbo::Point },
    /// Turning a layer about its centre: the centre on the page and in the parent, the
    /// pointer's starting angle, the starting rotation and half the layer's size.
    Rotate { id: NodeId, center_page: kurbo::Point, center_parent: kurbo::Point, a0: f64, rot0: f64, half: Vec2 },
    /// A vector point or handle (subpath, anchor, which part).
    Anchor { id: NodeId, sub: usize, idx: usize, part: Part },
    Create { tool: Tool, start: kurbo::Point, now: kurbo::Point },
}

/// What a press-and-drag in the Layers panel will do on release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drop {
    Above,
    Below,
    Inside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerDrag {
    pub id: NodeId,
    pub target: Option<(NodeId, Drop)>,
}

/// Another .jdb file whose components can be placed here.
pub struct Library {
    pub path: PathBuf,
    pub name: String,
    pub doc: Document,
    pub thumbnails: HashMap<NodeId, image::Handle>,
}

/// A frame rendered for presenting: picture, scale it was drawn at, size on the page.
#[derive(Debug, Clone)]
pub struct Shot {
    pub image: image::Handle,
    pub fit: f64,
    pub size: kurbo::Size,
}

/// The prototype being presented.
#[derive(Debug, Clone)]
pub struct Presentation {
    pub frame: NodeId,
    pub shot: Option<Shot>,
    pub overlay: Option<(NodeId, Shot)>,
    /// The frame we came from, sliding or fading out.
    pub leaving: Option<(Shot, jd_vector::Transition, Instant, f32)>,
    pub history: Vec<NodeId>,
    pub entered: Instant,
    pub delay_done: bool,
    pub cursor: Point,
    pub hovered: Option<NodeId>,
}

/// Name of the page that holds components copied in from libraries.
pub const LIBRARY_PAGE: &str = "Library components";

/// Placeable components of `doc`: plain components and component sets (not the variants
/// inside a set), with the page each is on.
pub fn placeable_components(doc: &Document) -> Vec<(NodeId, NodeId)> {
    let mut out = Vec::new();
    for &p in doc.pages() {
        for id in doc.descendants(p) {
            let Some(n) = doc.node(id) else { continue };
            let in_set = n.parent.and_then(|q| doc.node(q)).is_some_and(|q| matches!(q.kind, NodeKind::ComponentSet { .. }));
            let placeable = matches!(n.kind, NodeKind::ComponentSet { .. }) || (matches!(n.kind, NodeKind::Component { .. }) && !in_set);
            if placeable && !doc.is_inside_instance(id) {
                out.push((id, p));
            }
        }
    }
    out
}

/// Small pictures of a document's components.
fn thumbnails_of(doc: &Document, scale_factor: f32) -> HashMap<NodeId, image::Handle> {
    let mut out = HashMap::new();
    for (id, _) in placeable_components(doc) {
        let Some(n) = doc.node(id) else { continue };
        let scale = (40.0 / n.props.width.max(n.props.height).max(1.0)).min(2.0) * scale_factor as f64;
        if let Some(pm) = jd_vector_render::render_node(doc, id, RenderOptions { scale, ..RenderOptions::default() }) {
            out.insert(id, image::Handle::from_rgba(pm.width(), pm.height(), clipboard::to_rgba(&pm)));
        }
    }
    out
}

/// How a layer inside a component follows a component property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropLink {
    /// A text property sets the characters.
    Text,
    /// A boolean property shows or hides it.
    Visible,
    /// An instance-swap property picks the component.
    Swap,
}

/// What to do once unsaved changes are dealt with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum After {
    Quit,
    New,
    Open,
}

/// Id of the on-canvas text editor.
pub const TEXT_EDITOR: &str = "canvas-text";

/// Id of the layer-name box in the Layers panel.
pub const RENAME_INPUT: &str = "layer-rename";

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
    /// A component's text property default.
    PropDefault(String),
    VarName(NodeId),
    /// A part of reaction `index`: "delay", "duration" or "url".
    Reaction(usize, &'static str),
    /// A part of effect `index`: "x", "y", "blur", "spread", "color" or "radius".
    Effect(usize, &'static str),
    VarValue(NodeId, String),
    StyleName(NodeId),
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
    EditText(NodeId),
    TextAction(text_editor::Action),
    EndTextEdit,
    Copy,
    Cut,
    Paste,
    Pasted(Option<Pasted>),
    CopyAsPng,
    Align(Align),
    Boolean(BoolOp),
    LeftTab(usize),
    /// The right panel's tab: 0 Design, 1 Prototype, 2 Inspect.
    RightTab(usize),
    AddReaction,
    RemoveReaction(usize),
    /// "click", "hover" or "delay".
    SetTrigger(usize, &'static str),
    /// "navigate", "overlay", "back" or "url".
    SetAction(usize, &'static str),
    SetTarget(usize, NodeId),
    SetTransition(usize, jd_vector::Transition),
    /// Present the prototype from a frame (None: the selection's frame, or the first).
    Present(Option<NodeId>),
    StopPresenting,
    PresentMove(Point),
    PresentClick,
    PresentTick,
    /// Next (+1) or previous (-1) top-level frame while presenting.
    PresentStep(i32),
    CopyText(String),
    /// The Local variables dialog.
    ShowVariables(bool),
    SelectCollection(NodeId),
    AddCollection,
    AddMode,
    /// Add a variable of a type: "color", "number", "text" or "bool".
    AddVariable(&'static str),
    DeleteToken(NodeId),
    SetVarBool(NodeId, String, bool),
    /// Bind fill `index` of the selection to a colour variable (None unbinds).
    BindFill(usize, Option<NodeId>),
    /// Bind a property path (`gap`, `corner`, ...) to a variable (None unbinds).
    Bind(String, Option<NodeId>),
    AddEffect,
    RemoveEffect(usize),
    /// Change an effect's type: "drop", "inner", "blur" or "background".
    SetEffectKind(usize, &'static str),
    UseEffectStyle(Option<NodeId>),
    CreateEffectStyle,
    /// Set a component property on the selected instance.
    SetPropValue(String, PropValue),
    ImportSvg,
    SvgPicked(Option<PathBuf>),
    /// The selection (or every frame on the page) as a PDF, a page each.
    ExportPdf,
    AddLibrary,
    LibraryPicked(Option<PathBuf>),
    RemoveLibrary(usize),
    /// Place an instance of a library's component (copied into this file once).
    InsertLibraryComponent(usize, NodeId),
    /// Use a paint style for the fills, or a text style (None detaches).
    UseFillStyle(Option<NodeId>),
    UseTextStyle(Option<NodeId>),
    /// Make a style from the selection's fills or text.
    CreateFillStyle,
    CreateTextStyle,
    /// Pick a frame's mode for a collection (None: inherit).
    SetMode(NodeId, Option<String>),
    AssetFilter(String),
    /// Place an instance of this component.
    InsertComponent(NodeId),
    CombineVariants,
    SwapComponent(NodeId),
    /// Add a component property: "text" or "boolean".
    AddProperty(&'static str),
    RemoveProperty(String),
    SetPropertyDefault(String, PropValue),
    /// Link the selected layer (inside a component) to a property: which link, and the
    /// property's name (None unlinks).
    LinkProperty(PropLink, Option<String>),
    Flatten,
    Distribute(bool),
    Group,
    FrameSelection,
    Ungroup,
    Reorder(Order),
    LayerPress(NodeId),
    LayerHover(NodeId, f32),
    LayerRelease,
    StartRename(NodeId),
    RenameInput(String),
    RenameSubmit,
    /// The unsaved-changes dialog: save first, discard, or cancel.
    ConfirmSave,
    ConfirmDiscard,
    ConfirmCancel,
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
    /// Lines to draw while a drag snaps.
    pub guides: Vec<Line>,
    pub layer_drag: Option<LayerDrag>,
    pub renaming: Option<(NodeId, String)>,
    /// The unsaved-changes dialog is open for this.
    pub confirm: Option<After>,
    /// Continue with this once the Save As dialog has saved.
    after_save: Option<After>,
    /// The last copy, for when the system clipboard can't be read.
    clip: Option<Clip>,
    /// A text layer being typed into on the canvas.
    pub text_edit: Option<(NodeId, text_editor::Content)>,
    /// A path being drawn with the Pen.
    pub pen: Option<PenPath>,
    /// A vector whose points are being edited.
    pub vector_edit: Option<VectorEdit>,
    /// The left panel's tab: 0 Layers, 1 Assets.
    pub left_tab: usize,
    pub right_tab: usize,
    pub present: Option<Presentation>,
    /// The Local variables dialog is open, showing this collection.
    pub variables_open: bool,
    pub var_collection: Option<NodeId>,
    pub asset_filter: String,
    /// Small pictures of components for the Assets panel, and the version they show.
    pub thumbnails: HashMap<NodeId, image::Handle>,
    thumbs_version: u64,
    pub libraries: Vec<Library>,
    /// Library components already copied in: (library file, its id) to the local id.
    imported: HashMap<(PathBuf, NodeId), NodeId>,
    last_click: Option<(Instant, Point)>,
    fit_pending: bool,
}

const MIN_ZOOM: f64 = 0.02;
/// Screen pixels the pointer must travel before a press becomes a drag.
const DRAG_THRESHOLD: f64 = 3.0;
/// Screen pixels within which edges snap.
const SNAP: f64 = 6.0;

/// A page-space displacement in a parent's space (`c`: that map's coefficients).
fn local_vec(c: [f64; 6], d: Vec2) -> Vec2 {
    Vec2::new(c[0] * d.x + c[2] * d.y, c[1] * d.x + c[3] * d.y)
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
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
            guides: Vec::new(),
            layer_drag: None,
            renaming: None,
            confirm: None,
            after_save: None,
            clip: None,
            text_edit: None,
            pen: None,
            vector_edit: None,
            left_tab: 0,
            right_tab: 0,
            present: None,
            variables_open: false,
            var_collection: None,
            asset_filter: String::new(),
            thumbnails: HashMap::new(),
            thumbs_version: u64::MAX,
            libraries: Vec::new(),
            imported: HashMap::new(),
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
            if self.present_animating() { window::frames().map(|_| Message::PresentTick) } else { Subscription::none() },
        ])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        // Anything but typing (and redraws) finishes the text being typed first.
        if self.text_edit.is_some()
            && !matches!(
                message,
                Message::TextAction(_)
                    | Message::EditText(_)
                    | Message::CanvasPress { .. }
                    | Message::CanvasMove(_)
                    | Message::CanvasRelease(_)
                    | Message::CanvasScroll { .. }
                    | Message::Frame
                    | Message::CanvasReady(..)
                    | Message::Viewport(_)
                    | Message::ScaleFactor(_)
                    | Message::Chrome(_)
                    | Message::Key(_)
                    | Message::Noop
            )
        {
            self.commit_text();
        }
        let before = self.selection.clone();
        let task = self.handle(message);
        // Half-typed values belong to the layer they were typed for.
        if self.selection != before {
            self.drafts.clear();
        }
        // While dragging, mouse moves can outpace the screen: render once per frame instead.
        self.refresh_thumbnails();
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
            Message::Quit => return self.guard(After::Quit),
            Message::ConfirmCancel => self.confirm = None,
            Message::ConfirmDiscard => {
                if let Some(a) = self.confirm.take() {
                    return self.proceed(a);
                }
            }
            Message::ConfirmSave => {
                let Some(a) = self.confirm.take() else { return Task::none() };
                match self.path.clone() {
                    Some(path) => {
                        self.save_to(path);
                        if !self.ed.history.is_dirty() {
                            return self.proceed(a);
                        }
                    }
                    None => {
                        self.after_save = Some(a);
                        return self.handle(Message::SaveAs);
                    }
                }
            }
            Message::Copy => {
                self.copy();
            }
            Message::Cut => {
                if self.copy() {
                    return self.handle(Message::Delete);
                }
            }
            Message::Paste => return Task::perform(clipboard::read_async(), Message::Pasted),
            Message::Pasted(p) => match p {
                Some(Pasted::Layers(c)) => self.paste(*c),
                Some(Pasted::Image { width, height, png }) => self.paste(Clip::image("Image", width as f64, height as f64, png)),
                Some(Pasted::Text(t)) if !t.trim().is_empty() => self.paste_text(t),
                _ => match self.clip.clone() {
                    Some(c) => self.paste(c),
                    None => self.info("Nothing to paste".into()),
                },
            },
            Message::CopyAsPng => {
                let Some(&id) = self.selection.first() else { return Task::none() };
                match jd_vector_render::render_node(self.doc(), id, RenderOptions { scale: 2.0, ..RenderOptions::default() }) {
                    Some(pm) => {
                        clipboard::put_image(pm.width(), pm.height(), clipboard::to_rgba(&pm));
                        self.info("Copied as PNG (2x)".into());
                    }
                    None => self.error("Nothing to copy: the layer is empty or hidden".into()),
                }
            }
            Message::LeftTab(t) => self.left_tab = t,
            Message::RightTab(t) => self.right_tab = t,
            Message::AddReaction => {
                let doc = self.doc();
                let Some(&id) = self.selection.first() else { return Task::none() };
                // To the next top-level frame after this layer's own, as a start.
                let own = self.top_frame(id);
                let frames = self.top_frames();
                let next = frames.iter().position(|&f| Some(f) == own).map_or(frames.first().copied(), |i| frames.get((i + 1) % frames.len()).copied());
                let action = match next.filter(|&n| Some(n) != own || frames.len() == 1) {
                    Some(to) => jd_vector::Action::Navigate { to },
                    None => jd_vector::Action::Back,
                };
                let _ = doc;
                self.edit_reactions("Add interaction", move |r| {
                    r.push(jd_vector::Reaction { trigger: jd_vector::Trigger::Click, action: action.clone(), transition: jd_vector::Transition::Instant, duration: 0.3 })
                });
            }
            Message::RemoveReaction(i) => self.edit_reactions("Remove interaction", |r| {
                if i < r.len() {
                    r.remove(i);
                }
            }),
            Message::SetTrigger(i, t) => self.edit_reactions("Interaction trigger", |r| {
                if let Some(x) = r.get_mut(i) {
                    x.trigger = match t {
                        "hover" => jd_vector::Trigger::Hover,
                        "delay" => jd_vector::Trigger::AfterDelay { seconds: 1.0 },
                        _ => jd_vector::Trigger::Click,
                    };
                }
            }),
            Message::SetAction(i, a) => {
                let fallback = self.top_frames().first().copied();
                self.edit_reactions("Interaction action", move |r| {
                    let Some(x) = r.get_mut(i) else { return };
                    let target = match &x.action {
                        jd_vector::Action::Navigate { to } | jd_vector::Action::Overlay { to } => Some(*to),
                        _ => fallback,
                    };
                    x.action = match (a, target) {
                        ("navigate", Some(to)) => jd_vector::Action::Navigate { to },
                        ("overlay", Some(to)) => jd_vector::Action::Overlay { to },
                        ("url", _) => jd_vector::Action::OpenUrl { url: "https://".into() },
                        _ => jd_vector::Action::Back,
                    };
                })
            }
            Message::SetTarget(i, to) => self.edit_reactions("Interaction destination", |r| {
                if let Some(x) = r.get_mut(i) {
                    match &mut x.action {
                        jd_vector::Action::Navigate { to: t } | jd_vector::Action::Overlay { to: t } => *t = to,
                        _ => x.action = jd_vector::Action::Navigate { to },
                    }
                }
            }),
            Message::SetTransition(i, t) => self.edit_reactions("Interaction transition", |r| {
                if let Some(x) = r.get_mut(i) {
                    x.transition = t;
                }
            }),
            Message::Present(from) => self.start_presenting(from),
            Message::StopPresenting => self.present = None,
            Message::PresentMove(at) => self.present_move(at),
            Message::PresentClick => self.present_click(),
            Message::PresentTick => self.present_tick(),
            Message::PresentStep(d) => {
                let frames = self.top_frames();
                if let Some(p) = &self.present
                    && let Some(i) = frames.iter().position(|&f| f == p.frame)
                {
                    let j = (i as i32 + d).rem_euclid(frames.len() as i32) as usize;
                    self.go_to(frames[j], jd_vector::Transition::Instant, 0.0, true);
                }
            }
            Message::CopyText(t) => {
                clipboard::put_text(t);
                self.info("Copied".into());
            }
            Message::ShowVariables(open) => {
                self.variables_open = open;
                if self.var_collection.is_none_or(|c| !self.doc().tokens().collections.contains_key(&c)) {
                    self.var_collection = self.doc().tokens().collections.keys().next().copied();
                }
            }
            Message::SelectCollection(c) => self.var_collection = Some(c),
            Message::AddCollection => {
                let n = self.doc().tokens().collections.len() + 1;
                let op = Op::CreateCollection { name: format!("Collection {n}"), modes: vec!["Mode 1".into()], key: Some("c".into()) };
                if let Some(done) = self.run("Add collection", &[op]) {
                    self.var_collection = done.id("c");
                }
            }
            Message::AddMode => {
                if let Some(c) = self.var_collection {
                    let n = self.doc().tokens().collections.get(&c).map_or(1, |c| c.modes.len() + 1);
                    self.run("Add mode", &[Op::SetCollection { id: c.into(), name: None, add_mode: Some(format!("Mode {n}")) }]);
                }
            }
            Message::AddVariable(kind) => {
                if self.var_collection.is_none() {
                    let _ = self.handle(Message::AddCollection);
                }
                let Some(c) = self.var_collection else { return Task::none() };
                let Some(col) = self.doc().tokens().collections.get(&c) else { return Task::none() };
                let value = match kind {
                    "color" => VarValue::Color(Rgba::from_hex("#4C8DFF").expect("valid")),
                    "number" => VarValue::Number(8.0),
                    "text" => VarValue::Text("Text".into()),
                    _ => VarValue::Bool(true),
                };
                let label = match kind {
                    "color" => "Color",
                    "number" => "Number",
                    "text" => "Text",
                    _ => "Boolean",
                };
                let n = (1..)
                    .find(|i| !self.doc().tokens().variables.values().any(|v| v.collection == c && v.name == format!("{label} {i}")))
                    .expect("endless");
                let values = [(col.modes[0].clone(), value)].into();
                self.run("Add variable", &[Op::CreateVariable { collection: c.into(), name: format!("{label} {n}"), values, key: None }]);
            }
            Message::DeleteToken(id) => {
                self.run("Delete", &[Op::DeleteToken { id: id.into() }]);
                if self.var_collection == Some(id) {
                    self.var_collection = self.doc().tokens().collections.keys().next().copied();
                }
            }
            Message::SetVarBool(id, mode, b) => {
                self.run("Set variable", &[Op::SetVariable { id: id.into(), name: None, mode: Some(mode), value: Some(VarValue::Bool(b)) }]);
            }
            Message::BindFill(i, var) => {
                let key = format!("fills/{i}");
                self.per_node("Bind variable", |n| {
                    (n.props.fills.len() > i || var.is_none()).then(|| Patch { bind: Some([(key.clone(), var)].into()), ..Patch::default() })
                });
            }
            Message::Bind(path, var) => self.per_node("Bind variable", |_| Some(Patch { bind: Some([(path.clone(), var)].into()), ..Patch::default() })),
            Message::AddEffect => self.edit_effects("Add effect", |e| {
                e.push(jd_vector::Effect::DropShadow(jd_vector::Shadow {
                    color: Rgba::new(0.0, 0.0, 0.0, 0.25),
                    x: 0.0,
                    y: 4.0,
                    blur: 4.0,
                    spread: 0.0,
                    visible: true,
                }))
            }),
            Message::RemoveEffect(i) => self.edit_effects("Remove effect", |e| {
                if i < e.len() {
                    e.remove(i);
                }
            }),
            Message::SetEffectKind(i, kind) => self.edit_effects("Effect type", |e| {
                use jd_vector::Effect as E;
                let Some(cur) = e.get(i).cloned() else { return };
                let shadow = match &cur {
                    E::DropShadow(s) | E::InnerShadow(s) => s.clone(),
                    E::LayerBlur { radius, .. } | E::BackgroundBlur { radius, .. } => {
                        jd_vector::Shadow { color: Rgba::new(0.0, 0.0, 0.0, 0.25), x: 0.0, y: 4.0, blur: *radius, spread: 0.0, visible: true }
                    }
                };
                let radius = match &cur {
                    E::LayerBlur { radius, .. } | E::BackgroundBlur { radius, .. } => *radius,
                    E::DropShadow(s) | E::InnerShadow(s) => s.blur.max(4.0),
                };
                e[i] = match kind {
                    "drop" => E::DropShadow(shadow),
                    "inner" => E::InnerShadow(shadow),
                    "blur" => E::LayerBlur { radius, visible: true },
                    _ => E::BackgroundBlur { radius, visible: true },
                };
            }),
            Message::UseEffectStyle(style) => self.per_node("Effect style", |n| {
                let mut refs = n.props.styles.clone();
                refs.effect = style;
                Some(Patch { styles: Some(refs), ..Patch::default() })
            }),
            Message::CreateEffectStyle => {
                let Some(n) = self.selection.first().and_then(|&id| self.doc().node(id)) else { return Task::none() };
                if n.props.effects.is_empty() {
                    return Task::none();
                }
                let count = self.doc().tokens().styles.len() + 1;
                let op = Op::CreateStyle { name: format!("Effect {count}"), style: StyleKind::Effect { effects: n.props.effects.clone() }, key: Some("s".into()) };
                if let Some(done) = self.run("Create style", &[op])
                    && let Some(sid) = done.id("s")
                {
                    let _ = self.handle(Message::UseEffectStyle(Some(sid)));
                    self.ed.history.merge_last();
                }
            }
            Message::SetPropValue(name, v) => self.patch_selection("Set property", Patch { values: Some([(name, v)].into()), ..Patch::default() }),
            Message::ImportSvg => {
                return Task::perform(
                    rfd::AsyncFileDialog::new().add_filter("SVG", &["svg"]).set_title("Import SVG").pick_file(),
                    |f| Message::SvgPicked(f.map(|f| f.path().to_path_buf())),
                );
            }
            Message::SvgPicked(Some(path)) => match std::fs::read(&path) {
                Ok(data) => {
                    let name = path.file_stem().map_or("SVG".into(), |n| n.to_string_lossy().into_owned());
                    self.import_svg(&data, &name);
                }
                Err(e) => self.error(format!("Couldn't read {}: {e}", path.display())),
            },
            Message::SvgPicked(None) => {}
            Message::ExportPdf => {
                let ids = if self.selection.is_empty() { self.top_frames() } else { self.selection.clone() };
                if ids.is_empty() {
                    self.error("Nothing to export: add a frame".into());
                    return Task::none();
                }
                let bytes = match jd_vector_render::export_pdf(self.doc(), &ids) {
                    Ok(b) => b,
                    Err(e) => {
                        self.error(e);
                        return Task::none();
                    }
                };
                let name = format!("{}.pdf", self.path.as_ref().and_then(|p| p.file_stem()).map_or("Untitled".into(), |n| n.to_string_lossy().into_owned()));
                return Task::perform(rfd::AsyncFileDialog::new().set_file_name(name).save_file(), move |f| Message::Exported(f.map(|f| f.path().to_path_buf()), bytes));
            }
            Message::AddLibrary => {
                return Task::perform(
                    rfd::AsyncFileDialog::new().add_filter("Blobr document", &[file::EXTENSION]).set_title("Add a library").pick_file(),
                    |f| Message::LibraryPicked(f.map(|f| f.path().to_path_buf())),
                );
            }
            Message::LibraryPicked(Some(path)) => self.add_library(path),
            Message::LibraryPicked(None) => {}
            Message::RemoveLibrary(i) => {
                if i < self.libraries.len() {
                    self.libraries.remove(i);
                }
            }
            Message::InsertLibraryComponent(i, id) => self.insert_library_component(i, id),
            Message::UseFillStyle(style) => self.per_node("Fill style", |n| {
                let mut refs = n.props.styles.clone();
                refs.fill = style;
                Some(Patch { styles: Some(refs), ..Patch::default() })
            }),
            Message::UseTextStyle(style) => self.per_node("Text style", |n| {
                if !matches!(n.kind, NodeKind::Text { .. }) {
                    return None;
                }
                let mut refs = n.props.styles.clone();
                refs.text = style;
                Some(Patch { styles: Some(refs), ..Patch::default() })
            }),
            Message::CreateFillStyle => {
                let Some(n) = self.selection.first().and_then(|&id| self.doc().node(id)) else { return Task::none() };
                let count = self.doc().tokens().styles.len() + 1;
                let op = Op::CreateStyle { name: format!("Color {count}"), style: StyleKind::Paint { paints: n.props.fills.clone() }, key: Some("s".into()) };
                // Styles are referenced by id, known only once created: two batches, one step.
                if let Some(done) = self.run("Create style", &[op])
                    && let Some(sid) = done.id("s")
                {
                    let _ = self.handle(Message::UseFillStyle(Some(sid)));
                    self.ed.history.merge_last();
                }
            }
            Message::CreateTextStyle => {
                let Some(NodeKind::Text { text }) = self.selection.first().and_then(|&id| self.doc().node(id)).map(|n| &n.kind) else { return Task::none() };
                let count = self.doc().tokens().styles.len() + 1;
                let op = Op::CreateStyle { name: format!("Text {count}"), style: StyleKind::Text { text: text.style.clone() }, key: Some("s".into()) };
                if let Some(done) = self.run("Create style", &[op])
                    && let Some(sid) = done.id("s")
                {
                    let _ = self.handle(Message::UseTextStyle(Some(sid)));
                    self.ed.history.merge_last();
                }
            }
            Message::SetMode(collection, mode) => self.per_node("Variable mode", |n| {
                n.kind.is_frame_like().then(|| Patch { modes: Some([(collection, mode.clone())].into()), ..Patch::default() })
            }),
            Message::AssetFilter(f) => self.asset_filter = f,
            Message::InsertComponent(c) => self.insert_component(c),
            Message::CombineVariants => self.combine_variants(),
            Message::SwapComponent(c) => self.patch_selection("Swap instance", Patch { component: Some(c), ..Patch::default() }),
            Message::AddProperty(kind) => self.add_property(kind),
            Message::RemoveProperty(name) => self.edit_properties("Remove property", |props| {
                props.remove(&name);
            }),
            Message::SetPropertyDefault(name, v) => self.edit_properties("Property default", move |props| {
                if let Some(d) = props.get_mut(&name) {
                    match (d, &v) {
                        (PropDef::Text { default }, PropValue::Text(t)) => *default = t.clone(),
                        (PropDef::Boolean { default }, PropValue::Bool(b)) => *default = *b,
                        (PropDef::InstanceSwap { default }, PropValue::Node(n)) => *default = *n,
                        _ => {}
                    }
                }
            }),
            Message::LinkProperty(link, prop) => {
                self.per_node("Link property", |n| {
                    let mut refs = n.props.refs.clone();
                    match link {
                        PropLink::Text => refs.text = prop.clone(),
                        PropLink::Visible => refs.visible = prop.clone(),
                        PropLink::Swap => refs.swap = prop.clone(),
                    }
                    Some(Patch { refs: Some(refs), ..Patch::default() })
                });
            }
            Message::Boolean(op) => {
                let ops = boolean::boolean(self.doc(), &self.selection, op);
                self.replace_with("result", op.name(), ops);
            }
            Message::Flatten => {
                let ops = boolean::flatten(self.doc(), &self.selection);
                self.replace_with("result", "Flatten", ops);
            }
            Message::Align(to) => {
                let ops = arrange::align(self.doc(), &self.selection, to);
                self.run("Align", &ops);
            }
            Message::Distribute(horizontal) => {
                let ops = arrange::distribute(self.doc(), &self.selection, horizontal);
                if ops.is_empty() && self.selection.len() < 3 {
                    self.error("Distribute needs three or more layers".into());
                }
                self.run("Distribute", &ops);
            }
            Message::Group => self.group(false),
            Message::FrameSelection => self.group(true),
            Message::Ungroup => match arrange::ungroup(self.doc(), &self.selection) {
                Ok((ops, freed)) => {
                    if self.run("Ungroup", &ops).is_some() {
                        self.selection = freed;
                    }
                }
                Err(e) => self.error(e),
            },
            Message::Reorder(order) => {
                let ops = arrange::reorder(self.doc(), &self.selection, order);
                self.run("Reorder", &ops);
            }
            Message::LayerPress(id) => {
                if self.renaming.as_ref().is_some_and(|(r, _)| *r != id) {
                    self.commit_rename();
                }
                if self.modifiers.shift() || self.modifiers.command() {
                    self.select(id, true);
                } else if !self.selection.contains(&id) {
                    self.select(id, false);
                }
                self.layer_drag = Some(LayerDrag { id, target: None });
            }
            Message::LayerHover(target, y) => self.layer_hover(target, y),
            Message::LayerRelease => {
                if let Some(LayerDrag { id, target: Some((t, drop)) }) = self.layer_drag.take() {
                    self.drop_layer(id, t, drop);
                } else if let Some(LayerDrag { id, target: None }) = self.layer_drag.take()
                    && !self.modifiers.shift()
                    && !self.modifiers.command()
                {
                    // A plain click on an already-selected layer selects just it.
                    self.select(id, false);
                }
            }
            Message::StartRename(id) => {
                let name = self.doc().node(id).map(|n| n.name.clone()).unwrap_or_default();
                self.layer_drag = None;
                self.renaming = Some((id, name));
                return Task::batch([operation::focus(RENAME_INPUT), operation::select_all(RENAME_INPUT)]);
            }
            Message::RenameInput(s) => {
                if let Some((_, name)) = &mut self.renaming {
                    *name = s;
                }
            }
            Message::RenameSubmit => self.commit_rename(),
            Message::Key(e) => return self.on_key(e),
            Message::ScaleFactor(s) => self.scale_factor = s.max(0.5),
            Message::Viewport(size) => {
                self.viewport = size;
                if std::mem::take(&mut self.fit_pending) {
                    self.zoom_fit(false);
                }
            }
            Message::Tool(t) => {
                if t != Tool::Pen {
                    self.finish_pen(false);
                }
                self.vector_edit = None;
                self.tool = t;
                self.drag = None;
            }
            Message::CanvasPress { at, button } => return self.press(at, button),
            Message::EditText(id) => return self.start_text_edit(id),
            Message::TextAction(action) => {
                if let Some((_, content)) = &mut self.text_edit {
                    content.perform(action);
                }
            }
            Message::EndTextEdit => self.commit_text(),
            Message::CanvasMove(at) => self.drag_to(at),
            Message::CanvasRelease(at) => return self.release(at),
            Message::CanvasScroll { delta, at } => self.scroll(delta, at),
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
            Message::New => return self.guard(After::New),
            Message::Open => return self.guard(After::Open),
            Message::Opened(Some(path)) => self.open_path(path),
            Message::SavedAs(None) => self.after_save = None,
            Message::Opened(None) | Message::Exported(None, _) => {}
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
                if let Some(a) = self.after_save.take()
                    && !self.ed.history.is_dirty()
                {
                    return self.proceed(a);
                }
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
        if self.present.is_some() {
            return match key.as_ref() {
                Key::Named(Named::Escape) => self.handle(Message::StopPresenting),
                Key::Named(Named::ArrowRight | Named::Space) => self.handle(Message::PresentStep(1)),
                Key::Named(Named::ArrowLeft) => self.handle(Message::PresentStep(-1)),
                _ => Task::none(),
            };
        }
        if matches!(key.as_ref(), Key::Named(Named::Escape | Named::Enter)) {
            if self.pen.is_some() {
                self.finish_pen(false);
                return Task::none();
            }
            if self.vector_edit.take().is_some() {
                return Task::none();
            }
        }
        if let Some(VectorEdit { id, selected: Some((s, i)) }) = self.vector_edit
            && matches!(key.as_ref(), Key::Named(Named::Delete | Named::Backspace))
        {
            let path = match self.doc().node(id).map(|n| &n.kind) {
                Some(NodeKind::Vector { path }) => pen::without(path, s, i),
                _ => return Task::none(),
            };
            self.vector_edit = Some(VectorEdit { id, selected: None });
            match path {
                Some(path) => {
                    let patch = Patch { path: Some(path), ..Patch::default() };
                    let refit = self.doc().node(id).map(|n| {
                        let mut n = n.clone();
                        if let NodeKind::Vector { path } = &mut n.kind {
                            *path = patch.path.clone().expect("set");
                        }
                        pen::refit(&n)
                    });
                    let mut ops = vec![Op::set(id, patch)];
                    ops.extend(refit.flatten().map(|p| Op::set(id, p)));
                    self.run("Delete point", &ops);
                }
                None => {
                    self.vector_edit = None;
                    self.run("Delete", &[Op::Delete { id: id.into() }]);
                }
            }
            return Task::none();
        }
        if matches!(key.as_ref(), Key::Named(Named::Escape)) {
            if self.text_edit.is_some() {
                self.commit_text();
                return Task::none();
            }
            if self.chrome.open_menu.is_some() {
                self.chrome.open_menu = None;
                return Task::none();
            }
            if self.confirm.take().is_some() || self.renaming.take().is_some() {
                return Task::none();
            }
        }
        if self.confirm.is_some() {
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
            Key::Named(Named::Enter) if cmd && alt => Some(Message::Present(None)),
            Key::Named(Named::Enter) if plain => match self.selection[..] {
                [one] if self.is_text(one) => Some(Message::EditText(one)),
                [one] if self.is_vector(one) => {
                    self.vector_edit = Some(VectorEdit { id: one, selected: None });
                    None
                }
                // Enter goes into a frame, like Figma.
                [one] => {
                    let kids = self.doc().children(one).to_vec();
                    if !kids.is_empty() {
                        self.selection = kids;
                    }
                    None
                }
                _ => None,
            },
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
                    ("c", true, false, false) => Some(Message::Copy),
                    ("x", true, false, false) => Some(Message::Cut),
                    ("v", true, false, false) => Some(Message::Paste),
                    ("c", true, true, false) => Some(Message::CopyAsPng),
                    ("a", false, false, true) => Some(Message::Align(Align::Left)),
                    ("h", false, false, true) => Some(Message::Align(Align::Center)),
                    ("d", false, false, true) => Some(Message::Align(Align::Right)),
                    ("w", false, false, true) => Some(Message::Align(Align::Top)),
                    ("v", false, false, true) => Some(Message::Align(Align::Middle)),
                    ("s", false, false, true) => Some(Message::Align(Align::Bottom)),
                    ("h", false, true, true) => Some(Message::Distribute(true)),
                    ("v", false, true, true) => Some(Message::Distribute(false)),
                    ("e", true, false, false) => Some(Message::Flatten),
                    ("g", true, false, false) => Some(Message::Group),
                    ("g", true, false, true) => Some(Message::FrameSelection),
                    ("g", true, true, false) => Some(Message::Ungroup),
                    ("]", true, false, false) => Some(Message::Reorder(Order::Front)),
                    ("[", true, false, false) => Some(Message::Reorder(Order::Back)),
                    ("]" | "}", true, true, false) => Some(Message::Reorder(Order::Forward)),
                    ("[" | "{", true, true, false) => Some(Message::Reorder(Order::Backward)),
                    ("r", true, false, false) => self.selection.first().map(|&id| Message::StartRename(id)),
                    ("a", true, false, false) => Some(Message::SelectAll),
                    ("a", true, true, false) => Some(Message::Deselect),
                    ("q", true, false, false) => Some(Message::Quit),
                    ("s", true, false, false) => Some(Message::Save),
                    ("s", true, true, false) => Some(Message::SaveAs),
                    ("o", true, false, false) => Some(Message::Open),
                    ("n", true, false, false) => Some(Message::New),
                    ("e", true, true, false) => Some(Message::Export(ExportFormat::Png)),
                    ("k", true, false, true) => Some(Message::CreateComponent),
                    ("i", false, false, true) => Some(Message::LeftTab(1)),
                    ("l", false, false, true) => Some(Message::LeftTab(0)),
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
        // Variables and styles don't need a selection.
        match &f {
            Field::VarName(id) => {
                if !input.is_empty() {
                    self.run("Rename variable", &[Op::SetVariable { id: (*id).into(), name: Some(input), mode: None, value: None }]);
                }
                return;
            }
            Field::VarValue(id, mode) => {
                let doc = self.doc();
                let Some(var) = doc.tokens().variables.get(id) else { return };
                let current = var.values.values().next().cloned();
                let value = match current {
                    Some(VarValue::Color(_)) => Rgba::from_hex(&input).map(VarValue::Color),
                    Some(VarValue::Number(_)) => num().map(VarValue::Number),
                    Some(VarValue::Bool(_)) => input.parse().ok().map(VarValue::Bool),
                    // An alias can be replaced by a plain value of the same text.
                    _ => Some(VarValue::Text(input.clone())),
                };
                match value {
                    Some(v) => {
                        self.run("Set variable", &[Op::SetVariable { id: (*id).into(), name: None, mode: Some(mode.clone()), value: Some(v) }]);
                    }
                    None => self.error(format!("{input:?} isn't a valid value")),
                }
                return;
            }
            Field::StyleName(id) => {
                if !input.is_empty() {
                    self.run("Rename style", &[Op::SetStyle { id: (*id).into(), name: Some(input), style: None }]);
                }
                return;
            }
            _ => {}
        }
        let Some(&first) = self.selection.first() else {
            if f == Field::PageBackground
                && let Some(c) = Rgba::from_hex(&input)
            {
                self.run("Page background", &[Op::set(self.page, Patch { background: Some(c), ..Patch::default() })]);
            }
            return;
        };
        if !self.doc().contains(first) {
            return;
        }
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
                return self.per_node("Auto layout", |n| {
                    let mut l = n.props.layout.clone()?;
                    match f {
                        Field::Gap => l.gap = v,
                        Field::PadH => (l.padding.left, l.padding.right) = (v, v),
                        _ => (l.padding.top, l.padding.bottom) = (v, v),
                    }
                    Some(Patch { layout: Some(Some(l)), ..Patch::default() })
                });
            }
            Field::Text => Patch { text: Some(input.replace("\\n", "\n")), ..Patch::default() },
            Field::FontSize | Field::FontStyle => {
                let size = num().filter(|v| *v > 0.0);
                if f == Field::FontSize && size.is_none() {
                    return bad(self);
                }
                return self.per_node("Text style", |n| {
                    let NodeKind::Text { text } = &n.kind else { return None };
                    let mut style = text.style.clone();
                    match size {
                        Some(v) if f == Field::FontSize => style.size = v as f32,
                        _ => style.style = input.clone(),
                    }
                    Some(Patch { text_style: Some(style), ..Patch::default() })
                });
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
            Field::Reaction(i, part) => {
                let (i, part) = (*i, *part);
                let v = num();
                if part != "url" && v.is_none() {
                    return bad(self);
                }
                let url = input.clone();
                return self.edit_reactions("Edit interaction", move |r| {
                    let Some(x) = r.get_mut(i) else { return };
                    match part {
                        "delay" => {
                            if let jd_vector::Trigger::AfterDelay { seconds } = &mut x.trigger {
                                *seconds = v.unwrap_or(1.0).max(0.0) as f32;
                            }
                        }
                        "duration" => x.duration = (v.unwrap_or(300.0) / 1000.0).clamp(0.0, 10.0) as f32,
                        _ => {
                            if let jd_vector::Action::OpenUrl { url: u } = &mut x.action {
                                *u = url.clone();
                            }
                        }
                    }
                });
            }
            Field::Effect(i, part) => {
                let (i, part) = (*i, *part);
                // Shadow colours keep their alpha (#RRGGBBAA).
                let color = jd_vector::color::from_hex(&input);
                let v = num();
                if (part == "color" && color.is_none()) || (part != "color" && v.is_none()) {
                    return bad(self);
                }
                let v = v.unwrap_or(0.0) as f32;
                return self.edit_effects("Edit effect", move |e| {
                    use jd_vector::Effect as E;
                    match e.get_mut(i) {
                        Some(E::DropShadow(s) | E::InnerShadow(s)) => match part {
                            "x" => s.x = v,
                            "y" => s.y = v,
                            "blur" => s.blur = v.max(0.0),
                            "spread" => s.spread = v,
                            "color" => s.color = color.expect("checked"),
                            _ => {}
                        },
                        Some(E::LayerBlur { radius, .. } | E::BackgroundBlur { radius, .. }) if part == "radius" => *radius = v.max(0.0),
                        _ => {}
                    }
                });
            }
            Field::StrokeWeight => match num().filter(|v| *v >= 0.0) {
                Some(v) => {
                    return self.per_node("Stroke weight", |n| {
                        Some(Patch { stroke: Some(jd_vector::StrokeStyle { weight: v as f32, ..n.props.stroke.clone() }), ..Patch::default() })
                    });
                }
                None => return bad(self),
            },
            Field::Prop(name) => Patch { values: Some([(name.clone(), PropValue::Text(input.clone()))].into()), ..Patch::default() },
            Field::PropDefault(name) => return self.handle_default(name.clone(), input.clone()),
            Field::PageBackground | Field::VarName(_) | Field::VarValue(..) | Field::StyleName(_) => return,
        };
        self.patch_selection("Edit", patch);
    }

    /// A patch worked out for each selected layer (None skips it).
    fn per_node(&mut self, label: &str, f: impl Fn(&jd_vector::Node) -> Option<Patch>) {
        let ops: Vec<Op> = self.selection.iter().filter_map(|&id| Some(Op::set(id, f(self.doc().node(id)?)?))).collect();
        self.run(label, &ops);
    }

    // ----- Clipboard ---------------------------------------------------------------------

    /// Copies the selection; false if there was nothing to copy.
    fn copy(&mut self) -> bool {
        let Some(clip) = Clip::copy(self.doc(), &self.selection) else { return false };
        clipboard::put_text(clip.to_text());
        self.info(format!("Copied {} layer{}", clip.len(), if clip.len() == 1 { "" } else { "s" }));
        self.clip = Some(clip);
        true
    }

    /// The visible part of the page.
    fn view_rect(&self) -> Rect {
        let (w, h) = (self.viewport.width as f64 / self.camera.zoom, self.viewport.height as f64 / self.camera.zoom);
        Rect::from_origin_size(self.camera.origin, (w, h))
    }

    /// Where pasted layers go, Figma-style: into a selected frame (unless it's what was
    /// copied), else just above the selected layer, else on the page.
    fn paste_target(&self, sources: &[NodeId]) -> (NodeId, Option<usize>) {
        let doc = self.doc();
        let [one] = self.selection[..] else { return (self.page, None) };
        let Some(n) = doc.node(one) else { return (self.page, None) };
        let open = n.kind.is_container() && !matches!(n.kind, NodeKind::Instance { .. }) && !doc.is_inside_instance(one);
        if open && !sources.contains(&one) {
            return (one, None);
        }
        // Next to it, climbing out of instances (nothing can be added inside them).
        let mut at = one;
        while doc.is_inside_instance(at) || doc.node(doc.parent(at).unwrap_or(self.page)).is_some_and(|p| matches!(p.kind, NodeKind::Instance { .. })) {
            match doc.parent(at) {
                Some(p) if p != self.page => at = p,
                _ => break,
            }
        }
        let parent = doc.parent(at).unwrap_or(self.page);
        let index = doc.children(parent).iter().position(|&c| c == at).map(|i| i + 1);
        (parent, index)
    }

    fn paste(&mut self, clip: Clip) {
        let doc = self.doc();
        let (parent, index) = self.paste_target(&clip.sources());
        // In place when it lands where it came from (and is in view); else centred on the
        // frame it goes into, or on the view.
        let delta = match clip.bounds() {
            Some(b) => {
                let same = !clip.parents().is_empty() && clip.parents().iter().all(|&p| p == parent);
                let target = if parent == self.page { self.view_rect() } else { doc.world_bounds(parent).unwrap_or(b) };
                if same && (parent != self.page || target.intersect(b).area() > 0.0) { Vec2::ZERO } else { target.center() - b.center() }
            }
            None => Vec2::ZERO,
        };
        let to_local = doc.world_transform(parent).inverse();
        let mut ops = vec![Op::Paste { parent: parent.into(), clip: clip.clone(), index, key: Some("p".into()), offset: [0.0, 0.0] }];
        for (i, o) in clip.origins().iter().enumerate() {
            let p = to_local * (kurbo::Point::new(o[0], o[1]) + delta);
            ops.push(Op::set(format!("p{i}").as_str(), Patch { x: Some(round2(p.x)), y: Some(round2(p.y)), ..Patch::default() }));
        }
        if let Some(done) = self.run("Paste", &ops) {
            self.selection = (0..clip.len()).filter_map(|i| done.id(&format!("p{i}"))).collect();
            if let Some(&first) = self.selection.first() {
                let sel = std::mem::take(&mut self.selection);
                self.select(first, false);
                self.selection = sel;
            }
        }
    }

    /// An SVG as a frame of vector layers, in the middle of the view (or selected frame).
    fn import_svg(&mut self, data: &[u8], name: &str) {
        let (parent, index) = self.paste_target(&[]);
        let mut ops = match jd_vector_render::import_svg(data, parent.into(), name, 0.0, 0.0) {
            Ok(ops) => ops,
            Err(e) => return self.error(e),
        };
        let doc = self.doc();
        let center = if parent == self.page { self.view_rect().center() } else { doc.world_bounds(parent).map_or(self.view_rect().center(), |b| b.center()) };
        if let Some(Op::Create { node, index: at, .. }) = ops.first_mut() {
            let local = doc.world_transform(parent).inverse() * (center - kurbo::Vec2::new(node.props.width / 2.0, node.props.height / 2.0));
            node.props.x = local.x.round();
            node.props.y = local.y.round();
            *at = index;
        }
        if let Some(done) = self.run("Import SVG", &ops)
            && let Some(f) = done.id("svg")
        {
            self.select(f, false);
        }
    }

    /// Text from another app becomes a text layer in the middle of the view; SVG markup is
    /// imported as shapes.
    fn paste_text(&mut self, text: String) {
        let t = text.trim_start();
        if t.starts_with("<svg") || (t.starts_with("<?xml") && t.contains("<svg")) {
            return self.import_svg(text.as_bytes(), "SVG");
        }
        let c = self.view_rect().center();
        let spec = NodeSpec::text("Text", text.trim_end(), TextStyle { size: 16.0, ..TextStyle::default() }).at(c.x.round(), c.y.round());
        if let Some(done) = self.run("Paste text", &[Op::create(self.page, spec)]) {
            self.select(done.created[0], false);
        }
    }

    // ----- Components --------------------------------------------------------------------

    /// Every component in the document that can be placed: plain components and component
    /// sets (not the variants inside a set), with the page each is on.
    pub fn components(&self) -> Vec<(NodeId, NodeId)> {
        placeable_components(self.doc())
    }

    /// The component that `id` sits in (or is), not counting instances.
    pub fn owning_component(&self, id: NodeId) -> Option<NodeId> {
        let doc = self.doc();
        let mut cur = Some(id);
        while let Some(c) = cur {
            let n = doc.node(c)?;
            match n.kind {
                NodeKind::Component { .. } => return Some(c),
                NodeKind::Instance { .. } | NodeKind::Page { .. } => return None,
                _ => cur = n.parent,
            }
        }
        None
    }

    /// An instance of `c` (a component set places its first variant) in the middle of the
    /// view, or of the selected frame.
    fn insert_component(&mut self, c: NodeId) {
        let doc = self.doc();
        let component = match doc.node(c).map(|n| &n.kind) {
            Some(NodeKind::ComponentSet { .. }) => match doc.children(c).first() {
                Some(&v) => v,
                None => return self.error("this component set has no variants".into()),
            },
            Some(NodeKind::Component { .. }) => c,
            _ => return,
        };
        let (parent, index) = self.paste_target(&[]);
        let size = doc.node(component).map_or(kurbo::Size::ZERO, |n| kurbo::Size::new(n.props.width, n.props.height));
        let center = if parent == self.page { self.view_rect().center() } else { doc.world_bounds(parent).map_or(self.view_rect().center(), |b| b.center()) };
        let at = doc.world_transform(parent).inverse() * (center - size.to_vec2() / 2.0);
        let op = Op::CreateInstance { parent: parent.into(), component: component.into(), key: Some("i".into()), index, x: at.x.round(), y: at.y.round() };
        if let Some(done) = self.run("Place instance", &[op])
            && let Some(i) = done.id("i")
        {
            self.select(i, false);
        }
    }

    /// Puts the selected components in a component set, naming plain ones as variants.
    fn combine_variants(&mut self) {
        let doc = self.doc();
        let comps: Vec<NodeId> = self.selection.iter().copied().filter(|&id| doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Component { .. }))).collect();
        if comps.len() < 2 {
            return self.error("select two or more components to combine".into());
        }
        let mut ops = Vec::new();
        for &c in &comps {
            let name = &doc.node(c).expect("checked").name;
            if jd_vector::component::parse_variant(name).is_empty() {
                ops.push(Op::set(c, Patch { name: Some(format!("Property 1={name}")), ..Patch::default() }));
            }
        }
        let base = doc.node(comps[0]).map(|n| n.name.split('=').next_back().unwrap_or("Component").trim().to_owned()).unwrap_or_default();
        ops.push(Op::CombineAsVariants { ids: comps.iter().map(|&c| c.into()).collect(), name: base, key: Some("set".into()) });
        if let Some(done) = self.run("Combine as variants", &ops)
            && let Some(set) = done.id("set")
        {
            self.select(set, false);
        }
    }

    fn handle_default(&mut self, name: String, value: String) {
        let _ = self.handle(Message::SetPropertyDefault(name, PropValue::Text(value)));
    }

    fn edit_reactions(&mut self, label: &str, f: impl Fn(&mut Vec<jd_vector::Reaction>)) {
        let [id] = self.selection[..] else { return };
        let Some(n) = self.doc().node(id) else { return };
        let mut r = n.props.reactions.clone();
        f(&mut r);
        self.run(label, &[Op::set(id, Patch { reactions: Some(r), ..Patch::default() })]);
    }

    /// The current page's top-level frames, in layer order (bottom first).
    pub fn top_frames(&self) -> Vec<NodeId> {
        let doc = self.doc();
        doc.children(self.page).iter().copied().filter(|&c| doc.node(c).is_some_and(|n| n.kind.is_frame_like())).collect()
    }

    /// The top-level frame a layer is in (or is).
    pub fn top_frame(&self, id: NodeId) -> Option<NodeId> {
        let doc = self.doc();
        let mut cur = id;
        loop {
            let p = doc.parent(cur)?;
            if p == self.page {
                return doc.node(cur).is_some_and(|n| n.kind.is_frame_like()).then_some(cur);
            }
            cur = p;
        }
    }

    // ----- Presenting --------------------------------------------------------------------

    fn shoot(&self, id: NodeId, overlay: bool) -> Option<Shot> {
        let n = self.doc().node(id)?;
        let size = kurbo::Size::new(n.props.width, n.props.height);
        let (w, h) = (self.chrome.size.width as f64, self.chrome.size.height as f64);
        let room = if overlay { 0.9 } else { 1.0 };
        let fit = ((w - 48.0) * room / size.width.max(1.0)).min((h - 48.0) * room / size.height.max(1.0)).max(0.05);
        // Overlays keep the main frame's scale, so they sit at their real size relative to it.
        let fit = if overlay { self.present.as_ref().and_then(|p| p.shot.as_ref()).map_or(fit, |s| s.fit.min(fit)) } else { fit };
        let pm = jd_vector_render::render_node(self.doc(), id, RenderOptions { scale: fit * self.scale_factor as f64, background: None })?;
        Some(Shot { image: image::Handle::from_rgba(pm.width(), pm.height(), clipboard::to_rgba(&pm)), fit, size })
    }

    fn start_presenting(&mut self, from: Option<NodeId>) {
        let frames = self.top_frames();
        let start = from.or_else(|| self.selection.first().and_then(|&s| self.top_frame(s))).or_else(|| frames.first().copied());
        let Some(start) = start else { return self.error("Add a frame to present".into()) };
        self.chrome.open_menu = None;
        self.present = Some(Presentation {
            frame: start,
            shot: None,
            overlay: None,
            leaving: None,
            history: Vec::new(),
            entered: Instant::now(),
            delay_done: false,
            cursor: Point::ORIGIN,
            hovered: None,
        });
        let shot = self.shoot(start, false);
        if let Some(p) = &mut self.present {
            p.shot = shot;
        }
    }

    /// Shows `to` (a top-level frame), animating from the current one.
    fn go_to(&mut self, to: NodeId, transition: jd_vector::Transition, duration: f32, remember: bool) {
        if !self.doc().node(to).is_some_and(|n| n.kind.is_frame_like()) {
            return;
        }
        let shot = self.shoot(to, false);
        let Some(p) = &mut self.present else { return };
        if remember {
            p.history.push(p.frame);
        }
        let old = p.shot.take();
        if transition != jd_vector::Transition::Instant
            && let Some(old) = old
        {
            p.leaving = Some((old, transition, Instant::now(), duration.max(0.05)));
        }
        p.frame = to;
        p.shot = shot;
        p.overlay = None;
        p.entered = Instant::now();
        p.delay_done = false;
        p.hovered = None;
    }

    fn act(&mut self, r: &jd_vector::Reaction) {
        match &r.action {
            jd_vector::Action::Navigate { to } => self.go_to(*to, r.transition, r.duration, true),
            jd_vector::Action::Overlay { to } => {
                let shot = self.shoot(*to, true);
                if let (Some(p), Some(s)) = (&mut self.present, shot) {
                    p.overlay = Some((*to, s));
                }
            }
            jd_vector::Action::Back => {
                let Some(p) = &mut self.present else { return };
                if p.overlay.take().is_some() {
                    return;
                }
                if let Some(prev) = p.history.pop() {
                    self.go_to(prev, r.transition, r.duration, false);
                }
            }
            jd_vector::Action::OpenUrl { url } => {
                // Only web links, opened by the desktop's browser.
                if url.starts_with("https://") || url.starts_with("http://") {
                    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
                }
            }
        }
    }

    /// Where a frame's picture sits in the window: its top-left and scale.
    pub fn shot_origin(&self, shot: &Shot) -> Point {
        let (w, h) = (self.chrome.size.width, self.chrome.size.height);
        let (sw, sh) = ((shot.size.width * shot.fit) as f32, (shot.size.height * shot.fit) as f32);
        Point::new((w - sw) / 2.0, (h - sh) / 2.0)
    }

    /// The topmost layer under the pointer (in `frame`) with a reaction for `trigger`.
    fn reaction_at(&self, frame: NodeId, shot: &Shot, at: Point, trigger: fn(&jd_vector::Trigger) -> bool) -> Option<(NodeId, jd_vector::Reaction)> {
        let doc = self.doc();
        let o = self.shot_origin(shot);
        let local = kurbo::Point::new((at.x - o.x) as f64 / shot.fit, (at.y - o.y) as f64 / shot.fit);
        if local.x < 0.0 || local.y < 0.0 || local.x > shot.size.width || local.y > shot.size.height {
            return None;
        }
        let page_pt = doc.world_transform(frame) * local;
        let page = doc.page_of(frame)?;
        for hit in doc.nodes_at(page, page_pt, 0.0) {
            if hit != frame && !doc.is_ancestor(frame, hit) {
                continue;
            }
            let mut cur = Some(hit);
            while let Some(c) = cur {
                if let Some(r) = doc.node(c).and_then(|n| n.props.reactions.iter().find(|r| trigger(&r.trigger))) {
                    return Some((c, r.clone()));
                }
                if c == frame {
                    break;
                }
                cur = doc.parent(c);
            }
        }
        None
    }

    fn present_move(&mut self, at: Point) {
        let Some(p) = &mut self.present else { return };
        p.cursor = at;
        let (frame, shot) = match (&p.overlay, &p.shot) {
            (Some((o, s)), _) => (*o, s.clone()),
            (None, Some(s)) => (p.frame, s.clone()),
            _ => return,
        };
        let found = self.reaction_at(frame, &shot, at, |t| matches!(t, jd_vector::Trigger::Hover));
        let Some(p) = &mut self.present else { return };
        let was = p.hovered;
        p.hovered = found.as_ref().map(|f| f.0);
        if let Some((id, r)) = found
            && was != Some(id)
        {
            self.act(&r);
        }
    }

    fn present_click(&mut self) {
        let Some(p) = &self.present else { return };
        let at = p.cursor;
        if let Some((o, s)) = &p.overlay {
            match self.reaction_at(*o, s, at, |t| matches!(t, jd_vector::Trigger::Click)) {
                Some((_, r)) => return self.act(&r),
                None => {
                    // Clicking outside an overlay closes it.
                    let org = self.shot_origin(s);
                    let inside = at.x >= org.x && at.y >= org.y && at.x <= org.x + (s.size.width * s.fit) as f32 && at.y <= org.y + (s.size.height * s.fit) as f32;
                    if !inside && let Some(p) = &mut self.present {
                        p.overlay = None;
                    }
                    return;
                }
            }
        }
        let Some(shot) = p.shot.clone() else { return };
        if let Some((_, r)) = self.reaction_at(p.frame, &shot, at, |t| matches!(t, jd_vector::Trigger::Click)) {
            self.act(&r);
        }
    }

    fn present_tick(&mut self) {
        let Some(p) = &mut self.present else { return };
        if let Some((_, _, start, d)) = &p.leaving
            && start.elapsed().as_secs_f32() >= *d
        {
            p.leaving = None;
        }
        if p.delay_done {
            return;
        }
        let elapsed = p.entered.elapsed().as_secs_f32();
        let frame = p.frame;
        let due = self.doc().node(frame).and_then(|n| {
            n.props.reactions.iter().find(|r| matches!(r.trigger, jd_vector::Trigger::AfterDelay { seconds } if elapsed >= seconds)).cloned()
        });
        if let Some(r) = due {
            if let Some(p) = &mut self.present {
                p.delay_done = true;
            }
            self.act(&r);
        }
    }

    /// Whether presenting needs a tick every frame (a transition or a timer running).
    pub fn present_animating(&self) -> bool {
        self.present.as_ref().is_some_and(|p| {
            p.leaving.is_some()
                || (!p.delay_done && self.doc().node(p.frame).is_some_and(|n| n.props.reactions.iter().any(|r| matches!(r.trigger, jd_vector::Trigger::AfterDelay { .. }))))
        })
    }

    fn edit_effects(&mut self, label: &str, f: impl Fn(&mut Vec<jd_vector::Effect>)) {
        self.per_node(label, |n| {
            let mut e = n.props.effects.clone();
            f(&mut e);
            Some(Patch { effects: Some(e), ..Patch::default() })
        });
    }

    fn add_library(&mut self, path: PathBuf) {
        if self.libraries.iter().any(|l| l.path == path) {
            return;
        }
        match file::load(&path) {
            Ok(mut doc) => {
                jd_vector::layout::run(&mut doc, &CosmicMeasure);
                let name = path.file_stem().map_or("Library".into(), |n| n.to_string_lossy().into_owned());
                let count = placeable_components(&doc).len();
                let thumbnails = thumbnails_of(&doc, self.scale_factor);
                self.info(format!("Added library {name} ({count} components)"));
                self.libraries.push(Library { path, name, doc, thumbnails });
                self.left_tab = 1;
            }
            Err(e) => self.error(format!("Couldn't open {}: {e}", path.display())),
        }
    }

    /// Copies a library component into this file (once, onto the library page) and places
    /// an instance of it.
    fn insert_library_component(&mut self, lib: usize, id: NodeId) {
        let Some(l) = self.libraries.get(lib) else { return };
        let key = (l.path.clone(), id);
        if let Some(&local) = self.imported.get(&key)
            && self.doc().contains(local)
        {
            return self.insert_component(local);
        }
        let Some(clip) = Clip::copy(&l.doc, &[id]) else { return };
        let doc = self.doc();
        let page = doc.pages().iter().copied().find(|&p| doc.node(p).is_some_and(|n| n.name == LIBRARY_PAGE));
        let right = page.map_or(0.0, |p| doc.children(p).iter().filter_map(|&c| doc.world_bounds(c)).map(|b| b.x1 + 80.0).fold(0.0, f64::max));
        let mut ops = Vec::new();
        let parent: jd_vector::Ref = match page {
            Some(p) => p.into(),
            None => {
                ops.push(Op::CreatePage { name: LIBRARY_PAGE.into(), key: Some("lp".into()), index: None });
                "lp".into()
            }
        };
        ops.push(Op::Paste { parent, clip, index: None, key: Some("c".into()), offset: [0.0, 0.0] });
        ops.push(Op::set("c0", Patch { x: Some(right), y: Some(0.0), ..Patch::default() }));
        let Some(done) = self.run("Import component", &ops) else { return };
        let Some(local) = done.id("c0") else { return };
        self.imported.insert(key, local);
        self.insert_component(local);
        self.ed.history.merge_last();
    }

    /// Changes the selected component's property definitions.
    fn edit_properties(&mut self, label: &str, f: impl Fn(&mut std::collections::BTreeMap<String, PropDef>)) {
        let [id] = self.selection[..] else { return };
        let Some(NodeKind::Component { properties, .. }) = self.doc().node(id).map(|n| &n.kind) else { return };
        let mut props = properties.clone();
        f(&mut props);
        self.run(label, &[Op::set(id, Patch { properties: Some(props), ..Patch::default() })]);
    }

    fn add_property(&mut self, kind: &'static str) {
        let [id] = self.selection[..] else { return };
        let Some(NodeKind::Component { properties, .. }) = self.doc().node(id).map(|n| &n.kind) else { return };
        let base = match kind {
            "text" => "Text",
            "swap" => "Icon",
            _ => "Show",
        };
        let name = (1..).map(|i| if i == 1 { base.to_owned() } else { format!("{base} {i}") }).find(|n| !properties.contains_key(n)).expect("endless");
        let def = match kind {
            "text" => PropDef::Text { default: "Text".into() },
            "swap" => {
                // Defaults to another component: a set's first variant.
                let doc = self.doc();
                let other = placeable_components(doc).into_iter().map(|(c, _)| c).find(|&c| c != id).and_then(|c| match doc.node(c).map(|n| &n.kind) {
                    Some(NodeKind::ComponentSet { .. }) => doc.children(c).first().copied(),
                    _ => Some(c),
                });
                match other {
                    Some(c) => PropDef::InstanceSwap { default: c },
                    None => return self.error("an instance-swap property needs another component to swap to".into()),
                }
            }
            _ => PropDef::Boolean { default: true },
        };
        self.edit_properties("Add property", move |props| {
            props.insert(name.clone(), def.clone());
        });
    }

    /// Re-renders component thumbnails when the document changed and Assets is showing.
    fn refresh_thumbnails(&mut self) {
        if self.left_tab != 1 || self.thumbs_version == self.version {
            return;
        }
        self.thumbs_version = self.version;
        self.thumbnails = thumbnails_of(self.doc(), self.scale_factor);
    }

    // ----- Arranging ---------------------------------------------------------------------

    /// Runs ops that replace the selection with one new layer (known as `key`), and selects it.
    fn replace_with(&mut self, key: &str, label: &str, ops: Result<Vec<Op>, String>) {
        match ops {
            Ok(ops) if ops.is_empty() => {}
            Ok(ops) => {
                if let Some(done) = self.run(label, &ops)
                    && let Some(id) = done.id(key)
                {
                    self.select(id, false);
                }
            }
            Err(e) => self.error(e),
        }
    }

    fn group(&mut self, frame: bool) {
        match arrange::group(self.doc(), &self.selection, frame) {
            Ok(ops) => {
                if let Some(done) = self.run(if frame { "Frame selection" } else { "Group" }, &ops)
                    && let Some(g) = done.id("group")
                {
                    self.select(g, false);
                }
            }
            Err(e) => self.error(e),
        }
    }

    fn commit_rename(&mut self) {
        let Some((id, name)) = self.renaming.take() else { return };
        let name = name.trim().to_owned();
        if !name.is_empty() && self.doc().node(id).is_some_and(|n| n.name != name) {
            self.run("Rename", &[Op::set(id, Patch { name: Some(name), ..Patch::default() })]);
        }
    }

    /// The pointer is `y` px into `target`'s row while a layer is dragged.
    fn layer_hover(&mut self, target: NodeId, y: f32) {
        let Some(drag) = &mut self.layer_drag else { return };
        let doc = &self.ed.doc;
        if target == drag.id || doc.is_ancestor(drag.id, target) {
            drag.target = None;
            return;
        }
        let f = y / crate::view::LAYER_ROW;
        let container = doc.node(target).is_some_and(|n| n.kind.is_container()) && !doc.is_inside_instance(target);
        let drop = if container && (0.3..0.7).contains(&f) {
            Drop::Inside
        } else if f < 0.5 {
            Drop::Above
        } else {
            Drop::Below
        };
        drag.target = Some((target, drop));
    }

    /// Moves `id` next to (or into) `target`, keeping it where it is on the page.
    fn drop_layer(&mut self, id: NodeId, target: NodeId, drop: Drop) {
        let doc = self.doc();
        let (parent, index) = match drop {
            Drop::Inside => (target, None),
            _ => {
                let Some(parent) = doc.parent(target) else { return };
                let kids = doc.children(parent);
                let t = kids.iter().position(|&c| c == target).unwrap_or(0);
                // The list shows the top of the stack first: "above" is a higher index.
                let mut i = if drop == Drop::Above { t + 1 } else { t };
                if let Some(cur) = kids.iter().position(|&c| c == id)
                    && cur < t
                {
                    i -= 1;
                }
                (parent, Some(i))
            }
        };
        let Some(world) = doc.node(id).map(|_| doc.world_transform(id)) else { return };
        let local = doc.world_transform(parent).inverse() * world;
        let [.., x, y] = local.as_coeffs();
        let ops = [Op::Move { id: id.into(), parent: parent.into(), index }, Op::set(id, Patch { x: Some(round2(x)), y: Some(round2(y)), ..Patch::default() })];
        if self.run("Move layer", &ops).is_some() && drop == Drop::Inside {
            self.expanded.insert(target);
        }
    }

    // ----- Unsaved changes --------------------------------------------------------------

    /// Asks about unsaved changes before `after`, or just does it.
    fn guard(&mut self, after: After) -> Task<Message> {
        if self.ed.history.is_dirty() {
            self.chrome.open_menu = None;
            self.confirm = Some(after);
            Task::none()
        } else {
            self.proceed(after)
        }
    }

    fn proceed(&mut self, after: After) -> Task<Message> {
        match after {
            After::Quit => self.chrome.close(),
            After::New => {
                self.load(Document::new(), None);
                Task::none()
            }
            After::Open => Task::perform(
                rfd::AsyncFileDialog::new().add_filter("Blobr document", &[file::EXTENSION]).pick_file(),
                |f| Message::Opened(f.map(|f| f.path().to_path_buf())),
            ),
        }
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

    /// The selection's page box.
    pub fn selection_bounds(&self) -> Option<Rect> {
        self.selection.iter().filter_map(|&id| self.doc().world_bounds(id)).reduce(|a, b| a.union(b))
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

    fn press(&mut self, at: Point, button: mouse::Button) -> Task<Message> {
        let page = self.camera.to_page(at);
        if button == mouse::Button::Middle || self.space_held || self.tool == Tool::Hand {
            self.drag = Some(Drag::Pan { start: at, origin: self.camera.origin });
            return Task::none();
        }
        if button != mouse::Button::Left {
            return Task::none();
        }
        // Clicking anywhere else on the canvas finishes typing.
        self.commit_text();
        if self.tool == Tool::Pen {
            self.pen_press(page, at);
            return Task::none();
        }
        if let Some(ve) = self.vector_edit {
            if let Some((sub, idx, part)) = self.anchor_at(ve.id, at, ve.selected) {
                self.vector_edit = Some(VectorEdit { selected: Some((sub, idx)), ..ve });
                self.drag = Some(Drag::Anchor { id: ve.id, sub, idx, part });
                return Task::none();
            }
            // Clicking away from the points leaves point editing.
            self.vector_edit = None;
        }
        let now = Instant::now();
        let double = self.last_click.is_some_and(|(t, p)| now - t < Duration::from_millis(400) && (p.x - at.x).abs() < 4.0 && (p.y - at.y).abs() < 4.0);
        self.last_click = Some((now, at));
        if self.tool.creates() {
            self.drag = Some(Drag::Create { tool: self.tool, start: page, now: page });
            return Task::none();
        }
        if let Some((id, handle)) = self.handle_at(at) {
            let n = self.doc().node(id).expect("selected");
            let to_parent = n.parent.map_or(Affine::IDENTITY, |p| self.doc().world_transform(p).inverse());
            let rect = Rect::new(n.props.x, n.props.y, n.props.x + n.props.width, n.props.y + n.props.height);
            let targets = snap::targets(self.doc(), &[id]);
            self.drag = Some(Drag::Resize { id, handle, start: page, rect, to_parent, started: false, targets });
            return Task::none();
        }
        if let Some(id) = self.rotate_at(at) {
            let n = self.doc().node(id).expect("selected");
            let half = Vec2::new(n.props.width / 2.0, n.props.height / 2.0);
            let center_parent = jd_vector::Document::local_transform(n) * half.to_point();
            let center_page = self.doc().world_transform(id) * half.to_point();
            let a0 = (page - center_page).atan2();
            self.drag = Some(Drag::Rotate { id, center_page, center_parent, a0, rot0: n.props.rotation, half });
            return Task::none();
        }
        let shift = self.modifiers.shift();
        match self.pick_at(at, self.modifiers.command(), double) {
            // Double-clicking a selected text layer types into it.
            Some(id) if double && self.selection == [id] && self.is_text(id) => return self.start_text_edit(id),
            // And a selected vector edits its points.
            Some(id) if double && self.selection == [id] && self.is_vector(id) => {
                self.vector_edit = Some(VectorEdit { id, selected: None });
            }
            Some(id) => {
                if shift {
                    self.select(id, true);
                } else if !self.selection.contains(&id) {
                    self.select(id, false);
                }
                let items = self.movable();
                let ids: Vec<NodeId> = items.iter().map(|i| i.0).collect();
                let bounds = ids.iter().filter_map(|&id| self.doc().world_bounds(id)).reduce(|a, b| a.union(b)).unwrap_or_default();
                let targets = snap::targets(self.doc(), &ids);
                self.drag = Some(Drag::Move { start: page, items, started: false, bounds, targets });
            }
            None => {
                if !shift {
                    self.selection.clear();
                }
                self.drag = Some(Drag::Marquee { start: page, now: page });
            }
        }
        Task::none()
    }

    fn is_vector(&self, id: NodeId) -> bool {
        self.doc().node(id).is_some_and(|n| matches!(n.kind, NodeKind::Vector { .. })) && !self.doc().is_inside_instance(id)
    }

    /// The page corners of a layer, on screen (rotated layers aren't boxes on screen).
    pub fn screen_corners(&self, id: NodeId) -> Option<[Point; 4]> {
        let n = self.doc().node(id)?;
        let t = self.doc().world_transform(id);
        let (w, h) = (n.props.width, n.props.height);
        Some([(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|(x, y)| self.camera.to_screen(t * kurbo::Point::new(x, y))))
    }

    /// Maps a layer's own space to the screen.
    pub fn node_to_screen(&self, id: NodeId) -> Affine {
        let c = self.camera;
        Affine::scale(c.zoom) * Affine::translate(-c.origin.to_vec2()) * self.doc().world_transform(id)
    }

    /// Maps a vector's anchor space to the screen.
    pub fn anchors_to_screen(&self, id: NodeId) -> Option<Affine> {
        let n = self.doc().node(id)?;
        let NodeKind::Vector { path } = &n.kind else { return None };
        Some(self.node_to_screen(id) * pen::base_scale(n, path))
    }

    fn anchor_at(&self, id: NodeId, at: Point, selected: Option<(usize, usize)>) -> Option<(usize, usize, Part)> {
        let NodeKind::Vector { path } = &self.doc().node(id)?.kind else { return None };
        pen::hit(path, self.anchors_to_screen(id)?, kurbo::Point::new(at.x as f64, at.y as f64), selected, 6.0)
    }

    /// The rotate zone just outside a corner of the single selected layer.
    pub fn rotate_at(&self, at: Point) -> Option<NodeId> {
        let [id] = self.selection[..] else { return None };
        let n = self.doc().node(id)?;
        let local = self.doc().world_transform(id).inverse() * self.camera.to_page(at);
        if (0.0..=n.props.width).contains(&local.x) && (0.0..=n.props.height).contains(&local.y) {
            return None;
        }
        let corners = self.screen_corners(id)?;
        corners.iter().any(|c| (7.0..=22.0).contains(&c.distance(at))).then_some(id)
    }

    // ----- Pen ---------------------------------------------------------------------------

    fn pen_press(&mut self, page: kurbo::Point, at: Point) {
        let close = self.pen.as_ref().is_some_and(|p| {
            p.anchors.len() >= 2 && self.camera.to_screen(kurbo::Point::new(p.anchors[0].p[0], p.anchors[0].p[1])).distance(at) <= 8.0
        });
        if close {
            self.finish_pen(true);
            return;
        }
        let pen = self.pen.get_or_insert_with(PenPath::default);
        pen.anchors.push(jd_vector::Anchor::corner(page.x.round(), page.y.round()));
        pen.dragging = true;
    }

    /// Turns the Pen's path into a vector layer (two points or more).
    pub fn finish_pen(&mut self, closed: bool) {
        let Some(pen) = self.pen.take() else { return };
        if pen.anchors.len() < 2 {
            return;
        }
        let first = kurbo::Point::new(pen.anchors[0].p[0], pen.anchors[0].p[1]);
        let parent = self.frame_at(first);
        let to_local = self.doc().world_transform(parent).inverse();
        let (x, y, path) = pen::vector_from(&pen.anchors, closed, to_local);
        let n = self.doc().children(parent).len() + 1;
        let spec = NodeSpec::vector(&format!("Vector {n}"), path).at(round2(x), round2(y));
        if let Some(done) = self.run("Pen", &[Op::create(parent, spec)]) {
            self.select(done.created[0], false);
            self.tool = Tool::Move;
        }
    }

    /// The frame new layers drawn at `p` go into (else the page).
    fn frame_at(&self, p: kurbo::Point) -> NodeId {
        let doc = self.doc();
        doc.nodes_at(self.page, p, 0.0)
            .into_iter()
            .find(|&id| doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Frame | NodeKind::Section | NodeKind::Component { .. })) && !doc.is_inside_instance(id))
            .unwrap_or(self.page)
    }

    fn is_text(&self, id: NodeId) -> bool {
        self.doc().node(id).is_some_and(|n| matches!(n.kind, NodeKind::Text { .. }))
    }

    fn start_text_edit(&mut self, id: NodeId) -> Task<Message> {
        self.commit_text();
        let Some(NodeKind::Text { text }) = self.doc().node(id).map(|n| &n.kind) else { return Task::none() };
        let mut content = text_editor::Content::with_text(&text.content);
        content.perform(text_editor::Action::SelectAll);
        self.select(id, false);
        self.text_edit = Some((id, content));
        self.drag = None;
        operation::focus(TEXT_EDITOR)
    }

    /// Writes the typed text to the layer (one undo step); empty text removes the layer.
    pub fn commit_text(&mut self) {
        let Some((id, content)) = self.text_edit.take() else { return };
        let mut typed = content.text();
        if typed.ends_with('\n') {
            typed.pop();
        }
        let Some(NodeKind::Text { text }) = self.doc().node(id).map(|n| &n.kind) else { return };
        if typed.trim().is_empty() {
            if self.run("Delete empty text", &[Op::Delete { id: id.into() }]).is_some() {
                self.selection.retain(|&s| s != id);
            }
        } else if typed != text.content {
            self.run("Edit text", &[Op::set(id, Patch { text: Some(typed), ..Patch::default() })]);
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
        if self.tool == Tool::Pen
            && let Some(pen) = &mut self.pen
        {
            if pen.dragging
                && let Some(last) = pen.anchors.last_mut()
            {
                let a = kurbo::Point::new(last.p[0], last.p[1]);
                if (page - a).hypot() * self.camera.zoom > 2.0 {
                    let mirror = a + (a - page);
                    last.handle_out = Some([page.x, page.y]);
                    last.handle_in = Some([mirror.x, mirror.y]);
                }
            }
            pen.cursor = Some(page);
            return;
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
            Drag::Move { start, items, bounds, targets, .. } => {
                if items.is_empty() {
                    if !self.selection.is_empty() {
                        self.error("Auto layout places this layer; change its order in the Layers panel".into());
                    }
                    return;
                }
                let mut d = page - start;
                // Shift keeps to one axis.
                if self.modifiers.shift() {
                    if d.x.abs() > d.y.abs() {
                        d.y = 0.0;
                    } else {
                        d.x = 0.0;
                    }
                }
                // Snap to whole pixels first, then to nearby layers (Ctrl turns snapping off).
                d = Vec2::new((bounds.x0 + d.x).round() - bounds.x0, (bounds.y0 + d.y).round() - bounds.y0);
                self.guides.clear();
                if !self.modifiers.command() {
                    let s = snap::snap(bounds + d, Edges::ALL, &targets, SNAP / self.camera.zoom);
                    d += Vec2::new(s.dx, s.dy);
                    self.guides = s.guides;
                }
                let ops: Vec<Op> = items
                    .iter()
                    .map(|(id, x, y, to_parent)| {
                        let ld = to_parent.as_coeffs();
                        // Only the linear part applies to a displacement.
                        let dx = ld[0] * d.x + ld[2] * d.y;
                        let dy = ld[1] * d.x + ld[3] * d.y;
                        Op::set(*id, Patch { x: Some(round2(x + dx)), y: Some(round2(y + dy)), ..Patch::default() })
                    })
                    .collect();
                self.run_gesture("Move", &ops);
            }
            Drag::Resize { id, handle, start, rect, to_parent, targets, .. } => {
                let c = to_parent.as_coeffs();
                let mut d = page - start;
                self.guides.clear();
                // Snap the dragged edges (not with Shift's fixed ratio, or Ctrl).
                if !self.modifiers.shift() && !self.modifiers.command() {
                    let from_parent = to_parent.inverse();
                    let moved = resize_rect(rect, handle, local_vec(c, d), false);
                    let pr = from_parent.transform_rect_bbox(moved);
                    let (ax, ay) = handle.anchor();
                    let edges = Edges { x: [ax == 0.0, false, ax == 1.0], y: [ay == 0.0, false, ay == 1.0] };
                    let s = snap::snap(pr, edges, &targets, SNAP / self.camera.zoom);
                    d += Vec2::new(s.dx, s.dy);
                    self.guides = s.guides;
                }
                let r = resize_rect(rect, handle, local_vec(c, d), self.modifiers.shift());
                let ops = [
                    Op::Resize { id: id.into(), width: r.width().round(), height: r.height().round() },
                    Op::set(id, Patch { x: Some(r.x0.round()), y: Some(r.y0.round()), ..Patch::default() }),
                ];
                self.run_gesture("Resize", &ops);
            }
            Drag::Marquee { start, .. } => self.drag = Some(Drag::Marquee { start, now: page }),
            Drag::Rotate { id, center_page, center_parent, a0, rot0, half } => {
                let a = (page - center_page).atan2();
                let mut rot = rot0 - (a - a0).to_degrees();
                if self.modifiers.shift() {
                    rot = (rot / 15.0).round() * 15.0;
                }
                rot = round2(((rot + 180.0).rem_euclid(360.0)) - 180.0);
                let off = Affine::rotate(-rot.to_radians()) * half.to_point();
                let at = center_parent - off.to_vec2();
                self.run_gesture("Rotate", &[Op::set(id, Patch { rotation: Some(rot), x: Some(round2(at.x)), y: Some(round2(at.y)), ..Patch::default() })]);
            }
            Drag::Anchor { id, sub, idx, part } => {
                let Some(n) = self.doc().node(id) else { return };
                let NodeKind::Vector { path } = &n.kind else { return };
                let to_anchor = (self.doc().world_transform(id) * pen::base_scale(n, path)).inverse();
                let q = to_anchor * page;
                let path = pen::moved(path, sub, idx, part, [round2(q.x), round2(q.y)]);
                self.run_gesture("Edit points", &[Op::set(id, Patch { path: Some(path), ..Patch::default() })]);
            }
            Drag::Create { tool, start, .. } => self.drag = Some(Drag::Create { tool, start, now: page }),
        }
    }

    fn release(&mut self, at: Point) -> Task<Message> {
        let page = self.camera.to_page(at);
        if let Some(pen) = &mut self.pen {
            pen.dragging = false;
        }
        let Some(drag) = self.drag.take() else { return Task::none() };
        match drag {
            Drag::Move { .. } | Drag::Resize { .. } | Drag::Rotate { .. } => {
                self.ed.end_gesture();
                self.guides.clear();
            }
            Drag::Anchor { id, .. } => {
                // Fit the layer's box to the moved points, in the same undo step.
                if let Some(patch) = self.doc().node(id).and_then(pen::refit) {
                    self.run_gesture("Edit points", &[Op::set(id, patch)]);
                }
                self.ed.end_gesture();
            }
            Drag::Pan { .. } => {}
            Drag::Marquee { start, .. } => {
                let r = Rect::from_points(start, page);
                if r.width() * self.camera.zoom < 3.0 && r.height() * self.camera.zoom < 3.0 {
                    return Task::none();
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
            Drag::Create { tool, start, .. } => return self.create(tool, start, page),
        }
        Task::none()
    }

    fn create(&mut self, tool: Tool, start: kurbo::Point, end: kurbo::Point) -> Task<Message> {
        let mut r = Rect::from_points(start, end);
        let clicked = r.width() * self.camera.zoom < 3.0 && r.height() * self.camera.zoom < 3.0;
        if clicked {
            r = Rect::from_origin_size(start, (100.0, 100.0));
        }
        // Into the frame under the start point, like Figma.
        let parent = self.frame_at(start);
        let doc = self.doc();
        let to_local = doc.world_transform(parent).inverse();
        let at = to_local * kurbo::Point::new(r.x0, r.y0);
        let (w, h) = (r.width().round(), r.height().round());
        let n = doc.children(parent).len() + 1;
        let spec = match tool {
            Tool::Frame => NodeSpec::frame(&format!("Frame {n}"), w, h),
            Tool::Rectangle => NodeSpec::rect(&format!("Rectangle {n}"), w, h),
            Tool::Ellipse => NodeSpec::ellipse(&format!("Ellipse {n}"), w, h),
            Tool::Text => NodeSpec::text("Text", "Text", TextStyle { size: 16.0, ..TextStyle::default() }),
            Tool::Move | Tool::Hand | Tool::Pen => return Task::none(),
        }
        .at(at.x.round(), at.y.round());
        if let Some(done) = self.run(&format!("Add {}", tool.tip().split(' ').next().unwrap_or("layer")), &[Op::create(parent, spec)]) {
            self.select(done.created[0], false);
            self.tool = Tool::Move;
            if tool == Tool::Text {
                return self.start_text_edit(done.created[0]);
            }
        }
        Task::none()
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
        app.select(r, false);
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
        app.select(r, false);
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

    fn three_rects(app: &mut Blobr) -> Vec<NodeId> {
        let p = app.page;
        app.run(
            "Add",
            &[
                Op::create(p, NodeSpec::rect("A", 50.0, 50.0).at(100.0, 100.0)),
                Op::create(p, NodeSpec::rect("B", 50.0, 50.0).at(300.0, 100.0)),
                Op::create(p, NodeSpec::rect("C", 50.0, 50.0).at(100.0, 300.0)),
            ],
        )
        .unwrap()
        .created
    }

    fn drag(app: &mut Blobr, from: Point, to: Point) {
        let _ = app.update(Message::CanvasPress { at: from, button: mouse::Button::Left });
        let _ = app.update(Message::CanvasMove(to));
        let _ = app.update(Message::CanvasRelease(to));
    }

    #[test]
    fn moving_snaps_to_neighbours_with_guides_unless_ctrl() {
        let mut app = app();
        let ids = three_rects(&mut app);
        // Drag B so its top lands 4px below A's top: snaps back in line.
        let _ = app.update(Message::CanvasPress { at: Point::new(320.0, 120.0), button: mouse::Button::Left });
        let _ = app.update(Message::CanvasMove(Point::new(340.0, 124.0)));
        assert!(!app.guides.is_empty(), "a guide shows while snapped");
        let _ = app.update(Message::CanvasRelease(Point::new(340.0, 124.0)));
        let b = &app.doc().node(ids[1]).unwrap().props;
        assert_eq!((b.x, b.y), (320.0, 100.0));
        assert!(app.guides.is_empty());

        app.modifiers = keyboard::Modifiers::CTRL;
        drag(&mut app, Point::new(330.0, 120.0), Point::new(330.0, 124.0));
        assert_eq!(app.doc().node(ids[1]).unwrap().props.y, 104.0, "Ctrl turns snapping off");
    }

    #[test]
    fn group_frame_ungroup_and_restack_from_the_app() {
        let mut app = app();
        let ids = three_rects(&mut app);
        app.selection = vec![ids[0], ids[1]];
        let _ = app.update(Message::Group);
        let g = app.selection[0];
        assert!(matches!(app.doc().node(g).unwrap().kind, NodeKind::Group));
        assert_eq!(app.doc().children(g), &ids[..2]);
        let _ = app.update(Message::Ungroup);
        assert_eq!(app.selection, ids[..2].to_vec());
        assert!(!app.doc().contains(g));

        app.selection = vec![ids[0]];
        let _ = app.update(Message::Reorder(Order::Front));
        assert_eq!(app.doc().children(app.page).last(), Some(&ids[0]));

        app.selection = vec![ids[2], ids[0]];
        let _ = app.update(Message::FrameSelection);
        let f = app.doc().node(app.selection[0]).unwrap();
        assert_eq!((f.props.x, f.props.y, f.props.width, f.props.height), (100.0, 100.0, 50.0, 250.0));
    }

    #[test]
    fn dragging_layers_in_the_panel_and_renaming() {
        let mut app = app();
        let ids = three_rects(&mut app);
        let p = app.page;
        let f = app.run("Frame", &[Op::create(p, NodeSpec::frame("F", 400.0, 400.0).at(50.0, 50.0))]).unwrap().created[0];
        // A into F (middle of its row): stays where it is on the page.
        let _ = app.update(Message::LayerPress(ids[0]));
        let _ = app.update(Message::LayerHover(f, crate::view::LAYER_ROW / 2.0));
        let _ = app.update(Message::LayerRelease);
        assert_eq!(app.doc().parent(ids[0]), Some(f));
        assert_eq!(app.doc().world_bounds(ids[0]).unwrap().x0, 100.0);
        // C above B (top of B's row): one higher in the stack than B.
        let _ = app.update(Message::LayerPress(ids[2]));
        let _ = app.update(Message::LayerHover(ids[1], 2.0));
        let _ = app.update(Message::LayerRelease);
        let kids = app.doc().children(p);
        assert_eq!(kids.iter().position(|&c| c == ids[2]), Some(kids.iter().position(|&c| c == ids[1]).unwrap() + 1));
        // Can't drop a frame into its own child.
        let _ = app.update(Message::LayerPress(f));
        let _ = app.update(Message::LayerHover(ids[0], 13.0));
        assert_eq!(app.layer_drag.unwrap().target, None);
        let _ = app.update(Message::LayerRelease);

        let _ = app.update(Message::StartRename(ids[1]));
        let _ = app.update(Message::RenameInput("Card".into()));
        let _ = app.update(Message::RenameSubmit);
        assert_eq!(app.doc().node(ids[1]).unwrap().name, "Card");
    }

    #[test]
    fn unsaved_changes_are_asked_about() {
        let mut app = app();
        three_rects(&mut app);
        let _ = app.update(Message::New);
        assert_eq!(app.confirm, Some(After::New), "dirty: ask first");
        assert_eq!(app.doc().children(app.page).len(), 3);
        let _ = app.update(Message::ConfirmCancel);
        assert_eq!(app.confirm, None);
        assert_eq!(app.doc().children(app.page).len(), 3);
        let _ = app.update(Message::Quit);
        assert_eq!(app.confirm, Some(After::Quit));
        let _ = app.update(Message::ConfirmCancel);
        let _ = app.update(Message::New);
        let _ = app.update(Message::ConfirmDiscard);
        assert!(app.doc().children(app.page).is_empty(), "discarded");
        let _ = app.update(Message::New);
        assert_eq!(app.confirm, None, "clean: no question");
    }

    #[test]
    fn copy_paste_in_place_into_frames_and_cut() {
        let mut app = app();
        let ids = three_rects(&mut app);
        let p = app.page;
        // In place, on top, selected.
        app.selection = vec![ids[0]];
        let _ = app.update(Message::Copy);
        let _ = app.update(Message::Paste);
        let _ = app.update(Message::Pasted(None));
        let copy = app.selection[0];
        assert_ne!(copy, ids[0]);
        assert_eq!(app.doc().world_bounds(copy), app.doc().world_bounds(ids[0]));
        assert_eq!(app.doc().children(p).iter().position(|&c| c == copy), Some(1), "just above the original");

        // Into a selected frame: centred in it.
        let f = app.run("Frame", &[Op::create(p, NodeSpec::frame("F", 200.0, 200.0).at(500.0, 500.0))]).unwrap().created[0];
        app.selection = vec![f];
        let _ = app.update(Message::Pasted(None));
        let inside = app.selection[0];
        assert_eq!(app.doc().parent(inside), Some(f));
        assert_eq!(app.doc().world_bounds(inside).unwrap().center(), kurbo::Point::new(600.0, 600.0));

        // Cut removes; pasting brings it back where it was.
        app.selection = vec![ids[2]];
        let _ = app.update(Message::Cut);
        assert!(!app.doc().contains(ids[2]));
        app.selection.clear();
        app.camera = Camera { origin: kurbo::Point::new(0.0, 0.0), zoom: 1.0 };
        let _ = app.update(Message::Pasted(None));
        assert_eq!(app.doc().world_bounds(app.selection[0]).unwrap().origin(), kurbo::Point::new(100.0, 300.0));

        // Text from another app.
        app.selection.clear();
        let _ = app.update(Message::Pasted(Some(Pasted::Text("Hello".into()))));
        assert!(matches!(&app.doc().node(app.selection[0]).unwrap().kind, NodeKind::Text { text } if text.content == "Hello"));
    }

    #[test]
    fn align_and_edit_many_layers_at_once() {
        let mut app = app();
        let ids = three_rects(&mut app);
        app.selection = ids.clone();
        let _ = app.update(Message::Align(Align::Left));
        assert!(ids.iter().all(|&id| app.doc().node(id).unwrap().props.x == 100.0));
        let _ = app.update(Message::FieldInput(Field::W, "80".into()));
        let _ = app.update(Message::FieldSubmit(Field::W));
        assert!(ids.iter().all(|&id| app.doc().node(id).unwrap().props.width == 80.0));
        let _ = app.update(Message::FieldInput(Field::Fill(0), "#FF0000".into()));
        let _ = app.update(Message::FieldSubmit(Field::Fill(0)));
        assert!(ids.iter().all(|&id| app.doc().node(id).unwrap().props.fills == vec![Paint::hex("#FF0000")]));
        assert_eq!(app.ed.history.labels().count(), 4, "add, align, resize, fill: one step each");
    }

    #[test]
    fn typing_text_on_the_canvas() {
        use std::sync::Arc;
        use text_editor::{Action, Edit};
        let mut app = app();
        let _ = app.update(Message::Tool(Tool::Text));
        drag(&mut app, Point::new(50.0, 50.0), Point::new(50.0, 50.0));
        let t = app.selection[0];
        assert!(app.text_edit.is_some(), "a new text layer is typed into straight away");
        let _ = app.update(Message::TextAction(Action::Edit(Edit::Paste(Arc::new("Hello there".into())))));
        let _ = app.update(Message::EndTextEdit);
        assert!(matches!(&app.doc().node(t).unwrap().kind, NodeKind::Text { text } if text.content == "Hello there"));

        // Double-click to edit again; clearing it removes the layer.
        let _ = app.update(Message::EditText(t));
        let _ = app.update(Message::TextAction(Action::Edit(Edit::Delete)));
        let _ = app.update(Message::CanvasPress { at: Point::new(700.0, 500.0), button: mouse::Button::Left });
        let _ = app.update(Message::CanvasRelease(Point::new(700.0, 500.0)));
        assert!(!app.doc().contains(t));
    }

    fn click(app: &mut Blobr, at: Point) {
        let _ = app.update(Message::CanvasPress { at, button: mouse::Button::Left });
        let _ = app.update(Message::CanvasRelease(at));
    }

    #[test]
    fn the_pen_draws_closed_shapes_and_curves() {
        let mut app = app();
        let _ = app.update(Message::Tool(Tool::Pen));
        click(&mut app, Point::new(100.0, 100.0));
        click(&mut app, Point::new(200.0, 100.0));
        click(&mut app, Point::new(150.0, 180.0));
        click(&mut app, Point::new(101.0, 101.0)); // back on the first point: closes
        let v = app.selection[0];
        let n = app.doc().node(v).unwrap();
        assert_eq!((n.props.x, n.props.y, n.props.width, n.props.height), (100.0, 100.0, 100.0, 80.0));
        assert!(matches!(&n.kind, NodeKind::Vector { path } if path.subpaths[0].closed && path.subpaths[0].anchors.len() == 3));
        assert_eq!(app.tool, Tool::Move);

        // Dragging pulls out handles; Enter leaves the path open.
        let _ = app.update(Message::Tool(Tool::Pen));
        click(&mut app, Point::new(300.0, 300.0));
        let _ = app.update(Message::CanvasPress { at: Point::new(400.0, 300.0), button: mouse::Button::Left });
        let _ = app.update(Message::CanvasMove(Point::new(400.0, 350.0)));
        let _ = app.update(Message::CanvasRelease(Point::new(400.0, 350.0)));
        let _ = app.update(Message::Key(keyboard::Event::KeyPressed {
            key: Key::Named(Named::Enter),
            modified_key: Key::Named(Named::Enter),
            physical_key: keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        }));
        let curve = app.selection[0];
        let NodeKind::Vector { path } = &app.doc().node(curve).unwrap().kind else { panic!("a vector") };
        assert!(!path.subpaths[0].closed);
        assert!(path.subpaths[0].anchors[1].handle_in.is_some(), "the drag made a smooth point");
        assert!(app.doc().node(curve).unwrap().props.height > 0.0, "the curve bulges, so the box has height");
    }

    #[test]
    fn rotating_turns_about_the_centre() {
        let mut app = app();
        let p = app.page;
        let r = app.run("Add", &[Op::create(p, NodeSpec::rect("R", 100.0, 50.0).at(100.0, 100.0))]).unwrap().created[0];
        app.select(r, false);
        // Just outside the bottom-right corner (200, 150), then round to the right of the centre.
        let start = Point::new(210.0, 160.0);
        assert_eq!(app.rotate_at(start), Some(r));
        let _ = app.update(Message::CanvasPress { at: start, button: mouse::Button::Left });
        // Quarter turn clockwise about the centre (150, 125).
        let (dx, dy) = (start.x - 150.0, start.y - 125.0);
        let to = Point::new(150.0 - dy, 125.0 + dx);
        let _ = app.update(Message::CanvasMove(to));
        let _ = app.update(Message::CanvasRelease(to));
        let n = app.doc().node(r).unwrap();
        assert!((n.props.rotation + 90.0).abs() < 0.01, "clockwise is negative: {}", n.props.rotation);
        let c = app.doc().world_bounds(r).unwrap().center();
        assert!((c.x - 150.0).abs() < 0.02 && (c.y - 125.0).abs() < 0.02, "centre stays: {c:?}");
        let _ = app.update(Message::Undo);
        assert_eq!(app.doc().node(r).unwrap().props.rotation, 0.0, "one undo step");
    }

    #[test]
    fn editing_vector_points_refits_the_layer() {
        let mut app = app();
        let _ = app.update(Message::Tool(Tool::Pen));
        click(&mut app, Point::new(100.0, 100.0));
        click(&mut app, Point::new(200.0, 100.0));
        click(&mut app, Point::new(150.0, 200.0));
        click(&mut app, Point::new(100.0, 100.0));
        let v = app.selection[0];
        let _ = app.update(Message::Key(keyboard::Event::KeyPressed {
            key: Key::Named(Named::Enter),
            modified_key: Key::Named(Named::Enter),
            physical_key: keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        }));
        assert_eq!(app.vector_edit.map(|e| e.id), Some(v));
        // Drag the bottom point further down: the box grows to fit.
        let _ = app.update(Message::CanvasPress { at: Point::new(150.0, 200.0), button: mouse::Button::Left });
        assert_eq!(app.vector_edit.unwrap().selected, Some((0, 2)));
        let _ = app.update(Message::CanvasMove(Point::new(150.0, 260.0)));
        let _ = app.update(Message::CanvasRelease(Point::new(150.0, 260.0)));
        let n = app.doc().node(v).unwrap();
        assert_eq!((n.props.y, n.props.height), (100.0, 160.0));
        let _ = app.update(Message::Undo);
        assert_eq!(app.doc().node(v).unwrap().props.height, 100.0, "move and refit are one step");
    }

    #[test]
    fn booleans_and_flatten_from_the_app() {
        let mut app = app();
        let ids = three_rects(&mut app);
        let p = app.page;
        let big = app.run("Add", &[Op::create(p, NodeSpec::rect("Big", 300.0, 300.0).at(80.0, 80.0))]).unwrap().created[0];
        // Subtract cuts the upper layers out of the bottom one: put Big at the bottom.
        app.selection = vec![big];
        let _ = app.update(Message::Reorder(Order::Back));
        app.selection = vec![big, ids[0]];
        let _ = app.update(Message::Boolean(BoolOp::Subtract));
        let r = app.selection[0];
        assert!(matches!(app.doc().node(r).unwrap().kind, NodeKind::Vector { .. }));
        assert!(!app.doc().contains(big) && !app.doc().contains(ids[0]));
        // A boolean of separate layers that don't overlap is refused with a message.
        app.selection = vec![ids[1]];
        let _ = app.update(Message::Boolean(BoolOp::Union));
        assert!(app.status.error);
        let _ = app.update(Message::Flatten);
        assert!(matches!(app.doc().node(app.selection[0]).unwrap().kind, NodeKind::Vector { .. }));
    }

    #[test]
    fn components_properties_assets_and_variants() {
        let mut app = app();
        let p = app.page;
        let done = app
            .run(
                "Add",
                &[
                    Op::create_as("btn", p, NodeSpec::frame("Button", 120.0, 40.0).at(0.0, 0.0)),
                    Op::create_as("label", "btn", NodeSpec::text("Label", "OK", TextStyle::default()).at(10.0, 10.0)),
                ],
            )
            .unwrap();
        let (btn, label) = (done.id("btn").unwrap(), done.id("label").unwrap());
        app.selection = vec![btn];
        let _ = app.update(Message::CreateComponent);
        let _ = app.update(Message::AddProperty("text"));
        let _ = app.update(Message::FieldInput(Field::PropDefault("Text".into()), "Go".into()));
        let _ = app.update(Message::FieldSubmit(Field::PropDefault("Text".into())));
        app.select(label, false);
        assert_eq!(app.owning_component(label), Some(btn));
        let _ = app.update(Message::LinkProperty(PropLink::Text, Some("Text".into())));

        // Assets lists it; clicking places an instance whose label follows the property.
        let _ = app.update(Message::LeftTab(1));
        assert_eq!(app.components().iter().map(|c| c.0).collect::<Vec<_>>(), vec![btn]);
        assert!(app.thumbnails.contains_key(&btn), "a picture for the Assets panel");
        app.selection.clear();
        let _ = app.update(Message::InsertComponent(btn));
        let inst = app.selection[0];
        let text_in = |app: &Blobr| {
            app.doc().descendants(inst).into_iter().find_map(|d| match &app.doc().node(d)?.kind {
                NodeKind::Text { text } => Some(text.content.clone()),
                _ => None,
            })
        };
        assert_eq!(text_in(&app).as_deref(), Some("Go"));
        let _ = app.update(Message::FieldInput(Field::Prop("Text".into()), "Send".into()));
        let _ = app.update(Message::FieldSubmit(Field::Prop("Text".into())));
        assert_eq!(text_in(&app).as_deref(), Some("Send"));

        // A second component, combined with the first into a set; then swap the instance.
        let other = app.run("Add", &[Op::create(p, NodeSpec::frame("Ghost", 120.0, 40.0).at(0.0, 100.0))]).unwrap().created[0];
        app.selection = vec![other];
        let _ = app.update(Message::CreateComponent);
        app.selection = vec![btn, other];
        let _ = app.update(Message::CombineVariants);
        let set = app.selection[0];
        assert!(matches!(app.doc().node(set).unwrap().kind, NodeKind::ComponentSet { .. }), "{}", app.status.text);
        assert_eq!(app.doc().node(btn).unwrap().name, "Property 1=Button");
        app.selection = vec![inst];
        let _ = app.update(Message::SwapComponent(other));
        assert!(matches!(app.doc().node(inst).unwrap().kind, NodeKind::Instance { component, .. } if component == other));
    }

    #[test]
    fn variables_modes_and_styles() {
        let mut app = app();
        let p = app.page;
        let done = app
            .run(
                "Add",
                &[
                    Op::create_as("f", p, NodeSpec::frame("Screen", 200.0, 200.0)),
                    Op::create_as("r", "f", NodeSpec::rect("Card", 50.0, 50.0)),
                ],
            )
            .unwrap();
        let (f, r) = (done.id("f").unwrap(), done.id("r").unwrap());
        let _ = app.update(Message::ShowVariables(true));
        let _ = app.update(Message::AddVariable("color"));
        let c = app.var_collection.expect("a collection was made");
        let _ = app.update(Message::AddMode);
        let var = *app.doc().tokens().variables.keys().next().unwrap();
        let _ = app.update(Message::FieldInput(Field::VarValue(var, "Mode 2".into()), "#111111".into()));
        let _ = app.update(Message::FieldSubmit(Field::VarValue(var, "Mode 2".into())));
        let _ = app.update(Message::ShowVariables(false));

        let fill = |app: &Blobr| app.doc().node(r).unwrap().props.fills[0].clone();
        app.select(r, false);
        let _ = app.update(Message::BindFill(0, Some(var)));
        assert_eq!(fill(&app), Paint::hex("#4C8DFF"), "the first mode's value");
        app.select(f, false);
        let _ = app.update(Message::SetMode(c, Some("Mode 2".into())));
        assert_eq!(fill(&app), Paint::hex("#111111"), "the frame's mode applies inside it");

        // A style made from a fill, in one undo step; renaming it.
        app.select(r, false);
        let _ = app.update(Message::BindFill(0, None));
        let steps = app.ed.history.labels().count();
        let _ = app.update(Message::CreateFillStyle);
        assert_eq!(app.ed.history.labels().count(), steps + 1);
        let sid = app.doc().node(r).unwrap().props.styles.fill.expect("uses the new style");
        app.selection.clear();
        let _ = app.update(Message::FieldInput(Field::StyleName(sid), "Brand/Dark".into()));
        let _ = app.update(Message::FieldSubmit(Field::StyleName(sid)));
        assert_eq!(app.doc().tokens().styles[&sid].name, "Brand/Dark");
    }

    #[test]
    fn number_bindings_effects_swap_props_and_libraries() {
        let mut app = app();
        let p = app.page;
        let f = app.run("Add", &[Op::create(p, NodeSpec::frame("Row", 200.0, 50.0))]).unwrap().created[0];
        app.select(f, false);
        let _ = app.update(Message::ToggleAutoLayout);
        let _ = app.update(Message::ShowVariables(true));
        let _ = app.update(Message::AddVariable("number"));
        let _ = app.update(Message::ShowVariables(false));
        let var = *app.doc().tokens().variables.keys().next().unwrap();
        app.select(f, false);
        let _ = app.update(Message::Bind("gap".into(), Some(var)));
        assert_eq!(app.doc().node(f).unwrap().props.layout.as_ref().unwrap().gap, 8.0, "the variable's value");

        // Effects: add, edit, make a style.
        let _ = app.update(Message::AddEffect);
        let _ = app.update(Message::FieldInput(Field::Effect(0, "blur"), "12".into()));
        let _ = app.update(Message::FieldSubmit(Field::Effect(0, "blur")));
        assert!(matches!(&app.doc().node(f).unwrap().props.effects[0], jd_vector::Effect::DropShadow(s) if s.blur == 12.0));
        let _ = app.update(Message::SetEffectKind(0, "blur"));
        assert!(matches!(app.doc().node(f).unwrap().props.effects[0], jd_vector::Effect::LayerBlur { radius, .. } if radius == 12.0));
        let _ = app.update(Message::CreateEffectStyle);
        assert!(app.doc().node(f).unwrap().props.styles.effect.is_some());

        // A library file with a component; placing it twice copies it in once.
        let mut lib = Document::new();
        let lp = lib.pages()[0];
        let done = lib
            .apply(&[Op::create_as("b", lp, NodeSpec::frame("Badge", 30.0, 30.0)), Op::MakeComponent { id: "b".into() }], &jd_vector::ApproxMeasure)
            .unwrap();
        let badge = done.id("b").unwrap();
        let dir = std::env::temp_dir().join(format!("blobr-lib-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lib.jdb");
        file::save(&lib, &path).unwrap();
        let _ = app.update(Message::LibraryPicked(Some(path)));
        assert_eq!(app.libraries.len(), 1);
        app.selection.clear();
        let _ = app.update(Message::InsertLibraryComponent(0, badge));
        let _ = app.update(Message::InsertLibraryComponent(0, badge));
        let lib_page = *app.doc().pages().iter().find(|&&pg| app.doc().node(pg).unwrap().name == LIBRARY_PAGE).unwrap();
        assert_eq!(app.doc().children(lib_page).len(), 1, "copied in once");
        let local = app.doc().children(lib_page)[0];
        let instances = app.doc().descendants(p).into_iter().filter(|&d| matches!(app.doc().node(d).unwrap().kind, NodeKind::Instance { component, .. } if component == local)).count();
        assert_eq!(instances, 2);
        let _ = std::fs::remove_dir_all(dir);

        // An instance-swap property defaults to another component.
        let host = app.run("Add", &[Op::create(p, NodeSpec::frame("Card", 100.0, 100.0).at(400.0, 0.0))]).unwrap().created[0];
        app.selection = vec![host];
        let _ = app.update(Message::CreateComponent);
        let _ = app.update(Message::AddProperty("swap"));
        assert!(matches!(&app.doc().node(host).unwrap().kind, NodeKind::Component { properties, .. } if matches!(properties.get("Icon"), Some(PropDef::InstanceSwap { default }) if *default == local)));
    }

    #[test]
    fn prototype_links_present_and_inspect() {
        let mut app = app();
        let p = app.page;
        let done = app
            .run(
                "Add",
                &[
                    Op::create_as("a", p, NodeSpec::frame("Home", 400.0, 300.0)),
                    Op::create_as("btn", "a", NodeSpec::rect("Button", 100.0, 40.0).at(150.0, 130.0)),
                    Op::create_as("b", p, NodeSpec::frame("Details", 400.0, 300.0).at(500.0, 0.0)),
                ],
            )
            .unwrap();
        let (a, btn, b) = (done.id("a").unwrap(), done.id("btn").unwrap(), done.id("b").unwrap());
        app.select(btn, false);
        let _ = app.update(Message::RightTab(1));
        let _ = app.update(Message::AddReaction);
        let r = &app.doc().node(btn).unwrap().props.reactions[0];
        assert_eq!(r.action, jd_vector::Action::Navigate { to: b }, "links to the next frame by default");
        let _ = app.update(Message::SetTransition(0, jd_vector::Transition::SlideLeft));
        // Details goes back on its own after half a second.
        app.select(b, false);
        let _ = app.update(Message::AddReaction);
        let _ = app.update(Message::SetTrigger(0, "delay"));
        let _ = app.update(Message::SetAction(0, "back"));
        let _ = app.update(Message::FieldInput(Field::Reaction(0, "delay"), "0".into()));
        let _ = app.update(Message::FieldSubmit(Field::Reaction(0, "delay")));

        app.selection.clear();
        let _ = app.update(Message::Present(Some(a)));
        let shot = app.present.as_ref().unwrap().shot.clone().unwrap();
        let o = app.shot_origin(&shot);
        // The middle of the button, on screen.
        let at = Point::new(o.x + (200.0 * shot.fit) as f32, o.y + (150.0 * shot.fit) as f32);
        let _ = app.update(Message::PresentMove(at));
        let _ = app.update(Message::PresentClick);
        let pr = app.present.as_ref().unwrap();
        assert_eq!(pr.frame, b);
        assert!(pr.leaving.is_some(), "sliding");
        assert!(app.present_animating());
        let _ = app.update(Message::PresentTick);
        assert_eq!(app.present.as_ref().unwrap().frame, a, "the timer went back");
        let _ = app.update(Message::Key(keyboard::Event::KeyPressed {
            key: Key::Named(Named::Escape),
            modified_key: Key::Named(Named::Escape),
            physical_key: keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        }));
        assert!(app.present.is_none());

        // Copying both frames keeps the link between the copies.
        app.selection = vec![a, b];
        let _ = app.update(Message::Copy);
        app.selection.clear();
        let _ = app.update(Message::Pasted(None));
        let copies = app.selection.clone();
        let btn2 = app.doc().children(copies[0])[0];
        assert_eq!(app.doc().node(btn2).unwrap().props.reactions[0].action, jd_vector::Action::Navigate { to: copies[1] });

        let css = jd_vector::inspect::css(app.doc(), btn);
        assert!(css.contains(&"position: absolute;".to_string()) && css.contains(&"left: 150px;".to_string()), "{css:?}");
    }

    #[test]
    fn pasting_svg_markup_imports_shapes() {
        let mut app = app();
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" rx="4" fill="#4C8DFF"/></svg>"##;
        let _ = app.update(Message::Pasted(Some(Pasted::Text(svg.into()))));
        let f = app.selection[0];
        let n = app.doc().node(f).unwrap();
        assert_eq!((n.name.as_str(), n.props.width, n.props.height), ("SVG", 40.0, 20.0));
        assert_eq!(app.doc().world_bounds(f).unwrap().center(), app.view_rect().center());
        assert!(matches!(app.doc().node(app.doc().children(f)[0]).unwrap().kind, NodeKind::Vector { .. }));
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
