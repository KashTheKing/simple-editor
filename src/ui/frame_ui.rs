//! "Export Frame" window: save the frame at the playhead as an image.
//! Options: resolution (Project / 2x / 4x / Custom W x H), upscaling mode (`Scaler` for the render plus
//! the ffmpeg flag for the final resize), format (PNG / JPG / WebP with a quality slider for the lossy
//! ones), and "Render with effects" vs "Source frame only" (the decoded frame of the top-most clip under
//! the playhead, no compositing). Non-modal; returns the chosen options when the user confirms.

use crate::model::Scaler;
// the resize flags are the export window's list: a scaler picked there (area / spline) must not
// silently turn into something else here
use crate::ui::combo;
use crate::ui::export_ui::SCALERS as RESIZE;
use eframe::egui;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct FrameExport {
    pub out: PathBuf,
    pub size: (u32, u32),
    pub scaler: Scaler,
    /// ffmpeg scale flag for the final resize (one of `ui::export_ui::SCALERS`).
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
    /// Compositor resampling used while rendering (defaults to the project's).
    pub scaler: Scaler,
    /// ffmpeg scale flag for the final resize.
    pub resize: String,
}

const PRESETS: [(&str, &str); 4] = [("project", "Project"), ("2x", "2 ×"), ("4x", "4 ×"), ("custom", "Custom")];
const FORMATS: [(&str, &str); 3] = [("png", "PNG (lossless)"), ("jpg", "JPG"), ("webp", "WebP")];

/// Output size for a preset. `custom` is clamped to something ffmpeg will accept.
fn size_of(pw: u32, ph: u32, preset: &str, custom: (u32, u32)) -> (u32, u32) {
    match preset {
        "2x" => (pw * 2, ph * 2),
        "4x" => (pw * 4, ph * 4),
        "custom" => (custom.0.clamp(16, 16384), custom.1.clamp(16, 16384)),
        _ => (pw.max(1), ph.max(1)),
    }
}

/// What `settings.frame_resolution` remembers: a preset name, or "WxH" for a custom size (`init` parses
/// both — storing the literal "custom" would lose the size).
fn stored_resolution(state: &FrameUi) -> String {
    if state.preset == "custom" {
        format!("{}x{}", state.custom.0, state.custom.1)
    } else {
        state.preset.clone()
    }
}

fn lossy(format: &str) -> bool {
    format != "png"
}

pub fn show(
    ctx: &egui::Context,
    state: &mut FrameUi,
    project: &crate::model::Project,
    settings: &mut crate::settings::Settings,
) -> Option<FrameExport> {
    if !state.open {
        return None;
    }
    if state.preset.is_empty() {
        init(state, project, settings);
    }
    let mut out = None;
    let mut open = state.open;
    egui::Window::new("Export Frame").open(&mut open).resizable(false).default_width(300.0).show(ctx, |ui| {
        let size = egui::Grid::new("frame_opts")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Resolution");
                if combo(ui, "frame_res", &mut state.preset, &PRESETS, None) {
                    settings.frame_resolution = stored_resolution(state);
                }
                ui.end_row();
                if state.preset == "custom" {
                    ui.label("Size");
                    ui.horizontal(|ui| {
                        let (mut w, mut h) = state.custom;
                        ui.add(egui::DragValue::new(&mut w).range(16..=16384));
                        ui.label("×");
                        ui.add(egui::DragValue::new(&mut h).range(16..=16384));
                        state.custom = (w, h);
                    });
                    ui.end_row();
                }
                let size = size_of(project.width, project.height, &state.preset, state.custom);
                ui.label("Output");
                ui.weak(format!("{}×{}", size.0, size.1));
                ui.end_row();

                ui.label("Render");
                egui::ComboBox::from_id_salt("frame_scaler").selected_text(state.scaler.name()).show_ui(ui, |ui| {
                    for s in Scaler::ALL {
                        ui.selectable_value(&mut state.scaler, s, s.name());
                    }
                });
                ui.end_row();

                ui.label("Resize");
                ui.horizontal(|ui| {
                    combo(ui, "frame_resize", &mut state.resize, &RESIZE, None);
                    if size == (project.width, project.height) {
                        ui.weak("(same size)");
                    }
                });
                ui.end_row();

                ui.label("Format");
                combo(ui, "frame_format", &mut settings.frame_format, &FORMATS, None);
                ui.end_row();

                if lossy(&settings.frame_format) {
                    ui.label("Quality");
                    ui.add(egui::Slider::new(&mut state.quality, 1..=100));
                    ui.end_row();
                }
                size
            })
            .inner;

        ui.horizontal(|ui| {
            ui.radio_value(&mut state.with_effects, true, "Render with effects");
            ui.radio_value(&mut state.with_effects, false, "Source frame only");
        });
        if !state.with_effects {
            ui.weak("The decoded frame of the top-most clip under the playhead, at its own size.");
        }
        ui.add_space(4.0);
        if ui.button("Save Frame…").clicked() {
            settings.frame_quality = state.quality;
            settings.frame_resolution = stored_resolution(state);
            settings.save();
            if let Some(path) = pick(project, &settings.frame_format) {
                out = Some(FrameExport {
                    out: path,
                    size,
                    scaler: state.scaler,
                    resize: state.resize.clone(),
                    with_effects: state.with_effects,
                    quality: state.quality.clamp(1, 100),
                });
            }
        }
    });
    state.open = open && out.is_none();
    out
}

