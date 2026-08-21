//! egui UI. Layout (DaVinci-like, bare): menu bar / [Library | Preview | Inspector] / Timeline.

pub mod app;
pub mod inspector;
pub mod library;
pub mod preview;
pub mod settings_ui;
pub mod timeline;

use crate::model::Id;

/// Drag-and-drop payload from the library / recent panels to the timeline (egui dnd API).
#[derive(Clone, Debug)]
pub enum DragPayload {
    /// A library asset id.
    Asset(Id),
    /// A file path (recent panel) — the timeline reports it back as a dropped file.
    Path(String),
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
