//! Windows Media Foundation backend (IMFSourceReader). Native, hardware-accelerated, no external deps.
//!
//! Contract (see media/mod.rs):
//!  * `probe(path)`      -> Asset with duration, size, fps, every audio stream (language/title if available).
//!  * `open_video(path)` -> VideoSource producing top-down RGBA8 at the requested size (RGB32 output type +
//!                          MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING; handle negative stride / bottom-up;
//!                          scale to (w,h) — either via the reader's output type or a simple resize in Rust).
//!  * `open_audio(path, stream)` -> AudioSource producing stereo f32 @ 48 kHz (PCM float output type; stream
//!                          = Nth audio stream in container order; resample/upmix in Rust if MF refuses).
//! Seeking: SetCurrentPosition then decode forward until pts >= t. Sequential reads must not seek.
//! COM: MFStartup / CoInitializeEx per thread (decoders live on the thread that created them; they are `Send`
//! in the sense that a thread creates and owns them — mark types `unsafe impl Send` if needed, see ARCHITECTURE.md).

use super::{AudioSource, Frame, VideoSource};
use crate::model::Asset;

pub fn probe(_path: &str) -> Result<Asset, String> {
    Err("mf::probe not implemented".into())
}

pub fn open_video(_path: &str) -> Result<Box<dyn VideoSource>, String> {
    Err("mf::open_video not implemented".into())
}

pub fn open_audio(_path: &str, _stream: usize) -> Result<Box<dyn AudioSource>, String> {
    Err("mf::open_audio not implemented".into())
}

#[allow(dead_code)]
fn _unused(_: &mut Frame) {}
