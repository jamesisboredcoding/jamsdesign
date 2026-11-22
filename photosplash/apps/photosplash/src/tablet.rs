//! Pen tablet input (pressure, eraser end) on Wayland, via octotablet.
//!
//! iced doesn't deliver pen data, so we attach octotablet to Photosplash's own window: iced lends
//! us the native handles (`window::run`), and a background thread pumps tablet events. It
//! wakes every 3 ms to poll and only sends messages when the pen does something.
//!
//! Once an app reads the tablet, the compositor stops sending the pen as mouse input. So the
//! thread replays every pen move/press as a synthetic pointer event (through our patched
//! iced_winit, see vendor/iced_winit/src/synthetic.rs): buttons, sliders and all tools work
//! with the pen. Pressure and the eraser end arrive separately as `TabletMessage::Pen`.
//!
//! `PHOTOSPLASH_PEN=0` skips all of this; the pen is then a plain mouse from the compositor.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use iced::futures::channel::mpsc;
use iced::{Point, Subscription, Task, window};
use iced_winit::synthetic::{self, Pointer};
use octotablet::events::{Event, ToolEvent};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle, WindowHandle,
};

/// Set while the pen is in proximity, *before* its synthetic pointer events are queued. The
/// canvas checks it so the pen's mouse twin never starts a second, pressure-less stroke (the
/// pen paints through `Message::Tablet`, which carries every pressure sample).
static PEN_NEAR: AtomicBool = AtomicBool::new(false);

pub fn pen_near() -> bool {
    PEN_NEAR.load(Ordering::Acquire)
}

/// `PHOTOSPLASH_PEN=0` (or `off`/`false`) disables tablet input for this run.
pub fn enabled_by_env() -> bool {
    !matches!(
        std::env::var("PHOTOSPLASH_PEN").map(|v| v.to_ascii_lowercase()).as_deref(),
        Ok("0" | "off" | "false" | "no")
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    /// Pen came near the window (hovering).
    Near,
    Down,
    Move,
    Up,
    /// Pen left proximity.
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PenEvent {
    pub phase: Phase,
    /// Window coordinates, logical pixels.
    pub position: Point,
    pub pressure: f32,
    /// The pen's eraser end is in use.
    pub eraser: bool,
}

#[derive(Debug, Clone)]
pub enum TabletMessage {
    Pen(PenEvent),
    Unavailable(String),
}

/// Native window handles, copied out of iced. Raw pointers aren't `Send`, but the window and
/// display live as long as the app, and octotablet only uses them to reach the same Wayland
/// connection from its own event queue.
#[derive(Clone, Copy)]
pub struct Handles {
    display: RawDisplayHandle,
    window: RawWindowHandle,
}

// SAFETY: see the type docs. The pointers stay valid for the life of the window.
unsafe impl Send for Handles {}

impl HasDisplayHandle for Handles {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        // SAFETY: valid for the window's lifetime (see type docs).
        Ok(unsafe { DisplayHandle::borrow_raw(self.display) })
    }
}

impl HasWindowHandle for Handles {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: valid for the window's lifetime (see type docs).
        Ok(unsafe { WindowHandle::borrow_raw(self.window) })
    }
}

impl std::fmt::Debug for Handles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Handles")
    }
}

/// Asks iced for the window's native handles.
pub fn handles(id: window::Id) -> Task<Option<Handles>> {
    window::run(id, |w| {
        Some(Handles {
            display: w.display_handle().ok()?.as_raw(),
            window: w.window_handle().ok()?.as_raw(),
        })
    })
}

/// Handles waiting to be picked up by the subscription (a fn-pointer stream can't capture).
static PENDING: Mutex<Option<Handles>> = Mutex::new(None);

pub fn set_handles(h: Handles) {
    *PENDING.lock().expect("tablet handles lock") = Some(h);
}

pub fn subscription() -> Subscription<TabletMessage> {
    Subscription::run(stream)
}

