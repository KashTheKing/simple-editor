//! Subtitles panel. Two tabs - Cues and Transcript - and one header row each. The Cues row: "+ Add" (cue
//! [playhead, playhead+2 s) with the text "Subtitle", selected + text field focused), "Transcribe…" and
//! "Style ▾" (each opens its section above the cues) and ⋯ (also the cue list's empty-space right-click):
//! Import… (the app's one import path, shared with the timeline's subtitle lane), Export SRT… /
//! Export VTT… (engine::subtitles::to_srt/to_vtt), Open folder (the app writes the .srt sidecar and
//! opens the folder), Burn in (project.show_subtitles), To text clips (Project::cues_to_text_clips -
//! editable Text clips on a "Subtitles" track), Delete in range (the In/Out range) and Clear all.
//! The Style section: font combo from `fonts`, size, colour, outline width/colour, background box
//! colour, margin from bottom = project.subtitle_margin.
//! Below: the cue list - one row per cue: start and end as editable DragValues in seconds (3 decimals,
//! end ≥ start + 0.1, keep the list sorted via Project::sort_cues) and the text; the cue containing the
//! playhead is highlighted. Click a row to select it and seek there (Ctrl/Shift+click toggles it in the
//! multi-selection, Shift+drag sweeps rows in); its right-click: Play cue (→ `seeked` + `play`), Split at
//! playhead, Convert to text clip(s), Delete - on every selected cue when the row is part of the
//! selection. Undo once per gesture (same edit_start rule as the inspector); returns what changed.
//!
//! The "Transcribe" section drives `engine::transcribe`: pick a whisper.cpp model (its download size is
//! named before the click and the download shows a progress bar), and "Transcribe & generate subtitles"
//! hands the selected clip to the app (`want`), whose `transcribe_clip` job finishes into cues whether or
//! not this pane is still open (`transcript_ctl::tick`). Opening the section starts nothing. The raw
//! word timings are kept, so "Regenerate cues" rebuilds the cues with new grouping knobs (pause split,
//! punctuation, max words/chars) without re-transcribing; a `--prompt` field feeds whisper vocabulary
//! hints. Underneath it,
//! the double-take detector lists the lines that were said more than once - "Mark on timeline" drops a
//! marker per flubbed take (described by what was said) and "Cut the duplicates" ripples every take but
//! the last one out, dragging the cues, the markers and the transcript along with the cut. With no model
//! and no whisper.exe the section only ever explains what to install.
//!
//! ---- ws:transcript-captions ----
//! A word-timed run also persists its words into `Project.transcripts` (`Project::set_transcript`),
//! so they survive a reopen and feed the Transcript tab (`ui::transcript_ui`:
//! click = seek, select + Delete = ripple cut through `Project::cut_word_ranges`, filler removal with
//! Mark-instead first, word search across every transcribed clip). The double-take cutter now goes
//! through that same `cut_word_ranges`. "Get captions" is the one-click entry: it names the model's
//! size before any network call and never runs at startup.

use crate::engine::export::Progress;
use crate::engine::transcribe::{self, Segment};
use crate::hotkeys::Action;
use crate::model::{Id, Project};
use crate::theme::Palette;
use crate::ui::tools::{glyph_text_button, Glyph};
use crate::ui::transcript_ui;
use crate::ui::{edit_start, menu, once};
use eframe::egui::{self, Button, DragValue, Response, Sense, Slider};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// Minimum cue length (end ≥ start + this).
const MIN_CUE: f64 = 0.1;

/// How long a pause may sit between one take and its retake before they are unrelated lines.
const TAKE_WINDOW: f64 = 20.0;

/// Speech-to-text and the double-take detector (the "Transcribe" section).
pub struct TranscribeState {
    pub open: bool,
    /// Index into `transcribe::MODELS`.
    pub model: usize,
    pub language: String,
    pub max_chars: usize,
    /// Wrapped lines per cue, joined with newlines (1 = one-liners, 2 = the usual two-line subs).
    pub lines: usize,
    pub min_dur: f64,
    /// "Transcribe & generate subtitles" was pressed for this clip - drained by `transcript_ctl::tick`.
    pub want: Option<Id>,
    /// Vocabulary/style hints passed to whisper (`--prompt`).
    pub prompt: String,
    /// Sentence grouping for word-timed runs (gap, punctuation, max words/chars) - regenerate-time knobs.
    pub group: transcribe::GroupOpts,
    /// Raw one-word timings of the last word-timed run (timeline seconds) - what "Regenerate" regroups.
    pub raw_words: Vec<(f64, f64, String)>,
    /// Cues added by the last generate, so a regenerate replaces them instead of stacking duplicates.
    pub generated: Vec<Id>,
    /// Double-take similarity, 0.5..=1.0.
    pub threshold: f32,
    /// Transcript of the last run, in timeline seconds.
    pub segments: Vec<Segment>,
    /// Repeated takes over `segments` (each group keeps its last member).
    pub groups: Vec<Vec<usize>>,
    pub status: String,
    /// Clip the transcript came from, and its source -> timeline (offset, scale).
    clip: Option<Id>,
    map: (f64, f64),
    /// Markers dropped by "Mark on timeline", so a re-mark or a cut can take them away again.
    marks: Vec<Id>,
    download: Option<Arc<Progress>>,
    /// whisper binary, looked up once (the lookup stats PATH) - "Re-check" after installing it.
    exe: Option<Option<PathBuf>>,
}

impl Default for TranscribeState {
    fn default() -> Self {
        Self {
            open: false,
            model: transcribe::default_model(),
            language: "auto".into(),
            max_chars: 42,
            lines: 1,
            min_dur: 1.0,
            want: None,
            prompt: String::new(),
            group: transcribe::GroupOpts::default(),
            raw_words: Vec::new(),
            generated: Vec::new(),
            threshold: 0.85,
            segments: Vec::new(),
            groups: Vec::new(),
            status: String::new(),
            clip: None,
            map: (0.0, 1.0),
            marks: Vec::new(),
            download: None,
            exe: None,
        }
    }
}

// ---- ws:transcript-captions ----
impl TranscribeState {
    /// The model the section has picked: `(name, file, MB)` from `transcribe::MODELS`.
    pub fn model(&self) -> (&'static str, &'static str, u32) {
        transcribe::MODELS[self.model.min(transcribe::MODELS.len() - 1)]
    }
    /// Take over a word-timed transcript produced outside this section (the clip menu's
    /// "Transcribe…", the `transcribe.run`/`media.transcribe` tools): leaves exactly the state the
    /// section's own button would, so "Regenerate cues" (replacing `generated`) and the double-take
    /// list work on it without a second run.
    pub fn adopt(&mut self, clip: Id, map: (f64, f64), words: Vec<(f64, f64, String)>, generated: Vec<Id>) {
        self.clip = Some(clip);
        self.map = map;
        self.marks.clear();
        self.generated = generated;
        self.status.clear();
        self.segments = transcribe::group_words(&words, &self.group);
        self.groups = transcribe::duplicate_takes(&self.segments, self.threshold, TAKE_WINDOW);
        self.raw_words = words;
    }
}

