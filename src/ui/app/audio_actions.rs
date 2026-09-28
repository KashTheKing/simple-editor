//! ---- ws:audio-analysis ----
//! ACT_HANDLERS row for the 5 unbound Actions this workstream declares in hotkeys.rs (DetectBeats,
//! SplitAtBeats, AutoDuck, Normalize, MatchLoudness): calls the same `engine::analysis` fns the Auto-cut
//! pane's buttons (autocut_ui.rs) and the MCP tools (tools_audio.rs) call, so no entry point ever
//! re-derives the onset/duck/normalize math. Beat markers fire `marker_added` via
//! `App::fire_markers_added` — the same shared fn every other marker-creating call site in this
//! workstream uses. AutoDuck reads its music/dialogue picks from the Auto-cut pane's Duck section state
//! (per the plan: "Uses the Duck section state in the Auto-cut pane"); Normalize/MatchLoudness act on
//! the current clip selection directly.

use super::tools_helpers::asset_peaks;
use super::*;
use crate::engine::analysis::{self, NormMode};

pub fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::DetectBeats => {
            let targets = crate::ui::autocut_ui::audio_targets(&app.project, &app.selection);
            if targets.is_empty() {
                app.toast("Select an audio clip (or a video clip with linked audio)");
                return true;
            }
            let before = app.project.to_json();
            let sensitivity = app.settings.beat_thr;
            let ids = {
                let App { project, waveforms, .. } = app;
                let (per_clip, _bpm) = analysis::detect_beats(project, &targets, 0.25, sensitivity, &mut asset_peaks(waveforms));
                analysis::beat_markers(project, &per_clip)
            };
            if ids.is_empty() {
                app.toast("No beats found");
                return true;
            }
            app.push_undo_labeled(before, "Detect Beats");
            app.fire_markers_added(&ids);
            app.after_edit();
            true
        }
        Action::SplitAtBeats => {
            let targets = crate::ui::autocut_ui::audio_targets(&app.project, &app.selection);
            if targets.is_empty() {
                app.toast("Select an audio clip (or a video clip with linked audio)");
                return true;
            }
            let before = app.project.to_json();
            let sensitivity = app.settings.beat_thr;
            let n = {
                let App { project, waveforms, .. } = app;
                let (per_clip, _bpm) = analysis::detect_beats(project, &targets, 0.25, sensitivity, &mut asset_peaks(waveforms));
                analysis::split_beats(project, &per_clip)
            };
            if n == 0 {
                app.toast("No beats found");
                return true;
            }
            app.push_undo_labeled(before, "Split at Beats");
            app.after_edit();
            true
        }
        Action::AutoDuck => {
            let (music, dialogue) = (app.autocut.duck.music, app.autocut.duck.dialogue.clone());
            let Some(music) = music else {
                app.toast("Pick a music clip in the Auto-cut pane's Duck section first");
                return true;
            };
            if dialogue.is_empty() {
                app.toast("Pick at least one dialogue clip in the Auto-cut pane's Duck section first");
                return true;
            }
            let (depth_db, ramp_s) = (app.autocut.duck.depth_db as f64, app.autocut.duck.ramp_ms as f64 / 1000.0);
            let dialogue: Vec<Id> = dialogue.into_iter().collect();
            let before = app.project.to_json();
            let n = {
                let App { project, waveforms, .. } = app;
                analysis::duck(project, music, &dialogue, depth_db, ramp_s, &mut asset_peaks(waveforms))
            };
            if n == 0 {
                app.toast("No speech found to duck under");
                return true;
            }
            app.push_undo_labeled(before, "Duck Music under Dialogue");
            app.after_edit();
            true
        }
        Action::Normalize => {
            if app.selection.is_empty() {
                app.toast("Select one or more audio clips first");
                return true;
            }
            let ids = app.selection.clone();
            let before = app.project.to_json();
            let n = {
                let App { project, waveforms, .. } = app;
                analysis::normalize(project, &ids, -1.0, NormMode::Peak, &mut asset_peaks(waveforms))
            };
            if n == 0 {
                app.toast("Nothing to normalize (already keyframed, or peaks still computing)");
                return true;
            }
            app.push_undo_labeled(before, "Normalize Selection");
            app.after_edit();
            true
        }
        Action::MatchLoudness => {
            if app.selection.len() < 2 {
                app.toast("Select at least 2 clips first");
                return true;
            }
            let ids = app.selection.clone();
            let before = app.project.to_json();
            let n = {
                let App { project, waveforms, .. } = app;
                analysis::match_loudness(project, &ids, &mut asset_peaks(waveforms))
            };
            if n == 0 {
                app.toast("Nothing to match (already keyframed, or peaks still computing)");
                return true;
            }
            app.push_undo_labeled(before, "Match Loudness across Selection");
            app.after_edit();
            true
        }
        _ => false,
    }
}