fn stream() -> impl iced::futures::Stream<Item = TabletMessage> {
    iced::stream::channel(256, async |output: mpsc::Sender<TabletMessage>| {
        let handles = *PENDING.lock().expect("tablet handles lock");
        let Some(handles) = handles else { return };
        std::thread::Builder::new()
            .name("photosplash-tablet".into())
            .spawn(move || run(handles, output))
            .expect("spawn tablet thread");
        // The thread does the work; keep the stream (and its channel) alive.
        std::future::pending::<()>().await;
    })
}

fn run(handles: Handles, mut out: mpsc::Sender<TabletMessage>) {
    // SAFETY: `handles` outlive this thread (they live as long as the app window).
    let manager = unsafe { octotablet::Builder::default().build_raw(handles) };
    let mut manager = match manager {
        Ok(m) => m,
        Err(e) => {
            let _ = out.try_send(TabletMessage::Unavailable(e.to_string()));
            return;
        }
    };
    let mut down = false;
    // "Down" is reported before the pose that carries its pressure: hold it until then.
    let mut pending_down = false;
    let mut eraser = false;
    let mut last = PenEvent { phase: Phase::Out, position: Point::ORIGIN, pressure: 0.0, eraser: false };

    while !out.is_closed() {
        let events = match manager.pump() {
            Ok(e) => e,
            Err(e) => {
                let _ = out.try_send(TabletMessage::Unavailable(e.to_string()));
                return;
            }
        };
        let mut batch = Vec::new();
        for event in events {
            let Event::Tool { tool, event } = event else { continue };
            let phase = match event {
                ToolEvent::In { .. } => {
                    eraser = tool.tool_type == Some(octotablet::tool::Type::Eraser);
                    Phase::Near
                }
                ToolEvent::Down => {
                    down = true;
                    pending_down = true;
                    continue;
                }
                ToolEvent::Up => {
                    if std::mem::take(&mut pending_down) {
                        // A tap with no movement: still a dab.
                        last.pressure = last.pressure.max(0.5);
                        last.phase = Phase::Down;
                        batch.push(last);
                    }
                    down = false;
                    Phase::Up
                }
                ToolEvent::Out => {
                    down = false;
                    Phase::Out
                }
                ToolEvent::Pose(pose) => {
                    last.position = Point::new(pose.position[0], pose.position[1]);
                    last.pressure = if down { pose.pressure.get().unwrap_or(1.0) } else { 0.0 };
                    if std::mem::take(&mut pending_down) { Phase::Down } else { Phase::Move }
                }
                _ => continue,
            };
            last.phase = phase;
            last.eraser = eraser;
            batch.push(last);
        }
        // Hovering only moves the brush outline: keep the newest hover position per batch.
        // While drawing, every pose matters for a smooth stroke.
        let mut filtered: Vec<PenEvent> = Vec::with_capacity(batch.len());
        for e in batch {
            if e.phase == Phase::Move
                && e.pressure <= 0.0
                && let Some(prev) = filtered.last_mut()
                && prev.phase == Phase::Move
                && prev.pressure <= 0.0
            {
                *prev = e;
                continue;
            }
            filtered.push(e);
        }
        // The compositor no longer sends the pen as a mouse once we read the tablet, so we
        // replay it as pointer input: buttons, sliders and every tool work with the pen.
        let mut pointer = Vec::new();
        for e in &filtered {
            let at = Pointer::Moved { x: e.position.x, y: e.position.y };
            match e.phase {
                Phase::Near => {
                    PEN_NEAR.store(true, Ordering::Release);
                    pointer.push(at);
                }
                Phase::Move => pointer.push(at),
                Phase::Down => pointer.extend([at, Pointer::Pressed]),
                Phase::Up => pointer.extend([at, Pointer::Released]),
                Phase::Out => pointer.push(Pointer::Left),
            }
        }
        if !pointer.is_empty() {
            synthetic::push(pointer);
        }
        for e in filtered {
            if out.try_send(TabletMessage::Pen(e)).is_err() && out.is_closed() {
                return;
            }
        }
        // Cleared after the Out event is queued, so the matching release is still ignored.
        if last.phase == Phase::Out {
            PEN_NEAR.store(false, Ordering::Release);
        }
        std::thread::sleep(Duration::from_millis(3));
    }
}