#[derive(Default)]
pub struct SubtitlesState {
    pub selected: Option<crate::model::Id>,
    /// Multi-selected cues (Ctrl/Shift+click, Shift+drag): what a row's right-click acts on.
    pub checked: std::collections::HashSet<Id>,
    pub show_style: bool,
    /// Cue whose text field should grab focus (set by "+ Add").
    pub focus: Option<Id>,
    /// Cue whose text is being typed into and already has its undo step for this visit.
    typing: Option<Id>,
    pub transcribe: TranscribeState,
    // ---- ws:transcript-captions ----
    /// The Transcript tab is showing instead of the cues (`Action::ToggleTranscript` flips it).
    pub show_transcript: bool,
    pub transcript: transcript_ui::TranscriptUiState,
}

#[derive(Default)]
pub struct SubtitlesResponse {
    pub edited: bool,
    pub seeked: bool,
    /// A cue's right-click ▸ Play cue: seek there and start playback.
    pub play: bool,
    /// "Open folder" - the app writes the .srt sidecar next to the project and opens it in Explorer.
    pub open_folder: bool,
    /// ---- ws:forgiveness ----
    /// "Clear all" ran inline (no confirm dialog) - the app toasts an Undo.
    pub cleared_subtitles: bool,
    /// ⋯ ▸ Import…: the app runs the one import path the timeline's subtitle lane uses too
    /// (`timeline_pane::import_subtitles` - file dialog, parse, replace-or-confirm).
    pub import: bool,
}

/// "+ Add": a 2 s cue starting at the playhead.
fn add_at(project: &mut Project, playhead: f64) -> Id {
    project.add_cue(playhead, playhead + 2.0, "Subtitle")
}

/// Import parsed cues, replacing or appending. `pub(crate)`: also called by
/// `confirm::apply_to_project` (ws:forgiveness) to resolve `ConfirmAction::ReplaceSubtitles` - reused
/// rather than duplicated so the id-allocation (`Project::add_cue`) stays in one place.
pub(crate) fn apply_import(project: &mut Project, cues: &[(f64, f64, String)], replace: bool) {
    if replace {
        project.subtitles.clear();
    }
    for (s, e, t) in cues {
        project.add_cue(*s, *e, t.clone());
    }
}

