//! Subtitles panel. Toolbar: "Add at playhead" (cue [playhead, playhead+2 s) with the text "Subtitle",
//! selected + text field focused), "Import…" (rfd: srt/vtt → engine::subtitles::parse → replace or append
//! after a yes/no), "Export SRT…" / "Export VTT…" (engine::subtitles::to_srt/to_vtt), "Burn in" checkbox
//! (project.show_subtitles), and a "Style" collapsing section (font combo from `fonts`, size, colour,
//! outline width/colour, background box colour, margin from bottom = project.subtitle_margin).
//! Below: the cue list (egui::Grid / ScrollArea): start and end as editable timecode-ish DragValues in
//! seconds (3 decimals, end ≥ start + 0.1, keep the list sorted via Project::sort_cues), a multiline text
//! field, a play button (seek to the cue → `seeked`) and a delete one; the cue containing the playhead is highlighted; a
//! "Split at playhead" button on the highlighted cue. Undo once per gesture (same edit_start rule as the
//! inspector); returns what changed.
//!
//! The "Transcribe" section drives `engine::transcribe`: pick a whisper.cpp model (its download size is
//! named before the click and the download shows a progress bar), transcribe the selected clip's audio on
//! a worker thread, and turn the transcript into cues ("Transcribe & generate subtitles"). Underneath it,
//! the double-take detector lists the lines that were said more than once — "Mark on timeline" drops a
//! marker per flubbed take (described by what was said) and "Cut the duplicates" ripples every take but
//! the last one out, dragging the cues, the markers and the transcript along with the cut. With no model
//! and no whisper.exe the section only ever explains what to install.

use crate::engine::export::Progress;
use crate::engine::transcribe::{self, Segment};
use crate::model::{Id, Project};
use crate::theme::Palette;
use crate::ui::tools::{glyph_text_button, Glyph};
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
    pub min_dur: f64,
    pub words: bool,
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
    /// whisper binary, looked up once (the lookup stats PATH) — "Re-check" after installing it.
    exe: Option<Option<PathBuf>>,
}

