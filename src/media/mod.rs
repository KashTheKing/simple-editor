//! Media decoding: a small trait layer over two backends.
//!  * `mf`     — Windows Media Foundation (native, hardware accelerated, instant seeks). Primary.
//!  * `ffpipe` — ffmpeg.exe / ffprobe.exe child processes. Universal fallback, images, probing, export.
//! Everything produces top-down RGBA8 frames and interleaved stereo f32 audio at SAMPLE_RATE.

pub mod ffpipe;
pub mod mf;
pub mod waveform;

use crate::model::Asset;
use std::collections::HashMap;

pub const SAMPLE_RATE: u32 = 48000;
pub const CHANNELS: usize = 2;

/// Top-down RGBA8 image. `rgba.len() == width*height*4`.
#[derive(Clone, Default)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Source/timeline time this frame represents (informational).
    pub pts: f64,
    pub rgba: Vec<u8>,
}

impl Frame {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, pts: 0.0, rgba: vec![0; (width * height * 4) as usize] }
    }
    /// Resize (contents undefined/zero when the size changes).
    pub fn resize(&mut self, width: u32, height: u32) {
        if self.width != width || self.height != height {
            self.width = width;
            self.height = height;
            self.rgba.clear();
            self.rgba.resize((width * height * 4) as usize, 0);
        }
    }
    pub fn fill(&mut self, rgba: [u8; 4]) {
        for px in self.rgba.chunks_exact_mut(4) {
            px.copy_from_slice(&rgba);
        }
    }
    pub fn clear(&mut self) {
        self.rgba.fill(0);
    }
    pub fn stride(&self) -> usize {
        self.width as usize * 4
    }
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

pub trait VideoSource: Send {
    /// Native (coded) size.
    fn size(&self) -> (u32, u32);
    fn duration(&self) -> f64;
    fn fps(&self) -> f64;
    /// Decode the frame displayed at source time `t`, scaled (aspect-ignorant, caller picks w/h) into `out`.
    /// Returns false at/after EOF or on error (then `out` is untouched). Must be cheap for sequential
    /// increasing `t` (playback: keep decoding forward); may seek for other jumps.
    fn frame_at(&mut self, t: f64, w: u32, h: u32, out: &mut Frame) -> bool;
}

pub trait AudioSource: Send {
    fn duration(&self) -> f64;
    /// Fill `out` (interleaved stereo f32 @ SAMPLE_RATE, frames = out.len()/2) starting at source time `t`.
    /// Zero-fill past the end. Must be cheap for sequential calls (t advancing by exactly the previous block).
    fn read_at(&mut self, t: f64, out: &mut [f32]);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backend {
    Auto,
    Mf,
    Ffmpeg,
}

impl Backend {
    pub fn parse(s: &str) -> Self {
        match s {
            "mf" => Backend::Mf,
            "ffmpeg" => Backend::Ffmpeg,
            _ => Backend::Auto,
        }
    }
}

pub fn is_image_path(path: &str) -> bool {
    let ext = std::path::Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp" | "tif" | "tiff" | "tga" | "psd")
}

/// Probe a media file into an Asset (id = 0). ffprobe gives the richest stream metadata, so it is
/// preferred when present; Media Foundation otherwise.
pub fn probe(path: &str, backend: Backend) -> Result<Asset, String> {
    match backend {
        Backend::Ffmpeg => ffpipe::probe(path),
        Backend::Mf => mf::probe(path),
        Backend::Auto => {
            if ffpipe::ffprobe_exe().is_some() {
                ffpipe::probe(path).or_else(|e| mf::probe(path).map_err(|e2| format!("{e}; {e2}")))
            } else {
                mf::probe(path).or_else(|e| ffpipe::probe(path).map_err(|e2| format!("{e}; {e2}")))
            }
        }
    }
}

pub fn open_video(path: &str, backend: Backend) -> Result<Box<dyn VideoSource>, String> {
    if is_image_path(path) {
        return ffpipe::open_video(path);
    }
    match backend {
        Backend::Ffmpeg => ffpipe::open_video(path),
        Backend::Mf => mf::open_video(path),
        Backend::Auto => mf::open_video(path).or_else(|e| ffpipe::open_video(path).map_err(|e2| format!("{e}; {e2}"))),
    }
}

pub fn open_audio(path: &str, stream: usize, backend: Backend) -> Result<Box<dyn AudioSource>, String> {
    match backend {
        Backend::Ffmpeg => ffpipe::open_audio(path, stream),
        Backend::Mf => mf::open_audio(path, stream),
        Backend::Auto => mf::open_audio(path, stream)
            .or_else(|e| ffpipe::open_audio(path, stream).map_err(|e2| format!("{e}; {e2}"))),
    }
}

/// Lazily opened decoders, one per (path) / (path, audio stream). Failed opens are remembered
/// (None) so a missing file doesn't re-spawn work every frame.
pub struct DecoderPool {
    backend: Backend,
    videos: HashMap<String, Option<Box<dyn VideoSource>>>,
    audios: HashMap<(String, usize), Option<Box<dyn AudioSource>>>,
}

impl DecoderPool {
    pub fn new(backend: Backend) -> Self {
        Self { backend, videos: HashMap::new(), audios: HashMap::new() }
    }
    pub fn backend(&self) -> Backend {
        self.backend
    }
    pub fn set_backend(&mut self, b: Backend) {
        if b != self.backend {
            self.backend = b;
            self.clear();
        }
    }
    pub fn video(&mut self, path: &str) -> Option<&mut (dyn VideoSource + 'static)> {
        let b = self.backend;
        self.videos
            .entry(path.to_string())
            .or_insert_with(|| open_video(path, b).ok())
            .as_deref_mut()
    }
    pub fn audio(&mut self, path: &str, stream: usize) -> Option<&mut (dyn AudioSource + 'static)> {
        let b = self.backend;
        self.audios
            .entry((path.to_string(), stream))
            .or_insert_with(|| open_audio(path, stream, b).ok())
            .as_deref_mut()
    }
    /// Drop every decoder (releases file handles — required before overwriting a source file).
    pub fn clear(&mut self) {
        self.videos.clear();
        self.audios.clear();
    }
    /// Forget a failed open so it is retried next time.
    pub fn retry(&mut self, path: &str) {
        self.videos.retain(|k, v| !(k == path && v.is_none()));
        self.audios.retain(|k, v| !(k.0 == path && v.is_none()));
    }
}