/// The pane's two tabs, drawn at the start of whichever header row is showing.
fn tabs(ui: &mut egui::Ui, transcript: &mut bool) {
    ui.selectable_value(transcript, false, "Cues");
    ui.selectable_value(transcript, true, "Transcript");
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut SubtitlesState,
    project: &mut Project,
    playhead: &mut f64,
    selection: &[Id],
    fonts: &[String],
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> SubtitlesResponse {
    let mut resp = SubtitlesResponse::default();
    let mut undone = false;

    // ---- ws:transcript-captions ---- the Transcript tab (`Action::ToggleTranscript` flips to it)
    if state.show_transcript {
        // a selectable word label would take the right-click meant for the word menu
        ui.style_mut().interaction.selectable_labels = false;
        let SubtitlesState { show_transcript, transcript, .. } = state;
        let mut lead = |ui: &mut egui::Ui| tabs(ui, show_transcript);
        let r =
            transcript_ui::show(ui, &mut lead, transcript, project, playhead, selection, palette, &mut undone, undo);
        resp.edited |= r.edited;
        resp.seeked |= r.seeked;
        if r.cut > 0 {
            // the tab's own words follow the cut (Project.transcripts already did); the Transcribe
            // section's copy for "Regenerate cues" follows too
            if let Some(tr) = state.transcribe.clip.and_then(|c| project.transcript(c)) {
                if !state.transcribe.raw_words.is_empty() {
                    state.transcribe.raw_words = tr.words.clone();
                }
            }
        }
        return resp;
    }

    let ph = *playhead;
    let bg = crate::ui::markers_ui::menu_area(ui);
    // ---- the one header row: Cues | Transcript · + Add · Transcribe… · Style ▾ · ⋯ ----
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0; // six controls in a ~350 pt side pane
        tabs(ui, &mut state.show_transcript);
        ui.add_space(6.0);
        let key = menu::shortcut(Action::AddSubtitle);
        let tip = if key.is_empty() {
            "Add a cue at the playhead".to_string()
        } else {
            format!("Add a cue at the playhead ({key})")
        };
        if ui.button("+ Add").on_hover_text(tip).clicked() {
            once(&mut undone, undo, project);
            let id = add_at(project, ph);
            state.selected = Some(id);
            state.focus = Some(id);
            resp.edited = true;
        }
        ui.toggle_value(&mut state.transcribe.open, "Transcribe…")
            .on_hover_text("Speech to text: model, language and the cue grouping, then the double-take finder");
        // right-aligned, so ⋯ is never the one a narrow pane cuts off
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            menu::button(ui, "⋯", |ui| more_menu(ui, state, project, &mut undone, undo, &mut resp))
                .response
                .on_hover_text("More");
            let arrow = if state.show_style { "▴" } else { "▾" };
            ui.toggle_value(&mut state.show_style, format!("Style {arrow}"))
                .on_hover_text("The caption look: font, size, colours, box");
        });
    });

    let mut resort = false;
    let mut row_op: Option<(CueOp, Vec<Id>)> = None;
    let (mods, primary_down) = ui.input(|i| (i.modifiers, i.pointer.primary_down()));
    // the open sections scroll with the cue list: stacked above its own scroll area they squeezed it
    // to nothing
    egui::ScrollArea::vertical().id_salt("subtitles_body").auto_shrink(false).show(ui, |ui| {
        if state.show_style {
            style_section(ui, project, fonts, &mut undone, undo, &mut resp);
            ui.separator();
        }
        if state.transcribe.open {
            transcribe_section(ui, &mut state.transcribe, project, selection, &mut undone, undo, &mut resp);
            ui.separator();
        }
        for i in 0..project.subtitles.len() {
            let (id, start, end) = {
                let c = &project.subtitles[i];
                (c.id, c.start, c.end)
            };
            let active = ph >= start && ph < end;
            let picked = state.checked.contains(&id) || state.selected == Some(id);
            let fill = if active {
                palette.selection.gamma_multiply(0.25)
            } else if picked {
                palette.selection.gamma_multiply(0.12)
            } else {
                egui::Color32::TRANSPARENT
            };
            let row = egui::Frame::new().fill(fill).inner_margin(2.0).show(ui, |ui| {
                // the row senses clicks under its fields: click = select + seek, right-click = its menu
                ui.scope_builder(egui::UiBuilder::new().sense(Sense::click()), |ui| {
                    ui.horizontal(|ui| {
                        let mut v = start;
                        let s =
                            ui.add(DragValue::new(&mut v).range(0.0..=(end - MIN_CUE)).speed(0.05).fixed_decimals(3));
                        if edit_start(&s) {
                            once(&mut undone, undo, project);
                        }
                        if s.changed() {
                            project.subtitles[i].start = v.clamp(0.0, end - MIN_CUE);
                            resp.edited = true;
                        }
                        if s.drag_stopped() || (s.changed() && !s.dragged()) {
                            resort = true;
                        }
                        let mut v = end;
                        let e = ui.add(
                            DragValue::new(&mut v).range((start + MIN_CUE)..=86400.0).speed(0.05).fixed_decimals(3),
                        );
                        if edit_start(&e) {
                            once(&mut undone, undo, project);
                        }
                        if e.changed() {
                            project.subtitles[i].end = v.max(start + MIN_CUE);
                            resp.edited = true;
                        }
                        let mut text = project.subtitles[i].text.clone();
                        let t =
                            ui.add(egui::TextEdit::multiline(&mut text).desired_rows(1).desired_width(f32::INFINITY));
                        // One undo entry per visit to the field, taken at its first keystroke (not on focus:
                        // a right-click focuses the field too, and must not leave an empty undo step).
                        // "+ Add" focuses the new cue itself and has already pushed one.
                        if state.focus == Some(id) {
                            t.request_focus();
                            state.focus = None;
                            state.typing = Some(id);
                        }
                        if t.changed() && state.typing != Some(id) {
                            once(&mut undone, undo, project);
                            state.typing = Some(id);
                        }
                        if t.lost_focus() && state.typing == Some(id) {
                            state.typing = None;
                        }
                        if t.has_focus() {
                            state.selected = Some(id);
                        }
                        if t.changed() {
                            project.subtitles[i].text = text;
                            resp.edited = true;
                        }
                        #[cfg(test)]
                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("cue_text", id)), t.rect));
                        (t.clicked(), s.union(e).union(t))
                    })
                    .inner
                })
            });
            let scope = row.inner;
            let (text_clicked, fields) = scope.inner;
            let bg_r = scope.response;
            // Ctrl/Shift+click toggles a cue in the multi-selection; a plain click picks just it
            let toggle = mods.ctrl || mods.shift;
            if bg_r.clicked() || text_clicked {
                if toggle {
                    if !state.checked.remove(&id) {
                        state.checked.insert(id);
                    }
                } else {
                    state.checked = std::iter::once(id).collect();
                    state.selected = Some(id);
                }
            }
            if bg_r.clicked() && !toggle {
                *playhead = start;
                resp.seeked = true;
            }
            let menu_r = bg_r.union(fields);
            if menu_r.secondary_clicked() && !state.checked.contains(&id) {
                state.checked = std::iter::once(id).collect();
                state.selected = Some(id);
            }
            menu::context(&menu_r, |ui| {
                let targets: Vec<Id> =
                    if state.checked.contains(&id) { state.checked.iter().copied().collect() } else { vec![id] };
                let n = targets.len();
                if menu::row(ui, Some(Glyph::Play), "Play cue", "").clicked() {
                    row_op = Some((CueOp::Play, vec![id]));
                }
                let inside = ph > start + 0.05 && ph < end - 0.05;
                let r =
                    ui.add_enabled_ui(inside, |ui| menu::row(ui, Some(Glyph::Razor), "Split at playhead", "")).inner;
                if r.on_disabled_hover_text("Put the playhead inside this cue").clicked() {
                    row_op = Some((CueOp::Split, vec![id]));
                }
                let label = if n > 1 { format!("Convert {n} to text clips") } else { "Convert to text clip".into() };
                if menu::row(ui, Some(Glyph::Letter('T')), &label, "")
                    .on_hover_text("Editable Text clips on a \"Subtitles\" track")
                    .clicked()
                {
                    row_op = Some((CueOp::ToText, targets.clone()));
                }
                ui.separator();
                let label = if n > 1 { format!("Delete {n} cues") } else { "Delete".into() };
                if menu::row(ui, Some(Glyph::Cross), &label, "").clicked() {
                    row_op = Some((CueOp::Delete, targets.clone()));
                }
            });
            // Shift+drag over rows sweeps them into the selection (plain drags still edit the widgets)
            if mods.shift && primary_down && ui.rect_contains_pointer(row.response.rect) {
                state.checked.insert(id);
            }
        }
        if project.subtitles.is_empty() {
            ui.weak("No subtitles. \"+ Add\" one at the playhead, or right-click to import an .srt / .vtt file.");
        }
    });
    menu::context(&bg, |ui| more_menu(ui, state, project, &mut undone, undo, &mut resp));

    match row_op {
        Some((CueOp::Play, _)) | None => {}
        Some(_) => once(&mut undone, undo, project),
    }
    match row_op {
        Some((CueOp::Play, ids)) => {
            if let Some(c) = ids.first().and_then(|id| project.subtitles.iter().find(|c| c.id == *id)) {
                *playhead = c.start;
                state.selected = Some(c.id);
                resp.seeked = true;
                resp.play = true;
            }
        }
        Some((CueOp::Split, ids)) => {
            if let Some(&id) = ids.first() {
                if let Some(c) = project.subtitles.iter_mut().find(|c| c.id == id) {
                    let (end, text) = (c.end, c.text.clone());
                    c.end = ph;
                    let nid = project.add_cue(ph, end, text);
                    state.selected = Some(nid);
                    resp.edited = true;
                }
            }
        }
        Some((CueOp::ToText, ids)) => {
            project.cues_to_text_clips(Some(&ids));
            state.checked.retain(|c| !ids.contains(c));
            resp.edited = true;
        }
        Some((CueOp::Delete, ids)) => {
            for &id in &ids {
                project.remove_cue(id);
            }
            state.checked.retain(|c| !ids.contains(c));
            if state.selected.is_some_and(|s| ids.contains(&s)) {
                state.selected = None;
            }
            resp.edited = true;
        }
        None => {}
    }
    if resort {
        project.sort_cues();
    }
    resp
}

/// A cue row's right-click verbs, applied after the list is drawn.
enum CueOp {
    Play,
    Split,
    ToText,
    Delete,
}

