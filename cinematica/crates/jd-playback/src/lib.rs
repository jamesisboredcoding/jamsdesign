//! Sequence playback. A background thread mixes every audible audio clip into the output
//! device and shows the topmost video clip's frame for the time the device has actually
//! played, so picture follows sound. Without an audio device (or at speeds other than 1x,
//! which play silently) the wall clock drives it instead.
//!
//! Never depends on iced.

mod mixer;
mod video;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use jd_media::{AudioOutput, Frame};
use jd_timeline::{MediaId, Sequence, Time};

pub use mixer::Mixer;

/// Audio queued ahead of the device: enough to ride out a clip opening, short enough that
/// stopping feels immediate.
const AHEAD_SECONDS: f64 = 0.2;
const MIX_CHUNK: usize = 1024;

/// What to play.
#[derive(Debug, Clone)]
pub struct Request {
    pub sequence: Sequence,
    /// Where each clip's media lives.
    pub media: HashMap<MediaId, PathBuf>,
    pub from: Time,
    /// 1.0 plays with sound; anything else plays silently (negative is backwards).
    pub speed: f64,
    /// Frames are scaled to fit this.
    pub max_size: (u32, u32),
}

enum Command {
    /// A request and its generation (see `Shared::generation`).
    Play(Box<Request>, u64),
    Stop,
    Quit,
}

#[derive(Default)]
struct Shared {
    /// `Some(None)`: show black (no clip). `None`: nothing new since the last take.
    frame: Mutex<Option<Option<Frame>>>,
    position: AtomicI64,
    playing: AtomicBool,
    /// Bumped by every play/stop, so a stale "finished" from an older request can't
    /// clear `playing` for a newer one.
    generation: AtomicU64,
    peaks: Mutex<[f32; 2]>,
    error: Mutex<Option<String>>,
}

pub struct SequencePlayer {
    tx: Sender<Command>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Default for SequencePlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl SequencePlayer {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared::default());
        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("jd-playback".into())
            .spawn(move || run(rx, worker))
            .expect("spawn playback thread");
        Self { tx, shared, thread: Some(thread) }
    }

    pub fn play(&self, request: Request) {
        let generation = self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.shared.position.store(request.from, Ordering::Release);
        self.shared.playing.store(true, Ordering::Release);
        let _ = self.tx.send(Command::Play(Box::new(request), generation));
    }

    pub fn stop(&self) {
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        self.shared.playing.store(false, Ordering::Release);
        let _ = self.tx.send(Command::Stop);
    }

    pub fn playing(&self) -> bool {
        self.shared.playing.load(Ordering::Acquire)
    }

    /// The time on screen (follows the audio actually played).
    pub fn position(&self) -> Time {
        self.shared.position.load(Ordering::Acquire)
    }

    /// The newest frame: `Some(None)` means black.
    pub fn take_frame(&self) -> Option<Option<Frame>> {
        self.shared.frame.lock().expect("frame lock").take()
    }

    /// Left/right peak output levels (0..=1) since the last call.
    pub fn take_peaks(&self) -> [f32; 2] {
        std::mem::take(&mut *self.shared.peaks.lock().expect("peaks lock"))
    }

    pub fn take_error(&self) -> Option<String> {
        self.shared.error.lock().expect("error lock").take()
    }
}

impl Drop for SequencePlayer {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(rx: Receiver<Command>, shared: Arc<Shared>) {
    let mut next: Option<(Box<Request>, u64)> = None;
    loop {
        let (request, generation) = match next.take() {
            Some(r) => r,
            None => match rx.recv() {
                Ok(Command::Play(r, g)) => (r, g),
                Ok(Command::Stop) => continue,
                Ok(Command::Quit) | Err(_) => return,
            },
        };
        match play(&rx, &shared, &request) {
            Outcome::Finished | Outcome::Stopped => {
                if shared.generation.load(Ordering::Acquire) == generation {
                    shared.playing.store(false, Ordering::Release);
                }
            }
            Outcome::Restart(r, g) => next = Some((r, g)),
            Outcome::Quit => return,
        }
    }
}

enum Outcome {
    Finished,
    Stopped,
    Restart(Box<Request>, u64),
    Quit,
}

fn play(rx: &Receiver<Command>, shared: &Shared, req: &Request) -> Outcome {
    let end = req.sequence.duration();
    let out = if req.speed == 1.0 {
        match AudioOutput::open() {
            Ok(o) => Some(o),
            Err(e) => {
                *shared.error.lock().expect("error lock") = Some(format!("{e}; playing without sound"));
                None
            }
        }
    } else {
        None
    };
    let rate = out.as_ref().map_or(48_000, AudioOutput::rate);
    let mut mixer = Mixer::new(&req.sequence, &req.media, rate, req.from);
    let mut video = video::VideoState::new(req.max_size);
    let started = Instant::now();
    let played_at_start = out.as_ref().map_or(0, AudioOutput::played);

    loop {
        match rx.recv_timeout(Duration::from_millis(4)) {
            Ok(Command::Stop) => return Outcome::Stopped,
            Ok(Command::Quit) => return Outcome::Quit,
            Ok(Command::Play(r, g)) => return Outcome::Restart(r, g),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Outcome::Quit,
        }

        if let Some(out) = &out {
            while out.queued() < (rate as f64 * AHEAD_SECONDS) as usize {
                out.push(&mixer.mix(MIX_CHUNK));
            }
            let p = out.take_peaks();
            let mut peaks = shared.peaks.lock().expect("peaks lock");
            *peaks = [peaks[0].max(p[0]), peaks[1].max(p[1])];
        }

        let t = match &out {
            Some(o) => req.from + mixer::frames_to_flicks(o.played() - played_at_start, rate),
            None => req.from + jd_timeline::from_seconds(started.elapsed().as_secs_f64() * req.speed),
        };
        let done = if req.speed >= 0.0 { t >= end } else { t <= 0 };
        let t = t.clamp(0, end.max(0));
        shared.position.store(t, Ordering::Release);

        match video.show(&req.sequence, &req.media, t) {
            Ok(Some(frame)) => *shared.frame.lock().expect("frame lock") = Some(frame),
            Ok(None) => {}
            Err(e) => *shared.error.lock().expect("error lock") = Some(e),
        }
        if done {
            return Outcome::Finished;
        }
    }
}
