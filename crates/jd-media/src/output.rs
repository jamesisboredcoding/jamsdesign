//! Audio output through the default device (`cpal`: PipeWire/PulseAudio/ALSA on Linux).
//! The player pushes interleaved stereo; frames actually played are the playback clock.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::{Error, Result};

#[derive(Default)]
struct Shared {
    queue: Mutex<VecDeque<f32>>,
    /// Sample frames taken from the queue by the device (silence doesn't count).
    played: AtomicU64,
    /// Peak level per channel since the last `take_peaks` (f32 bits).
    peaks: [AtomicU32; 2],
}

/// An open output stream. Must be created and dropped on the same thread.
pub struct AudioOutput {
    _stream: cpal::Stream,
    rate: u32,
    shared: Arc<Shared>,
}

impl AudioOutput {
    /// Opens the default output device as f32 at 48 kHz if it can, else its default rate.
    pub fn open() -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or_else(|| Error::Audio("no audio output device".into()))?;
        let default = device.default_output_config().map_err(|e| Error::Audio(e.to_string()))?;
        let supports_48k = device
            .supported_output_configs()
            .map(|mut cs| cs.any(|c| c.sample_format() == cpal::SampleFormat::F32 && c.min_sample_rate() <= 48_000 && c.max_sample_rate() >= 48_000))
            .unwrap_or(false);
        let mut config = default.config();
        if supports_48k {
            config.sample_rate = 48_000;
        }
        let channels = config.channels.max(1) as usize;
        let rate = config.sample_rate;
        let shared = Arc::new(Shared::default());
        let cb = shared.clone();
        let stream = device
            .build_output_stream::<f32, _, _>(
                config,
                move |data: &mut [f32], _| fill(&cb, data, channels),
                |e| eprintln!("audio output error: {e}"),
                None,
            )
            .map_err(|e| Error::Audio(e.to_string()))?;
        stream.play().map_err(|e| Error::Audio(e.to_string()))?;
        Ok(Self { _stream: stream, rate, shared })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Queues interleaved stereo samples.
    pub fn push(&self, samples: &[f32]) {
        self.shared.queue.lock().expect("audio queue").extend(samples.iter().copied());
    }

    /// Sample frames waiting to be played.
    pub fn queued(&self) -> usize {
        self.shared.queue.lock().expect("audio queue").len() / 2
    }

    pub fn clear(&self) {
        self.shared.queue.lock().expect("audio queue").clear();
    }

    /// Sample frames played since the stream opened.
    pub fn played(&self) -> u64 {
        self.shared.played.load(Ordering::Acquire)
    }

    /// Left and right peak levels (0..=1) since the last call.
    pub fn take_peaks(&self) -> [f32; 2] {
        [0, 1].map(|i| f32::from_bits(self.shared.peaks[i].swap(0, Ordering::AcqRel)))
    }
}

fn fill(shared: &Shared, data: &mut [f32], channels: usize) {
    let mut queue = shared.queue.lock().expect("audio queue");
    let mut played = 0u64;
    let mut peak = [0.0f32; 2];
    for frame in data.chunks_mut(channels) {
        let (l, r) = if queue.len() >= 2 {
            played += 1;
            (queue.pop_front().unwrap_or(0.0), queue.pop_front().unwrap_or(0.0))
        } else {
            (0.0, 0.0)
        };
        peak[0] = peak[0].max(l.abs());
        peak[1] = peak[1].max(r.abs());
        match frame.len() {
            1 => frame[0] = (l + r) * 0.5,
            _ => {
                frame[0] = l;
                frame[1] = r;
                for extra in &mut frame[2..] {
                    *extra = 0.0;
                }
            }
        }
    }
    drop(queue);
    shared.played.fetch_add(played, Ordering::AcqRel);
    for (i, p) in peak.into_iter().enumerate() {
        // Non-negative floats order the same as their bit patterns.
        shared.peaks[i].fetch_max(p.to_bits(), Ordering::AcqRel);
    }
}
