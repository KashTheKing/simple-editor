//! Subtitles panel. Toolbar: "Add at playhead" (cue [playhead, playhead+2 s) with the text "Subtitle",
//! selected + text field focused), "Import…" (rfd: srt/vtt → engine::subtitles::parse → replace or append
//! after a yes/no), "Export SRT…" / "Export VTT…" (engine::subtitles::to_srt/to_vtt), "Burn in" checkbox
//! (project.show_subtitles), and a "Style" collapsing section (font combo from `fonts`, size, colour,
//! outline width/colour, background box colour, margin from bottom = project.subtitle_margin).
//! Below: the cue list (egui::Grid / ScrollArea): start and end as editable timecode-ish DragValues in
//! seconds (3 decimals, end ≥ start + 0.1, keep the list sorted via Project::sort_cues), a multiline text
//! field, a play button (seek to the cue and play → `seeked` + `play`), a select checkbox and a delete
//! one; the cue containing the playhead is highlighted; a "Split at playhead" button on the highlighted
//! cue. A second toolbar row: "To text clips" (Project::cues_to_text_clips - editable Text clips on a
//! "Subtitles" track), "Delete selected", "Delete in range" (the In/Out range) and "Clear all".
//! "Open folder" → `open_folder`: the app writes the .srt sidecar and opens the folder. Undo once per
//! gesture (same edit_start rule as the inspector); returns what changed.
//!
//! The "Transcribe" section drives `engine::transcribe`: pick a whisper.cpp model (its download size is
//! named before the click and the download shows a progress bar), transcribe the selected clip's audio on
//! a worker thread, and turn the transcript into cues ("Transcribe & generate subtitles"). The raw
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
//! so they survive a reopen and feed the collapsible "Transcript" section (`ui::transcript_ui`:
//! click = seek, select + Delete = ripple cut through `Project::cut_word_ranges`, filler removal with
//! Mark-instead first, word search across every transcribed clip). The double-take cutter now goes
//! through that same `cut_word_ranges`. "Get captions" is the one-click entry: it names the model's
//! size before any network call and never runs at startup.

use crate::engine::export::Progress;
use crate::engine::transcribe::{self, Segment};
use crate::model::{Id, Project};
use crate::theme::Palette;
use crate::ui::tools::{glyph_text_button, Glyph};
use crate::ui::transcript_ui;
use crate::ui::{edit_start, once};
use eframe::egui::{self, Button, DragValue, Response, Slider};
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
    pub words: bool,
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
    job: Option<transcribe::Job>,
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
            words: false,
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
            job: None,
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
    /// Multi-selected cues (the row checkboxes) for "Delete selected".
    pub checked: std::collections::HashSet<Id>,
    pub show_style: bool,
    /// Cue whose text field should grab focus (set by "Add at playhead").
    pub focus: Option<Id>,
    pub transcribe: TranscribeState,
    // ---- ws:transcript-captions ----
    /// The collapsible Transcript section (mirrors `show_style`; `Action::ToggleTranscript`).
    pub show_transcript: bool,
    pub transcript: transcript_ui::TranscriptUiState,
}

#[derive(Default)]
pub struct SubtitlesResponse {
    pub edited: bool,
    pub seeked: bool,
    /// The cue's play button: seek there and start playback.
    pub play: bool,
    /// "Open folder" - the app writes the .srt sidecar next to the project and opens it in Explorer.
    pub open_folder: bool,
    /// ---- ws:forgiveness ----
    /// "Clear all" ran inline (no confirm dialog) - the app toasts an Undo.
    pub cleared_subtitles: bool,
    /// Import parsed cues while the project ALREADY has subtitles: the app queues a non-blocking
    /// "Replace?" confirm (`ConfirmAction::ReplaceSubtitles`) instead of asking here (this module has
    /// no `App` to queue one against). `None` when there was nothing to import, or nothing existing to
    /// ask about (an empty project replaces inline, no prompt needed). Same `(start, end, text)` shape
    /// `engine::subtitles::parse`/`apply_import` already use - ids are allocated on Yes, via
    /// `Project::add_cue`, not carried here.
    pub import_replace_cues: Option<Vec<(f64, f64, String)>>,
}

