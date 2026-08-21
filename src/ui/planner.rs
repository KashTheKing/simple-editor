//! Planner pane: tabs "Plan" and "Notes".
//! Plan = a nested to-do tree (Project.plan): each row = checkbox (done), editable title, colour label dot
//! (click → LABEL_COLORS menu), ▸/▾ collapse, "+" add sub-task, "✕" remove, ↑/↓ reorder among siblings,
//! "⇥"/"⇤" indent/outdent; a selected item shows its notes (multiline) and its moodboard: the asset ids in
//! `assets` as rows (name, kind tag, thumbnail via ThumbCache if available, per-asset note in `asset_notes`,
//! "Add to timeline", "✕"); assets are added by dropping library items (DragPayload::Asset) onto the item or
//! onto the moodboard area. Progress bar "done/total" at the top; "Add task" at the top level;
//! "Clear completed". Notes tab = Project.notes as a large multiline TextEdit ("describe your process,
//! ideas, style while you edit — the style summary / AI tools read this"). Undo once per gesture.
//! Returns what changed.

use crate::media::thumbs::ThumbCache;
use crate::model::{Id, Project};
use crate::theme::Palette;
use eframe::egui;

#[derive(Default)]
pub struct PlannerState {
    pub tab: usize,
    pub selected: Option<Id>,
    pub collapsed: Vec<Id>,
}

#[derive(Default)]
pub struct PlannerResponse {
    pub edited: bool,
    /// Asset ids the user asked to place on the timeline at the playhead.
    pub add_to_timeline: Vec<Id>,
}

pub fn show(
    _ui: &mut egui::Ui,
    _state: &mut PlannerState,
    _project: &mut Project,
    _thumbs: &mut ThumbCache,
    _palette: &Palette,
    _undo: &mut dyn FnMut(&Project),
) -> PlannerResponse {
    todo!("ui::planner::show")
}
