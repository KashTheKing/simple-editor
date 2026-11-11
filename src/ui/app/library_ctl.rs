//! ---- ws:online-library ----
//! The online library, App side: catalogue refresh, downloads, apply.

use super::*;

#[derive(Default)]
pub struct OnlineState {}

pub(super) fn act(_app: &mut App, _a: Action) -> bool {
    false
}

pub(super) fn tick(_app: &mut App, _ctx: &egui::Context) {}

pub(super) fn windows(_app: &mut App, _ctx: &egui::Context) {}