/// The pane's ⋯ menu, also the cue list's empty-space right-click: files, burn-in and the bulk verbs.
fn more_menu(
    ui: &mut egui::Ui,
    state: &mut SubtitlesState,
    project: &mut Project,
    undone: &mut bool,
    undo: &mut dyn FnMut(&Project),
    resp: &mut SubtitlesResponse,
) {
    let any = !project.subtitles.is_empty();
    if menu::row(ui, Some(Glyph::ImportArrow), "Import…", "").clicked() {
        resp.import = true;
    }
    for (label, vtt) in [("Export SRT…", false), ("Export VTT…", true)] {
        if ui.add_enabled_ui(any, |ui| menu::row(ui, Some(Glyph::ExportArrow), label, "")).inner.clicked() {
            export_dialog(project, vtt);
        }
    }
    let r = menu::row(ui, Some(Glyph::Folder), "Open folder", "")
        .on_hover_text("Open the project's subtitle folder in Explorer");
    if r.clicked() {
        resp.open_folder = true;
    }
    ui.separator();
    let r = menu::check(ui, project.show_subtitles, "Burn in", "")
        .on_hover_text("Draw the cues onto the picture (and into exports)");
    if r.clicked() {
        once(undone, undo, project);
        project.show_subtitles = !project.show_subtitles;
        resp.edited = true;
    }
    let sel = state.checked.len();
    let label = if sel > 0 { format!("To text clips ({sel})") } else { "To text clips".into() };
    let r = ui.add_enabled_ui(any, |ui| menu::row(ui, Some(Glyph::Letter('T')), &label, "")).inner;
    if r.on_hover_text("Turn the selected cues (or all of them) into editable Text clips on a \"Subtitles\" track")
        .clicked()
    {
        once(undone, undo, project);
        let only: Vec<Id> = state.checked.iter().copied().collect();
        let n = project.cues_to_text_clips(if only.is_empty() { None } else { Some(&only) });
        state.checked.clear();
        resp.edited = n > 0;
    }
    let range = match (project.in_point, project.out_point) {
        (Some(a), Some(b)) if b > a => Some((a, b)),
        _ => None,
    };
    let r = ui.add_enabled_ui(any && range.is_some(), |ui| menu::row(ui, None, "Delete in range", "")).inner;
    if r.on_hover_text("Delete every cue that overlaps the In/Out range").clicked() {
        if let Some((a, b)) = range {
            once(undone, undo, project);
            project.subtitles.retain(|c| c.end <= a || c.start >= b);
            state.checked.retain(|id| project.subtitles.iter().any(|c| c.id == *id));
            resp.edited = true;
        }
    }
    // no confirm dialog: `once(...)` makes it a normal undoable edit - the app additionally toasts an
    // Undo button (resp.cleared_subtitles)
    if ui.add_enabled_ui(any, |ui| menu::row(ui, Some(Glyph::Cross), "Clear all", "")).inner.clicked() {
        once(undone, undo, project);
        project.subtitles.clear();
        state.checked.clear();
        resp.edited = true;
        resp.cleared_subtitles = true;
    }
}

fn style_section(
    ui: &mut egui::Ui,
    project: &mut Project,
    fonts: &[String],
    undone: &mut bool,
    undo: &mut dyn FnMut(&Project),
    resp: &mut SubtitlesResponse,
) {
    let mut style = project.subtitle_style.clone();
    let mut margin = project.subtitle_margin;
    let mut cont = (project.subtitle_cont_prefix.clone(), project.subtitle_cont_suffix.clone());
    let mut start = false;
    let mut changed = false;
    let note = |r: &Response, start: &mut bool, changed: &mut bool| {
        *start |= edit_start(r);
        *changed |= r.changed();
    };
    egui::Grid::new("subtitle_style").num_columns(2).show(ui, |ui| {
        ui.label("Font");
        egui::ComboBox::from_id_salt("sub_font").selected_text(style.font.clone()).show_ui(ui, |ui| {
            for f in fonts {
                note(&ui.selectable_value(&mut style.font, f.clone(), f), &mut start, &mut changed);
            }
        });
        ui.end_row();
        ui.label("Size");
        // ws:text-titles: style.size is now Animated - the project-wide caption style has no
        // keyframe UI of its own, so this just edits the constant value.
        note(&ui.add(DragValue::new(&mut style.size.value).range(8.0..=300.0)), &mut start, &mut changed);
        ui.end_row();
        ui.label("Colour");
        note(&ui.color_edit_button_srgba_unmultiplied(&mut style.color), &mut start, &mut changed);
        ui.end_row();
        ui.label("Outline");
        ui.horizontal(|ui| {
            note(&ui.color_edit_button_srgba_unmultiplied(&mut style.outline_color), &mut start, &mut changed);
            note(
                &ui.add(DragValue::new(&mut style.outline_width.value).range(0.0..=20.0).speed(0.1)),
                &mut start,
                &mut changed,
            );
        });
        ui.end_row();
        ui.label("Box");
        note(&ui.color_edit_button_srgba_unmultiplied(&mut style.box_color), &mut start, &mut changed);
        ui.end_row();
        ui.label("Margin");
        note(&ui.add(DragValue::new(&mut margin).range(0.0..=1000.0)), &mut start, &mut changed);
        ui.end_row();
        ui.label("Format");
        ui.horizontal(|ui| {
            note(&ui.toggle_value(&mut style.bold, "B"), &mut start, &mut changed);
            note(&ui.toggle_value(&mut style.italic, "I"), &mut start, &mut changed);
            for (v, lab) in [(0u8, "Left"), (1, "Center"), (2, "Right")] {
                note(&ui.selectable_value(&mut style.align, v, lab), &mut start, &mut changed);
            }
        });
        ui.end_row();
        ui.label("Line spacing");
        note(&ui.add(DragValue::new(&mut style.line_spacing).range(0.5..=3.0).speed(0.02)), &mut start, &mut changed);
        ui.end_row();
        ui.label("Letter spacing");
        note(
            &ui.add(DragValue::new(&mut style.letter_spacing.value).range(-5.0..=30.0).speed(0.1)),
            &mut start,
            &mut changed,
        );
        ui.end_row();
        ui.label("Shadow");
        ui.horizontal(|ui| {
            note(&ui.checkbox(&mut style.shadow, ""), &mut start, &mut changed);
            note(&ui.color_edit_button_srgba_unmultiplied(&mut style.shadow_color), &mut start, &mut changed);
        });
        ui.end_row();
        ui.label("Continuation").on_hover_text(
            "Added where a sentence is split across cues: the suffix ends the cut-off cue, the prefix \
             starts the next (e.g. suffix \" - \" for em-dashes). Applied by Transcribe / Regenerate cues.",
        );
        ui.horizontal(|ui| {
            note(
                &ui.add(egui::TextEdit::singleline(&mut cont.0).desired_width(50.0).hint_text("prefix")),
                &mut start,
                &mut changed,
            );
            note(
                &ui.add(egui::TextEdit::singleline(&mut cont.1).desired_width(50.0).hint_text("suffix")),
                &mut start,
                &mut changed,
            );
        });
        ui.end_row();
    });
    if start {
        once(undone, undo, project);
    }
    if changed {
        project.subtitle_style = style;
        project.subtitle_margin = margin;
        (project.subtitle_cont_prefix, project.subtitle_cont_suffix) = cont;
        resp.edited = true;
    }
}

/// The first selected clip that has footage behind it - the mapping itself lives in
/// `transcribe::target_for` (ws:transcript-captions), shared with the clip menu and the MCP tools.
fn target(project: &Project, selection: &[Id]) -> Option<transcribe::Target> {
    selection.iter().find_map(|&id| transcribe::target_for(project, id).ok())
}

