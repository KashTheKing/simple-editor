//! egui UI. Dockable layout (ui/layout.rs) of panes: Library, Preview, Inspector, Effects, Transitions,
//! Subtitles, Timeline, Curves, Planner, Auto-cut — DaVinci-like by default, bare Windows-forms styling.
//! Windows (Settings, Retime, Export) never block the editor.

pub mod app;
pub mod autocut_ui;
pub mod capture_ui;
pub mod curves;
pub mod effects_ui;
pub mod export_ui;
pub mod frame_ui;
pub mod import_ui;
pub mod inspector;
pub mod layout;
pub mod library;
pub mod markers_ui;
pub mod mixer_ui;
pub mod nodes;
pub mod paste_ui;
pub mod planner;
pub mod preview;
pub mod retime;
pub mod settings_ui;
pub mod subtitles_ui;
pub mod timeline;
pub mod tools;
pub mod transitions_ui;

use crate::model::{Id, LABEL_COLORS};
use crate::theme::Palette;
use eframe::egui;

/// Drag-and-drop payload from the library / recent / planner panels to the timeline (egui dnd API).
#[derive(Clone, Debug)]
pub enum DragPayload {
    /// A library asset id.
    Asset(Id),
    /// A file path (recent panel, linked folders) — the timeline reports it back as a dropped file.
    Path(String),
    /// A nested timeline (Project.sequences) id.
    Sequence(Id),
    /// A saved template (Settings.templates) by name.
    Template(String),
}

/// HH:MM:SS:FF timecode.
pub fn timecode(t: f64, fps: f64) -> String {
    let t = t.max(0.0);
    let fps = fps.max(1.0);
    let total_frames = (t * fps).round() as u64;
    let fpsr = fps.round() as u64;
    let f = total_frames % fpsr;
    let s = total_frames / fpsr;
    format!("{:02}:{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60, f)
}

/// Short duration text "1:23.4".
pub fn duration_text(t: f64) -> String {
    let m = (t / 60.0).floor() as u64;
    let s = t - m as f64 * 60.0;
    format!("{m}:{s:04.1}")
}

/// Colour of a clip/asset/recent label index (0 or out of range = "no label" dim).
pub fn label_color(idx: u8, palette: &Palette) -> egui::Color32 {
    if idx == 0 || idx as usize > LABEL_COLORS.len() {
        palette.text_dim
    } else {
        let [r, g, b] = LABEL_COLORS[idx as usize - 1].1;
        egui::Color32::from_rgb(r, g, b)
    }
}

/// Name of a label index ("None" for 0 / out of range).
pub fn label_name(idx: u8) -> &'static str {
    if idx == 0 || idx as usize > LABEL_COLORS.len() {
        "None"
    } else {
        LABEL_COLORS[idx as usize - 1].0
    }
}

/// ComboBox over (value, label) pairs writing into a String setting. Returns true if the value changed.
pub fn combo(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[(&str, &str)], width: Option<f32>) -> bool {
    let mut changed = false;
    let current = options.iter().find(|(v, _)| *v == value.as_str()).map(|(_, l)| *l).unwrap_or(value.as_str());
    let mut cb = egui::ComboBox::from_id_salt(id).selected_text(current);
    if let Some(w) = width {
        cb = cb.width(w);
    }
    cb.show_ui(ui, |ui| {
        for (v, label) in options {
            if ui.selectable_label(value.as_str() == *v, *label).clicked() && value.as_str() != *v {
                *value = (*v).to_string();
                changed = true;
            }
        }
    });
    changed
}

/// x264-style speed presets offered by the settings and export windows.
pub const ENCODER_PRESETS: [&str; 7] = ["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow"];

/// Encoder names worth offering: the h264 / hevc / vp9 / av1 families (software + nvenc/qsv/amf).
pub fn encoder_options(encoders: &[String]) -> Vec<&str> {
    const KEYS: [&str; 9] = ["264", "265", "hevc", "vp9", "av1", "x264", "x265", "nvenc", "qsv"];
    encoders.iter().map(String::as_str).filter(|e| KEYS.iter().any(|k| e.contains(k)) || e.contains("amf")).collect()
}
