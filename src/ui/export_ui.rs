//! Export window (non-blocking egui::Window "Export"): resolution (Project / 720p / 1080p / 1440p / 4K /
//! Custom W×H, keeping the project aspect unless unlocked), scaler for up/downscaling (Nearest, Bilinear,
//! Bicubic, Lanczos, Area, Spline → ffmpeg flags), encoder (auto + detected), quality CRF, preset, an
//! "audio only" hint by extension, and the Fast Lossless Cut option when `export::lossless_segments` applies
//! (with the keyframe-accuracy note). "Export…" opens the save dialog and returns the options; the window
//! stays usable while an export runs (progress + cancel are shown by the app). Settings remember the last
//! scaler/resolution. Returns Some(options) when the user confirmed an export this frame.

use crate::engine::export::{self, ExportOptions};
use crate::media::Backend;
use crate::model::Project;
use crate::settings::Settings;
use crate::ui::{combo, encoder_options, ENCODER_PRESETS};
use eframe::egui;
use std::path::Path;

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

const RES_PRESETS: [(&str, &str); 6] = [
    ("project", "Project"),
    ("720", "720p"),
    ("1080", "1080p"),
    ("1440", "1440p"),
    ("2160", "4K"),
    ("custom", "Custom"),
];
const SCALERS: [(&str, &str); 6] = [
    ("neighbor", "Nearest"),
    ("bilinear", "Bilinear"),
    ("bicubic", "Bicubic"),
    ("lanczos", "Lanczos"),
    ("area", "Area"),
    ("spline", "Spline"),
];

/// Output size for a resolution preset: None = project size. Height presets derive the width from the
/// project aspect (rounded to even).
fn preset_size(pw: u32, ph: u32, preset: &str, custom: (u32, u32)) -> Option<(u32, u32)> {
    match preset {
        "project" | "" => None,
        "custom" => Some((custom.0.max(16), custom.1.max(16))),
        h => {
            let h: u32 = h.parse().ok()?;
            let w = (pw as f64 / ph.max(1) as f64 * h as f64).round() as u32;
            let out = (w + (w & 1), h);
            if out == (pw, ph) {
                None
            } else {
                Some(out)
            }
        }
    }
}

pub fn show(
    ctx: &egui::Context,
    state: &mut ExportUi,
    project: &Project,
    settings: &mut Settings,
    encoders: &[String],
    exporting: bool,
) -> Option<ExportChoice> {
    if !state.open {
        return None;
    }
    if state.preset.is_empty() {
        init_from_settings(state, settings, project);
    }
    let mut out = None;
    let mut open = state.open;
    egui::Window::new("Export").open(&mut open).resizable(false).default_width(300.0).show(ctx, |ui| {
        egui::Grid::new("export_opts").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Resolution");
            let mut changed = combo(ui, "export_res", &mut state.preset, &RES_PRESETS, None);
            ui.end_row();
            if state.preset == "custom" {
                ui.label("Size");
                ui.horizontal(|ui| {
                    let (mut w, mut h) = state.custom;
                    changed |= ui.add(egui::DragValue::new(&mut w).range(16..=8192)).changed();
                    ui.label("×");
                    changed |= ui.add(egui::DragValue::new(&mut h).range(16..=8192)).changed();
                    state.custom = (w, h);
                });
                ui.end_row();
            }
            let size = preset_size(project.width, project.height, &state.preset, state.custom);
            ui.label("Output");
            let (w, h) = size.unwrap_or((project.width, project.height));
            ui.weak(format!("{w}×{h}"));
            ui.end_row();
            if changed {
                settings.export_resolution =
                    if state.preset == "project" { "project".into() } else { format!("{w}x{h}") };
            }

            ui.label("Scaler");
            ui.horizontal(|ui| {
                combo(ui, "export_scaler", &mut settings.export_scaler, &SCALERS, None);
                if size.is_none() {
                    ui.weak("(same size)");
                }
            });
            ui.end_row();

            ui.label("Encoder");
            let opts: Vec<(&str, &str)> = std::iter::once(("auto", "auto"))
                .chain(encoder_options(encoders).into_iter().map(|e| (e, e)))
                .collect();
            combo(ui, "export_encoder", &mut settings.encoder, &opts, None);
            ui.end_row();

            ui.label("Quality (CRF)");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut settings.crf).range(0..=51));
                ui.weak("lower = better");
            });
            ui.end_row();

            ui.label("Preset");
            let opts: Vec<(&str, &str)> = ENCODER_PRESETS.iter().map(|p| (*p, *p)).collect();
            combo(ui, "export_preset", &mut settings.preset, &opts, None);
            ui.end_row();
        });

        if export::lossless_segments(project).is_some() {
            ui.checkbox(&mut state.lossless, "Fast lossless cut (-c copy)");
            if state.lossless {
                ui.weak("Instant, no re-encode; cuts snap to keyframes (may shift up to a few frames).");
            }
        } else {
            state.lossless = false;
        }
        ui.weak("Audio extensions (mp3, wav, m4a, flac) export audio only; gif has no audio.");
        if settings.gpu || settings.preview_quality < 100 {
            // Settings ▸ Performance only changes what you watch, never what is written
            ui.weak("GPU preview and preview quality do not change the exported picture.");
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let can = !exporting && !project.is_empty();
            if ui.add_enabled(can, egui::Button::new("Export…")).clicked() {
                settings.save();
                if let Some(choice) = pick_and_build(state, project, settings) {
                    out = Some(choice);
                }
            }
            if exporting {
                ui.weak("export running…");
            }
        });
    });
    state.open = open;
    if !open {
        settings.save(); // the window's options are settings — keep them when it closes without exporting
    }
    out
}