/// A marker on every take "Cut the duplicates" would remove, named and described by what was said.
fn mark_dups(project: &mut Project, segs: &[Segment], groups: &[Vec<usize>]) -> Vec<Id> {
    let mut ids = Vec::new();
    for g in groups {
        for &i in &g[..g.len() - 1] {
            let Some(s) = segs.get(i) else { continue };
            let id = project.add_marker(s.start, transcribe::short_label(&s.text, 28));
            if let Some(m) = project.marker_mut(id) {
                m.duration = (s.end - s.start).max(0.0);
                m.note = s.text.clone();
            }
            ids.push(id);
        }
    }
    ids
}

/// Move one transcribed span through a ripple cut; false when the cut swallowed it.
fn ripple_segment(s: &mut Segment, ranges: &[(f64, f64)]) -> bool {
    let Some(a) = transcribe::ripple_time(s.start, ranges) else { return false };
    let b = transcribe::ripple_time(s.end, ranges).unwrap_or(a + (s.end - s.start));
    s.words.retain_mut(|w| match (transcribe::ripple_time(w.0, ranges), transcribe::ripple_time(w.1, ranges)) {
        (Some(x), Some(y)) => {
            (w.0, w.1) = (x, y);
            true
        }
        _ => false,
    });
    (s.start, s.end) = (a, b.max(a));
    true
}

/// Ripple every take but the last of each group out of the timeline, then drag the cues, our markers and
/// the transcript along with the cut. Returns how many clips went.
/// ws:transcript-captions: the cut itself (and the cue/marker/`Project.transcripts` ripple) is
/// `Project::cut_word_ranges` now - one path shared with filler removal, the Transcript section and MCP.
fn cut_dups(project: &mut Project, st: &mut TranscribeState) -> usize {
    let ranges = transcribe::dup_ranges(&st.segments, &st.groups);
    let (Some(clip), false) = (st.clip, ranges.is_empty()) else { return 0 };
    for id in st.marks.drain(..) {
        project.remove_marker(id);
    }
    let ripple = project.track_of(clip).and_then(|ti| project.tracks[ti].ripple).unwrap_or(false);
    let n = project.cut_word_ranges(clip, &ranges);
    if n == 0 {
        return 0;
    }
    if ripple {
        st.segments.retain_mut(|s| ripple_segment(s, &ranges));
    } else {
        st.segments.retain(|s| !ranges.iter().any(|&(a, b)| s.start >= a && s.start < b));
    }
    st.groups = transcribe::duplicate_takes(&st.segments, st.threshold, TAKE_WINDOW);
    if !st.raw_words.is_empty() {
        st.raw_words = project.transcript(clip).map(|t| t.words.clone()).unwrap_or_default();
    }
    n
}

/// Regroup the raw words (if the last run had word timings) with the current knobs and regenerate the
/// cues, replacing the previously generated ones. No re-transcription - pure post-processing.
fn generate(st: &mut TranscribeState, project: &mut Project) -> String {
    if !st.raw_words.is_empty() {
        st.segments = transcribe::group_words(&st.raw_words, &st.group);
        st.groups = transcribe::duplicate_takes(&st.segments, st.threshold, TAKE_WINDOW);
    }
    let cont = (project.subtitle_cont_prefix.clone(), project.subtitle_cont_suffix.clone());
    let cues = transcribe::to_cues(&st.segments, st.max_chars, st.lines, st.min_dur, (&cont.0, &cont.1));
    let old: std::collections::HashSet<Id> = st.generated.iter().copied().collect();
    project.subtitles.retain(|c| !old.contains(&c.id));
    st.generated = cues.iter().map(|(s, e, t)| project.add_cue(*s, *e, t.clone())).collect();
    format!("{} cues from {} segments", st.generated.len(), st.segments.len())
}

