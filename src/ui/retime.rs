//! Retime window (Ctrl+R): speed / reverse / freeze for the selected clips (linked clips follow).
//! egui::Window "Speed / Retime": speed as a percent DragValue (1..10000 %, 100 = normal) + preset buttons
//! (25 % 50 % 100 % 200 % 400 %), "Reverse" checkbox, "Freeze frame at playhead" button
//! (Project::freeze_at(playhead, selection)) and "Unfreeze" (clear `freeze`), "Apply" → Project::set_speed
//! (shows a note when the new length collides with a neighbour and nothing changed). Shows the resulting
//! duration live. Undo once per applied change. Returns true when the project changed.

use crate::model::{Id, Project};
use eframe::egui;

#[derive(Default)]
pub struct RetimeUi {
    pub open: bool,
    /// Percent being edited (synced from the first selected clip when the window opens / selection changes).
    pub percent: f64,
    pub reverse: bool,
    pub last_clip: Option<Id>,
    pub note: String,
}

pub fn show(
    _ctx: &egui::Context,
    _state: &mut RetimeUi,
    _project: &mut Project,
    _selection: &[Id],
    _playhead: f64,
    _undo: &mut dyn FnMut(&Project),
) -> bool {
    todo!("ui::retime::show")
}
