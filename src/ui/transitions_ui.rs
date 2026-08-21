//! Transitions panel. Catalogue: TransitionKind::ALL with a default duration DragValue (0.1..5 s) and,
//! for FadeToColor a colour button, for Push/Wipe a direction selector. "Add at start of selected clip"
//! (Project::add_transition(right = selected clip)) and "Add at end of selected clip" (right = the clip
//! that starts where the selected one ends, on the same track) — disabled with a hint when the cut has no
//! abutting neighbour. Below: the transitions touching the selected clip (Project::transitions_of):
//! kind combo, duration, colour/direction/ease editors, ✕ remove. Returns true when the project changed
//! (call `undo` once per gesture first).

use crate::model::{Id, Project};
use crate::theme::Palette;
use eframe::egui;

#[derive(Default)]
pub struct TransitionsState {
    /// Catalogue settings for the next transition to add.
    pub duration: f64,
    pub kind: usize,
    pub color: [u8; 4],
    pub direction: u8,
}

pub fn show(
    _ui: &mut egui::Ui,
    _state: &mut TransitionsState,
    _project: &mut Project,
    _selection: &[Id],
    _playhead: f64,
    _palette: &Palette,
    _undo: &mut dyn FnMut(&Project),
) -> bool {
    todo!("ui::transitions_ui::show")
}
