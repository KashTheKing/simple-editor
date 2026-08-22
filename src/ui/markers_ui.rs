//! Markers pane: every marker in timeline order (project markers and clip markers together, via
//! `Project::markers_in_timeline`). Rows: colour label dot (menu), time (editable DragValue, seconds),
//! duration for ranges, name, note (multiline when the row is selected), "Go" (seek) and X (delete).
//! Toolbar: "Add at playhead" (M), "Add on selected clip", filter by label, and "Copy as list" (markdown,
//! handy for the AI tools). Undo once per gesture; returns what changed.

use crate::model::{Id, Project};
use crate::theme::Palette;
use eframe::egui;

#[derive(Default)]
pub struct MarkersState {
    pub selected: Option<Id>,
    /// 0 = every label.
    pub filter_label: u8,
}

#[derive(Default)]
pub struct MarkersResponse {
    pub edited: bool,
    pub seek: Option<f64>,
}

pub fn show(
    _ui: &mut egui::Ui,
    _state: &mut MarkersState,
    _project: &mut Project,
    _selection: &[Id],
    _playhead: f64,
    _palette: &Palette,
    _undo: &mut dyn FnMut(&Project),
) -> MarkersResponse {
    todo!("ui::markers_ui::show")
}