/// The "Transcribe" section: model + download, the run, and the double-take list.
fn transcribe_section(
    ui: &mut egui::Ui,
    st: &mut TranscribeState,
    project: &mut Project,
    selection: &[Id],
    undone: &mut bool,
    undo: &mut dyn FnMut(&Project),
    resp: &mut SubtitlesResponse,
) {
    let (name, file, mb) = transcribe::MODELS[st.model.min(transcribe::MODELS.len() - 1)];
    let have = transcribe::have_model(file);
    let exe = st.exe.get_or_insert_with(transcribe::exe).clone();
    let warn = ui.visuals().warn_fg_color;

    let downloading = st.download.as_ref().is_some_and(|p| !p.is_done());
    if !have {
        // ws:transcript-captions: the one-click entry point, above the model combo. The opt-in:
        // nothing is fetched until this button is pressed, and the size is on it - never at startup.
        match st.download.clone() {
            Some(p) if !p.is_done() => {
                ui.add(egui::ProgressBar::new(p.fraction()).show_percentage().text(p.status()));
                if ui.button("Cancel").clicked() {
                    p.cancel.store(true, Ordering::SeqCst);
                }
                ui.ctx().request_repaint_after(Duration::from_millis(200));
            }
            done => {
                if let Some(e) = done.and_then(|p| p.error()) {
                    ui.colored_label(warn, e);
                }
                let short = name.split(" - ").next().unwrap_or(name);
                if glyph_text_button(
                    ui,
                    Glyph::Subtitles,
                    &format!("Get captions  (download whisper {short}, {mb} MB)"),
                )
                .on_hover_text(format!(
                    "Downloads the {mb} MB model once, from huggingface.co into {}. Nothing else is fetched.",
                    transcribe::models_dir().display()
                ))
                .clicked()
                {
                    st.download = Some(transcribe::download_model(file));
                }
            }
        }
    }
    ui.horizontal_wrapped(|ui| {
        ui.label("Model");
        ui.add_enabled_ui(!downloading, |ui| {
            egui::ComboBox::from_id_salt("whisper_model").selected_text(name).show_ui(ui, |ui| {
                for (i, (n, _, size)) in transcribe::MODELS.iter().enumerate() {
                    ui.selectable_value(&mut st.model, i, format!("{n}  ({size} MB)"));
                }
            });
        });
        if have {
            ui.weak("downloaded");
        }
    });
    if exe.is_none() {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(warn, transcribe::install_hint());
            if ui.small_button("Re-check").clicked() {
                st.exe = None;
            }
        });
    }

    egui::Grid::new("transcribe_params").num_columns(2).show(ui, |ui| {
        ui.label("Language");
        ui.add(egui::TextEdit::singleline(&mut st.language).desired_width(60.0).hint_text("auto"));
        ui.end_row();
        ui.label("Line length");
        ui.add(DragValue::new(&mut st.max_chars).range(20..=90).suffix(" chars"));
        ui.end_row();
        ui.label("Lines per cue");
        ui.add(DragValue::new(&mut st.lines).range(1..=4))
            .on_hover_text("Wrapped lines shown together in one cue (2 = classic two-line subtitles)");
        ui.end_row();
        ui.label("Min duration");
        ui.add(DragValue::new(&mut st.min_dur).range(0.3..=5.0).speed(0.05).suffix(" s"));
        ui.end_row();
        ui.label("Prompt").on_hover_text("Names, jargon and punctuation style hints for whisper - not commands");
        ui.add(egui::TextEdit::singleline(&mut st.prompt).desired_width(220.0).hint_text("vocabulary hints…"));
        ui.end_row();
        // every run is word-timed (`App::transcribe_clip`), so the grouping knobs always apply
        ui.label("Pause split");
        ui.add(DragValue::new(&mut st.group.max_gap).range(0.1..=5.0).speed(0.05).suffix(" s"))
            .on_hover_text("A silence longer than this starts a new sentence");
        ui.end_row();
        ui.label("Break on");
        ui.add(egui::TextEdit::singleline(&mut st.group.punct).desired_width(60.0).hint_text("none"))
            .on_hover_text("A word ending with any of these characters ends the sentence");
        ui.end_row();
        ui.label("Max words");
        let mut mw = st.group.max_words;
        if ui
            .add(DragValue::new(&mut mw).range(0..=40).custom_formatter(|v, _| {
                if v < 1.0 {
                    "off".into()
                } else {
                    format!("{v:.0}")
                }
            }))
            .on_hover_text("Cap a sentence at this many words (0 = no cap)")
            .changed()
        {
            st.group.max_words = mw;
        }
        ui.end_row();
        ui.label("Sentence chars");
        ui.add(DragValue::new(&mut st.group.max_chars).range(30..=300).suffix(" chars"))
            .on_hover_text("A sentence never grows past this many characters");
        ui.end_row();
    });

    let tgt = target(project, selection);
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(have && exe.is_some() && tgt.is_some(), |ui| {
            if glyph_text_button(ui, Glyph::Mic, "Transcribe & generate subtitles")
                .on_hover_text("Runs in the background (see the Jobs pane) - the cues land even if this pane is closed")
                .clicked()
            {
                st.want = tgt.as_ref().map(|t| t.clip);
            }
        });
        let can_regen = !st.segments.is_empty() || !st.raw_words.is_empty();
        if ui
            .add_enabled(can_regen, Button::new("Regenerate cues"))
            .on_hover_text("Rebuild the cues from the last transcript with the knobs above - no re-transcription")
            .clicked()
        {
            once(undone, undo, project);
            st.status = generate(st, project);
            resp.edited = true;
        }
        if tgt.is_none() {
            ui.weak("Select a clip to transcribe.");
        }
    });
    if !st.status.is_empty() {
        ui.weak(&st.status);
    }

    if st.segments.is_empty() {
        return;
    }
    ui.separator();
    let dups: usize = st.groups.iter().map(|g| g.len() - 1).sum();
    ui.horizontal_wrapped(|ui| {
        ui.label("Double takes");
        if ui.add(Slider::new(&mut st.threshold, 0.5..=1.0).fixed_decimals(2).text("similarity")).changed() {
            st.groups = transcribe::duplicate_takes(&st.segments, st.threshold, TAKE_WINDOW);
        }
        ui.weak(format!("{dups} repeated"));
    });
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(dups > 0, Button::new("Mark on timeline")).clicked() {
            once(undone, undo, project);
            for id in st.marks.drain(..) {
                project.remove_marker(id);
            }
            st.marks = mark_dups(project, &st.segments, &st.groups);
            st.status = format!("marked {} take(s)", st.marks.len());
            resp.edited = true;
        }
        if ui
            .add_enabled(dups > 0 && st.clip.is_some(), Button::new("Cut the duplicates"))
            .on_hover_text("Ripples every take but the last one out, and moves the cues with it")
            .clicked()
        {
            once(undone, undo, project);
            let n = cut_dups(project, st);
            st.status = format!("cut {n} clip(s)");
            resp.edited = true;
        }
    });
    for (shown, &i) in st.groups.iter().flat_map(|g| &g[..g.len() - 1]).enumerate() {
        if shown == 6 {
            ui.weak(format!("… and {} more", dups - 6));
            break;
        }
        if let Some(s) = st.segments.get(i) {
            ui.weak(format!("{}  {}", crate::ui::duration_text(s.start), transcribe::short_label(&s.text, 64)));
        }
    }
}

