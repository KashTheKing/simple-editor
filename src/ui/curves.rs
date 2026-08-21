//! Curve (graph) editor for the FIRST selected clip: x = clip-local time over [0, duration], y = value.
//! Left: a list of the clip's keyframeable properties (`Clip::props_mut` names + "Volume"/"Pan" +
//! "<effect>: <param>" for effect params) with a colour swatch and a checkbox to show/hide each curve
//! (auto-scaled per property; the selected property's scale drives the y axis labels). Right: the graph —
//! curves drawn with the eased segments (sample `Animated::at` every ~2 px), keys as diamonds (accent when
//! selected), playhead line (click on the ruler strip seeks → `seeked`), constant (unkeyed) properties
//! shown as a flat dashed line. Interaction: drag a key horizontally/vertically (time clamped to the clip,
//! vertical changes the value — `Animated::move_key` + `keys[i].v`; undo once at drag start), click a key
//! to select it, Delete removes it, double-click on empty graph area adds a key for the active property at
//! that (t, v), right-click a key → easing menu (Ease::ALL) + Delete; Ctrl+Scroll zooms the time axis,
//! Shift+Scroll pans. Returns what changed.

use crate::model::{Id, Project};
use crate::theme::Palette;
use eframe::egui;

#[derive(Default)]
pub struct CurvesState {
    /// Index of the active property in the panel's property list.
    pub active: usize,
    /// Hidden curves (property indices).
    pub hidden: Vec<usize>,
    /// Time zoom/pan of the graph (seconds at the left edge, px per second; 0 = fit the clip).
    pub scroll_x: f64,
    pub zoom: f32,
}

#[derive(Default)]
pub struct CurvesResponse {
    pub edited: bool,
    pub seeked: bool,
}

pub fn show(
    _ui: &mut egui::Ui,
    _state: &mut CurvesState,
    _project: &mut Project,
    _selection: &[Id],
    _playhead: &mut f64,
    _palette: &Palette,
    _undo: &mut dyn FnMut(&Project),
) -> CurvesResponse {
    todo!("ui::curves::show")
}
