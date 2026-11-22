//! Throwaway spike: does pen pressure / tilt reach us on this machine?
//!
//! Run with `cargo run -p tablet-spike --release` and draw on the window with your pen.
//! Stroke width follows pressure; the bar along the top shows live pressure, the one along the
//! bottom shows tilt magnitude. Events are also printed to the terminal. Esc / close to quit.
//!
//! Wayland only (zwp_tablet_v2). On X11 octotablet has no backend and the manager fails to start.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use octotablet::events::{Event, ToolEvent};
use octotablet::tool::Type as ToolType;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

const BG: u32 = 0x1E1E1E;
const INK: u32 = 0xF2733F;
const BAR: u32 = 0x4C8DFF;
const MAX_RADIUS: f32 = 14.0;

struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<u32>,
}

impl Canvas {
    fn new(width: usize, height: usize) -> Self {
        Self { width, height, pixels: vec![BG; width * height] }
    }

    fn rect(&mut self, x0: usize, y0: usize, w: usize, h: usize, color: u32) {
        for y in y0..(y0 + h).min(self.height) {
            let row = y * self.width;
            for x in x0..(x0 + w).min(self.width) {
                self.pixels[row + x] = color;
            }
        }
    }

    fn disc(&mut self, cx: f32, cy: f32, r: f32, color: u32) {
        let (x0, x1) = ((cx - r).floor().max(0.0) as usize, (cx + r).ceil() as usize);
        let (y0, y1) = ((cy - r).floor().max(0.0) as usize, (cy + r).ceil() as usize);
        for y in y0..y1.min(self.height) {
            for x in x0..x1.min(self.width) {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                if dx * dx + dy * dy <= r * r {
                    self.pixels[y * self.width + x] = color;
                }
            }
        }
    }

    /// Stamps discs along a segment, interpolating radius.
    fn stroke(&mut self, a: [f32; 2], ra: f32, b: [f32; 2], rb: f32, color: u32) {
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let steps = (len / (ra.min(rb).max(0.5) * 0.5)).ceil().max(1.0) as usize;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            self.disc(p[0], p[1], ra + (rb - ra) * t, color);
        }
    }

    /// Top bar = pressure, bottom bar = tilt (0..1).
    fn meters(&mut self, pressure: f32, tilt: f32) {
        let w = self.width;
        self.rect(0, 0, w, 6, 0x111111);
        self.rect(0, 0, (pressure.clamp(0.0, 1.0) * w as f32) as usize, 6, BAR);
        let y = self.height.saturating_sub(6);
        self.rect(0, y, w, 6, 0x111111);
        self.rect(0, y, (tilt.clamp(0.0, 1.0) * w as f32) as usize, 6, 0x4CC38A);
    }
}

#[derive(Default)]
struct Spike {
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    canvas: Option<Canvas>,
    manager: Option<octotablet::Manager>,
    /// Last point (physical px) and radius while the pen is down.
    last: Option<([f32; 2], f32)>,
    eraser: bool,
    poses: u64,
    last_report: Option<Instant>,
}

impl Spike {
    fn redraw(&mut self) {
        let (Some(surface), Some(canvas)) = (self.surface.as_mut(), self.canvas.as_ref()) else {
            return;
        };
        let (Some(w), Some(h)) = (NonZeroU32::new(canvas.width as u32), NonZeroU32::new(canvas.height as u32)) else {
            return;
        };
        surface.resize(w, h).expect("resize surface");
        let mut buffer = surface.buffer_mut().expect("buffer");
        buffer.copy_from_slice(&canvas.pixels);
        buffer.present().expect("present");
    }
}

impl ApplicationHandler for Spike {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("tablet spike: draw on me with your pen")
            .with_inner_size(winit::dpi::LogicalSize::new(800.0, 600.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));

        // On Wayland a window only appears once a buffer is attached, so paint immediately.
        let context = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        self.surface = Some(softbuffer::Surface::new(&context, window.clone()).expect("surface"));
        let size = window.inner_size();
        self.canvas = Some(Canvas::new(size.width as usize, size.height as usize));

        match octotablet::Builder::default().build_shared(&window) {
            Ok(manager) => {
                println!("✔ tablet manager up");
                self.manager = Some(manager);
            }
            Err(e) => println!("✘ could not start tablet manager: {e}"),
        }
        self.window = Some(window);
        self.redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested
            | WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: Key::Named(NamedKey::Escape),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.canvas = Some(Canvas::new(size.width as usize, size.height as usize));
                self.redraw();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(8)));
        let (Some(manager), Some(canvas), Some(window)) =
            (self.manager.as_mut(), self.canvas.as_mut(), self.window.as_ref())
        else {
            return;
        };
        let scale = window.scale_factor() as f32;
        let events = match manager.pump() {
            Ok(e) => e,
            Err(e) => {
                println!("pump error: {e}");
                return;
            }
        };
        let mut dirty = false;
        for event in events {
            match event {
                Event::Tool { tool, event } => match event {
                    ToolEvent::Down => {
                        self.eraser = tool.tool_type == Some(ToolType::Eraser);
                        // Mark "down" with a sentinel; the next pose starts the stroke.
                        self.last = Some(([f32::NAN, f32::NAN], 0.0));
                    }
                    ToolEvent::Up | ToolEvent::Out => self.last = None,
                    ToolEvent::Pose(pose) => {
                        self.poses += 1;
                        let pressure = pose.pressure.get().unwrap_or(0.0);
                        let tilt = pose.tilt.map_or(0.0, |[x, y]| (x * x + y * y).sqrt() / std::f32::consts::FRAC_PI_2);
                        canvas.meters(pressure, tilt);
                        dirty = true;

                        if let Some((prev, prev_r)) = self.last {
                            let p = [pose.position[0] * scale, pose.position[1] * scale];
                            let r = (pressure * MAX_RADIUS * scale).max(0.5);
                            let color = if self.eraser { BG } else { INK };
                            if prev[0].is_nan() {
                                canvas.disc(p[0], p[1], r, color);
                            } else {
                                canvas.stroke(prev, prev_r, p, r, color);
                            }
                            self.last = Some((p, r));
                        }

                        let now = Instant::now();
                        if self.last_report.is_none_or(|t| now - t > Duration::from_millis(100)) {
                            self.last_report = Some(now);
                            println!(
                                "pose #{:<6} pos=({:7.1},{:7.1}) pressure={:?} tilt={:?} distance={:?}",
                                self.poses, pose.position[0], pose.position[1], pose.pressure.get(), pose.tilt, pose.distance.get()
                            );
                        }
                    }
                    other => println!("tool {:?}: {:?}", tool.tool_type, other),
                },
                other => println!("{other:?}"),
            }
        }
        if dirty {
            window.request_redraw();
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("event loop");
    let mut app = Spike::default();
    event_loop.run_app(&mut app).expect("run");
}
