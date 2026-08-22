//! Screen recording and voiceover capture, both through ffmpeg child processes (no new deps).
//!
//! Screen: `ffmpeg -f gdigrab -framerate FPS [-offset_x X -offset_y Y -video_size WxH] -i desktop
//! [-f dshow -i audio="<mic>"] -c:v libx264 -preset veryfast <rate args> out.mp4`. Desktop audio needs a
//! loopback dshow device (see `audio_devices`). `auto_on_blur` (settings) starts the recorder when the
//! editor loses focus and stops when it regains focus, so another app can be recorded hands-free.
//!
//! Voiceover: `ffmpeg -f dshow -i audio="<device>" -ac N -ar 48000 out.wav`, started/stopped from the
//! voiceover panel while playback continues; the file is imported and placed at the record start time.
//!
//! Both report through `engine::export::Progress` and stop cleanly (ffmpeg gets `q` on stdin, so the
//! container is finalised).

use crate::engine::export::Progress;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct ScreenCaptureOptions {
    pub out: PathBuf,
    pub fps: u32,
    /// Video bitrate in kbit/s (0 = use `crf` instead).
    pub bitrate_kbps: u32,
    pub crf: u32,
    /// None = whole desktop; Some((x, y, w, h)) = a region in desktop pixels.
    pub region: Option<(i32, i32, u32, u32)>,
    /// dshow device name for the microphone (empty = none).
    pub mic: String,
    pub desktop_audio: bool,
    pub cursor: bool,
}

#[derive(Clone, Debug)]
pub struct VoiceoverOptions {
    pub out: PathBuf,
    pub device: String,
    pub sample_rate: u32,
    pub channels: u32,
}

/// A running capture. `stop()` finalises the file; dropping it stops too.
pub struct Capture {
    #[allow(dead_code)]
    progress: Arc<Progress>,
}

impl Capture {
    pub fn progress(&self) -> Arc<Progress> {
        self.progress.clone()
    }
    /// Ask ffmpeg to finish; the file is complete once `progress.is_done()`.
    pub fn stop(self) {
        todo!("engine::capture::Capture::stop")
    }
    /// Seconds recorded so far.
    pub fn elapsed(&self) -> f64 {
        todo!("engine::capture::Capture::elapsed")
    }
}

pub fn start_screen(_opts: ScreenCaptureOptions) -> Result<Capture, String> {
    todo!("engine::capture::start_screen")
}

pub fn start_voiceover(_opts: VoiceoverOptions) -> Result<Capture, String> {
    todo!("engine::capture::start_voiceover")
}

/// dshow audio input devices (`ffmpeg -list_devices true -f dshow -i dummy`), cached.
/// The bool marks devices that look like desktop/loopback capture.
pub fn audio_devices() -> Vec<(String, bool)> {
    todo!("engine::capture::audio_devices")
}
