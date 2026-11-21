use std::collections::HashMap;
use std::path::{Path, PathBuf};

use iced::keyboard::{self, Key, key::Named};
use iced::widget::{image, operation};
use iced::window::{self, Direction};
use iced::{Point, Rectangle, Size, Subscription, Task, Vector, event};
use jd_core::{Hsv, Rgba};
use jd_raster::paint::{BrushParams, Stroke};
use jd_raster::selection::{self, Combine, Selection};
use jd_raster::{BlendMode, BlendSpace, Document, History, LayerId, TileGrid, TileId};

use crate::adjust::{self as adj, Op, OpKind};
use crate::tablet::{self, Phase, TabletMessage};
use crate::tool::Tool;
use crate::transform::{self, Handle};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Zoom {
    Fit,
    Fixed(f32),
}

pub const ZOOM_STEPS: &[f32] = &[
    0.02, 0.03, 0.0625, 0.125, 0.25, 0.333, 0.5, 0.667, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0,
    16.0, 32.0, 64.0,
];

/// Id of the layer-rename text field, so it can be focused.
pub const RENAME_INPUT: &str = "layer-rename";

#[derive(Debug, Clone)]
pub struct BrushSettings {
    pub size: f32,
    pub hardness: f32,
    pub opacity: f32,
    pub flow: f32,
    pub smoothing: f32,
    pub mode: BlendMode,
    pub pressure_size: bool,
    pub pressure_opacity: bool,
}

/// A brush/eraser stroke in progress.
pub struct ActiveStroke {
    doc: u64,
    stroke: Stroke,
    /// Smoothed brush position (doc px); lags the input by the Smoothing setting.
    smoothed: Point,
    last_raw: (Point, f32),
}

/// Where the pen is, when a tablet is in use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pen {
    /// Canvas-local logical px.
    pub position: Point,
    pub down: bool,
    pub eraser: bool,
}

/// One open tab.
pub struct OpenDoc {
    /// Stable identity, so async results (saves) find the right tab even after tabs move.
    pub id: u64,
    pub doc: Document,
    /// Where it came from or was last saved. Only a `.jdp` path is a save target.
    pub path: Option<PathBuf>,
    pub history: History,
    pub zoom: Zoom,
    /// Offset of the document centre from the viewport centre, logical px.
    pub pan: Vector,
    /// Layers panel thumbnails, rebuilt when a layer's tiles change.
    pub thumbs: HashMap<LayerId, (Vec<TileId>, image::Handle)>,
    pub saving: bool,
    /// End of the last stroke in this document, for Shift+click straight lines.
    pub last_paint: Option<Point>,
    /// Marching-ants outline and its drawn geometry, rebuilt when the selection or view changes.
    pub ants: crate::view::Ants,
    /// The selection before the last Deselect, for Select > Reselect.
    pub last_selection: Option<Selection>,
}

impl OpenDoc {
    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    /// The `.jdp` this document saves to without asking, if any.
    pub fn save_path(&self) -> Option<&Path> {
        self.path.as_deref().filter(|p| jd_io::is_document(p))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Rect([f32; 4]),
    Ellipse([f32; 4]),
    Polygon(Vec<(f32, f32)>),
}

pub struct SelectOptions {
    /// Selection mode from the options bar (Shift/Alt at click time override it).
    pub mode: Combine,
    pub tolerance: u8,
    pub contiguous: bool,
    pub all_layers: bool,
}

impl Default for SelectOptions {
    fn default() -> Self {
        Self { mode: Combine::Replace, tolerance: 32, contiguous: true, all_layers: false }
    }
}

/// An open popup menu and where it appears (window coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Popup {
    pub kind: PopupKind,
    pub at: Point,
    /// Index of the open submenu row, if any.
    pub sub: Option<usize>,
}

/// A layer being dragged in the Layers panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerDrag {
    pub doc: u64,
    pub layer: LayerId,
    pub target: Option<DropTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DropTarget {
    /// Onto another row: reorder.
    Layer(LayerId),
    /// Onto a document tab: switches to it; dropping copies the layer there.
    Tab(usize),
    Canvas,
}

/// Small modal dialogs with one or two numeric fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    ImageSize,
    CanvasSize,
    Expand,
    Contract,
    Feather,
    Stroke,
}

pub struct Dialog {
    pub kind: DialogKind,
    /// Width (or the single value).
    pub a: String,
    /// Height.
    pub b: String,
    /// Image Size: keep proportions.
    pub constrain: bool,
    pub aspect: f32,
    /// Canvas Size anchor: column and row, 0..=2.
    pub anchor: (u8, u8),
}

impl Dialog {
    pub fn number(s: &str) -> Option<u32> {
        s.trim().parse().ok().filter(|v| (1..=30_000).contains(v))
    }
}

/// Which dock panels are shown (Window menu).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Panels {
    pub color: bool,
    pub properties: bool,
    pub layers: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Color,
    Properties,
    Layers,
}

/// Edit > Transform shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orient {
    FlipH,
    FlipV,
    Rotate90Cw,
    Rotate90Ccw,
    Rotate180,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupKind {
    /// Right-click on the canvas.
    Context,
    /// Right-click on a toolbar slot: its alternate tools.
    ToolSlot(usize),
    /// A title-bar menu (index into the menu titles).
    MenuBar(usize),
    /// Right-click on a row in the Layers panel.
    Layer(LayerId),
    /// The workspace switcher in the title bar.
    Workspace,
}

/// Panel arrangements from the title bar's workspace switcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workspace {
    /// Every panel.
    Essentials,
    /// Layers and Properties, History open (editing photos).
    Photography,
    /// Color and Layers (painting).
    Painting,
    /// No panels: the most room for the image.
    Minimal,
}

impl Workspace {
    pub const ALL: [Workspace; 4] = [Workspace::Essentials, Workspace::Photography, Workspace::Painting, Workspace::Minimal];

    pub fn name(self) -> &'static str {
        match self {
            Workspace::Essentials => "Essentials",
            Workspace::Photography => "Photography",
            Workspace::Painting => "Painting",
            Workspace::Minimal => "Minimal",
        }
    }
}

