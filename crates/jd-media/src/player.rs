//! A background playback thread for one file: play at any speed (negative plays backwards),
//! scrub, and step frames. The host sends commands and picks up decoded frames; it should
//! poll (e.g. every display frame) while [`Player::busy`] is true.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::decode::{Frame, VideoDecoder};
use crate::probe::Rate;

#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub path: Option<PathBuf>,
    /// Time of the frame on screen.
    pub position: f64,
    pub duration: f64,
    pub rate: Rate,
    pub playing: bool,
    /// Playback speed (1.0 normal, 2.0 double, -1.0 reverse).
    pub speed: f64,
    pub error: Option<String>,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            path: None,
            position: 0.0,
            duration: 0.0,
            rate: Rate { num: 25, den: 1 },
            playing: false,
            speed: 1.0,
            error: None,
        }
    }
}

enum Command {
    Load(PathBuf),
    Seek(f64),
    Play(f64),
    Pause,
    Step(i64),
    Quit,
}

#[derive(Default)]
struct Shared {
    frame: Mutex<Option<Frame>>,
    status: Mutex<Status>,
    /// Commands sent, and commands fully handled (frame published).
    sent: AtomicU64,
    done: AtomicU64,
}

pub struct Player {
    tx: Sender<Command>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Player {
    /// Frames are scaled to fit `max_size`.
    pub fn new(max_size: (u32, u32)) -> Self {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared::default());
        let for_worker = shared.clone();
        // FFmpeg contexts aren't Send, so the worker is built on its own thread.
        let thread = std::thread::Builder::new()
            .name("jd-media player".into())
            .spawn(move || {
                Worker { rx, shared: for_worker, max_size, decoder: None, status: Status::default(), clock: None }.run()
            })
            .expect("spawn player thread");
        Self { tx, shared, thread: Some(thread) }
    }

    fn send(&self, c: Command) {
        self.shared.sent.fetch_add(1, Ordering::SeqCst);
        let _ = self.tx.send(c);
    }

    pub fn load(&self, path: PathBuf) {
        self.send(Command::Load(path));
    }

    pub fn seek(&self, time: f64) {
        self.send(Command::Seek(time));
    }

    pub fn play(&self, speed: f64) {
        self.send(Command::Play(speed));
    }

    pub fn pause(&self) {
        self.send(Command::Pause);
    }

    /// Moves `frames` frames (negative: back) and pauses.
    pub fn step(&self, frames: i64) {
        self.send(Command::Step(frames));
    }

    /// The newest decoded frame, if one arrived since the last call.
    pub fn take_frame(&self) -> Option<Frame> {
        self.shared.frame.lock().expect("player frame lock").take()
    }

    pub fn status(&self) -> Status {
        self.shared.status.lock().expect("player status lock").clone()
    }

