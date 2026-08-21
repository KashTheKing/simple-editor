//! Left panel: tabs "Library" (project assets: name, kind, duration, size; select, double-click or "Add"
//! → timeline at the playhead; right-click → Remove; drag source with `DragPayload::Asset`) and "Recent"
//! (settings.recent_assets across all projects: double-click / "Open" → `open_paths`; drag source with
//! `DragPayload::Path`; "Clear"). An "Import…" button at the top of Library.

use crate::model::{Id, Project};
use crate::settings::Settings;
use crate::theme::Palette;
use eframe::egui;
use std::path::PathBuf;

#[derive(Default)]
pub struct LibraryState {
    /// 0 = Library, 1 = Recent
    pub tab: usize,
    pub selected: Option<Id>,
}

#[derive(Default)]
pub struct LibraryResponse {
    pub import: bool,
    pub add_to_timeline: Vec<Id>,
    /// Recent files to import into the library (and select).
    pub open_paths: Vec<PathBuf>,
    pub remove: Vec<Id>,
    pub clear_recent: bool,
}

pub fn show(
    _ui: &mut egui::Ui,
    _state: &mut LibraryState,
    _project: &Project,
    _settings: &Settings,
    _palette: &Palette,
) -> LibraryResponse {
    todo!("ui::library::show")
}
