//! Preview panel: the rendered frame (letterboxed, black bars), a selection outline for the selected
//! visual clip (drag to move = edits clip.x / clip.y at the playhead time via `Animated::set_at`, calling
//! `undo` once at drag start), and the transport bar: |◀  ◀◀(prev cut)  ◀(step)  ▶/⏸  ▶(step)  ▶▶  ▶|
//! plus timecode / duration, In/Out buttons and the in/out times.

use crate::media::Frame;
use crate::model::{Id, Project};
use crate::theme::Palette;
use eframe::egui;
use std::sync::Arc;

#[derive(Default)]
pub struct PreviewState {
    pub texture: Option<egui::TextureHandle>,
}

pub struct PreviewCtx<'a> {
    pub project: &'a mut Project,
    pub selection: &'a [Id],
    pub playhead: f64,
    pub playing: bool,
    pub palette: &'a Palette,
    pub undo: &'a mut dyn FnMut(&Project),
    /// A newly rendered frame to upload this update (None = keep the current texture).
    pub frame: Option<Arc<Frame>>,
}

#[derive(Default)]
pub struct PreviewResponse {
    pub seek: Option<f64>,
    pub toggle_play: bool,
    pub stop: bool,
    /// ±1 frame step.
    pub step: i32,
    pub prev_cut: bool,
    pub next_cut: bool,
    pub go_start: bool,
    pub go_end: bool,
    pub mark_in: bool,
    pub mark_out: bool,
    pub clear_in_out: bool,
    pub edited: bool,
    /// Pixel size available for the video image (for Player::set_canvas).
    pub canvas: (u32, u32),
}

pub fn show(_ui: &mut egui::Ui, _state: &mut PreviewState, _c: PreviewCtx<'_>) -> PreviewResponse {
    todo!("ui::preview::show")
}