    /// Playing, or still working on a command: keep polling.
    pub fn busy(&self) -> bool {
        self.shared.sent.load(Ordering::SeqCst) != self.shared.done.load(Ordering::SeqCst) || self.status().playing
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Worker {
    rx: Receiver<Command>,
    shared: Arc<Shared>,
    max_size: (u32, u32),
    decoder: Option<VideoDecoder>,
    status: Status,
    /// Playback started at this instant from this position.
    clock: Option<(Instant, f64)>,
}

impl Worker {
    fn run(mut self) {
        let mut wait = None;
        loop {
            let first = match wait {
                Some(d) => match self.rx.recv_timeout(d) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                },
                None => match self.rx.recv() {
                    Ok(c) => Some(c),
                    Err(_) => return,
                },
            };
            let mut batch: Vec<Command> = first.into_iter().collect();
            batch.extend(self.rx.try_iter());
            let count = batch.len() as u64;

            // Seeks are coalesced: scrubbing sends many, only the last one is decoded.
            let mut seek_to = None;
            for c in batch {
                match c {
                    Command::Quit => return,
                    Command::Load(path) => {
                        self.load(path);
                        seek_to = Some(0.0);
                    }
                    Command::Seek(t) => seek_to = Some(t),
                    Command::Play(speed) => {
                        self.status.playing = self.decoder.is_some();
                        self.status.speed = speed;
                        // Play from the end restarts, like a monitor.
                        if speed > 0.0 && self.status.position >= self.last_frame_time() {
                            seek_to = Some(0.0);
                        }
                    }
                    Command::Pause => self.status.playing = false,
                    Command::Step(n) => {
                        self.status.playing = false;
                        let from = seek_to.unwrap_or(self.status.position);
                        seek_to = Some(from + n as f64 / self.status.rate.as_f64());
                    }
                }
            }
            if let Some(t) = seek_to {
                self.seek(t);
            }
            if self.status.playing && (count > 0 || self.clock.is_none()) {
                self.clock = Some((Instant::now(), self.status.position));
            }
            wait = if self.status.playing { self.advance() } else { None };
            if !self.status.playing {
                self.clock = None;
            }
            *self.shared.status.lock().expect("player status lock") = self.status.clone();
            self.shared.done.fetch_add(count, Ordering::SeqCst);
        }
    }

    fn frame_duration(&self) -> f64 {
        1.0 / self.status.rate.as_f64().max(1.0)
    }

    fn last_frame_time(&self) -> f64 {
        (self.status.duration - self.frame_duration()).max(0.0)
    }

    fn load(&mut self, path: PathBuf) {
        self.status = Status { path: Some(path.clone()), ..Status::default() };
        match VideoDecoder::open(&path, self.max_size) {
            Ok(d) => {
                self.status.duration = d.duration();
                self.status.rate = d.rate();
                self.decoder = Some(d);
            }
            Err(e) => {
                self.decoder = None;
                self.status.error = Some(e.to_string());
            }
        }
    }

    fn publish(&mut self) {
        let Some(d) = self.decoder.as_mut() else { return };
        match d.frame() {
            Ok(Some(f)) => {
                self.status.position = f.time;
                *self.shared.frame.lock().expect("player frame lock") = Some(f);
            }
            Ok(None) => {}
            Err(e) => self.fail(e.to_string()),
        }
    }

    fn fail(&mut self, e: String) {
        self.status.error = Some(e);
        self.status.playing = false;
    }

    fn seek(&mut self, t: f64) {
        let t = t.clamp(0.0, self.last_frame_time());
        let Some(d) = self.decoder.as_mut() else { return };
        match d.seek(t) {
            Ok(Some(_)) => self.publish(),
            Ok(None) => {}
            Err(e) => self.fail(e.to_string()),
        }
    }

    /// Shows whatever frame is due now. Returns how long to sleep before the next one.
    fn advance(&mut self) -> Option<Duration> {
        let (start, from) = self.clock?;
        let speed = self.status.speed;
        let target = from + start.elapsed().as_secs_f64() * speed;
        let fd = self.frame_duration();

        if speed < 0.0 {
            // Backwards: seek frame by frame (each seek decodes from a keyframe).
            if target <= 0.0 {
                self.seek(0.0);
                self.status.playing = false;
                return None;
            }
            if self.status.position - target >= fd * 0.5 {
                self.seek(target);
            }
            return Some(Duration::from_secs_f64(fd / speed.abs()));
        }

        if target >= self.last_frame_time() + fd {
            self.seek(self.last_frame_time());
            self.status.playing = false;
            return None;
        }
        if target > self.status.position + 2.0 {
            // Far behind (slow decoder or a big speed): jump instead of decoding everything.
            self.seek(target);
        } else if target >= self.status.position + fd {
            // Decode up to the frame due now, converting only that one.
            let Some(d) = self.decoder.as_mut() else { return None };
            loop {
                match d.next_time() {
                    Ok(Some(t)) if t + fd <= target => continue,
                    Ok(Some(_)) => {
                        self.publish();
                        break;
                    }
                    Ok(None) => {
                        self.status.playing = false;
                        return None;
                    }
                    Err(e) => {
                        self.fail(e.to_string());
                        return None;
                    }
                }
            }
        }
        let next_due = self.status.position + fd;
        Some(Duration::from_secs_f64(((next_due - target) / speed).clamp(0.001, 0.25)))
    }
}
