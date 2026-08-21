//! Inspector panel. Nothing selected → Project settings (name, width, height, fps). One clip selected →
//! clip name, enabled, timing (start / duration / source in, read-only or DragValue), and:
//!  * visual clips: Position X/Y, Scale, Rotation, Opacity — each row = DragValue + "◆" keyframe toggle
//!    (`Animated::toggle_key(clip.local(playhead))`, highlighted when a key exists at the playhead) +
//!    "clear keys"; edits go through `Animated::set_at(local_t, v)` so keyframed props get keys; Blend mode combo.
//!  * audio clips: Volume (dB slider mapped to linear gain) with the same keyframe controls.
//!  * text clips: multiline text, font family combo (`fonts`), size, bold/italic, colour pickers (fill,
//!    outline, shadow, box), outline width, shadow on/off + x/y/blur, alignment, line/letter spacing, box padding.
//! Call `undo(project)` once per gesture (on `drag_started()` / first `changed()` of a widget) before mutating.
//! Returns true if the project changed.

use crate::model::{Id, Project};
use crate::theme::Palette;
use eframe::egui;

pub fn show(
    _ui: &mut egui::Ui,
    _project: &mut Project,
    _selection: &[Id],
    _playhead: f64,
    _fonts: &[String],
    _palette: &Palette,
    _undo: &mut dyn FnMut(&Project),
) -> bool {
    todo!("ui::inspector::show")
}
