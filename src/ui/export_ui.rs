//! Export window (non-blocking egui::Window "Export"): resolution (Project / 720p / 1080p / 1440p / 4K /
//! Custom W×H, keeping the project aspect unless unlocked), scaler for up/downscaling (Nearest, Bilinear,
//! Bicubic, Lanczos, Area, Spline → ffmpeg flags), encoder (auto + detected), quality CRF, preset, an
//! "audio only" hint by extension, and the Fast Lossless Cut option when `export::lossless_segments` applies
//! (with the keyframe-accuracy note). "Export…" opens the save dialog and returns the options; the window
//! stays usable while an export runs (progress + cancel are shown by the app). Settings remember the last
//! scaler/resolution. Returns Some(options) when the user confirmed an export this frame.

use crate::engine::export::ExportOptions;
use crate::model::Project;
use crate::settings::Settings;
use eframe::egui;

#[derive(Default)]
pub struct ExportUi {
    pub open: bool,
    /// "project" | "720" | "1080" | "1440" | "2160" | "custom"
    pub preset: String,
    pub custom: (u32, u32),
    pub lossless: bool,
}

pub struct ExportChoice {
    pub opts: ExportOptions,
    pub lossless: bool,
}

pub fn show(
    _ctx: &egui::Context,
    _state: &mut ExportUi,
    _project: &Project,
    _settings: &mut Settings,
    _encoders: &[String],
    _exporting: bool,
) -> Option<ExportChoice> {
    todo!("ui::export_ui::show")
}
