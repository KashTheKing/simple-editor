//! ---- ws:panels-multiscreen ----
//! PANE_DRAWERS glue for the Transcript pane (`subtitles_ui::show_transcript_pane`): transcribe a clip,
//! edit its words, captions from the transcript. Was the Subtitles pane's Transcript tab + Transcribe
//! section and the View transcript window; the actions that opened those (`transcript_ctl::act`) bring
//! this pane up instead.

use super::*;

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::Transcript {
        return false;
    }
    let resp = {
        let App { project, playhead, selection, undo, redo, subtitles_ui: st, palette, .. } = app;
        let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
        subtitles_ui::show_transcript_pane(ui, st, project, playhead, selection, palette, &mut push)
    };
    if resp.seeked {
        app.player.pause();
        app.player.seek(app.playhead);
    }
    if resp.edited {
        app.after_edit();
    }
    true
}

/// Show the Transcript pane on `clip`'s words (the View transcript / Transcribe actions).
pub(super) fn open(app: &mut App, clip: Option<Id>, setup: bool) {
    if let Some(c) = clip {
        app.subtitles_ui.transcript.clip = Some(c);
    }
    if setup {
        app.subtitles_ui.transcribe.open = true;
    }
    app.layout.reveal(Pane::Transcript);
    app.layout_dirty = true;
}
