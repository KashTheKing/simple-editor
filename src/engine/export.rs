//! Export: render the timeline with the Compositor + Mixer and pipe it into ffmpeg.exe
//! (`-f rawvideo -pix_fmt rgba -s WxH -r fps -i pipe:0` + pre-mixed temp WAV), encoder chosen by the
//! output extension / settings. Plus the lossless `-c copy` fast path for pure cuts of one source.
//! Runs on a background thread; the UI polls `Progress`.

use crate::engine::text::TextRasterizer;
use crate::media::Backend;
use crate::model::Project;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct ExportOptions {
    pub out_path: PathBuf,
    /// "auto" or an ffmpeg encoder name (see Settings.encoder).
    pub encoder: String,
    pub crf: u32,
    pub preset: String,
    pub backend: Backend,
}

pub struct Progress {
    fraction: Mutex<f32>,
    status: Mutex<String>,
    error: Mutex<Option<String>>,
    pub cancel: AtomicBool,
    done: AtomicBool,
}

impl Progress {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            fraction: Mutex::new(0.0),
            status: Mutex::new(String::new()),
            error: Mutex::new(None),
            cancel: AtomicBool::new(false),
            done: AtomicBool::new(false),
        })
    }
    pub fn set(&self, fraction: f32, status: impl Into<String>) {
        *self.fraction.lock().unwrap() = fraction;
        *self.status.lock().unwrap() = status.into();
    }
    pub fn fraction(&self) -> f32 {
        *self.fraction.lock().unwrap()
    }
    pub fn status(&self) -> String {
        self.status.lock().unwrap().clone()
    }
    pub fn finish(&self, error: Option<String>) {
        *self.error.lock().unwrap() = error;
        self.done.store(true, Ordering::SeqCst);
    }
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }
}

/// Start a full (re-encoding) export on a background thread. Frames are rendered at project size
/// with `Compositor` and audio mixed with `Mixer` into a temp WAV first (fast), then video frames are
/// piped to ffmpeg's stdin. Progress 0..1; cancel kills ffmpeg and removes the partial file.
/// Audio-only extensions (mp3, wav, m4a, flac, ogg, aac, opus) skip video (`-vn`); gif skips audio.
pub fn start_export(_project: Project, _opts: ExportOptions, _text: Arc<Mutex<TextRasterizer>>) -> Arc<Progress> {
    todo!("engine::export::start_export")
}

/// If the project is a pure cut of exactly one video source (every clip from the same asset, audio clips
/// exactly mirroring their linked video clip's timing on each audio stream (muted tracks may be dropped),
/// no gaps between clips, no text/image clips, no effects, volume 1, all clips enabled), return the
/// segments as (src_in, duration) in timeline order. Such a project can be written with `-c copy`.
pub fn lossless_segments(_project: &Project) -> Option<Vec<(f64, f64)>> {
    todo!("engine::export::lossless_segments")
}

/// Lossless cut: `ffmpeg -ss in -t dur -i src -c copy -avoid_negative_ts make_zero` per segment (with
/// `-map` dropping muted audio streams), then concat demuxer when there is more than one segment.
/// Cuts land on keyframes (not frame-accurate) — that's the trade for being instant.
pub fn start_lossless_cut(_project: Project, _out_path: PathBuf) -> Arc<Progress> {
    todo!("engine::export::start_lossless_cut")
}

/// Encoder names ffmpeg reports (`ffmpeg -hide_banner -encoders`), cached after the first call.
/// Empty if ffmpeg is missing.
pub fn detect_encoders() -> Vec<String> {
    todo!("engine::export::detect_encoders")
}

/// ffmpeg output arguments (codec/quality) for an extension, given the user's encoder preference and the
/// available encoders. "auto": mp4/mov/mkv/m4v → libx264 (+aac), webm → libvpx-vp9 (+libopus),
/// gif → gif, avi → mpeg4 (+mp3), audio-only → sensible codec for the container.
/// Hardware encoders (nvenc/qsv/amf) use `-cq`/`-global_quality`/`-qp` instead of `-crf`.
pub fn codec_args(_ext: &str, _encoder: &str, _crf: u32, _preset: &str, _available: &[String]) -> Vec<String> {
    todo!("engine::export::codec_args")
}

pub const VIDEO_EXTS: &[&str] = &["mp4", "mov", "mkv", "webm", "avi", "m4v", "gif"];
pub const AUDIO_EXTS: &[&str] = &["mp3", "wav", "m4a", "flac", "ogg", "aac", "opus"];