/// "Add at playhead": a 2 s cue starting at the playhead.
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
    let ph = *playhead;

    ui.horizontal_wrapped(|ui| {
        if ui.button("Add at playhead").clicked() {
            once(&mut undone, undo, project);
            let id = add_at(project, ph);
            state.selected = Some(id);
            state.focus = Some(id);
            resp.edited = true;
        }
        if crate::ui::tools::glyph_text_button(ui, crate::ui::tools::Glyph::ImportArrow, "Import…").clicked() {
            import_dialog(project, &mut undone, undo, &mut resp);
        }
        if crate::ui::tools::glyph_text_button(ui, crate::ui::tools::Glyph::ExportArrow, "Export SRT…").clicked() {
            export_dialog(project, false);
        }
        if crate::ui::tools::glyph_text_button(ui, crate::ui::tools::Glyph::ExportArrow, "Export VTT…").clicked() {
            export_dialog(project, true);
        }
        if ui.button("Open folder").on_hover_text("Open the project's subtitle folder in Explorer").clicked() {
            resp.open_folder = true;
        }
        let mut burn = project.show_subtitles;
        let r = ui.checkbox(&mut burn, "Burn in");
        if r.changed() {
            once(&mut undone, undo, project);
            project.show_subtitles = burn;
            resp.edited = true;
        }
        ui.toggle_value(&mut state.show_style, "Style");
        ui.toggle_value(&mut state.transcribe.open, "Transcribe");
        // ---- ws:transcript-captions ----
        ui.toggle_value(&mut state.show_transcript, "Transcript")
            .on_hover_text("The words of a transcribed clip: click to seek, select + Delete to cut, fillers");
    });
    ui.horizontal_wrapped(|ui| {
        let any = !project.subtitles.is_empty();
        let sel = state.checked.len();
        let label = if sel > 0 { format!("To text clips ({sel})") } else { "To text clips".into() };
        if ui
            .add_enabled(any, Button::new(label))
            .on_hover_text("Turn the selected cues (or all of them) into editable Text clips on a \"Subtitles\" track")
            .clicked()
        {
            once(&mut undone, undo, project);
            let only: Vec<Id> = state.checked.iter().copied().collect();
            let n = project.cues_to_text_clips(if only.is_empty() { None } else { Some(&only) });
            state.checked.clear();
            resp.edited = n > 0;
        }
        let n = state.checked.len();
        if ui.add_enabled(n > 0, Button::new(format!("Delete selected ({n})"))).clicked() {
            once(&mut undone, undo, project);
            project.subtitles.retain(|c| !state.checked.contains(&c.id));
            state.checked.clear();
            resp.edited = true;
        }
        let range = match (project.in_point, project.out_point) {
            (Some(a), Some(b)) if b > a => Some((a, b)),
            _ => None,
        };
        if ui
            .add_enabled(any && range.is_some(), Button::new("Delete in range"))
            .on_hover_text("Delete every cue that overlaps the In/Out range (set with I and O)")
            .clicked()
        {
            let (a, b) = range.expect("button enabled only with a range");
            once(&mut undone, undo, project);
            project.subtitles.retain(|c| c.end <= a || c.start >= b);
            state.checked.retain(|id| project.subtitles.iter().any(|c| c.id == *id));
            resp.edited = true;
        }
        // no confirm dialog: this is already inside the undo-snapshotting `once(...)` helper below, so
        // it's a normal undoable edit - the app additionally toasts an Undo button (resp.cleared_subtitles).
        if ui.add_enabled(any, Button::new("Clear all")).clicked() {
            once(&mut undone, undo, project);
            project.subtitles.clear();
            state.checked.clear();
            resp.edited = true;
            resp.cleared_subtitles = true;
        }
    });

    if state.show_style {
        style_section(ui, project, fonts, &mut undone, undo, &mut resp);
    }
    if state.transcribe.open {
        ui.separator();
        transcribe_section(ui, &mut state.transcribe, project, selection, &mut undone, undo, &mut resp);
    }
    // ---- ws:transcript-captions ----
    if state.show_transcript {
        ui.separator();
        let r =
            transcript_ui::show(ui, &mut state.transcript, project, playhead, selection, palette, &mut undone, undo);
        resp.edited |= r.edited;
        resp.seeked |= r.seeked;
        if r.cut > 0 {
            // the section's own words follow the cut (Project.transcripts already did); the
            // Transcribe section's copy for "Regenerate cues" follows too
            if let Some(tr) = state.transcribe.clip.and_then(|c| project.transcript(c)) {
                if !state.transcribe.raw_words.is_empty() {
                    state.transcribe.raw_words = tr.words.clone();
                }
            }
        }
    }
    ui.separator();

    let mut resort = false;
    let mut del: Option<Id> = None;
    let mut split: Option<Id> = None;
    let mut convert: Option<Id> = None;
    let (shift, primary_down) = ui.input(|i| (i.modifiers.shift, i.pointer.primary_down()));
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for i in 0..project.subtitles.len() {
            let (id, start, end) = {
                let c = &project.subtitles[i];
                (c.id, c.start, c.end)
            };
            let active = ph >= start && ph < end;
            let fill = if active {
                palette.selection.gamma_multiply(0.25)
            } else if state.selected == Some(id) {
                palette.selection.gamma_multiply(0.12)
            } else {
                egui::Color32::TRANSPARENT
            };
            let row_rect = egui::Frame::new()
                .fill(fill)
                .inner_margin(2.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut on = state.checked.contains(&id);
                        if ui.checkbox(&mut on, "").on_hover_text("Select for \"Delete selected\"").changed() {
                            if on {
                                state.checked.insert(id);
                            } else {
                                state.checked.remove(&id);
                            }
                        }
                        let mut v = start;
                        let r =
                            ui.add(DragValue::new(&mut v).range(0.0..=(end - MIN_CUE)).speed(0.05).fixed_decimals(3));
                        if edit_start(&r) {
                            once(&mut undone, undo, project);
                        }
                        if r.changed() {
                            project.subtitles[i].start = v.clamp(0.0, end - MIN_CUE);
                            resp.edited = true;
                        }
                        if r.drag_stopped() || (r.changed() && !r.dragged()) {
                            resort = true;
                        }
                        let mut v = end;
                        let r = ui.add(
                            DragValue::new(&mut v).range((start + MIN_CUE)..=86400.0).speed(0.05).fixed_decimals(3),
                        );
                        if edit_start(&r) {
                            once(&mut undone, undo, project);
                        }
                        if r.changed() {
                            project.subtitles[i].end = v.max(start + MIN_CUE);
                            resp.edited = true;
                        }
                        if glyph_text_button(ui, Glyph::Play, "").on_hover_text("Play from this cue").clicked() {
                            *playhead = start;
                            state.selected = Some(id);
                            resp.seeked = true;
                            resp.play = true;
                        }
                        if active && ph > start + 0.05 && ph < end - 0.05 && ui.small_button("Split").clicked() {
                            split = Some(id);
                        }
                        if ui.small_button("T").on_hover_text("Convert to an editable Text clip").clicked() {
                            convert = Some(id);
                        }
                        if crate::ui::markers_ui::x_button(ui).on_hover_text("Delete this cue").clicked() {
                            del = Some(id);
                        }
                    });
                    let mut text = project.subtitles[i].text.clone();
                    let r = ui.add(egui::TextEdit::multiline(&mut text).desired_rows(1).desired_width(f32::INFINITY));
                    // One undo entry per visit to the field, not per keystroke. "Add at playhead" focuses the
                    // new cue itself and has already pushed one, so that focus does not push another.
                    if state.focus == Some(id) {
                        r.request_focus();
                        state.focus = None;
                    } else if r.gained_focus() {
                        once(&mut undone, undo, project);
                    }
                    if r.has_focus() {
                        state.selected = Some(id);
                    }
                    if r.changed() {
                        project.subtitles[i].text = text;
                        resp.edited = true;
                    }
                })
                .response
                .rect;
            // Shift+drag over rows sweeps them into the selection (plain drags still edit the widgets)
            if shift && primary_down && ui.rect_contains_pointer(row_rect) {
                state.checked.insert(id);
            }
        }
        if project.subtitles.is_empty() {
            ui.weak("No subtitles. \"Add at playhead\" or import an .srt / .vtt file.");
        }
    });

    if let Some(id) = convert {
        once(&mut undone, undo, project);
        project.cues_to_text_clips(Some(&[id]));
        state.checked.remove(&id);
        resp.edited = true;
    }
    if let Some(id) = del {
        once(&mut undone, undo, project);
        project.remove_cue(id);
        state.checked.remove(&id);
        if state.selected == Some(id) {
            state.selected = None;
        }
        resp.edited = true;
    }
    if let Some(id) = split {
        if let Some(c) = project.subtitles.iter_mut().find(|c| c.id == id) {
            let (end, text) = (c.end, c.text.clone());
            once(&mut undone, undo, project);
            if let Some(c) = project.subtitles.iter_mut().find(|c| c.id == id) {
                c.end = ph;
            }
            let nid = project.add_cue(ph, end, text);
            state.selected = Some(nid);
            resp.edited = true;
        }
    }
    if resort {
        project.sort_cues();
    }
    resp
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
        ui.label("Word timings");
        ui.checkbox(&mut st.words, "")
            .on_hover_text("Slower: whisper times every word, so the cues break exactly on speech");
        ui.end_row();
        ui.label("Prompt").on_hover_text("Names, jargon and punctuation style hints for whisper - not commands");
        ui.add(egui::TextEdit::singleline(&mut st.prompt).desired_width(220.0).hint_text("vocabulary hints…"));
        ui.end_row();
        if st.words || !st.raw_words.is_empty() {
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
        }
    });

    let tgt = target(project, selection);
    let running = st.job.as_ref().is_some_and(|j| !j.progress.is_done());
    let mut go = false;
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(have && exe.is_some() && tgt.is_some() && !running, |ui| {
            go = glyph_text_button(ui, Glyph::Mic, "Transcribe & generate subtitles").clicked();
        });
        if running && ui.button("Cancel").clicked() {
            if let Some(j) = &st.job {
                j.cancel();
            }
        }
        let can_regen = !running && (!st.segments.is_empty() || !st.raw_words.is_empty());
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
    if let (true, Some(t)) = (go, &tgt) {
        st.segments.clear();
        st.groups.clear();
        st.marks.clear();
        st.status.clear();
        st.raw_words.clear();
        // a fresh run appends to whatever cues exist - only a Regenerate replaces its own
        st.generated.clear();
        st.clip = Some(t.clip);
        st.map = (t.offset, t.scale);
        st.job = Some(transcribe::start(transcribe::Options {
            path: t.path.clone(),
            src_start: t.src_start,
            src_duration: t.src_dur,
            model: file.to_string(),
            language: st.language.clone(),
            words: st.words,
            prompt: st.prompt.clone(),
        }));
    }

    let done = st.job.as_ref().is_some_and(|j| j.progress.is_done());
    if let Some(j) = &st.job {
        ui.add(egui::ProgressBar::new(j.progress.fraction()).show_percentage().text(j.progress.status()));
        if !done {
            ui.ctx().request_repaint_after(Duration::from_millis(150));
        }
    }
    if done {
        let job = st.job.take().expect("done implies a job");
        st.status = match job.progress.error() {
            Some(e) => e,
            None => {
                let mut segs = job.segments();
                transcribe::retime(&mut segs, st.map.0, st.map.1);
                once(undone, undo, project);
                if st.words {
                    // one word per segment: keep the raw words so "Regenerate cues" can regroup them
                    st.raw_words = segs.iter().map(|s| (s.start, s.end, s.text.clone())).collect();
                    // ws:transcript-captions: and persist them, so they survive a save/reopen and
                    // feed the Transcript section
                    if let Some(clip) = st.clip {
                        project.set_transcript(clip, st.raw_words.clone());
                    }
                } else {
                    st.raw_words.clear();
                    st.segments = segs;
                    st.groups = transcribe::duplicate_takes(&st.segments, st.threshold, TAKE_WINDOW);
                }
                let msg = generate(st, project);
                resp.edited = true;
                msg
            }
        };
    }
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