fn export_dialog(project: &Project, vtt: bool) {
    if project.subtitles.is_empty() {
        return;
    }
    let (ext, name) = if vtt { ("vtt", "WebVTT") } else { ("srt", "SubRip") };
    let Some(path) =
        rfd::FileDialog::new().add_filter(name, &[ext]).set_file_name(format!("{}.{ext}", project.name)).save_file()
    else {
        return;
    };
    let cues = &project.subtitles;
    let text = if vtt { crate::engine::subtitles::to_vtt(cues) } else { crate::engine::subtitles::to_srt(cues) };
    let _ = std::fs::write(path, text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, AudioStreamInfo, ClipKind};
    use eframe::egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect};

    /// Headless pane: real fonts, so popup entries land where they are painted.
    struct H {
        ctx: egui::Context,
        state: SubtitlesState,
        project: Project,
        playhead: f64,
        undos: usize,
        time: f64,
        shapes: Vec<egui::epaint::ClippedShape>,
        resp: SubtitlesResponse,
    }

    impl H {
        fn new() -> Self {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::theme::test_fonts());
            let mut project = Project::new();
            project.add_cue(0.0, 1.0, "a");
            project.add_cue(2.0, 3.0, "b");
            project.add_cue(4.0, 5.0, "c");
            let resp = SubtitlesResponse::default();
            Self {
                ctx,
                state: SubtitlesState::default(),
                project,
                playhead: 0.5,
                undos: 0,
                time: 0.0,
                shapes: vec![],
                resp,
            }
        }
        fn id(&self, text: &str) -> Id {
            self.project.subtitles.iter().find(|c| c.text == text).expect("cue").id
        }
        fn frame_mod(&mut self, events: Vec<Event>, modifiers: Modifiers) {
            self.time += 0.05;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 500.0))),
                time: Some(self.time),
                modifiers,
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, egui::Color32::WHITE);
            let H { ctx, state, project, playhead, undos, shapes, resp, .. } = self;
            let full = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    let r = show(ui, state, project, playhead, &[], &[], &pal, &mut undo);
                    resp.edited |= r.edited;
                    resp.seeked |= r.seeked;
                    resp.play |= r.play;
                    resp.cleared_subtitles |= r.cleared_subtitles;
                });
            });
            *shapes = full.shapes;
        }
        fn frame(&mut self, events: Vec<Event>) {
            self.frame_mod(events, Modifiers::NONE);
        }
        fn press(&mut self, at: Pos2, button: PointerButton, modifiers: Modifiers) {
            self.time += 1.0; // every press is its own gesture, never half of a double-click
            self.frame_mod(vec![Event::PointerMoved(at)], modifiers);
            for pressed in [true, false] {
                self.frame_mod(vec![Event::PointerButton { pos: at, button, pressed, modifiers }], modifiers);
            }
            self.frame(vec![]);
        }
        fn text_at(&self, label: &str) -> Pos2 {
            self.shapes
                .iter()
                .find_map(|c| match &c.shape {
                    egui::epaint::Shape::Text(t) if t.galley.text() == label => Some(t.visual_bounding_rect().center()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("nothing painted reads '{label}'"))
        }
        fn cue_text(&self, id: Id) -> Pos2 {
            self.ctx.data(|d| d.get_temp::<Rect>(egui::Id::new(("cue_text", id)))).expect("cue row drawn").center()
        }
    }

    /// Right-click ▸ Delete acts on every selected cue when the row is part of the selection - one undo.
    #[test]
    fn cue_row_menu_deletes_the_selection_with_one_undo() {
        let mut h = H::new();
        let (a, b) = (h.id("a"), h.id("b"));
        h.state.checked = [a, b].into_iter().collect();
        h.frame(vec![]);
        let at = h.cue_text(b);
        h.press(at, PointerButton::Secondary, Modifiers::NONE);
        let item = h.text_at("Delete 2 cues");
        h.press(item, PointerButton::Primary, Modifiers::NONE);
        let left: Vec<&str> = h.project.subtitles.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(left, vec!["c"]);
        assert_eq!(h.undos, 1);
        assert!(h.state.checked.is_empty());
    }

    /// Right-click ▸ Play cue seeks to the cue and asks for playback - the old ▶ button's job. The
    /// right-click focuses the text field under it, which must not leave an empty undo step behind.
    #[test]
    fn cue_row_menu_plays_the_cue() {
        let mut h = H::new();
        let b = h.id("b");
        h.frame(vec![]);
        let at = h.cue_text(b);
        h.press(at, PointerButton::Secondary, Modifiers::NONE);
        let item = h.text_at("Play cue");
        h.press(item, PointerButton::Primary, Modifiers::NONE);
        assert!(h.resp.seeked && h.resp.play);
        assert_eq!(h.playhead, 2.0);
        assert_eq!(h.undos, 0, "playing is not an edit");
        // typing into the (now focused) field is: one undo step for the whole visit
        let at = h.cue_text(b);
        h.press(at, PointerButton::Primary, Modifiers::NONE);
        for ch in ["x", "y"] {
            h.frame(vec![Event::Text(ch.into())]);
        }
        assert_eq!(h.project.subtitles.iter().find(|c| c.id == b).unwrap().text, "bxy");
        assert_eq!(h.undos, 1, "one undo per visit, taken at the first keystroke");
    }

    /// A click picks one cue, Ctrl+click adds another - the multi-selection the row menu acts on.
    #[test]
    fn clicks_pick_and_ctrl_clicks_add_cues() {
        let mut h = H::new();
        let (b, c) = (h.id("b"), h.id("c"));
        h.frame(vec![]);
        let at = h.cue_text(b);
        h.press(at, PointerButton::Primary, Modifiers::NONE);
        assert_eq!(h.state.checked, [b].into_iter().collect());
        let at = h.cue_text(c);
        h.press(at, PointerButton::Primary, Modifiers::CTRL);
        assert_eq!(h.state.checked, [b, c].into_iter().collect());
    }

    /// The header's ⋯ holds the bulk verbs: Clear all is one undoable edit the app toasts an Undo for.
    #[test]
    fn more_menu_clears_all() {
        let mut h = H::new();
        h.frame(vec![]);
        let dots = h.text_at("⋯");
        h.press(dots, PointerButton::Primary, Modifiers::NONE);
        let item = h.text_at("Clear all");
        h.press(item, PointerButton::Primary, Modifiers::NONE);
        assert!(h.project.subtitles.is_empty());
        assert!(h.resp.cleared_subtitles);
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn add_at_creates_cue_at_playhead() {
        let mut p = Project::new();
        let id = add_at(&mut p, 3.5);
        let c = p.cue_at(3.5).expect("cue at playhead");
        assert_eq!(c.id, id);
        assert!((c.start - 3.5).abs() < 1e-9 && (c.end - 5.5).abs() < 1e-9);
        assert_eq!(c.text, "Subtitle");
    }

    #[test]
    fn import_replace_and_append() {
        let mut p = Project::new();
        p.add_cue(0.0, 1.0, "old");
        let cues = vec![(2.0, 3.0, "b".to_string()), (0.5, 1.5, "a".to_string())];
        apply_import(&mut p, &cues, false);
        assert_eq!(p.subtitles.len(), 3);
        assert_eq!(p.subtitles[0].text, "old"); // kept + sorted
        apply_import(&mut p, &cues, true);
        assert_eq!(p.subtitles.len(), 2);
        assert_eq!(p.subtitles[0].text, "a");
    }

    /// Runs engine::subtitles::parse on a small SRT.
    #[test]
    fn import_parses_srt() {
        let srt = "1\n00:00:01,000 --> 00:00:02,500\nHello\n\n2\n00:00:03,000 --> 00:00:04,000\nWorld\n";
        let cues = crate::engine::subtitles::parse(srt);
        assert_eq!(cues.len(), 2);
        assert!((cues[0].0 - 1.0).abs() < 1e-3 && (cues[0].1 - 2.5).abs() < 1e-3);
        assert_eq!(cues[0].2, "Hello");
        let mut p = Project::new();
        apply_import(&mut p, &cues, true);
        assert_eq!(p.subtitles.len(), 2);
    }

    /// Headless: panel lays out with cues and every section open, reports nothing without interaction,
    /// leaves the project alone, starts no job and asks for no repaint (assert_no_idle_repaint).
    #[test]
    fn assert_no_idle_repaint_subtitles_all_sections_open() {
        let (mut p, clip) = clip_project(1.0);
        p.add_cue(0.0, 1.0, "a");
        p.add_cue(2.0, 4.0, "b");
        let before = p.to_json();
        let palette = Palette::new(true, egui::Color32::WHITE);
        let fonts = vec!["Segoe UI".to_string()];
        // the transcribe section draws too, with a clip selected - only its hints, nothing started
        let mut state = SubtitlesState {
            show_style: true,
            show_transcript: true,
            transcribe: TranscribeState { open: true, ..Default::default() },
            ..Default::default()
        };
        let mut playhead = 2.5; // inside cue "b" → highlighted row
        let ctx = egui::Context::default();
        for i in 0..60 {
            state.show_transcript = i >= 30; // both tabs
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| panic!("no undo without edits");
                    let r = show(ui, &mut state, &mut p, &mut playhead, &[clip], &fonts, &palette, &mut undo);
                    assert!(!r.edited && !r.seeked);
                });
            });
        }
        assert_eq!(p.to_json(), before);
        assert!(state.transcribe.want.is_none() && state.transcribe.segments.is_empty());
        assert!(!ctx.has_requested_repaint(), "an idle Subtitles pane must not spin");
    }

    /// A clip with an asset, on V1 + A1, running 0..10 s of the source.
    fn clip_project(speed: f64) -> (Project, Id) {
        let mut p = Project::new();
        let aid = p.add_asset(Asset {
            id: 0,
            path: "C:/take.mp4".into(),
            kind: ClipKind::Video,
            duration: 10.0,
            width: 320,
            height: 240,
            fps: 30.0,
            audio_streams: vec![AudioStreamInfo { channels: 2, sample_rate: 48000, ..Default::default() }],
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
        });
        p.insert_asset_clips(aid, 0.0, Some(0));
        let id = p.tracks[0].clips[0].id;
        if speed != 1.0 {
            p.set_speed(&[id], speed, false);
        }
        (p, id)
    }

    #[test]
    fn regenerate_replaces_its_own_cues_and_keeps_the_rest() {
        let mut p = Project::new();
        let manual = p.add_cue(50.0, 51.0, "hand-written");
        let mut st = TranscribeState {
            raw_words: vec![(0.0, 0.4, "One".into()), (0.5, 0.9, "two.".into()), (1.2, 1.6, "Three.".into())],
            ..Default::default()
        };
        generate(&mut st, &mut p);
        assert_eq!(st.segments.len(), 2, "grouped on punctuation");
        let n = p.subtitles.len();
        assert!(p.subtitles.iter().any(|c| c.id == manual));
        // tighter knobs: every word its own sentence - same word data, no re-transcription
        st.group.max_words = 1;
        generate(&mut st, &mut p);
        assert_eq!(st.segments.len(), 3);
        assert!(p.subtitles.iter().any(|c| c.id == manual), "manual cue survives");
        assert_eq!(p.subtitles.len(), n + 1, "old generated cues were replaced, not stacked");
    }

    #[test]
    fn split_cue_makes_two_halves_with_the_same_text() {
        let mut p = Project::new();
        let id = p.add_cue(1.0, 3.0, "hello");
        assert!(p.split_cue(id, 0.5).is_none(), "outside the cue");
        assert!(p.split_cue(id, 1.01).is_none(), "too close to the edge");
        let right = p.split_cue(id, 2.0).expect("split");
        assert_eq!(p.subtitles.len(), 2);
        assert!((p.subtitles[0].end - 2.0).abs() < 1e-9 && (p.subtitles[1].start - 2.0).abs() < 1e-9);
        assert_eq!(p.subtitles[1].id, right);
        assert_eq!(p.subtitles[1].text, "hello");
    }

    #[test]
    fn cues_convert_to_text_clips() {
        let mut p = Project::new();
        let a = p.add_cue(1.0, 2.0, "first");
        p.add_cue(3.0, 4.0, "second");
        assert_eq!(p.cues_to_text_clips(Some(&[a])), 1);
        assert_eq!(p.subtitles.len(), 1, "the converted cue is gone");
        let track = p.tracks.iter().find(|t| t.name == "Subtitles").expect("subtitle track");
        assert_eq!(track.clips.len(), 1);
        let c = &track.clips[0];
        assert_eq!(c.kind, ClipKind::Text);
        assert_eq!(c.text.as_ref().unwrap().text, "first");
        assert!((c.start - 1.0).abs() < 1e-9 && (c.duration - 1.0).abs() < 1e-9);
        assert_eq!(p.cues_to_text_clips(None), 1, "convert-all reuses the track");
        assert!(p.subtitles.is_empty());
        assert_eq!(p.tracks.iter().filter(|t| t.name == "Subtitles").count(), 1);
    }

    #[test]
    fn target_maps_source_time_onto_the_timeline() {
        let (p, id) = clip_project(1.0);
        assert!(target(&p, &[]).is_none(), "nothing selected");
        let t = target(&p, &[id]).expect("a clip with footage");
        assert_eq!((t.src_start, t.src_dur, t.offset, t.scale), (0.0, 10.0, 0.0, 1.0));
        // at 2x the clip is 5 s of timeline over 10 s of source, so the transcript is squeezed by half
        let (p, id) = clip_project(2.0);
        let t = target(&p, &[id]).expect("a clip with footage");
        assert!((t.src_dur - 10.0).abs() < 1e-6 && (t.scale - 0.5).abs() < 1e-6, "{:?}", (t.src_dur, t.scale));
        // a text clip has no footage to listen to
        let mut p = Project::new();
        let txt = p.add_text_clip(0.0, 2.0);
        assert!(target(&p, &[txt]).is_none());
    }

    #[test]
    fn duplicate_takes_are_marked_then_cut_with_the_cues() {
        let (mut p, id) = clip_project(1.0);
        let seg = |a: f64, b: f64, t: &str| Segment { start: a, end: b, text: t.into(), words: Vec::new() };
        let mut st = TranscribeState {
            clip: Some(id),
            segments: vec![
                seg(0.0, 2.0, "Welcome to the channel"),
                seg(2.5, 4.5, "Welcome to the channel"), // retake
                seg(5.0, 7.0, "Welcome to the channel"), // keeper
                seg(7.5, 9.5, "Now the actual video"),
            ],
            ..Default::default()
        };
        st.groups = transcribe::duplicate_takes(&st.segments, st.threshold, TAKE_WINDOW);
        assert_eq!(st.groups, vec![vec![0, 1, 2]]);

        st.marks = mark_dups(&mut p, &st.segments, &st.groups);
        assert_eq!(st.marks.len(), 2, "the two flubbed takes, not the keeper");
        let m = p.markers.iter().find(|m| m.id == st.marks[0]).expect("marker");
        assert_eq!(m.note, "Welcome to the channel", "the marker is described by what was said");
        assert!(m.duration > 0.0 && m.t == 0.0);

        // cues over the whole transcript, then the cut takes the duplicates and drags the rest left
        for s in &st.segments {
            p.add_cue(s.start, s.end, s.text.clone());
        }
        let n = cut_dups(&mut p, &mut st);
        assert!(n >= 2, "clips removed: {n}");
        assert!(p.markers.is_empty(), "our markers went with the cut");
        assert!((p.duration() - 6.0).abs() < 0.05, "the two takes (4 s) are gone: {}", p.duration());
        assert_eq!(p.subtitles.len(), 2, "the duplicated cues went too");
        assert!((p.subtitles[0].start - 1.0).abs() < 1e-6, "the keeper moved up: {:?}", p.subtitles[0]);
        assert_eq!(p.subtitles[1].text, "Now the actual video");
        assert_eq!(st.segments.len(), 2, "the transcript follows the cut");
        assert!(st.groups.is_empty(), "nothing is a duplicate any more");
    }
}
