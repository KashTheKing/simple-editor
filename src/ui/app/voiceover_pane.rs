//! ---- ws:panels-multiscreen ----
//! PANE_DRAWERS glue for the Voiceover pane (`ui::voiceover_ui`): starts / stops takes through the
//! existing `start_voiceover` / `stop_voiceover` / `drop_last_take` (jobs.rs), seeks to a take and deletes
//! one as an undo step. The level meter is fed by `App::update`'s capture tick (`wav_tail_peak`).

use super::*;
use crate::ui::voiceover_ui;

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::Voiceover {
        return false;
    }
    let mut devices = app.audio_inputs();
    // ws:page-switch-freeze: while the list is still being probed the picked input stays on show
    if app.audio_inputs_pending() && !app.settings.voice_device.is_empty() {
        devices.push((app.settings.voice_device.clone(), false));
    }
    let takes = voiceover_ui::takes(&app.project);
    let r = {
        let App { capture_ui, settings, voice_rec, playhead, palette, .. } = app;
        voiceover_ui::show(ui, capture_ui, settings, &devices, voice_rec.is_some(), *playhead, &takes, palette)
    };
    if r.stop {
        app.stop_voiceover();
    }
    if let Some(o) = r.start {
        if r.retake {
            app.drop_last_take();
        }
        app.start_voiceover(o, app.capture_ui.from_playhead);
    }
    if let Some(t) = r.seek {
        app.player.pause();
        app.seek(t);
    }
    if let Some(id) = r.delete {
        app.push_undo();
        app.project.delete_clips(&[id], false);
        app.after_edit();
    }
    // ---- ws:addon-voice ----
    ui.separator();
    egui::CollapsingHeader::new("Speech").id_salt("vo_speech").default_open(true).show(ui, |ui| super::speech_ctl::draw(app, ui));
    true
}