impl Default for TranscribeState {
    fn default() -> Self {
        Self {
            open: false,
            model: transcribe::default_model(),
            language: "auto".into(),
            max_chars: 42,
            min_dur: 1.0,
            words: false,
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

#[derive(Default)]
pub struct SubtitlesState {
    pub selected: Option<crate::model::Id>,
    pub show_style: bool,
    /// Cue whose text field should grab focus (set by "Add at playhead").
    pub focus: Option<Id>,
    pub transcribe: TranscribeState,
}

#[derive(Default)]
pub struct SubtitlesResponse {
    pub edited: bool,
    pub seeked: bool,
}

/// "Add at playhead": a 2 s cue starting at the playhead.
fn add_at(project: &mut Project, playhead: f64) -> Id {
    project.add_cue(playhead, playhead + 2.0, "Subtitle")
}

/// Import parsed cues, replacing or appending.
fn apply_import(project: &mut Project, cues: &[(f64, f64, String)], replace: bool) {
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
        if ui.button("Import…").clicked() {
            import_dialog(project, &mut undone, undo, &mut resp);
        }
        if ui.button("Export SRT…").clicked() {
            export_dialog(project, false);
        }
        if ui.button("Export VTT…").clicked() {
            export_dialog(project, true);
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
    });

    if state.show_style {
        style_section(ui, project, fonts, &mut undone, undo, &mut resp);
    }
    if state.transcribe.open {
        ui.separator();
        transcribe_section(ui, &mut state.transcribe, project, selection, &mut undone, undo, &mut resp);
    }
    ui.separator();

    let mut resort = false;
    let mut del: Option<Id> = None;
    let mut split: Option<Id> = None;
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
            egui::Frame::new().fill(fill).inner_margin(2.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut v = start;
                    let r = ui.add(DragValue::new(&mut v).range(0.0..=(end - MIN_CUE)).speed(0.05).fixed_decimals(3));
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
                    let r =
                        ui.add(DragValue::new(&mut v).range((start + MIN_CUE)..=86400.0).speed(0.05).fixed_decimals(3));
                    if edit_start(&r) {
                        once(&mut undone, undo, project);
                    }
                    if r.changed() {
                        project.subtitles[i].end = v.max(start + MIN_CUE);
                        resp.edited = true;
                    }
                    if glyph_text_button(ui, Glyph::Play, "").on_hover_text("Seek to cue").clicked() {
                        *playhead = start;
                        state.selected = Some(id);
                        resp.seeked = true;
                    }
                    if active && ph > start + 0.05 && ph < end - 0.05 && ui.small_button("Split").clicked() {
                        split = Some(id);
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
            });
        }
        if project.subtitles.is_empty() {
            ui.weak("No subtitles. \"Add at playhead\" or import an .srt / .vtt file.");
        }
    });

    if let Some(id) = del {
        once(&mut undone, undo, project);
        project.remove_cue(id);
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
        note(&ui.add(DragValue::new(&mut style.size).range(8.0..=300.0)), &mut start, &mut changed);
        ui.end_row();
        ui.label("Colour");
        note(&ui.color_edit_button_srgba_unmultiplied(&mut style.color), &mut start, &mut changed);
        ui.end_row();
        ui.label("Outline");
        ui.horizontal(|ui| {
            note(&ui.color_edit_button_srgba_unmultiplied(&mut style.outline_color), &mut start, &mut changed);
            note(
                &ui.add(DragValue::new(&mut style.outline_width).range(0.0..=20.0).speed(0.1)),
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
    });
    if start {
        once(undone, undo, project);
    }
    if changed {
        project.subtitle_style = style;
        project.subtitle_margin = margin;
        resp.edited = true;
    }
}

/// What a run needs: the clip's audio and how its source time maps back onto the timeline.
struct Target {
    clip: Id,
    path: String,
    src_start: f64,
    src_dur: f64,
    /// Timeline seconds of the transcript's zero, and source seconds -> timeline seconds.
    offset: f64,
    scale: f64,
}

/// The first selected clip that has footage behind it.
///
/// ponytail: one linear map over the whole clip, so a reversed clip transcribes forwards and a keyframed
/// speed ramp drifts between its keys. Transcribe per ramp segment if that ever shows.
fn target(project: &Project, selection: &[Id]) -> Option<Target> {
    let c = selection.iter().find_map(|&id| project.clip(id))?;
    let path = project.asset(c.asset).map(|a| a.path.clone()).filter(|p| !p.is_empty())?;
    let (a, b) = (c.src_time(c.start), c.src_time(c.start + c.duration));
    let (a, b) = if b > a + 0.05 { (a, b) } else { (c.src_in, c.src_in + c.duration.max(0.1)) };
    Some(Target { clip: c.id, path, src_start: a, src_dur: b - a, offset: c.start, scale: c.duration / (b - a) })
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
fn cut_dups(project: &mut Project, st: &mut TranscribeState) -> usize {
    let ranges = transcribe::dup_ranges(&st.segments, &st.groups);
    let (Some(clip), false) = (st.clip, ranges.is_empty()) else { return 0 };
    let cuts: Vec<f64> = ranges.iter().flat_map(|&(a, b)| [a, b]).collect();
    let n = project.auto_cut(&[clip], &cuts, &ranges, true);
    for id in st.marks.drain(..) {
        project.remove_marker(id);
    }
    project.subtitles.retain_mut(|c| {
        let Some(a) = transcribe::ripple_time(c.start, &ranges) else { return false };
        let b = transcribe::ripple_time(c.end, &ranges).unwrap_or(a + (c.end - c.start));
        (c.start, c.end) = (a, b.max(a + MIN_CUE));
        true
    });
    project.sort_cues();
    st.segments.retain_mut(|s| ripple_segment(s, &ranges));
    st.groups = transcribe::duplicate_takes(&st.segments, st.threshold, TAKE_WINDOW);
    n
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
    if !have {
        // the opt-in: nothing is fetched until this button is pressed, and the size is on it
        ui.weak(format!("Downloads {mb} MB once, from huggingface.co into {}.", transcribe::models_dir().display()));
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
                if ui.button(format!("Download model ({mb} MB)")).clicked() {
                    st.download = Some(transcribe::download_model(file));
                }
            }
        }
    }
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
        ui.label("Min duration");
        ui.add(DragValue::new(&mut st.min_dur).range(0.3..=5.0).speed(0.05).suffix(" s"));
        ui.end_row();
        ui.label("Word timings");
        ui.checkbox(&mut st.words, "")
            .on_hover_text("Slower: whisper times every word, so the cues break exactly on speech");
        ui.end_row();
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
        if tgt.is_none() {
            ui.weak("Select a clip to transcribe.");
        }
    });
    if let (true, Some(t)) = (go, &tgt) {
        st.segments.clear();
        st.groups.clear();
        st.marks.clear();
        st.status.clear();
        st.clip = Some(t.clip);
        st.map = (t.offset, t.scale);
        st.job = Some(transcribe::start(transcribe::Options {
            path: t.path.clone(),
            src_start: t.src_start,
            src_duration: t.src_dur,
            model: file.to_string(),
            language: st.language.clone(),
            words: st.words,
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
                let cues = transcribe::to_cues(&segs, st.max_chars, st.min_dur);
                once(undone, undo, project);
                apply_import(project, &cues, false);
                st.segments = segs;
                st.groups = transcribe::duplicate_takes(&st.segments, st.threshold, TAKE_WINDOW);
                resp.edited = true;
                format!("{} cues from {} segments", cues.len(), st.segments.len())
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

/// Import an .srt/.vtt via rfd; asks replace (Yes) or append (No) when cues already exist.
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
    let replace = project.subtitles.is_empty()
        || rfd::MessageDialog::new()
            .set_title("Import subtitles")
            .set_description("Replace the existing subtitles? (No = append)")
            .set_buttons(rfd::MessageButtons::YesNo)
            .show()
            == rfd::MessageDialogResult::Yes;
    once(undone, undo, project);
    apply_import(project, &cues, replace);
    resp.edited = true;
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
        // the transcribe section draws too: no model, no whisper.exe, no selection — only its hints
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
        });
        p.insert_asset_clips(aid, 0.0, Some(0));
        let id = p.tracks[0].clips[0].id;
        if speed != 1.0 {
            p.set_speed(&[id], speed, false);
        }
        (p, id)
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