fn init(state: &mut FrameUi, project: &crate::model::Project, settings: &crate::settings::Settings) {
    state.custom = (project.width, project.height);
    state.scaler = project.scaler;
    state.quality = settings.frame_quality.clamp(1, 100);
    state.with_effects = true;
    if state.resize.is_empty() {
        state.resize = if RESIZE.iter().any(|(v, _)| *v == settings.export_scaler) {
            settings.export_scaler.clone()
        } else {
            "lanczos".into()
        };
    }
    let r = settings.frame_resolution.as_str();
    state.preset = if PRESETS.iter().any(|(v, _)| *v == r) {
        r.to_string()
    } else if let Some((w, h)) = r.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))) {
        state.custom = (w, h);
        "custom".into()
    } else {
        "project".into()
    };
}

fn pick(project: &crate::model::Project, format: &str) -> Option<PathBuf> {
    let ext = if format.is_empty() { "png" } else { format };
    rfd::FileDialog::new()
        .add_filter(ext.to_uppercase(), &[ext])
        .set_file_name(format!("{}-frame.{ext}", project.name))
        .save_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Project;
    use crate::settings::Settings;

    #[test]
    fn sizes() {
        assert_eq!(size_of(1920, 1080, "project", (0, 0)), (1920, 1080));
        assert_eq!(size_of(1920, 1080, "2x", (0, 0)), (3840, 2160));
        assert_eq!(size_of(1920, 1080, "4x", (0, 0)), (7680, 4320));
        assert_eq!(size_of(1920, 1080, "custom", (640, 4)), (640, 16), "custom sizes are clamped");
        assert_eq!(size_of(1920, 1080, "", (0, 0)), (1920, 1080), "an unknown preset is the project size");
    }

    #[test]
    fn init_from_settings() {
        let p = Project::new();
        let mut s = Settings::default();
        let mut st = FrameUi::default();
        init(&mut st, &p, &s);
        assert_eq!(st.preset, "project");
        assert_eq!(st.resize, "lanczos");
        assert_eq!(st.quality, s.frame_quality);
        // a stored "WxH" comes back as a custom size
        s.frame_resolution = "800x600".into();
        s.export_scaler = "bicubic".into();
        let mut st = FrameUi::default();
        init(&mut st, &p, &s);
        assert_eq!((st.preset.as_str(), st.custom), ("custom", (800, 600)));
        assert_eq!(st.resize, "bicubic");
        // every export scaler is offered here too — no silent downgrade to lanczos
        s.export_scaler = "spline".into();
        let mut st = FrameUi::default();
        init(&mut st, &p, &s);
        assert_eq!(st.resize, "spline");
        s.export_scaler = "nonsense".into();
        let mut st = FrameUi::default();
        init(&mut st, &p, &s);
        assert_eq!(st.resize, "lanczos");
    }

    /// A custom size survives a restart: settings store "WxH", not the literal "custom".
    #[test]
    fn custom_size_round_trips_through_settings() {
        let p = Project::new();
        let mut s = Settings::default();
        let st = FrameUi { preset: "custom".into(), custom: (3840, 2160), ..Default::default() };
        s.frame_resolution = stored_resolution(&st);
        assert_eq!(s.frame_resolution, "3840x2160");
        let mut back = FrameUi::default();
        init(&mut back, &p, &s);
        assert_eq!((back.preset.as_str(), back.custom), ("custom", (3840, 2160)));
        // a plain preset still stores its own name
        let st = FrameUi { preset: "2x".into(), ..Default::default() };
        assert_eq!(stored_resolution(&st), "2x");
    }

    /// Headless: the window lays out for every preset and confirms nothing on its own.
    #[test]
    fn show_headless() {
        let ctx = egui::Context::default();
        let p = Project::new();
        let mut s = Settings::default();
        for preset in ["project", "2x", "custom"] {
            for format in ["png", "jpg"] {
                s.frame_format = format.into();
                let mut st = FrameUi { open: true, ..Default::default() };
                st.preset = preset.into();
                st.custom = (640, 480);
                let _ = ctx.run(egui::RawInput::default(), |ctx| assert!(show(ctx, &mut st, &p, &mut s).is_none()));
                assert!(st.open);
            }
        }
    }
}
