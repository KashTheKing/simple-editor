//! "Export Frame" window: save the frame at the playhead as an image.
//! Options: resolution (Project / 2x / 4x / Custom W x H), upscaling mode (`Scaler` for the render plus
//! the ffmpeg flag for the final resize), format (PNG / JPG / WebP with a quality slider for the lossy
//! ones), and "Render with effects" vs "Source frame only" (the decoded frame of the top-most clip under
//! the playhead, no compositing). Non-modal; returns the chosen options when the user confirms.

use crate::model::Scaler;
use eframe::egui;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct FrameExport {
    pub out: PathBuf,
    pub size: (u32, u32),
    pub scaler: Scaler,
    /// ffmpeg scale flag for the final resize ("neighbor" | "bilinear" | "bicubic" | "lanczos").
    pub resize: String,
    /// false = the raw source frame under the playhead.
    pub with_effects: bool,
    /// 1..100 for JPG/WebP.
    pub quality: u32,
}

#[derive(Default)]
pub struct FrameUi {
    pub open: bool,
    pub preset: String,
    pub custom: (u32, u32),
    pub with_effects: bool,
    pub quality: u32,
}

pub fn show(
    _ctx: &egui::Context,
    _state: &mut FrameUi,
    _project: &crate::model::Project,
    _settings: &mut crate::settings::Settings,
) -> Option<FrameExport> {
    todo!("ui::frame_ui::show")
}
