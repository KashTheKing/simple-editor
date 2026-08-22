//! "Paste Attributes" window (Ctrl+Alt+V): a checkbox per `AttrSet` field, All / None buttons, a line
//! saying what is on the clipboard ("from 'clip.mp4'") and how many clips will change. Non-modal, like
//! every other window. Returns Some(set) when the user confirms; the app then calls
//! `Project::paste_attributes` (one undo step).

use crate::model::AttrSet;
use eframe::egui;

#[derive(Default)]
pub struct PasteUi {
    pub open: bool,
    pub set: AttrSet,
}

pub fn show(_ctx: &egui::Context, _state: &mut PasteUi, _source_name: &str, _targets: usize) -> Option<AttrSet> {
    todo!("ui::paste_ui::show")
}
