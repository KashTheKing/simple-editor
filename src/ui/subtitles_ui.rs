//! Subtitles panel. Toolbar: "Add at playhead" (cue [playhead, playhead+2 s) with the text "Subtitle",
//! selected + text field focused), "Import…" (rfd: srt/vtt → engine::subtitles::parse → replace or append
//! after a yes/no), "Export SRT…" / "Export VTT…" (engine::subtitles::to_srt/to_vtt), "Burn in" checkbox
//! (project.show_subtitles), and a "Style" collapsing section (font combo from `fonts`, size, colour,
//! outline width/colour, background box colour, margin from bottom = project.subtitle_margin).
//! Below: the cue list (egui::Grid / ScrollArea): start and end as editable timecode-ish DragValues in
//! seconds (3 decimals, end ≥ start + 0.1, keep the list sorted via Project::sort_cues), a multiline text
//! field, "▶" (seek to the cue → `seeked`), "✕" delete; the cue containing the playhead is highlighted; a
//! "Split at playhead" button on the highlighted cue. Undo once per gesture (same edit_start rule as the
//! inspector); returns what changed.

use crate::model::Project;
use crate::theme::Palette;
use eframe::egui;

#[derive(Default)]
pub struct SubtitlesState {
    pub selected: Option<crate::model::Id>,
    pub show_style: bool,
}

#[derive(Default)]
pub struct SubtitlesResponse {
    pub edited: bool,
    pub seeked: bool,
}

pub fn show(
    _ui: &mut egui::Ui,
    _state: &mut SubtitlesState,
    _project: &mut Project,
    _playhead: &mut f64,
    _fonts: &[String],
    _palette: &Palette,
    _undo: &mut dyn FnMut(&Project),
) -> SubtitlesResponse {
    todo!("ui::subtitles_ui::show")
}
