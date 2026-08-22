//! Screen recording + voiceover windows (both non-modal, both keep the editor usable).
//!
//! Screen recorder: output path, fps, bitrate (or CRF), region (Whole desktop / Region… with a
//! drag-to-pick overlay / current monitor), microphone combo (`engine::capture::audio_devices`), desktop
//! audio toggle, cursor toggle, and "Record when the editor loses focus" (auto start/stop) —
//! the app watches focus and drives `engine::capture`. While recording: elapsed time, a stop button and a
//! note that the file is imported into the library when it finishes.
//!
//! Voiceover: input device, channels, a level meter, "Record from the playhead" (playback rolls while
//! recording, so the take lines up), stop → the WAV is imported and placed on an audio track at the start
//! time, with a "Retake" that deletes the last take and records again.

use crate::settings::Settings;
use crate::theme::Palette;
use eframe::egui;
use std::path::PathBuf;

#[derive(Default)]
pub struct CaptureUi {
    pub screen_open: bool,
    pub voice_open: bool,
    /// Region picker armed (the next drag on the desktop overlay sets the region).
    pub picking_region: bool,
    pub elapsed: f64,
}

/// What the app should do this frame.
#[derive(Default)]
pub struct CaptureResponse {
    pub start_screen: bool,
    pub stop_screen: bool,
    pub start_voice: bool,
    pub stop_voice: bool,
    /// A finished recording to import (path, place on the timeline at this time).
    pub import: Option<(PathBuf, Option<f64>)>,
}

pub fn show(
    _ctx: &egui::Context,
    _state: &mut CaptureUi,
    _settings: &mut Settings,
    _recording_screen: bool,
    _recording_voice: bool,
    _palette: &Palette,
) -> CaptureResponse {
    todo!("ui::capture_ui::show")
}
