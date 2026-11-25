//! On-device background removal (Image > Remove Background).
//!
//! Runs an open salient-object segmentation model (from the rembg project, Apache-2.0) with
//! ONNX Runtime, entirely on this computer. The model is downloaded once, only after the user agrees,
//! into the user's data folder.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use ort::session::Session;
use ort::value::Tensor;

/// A downloadable segmentation model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Model {
    pub name: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    /// Download size in bytes (for the prompt and progress).
    pub size: u64,
    /// Square input size the model expects.
    input: usize,
    mean: [f32; 3],
    std: [f32; 3],
}

/// IS-Net general use: the best general-purpose quality of the rembg models.
pub const ISNET: Model = Model {
    name: "IS-Net (general use)",
    file: "isnet-general-use.onnx",
    url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/isnet-general-use.onnx",
    size: 178_648_008,
    input: 1024,
    mean: [0.5, 0.5, 0.5],
    std: [1.0, 1.0, 1.0],
};

/// Silueta: a small U^2-Net (faster download and run, rougher edges).
pub const SILUETA: Model = Model {
    name: "Silueta (U²-Net, small)",
    file: "silueta.onnx",
    url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/silueta.onnx",
    size: 44_173_029,
    input: 320,
    mean: [0.485, 0.456, 0.406],
    std: [0.229, 0.224, 0.225],
};

/// Where models live: `$XDG_DATA_HOME/jamsdesign/models` (or `~/.local/share/...`).
pub fn models_dir() -> PathBuf {
    let base = jd_core::fs::user_dir("XDG_DATA_HOME", ".local/share").unwrap_or_else(|| PathBuf::from("."));
    base.join("jamsdesign/models")
}

impl Model {
    pub fn path(&self) -> PathBuf {
        models_dir().join(self.file)
    }

    /// Downloaded and complete.
    pub fn is_installed(&self) -> bool {
        std::fs::metadata(self.path()).is_ok_and(|m| m.len() == self.size)
    }

    /// Downloads the model (into a temporary file, renamed when complete). `progress` gets
    /// the bytes received so far.
    pub fn download(&self, mut progress: impl FnMut(u64)) -> Result<(), String> {
        use std::io::{Read, Write};
        let dir = models_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("Can't create {}: {e}", dir.display()))?;
        let part = dir.join(format!("{}.part", self.file));
        let response = ureq::get(self.url).call().map_err(|e| format!("Download failed: {e}"))?;
        let mut body = response.into_body().into_reader();
        let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 1 << 16];
        let mut got = 0u64;
        loop {
            let n = body.read(&mut buf).map_err(|e| format!("Download interrupted: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            got += n as u64;
            progress(got);
        }
        file.sync_all().map_err(|e| e.to_string())?;
        if got != self.size {
            let _ = std::fs::remove_file(&part);
            return Err(format!("Download incomplete ({got} of {} bytes)", self.size));
        }
        std::fs::rename(&part, self.path()).map_err(|e| e.to_string())
    }
}

/// Loaded models (loading takes a moment, so once per model and run).
static SESSIONS: Mutex<Vec<(&'static str, Session)>> = Mutex::new(Vec::new());

fn environment() -> Result<&'static ort::environment::Environment, String> {
    static ENV: OnceLock<Result<ort::environment::Environment, String>> = OnceLock::new();
    ENV.get_or_init(|| ort::environment::init().with_name("jamsdesign").with_telemetry(false).build().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| format!("Can't start ONNX Runtime: {e}"))
}

/// Runs the model on a 1x3xSxS input and returns its first output (SxS).
fn infer(model: &Model, input: Vec<f32>) -> Result<Vec<f32>, String> {
    let mut sessions = SESSIONS.lock().map_err(|_| "model cache poisoned".to_string())?;
    if !sessions.iter().any(|(f, _)| *f == model.file) {
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let fail = |e: &dyn std::fmt::Display| format!("Can't load the model: {e}");
        let builder = Session::builder(environment()?).map_err(|e| fail(&e))?;
        let mut builder = builder.with_intra_threads(threads).map_err(|e| fail(&e))?;
        let session = builder.commit_from_file(model.path()).map_err(|e| fail(&e))?;
        sessions.push((model.file, session));
    }
    let session = &mut sessions.iter_mut().find(|(f, _)| *f == model.file).expect("just loaded").1;
    let s = model.input;
    let tensor = Tensor::from_array(([1usize, 3, s, s], input)).map_err(|e| e.to_string())?;
    let outputs = session.run(ort::inputs![tensor]).map_err(|e| format!("The model failed: {e}"))?;
    let (_, data) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
    Ok(data.iter().copied().take(s * s).collect())
}

/// Bilinear resample of a single-channel f32 image.
fn resample(src: &[f32], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<f32> {
    let mut out = vec![0f32; dw * dh];
    for y in 0..dh {
        let fy = ((y as f32 + 0.5) * sh as f32 / dh as f32 - 0.5).clamp(0.0, (sh - 1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy - fy.floor());
        let y1 = (y0 + 1).min(sh - 1);
        for x in 0..dw {
            let fx = ((x as f32 + 0.5) * sw as f32 / dw as f32 - 0.5).clamp(0.0, (sw - 1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx - fx.floor());
            let x1 = (x0 + 1).min(sw - 1);
            let top = src[y0 * sw + x0] * (1.0 - tx) + src[y0 * sw + x1] * tx;
            let bottom = src[y1 * sw + x0] * (1.0 - tx) + src[y1 * sw + x1] * tx;
            out[y * dw + x] = top * (1.0 - ty) + bottom * ty;
        }
    }
    out
}

/// Area-average downscale (or bilinear upscale) of one channel, for the model's input.
fn fit(src: &[f32], sw: usize, sh: usize, d: usize) -> Vec<f32> {
    if sw <= d && sh <= d {
        return resample(src, sw, sh, d, d);
    }
    // Box-average into the target grid, then it's smooth enough for the network.
    let mut out = vec![0f32; d * d];
    let mut n = vec![0f32; d * d];
    for y in 0..sh {
        let ty = (y * d / sh).min(d - 1);
        for x in 0..sw {
            let tx = (x * d / sw).min(d - 1);
            out[ty * d + tx] += src[y * sw + x];
            n[ty * d + tx] += 1.0;
        }
    }
    for (o, c) in out.iter_mut().zip(&n) {
        *o /= c.max(1.0);
    }
    out
}

/// The foreground mask of an RGBA8 image (0 = background, 255 = subject), same size.
pub fn foreground_mask(model: &Model, rgba: &[u8], w: usize, h: usize) -> Result<Vec<u8>, String> {
    if w == 0 || h == 0 || rgba.len() != w * h * 4 {
        return Err("Empty image".into());
    }
    let s = model.input;
    // Channels scaled like rembg: divide by the image's maximum, then normalise.
    let max = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).max().unwrap_or(1).max(1) as f32;
    let mut input = Vec::with_capacity(3 * s * s);
    for c in 0..3 {
        let chan: Vec<f32> = rgba.chunks_exact(4).map(|p| p[c] as f32 / max).collect();
        input.extend(fit(&chan, w, h, s).into_iter().map(|v| (v - model.mean[c]) / model.std[c]));
    }
    let pred = infer(model, input)?;
    let (lo, hi) = pred.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let span = (hi - lo).max(1e-6);
    let norm: Vec<f32> = pred.iter().map(|v| (v - lo) / span).collect();
    Ok(resample(&norm, s, s, w, h).into_iter().map(|v| (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8).collect())
}
