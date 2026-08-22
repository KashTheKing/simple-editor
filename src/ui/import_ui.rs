//! Import report window: after `engine::import::import_file`, show what came through and what did not —
//! a summary line (clips / tracks / missing media), then a scrollable table of `Issue`s grouped by level
//! (Ok / Warning / Skipped) with the subject and detail. Buttons: "Use this project" (replaces the current
//! one, with the usual unsaved-changes prompt), "Copy report" (markdown) and Close. Non-modal.

use crate::engine::import::ImportReport;
use crate::theme::Palette;
use eframe::egui;

#[derive(Default)]
pub struct ImportUi {
    pub open: bool,
    pub report: Option<ImportReport>,
}

/// True when the user accepted the imported project (the app swaps it in).
pub fn show(_ctx: &egui::Context, _state: &mut ImportUi, _palette: &Palette) -> bool {
    todo!("ui::import_ui::show")
}