pub struct Clip {
    /// Document-sized grid holding the copied pixels at their original position.
    pub pixels: TileGrid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewBackground {
    White,
    Transparent,
    BackgroundColor,
}

pub struct NewDocForm {
    pub name: String,
    pub width: String,
    pub height: String,
    pub preset: Option<usize>,
    pub background: NewBackground,
}

impl NewDocForm {
    /// Parsed size, if valid (1 to 30000 px per side).
    pub fn size(&self) -> Option<(u32, u32)> {
        let w: u32 = self.width.trim().parse().ok()?;
        let h: u32 = self.height.trim().parse().ok()?;
        ((1..=30_000).contains(&w) && (1..=30_000).contains(&h)).then_some((w, h))
    }
}

/// Preset document sizes for the New dialog: (group, name, width, height).
pub const PRESETS: &[(&str, &str, u32, u32)] = &[
    ("Screen", "Full HD", 1920, 1080),
    ("Screen", "4K UHD", 3840, 2160),
    ("Screen", "HD 720p", 1280, 720),
    ("Screen", "Wallpaper 1440p", 2560, 1440),
    ("Social", "Square post", 1080, 1080),
    ("Social", "Portrait post", 1080, 1350),
    ("Social", "Story / Reel", 1080, 1920),
    ("Social", "YouTube thumbnail", 1280, 720),
    ("Print 300 ppi", "A4", 2480, 3508),
    ("Print 300 ppi", "A5", 1748, 2480),
    ("Print 300 ppi", "US Letter", 2550, 3300),
    ("Print 300 ppi", "4×6 in photo", 1200, 1800),
    ("Art", "Icon 512", 512, 512),
    ("Art", "Canvas 4000×3000", 4000, 3000),
];

/// What to do once the user answers "save changes?".
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AfterConfirm {
    CloseTab,
    /// Part of quitting: continue with the next unsaved document, then close the window.
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Confirm {
    pub doc: u64,
    pub then: AfterConfirm,
}

#[derive(Debug, Clone, Copy)]
pub enum Choice {
    Save,
    Discard,
    Cancel,
}

pub struct Photo {
    pub docs: Vec<OpenDoc>,
    pub active: usize,
    next_doc_id: u64,
    pub tool: Tool,
    /// The tool each toolbar slot currently shows.
    pub slot_tools: Vec<Tool>,
    pub popup: Option<Popup>,
    /// Pointer position over the title bar / toolbar, to place their popups.
    pub title_pointer: Point,
    pub toolbar_pointer: Point,
    /// Menu title under the pointer and the pointer's x inside it, waiting for the title
    /// bar's window-space position to arrive (see `TitleMoved`).
    hovered_menu: Option<(usize, f32)>,
    /// Left edge of each title-bar menu, in window coordinates (measured as you hover).
    pub menu_left: Vec<f32>,
    pub window_size: Size,
    /// Foreground colour, kept as HSV so the picker doesn't lose hue at s=0 / v=0.
    pub fg: Hsv,
    pub bg: Hsv,
    pub brush: BrushSettings,
    pub viewport: Size,
    /// The canvas widget's bounds in the window (pen positions are window-relative).
    pub canvas_bounds: Rectangle,
    pub stroke: Option<ActiveStroke>,
    /// Started with tablet support (not disabled by `GALLERION_PEN=0`).
    pub tablet_allowed: bool,
    pub tablet_ready: bool,
    /// Pen strokes use pressure. Off: the pen paints like a mouse (full pressure).
    pub pen_pressure: bool,
    pub pen: Option<Pen>,
    pub select: SelectOptions,
    pub transform: Option<transform::Session>,
    /// An open adjustment / filter dialog.
    pub adjusting: Option<adj::Session>,
    /// Pixels copied with Ctrl+C / Ctrl+X (an internal clipboard for now).
    pub clipboard: Option<Clip>,
    pub new_doc: Option<NewDocForm>,
    pub dialog: Option<Dialog>,
    pub layer_drag: Option<LayerDrag>,
    /// The last filter applied, for Filter > Last Filter (Alt+Ctrl+F).
    pub last_filter: Option<Op>,
    pub panels: Panels,
    pub workspace: Workspace,
    /// Command search (search icon / Ctrl+F): the query, while open.
    pub palette: Option<String>,
    /// Pointer over the dock, to place the layer right-click menu.
    pub dock_pointer: Point,
    /// Cursor position in document pixels, when over the canvas.
    pub cursor: Option<Point>,
    /// Space bar held: temporary Hand tool, like Photoshop.
    pub space_held: bool,
    pub color_tab: usize,
    pub props_tab: usize,
    pub layers_tab: usize,
    pub open_strip: Option<usize>,
    /// Layer being renamed, and the text so far.
    pub renaming: Option<(LayerId, String)>,
    pub confirm: Option<Confirm>,
    pub window: Option<window::Id>,
    pub maximized: bool,
    pub fullscreen: bool,
    /// Title bar pressed but not yet dragged (dragging starts on the first move, so a
    /// double-click can still maximise without fighting a compositor move).
    pub title_pressed: bool,
    pub loading: usize,
    /// Last error / info message, shown in the status bar.
    pub notice: Option<Notice>,
}

#[derive(Debug, Clone)]
pub struct Notice {
    pub text: String,
    pub error: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    // Files & tabs
    OpenDialog,
    Open(Vec<PathBuf>),
    Loaded(PathBuf, Result<Box<Document>, String>),
    Save,
    SaveAs,
    SaveTo(u64, Option<PathBuf>),
    Saved { doc: u64, path: PathBuf, depth: usize, result: Result<(), String> },
    Export,
    ExportTo(Option<PathBuf>),
    Exported(PathBuf, Result<(), String>),
    SelectDoc(usize),
    RequestCloseDoc(usize),
    RequestQuit,
    Confirmed(Choice),

    // Selection
    SelectShape { shape: Shape, combine: Combine },
    Wand { at: Point, combine: Combine },
    SelectAll,
    Deselect,
    InvertSelection,
    SetSelectMode(Combine),
    WandTolerance(f32),
    WandContiguous(bool),
    WandAllLayers(bool),
    ClearSelected,
    /// Fill the selection (or layer) with the foreground (true) or background colour.
    Fill(bool),
    Copy,
    Cut,
    Paste,
    /// Ctrl+J / Ctrl+Shift+J with a selection: new layer from the selected pixels.
    LayerVia { cut: bool },

    // Move / Free Transform (document pixels)
    MoveBegin(Point),
    MoveTo(Point),
    MoveEnd,
    Nudge(f32, f32),
    FreeTransform,
    TransformGrab(Handle, Point),
    TransformDrag { at: Point, shift: bool, alt: bool },
    TransformRelease,
    CommitTransform,
    CancelTransform,

    // Adjustments & filters
    OpenOp(OpKind),
    AdjustSet(Op),
    AdjustPreviewReady(u64, Box<TileGrid>),
    AdjustTogglePreview,
    AdjustReset,
    AdjustOk,
    AdjustCancel,

    // Layers panel right-click menu
    OpenLayerMenu(LayerId),
    DockPointer(Point),
    SelectPixels,
    DuplicateInto(usize),
    About,

    // Command search and workspaces
    OpenPalette,
    PaletteInput(String),
    ClosePalette,
    SetWorkspace(Workspace),

    // Layer dragging
    LayerPress(LayerId),
    DragOver(DropTarget),
    PointerReleased,

    // More menu commands
    PopupSub(Option<usize>),
    Place,
    Placed(PathBuf, Result<Box<Document>, String>),
    DuplicateImage,
    CopyMerged,
    OrientLayer(Orient),
    AutoTone,
    AutoContrast,
    RotateCanvas(u32),
    FlipCanvas(bool),
    CropToSelection,
    Trim,
    MergeDown,
    MergeVisible,
    Flatten,
    /// Bring to front (true) / send to back (false).
    ArrangeEnd(bool),
    Reselect,
    LastFilter,
    TogglePanel(Panel),
    ShowDialog(DialogKind),
    DialogA(String),
    DialogB(String),
    DialogConstrain,
    DialogAnchor(u8, u8),
    DialogOk,
    DialogCancel,

    // New document
    NewDocDialog,
    NewDocPreset(usize),
    NewDocName(String),
    NewDocWidth(String),
    NewDocHeight(String),
    NewDocBackground(NewBackground),
    CreateNewDoc,
    CancelNewDoc,

    // History
    Undo,
    Redo,
    JumpHistory(usize),
    /// A slider drag finished: the next change starts a new undo step.
    EndGesture,

    // Tools & colour
    SelectTool(Tool),
    SwapColors,
    ResetColors,
    PickSv(f32, f32),
    PickHue(f32),
    SetFg(Hsv),

    // Layers
    SelectLayer(LayerId),
    ToggleVisible(LayerId),
    AddLayer,
    DuplicateLayer,
    DeleteLayer,
    MoveLayer(isize),
    LayerOpacity(f32),
    LayerBlend(BlendMode),
    ToggleLock,
    ToggleLockOf(LayerId),
    StartRename(LayerId),
    RenameInput(String),
    CommitRename,
    SetBlendSpace(BlendSpace),

    // Brush options
    BrushSize(f32),
    BrushHardness(f32),
    BrushOpacity(f32),
    BrushFlow(f32),
    BrushSmoothing(f32),
    BrushMode(BlendMode),

    // View
    ZoomIn,
    ZoomOut,
    /// Zoom one step in/out keeping `anchor` (viewport-local, logical px) fixed.
    ZoomAt { zoom_in: bool, anchor: Point },
    ZoomTo(Zoom),
    PanBy(Vector),
    CanvasBounds(Rectangle),
    CursorMoved(Option<Point>),

    // Painting (positions in document pixels)
    StrokeBegin { at: Point, pressure: f32, erase: bool, line: bool },
    StrokeTo { at: Point, pressure: f32 },
    StrokeEnd,
    PickColor(Point),
    PressureSize(bool),
    PressureOpacity(bool),
    ToggleTablet,
    TabletHandles(Option<tablet::Handles>),
    Tablet(TabletMessage),

    // Panels
    ColorTab(usize),
    PropsTab(usize),
    LayersTab(usize),
    ToggleStrip(usize),

    // Window (we draw our own title bar)
    WindowOpened(Option<window::Id>),
    TitlePressed,
    TitleMoved(Point),
    TitleReleased,
    /// The window was resized somehow (our buttons, the compositor, keyboard shortcuts...).
    WindowResized(Size),

    // Popups
    OpenPopup(PopupKind, Point),
    ClosePopup,
    ToolbarPointer(Point),
    /// Pointer over title-bar menu `i`, at this position inside the title.
    MenuPointer(usize, Point),
    /// Click on title-bar menu `i`: open it (or close it if it's already open).
    MenuTitle(usize),
    /// A menu item was chosen: close the popup, then run it.
    Menu(Box<Message>),
    Maximized(bool),
    ModeChanged(window::Mode),
    ResizeWindow(Direction),
    Minimize,
    ToggleMaximize,
    ToggleFullscreen,

    Key(keyboard::Event),
    Noop,
}

/// `src` moved into a `w`×`h` document: same position if the sizes match, otherwise centred.
fn centred(src: &TileGrid, w: u32, h: u32) -> TileGrid {
    if src.width() == w && src.height() == h {
        return src.clone();
    }
    let Some([x0, y0, x1, y1]) = jd_raster::transform::content_bounds(src) else { return TileGrid::empty(w, h) };
    let dx = (w as i64 - (x1 - x0) as i64) / 2 - x0 as i64;
    let dy = (h as i64 - (y1 - y0) as i64) / 2 - y0 as i64;
    jd_raster::transform::place(src, w, h, dx, dy)
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

impl Photo {
    pub fn new() -> (Self, Task<Message>) {
        let app = Self {
            docs: Vec::new(),
            active: 0,
            next_doc_id: 1,
            tool: Tool::Brush,
            slot_tools: crate::tool::slots().map(|s| s[0]).collect(),
            popup: None,
            title_pointer: Point::ORIGIN,
            toolbar_pointer: Point::ORIGIN,
            hovered_menu: None,
            menu_left: vec![0.0; crate::view::MENU_COUNT],
            window_size: Size::new(1600.0, 960.0),
            fg: Hsv::from_rgba(Rgba::from_hex("F2733F").expect("valid hex")),
            bg: Hsv { h: 0.0, s: 0.0, v: 1.0 },
            brush: BrushSettings {
                size: 45.0,
                hardness: 0.8,
                opacity: 1.0,
                flow: 1.0,
                smoothing: 0.1,
                mode: BlendMode::Normal,
                pressure_size: true,
                pressure_opacity: false,
            },
            viewport: Size::new(1200.0, 800.0),
            canvas_bounds: Rectangle::new(Point::ORIGIN, Size::new(1200.0, 800.0)),
            stroke: None,
            tablet_allowed: tablet::enabled_by_env(),
            pen_pressure: true,
            tablet_ready: false,
            pen: None,
            select: SelectOptions::default(),
            transform: None,
            adjusting: None,
            clipboard: None,
            new_doc: None,
            dialog: None,
            layer_drag: None,
            last_filter: None,
            panels: Panels { color: true, properties: true, layers: true },
            workspace: Workspace::Essentials,
            palette: None,
            dock_pointer: Point::ORIGIN,
            cursor: None,
            space_held: false,
            color_tab: 0,
            props_tab: 0,
            layers_tab: 0,
            open_strip: None,
            renaming: None,
            confirm: None,
            window: None,
            maximized: false,
            fullscreen: false,
            title_pressed: false,
            loading: 0,
            notice: None,
        };
        // `gallerion photo.jpg other.png` opens those files.
        let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        let open = if args.is_empty() { Task::none() } else { Task::done(Message::Open(args)) };
        (app, Task::batch([window::oldest().map(Message::WindowOpened), open]))
    }

    pub fn doc(&self) -> Option<&OpenDoc> {
        self.docs.get(self.active)
    }

    fn doc_mut(&mut self) -> Option<&mut OpenDoc> {
        self.docs.get_mut(self.active)
    }

    fn doc_index(&self, id: u64) -> Option<usize> {
        self.docs.iter().position(|d| d.id == id)
    }

    pub fn document(&self) -> Option<&Document> {
        self.doc().map(|d| &d.doc)
    }

    /// The tool actually in effect: space bar temporarily switches to Hand, and the pen's
    /// eraser end to Eraser (like Photoshop).
    pub fn effective_tool(&self) -> Tool {
        if self.space_held {
            Tool::Hand
        } else if self.pen.is_some_and(|p| p.eraser) {
            Tool::Eraser
        } else {
            self.tool
        }
    }

    /// The zoom factor in use for the active document (resolves `Fit`).
    pub fn zoom_factor(&self) -> f32 {
        self.doc().map_or(1.0, |d| self.resolve_zoom(d))
    }

    fn resolve_zoom(&self, d: &OpenDoc) -> f32 {
        match d.zoom {
            Zoom::Fixed(z) => z,
            Zoom::Fit => {
                let margin = 60.0;
                let zx = (self.viewport.width - margin).max(1.0) / d.doc.width as f32;
                let zy = (self.viewport.height - margin).max(1.0) / d.doc.height as f32;
                zx.min(zy).min(1.0)
            }
        }
    }

    /// Where document (0,0) sits in the viewport (logical px), given a zoom and pan.
    pub fn doc_origin(&self) -> Option<(Point, f32)> {
        let d = self.doc()?;
        let z = self.resolve_zoom(d);
        Some((self.origin_for(d, z, d.pan), z))
    }

    fn origin_for(&self, d: &OpenDoc, z: f32, pan: Vector) -> Point {
        Point::new(
            self.viewport.width / 2.0 - d.doc.width as f32 * z / 2.0 + pan.x,
            self.viewport.height / 2.0 - d.doc.height as f32 * z / 2.0 + pan.y,
        )
    }

    fn info(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice { text: text.into(), error: false });
    }

    fn error(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice { text: text.into(), error: true });
    }

    /// Applies an undoable change to the active document. `gesture` merges repeated changes
    /// (slider drags) into one undo step.
    fn edit(&mut self, label: &str, gesture: Option<&str>, f: impl FnOnce(&mut Document) -> bool) {
        let Some(d) = self.doc_mut() else { return };
        let mut draft = d.doc.clone();
        if f(&mut draft) {
            d.history.record(label, &d.doc, gesture);
            d.doc = draft;
        }
    }

    /// Edits the active layer, unless it's locked.
    fn edit_layer(&mut self, label: &str, gesture: Option<&str>, f: impl FnOnce(&mut jd_raster::Layer) -> bool) {
        let locked = self.document().and_then(|d| d.active()).is_some_and(|l| l.locked);
        if locked {
            self.error("The layer is locked");
            return;
        }
        self.edit(label, gesture, |doc| {
            let Some(id) = doc.active_id() else { return false };
            doc.layer_mut(id).is_some_and(f)
        });
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        self.refresh_thumbnails();
        task
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        // An open Move/Free Transform is committed before anything else edits the document
        // (Photoshop asks; applying is the common answer). Undo cancels it instead.
        if self.transform.is_some() {
            match message {
                Message::Undo | Message::Redo | Message::JumpHistory(_) => self.transform = None,
                ref m if !Self::keeps_transform(m) => self.commit_transform(),
                _ => {}
            }
        }
        match message {
            Message::SelectShape { shape, combine } => {
                let Some(doc) = self.document() else { return Task::none() };
                let (w, h) = (doc.width, doc.height);
                let (label, new) = match &shape {
                    Shape::Rect([x0, y0, x1, y1]) => ("Rectangular Marquee", Selection::rect(w, h, *x0, *y0, *x1, *y1)),
                    Shape::Ellipse([x0, y0, x1, y1]) => ("Elliptical Marquee", Selection::ellipse(w, h, *x0, *y0, *x1, *y1)),
                    Shape::Polygon(points) => ("Lasso", Selection::polygon(w, h, points)),
                };
                self.apply_selection(label, new, combine);
            }
            Message::Wand { at, combine } => {
                let Some(doc) = self.document() else { return Task::none() };
                if at.x < 0.0 || at.y < 0.0 {
                    return Task::none();
                }
                let opts = &self.select;
                let new = if opts.all_layers {
                    Selection::magic_wand(doc.width, doc.height, at.x as u32, at.y as u32, opts.tolerance, opts.contiguous, |x, y| {
                        jd_raster::composite::pixel_srgb8(doc, x, y)
                    })
                } else {
                    let Some(layer) = doc.active() else { return Task::none() };
                    Selection::magic_wand(doc.width, doc.height, at.x as u32, at.y as u32, opts.tolerance, opts.contiguous, |x, y| {
                        layer.pixels.pixel(x, y)
                    })
                };
                self.apply_selection("Magic Wand", new, combine);
            }
            Message::SelectAll => self.edit("Select All", None, |doc| {
                doc.selection = Some(Selection::all(doc.width, doc.height));
                true
            }),
            Message::Deselect => {
                if let Some(d) = self.doc_mut()
                    && d.doc.selection.is_some()
                {
                    d.last_selection = d.doc.selection.clone();
                }
                self.edit("Deselect", None, |doc| doc.selection.take().is_some());
            }
            Message::Reselect => {
                let last = self.doc().and_then(|d| d.last_selection.clone());
                if let Some(sel) = last {
                    self.edit("Reselect", None, |doc| {
                        let ok = sel.width() == doc.width && sel.height() == doc.height;
                        if ok {
                            doc.selection = Some(sel);
                        }
                        ok
                    });
                }
            }
            Message::InvertSelection => self.edit("Inverse", None, |doc| {
                let mut s = doc.selection.take().unwrap_or_else(|| Selection::empty(doc.width, doc.height));
                s.invert();
                doc.selection = (!s.is_empty()).then_some(s);
                true
            }),
            Message::SetSelectMode(mode) => self.select.mode = mode,
            Message::WandTolerance(t) => self.select.tolerance = t.round().clamp(0.0, 255.0) as u8,
            Message::WandContiguous(on) => self.select.contiguous = on,
            Message::WandAllLayers(on) => self.select.all_layers = on,
            Message::ClearSelected => {
                if self.check_layer_editable() {
                    self.edit("Clear", None, |doc| {
                        let Some(sel) = doc.selection.clone() else { return false };
                        let Some(id) = doc.active_id() else { return false };
                        doc.layer_mut(id).map(|l| selection::clear(&mut l.pixels, &sel)).is_some()
                    });
                }
            }
            Message::Fill(foreground) => {
                if self.check_layer_editable() {
                    let color = if foreground { self.fg } else { self.bg }.to_rgba().to_srgb8();
                    self.edit("Fill", None, |doc| {
                        let sel = doc.selection.clone().unwrap_or_else(|| Selection::all(doc.width, doc.height));
                        let Some(id) = doc.active_id() else { return false };
                        doc.layer_mut(id).map(|l| selection::fill(&mut l.pixels, &sel, color)).is_some()
                    });
                }
            }
            Message::Copy | Message::Cut => {
                let cut = matches!(message, Message::Cut);
                let Some(doc) = self.document() else { return Task::none() };
                let Some(layer) = doc.active() else { return Task::none() };
                let pixels = match &doc.selection {
                    Some(sel) => selection::extract(&layer.pixels, sel),
                    None => layer.pixels.clone(),
                };
                if jd_raster::transform::content_bounds(&pixels).is_none() {
                    self.info("Nothing to copy: the selected area is empty");
                    return Task::none();
                }
                self.clipboard = Some(Clip { pixels });
                if cut {
                    if self.check_layer_editable() {
                        self.edit("Cut", None, |doc| {
                            let Some(id) = doc.active_id() else { return false };
                            let sel = doc.selection.clone();
                            doc.layer_mut(id)
                                .map(|l| match &sel {
                                    Some(sel) => selection::clear(&mut l.pixels, sel),
                                    None => l.pixels = TileGrid::empty(l.pixels.width(), l.pixels.height()),
                                })
                                .is_some()
                        });
                    }
                } else {
                    self.info("Copied");
                }
            }
            Message::Paste => {
                let Some(clip) = &self.clipboard else {
                    self.info("Nothing to paste");
                    return Task::none();
                };
                let Some(doc) = self.document() else { return Task::none() };
                // Same position when the sizes match; centred in a different-size document.
                let pixels = centred(&clip.pixels, doc.width, doc.height);
                self.edit("Paste", None, |doc| {
                    let n = doc.layers().len();
                    let id = doc.add_layer(format!("Layer {n}"));
                    doc.layer_mut(id).map(|l| l.pixels = pixels).is_some()
                });
            }
            Message::LayerVia { cut } => {
                let has_selection = self.document().is_some_and(|d| d.selection.is_some());
                if !has_selection {
                    return self.update(Message::DuplicateLayer);
                }
                if cut && !self.check_layer_editable() {
                    return Task::none();
                }
                self.edit(if cut { "Layer via Cut" } else { "Layer via Copy" }, None, |doc| {
                    let Some(sel) = doc.selection.clone() else { return false };
                    let Some(src) = doc.active().cloned() else { return false };
                    let pixels = selection::extract(&src.pixels, &sel);
                    if cut && let Some(l) = doc.layer_mut(src.id) {
                        selection::clear(&mut l.pixels, &sel);
                    }
                    let n = doc.layers().len();
                    let id = doc.add_layer(format!("Layer {n}"));
                    doc.layer_mut(id).map(|l| l.pixels = pixels).is_some()
                });
            }

            Message::MoveBegin(at) => {
                if self.begin_transform(transform::Kind::Move)
                    && let Some(t) = self.transform.as_mut()
                {
                    t.grab(Handle::Inside, at);
                }
            }
            Message::MoveTo(at) => {
                if let Some(t) = self.transform.as_mut() {
                    t.drag(at, false, false);
                }
            }
            Message::MoveEnd => {
                if self.transform.as_ref().is_some_and(|t| t.kind == transform::Kind::Move) {
                    self.commit_transform();
                }
            }
            Message::Nudge(dx, dy) => {
                if self.begin_transform(transform::Kind::Move) {
                    if let Some(t) = self.transform.as_mut() {
                        t.translate(dx, dy);
                    }
                    self.commit_transform();
                }
            }
            Message::FreeTransform => {
                if self.transform.is_none() {
                    self.begin_transform(transform::Kind::Free);
                }
            }
            Message::TransformGrab(handle, at) => {
                if let Some(t) = self.transform.as_mut() {
                    t.grab(handle, at);
                }
            }
            Message::TransformDrag { at, shift, alt } => {
                if let Some(t) = self.transform.as_mut() {
                    t.drag(at, shift, alt);
                }
            }
            Message::TransformRelease => {
                if let Some(t) = self.transform.as_mut() {
                    t.release();
                }
            }
            Message::CommitTransform => self.commit_transform(),
            Message::CancelTransform => self.transform = None,

            Message::OpenOp(kind) => {
                if self.adjusting.is_some() {
                    return Task::none();
                }
                let Some(d) = self.doc() else { return Task::none() };
                match adj::Session::begin(d.id, &d.doc, kind) {
                    Err(e) => self.error(e),
                    Ok(session) if kind.is_instant() => {
                        let (layer, label, pixels) = (session.layer, session.op.name(), session.result());
                        self.edit(label, None, |doc| doc.layer_mut(layer).map(|l| l.pixels = pixels).is_some());
                    }
                    Ok(mut session) => {
                        let task = session.schedule();
                        self.adjusting = Some(session);
                        return task;
                    }
                }
            }
            Message::AdjustSet(op) => {
                if let Some(s) = self.adjusting.as_mut() {
                    s.set_op(op);
                    return s.schedule();
                }
            }
            Message::AdjustPreviewReady(generation, grid) => {
                if let Some(s) = self.adjusting.as_mut() {
                    return s.preview_ready(generation, *grid);
                }
            }
            Message::AdjustTogglePreview => {
                if let Some(s) = self.adjusting.as_mut() {
                    return s.toggle_preview();
                }
            }
            Message::AdjustReset => {
                if let Some(s) = self.adjusting.as_mut() {
                    s.reset();
                    return s.schedule();
                }
            }
            Message::AdjustOk => {
                let Some(s) = self.adjusting.take() else { return Task::none() };
                let Some(i) = self.doc_index(s.doc) else { return Task::none() };
                if matches!(s.op, Op::Filter(_)) {
                    self.last_filter = Some(s.op.clone());
                }
                let pixels = s.result();
                let d = &mut self.docs[i];
                d.history.record(s.op.name(), &d.doc, None);
                if let Some(l) = d.doc.layer_mut(s.layer) {
                    l.pixels = pixels;
                }
            }
            Message::AdjustCancel => self.adjusting = None,

            Message::LayerPress(id) => {
                let _ = self.update(Message::SelectLayer(id));
                if let Some(d) = self.doc() {
                    self.layer_drag = Some(LayerDrag { doc: d.id, layer: id, target: None });
                }
            }
            Message::DragOver(target) => {
                if let Some(drag) = self.layer_drag.as_mut() {
                    drag.target = Some(target);
                    // Hovering a tab while dragging brings that document forward, like Photoshop.
                    if let DropTarget::Tab(i) = target
                        && i < self.docs.len()
                    {
                        self.active = i;
                    }
                }
            }
            Message::PointerReleased => {
                if let Some(drag) = self.layer_drag.take() {
                    self.drop_layer(drag);
                }
            }
            Message::Place => {
                if self.doc().is_none() {
                    return Task::none();
                }
                let mut types = jd_io::OPEN_EXTENSIONS.to_vec();
                types.push(jd_io::jdp::EXTENSION);
                return Task::perform(
                    async move {
                        let file = rfd::AsyncFileDialog::new()
                            .set_title("Place")
                            .add_filter("Images and documents", &types)
                            .pick_file()
                            .await?;
                        let path = file.path().to_path_buf();
                        let loaded = jd_io::open(&path).map(Box::new).map_err(|e| e.to_string());
                        Some((path, loaded))
                    },
                    |picked| match picked {
                        Some((path, loaded)) => Message::Placed(path, loaded),
                        None => Message::Noop,
                    },
                );
            }
            Message::Placed(path, result) => match result {
                Err(e) => self.error(format!("Couldn't place {}: {e}", file_name(&path))),
                Ok(placed) => {
                    let flat = TileGrid::from_rgba8(placed.width, placed.height, &jd_raster::composite::flatten(&placed));
                    let name = path.file_stem().map_or_else(|| "Placed".to_owned(), |s| s.to_string_lossy().into_owned());
                    self.edit("Place", None, |doc| {
                        let pixels = centred(&flat, doc.width, doc.height);
                        let id = doc.add_layer(name);
                        doc.layer_mut(id).map(|l| l.pixels = pixels).is_some()
                    });
                }
            },
            Message::DuplicateImage => {
                if let Some(d) = self.document() {
                    let mut copy = d.clone();
                    let stem = Path::new(&copy.name).file_stem().map_or_else(|| copy.name.clone(), |s| s.to_string_lossy().into_owned());
                    copy.name = format!("{stem} copy");
                    self.push_doc(copy, None);
                    // Never saved: closing it should ask.
                    if let Some(d) = self.doc_mut() {
                        d.history = History::default();
                    }
                }
            }
            Message::CopyMerged => {
                let Some(doc) = self.document() else { return Task::none() };
                let flat = TileGrid::from_rgba8(doc.width, doc.height, &jd_raster::composite::flatten(doc));
                let pixels = match &doc.selection {
                    Some(sel) => selection::extract(&flat, sel),
                    None => flat,
                };
                self.clipboard = Some(Clip { pixels });
                self.info("Copied (merged)");
            }
            Message::OrientLayer(o) => {
                if self.begin_transform(transform::Kind::Free)
                    && let Some(t) = self.transform.as_mut()
                {
                    match o {
                        Orient::FlipH => t.orient(true, false, 0),
                        Orient::FlipV => t.orient(false, true, 0),
                        Orient::Rotate90Cw => t.orient(false, false, 1),
                        Orient::Rotate90Ccw => t.orient(false, false, 3),
                        Orient::Rotate180 => t.orient(false, false, 2),
                    }
                    self.commit_transform();
                }
            }
            Message::AutoTone | Message::AutoContrast => {
                let tone = matches!(message, Message::AutoTone);
                if self.check_layer_editable() {
                    self.edit(if tone { "Auto Tone" } else { "Auto Contrast" }, None, |doc| {
                        let sel = doc.selection.clone();
                        let Some(id) = doc.active_id() else { return false };
                        let Some(l) = doc.layer_mut(id) else { return false };
                        let adj = if tone {
                            jd_raster::adjust::auto_tone(&l.pixels, sel.as_ref())
                        } else {
                            jd_raster::adjust::auto_contrast(&l.pixels, sel.as_ref())
                        };
                        l.pixels = jd_raster::adjust::apply(&l.pixels, &adj, sel.as_ref());
                        true
                    });
                }
            }
            Message::RotateCanvas(q) => self.edit("Rotate Canvas", None, |doc| {
                doc.rotate_canvas(q);
                true
            }),
            Message::FlipCanvas(h) => self.edit(if h { "Flip Canvas Horizontal" } else { "Flip Canvas Vertical" }, None, |doc| {
                doc.flip_canvas(h);
                true
            }),
            Message::CropToSelection => self.edit("Crop", None, |doc| {
                let Some(b) = doc.selection.as_ref().and_then(|s| s.bounds()) else { return false };
                doc.crop(b);
                true
            }),
            Message::Trim => self.edit("Trim", None, |doc| doc.trim()),
            Message::MergeDown => self.edit("Merge Down", None, |doc| doc.merge_down()),
            Message::MergeVisible => self.edit("Merge Visible", None, |doc| doc.merge_visible()),
            Message::Flatten => self.edit("Flatten Image", None, |doc| {
                doc.flatten();
                true
            }),
            Message::ArrangeEnd(front) => self.edit(if front { "Bring to Front" } else { "Send to Back" }, None, |doc| {
                let Some(id) = doc.active_id() else { return false };
                let Some(i) = doc.layers().iter().position(|l| l.id == id) else { return false };
                let delta = if front { (doc.layers().len() - 1 - i) as isize } else { -(i as isize) };
                delta != 0 && doc.move_layer(id, delta)
            }),
            Message::LastFilter => {
                if let Some(op) = self.last_filter.clone()
                    && self.check_layer_editable()
                {
                    self.edit(op.name(), None, |doc| {
                        let sel = doc.selection.clone();
                        let Some(id) = doc.active_id() else { return false };
                        doc.layer_mut(id).map(|l| l.pixels = op.run(&l.pixels, sel.as_ref())).is_some()
                    });
                }
            }
            Message::TogglePanel(p) => {
                let flag = match p {
                    Panel::Color => &mut self.panels.color,
                    Panel::Properties => &mut self.panels.properties,
                    Panel::Layers => &mut self.panels.layers,
                };
                *flag = !*flag;
            }
            Message::ShowDialog(kind) => {
                let Some(doc) = self.document() else { return Task::none() };
                let needs_selection = matches!(kind, DialogKind::Expand | DialogKind::Contract | DialogKind::Feather | DialogKind::Stroke);
                if needs_selection && doc.selection.is_none() {
                    self.error("Make a selection first");
                    return Task::none();
                }
                let (a, b) = match kind {
                    DialogKind::ImageSize | DialogKind::CanvasSize => (doc.width.to_string(), doc.height.to_string()),
                    DialogKind::Feather => ("5".into(), String::new()),
                    _ => ("3".into(), String::new()),
                };
                self.dialog = Some(Dialog { kind, a, b, constrain: true, aspect: doc.width as f32 / doc.height as f32, anchor: (1, 1) });
            }
            Message::DialogA(ref v) | Message::DialogB(ref v) => {
                let first = matches!(message, Message::DialogA(_));
                if let Some(d) = self.dialog.as_mut() {
                    let digits: String = v.chars().filter(char::is_ascii_digit).take(5).collect();
                    let linked = d.kind == DialogKind::ImageSize && d.constrain;
                    if first {
                        if linked && let Some(w) = Dialog::number(&digits) {
                            d.b = ((w as f32 / d.aspect).round().max(1.0) as u32).to_string();
                        }
                        d.a = digits;
                    } else {
                        if linked && let Some(h) = Dialog::number(&digits) {
                            d.a = ((h as f32 * d.aspect).round().max(1.0) as u32).to_string();
                        }
                        d.b = digits;
                    }
                }
            }
            Message::DialogConstrain => {
                if let Some(d) = self.dialog.as_mut() {
                    d.constrain = !d.constrain;
                }
            }
            Message::DialogAnchor(cx, cy) => {
                if let Some(d) = self.dialog.as_mut() {
                    d.anchor = (cx.min(2), cy.min(2));
                }
            }
            Message::DialogCancel => self.dialog = None,
            Message::DialogOk => return self.dialog_ok(),
            Message::NewDocDialog => {
                let n = self.docs.len() + 1;
                self.new_doc = Some(NewDocForm {
                    name: format!("Untitled-{n}"),
                    width: "1920".into(),
                    height: "1080".into(),
                    preset: Some(0),
                    background: NewBackground::White,
                });
            }
            Message::NewDocPreset(i) => {
                if let (Some(f), Some(&(_, _, w, h))) = (self.new_doc.as_mut(), PRESETS.get(i)) {
                    f.width = w.to_string();
                    f.height = h.to_string();
                    f.preset = Some(i);
                }
            }
            Message::NewDocName(v) => {
                if let Some(f) = self.new_doc.as_mut() {
                    f.name = v;
                }
            }
            Message::NewDocWidth(ref v) | Message::NewDocHeight(ref v) => {
                let is_width = matches!(message, Message::NewDocWidth(_));
                if let Some(f) = self.new_doc.as_mut() {
                    let digits: String = v.chars().filter(char::is_ascii_digit).take(5).collect();
                    if is_width { f.width = digits } else { f.height = digits }
                    f.preset = PRESETS
                        .iter()
                        .position(|&(_, _, w, h)| f.width == w.to_string() && f.height == h.to_string());
                }
            }
            Message::NewDocBackground(b) => {
                if let Some(f) = self.new_doc.as_mut() {
                    f.background = b;
                }
            }
            Message::CreateNewDoc => {
                let Some(f) = self.new_doc.as_ref() else { return Task::none() };
                let Some((w, h)) = f.size() else {
                    self.error("Width and height must be between 1 and 30000 pixels");
                    return Task::none();
                };
                let name = if f.name.trim().is_empty() { "Untitled".to_owned() } else { f.name.trim().to_owned() };
                let doc = match f.background {
                    NewBackground::Transparent => {
                        let mut d = Document::new(name, w, h);
                        // A transparent start is meant to be painted on: don't lock it.
                        let id = d.active_id().expect("background");
                        if let Some(l) = d.layer_mut(id) {
                            l.locked = false;
                            l.name = "Layer 0".into();
                        }
                        d
                    }
                    bg => {
                        let c = if bg == NewBackground::White { [255, 255, 255] } else { self.bg.to_rgba().to_srgb8() };
                        let mut d = Document::new(name, w, h);
                        let id = d.active_id().expect("background");
                        if let Some(l) = d.layer_mut(id) {
                            selection::fill(&mut l.pixels, &Selection::all(w, h), c);
                        }
                        d
                    }
                };
                self.new_doc = None;
                // Untouched new documents close without asking, like Photoshop.
                self.push_doc(doc, None);
            }
            Message::CancelNewDoc => self.new_doc = None,

            Message::OpenDialog => {
                return Task::perform(
                    async {
                        let mut images = jd_io::OPEN_EXTENSIONS.to_vec();
                        images.push(jd_io::jdp::EXTENSION);
                        rfd::AsyncFileDialog::new()
                            .set_title("Open")
                            .add_filter("Images and documents", &images)
                            .add_filter("Gallerion documents", &[jd_io::jdp::EXTENSION])
                            .add_filter("All files", &["*"])
                            .pick_files()
                            .await
                            .map(|files| files.iter().map(|f| f.path().to_path_buf()).collect())
                            .unwrap_or_default()
                    },
                    Message::Open,
                );
            }
            Message::Open(mut paths) => {
                // Already-open files just get focused.
                paths.retain(|p| {
                    let open = self.docs.iter().position(|d| d.path.as_deref() == Some(p.as_path()));
                    if let Some(i) = open {
                        self.active = i;
                    }
                    open.is_none()
                });
                self.loading += paths.len();
                return Task::batch(paths.into_iter().map(|path| {
                    let p = path.clone();
                    Task::perform(
                        async move { jd_io::open(&p).map(Box::new).map_err(|e| e.to_string()) },
                        move |result| Message::Loaded(path.clone(), result),
                    )
                }));
            }
            Message::Loaded(path, result) => {
                self.loading = self.loading.saturating_sub(1);
                match result {
                    Ok(doc) => {
                        self.push_doc(*doc, Some(path));
                        self.notice = None;
                    }
                    Err(e) => self.error(format!("Couldn't open {}: {e}", file_name(&path))),
                }
            }
            Message::Save => {
                if let Some(d) = self.doc() {
                    let target = d.save_path().map(Path::to_path_buf);
                    return match target {
                        Some(path) => self.save(d.id, path),
                        None => self.update(Message::SaveAs),
                    };
                }
            }
            Message::SaveAs => {
                if let Some(d) = self.doc() {
                    return self.ask_save_path(d.id);
                }
            }
            Message::SaveTo(id, path) => match path {
                Some(path) => return self.save(id, path),
                // Dialog cancelled: a pending close/quit is cancelled too.
                None => self.confirm = None,
            },
            Message::Saved { doc, path, depth, result } => {
                let Some(i) = self.doc_index(doc) else { return Task::none() };
                self.docs[i].saving = false;
                match result {
                    Ok(()) => {
                        let d = &mut self.docs[i];
                        d.history.mark_saved_at(depth);
                        d.doc.name = file_name(&path);
                        d.path = Some(path.clone());
                        self.info(format!("Saved {}", file_name(&path)));
                        // Edits made while saving leave it dirty: keep asking in that case.
                        if let Some(c) = self.confirm.filter(|c| c.doc == doc)
                            && !self.docs[i].is_dirty()
                        {
                            self.confirm = None;
                            return self.finish_close(i, c.then);
                        }
                    }
                    Err(e) => {
                        self.confirm = None;
                        self.error(format!("Couldn't save {}: {e}", file_name(&path)));
                    }
                }
            }
            Message::Export => {
                let Some(d) = self.doc() else { return Task::none() };
                let stem = Path::new(&d.doc.name).file_stem().map(|s| s.to_string_lossy().into_owned());
                let suggested = format!("{}.png", stem.unwrap_or_else(|| "Untitled".into()));
                let dir = d.path.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf);
                return Task::perform(
                    async move {
                        let mut dialog = rfd::AsyncFileDialog::new()
                            .set_title("Export")
                            .set_file_name(suggested)
                            .add_filter("PNG", &["png"])
                            .add_filter("JPEG", &["jpg", "jpeg"])
                            .add_filter("WebP", &["webp"])
                            .add_filter("TIFF", &["tif", "tiff"])
                            .add_filter("Other", &["bmp", "tga", "qoi"]);
                        if let Some(dir) = dir {
                            dialog = dialog.set_directory(dir);
                        }
                        dialog.save_file().await.map(|f| f.path().to_path_buf())
                    },
                    Message::ExportTo,
                );
            }
            Message::ExportTo(Some(path)) => {
                let Some(d) = self.doc() else { return Task::none() };
                let doc = d.doc.clone();
                self.info(format!("Exporting {}…", file_name(&path)));
                return Task::perform(
                    {
                        let path = path.clone();
                        async move { jd_io::export(&doc, &path).map_err(|e| e.to_string()) }
                    },
                    move |r| Message::Exported(path.clone(), r),
                );
            }
            Message::ExportTo(None) => {}
            Message::Exported(path, result) => match result {
                Ok(()) => self.info(format!("Exported {}", file_name(&path))),
                Err(e) => self.error(format!("Couldn't export {}: {e}", file_name(&path))),
            },
            Message::SelectDoc(i) => {
                if i < self.docs.len() {
                    self.active = i;
                    self.renaming = None;
                }
            }
            Message::RequestCloseDoc(i) => {
                let Some(d) = self.docs.get(i) else { return Task::none() };
                if d.is_dirty() {
                    self.active = i;
                    self.confirm = Some(Confirm { doc: d.id, then: AfterConfirm::CloseTab });
                } else {
                    self.close_doc(i);
                }
            }
            Message::RequestQuit => return self.continue_quit(),
            Message::Confirmed(choice) => {
                let Some(c) = self.confirm else { return Task::none() };
                let Some(i) = self.doc_index(c.doc) else {
                    self.confirm = None;
                    return Task::none();
                };
                match choice {
                    Choice::Cancel => self.confirm = None,
                    Choice::Discard => {
                        self.confirm = None;
                        return self.finish_close(i, c.then);
                    }
                    // Keeps `confirm` set; `Saved` finishes the close.
                    Choice::Save => return self.update(Message::Save),
                }
            }

            Message::Undo | Message::Redo | Message::JumpHistory(_) => {
                self.renaming = None;
                self.stroke = None;
                if let Some(d) = self.doc_mut() {
                    // The name follows the file (Save As renames it), not the edit history.
                    let name = d.doc.name.clone();
                    d.last_paint = None;
                    let changed = match message {
                        Message::Undo => d.history.undo(&mut d.doc),
                        Message::Redo => d.history.redo(&mut d.doc),
                        Message::JumpHistory(n) => {
                            d.history.jump_to(n, &mut d.doc);
                            true
                        }
                        _ => unreachable!(),
                    };
                    d.doc.name = name;
                    if !changed {
                        self.info("Nothing to undo or redo");
                    }
                }
            }
            Message::EndGesture => {
                if let Some(d) = self.doc_mut() {
                    d.history.end_gesture();
                }
            }

            Message::SelectTool(t) => {
                self.tool = t;
                self.slot_tools[t.slot()] = t;
                self.popup = None;
            }
            Message::OpenPopup(kind, at) => self.popup = Some(Popup { kind, at, sub: None }),
            Message::PopupSub(sub) => {
                if let Some(p) = self.popup.as_mut() {
                    p.sub = sub;
                }
            }
            Message::ClosePopup => self.popup = None,
            Message::ToolbarPointer(p) => self.toolbar_pointer = p,
            // The inner (title) mouse area reports before the title bar's, so stash it and
            // finish in `TitleMoved`, which knows the window position.
            Message::MenuPointer(i, p) => self.hovered_menu = Some((i, p.x)),
            Message::MenuTitle(i) => {
                self.popup = match self.popup {
                    Some(Popup { kind: PopupKind::MenuBar(j), .. }) if j == i => None,
                    _ => Some(Popup { kind: PopupKind::MenuBar(i), at: Point::new(self.menu_left[i], 30.0), sub: None }),
                };
            }
            Message::Menu(m) => {
                self.popup = None;
                self.palette = None;
                return self.update(*m);
            }
            Message::DockPointer(p) => self.dock_pointer = p,
            Message::OpenLayerMenu(id) => {
                let _ = self.update(Message::SelectLayer(id));
                // The dock is the rightmost 300px, below the toolbar line.
                let at = Point::new(self.window_size.width - 300.0 + self.dock_pointer.x, crate::view::TOOLBAR_TOP + self.dock_pointer.y);
                self.popup = Some(Popup { kind: PopupKind::Layer(id), at, sub: None });
            }
            Message::SelectPixels => self.edit("Select Pixels", None, |doc| {
                let Some(layer) = doc.active() else { return false };
                let sel = Selection::from_alpha(&layer.pixels);
                doc.selection = (!sel.is_empty()).then_some(sel);
                true
            }),
            Message::DuplicateInto(target) => {
                if let (Some(d), true) = (self.doc(), target < self.docs.len())
                    && let Some(layer) = d.doc.active_id()
                {
                    let src = d.id;
                    self.active = target;
                    self.copy_layer_into(src, layer);
                }
            }
            Message::About => self.info("Gallerion, the jamsdesign photo editor (Rust + iced)"),
            Message::OpenPalette => {
                self.popup = None;
                self.palette = Some(String::new());
                return operation::focus(crate::view::PALETTE_INPUT);
            }
            Message::PaletteInput(q) => {
                if let Some(p) = self.palette.as_mut() {
                    *p = q;
                }
            }
            Message::ClosePalette => self.palette = None,
            Message::SetWorkspace(w) => {
                self.workspace = w;
                self.popup = None;
                let (color, properties, layers, history) = match w {
                    Workspace::Essentials => (true, true, true, false),
                    Workspace::Photography => (false, true, true, true),
                    Workspace::Painting => (true, false, true, false),
                    Workspace::Minimal => (false, false, false, false),
                };
                self.panels = Panels { color, properties, layers };
                self.open_strip = history.then_some(0);
            }
            Message::SwapColors => std::mem::swap(&mut self.fg, &mut self.bg),
            Message::ResetColors => {
                self.fg = Hsv { h: self.fg.h, s: 0.0, v: 0.0 };
                self.bg = Hsv { h: self.bg.h, s: 0.0, v: 1.0 };
            }
            Message::PickSv(s, v) => {
                self.fg.s = s.clamp(0.0, 1.0);
                self.fg.v = v.clamp(0.0, 1.0);
            }
            Message::PickHue(h) => self.fg.h = h.clamp(0.0, 359.999),
            Message::SetFg(c) => self.fg = c,

            // Selecting a layer isn't an undo step (same as Photoshop).
            Message::SelectLayer(id) => {
                if self.renaming.as_ref().is_some_and(|(r, _)| *r != id) {
                    self.commit_rename();
                }
                if let Some(d) = self.doc_mut() {
                    d.doc.set_active(id);
                }
            }
            Message::ToggleVisible(id) => self.edit("Visibility", None, |doc| {
                doc.layer_mut(id).map(|l| l.visible = !l.visible).is_some()
            }),
            Message::AddLayer => self.edit("New Layer", None, |doc| {
                let n = doc.layers().len();
                doc.add_layer(format!("Layer {n}"));
                true
            }),
            Message::DuplicateLayer => self.edit("Duplicate Layer", None, |doc| {
                doc.active_id().and_then(|id| doc.duplicate_layer(id)).is_some()
            }),
            Message::DeleteLayer => {
                let active = self.document().and_then(|d| d.active());
                if active.is_some_and(|l| l.locked) {
                    self.error("The layer is locked");
                } else if self.document().is_some_and(|d| d.layers().len() <= 1) {
                    self.error("A document needs at least one layer");
                } else {
                    self.edit("Delete Layer", None, |doc| {
                        doc.active_id().and_then(|id| doc.remove_layer(id)).is_some()
                    });
                }
            }
            Message::MoveLayer(delta) => {
                if self.document().and_then(|d| d.active()).is_some_and(|l| l.locked) {
                    self.error("The layer is locked");
                } else {
                    self.edit("Move Layer", None, |doc| doc.active_id().is_some_and(|id| doc.move_layer(id, delta)));
                }
            }
            Message::LayerOpacity(o) => self.edit_layer("Opacity", Some("opacity"), |l| {
                l.opacity = o;
                true
            }),
            Message::LayerBlend(b) => self.edit_layer("Blend Mode", None, |l| {
                l.blend = b;
                true
            }),
            Message::ToggleLock => self.edit("Lock", None, |doc| {
                let Some(id) = doc.active_id() else { return false };
                doc.layer_mut(id).map(|l| l.locked = !l.locked).is_some()
            }),
            Message::ToggleLockOf(id) => self.edit("Lock", None, |doc| {
                doc.layer_mut(id).map(|l| l.locked = !l.locked).is_some()
            }),
            Message::StartRename(id) => {
                let name = self.document().and_then(|d| d.layer(id)).map(|l| l.name.clone());
                if let Some(name) = name {
                    self.renaming = Some((id, name));
                    return Task::batch([operation::focus(RENAME_INPUT), operation::select_all(RENAME_INPUT)]);
                }
            }
            Message::RenameInput(text) => {
                if let Some((_, name)) = &mut self.renaming {
                    *name = text;
                }
            }
            Message::CommitRename => self.commit_rename(),
            Message::SetBlendSpace(space) => self.edit("Blend Space", None, |doc| {
                let changed = doc.blend_space != space;
                doc.blend_space = space;
                changed
            }),

            Message::BrushSize(v) => self.brush.size = v.clamp(1.0, 5000.0),
            Message::BrushHardness(v) => self.brush.hardness = v,
            Message::BrushOpacity(v) => self.brush.opacity = v,
            Message::BrushFlow(v) => self.brush.flow = v,
            Message::BrushSmoothing(v) => self.brush.smoothing = v,
            Message::BrushMode(m) => self.brush.mode = m,

            Message::ZoomIn | Message::ZoomOut => {
                let anchor = Point::new(self.viewport.width / 2.0, self.viewport.height / 2.0);
                self.zoom_at(matches!(message, Message::ZoomIn), anchor);
            }
            Message::ZoomAt { zoom_in, anchor } => self.zoom_at(zoom_in, anchor),
            Message::ZoomTo(z) => {
                if let Some(d) = self.doc_mut() {
                    d.zoom = z;
                    d.pan = Vector::ZERO;
                }
            }
            Message::PanBy(delta) => {
                let z = self.zoom_factor();
                if let Some(d) = self.doc_mut() {
                    // Panning detaches from "fit" so the image stays where it was dragged.
                    d.zoom = Zoom::Fixed(z);
                    d.pan = d.pan + delta;
                }
            }
            Message::CanvasBounds(b) => {
                self.canvas_bounds = b;
                self.viewport = b.size();
            }
            Message::CursorMoved(p) => self.cursor = p,

            Message::ColorTab(i) => self.color_tab = i,
            Message::PropsTab(i) => self.props_tab = i,
            Message::LayersTab(i) => self.layers_tab = i,
            Message::ToggleStrip(i) => self.open_strip = (self.open_strip != Some(i)).then_some(i),

            Message::WindowOpened(id) => {
                self.window = id;
                return self.with_window(|id| tablet::handles(id).map(Message::TabletHandles));
            }
            Message::TabletHandles(h) => {
                if let Some(h) = h {
                    tablet::set_handles(h);
                    self.tablet_ready = true;
                }
            }
            Message::ToggleTablet => self.pen_pressure = !self.pen_pressure,
            Message::Tablet(TabletMessage::Unavailable(reason)) => {
                self.tablet_allowed = false;
                self.pen = None;
                self.error(format!("Pen tablet unavailable: {reason}"));
            }
            Message::Tablet(TabletMessage::Pen(e)) => return self.on_pen(e),

            Message::StrokeBegin { at, pressure, erase, line } => self.stroke_begin(at, pressure, erase, line),
            Message::StrokeTo { at, pressure } => self.stroke_to(at, pressure),
            Message::StrokeEnd => self.stroke_end(),
            Message::PickColor(at) => {
                if let Some(doc) = self.document()
                    && at.x >= 0.0
                    && at.y >= 0.0
                    && (at.x as u32) < doc.width
                    && (at.y as u32) < doc.height
                {
                    let [r, g, b, a] = jd_raster::composite::pixel_srgb8(doc, at.x as u32, at.y as u32);
                    if a > 0 {
                        self.fg = Hsv::from_rgba(Rgba::from_srgb8(r, g, b));
                    }
                }
            }
            Message::PressureSize(on) => self.brush.pressure_size = on,
            Message::PressureOpacity(on) => self.brush.pressure_opacity = on,
            Message::TitlePressed => {
                self.title_pressed = true;
                self.popup = None;
            }
            Message::TitleReleased => self.title_pressed = false,
            Message::TitleMoved(p) => {
                self.title_pointer = p;
                if let Some((i, local_x)) = self.hovered_menu.take() {
                    self.menu_left[i] = p.x - local_x;
                    // With a menu open, hovering another title switches to it.
                    if let Some(Popup { kind: PopupKind::MenuBar(j), .. }) = self.popup
                        && j != i
                    {
                        self.popup = Some(Popup { kind: PopupKind::MenuBar(i), at: Point::new(self.menu_left[i], 30.0), sub: None });
                    }
                }
                if std::mem::take(&mut self.title_pressed) {
                    return self.with_window(window::drag);
                }
            }
            Message::WindowResized(size) => {
                self.window_size = size;
                return self.with_window(|id| {
                    Task::batch([
                        window::is_maximized(id).map(Message::Maximized),
                        window::mode(id).map(Message::ModeChanged),
                    ])
                });
            }
            Message::Maximized(m) => self.maximized = m,
            Message::ModeChanged(mode) => self.fullscreen = mode == window::Mode::Fullscreen,
            Message::ResizeWindow(dir) => return self.with_window(move |id| window::drag_resize(id, dir)),
            Message::Minimize => return self.with_window(|id| window::minimize(id, true)),
            // State flags are updated from the compositor's answer (see `WindowResized`), so
            // they stay right when KDE maximises/snaps the window by other means.
            Message::ToggleMaximize => {
                self.title_pressed = false;
                return self.with_window(window::toggle_maximize);
            }
            Message::ToggleFullscreen => {
                let mode = if self.fullscreen { window::Mode::Windowed } else { window::Mode::Fullscreen };
                return self.with_window(move |id| window::set_mode(id, mode));
            }

            Message::Key(event) => return self.on_key(event),
            Message::Noop => {}
        }
        Task::none()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            keyboard::listen().map(Message::Key),
            if self.tablet_allowed && self.tablet_ready {
                tablet::subscription().map(Message::Tablet)
            } else {
                Subscription::none()
            },
            window::resize_events().map(|(_, size)| Message::WindowResized(size)),
            window::close_requests().map(|_| Message::RequestQuit),
            // Only fires on X11: winit 0.30 has no Wayland drag-and-drop yet.
            event::listen_with(|event, _status, _id| match event {
                iced::Event::Window(window::Event::FileDropped(path)) => Some(Message::Open(vec![path])),
                // Ends layer drags wherever the button is released.
                iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => Some(Message::PointerReleased),
                _ => None,
            }),
        ])
    }

    fn push_doc(&mut self, doc: Document, path: Option<PathBuf>) {
        let id = self.next_doc_id;
        self.next_doc_id += 1;
        self.docs.push(OpenDoc {
            id,
            doc,
            path,
            history: History::new_saved(),
            zoom: Zoom::Fit,
            pan: Vector::ZERO,
            thumbs: HashMap::new(),
            saving: false,
            last_paint: None,
            ants: Default::default(),
            last_selection: None,
        });
        self.active = self.docs.len() - 1;
    }

    fn close_doc(&mut self, i: usize) {
        self.docs.remove(i);
        if self.active >= i && self.active > 0 {
            self.active -= 1;
        }
        self.renaming = None;
    }

    fn finish_close(&mut self, i: usize, then: AfterConfirm) -> Task<Message> {
        self.close_doc(i);
        match then {
            AfterConfirm::CloseTab => Task::none(),
            AfterConfirm::Quit => self.continue_quit(),
        }
    }

    /// Asks about the next unsaved document, or closes the window when none are left.
    fn continue_quit(&mut self) -> Task<Message> {
        match self.docs.iter().position(OpenDoc::is_dirty) {
            Some(i) => {
                self.active = i;
                self.confirm = Some(Confirm { doc: self.docs[i].id, then: AfterConfirm::Quit });
                Task::none()
            }
            None => self.with_window(window::close),
        }
    }

    fn ask_save_path(&self, id: u64) -> Task<Message> {
        let Some(d) = self.docs.iter().find(|d| d.id == id) else { return Task::none() };
        let stem = Path::new(&d.doc.name).file_stem().map(|s| s.to_string_lossy().into_owned());
        let suggested = format!("{}.{}", stem.unwrap_or_else(|| "Untitled".into()), jd_io::jdp::EXTENSION);
        let dir = d.path.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf);
        Task::perform(
            async move {
                let mut dialog = rfd::AsyncFileDialog::new()
                    .set_title("Save As")
                    .set_file_name(suggested)
                    .add_filter("Gallerion document", &[jd_io::jdp::EXTENSION]);
                if let Some(dir) = dir {
                    dialog = dialog.set_directory(dir);
                }
                dialog.save_file().await.map(|f| {
                    let mut p = f.path().to_path_buf();
                    if !jd_io::is_document(&p) {
                        p.set_extension(jd_io::jdp::EXTENSION);
                    }
                    p
                })
            },
            move |path| Message::SaveTo(id, path),
        )
    }

    fn save(&mut self, id: u64, path: PathBuf) -> Task<Message> {
        let Some(i) = self.doc_index(id) else { return Task::none() };
        let d = &mut self.docs[i];
        if d.saving {
            // Two writers on one temp file would corrupt it; the running save will finish.
            return Task::none();
        }
        d.history.end_gesture();
        d.saving = true;
        let depth = d.history.depth();
        let doc = d.doc.clone();
        Task::perform(
            {
                let path = path.clone();
                async move { jd_io::jdp::save(&doc, &path).map_err(|e| e.to_string()) }
            },
            move |result| Message::Saved { doc: id, path: path.clone(), depth, result },
        )
    }

    /// Finishes a layer drag: reorder within the document, or copy into another one.
    fn drop_layer(&mut self, drag: LayerDrag) {
        let Some(src) = self.doc_index(drag.doc) else { return };
        let into_other = self.active != src;
        match drag.target {
            Some(DropTarget::Layer(target)) if !into_other && target != drag.layer => {
                let doc = &self.docs[src].doc;
                let (Some(from), Some(to)) = (
                    doc.layers().iter().position(|l| l.id == drag.layer),
                    doc.layers().iter().position(|l| l.id == target),
                ) else {
                    return;
                };
                if doc.layers()[from].locked {
                    self.error("The layer is locked");
                    return;
                }
                self.edit("Move Layer", None, |doc| doc.move_layer(drag.layer, to as isize - from as isize));
            }
            Some(DropTarget::Tab(_) | DropTarget::Canvas | DropTarget::Layer(_)) if into_other => {
                self.copy_layer_into(drag.doc, drag.layer);
            }
            _ => {}
        }
    }

    /// Copies a layer from document `src_doc` into the active document (centred if the sizes
    /// differ), as one undo step there.
    fn copy_layer_into(&mut self, src_doc: u64, layer: LayerId) {
        let Some(src) = self.doc_index(src_doc) else { return };
        let Some(layer) = self.docs[src].doc.layer(layer).cloned() else { return };
        self.edit("Duplicate Layer", None, |doc| {
            let pixels = centred(&layer.pixels, doc.width, doc.height);
            let id = doc.add_layer(layer.name.clone());
            doc.layer_mut(id)
                .map(|l| {
                    l.pixels = pixels;
                    l.opacity = layer.opacity;
                    l.blend = layer.blend;
                    l.visible = layer.visible;
                })
                .is_some()
        });
        let target = self.docs[self.active].doc.name.clone();
        self.info(format!("Copied layer \"{}\" into {target}", layer.name));
    }

    fn dialog_ok(&mut self) -> Task<Message> {
        let Some(d) = self.dialog.take() else { return Task::none() };
        let a = Dialog::number(&d.a);
        let b = Dialog::number(&d.b);
        match d.kind {
            DialogKind::ImageSize | DialogKind::CanvasSize => {
                let (Some(w), Some(h)) = (a, b) else {
                    self.error("Width and height must be between 1 and 30000 pixels");
                    self.dialog = Some(d);
                    return Task::none();
                };
                let bg = self.bg.to_rgba().to_srgb8();
                let anchor = (d.anchor.0 as f32 / 2.0, d.anchor.1 as f32 / 2.0);
                let image = d.kind == DialogKind::ImageSize;
                self.edit(if image { "Image Size" } else { "Canvas Size" }, None, |doc| {
                    if (w, h) == (doc.width, doc.height) {
                        return false;
                    }
                    if image {
                        doc.resize_image(w, h)
                    } else {
                        doc.canvas_size(w, h, anchor, bg)
                    }
                    true
                });
            }
            kind => {
                let Some(v) = a.filter(|v| *v <= 500) else {
                    self.error("Enter a value from 1 to 500 pixels");
                    self.dialog = Some(d);
                    return Task::none();
                };
                if kind == DialogKind::Stroke {
                    if !self.check_layer_editable() {
                        return Task::none();
                    }
                    let color = self.fg.to_rgba().to_srgb8();
                    self.edit("Stroke", None, |doc| {
                        let Some(sel) = doc.selection.clone() else { return false };
                        let band = sel.border(v);
                        let Some(id) = doc.active_id() else { return false };
                        doc.layer_mut(id).map(|l| selection::fill(&mut l.pixels, &band, color)).is_some()
                    });
                } else {
                    let label = match kind {
                        DialogKind::Expand => "Expand",
                        DialogKind::Contract => "Contract",
                        _ => "Feather",
                    };
                    self.edit(label, None, |doc| {
                        let Some(sel) = doc.selection.take() else { return false };
                        let out = match kind {
                            DialogKind::Expand => sel.expand(v),
                            DialogKind::Contract => sel.contract(v),
                            _ => sel.feather(v as f32),
                        };
                        doc.selection = (!out.is_empty()).then_some(out);
                        true
                    });
                }
            }
        }
        Task::none()
    }

    /// Messages that leave an open Move/Free Transform alone (view, input plumbing, its own).
    fn keeps_transform(m: &Message) -> bool {
        matches!(
            m,
            Message::MoveBegin(_)
                | Message::MoveTo(_)
                | Message::MoveEnd
                | Message::TransformGrab(..)
                | Message::TransformDrag { .. }
                | Message::TransformRelease
                | Message::CommitTransform
                | Message::CancelTransform
                | Message::FreeTransform
                | Message::ZoomIn
                | Message::ZoomOut
                | Message::ZoomAt { .. }
                | Message::ZoomTo(_)
                | Message::PanBy(_)
                | Message::CanvasBounds(_)
                | Message::CursorMoved(_)
                | Message::Key(_)
                | Message::Tablet(_)
                | Message::TabletHandles(_)
                | Message::ColorTab(_)
                | Message::PropsTab(_)
                | Message::LayersTab(_)
                | Message::ToggleStrip(_)
                | Message::WindowOpened(_)
                | Message::TitlePressed
                | Message::TitleMoved(_)
                | Message::TitleReleased
                | Message::WindowResized(_)
                | Message::OpenPopup(..)
                | Message::ClosePopup
                | Message::ToolbarPointer(_)
                | Message::MenuPointer(..)
                | Message::MenuTitle(_)
                | Message::PopupSub(_)
                | Message::DockPointer(_)
                | Message::OpenPalette
                | Message::PaletteInput(_)
                | Message::ClosePalette
                | Message::DragOver(_)
                | Message::PointerReleased
                | Message::Maximized(_)
                | Message::ModeChanged(_)
                | Message::ResizeWindow(_)
                | Message::Minimize
                | Message::ToggleMaximize
                | Message::ToggleFullscreen
                | Message::SwapColors
                | Message::ResetColors
                | Message::PickSv(..)
                | Message::PickHue(_)
                | Message::SetFg(_)
                | Message::Noop
        )
    }

    /// Errors (and returns false) if the active layer can't be edited.
    fn check_layer_editable(&mut self) -> bool {
        match self.document().and_then(|d| d.active()) {
            None => false,
            Some(l) if l.locked => {
                self.error("The layer is locked");
                false
            }
            Some(_) => true,
        }
    }

    /// Combines a new selection shape with the current one as one undo step. A click without a
    /// shape (empty result) in Replace mode deselects, like Photoshop.
    fn apply_selection(&mut self, label: &str, new: Selection, combine: Combine) {
        self.edit(label, None, |doc| {
            let mut sel = doc.selection.take().unwrap_or_else(|| Selection::empty(doc.width, doc.height));
            sel.combine(&new, combine);
            doc.selection = (!sel.is_empty()).then_some(sel);
            true
        });
    }

    /// Opens a Move/Free Transform session on the active layer (or its selected pixels).
    fn begin_transform(&mut self, kind: transform::Kind) -> bool {
        let Some(d) = self.doc() else { return false };
        match transform::Session::begin(d.id, &d.doc, kind) {
            Ok(session) => {
                self.transform = Some(session);
                true
            }
            Err(e) => {
                self.error(e);
                false
            }
        }
    }

    fn commit_transform(&mut self) {
        let Some(session) = self.transform.take() else { return };
        if session.is_identity() {
            return;
        }
        let label = match session.kind {
            transform::Kind::Move => "Move",
            transform::Kind::Free => "Free Transform",
        };
        let Some(i) = self.doc_index(session.doc) else { return };
        let d = &mut self.docs[i];
        d.history.record(label, &d.doc, None);
        session.commit(&mut d.doc);
    }

    fn brush_params(&self, erase: bool) -> BrushParams {
        let b = &self.brush;
        BrushParams {
            size: b.size,
            hardness: b.hardness,
            opacity: b.opacity,
            flow: b.flow,
            spacing: 0.1,
            pressure_size: b.pressure_size,
            pressure_opacity: b.pressure_opacity,
            color: self.fg.to_rgba().to_srgb8(),
            mode: b.mode,
            erase,
        }
    }

    /// Starts painting on the active layer. `line`: Shift-click draws a straight line from the
    /// end of the previous stroke, like Photoshop.
    fn stroke_begin(&mut self, at: Point, pressure: f32, erase: bool, line: bool) {
        self.stroke = None;
        let Some(layer) = self.document().and_then(|d| d.active()) else { return };
        if layer.locked {
            self.error("The layer is locked");
            return;
        }
        if !layer.visible {
            self.error("The layer is hidden");
            return;
        }
        let params = self.brush_params(erase);
        let from = if line { self.doc().and_then(|d| d.last_paint) } else { None };
        let label = if erase { "Eraser" } else { "Brush Tool" };
        let Some(d) = self.doc_mut() else { return };
        let Some(mut stroke) = Stroke::begin(&d.doc, params) else { return };
        d.history.record(label, &d.doc, None);
        if let Some(p) = from {
            stroke.add_point(&mut d.doc, p.x, p.y, pressure);
        }
        stroke.add_point(&mut d.doc, at.x, at.y, pressure);
        let id = d.id;
        self.stroke = Some(ActiveStroke { doc: id, stroke, smoothed: at, last_raw: (at, pressure) });
    }

    fn stroke_to(&mut self, at: Point, pressure: f32) {
        let smoothing = self.brush.smoothing.clamp(0.0, 0.95);
        let Some(s) = self.stroke.as_mut() else { return };
        let Some(i) = self.docs.iter().position(|d| d.id == s.doc) else {
            self.stroke = None;
            return;
        };
        // Exponential smoothing: higher Smoothing = steadier line, more lag.
        let k = 1.0 - smoothing;
        s.smoothed = Point::new(s.smoothed.x + (at.x - s.smoothed.x) * k, s.smoothed.y + (at.y - s.smoothed.y) * k);
        s.last_raw = (at, pressure);
        s.stroke.add_point(&mut self.docs[i].doc, s.smoothed.x, s.smoothed.y, pressure);
    }

    fn stroke_end(&mut self) {
        let Some(mut s) = self.stroke.take() else { return };
        let Some(i) = self.docs.iter().position(|d| d.id == s.doc) else { return };
        // Catch the smoothed line up with where the input actually stopped.
        let (at, pressure) = s.last_raw;
        s.stroke.add_point(&mut self.docs[i].doc, at.x, at.y, pressure);
        self.docs[i].last_paint = Some(at);
    }

    fn to_doc(&self, canvas_local: Point) -> Option<Point> {
        let (origin, z) = self.doc_origin()?;
        Some(Point::new((canvas_local.x - origin.x) / z, (canvas_local.y - origin.y) / z))
    }

    /// Pen events arrive in window coordinates.
    fn on_pen(&mut self, e: tablet::PenEvent) -> Task<Message> {
        let b = self.canvas_bounds;
        let local = Point::new(e.position.x - b.x, e.position.y - b.y);
        let over_canvas = b.contains(e.position);
        // Update first so the eraser end switches the tool before anything else happens.
        let down = self.stroke.is_some();
        self.pen = Some(Pen { position: local, down, eraser: e.eraser });
        let tool = self.effective_tool();
        // Without pressure the pen just paints through its pointer events, like a mouse. While
        // a transform or dialog is open the pen only works the UI (via its pointer events).
        let busy = self.transform.is_some() || self.adjusting.is_some();
        let paints = tool.paints() && self.pen_pressure && !busy;
        match e.phase {
            Phase::Out => {
                self.pen = None;
                if self.stroke.is_some() {
                    self.stroke_end();
                }
                return Task::none();
            }
            Phase::Down if over_canvas => {
                if let Some(at) = self.to_doc(local) {
                    if tool == Tool::Eyedropper && !busy {
                        return self.update(Message::PickColor(at));
                    }
                    if paints {
                        self.stroke_begin(at, e.pressure, tool == Tool::Eraser, false);
                    }
                }
            }
            Phase::Move => {
                if let Some(at) = self.to_doc(local) {
                    if self.stroke.is_some() {
                        self.stroke_to(at, e.pressure);
                    } else {
                        self.cursor = over_canvas.then_some(at);
                    }
                }
            }
            Phase::Up => self.stroke_end(),
            Phase::Near | Phase::Down => {}
        }
        self.pen = Some(Pen { position: local, down: self.stroke.is_some(), eraser: e.eraser });
        Task::none()
    }

    fn commit_rename(&mut self) {
        let Some((id, name)) = self.renaming.take() else { return };
        let name = name.trim().to_owned();
        if name.is_empty() {
            return;
        }
        self.edit("Rename Layer", None, |doc| match doc.layer_mut(id) {
            Some(l) if l.name != name => {
                l.name = name;
                true
            }
            _ => false,
        });
    }

    /// Rebuilds layer thumbnails whose tiles changed (cheap check: compares tile ids).
    fn refresh_thumbnails(&mut self) {
        let Some(d) = self.docs.get_mut(self.active) else { return };
        let mut live = HashMap::new();
        for layer in d.doc.layers() {
            let ids: Vec<TileId> = layer.pixels.iter().map(|(_, _, t)| t.id()).collect();
            let handle = match d.thumbs.remove(&layer.id) {
                Some((old, h)) if old == ids => h,
                _ => {
                    let (w, h, px) = jd_raster::composite::thumbnail(&d.doc, 40, Some(layer));
                    image::Handle::from_rgba(w, h, px)
                }
            };
            live.insert(layer.id, (ids, handle));
        }
        d.thumbs = live;
    }

    fn with_window(&self, f: impl FnOnce(window::Id) -> Task<Message>) -> Task<Message> {
        self.window.map_or_else(Task::none, f)
    }

    fn zoom_at(&mut self, zoom_in: bool, anchor: Point) {
        let Some(d) = self.doc() else { return };
        let current = self.resolve_zoom(d);
        let next = if zoom_in {
            ZOOM_STEPS.iter().copied().find(|&z| z > current * 1.001)
        } else {
            ZOOM_STEPS.iter().rev().copied().find(|&z| z < current * 0.999)
        };
        let Some(next) = next else { return };

        // Keep the document point under `anchor` fixed on screen.
        let origin = self.origin_for(d, current, d.pan);
        let doc_point = Vector::new((anchor.x - origin.x) / current, (anchor.y - origin.y) / current);
        let new_origin = Point::new(anchor.x - doc_point.x * next, anchor.y - doc_point.y * next);
        let centred = self.origin_for(d, next, Vector::ZERO);
        let pan = new_origin - centred;

        if let Some(d) = self.doc_mut() {
            d.zoom = Zoom::Fixed(next);
            d.pan = pan;
        }
    }

    /// Photoshop-style shortcuts.
    fn on_key(&mut self, event: keyboard::Event) -> Task<Message> {
        let (key, modifiers, pressed) = match event {
            keyboard::Event::KeyPressed { key, modifiers, .. } => (key, modifiers, true),
            keyboard::Event::KeyReleased { key, modifiers, .. } => (key, modifiers, false),
            _ => return Task::none(),
        };
        if let Key::Named(Named::Space) = key.as_ref() {
            self.space_held = pressed;
            return Task::none();
        }
        if !pressed {
            return Task::none();
        }
        if self.palette.is_some() && matches!(key.as_ref(), Key::Named(Named::Escape)) {
            self.palette = None;
            return Task::none();
        }
        if self.popup.is_some() && matches!(key.as_ref(), Key::Named(Named::Escape)) {
            self.popup = None;
            return Task::none();
        }
        if self.confirm.is_some() {
            let choice = match key.as_ref() {
                Key::Named(Named::Escape) => Some(Choice::Cancel),
                Key::Named(Named::Enter) => Some(Choice::Save),
                _ => None,
            };
            return choice.map_or_else(Task::none, |c| self.update(Message::Confirmed(c)));
        }
        if self.adjusting.is_some() {
            // Dialog open: Enter/Esc answer it; zoom keys still work; nothing else edits.
            return match key.as_ref() {
                Key::Named(Named::Escape) => self.update(Message::AdjustCancel),
                Key::Named(Named::Enter) => self.update(Message::AdjustOk),
                Key::Character("=" | "+") if modifiers.command() => self.update(Message::ZoomIn),
                Key::Character("-") if modifiers.command() => self.update(Message::ZoomOut),
                Key::Character("0") if modifiers.command() => self.update(Message::ZoomTo(Zoom::Fit)),
                _ => Task::none(),
            };
        }
        if self.dialog.is_some() {
            return match key.as_ref() {
                Key::Named(Named::Escape) => self.update(Message::DialogCancel),
                Key::Named(Named::Enter) => self.update(Message::DialogOk),
                _ => Task::none(),
            };
        }
        if self.new_doc.is_some() {
            return match key.as_ref() {
                Key::Named(Named::Escape) => self.update(Message::CancelNewDoc),
                Key::Named(Named::Enter) => self.update(Message::CreateNewDoc),
                _ => Task::none(),
            };
        }
        if self.transform.is_some() {
            match key.as_ref() {
                Key::Named(Named::Escape) => return self.update(Message::CancelTransform),
                Key::Named(Named::Enter) => return self.update(Message::CommitTransform),
                _ => {}
            }
        }
        let (cmd, shift, alt) = (modifiers.command(), modifiers.shift(), modifiers.alt());
        let msg = match key.as_ref() {
            Key::Character(c) if cmd => match c.to_lowercase().as_str() {
                "=" | "+" => Some(Message::ZoomIn),
                "-" => Some(Message::ZoomOut),
                "0" => Some(Message::ZoomTo(Zoom::Fit)),
                "1" => Some(Message::ZoomTo(Zoom::Fixed(1.0))),
                "o" => Some(Message::OpenDialog),
                "n" if !shift => Some(Message::NewDocDialog),
                "a" => Some(Message::SelectAll),
                "i" if shift => Some(Message::InvertSelection),
                "x" => Some(Message::Cut),
                "v" => Some(Message::Paste),
                "t" => Some(Message::FreeTransform),
                "l" if shift && alt => Some(Message::AutoContrast),
                "l" if shift => Some(Message::AutoTone),
                "l" => Some(Message::OpenOp(OpKind::Levels)),
                "e" if shift => Some(Message::MergeVisible),
                "e" => Some(Message::MergeDown),
                "f" if alt => Some(Message::LastFilter),
                "f" => Some(Message::OpenPalette),
                "c" if alt => Some(Message::ShowDialog(DialogKind::CanvasSize)),
                "c" if shift => Some(Message::CopyMerged),
                "d" if shift => Some(Message::Reselect),
                "d" => Some(Message::Deselect),
                "c" => Some(Message::Copy),
                "m" => Some(Message::OpenOp(OpKind::Curves)),
                "u" if shift => Some(Message::OpenOp(OpKind::Desaturate)),
                "u" => Some(Message::OpenOp(OpKind::HueSaturation)),
                "i" if alt => Some(Message::ShowDialog(DialogKind::ImageSize)),
                "i" if !shift => Some(Message::OpenOp(OpKind::Invert)),
                "s" if shift => Some(Message::SaveAs),
                "s" => Some(Message::Save),
                "w" if shift && alt => Some(Message::Export),
                "w" => Some(Message::RequestCloseDoc(self.active)),
                "q" => Some(Message::RequestQuit),
                "z" if shift => Some(Message::Redo),
                "z" => Some(Message::Undo),
                "y" => Some(Message::Redo),
                "j" => Some(Message::LayerVia { cut: shift }),
                "n" if shift => Some(Message::AddLayer),
                "]" | "}" if shift => Some(Message::ArrangeEnd(true)),
                "[" | "{" if shift => Some(Message::ArrangeEnd(false)),
                "]" => Some(Message::MoveLayer(1)),
                "[" => Some(Message::MoveLayer(-1)),
                _ => None,
            },
            Key::Character(c) if modifiers.is_empty() || modifiers == keyboard::Modifiers::SHIFT => match c {
                "x" => Some(Message::SwapColors),
                "d" => Some(Message::ResetColors),
                "[" => Some(Message::BrushSize(self.brush.size * 0.9)),
                "]" => Some(Message::BrushSize(self.brush.size * 1.1 + 1.0)),
                // A tool key picks that slot's current tool; with Shift it cycles the slot's
                // alternates (Shift+M: rectangle/ellipse marquee), like Photoshop.
                _ => c.chars().next().and_then(|ch| Tool::slot_for_shortcut(ch.to_ascii_lowercase())).map(|slot| {
                    let current = self.slot_tools[slot];
                    let tools = crate::tool::slots().nth(slot).expect("slot exists");
                    let tool = if shift && current == self.tool {
                        let i = tools.iter().position(|t| *t == current).unwrap_or(0);
                        tools[(i + 1) % tools.len()]
                    } else {
                        current
                    };
                    Message::SelectTool(tool)
                }),
            },
            Key::Named(Named::Delete | Named::Backspace) => Some(if alt {
                Message::Fill(true)
            } else if cmd {
                Message::Fill(false)
            } else if self.document().is_some_and(|d| d.selection.is_some()) {
                Message::ClearSelected
            } else {
                Message::DeleteLayer
            }),
            Key::Named(arrow @ (Named::ArrowLeft | Named::ArrowRight | Named::ArrowUp | Named::ArrowDown))
                if self.tool == Tool::Move =>
            {
                let step = if shift { 10.0 } else { 1.0 };
                Some(match arrow {
                    Named::ArrowLeft => Message::Nudge(-step, 0.0),
                    Named::ArrowRight => Message::Nudge(step, 0.0),
                    Named::ArrowUp => Message::Nudge(0.0, -step),
                    _ => Message::Nudge(0.0, step),
                })
            }
            Key::Named(Named::F11) => Some(Message::ToggleFullscreen),
            Key::Named(Named::F6) if shift => Some(Message::ShowDialog(DialogKind::Feather)),
            Key::Named(Named::Tab) if modifiers.control() => {
                (!self.docs.is_empty()).then(|| Message::SelectDoc((self.active + 1) % self.docs.len()))
            }
            _ => None,
        };
        msg.map_or_else(Task::none, |m| self.update(m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_doc(w: u32, h: u32) -> Photo {
        let (mut app, _) = Photo::new();
        app.viewport = Size::new(1000.0, 800.0);
        app.push_doc(Document::new("t", w, h), None);
        app
    }

    fn doc_point_under(app: &Photo, screen: Point) -> (f32, f32) {
        let (origin, z) = app.doc_origin().unwrap();
        ((screen.x - origin.x) / z, (screen.y - origin.y) / z)
    }

    #[test]
    fn fit_centres_and_never_upscales() {
        let app = app_with_doc(4000, 2000);
        let (origin, z) = app.doc_origin().unwrap();
        assert!((z - (1000.0 - 60.0) / 4000.0).abs() < 1e-6);
        assert!((origin.x + 4000.0 * z / 2.0 - 500.0).abs() < 1e-3);

        let small = app_with_doc(100, 100);
        assert_eq!(small.zoom_factor(), 1.0);
    }

    #[test]
    fn zoom_at_keeps_anchor_fixed() {
        let mut app = app_with_doc(4000, 2000);
        let anchor = Point::new(730.0, 210.0);
        let before = doc_point_under(&app, anchor);
        for zoom_in in [true, true, true, false, true] {
            let _ = app.update(Message::ZoomAt { zoom_in, anchor });
            let after = doc_point_under(&app, anchor);
            assert!((before.0 - after.0).abs() < 0.01 && (before.1 - after.1).abs() < 0.01, "{before:?} vs {after:?}");
        }
        assert!(app.zoom_factor() > 0.235);
    }

    #[test]
    fn pan_moves_document_and_leaves_fit() {
        let mut app = app_with_doc(4000, 2000);
        let (o1, z1) = app.doc_origin().unwrap();
        let _ = app.update(Message::PanBy(Vector::new(15.0, -4.0)));
        let (o2, z2) = app.doc_origin().unwrap();
        assert_eq!(z1, z2);
        assert_eq!(o2 - o1, Vector::new(15.0, -4.0));
        assert!(matches!(app.doc().unwrap().zoom, Zoom::Fixed(_)));
        let _ = app.update(Message::ZoomTo(Zoom::Fit));
        assert_eq!(app.doc_origin().unwrap().0, o1);
    }

    #[test]
    fn closing_tabs_keeps_a_valid_active_index() {
        let mut app = app_with_doc(10, 10);
        app.push_doc(Document::new("b", 10, 10), None);
        let _ = app.update(Message::RequestCloseDoc(1));
        assert_eq!(app.active, 0);
        let _ = app.update(Message::RequestCloseDoc(0));
        assert!(app.doc().is_none());
    }

    #[test]
    fn edits_are_undoable_and_mark_dirty() {
        let mut app = app_with_doc(10, 10);
        assert!(!app.doc().unwrap().is_dirty());
        let _ = app.update(Message::AddLayer);
        let _ = app.update(Message::LayerBlend(BlendMode::Multiply));
        assert!(app.doc().unwrap().is_dirty());
        assert_eq!(app.document().unwrap().active().unwrap().blend, BlendMode::Multiply);
        let _ = app.update(Message::Undo);
        assert_eq!(app.document().unwrap().active().unwrap().blend, BlendMode::Normal);
        let _ = app.update(Message::Undo);
        assert_eq!(app.document().unwrap().layers().len(), 1);
        assert!(!app.doc().unwrap().is_dirty());
        let _ = app.update(Message::Redo);
        assert_eq!(app.document().unwrap().layers().len(), 2);
    }

    #[test]
    fn opacity_drag_is_one_undo_step() {
        let mut app = app_with_doc(10, 10);
        let _ = app.update(Message::AddLayer);
        for o in [0.9, 0.7, 0.4] {
            let _ = app.update(Message::LayerOpacity(o));
        }
        let _ = app.update(Message::EndGesture);
        let _ = app.update(Message::Undo);
        assert_eq!(app.document().unwrap().active().unwrap().opacity, 1.0);
    }

    #[test]
    fn locked_layer_refuses_edits() {
        let mut app = app_with_doc(10, 10);
        let _ = app.update(Message::LayerOpacity(0.3));
        assert_eq!(app.document().unwrap().active().unwrap().opacity, 1.0);
        assert!(app.notice.as_ref().is_some_and(|n| n.error));
        assert!(!app.doc().unwrap().is_dirty());
    }

    #[test]
    fn closing_dirty_tab_asks_first_and_discard_closes() {
        let mut app = app_with_doc(10, 10);
        let _ = app.update(Message::AddLayer);
        let _ = app.update(Message::RequestCloseDoc(0));
        assert!(app.confirm.is_some());
        assert_eq!(app.docs.len(), 1);
        let _ = app.update(Message::Confirmed(Choice::Cancel));
        assert!(app.confirm.is_none() && app.docs.len() == 1);
        let _ = app.update(Message::RequestCloseDoc(0));
        let _ = app.update(Message::Confirmed(Choice::Discard));
        assert!(app.docs.is_empty());
    }

    #[test]
    fn undo_keeps_the_name_from_save_as() {
        let mut app = app_with_doc(10, 10);
        let _ = app.update(Message::AddLayer);
        let id = app.doc().unwrap().id;
        let _ = app.update(Message::Saved { doc: id, path: "/tmp/x.jdp".into(), depth: 1, result: Ok(()) });
        assert_eq!(app.document().unwrap().name, "x.jdp");
        let _ = app.update(Message::Undo);
        assert_eq!(app.document().unwrap().name, "x.jdp");
        assert!(app.doc().unwrap().is_dirty());
    }

    #[test]
    fn save_while_saving_is_ignored_and_dirty_save_keeps_dialog() {
        let mut app = app_with_doc(10, 10);
        let _ = app.update(Message::AddLayer);
        let id = app.doc().unwrap().id;
        let _ = app.update(Message::RequestCloseDoc(0));
        let _ = app.save(id, "/tmp/y.jdp".into());
        assert!(app.doc().unwrap().saving);
        // A second save request while the first runs does nothing.
        let _ = app.save(id, "/tmp/y.jdp".into());
        // An edit lands during the save, so finishing it must not close the tab.
        let _ = app.update(Message::DuplicateLayer);
        let _ = app.update(Message::Saved { doc: id, path: "/tmp/y.jdp".into(), depth: 1, result: Ok(()) });
        assert_eq!(app.docs.len(), 1);
        assert!(app.confirm.is_some());
    }

    #[test]
    fn unlocking_background_allows_edits() {
        let mut app = app_with_doc(10, 10);
        let _ = app.update(Message::ToggleLock);
        let _ = app.update(Message::LayerOpacity(0.5));
        assert_eq!(app.document().unwrap().active().unwrap().opacity, 0.5);
    }

    fn white(app: &mut Photo, w: u32, h: u32) {
        let px = [255, 255, 255, 255].repeat((w * h) as usize);
        app.docs.clear();
        app.push_doc(Document::from_rgba8("w", w, h, &px), None);
        let _ = app.update(Message::ToggleLock);
        app.fg = Hsv { h: 0.0, s: 0.0, v: 0.0 };
    }

    fn px(app: &Photo, x: u32, y: u32) -> [u8; 4] {
        app.document().unwrap().layers()[0].pixels.pixel(x, y)
    }

    #[test]
    fn a_stroke_paints_and_is_one_undo_step() {
        let (mut app, _) = Photo::new();
        white(&mut app, 200, 100);
        app.brush.size = 10.0;
        app.brush.smoothing = 0.0;
        let _ = app.update(Message::StrokeBegin { at: Point::new(20.0, 50.0), pressure: 1.0, erase: false, line: false });
        for x in (30..=180).step_by(10) {
            let _ = app.update(Message::StrokeTo { at: Point::new(x as f32, 50.0), pressure: 1.0 });
        }
        let _ = app.update(Message::StrokeEnd);
        assert_eq!(px(&app, 100, 50)[0], 0);
        let _ = app.update(Message::Undo);
        assert_eq!(px(&app, 100, 50), [255, 255, 255, 255]);
        let _ = app.update(Message::Redo);
        assert_eq!(px(&app, 100, 50)[0], 0);
    }

    #[test]
    fn shift_click_draws_a_line_from_the_last_stroke() {
        let (mut app, _) = Photo::new();
        white(&mut app, 200, 100);
        app.brush.size = 6.0;
        let _ = app.update(Message::StrokeBegin { at: Point::new(20.0, 20.0), pressure: 1.0, erase: false, line: false });
        let _ = app.update(Message::StrokeEnd);
        let _ = app.update(Message::StrokeBegin { at: Point::new(180.0, 20.0), pressure: 1.0, erase: false, line: true });
        let _ = app.update(Message::StrokeEnd);
        assert_eq!(px(&app, 100, 20)[0], 0);
    }

    #[test]
    fn painting_a_locked_layer_is_refused() {
        let mut app = app_with_doc(50, 50);
        let _ = app.update(Message::StrokeBegin { at: Point::new(5.0, 5.0), pressure: 1.0, erase: false, line: false });
        assert!(app.stroke.is_none());
        assert!(!app.doc().unwrap().is_dirty());
    }

    #[test]
    fn eyedropper_picks_composite_colour() {
        let (mut app, _) = Photo::new();
        app.push_doc(Document::from_rgba8("c", 1, 1, &[255, 0, 0, 255]), None);
        let _ = app.update(Message::PickColor(Point::new(0.5, 0.5)));
        assert_eq!(app.fg.to_rgba().to_srgb8(), [255, 0, 0]);
    }

    #[test]
    fn pen_events_map_window_to_document_coordinates() {
        let (mut app, _) = Photo::new();
        white(&mut app, 100, 100);
        app.brush.size = 4.0;
        // Canvas at (50, 80) in the window, 1:1 zoom, document centred.
        let _ = app.update(Message::CanvasBounds(Rectangle::new(Point::new(50.0, 80.0), Size::new(300.0, 200.0))));
        let _ = app.update(Message::ZoomTo(Zoom::Fixed(1.0)));
        let (origin, _) = app.doc_origin().unwrap();
        let window = |dx: f32, dy: f32| Point::new(50.0 + origin.x + dx, 80.0 + origin.y + dy);
        let pen = |phase, position, pressure| {
            Message::Tablet(TabletMessage::Pen(tablet::PenEvent { phase, position, pressure, eraser: false }))
        };
        let _ = app.update(pen(Phase::Near, window(10.0, 10.0), 0.0));
        let _ = app.update(pen(Phase::Down, window(10.0, 10.0), 0.8));
        let _ = app.update(pen(Phase::Move, window(60.0, 10.0), 0.8));
        let _ = app.update(pen(Phase::Up, window(60.0, 10.0), 0.0));
        assert_eq!(px(&app, 35, 10)[0], 0);
        assert_eq!(px(&app, 35, 40), [255, 255, 255, 255]);
        assert!(app.doc().unwrap().is_dirty());
    }

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Message {
        Message::SelectShape { shape: Shape::Rect([x0, y0, x1, y1]), combine: Combine::Replace }
    }

    #[test]
    fn delete_clears_only_the_selection_and_undoes() {
        let (mut app, _) = Photo::new();
        white(&mut app, 100, 100);
        let _ = app.update(rect(10.0, 10.0, 20.0, 20.0));
        let _ = app.update(Message::ClearSelected);
        assert_eq!(px(&app, 15, 15)[3], 0);
        assert_eq!(px(&app, 25, 25), [255, 255, 255, 255]);
        let _ = app.update(Message::Undo);
        assert_eq!(px(&app, 15, 15), [255, 255, 255, 255]);
        assert!(app.document().unwrap().selection.is_some(), "undoing Clear keeps the selection");
        let _ = app.update(Message::Undo);
        assert!(app.document().unwrap().selection.is_none(), "selections are undoable");
    }

    #[test]
    fn select_all_invert_and_click_deselect() {
        let (mut app, _) = Photo::new();
        white(&mut app, 50, 50);
        let _ = app.update(Message::SelectAll);
        let _ = app.update(Message::InvertSelection);
        assert!(app.document().unwrap().selection.is_none(), "inverting everything selects nothing");
        let _ = app.update(rect(0.0, 0.0, 10.0, 10.0));
        let _ = app.update(Message::Deselect);
        assert!(app.document().unwrap().selection.is_none());
    }

    #[test]
    fn layer_via_copy_and_paste() {
        let (mut app, _) = Photo::new();
        white(&mut app, 100, 100);
        let _ = app.update(rect(10.0, 10.0, 20.0, 20.0));
        let _ = app.update(Message::LayerVia { cut: false });
        let doc = app.document().unwrap();
        assert_eq!(doc.layers().len(), 2);
        assert_eq!(doc.layers()[1].pixels.pixel(15, 15), [255, 255, 255, 255]);
        assert_eq!(doc.layers()[1].pixels.pixel(25, 25)[3], 0);

        let _ = app.update(Message::Copy);
        let _ = app.update(Message::Paste);
        assert_eq!(app.document().unwrap().layers().len(), 3);
    }

    #[test]
    fn fill_uses_foreground_inside_selection() {
        let (mut app, _) = Photo::new();
        white(&mut app, 40, 40);
        app.fg = Hsv::from_rgba(Rgba::from_srgb8(0, 0, 255));
        let _ = app.update(rect(0.0, 0.0, 10.0, 10.0));
        let _ = app.update(Message::Fill(true));
        assert_eq!(px(&app, 5, 5), [0, 0, 255, 255]);
        assert_eq!(px(&app, 20, 20), [255, 255, 255, 255]);
    }

    #[test]
    fn nudge_moves_selected_pixels_one_step_each() {
        let (mut app, _) = Photo::new();
        white(&mut app, 50, 50);
        app.fg = Hsv { h: 0.0, s: 0.0, v: 0.0 };
        let _ = app.update(rect(10.0, 10.0, 12.0, 12.0));
        let _ = app.update(Message::Fill(true));
        let _ = app.update(Message::Nudge(10.0, 0.0));
        assert_eq!(px(&app, 20, 10), [0, 0, 0, 255]);
        assert_eq!(px(&app, 10, 10)[3], 0, "hole where it was");
        let _ = app.update(Message::Undo);
        assert_eq!(px(&app, 10, 10), [0, 0, 0, 255]);
    }

    #[test]
    fn free_transform_commits_on_enter_and_undo_cancels() {
        let (mut app, _) = Photo::new();
        white(&mut app, 60, 60);
        let _ = app.update(Message::FreeTransform);
        assert!(app.transform.is_some());
        let _ = app.update(Message::TransformGrab(Handle::Corner(2), Point::new(60.0, 60.0)));
        let _ = app.update(Message::TransformDrag { at: Point::new(30.0, 30.0), shift: false, alt: false });
        let _ = app.update(Message::TransformRelease);
        let _ = app.update(Message::CommitTransform);
        assert!(app.transform.is_none());
        assert_eq!(px(&app, 20, 20), [255, 255, 255, 255]);
        assert_eq!(px(&app, 45, 45)[3], 0, "scaled to half");

        let _ = app.update(Message::FreeTransform);
        let _ = app.update(Message::Undo);
        assert!(app.transform.is_none());
        assert_eq!(px(&app, 45, 45), [255, 255, 255, 255], "undo reverted the committed transform");
    }

    #[test]
    fn other_edits_commit_an_open_transform() {
        let (mut app, _) = Photo::new();
        white(&mut app, 40, 40);
        let _ = app.update(Message::FreeTransform);
        let _ = app.update(Message::TransformGrab(Handle::Inside, Point::new(20.0, 20.0)));
        let _ = app.update(Message::TransformDrag { at: Point::new(30.0, 20.0), shift: false, alt: false });
        let _ = app.update(Message::AddLayer);
        assert!(app.transform.is_none());
        assert_eq!(px(&app, 5, 5)[3], 0, "moved right by 10 before the new layer");
        assert_eq!(app.document().unwrap().layers().len(), 2);
    }

    #[test]
    fn new_document_dialog_creates_sized_documents() {
        let (mut app, _) = Photo::new();
        let _ = app.update(Message::NewDocDialog);
        let _ = app.update(Message::NewDocPreset(PRESETS.iter().position(|p| p.1 == "A4").unwrap()));
        let _ = app.update(Message::NewDocBackground(NewBackground::Transparent));
        let _ = app.update(Message::CreateNewDoc);
        let doc = app.document().unwrap();
        assert_eq!((doc.width, doc.height), (2480, 3508));
        assert!(!doc.active().unwrap().locked);
        assert!(!app.doc().unwrap().is_dirty());

        let _ = app.update(Message::NewDocDialog);
        let _ = app.update(Message::NewDocWidth("12ab3".into()));
        assert_eq!(app.new_doc.as_ref().unwrap().width, "123");
        let _ = app.update(Message::NewDocHeight("0".into()));
        let _ = app.update(Message::CreateNewDoc);
        assert!(app.new_doc.is_some(), "invalid size keeps the dialog open");
    }

    fn pen(phase: Phase, at: Point, pressure: f32, eraser: bool) -> Message {
        Message::Tablet(TabletMessage::Pen(tablet::PenEvent { phase, position: at, pressure, eraser }))
    }

    #[test]
    fn pen_does_not_paint_while_transforming() {
        let (mut app, _) = Photo::new();
        white(&mut app, 100, 100);
        let _ = app.update(Message::CanvasBounds(Rectangle::new(Point::ORIGIN, Size::new(400.0, 300.0))));
        let _ = app.update(Message::ZoomTo(Zoom::Fixed(1.0)));
        let (o, _) = app.doc_origin().unwrap();
        let _ = app.update(Message::FreeTransform);
        let depth = app.doc().unwrap().history.depth();
        let at = |x: f32| Point::new(o.x + x, o.y + 50.0);
        for m in [pen(Phase::Near, at(10.0), 0.0, false), pen(Phase::Down, at(10.0), 0.9, false), pen(Phase::Move, at(80.0), 0.9, false), pen(Phase::Up, at(80.0), 0.0, false)] {
            let _ = app.update(m);
        }
        assert!(app.transform.is_some());
        assert_eq!(app.doc().unwrap().history.depth(), depth);
        assert_eq!(px(&app, 40, 50), [255, 255, 255, 255]);
    }

    #[test]
    fn pen_does_not_paint_while_a_dialog_is_open() {
        let (mut app, _) = Photo::new();
        white(&mut app, 100, 100);
        let _ = app.update(Message::CanvasBounds(Rectangle::new(Point::ORIGIN, Size::new(400.0, 300.0))));
        let _ = app.update(Message::ZoomTo(Zoom::Fixed(1.0)));
        let (o, _) = app.doc_origin().unwrap();
        let _ = app.update(Message::OpenOp(OpKind::Levels));
        let depth = app.doc().unwrap().history.depth();
        let at = |x: f32| Point::new(o.x + x, o.y + 50.0);
        for m in [pen(Phase::Near, at(10.0), 0.0, false), pen(Phase::Down, at(10.0), 0.9, false), pen(Phase::Move, at(80.0), 0.9, false), pen(Phase::Up, at(80.0), 0.0, false)] {
            let _ = app.update(m);
        }
        assert!(app.adjusting.is_some());
        assert_eq!(app.doc().unwrap().history.depth(), depth);
        assert_eq!(px(&app, 40, 50), [255, 255, 255, 255]);
    }

    #[test]
    fn pen_eraser_end_switches_to_eraser() {
        let (mut app, _) = Photo::new();
        white(&mut app, 100, 100);
        app.tool = Tool::Lasso;
        let _ = app.update(pen(Phase::Near, Point::new(5.0, 5.0), 0.0, true));
        assert_eq!(app.effective_tool(), Tool::Eraser);
        let _ = app.update(pen(Phase::Out, Point::new(5.0, 5.0), 0.0, true));
        assert_eq!(app.effective_tool(), Tool::Lasso);
    }

    fn key(c: &str, modifiers: keyboard::Modifiers) -> Message {
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

    #[test]
    fn tool_keys_pick_and_cycle_slots() {
        let (mut app, _) = Photo::new();
        let _ = app.update(key("m", keyboard::Modifiers::empty()));
        assert_eq!(app.tool, Tool::Marquee);
        let _ = app.update(key("M", keyboard::Modifiers::SHIFT));
        assert_eq!(app.tool, Tool::EllipticalMarquee);
        let _ = app.update(key("b", keyboard::Modifiers::empty()));
        // The slot remembers its last tool.
        let _ = app.update(key("m", keyboard::Modifiers::empty()));
        assert_eq!(app.tool, Tool::EllipticalMarquee);
        let _ = app.update(key("M", keyboard::Modifiers::SHIFT));
        assert_eq!(app.tool, Tool::Marquee);
    }

    #[test]
    fn menu_items_close_the_popup_and_run() {
        let (mut app, _) = Photo::new();
        white(&mut app, 20, 20);
        let _ = app.update(Message::OpenPopup(PopupKind::Context, Point::new(5.0, 5.0)));
        assert!(app.popup.is_some());
        let _ = app.update(Message::Menu(Box::new(Message::SelectAll)));
        assert!(app.popup.is_none());
        assert!(app.document().unwrap().selection.is_some());

        let _ = app.update(Message::OpenPopup(PopupKind::ToolSlot(Tool::Lasso.slot()), Point::ORIGIN));
        let _ = app.update(Message::Key(keyboard::Event::KeyPressed {
            key: Key::Named(Named::Escape),
            modified_key: Key::Named(Named::Escape),
            physical_key: keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        }));
        assert!(app.popup.is_none());
    }

    #[test]
    fn menu_bar_clicks_toggle_switch_and_hover_switches() {
        let (mut app, _) = Photo::new();
        // Hovering measures each title's left edge.
        let _ = app.update(Message::MenuPointer(1, Point::new(5.0, 10.0)));
        let _ = app.update(Message::TitleMoved(Point::new(85.0, 10.0)));
        assert_eq!(app.menu_left[1], 80.0);
        let _ = app.update(Message::MenuTitle(1));
        assert_eq!(app.popup.map(|p| (p.kind, p.at.x)), Some((PopupKind::MenuBar(1), 80.0)));
        // Hover another title while open: switches.
        let _ = app.update(Message::MenuPointer(2, Point::new(3.0, 10.0)));
        let _ = app.update(Message::TitleMoved(Point::new(130.0, 10.0)));
        assert_eq!(app.popup.map(|p| (p.kind, p.at.x)), Some((PopupKind::MenuBar(2), 127.0)));
        // Clicking another title switches; clicking the open one closes.
        let _ = app.update(Message::MenuTitle(0));
        assert_eq!(app.popup.map(|p| p.kind), Some(PopupKind::MenuBar(0)));
        let _ = app.update(Message::MenuTitle(0));
        assert!(app.popup.is_none());
        // Hover alone doesn't open anything.
        let _ = app.update(Message::MenuPointer(3, Point::new(1.0, 1.0)));
        let _ = app.update(Message::TitleMoved(Point::new(200.0, 1.0)));
        assert!(app.popup.is_none());
    }

    #[test]
    fn adjustment_dialog_previews_then_applies_once() {
        let (mut app, _) = Photo::new();
        white(&mut app, 30, 30);
        let _ = app.update(Message::OpenOp(OpKind::Levels));
        assert!(app.adjusting.is_some());
        let op = Op::Adjust(jd_raster::adjust::Adjustment::Levels { in_black: 0, in_white: 255, gamma: 1.0, out_black: 0, out_white: 100 });
        let _ = app.update(Message::AdjustSet(op));
        assert_eq!(px(&app, 5, 5), [255, 255, 255, 255], "document untouched while previewing");
        let _ = app.update(Message::AdjustOk);
        assert!(app.adjusting.is_none());
        assert_eq!(px(&app, 5, 5), [100, 100, 100, 255]);
        let _ = app.update(Message::Undo);
        assert_eq!(px(&app, 5, 5), [255, 255, 255, 255]);
    }

    #[test]
    fn cancel_and_instant_adjustments() {
        let (mut app, _) = Photo::new();
        white(&mut app, 10, 10);
        let depth = app.doc().unwrap().history.depth();
        let _ = app.update(Message::OpenOp(OpKind::GaussianBlur));
        let _ = app.update(Message::AdjustCancel);
        assert!(app.adjusting.is_none());
        assert_eq!(app.doc().unwrap().history.depth(), depth, "cancel adds no undo step");
        let _ = app.update(Message::OpenOp(OpKind::Invert));
        assert!(app.adjusting.is_none(), "Invert needs no dialog");
        assert_eq!(px(&app, 3, 3), [0, 0, 0, 255]);
        assert_eq!(app.doc().unwrap().history.labels().0.last(), Some(&"Invert"));
    }

    #[test]
    fn dragging_a_layer_reorders_or_copies_into_another_document() {
        let (mut app, _) = Photo::new();
        white(&mut app, 40, 40);
        let _ = app.update(Message::AddLayer);
        let top = app.document().unwrap().active_id().unwrap();
        let bottom = app.document().unwrap().layers()[0].id;
        // Reorder: drag the top layer onto the bottom row (it's unlocked by `white`).
        let _ = app.update(Message::LayerPress(top));
        let _ = app.update(Message::DragOver(DropTarget::Layer(bottom)));
        let _ = app.update(Message::PointerReleased);
        assert_eq!(app.document().unwrap().layers()[0].id, top);
        assert!(app.layer_drag.is_none());

        // Copy into another, smaller document: hover its tab, drop on its canvas.
        app.push_doc(Document::new("small", 10, 10), None);
        let _ = app.update(Message::SelectDoc(0));
        let _ = app.update(Message::LayerPress(bottom));
        let _ = app.update(Message::DragOver(DropTarget::Tab(1)));
        assert_eq!(app.active, 1, "hovering a tab switches to it");
        let _ = app.update(Message::DragOver(DropTarget::Canvas));
        let _ = app.update(Message::PointerReleased);
        let small = app.document().unwrap();
        assert_eq!(small.layers().len(), 2);
        assert_eq!(small.layers()[1].pixels.width(), 10, "re-cut to the target size");
        assert_eq!(small.layers()[1].pixels.pixel(5, 5), [255, 255, 255, 255], "centred");
        // A plain click on a row is not a drag.
        let id = small.layers()[1].id;
        let _ = app.update(Message::LayerPress(id));
        let _ = app.update(Message::PointerReleased);
        assert_eq!(app.document().unwrap().layers().len(), 2);
    }

    #[test]
    fn image_and_canvas_size_dialogs() {
        let (mut app, _) = Photo::new();
        white(&mut app, 40, 20);
        let _ = app.update(Message::ShowDialog(DialogKind::ImageSize));
        let _ = app.update(Message::DialogA("80".into()));
        assert_eq!(app.dialog.as_ref().unwrap().b, "40", "proportions kept");
        let _ = app.update(Message::DialogOk);
        assert_eq!((app.document().unwrap().width, app.document().unwrap().height), (80, 40));
        let _ = app.update(Message::ShowDialog(DialogKind::CanvasSize));
        let _ = app.update(Message::DialogA("100".into()));
        let _ = app.update(Message::DialogAnchor(0, 0));
        let _ = app.update(Message::DialogOk);
        let d = app.document().unwrap();
        assert_eq!((d.width, d.height), (100, 40));
        assert_eq!(d.layers()[0].pixels.pixel(99, 5)[3], 0, "unlocked layer: new area transparent");
        let _ = app.update(Message::Undo);
        assert_eq!(app.document().unwrap().width, 80);
    }

    #[test]
    fn stroke_feather_merge_crop_orient_and_last_filter() {
        let (mut app, _) = Photo::new();
        white(&mut app, 60, 60);
        app.fg = Hsv { h: 0.0, s: 0.0, v: 0.0 };
        let _ = app.update(rect(20.0, 20.0, 40.0, 40.0));
        let _ = app.update(Message::ShowDialog(DialogKind::Stroke));
        let _ = app.update(Message::DialogA("4".into()));
        let _ = app.update(Message::DialogOk);
        assert_eq!(px(&app, 20, 30), [0, 0, 0, 255]);
        assert_eq!(px(&app, 30, 30), [255, 255, 255, 255]);
        let _ = app.update(Message::ShowDialog(DialogKind::Feather));
        let _ = app.update(Message::DialogOk);
        let edge = app.document().unwrap().selection.as_ref().unwrap().get(20, 30);
        assert!(edge > 0 && edge < 255);
        let _ = app.update(Message::CropToSelection);
        assert!(app.document().unwrap().width > 20 && app.document().unwrap().width < 60);

        let _ = app.update(Message::AddLayer);
        let _ = app.update(Message::MergeDown);
        assert_eq!(app.document().unwrap().layers().len(), 1);
        let w = app.document().unwrap().width;
        let _ = app.update(Message::RotateCanvas(1));
        let _ = app.update(Message::OrientLayer(Orient::FlipH));
        assert_eq!(app.document().unwrap().height, w);

        assert!(app.last_filter.is_none());
        let _ = app.update(Message::OpenOp(OpKind::GaussianBlur));
        let _ = app.update(Message::AdjustOk);
        let depth = app.doc().unwrap().history.depth();
        let _ = app.update(Message::LastFilter);
        assert_eq!(app.doc().unwrap().history.depth(), depth + 1);
    }

    #[test]
    fn layer_menu_commands_and_workspaces() {
        let (mut app, _) = Photo::new();
        white(&mut app, 30, 30);
        let _ = app.update(Message::AddLayer);
        let id = app.document().unwrap().active_id().unwrap();
        app.docs[0].doc.layer_mut(id).unwrap().pixels.set_pixel(3, 3, [1, 1, 1, 255]);
        let _ = app.update(Message::OpenLayerMenu(id));
        assert_eq!(app.popup.map(|p| p.kind), Some(PopupKind::Layer(id)));
        let _ = app.update(Message::Menu(Box::new(Message::SelectPixels)));
        assert_eq!(app.document().unwrap().selection.as_ref().unwrap().bounds(), Some([3, 3, 4, 4]));

        app.push_doc(Document::new("other", 10, 10), None);
        let _ = app.update(Message::SelectDoc(0));
        let _ = app.update(Message::DuplicateInto(1));
        assert_eq!(app.active, 1);
        assert_eq!(app.document().unwrap().layers().len(), 2);

        let _ = app.update(Message::SetWorkspace(Workspace::Minimal));
        assert!(!app.panels.color && !app.panels.layers);
        let _ = app.update(Message::SetWorkspace(Workspace::Essentials));
        assert!(app.panels.color && app.panels.layers && app.panels.properties);
    }

    #[test]
    fn command_search_runs_menu_commands() {
        let (mut app, _) = Photo::new();
        white(&mut app, 20, 20);
        let _ = app.update(Message::OpenPalette);
        let _ = app.update(Message::PaletteInput("select all".into()));
        assert!(app.palette.is_some());
        // The palette runs entries through Message::Menu, which closes it.
        let _ = app.update(Message::Menu(Box::new(Message::SelectAll)));
        assert!(app.palette.is_none());
        assert!(app.document().unwrap().selection.is_some());
    }

    #[test]
    fn rename_commits_as_one_undo_step() {
        let mut app = app_with_doc(10, 10);
        let _ = app.update(Message::AddLayer);
        let id = app.document().unwrap().active_id().unwrap();
        let _ = app.update(Message::StartRename(id));
        let _ = app.update(Message::RenameInput("Sky".into()));
        let _ = app.update(Message::CommitRename);
        assert_eq!(app.document().unwrap().active().unwrap().name, "Sky");
        let _ = app.update(Message::Undo);
        assert_eq!(app.document().unwrap().active().unwrap().name, "Layer 1");
    }
}