fn init_from_settings(state: &mut ExportUi, settings: &Settings, project: &Project) {
    state.custom = (project.width, project.height);
    let r = settings.export_resolution.as_str();
    if r == "project" || r.is_empty() {
        state.preset = "project".into();
        return;
    }
    let parsed = r.split_once('x').and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)));
    let Some((w, h)) = parsed else {
        state.preset = "project".into();
        return;
    };
    state.custom = (w, h);
    // matches a height preset with the project aspect? use the preset, else custom
    let named = ["720", "1080", "1440", "2160"]
        .iter()
        .find(|p| preset_size(project.width, project.height, p, (0, 0)) == Some((w, h)));
    state.preset = named.map(|p| (*p).to_string()).unwrap_or_else(|| "custom".into());
}

/// Save dialog (same filters as the app's Export) → ExportChoice.
fn pick_and_build(state: &ExportUi, project: &Project, settings: &Settings) -> Option<ExportChoice> {
    let mut d = rfd::FileDialog::new();
    if state.lossless {
        let src = project.source_video.clone().or_else(|| project.assets.first().map(|a| a.path.clone()));
        let ext = src
            .as_ref()
            .and_then(|s| Path::new(s).extension().map(|e| e.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "mp4".into());
        d = d
            .add_filter(format!("{} (same container)", ext.to_uppercase()), &[ext.as_str()])
            .set_file_name(format!("{}_cut.{ext}", project.name));
    } else {
        d = d
            .add_filter("MP4 video", &["mp4"])
            .add_filter("MOV video", &["mov"])
            .add_filter("MKV video", &["mkv"])
            .add_filter("WebM video", &["webm"])
            .add_filter("AVI video", &["avi"])
            .add_filter("GIF", &["gif"])
            .add_filter("MP3 audio", &["mp3"])
            .add_filter("WAV audio", &["wav"])
            .add_filter("M4A audio", &["m4a"])
            .add_filter("FLAC audio", &["flac"])
            .add_filter("Any (by extension)", &["*"])
            .set_file_name(format!("{}_edit.mp4", project.name));
    }
    if let Some(dir) = project.source_video.as_ref().and_then(|p| Path::new(p).parent()) {
        d = d.set_directory(dir);
    }
    let out_path = d.save_file()?;
    Some(ExportChoice {
        opts: ExportOptions {
            out_path,
            encoder: settings.encoder.clone(),
            crf: settings.crf,
            preset: settings.preset.clone(),
            backend: Backend::parse(&settings.decoder),
            out_size: preset_size(project.width, project.height, &state.preset, state.custom),
            scaler: settings.export_scaler.clone(),
        },
        lossless: state.lossless,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_sizes() {
        // 16:9 project: 720p derives 1280 wide
        assert_eq!(preset_size(1920, 1080, "720", (0, 0)), Some((1280, 720)));
        assert_eq!(preset_size(1920, 1080, "1440", (0, 0)), Some((2560, 1440)));
        // same as project → None
        assert_eq!(preset_size(1920, 1080, "1080", (0, 0)), None);
        assert_eq!(preset_size(1920, 1080, "project", (0, 0)), None);
        // odd widths are rounded to even: 854.4 → 854
        assert_eq!(preset_size(1280, 720, "480", (0, 0)), Some((854, 480)));
        assert_eq!(preset_size(1920, 1080, "custom", (100, 50)), Some((100, 50)));
    }

    #[test]
    fn init_matches_named_preset() {
        let mut p = Project::new(); // 1920x1080
        p.width = 1920;
        p.height = 1080;
        let mut s = Settings::default();
        s.export_resolution = "1280x720".into();
        let mut st = ExportUi::default();
        init_from_settings(&mut st, &s, &p);
        assert_eq!(st.preset, "720");
        s.export_resolution = "640x360".into();
        st.preset.clear();
        init_from_settings(&mut st, &s, &p);
        assert_eq!(st.preset, "custom");
        assert_eq!(st.custom, (640, 360));
    }

    /// Headless: the window lays out without panicking and returns None with no interaction.
    #[test]
    fn show_headless() {
        let project = Project::new();
        let mut settings = Settings::default();
        let mut state = ExportUi { open: true, ..Default::default() };
        let ctx = egui::Context::default();
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                let r = show(ctx, &mut state, &project, &mut settings, &["libx264".into()], false);
                assert!(r.is_none());
            });
        }
        assert!(state.open);
        assert_eq!(state.preset, "project");
    }
}
