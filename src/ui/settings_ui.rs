//! Settings window (egui::Window, closable). Tabs:
//!  * General: theme (system/dark/light), decoder (auto/mf/ffmpeg), ffmpeg folder (text + Browse…, shows
//!    whether ffmpeg/ffprobe were found), preview max width, snapping, confirm overwrite,
//!    "Edit with Simple Editor" context menu checkbox (install/uninstall via crate::contextmenu).
//!  * Export: encoder combo (auto + detected encoders filtered to h264/hevc/vp9/av1 families), CRF, preset.
//!  * Hotkeys: table of every Action (label, current binding, "Rebind" → waits for the next key press with
//!    modifiers (Escape cancels, Backspace/Delete unbinds), "Reset"), plus "Reset all". Conflicts are
//!    resolved by unbinding the other action (Hotkeys::set). Note the fixed mouse modifiers.
//! Returns true when settings changed (caller saves + re-applies theme/backend/hotkeys).

use crate::hotkeys::{Action, Hotkeys};
use crate::settings::Settings;
use crate::theme::Palette;
use eframe::egui;

#[derive(Default)]
pub struct SettingsUi {
    pub open: bool,
    pub tab: usize,
    pub rebinding: Option<Action>,
}

pub fn show(
    _ctx: &egui::Context,
    _state: &mut SettingsUi,
    _settings: &mut Settings,
    _hotkeys: &mut Hotkeys,
    _encoders: &[String],
    _palette: &Palette,
) -> bool {
    todo!("ui::settings_ui::show")
}