/// Import an .srt/.vtt via rfd. An empty project replaces inline (nothing to lose, no prompt needed);
/// otherwise the parsed cues are handed back via `resp.import_replace_cues` for the app to confirm a
/// replace (`ConfirmAction::ReplaceSubtitles`) - this module has no `App` to queue a `confirm::ask`
/// against directly. deviation from the plan text: "No/append" is now "Cancel discards the import"
/// (see the PR body) - silently appending data the user just declined to confirm was the more
/// surprising default of the two.
fn import_dialog(
    project: &mut Project,
    undone: &mut bool,
    undo: &mut dyn FnMut(&Project),
    resp: &mut SubtitlesResponse,
) {
    let Some(path) = rfd::FileDialog::new().add_filter("Subtitles", &["srt", "vtt"]).pick_file() else { return };
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let cues = crate::engine::subtitles::parse(&text);
    if cues.is_empty() {
        return;
    }
    if project.subtitles.is_empty() {
        once(undone, undo, project);
        apply_import(project, &cues, true);
        resp.edited = true;
    } else {
        resp.import_replace_cues = Some(cues);
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

    /// Headless: panel lays out with cues and reports nothing without interaction.
    #[test]
    fn show_headless() {
        let mut p = Project::new();
        p.add_cue(0.0, 1.0, "a");
        p.add_cue(2.0, 4.0, "b");
        let palette = Palette::new(true, egui::Color32::WHITE);
        let fonts = vec!["Segoe UI".to_string()];
        // the transcribe section draws too: no model, no whisper.exe, no selection - only its hints
        let mut state = SubtitlesState {
            show_style: true,
            transcribe: TranscribeState { open: true, ..Default::default() },
            ..Default::default()
        };
        let mut playhead = 2.5; // inside cue "b" → highlighted row with Split button
        let ctx = egui::Context::default();
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| panic!("no undo without edits");
                    let r = show(ui, &mut state, &mut p, &mut playhead, &[], &fonts, &palette, &mut undo);
                    assert!(!r.edited && !r.seeked);
                });
            });
        }
        assert_eq!(p.subtitles.len(), 2);
        assert!(state.transcribe.job.is_none() && state.transcribe.segments.is_empty());
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
