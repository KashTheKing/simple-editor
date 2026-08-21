//! Effects panel. Top: the catalogue (EffectKind::ALL as buttons / a list; click = add to every selected
//! visual clip, with undo). Below: the FIRST selected clip's effect stack, in order: for each effect a
//! header row (enabled checkbox, name, ▲ ▼ reorder, ✕ remove) and its parameters from
//! `effect.specs()`: label + DragValue (range from ParamSpec, speed ≈ (max-min)/200) + "◆" keyframe toggle
//! at clip-local playhead time (Animated::toggle_key / set_at, highlighted when a key exists) + "✕ keys".
//! Tint shows a colour button bound to the R/G/B params as well. Every change → undo once per gesture
//! (same `edit_start` rule as the inspector) then returns true.

use crate::model::{Id, Project};
use crate::theme::Palette;
use eframe::egui;

pub fn show(
    _ui: &mut egui::Ui,
    _project: &mut Project,
    _selection: &[Id],
    _playhead: f64,
    _palette: &Palette,
    _undo: &mut dyn FnMut(&Project),
) -> bool {
    todo!("ui::effects_ui::show")
}
