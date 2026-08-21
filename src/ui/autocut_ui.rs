//! Auto-cut pane (non-blocking; the timeline stays usable while it is open). Works on the selected audio
//! clips (a selected video clip uses its linked audio). Controls: threshold (dBFS slider -80..0),
//! min silence, min speech, padding (DragValues), "Keep: loud parts / quiet parts" toggle, "Ripple (close
//! gaps)" checkbox. It shows the detected segments live (count + total kept seconds) and publishes them to
//! `overlay` (timeline time ranges to KEEP, per selected clip) so the timeline can shade them; buttons:
//! "Split only" (cuts at the boundaries, removes nothing), "Apply" (Project::auto_cut with the cuts and the
//! quiet ranges; linked video follows), "Clear". Peaks come from WaveformCache (None while computing →
//! "analysing…"); detection uses engine::autocut. Undo once per Apply/Split. Returns what changed.

use crate::engine::autocut::AutoCutParams;
use crate::media::waveform::WaveformCache;
use crate::model::{Id, Project};
use crate::theme::Palette;
use eframe::egui;

pub struct AutoCutState {
    pub params: AutoCutParams,
    pub keep_quiet: bool,
    pub ripple: bool,
    /// Timeline ranges to keep (shaded by the timeline), recomputed each frame from the selection.
    pub overlay: Vec<(f64, f64)>,
}

impl Default for AutoCutState {
    fn default() -> Self {
        Self { params: AutoCutParams::default(), keep_quiet: false, ripple: true, overlay: Vec::new() }
    }
}

pub fn show(
    _ui: &mut egui::Ui,
    _state: &mut AutoCutState,
    _project: &mut Project,
    _selection: &[Id],
    _waveforms: &mut WaveformCache,
    _palette: &Palette,
    _undo: &mut dyn FnMut(&Project),
) -> bool {
    todo!("ui::autocut_ui::show")
}
