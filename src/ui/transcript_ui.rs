//! ---- ws:transcript-captions ----
//! The Transcript section of the Subtitles pane (a collapsible section, not a pane): the words of one
//! transcribed clip as a selectable run - click a word to seek, drag (or Shift+click) to select a
//! range, Delete / "Cut selected words" to ripple-cut it through `Project::cut_word_ranges` - a
//! search box with Prev/Next jumping across EVERY transcribed clip (`transcript_hits`), the editable
//! filler-word chips with Mark-instead-first ("Mark fillers" drops a range marker per hit; "Remove
//! fillers" only lights up once marks exist) and a small Speech (TTS) panel. Also the non-blocking
//! "View transcript" `egui::Window` the clip menu opens.
//!
//! This module has no `App`: app-level requests (start a transcription, speak a line) are left in
//! `TranscriptUiState` and drained by `ui::app::transcript_ctl::tick`; edits go through the same
//! `once(undone, undo, project)` rule as the rest of the Subtitles pane.

use crate::engine::transcribe;
use crate::model::ops::subtitles::transcript_hits;
use crate::model::{Id, Project};
use crate::theme::Palette;
use crate::ui::{duration_text, once};
use eframe::egui::{self, Button, DragValue, RichText, TextEdit};

/// Word count above which the section stops laying out every word (a long interview would cost a
/// widget per word per frame) and asks for the search box / window instead.
const MAX_INLINE_WORDS: usize = 4000;

#[derive(Default)]
pub struct TranscriptUiState {
    /// Transcript shown (clip id); resolved against the selection / the first transcript each frame.
    pub clip: Option<Id>,
    /// Selected word range `[a, b]` (inclusive indices into the shown transcript).
    pub sel: Option<(usize, usize)>,
    anchor: Option<usize>,
    dragging: bool,
    pub query: String,
    hit: usize,
    /// Mirror of `Settings.filler_words` / `filler_pad_ms`, two-way synced by `transcript_ctl::tick`
    /// (`fillers_dirty` = the chips changed here, write them back).
    pub fillers: Vec<String>,
    pub filler_pad_ms: u32,
    pub fillers_dirty: bool,
    new_filler: String,
    /// Range markers dropped by "Mark fillers" (taken away again by Remove / Clear marks).
    pub filler_marks: Vec<Id>,
    pub status: String,
    /// "Transcribe selected clip" - drained by the app.
    pub want_transcribe: Option<Id>,
    /// (text, voice) from the Speech panel's Speak - drained by the app.
    pub tts_request: Option<(String, Option<String>)>,
    pub show_tts: bool,
    tts_text: String,
    tts_voice: String,
    /// Installed voices, filled by the app the first time the Speech panel is open (`engine::tts::
    /// voices()` is a one-second powershell call - never at startup, never here).
    pub tts_voices: Option<Vec<String>>,
}

#[derive(Default)]
pub struct TranscriptResponse {
    pub edited: bool,
    pub seeked: bool,
    /// Words removed by a cut this frame (for the app's toast).
    pub cut: usize,
}

/// "clip name" for a transcript's clip id, or "clip #id" once the clip is gone.
pub fn clip_label(project: &Project, clip: Id) -> String {
    match project.clip(clip) {
        Some(c) if !c.name.trim().is_empty() => c.name.clone(),
        _ => format!("clip #{clip}"),
    }
}

/// Words `[a, b]` of the shown transcript as ONE timeline span (the pauses between them go too -
/// selecting "um … um" means "this stretch of speech").
fn span(words: &[(f64, f64, String)], (a, b): (usize, usize)) -> Option<(f64, f64)> {
    Some((words.get(a)?.0, words.get(b)?.1))
}

/// A range marker per filler hit, named by the words inside it (mirrors the double-take marks).
/// Shared with `Action::RemoveFillers` and `transcript.remove_fillers`'s Mark-instead path.
pub fn mark_ranges(project: &mut Project, words: &[(f64, f64, String)], ranges: &[(f64, f64)]) -> Vec<Id> {
    let mut ids = Vec::new();
    for &(a, b) in ranges {
        let said: Vec<&str> =
            words.iter().filter(|w| w.0 >= a - 1e-6 && w.0 < b).map(|w| w.2.as_str()).collect();
        let text = said.join(" ");
        let id = project.add_marker(a, transcribe::short_label(&text, 28));
        if let Some(m) = project.marker_mut(id) {
            m.duration = (b - a).max(0.0);
            m.note = text;
        }
        ids.push(id);
    }
    ids
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    st: &mut TranscriptUiState,
    project: &mut Project,
    playhead: &mut f64,
    selection: &[Id],
    palette: &Palette,
    undone: &mut bool,
    undo: &mut dyn FnMut(&Project),
) -> TranscriptResponse {
    let mut resp = TranscriptResponse::default();
    let ph = *playhead;
    // which transcript: the shown one while it exists, else the selection's, else the first
    let existing: Vec<Id> =
        project.transcripts.iter().map(|t| t.clip).filter(|&c| project.clip(c).is_some()).collect();
    if st.clip.is_none_or(|c| !existing.contains(&c)) {
        st.clip = selection.iter().find(|id| existing.contains(id)).copied().or(existing.first().copied());
        st.sel = None;
        st.anchor = None;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label("Clip");
        let current = st.clip.map(|c| clip_label(project, c)).unwrap_or_else(|| "none transcribed".into());
        egui::ComboBox::from_id_salt("transcript_clip").selected_text(current).show_ui(ui, |ui| {
            for &c in &existing {
                if ui.selectable_value(&mut st.clip, Some(c), clip_label(project, c)).changed() {
                    st.sel = None;
                    st.anchor = None;
                }
            }
        });
        if let Some(&sel) = selection.first() {
            if !existing.contains(&sel) && project.clip(sel).is_some_and(|c| c.uses_asset()) {
                if ui.small_button("Transcribe selected clip").clicked() {
                    st.want_transcribe = Some(sel);
                }
            }
        }
        if let Some(n) = st.clip.and_then(|c| project.transcript(c)).map(|t| t.words.len()) {
            ui.weak(format!("{n} words"));
        }
    });

    // search across every transcript
    ui.horizontal(|ui| {
        let r = ui.add(TextEdit::singleline(&mut st.query).desired_width(150.0).hint_text("Find a word…"));
        let hits = if st.query.trim().is_empty() { Vec::new() } else { transcript_hits(&project.transcripts, &st.query) };
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let mut jump: Option<usize> = None;
        if r.changed() {
            st.hit = 0;
        }
        ui.add_enabled_ui(!hits.is_empty(), |ui| {
            if ui.small_button("◀").on_hover_text("Previous hit").clicked() {
                jump = Some((st.hit + hits.len().max(1) - 1) % hits.len().max(1));
            }
            if ui.small_button("▶").on_hover_text("Next hit").clicked() || (enter && !hits.is_empty()) {
                jump = Some(if r.changed() || enter { st.hit } else { (st.hit + 1) % hits.len().max(1) });
            }
        });
        if !st.query.trim().is_empty() {
            ui.weak(format!("{} hit(s)", hits.len()));
        }
        if let Some(j) = jump.filter(|&j| j < hits.len()) {
            let (clip, i, t) = hits[j];
            st.hit = j;
            st.clip = Some(clip);
            st.sel = Some((i, i));
            st.anchor = Some(i);
            *playhead = t;
            resp.seeked = true;
        }
    });

    let Some(clip) = st.clip else {
        ui.weak("No transcript yet - select a clip and Transcribe (or right-click it ▸ Transcript ▸ Transcribe…).");
        tts_panel(ui, st);
        return resp;
    };
    let words: Vec<(f64, f64, String)> = project.transcript(clip).map(|t| t.words.clone()).unwrap_or_default();
    if let Some((a, b)) = st.sel {
        if b >= words.len() || a > b {
            st.sel = None;
        }
    }

    // the word run
    let shift = ui.input(|i| i.modifiers.shift);
    let primary_down = ui.input(|i| i.pointer.primary_down());
    let mut clicked: Option<usize> = None;
    let mut pressed: Option<usize> = None;
    let mut under: Option<usize> = None;
    let mut any_focus = false;
    if words.len() > MAX_INLINE_WORDS {
        ui.weak(format!("{} words - too many to lay out inline; use the search box or View transcript.", words.len()));
    } else {
        egui::ScrollArea::vertical().id_salt("transcript_words").max_height(170.0).auto_shrink([false, true]).show(
            ui,
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    for (i, w) in words.iter().enumerate() {
                        let selected = st.sel.is_some_and(|(a, b)| i >= a && i <= b);
                        let active = ph >= w.0 && ph < w.1;
                        let text = if active { RichText::new(&w.2).color(palette.accent).strong() } else { RichText::new(&w.2) };
                        let r = ui.selectable_label(selected, text);
                        if r.contains_pointer() {
                            under = Some(i);
                        }
                        if r.is_pointer_button_down_on() {
                            pressed = Some(i);
                        }
                        if r.clicked() {
                            clicked = Some(i);
                            r.request_focus();
                        }
                        any_focus |= r.has_focus();
                        r.on_hover_text(format!("{} – {}", duration_text(w.0), duration_text(w.1)));
                    }
                });
            },
        );
    }
    // press starts a range at the anchor, holding the button over other words extends it
    if let Some(i) = pressed {
        if !st.dragging {
            st.dragging = true;
            st.anchor = Some(i);
        }
    }
    if st.dragging && primary_down {
        if let (Some(a), Some(h)) = (st.anchor, under) {
            st.sel = Some((a.min(h), a.max(h)));
        }
    }
    if !primary_down {
        st.dragging = false;
    }
    if let Some(i) = clicked {
        match (shift, st.anchor) {
            (true, Some(a)) => st.sel = Some((a.min(i), a.max(i))),
            _ => {
                st.sel = Some((i, i));
                st.anchor = Some(i);
                if let Some(w) = words.get(i) {
                    *playhead = w.0;
                    resp.seeked = true;
                }
            }
        }
    }
    let n_sel = st.sel.map_or(0, |(a, b)| b - a + 1);
    let mut do_cut = false;
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(n_sel > 0, Button::new(format!("Cut selected words ({n_sel})")))
            .on_hover_text("Ripple-cut the selected stretch of speech; cues, markers and the transcript follow (Delete)")
            .clicked()
        {
            do_cut = true;
        }
        if n_sel > 0 && ui.small_button("Deselect").clicked() {
            st.sel = None;
        }
        if !st.status.is_empty() {
            ui.weak(&st.status);
        }
    });
    // Delete while a word has keyboard focus (focus keeps the timeline's own Delete hotkey quiet -
    // `Hotkeys::poll` yields nothing while `wants_keyboard_input`)
    if any_focus && n_sel > 0 && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)) {
        do_cut = true;
    }
    if do_cut {
        if let Some(range) = st.sel.and_then(|s| span(&words, s)) {
            // snapshot-then-rollback (mirrors transcript_ctl::remove_fillers_action): only a real cut
            // earns an undo entry, so a no-op (locked track, speed ramp, empty range) neither pushes a
            // spurious undo step nor destroys the redo stack.
            let before = project.clone();
            let n = project.cut_word_ranges(clip, &[range]);
            st.status = if n > 0 {
                once(undone, undo, &before);
                resp.edited = true;
                resp.cut = n_sel;
                format!("cut {n_sel} word(s)")
            } else {
                *project = before;
                "nothing cut - the track is locked, or the clip has a speed ramp".into()
            };
        }
        st.sel = None;
        st.anchor = None;
    }

    // fillers: Mark-instead first
    ui.horizontal_wrapped(|ui| {
        ui.label("Fillers").on_hover_text("Words to cut out - case and punctuation don't matter; phrases allowed");
        let mut remove: Option<usize> = None;
        for (i, f) in st.fillers.iter().enumerate() {
            if ui.small_button(format!("{f} ×")).on_hover_text("Remove from the list").clicked() {
                remove = Some(i);
            }
        }
        if let Some(i) = remove {
            st.fillers.remove(i);
            st.fillers_dirty = true;
        }
        let r = ui.add(TextEdit::singleline(&mut st.new_filler).desired_width(70.0).hint_text("add…"));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.small_button("Add").clicked() || enter) && !st.new_filler.trim().is_empty() {
            let f = st.new_filler.trim().to_lowercase();
            if !st.fillers.contains(&f) {
                st.fillers.push(f);
                st.fillers_dirty = true;
            }
            st.new_filler.clear();
        }
        if ui.add(DragValue::new(&mut st.filler_pad_ms).range(0..=500).suffix(" ms pad")).changed() {
            st.fillers_dirty = true;
        }
    });
    let fillers: Vec<&str> = st.fillers.iter().map(String::as_str).collect();
    let ranges = transcribe::filler_ranges(&words, &fillers, st.filler_pad_ms);
    ui.horizontal_wrapped(|ui| {
        ui.weak(format!("{} filler(s) found", ranges.len()));
        if ui
            .add_enabled(!ranges.is_empty(), Button::new("Mark fillers"))
            .on_hover_text("Drop a range marker on every filler first - look them over, then Remove")
            .clicked()
        {
            once(undone, undo, project);
            for id in st.filler_marks.drain(..) {
                project.remove_marker(id);
            }
            st.filler_marks = mark_ranges(project, &words, &ranges);
            st.status = format!("marked {} filler(s)", st.filler_marks.len());
            resp.edited = true;
        }
        let can_remove = !st.filler_marks.is_empty() && !ranges.is_empty();
        if ui
            .add_enabled(can_remove, Button::new("Remove fillers"))
            .on_hover_text("Ripple-cut every marked filler (Mark fillers first)")
            .clicked()
        {
            // snapshot-then-rollback (mirrors transcript_ctl::remove_fillers_action): if the cut turns
            // out to be a no-op, restore the project so the marks removed just above come back too,
            // instead of leaving them gone with no way back.
            let before = project.clone();
            for id in st.filler_marks.drain(..) {
                project.remove_marker(id);
            }
            let n = project.cut_word_ranges(clip, &ranges);
            st.status = if n > 0 {
                once(undone, undo, &before);
                resp.edited = true;
                resp.cut = ranges.len();
                format!("removed {} filler(s)", ranges.len())
            } else {
                *project = before;
                "nothing cut - the track is locked, or the clip has a speed ramp".into()
            };
            st.sel = None;
        }
        if !st.filler_marks.is_empty() && ui.small_button("Clear marks").clicked() {
            once(undone, undo, project);
            for id in st.filler_marks.drain(..) {
                project.remove_marker(id);
            }
            resp.edited = true;
        }
    });
    tts_panel(ui, st);
    resp
}

/// Speech (TTS): a line of text, an installed SAPI voice, Speak → the app synthesizes a WAV and
/// places it at the playhead (linked to the selected text clip, if any).
fn tts_panel(ui: &mut egui::Ui, st: &mut TranscriptUiState) {
    ui.toggle_value(&mut st.show_tts, "Speech (TTS)").on_hover_text("Windows' own voices, via System.Speech");
    if !st.show_tts {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.add(TextEdit::singleline(&mut st.tts_text).desired_width(220.0).hint_text("Text to speak…"));
        let shown = if st.tts_voice.is_empty() { "(default voice)".to_string() } else { st.tts_voice.clone() };
        egui::ComboBox::from_id_salt("tts_voice").selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(&mut st.tts_voice, String::new(), "(default voice)");
            match &st.tts_voices {
                Some(v) => {
                    for name in v {
                        ui.selectable_value(&mut st.tts_voice, name.clone(), name);
                    }
                }
                None => {
                    ui.weak("listing voices…");
                }
            }
        });
        if ui
            .add_enabled(!st.tts_text.trim().is_empty(), Button::new("Speak at playhead"))
            .on_hover_text("Writes a WAV with an OS voice and imports it at the playhead")
            .clicked()
        {
            let voice = (!st.tts_voice.is_empty()).then(|| st.tts_voice.clone());
            st.tts_request = Some((st.tts_text.trim().to_string(), voice));
        }
    });
}

// ---------------------------------------------------------------- "View transcript" window

/// The clip menu's "View transcript" window: every word with its timestamp, a filter box, click =
/// seek. Non-blocking (`egui::Window`), one per app.
#[derive(Default)]
pub struct TranscriptWindow {
    pub open: bool,
    pub clip: Id,
    pub query: String,
}

#[derive(Default)]
pub struct WindowResponse {
    pub seek: Option<f64>,
    /// "No transcript yet - Transcribe…" was clicked.
    pub transcribe: bool,
}

pub fn window(ctx: &egui::Context, st: &mut TranscriptWindow, project: &Project, playhead: f64) -> WindowResponse {
    let mut resp = WindowResponse::default();
    if !st.open {
        return resp;
    }
    let mut open = true;
    let title = format!("Transcript - {}", clip_label(project, st.clip));
    egui::Window::new(title)
        .id(egui::Id::new("transcript_window"))
        .open(&mut open)
        .default_width(380.0)
        .default_height(320.0)
        .show(ctx, |ui| match project.transcript(st.clip) {
            None => {
                ui.horizontal(|ui| {
                    ui.label("No transcript yet - ");
                    if ui.button("Transcribe…").clicked() {
                        resp.transcribe = true;
                    }
                });
            }
            Some(tr) => {
                ui.horizontal(|ui| {
                    ui.add(TextEdit::singleline(&mut st.query).desired_width(180.0).hint_text("Filter words…"));
                    ui.weak(format!("{} words", tr.words.len()));
                });
                let q = st.query.trim().to_lowercase();
                let rows: Vec<usize> = tr
                    .words
                    .iter()
                    .enumerate()
                    .filter(|(_, w)| q.is_empty() || w.2.to_lowercase().contains(&q))
                    .map(|(i, _)| i)
                    .collect();
                let row_h = ui.spacing().interact_size.y;
                egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_h, rows.len(), |ui, range| {
                    for &i in &rows[range] {
                        let w = &tr.words[i];
                        ui.horizontal(|ui| {
                            let active = playhead >= w.0 && playhead < w.1;
                            if ui.small_button(duration_text(w.0)).on_hover_text("Seek here").clicked() {
                                resp.seek = Some(w.0);
                            }
                            let text = if active { RichText::new(&w.2).strong() } else { RichText::new(&w.2) };
                            if ui.add(egui::Label::new(text).sense(egui::Sense::click())).clicked() {
                                resp.seek = Some(w.0);
                            }
                        });
                    }
                });
            }
        });
    if !open {
        st.open = false;
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, AudioStreamInfo, ClipKind};
    use eframe::egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, vec2};

    fn words(list: &[(f64, f64, &str)]) -> Vec<(f64, f64, String)> {
        list.iter().map(|(a, b, w)| (*a, *b, w.to_string())).collect()
    }

    /// A clip with an asset on V1 + A1 (0..10 s) and a five-word transcript.
    fn project() -> (Project, Id) {
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
        p.set_transcript(
            id,
            words(&[(0.0, 0.4, "Welcome"), (1.0, 1.4, "um"), (2.0, 2.4, "to"), (3.0, 3.4, "the"), (4.0, 4.4, "show")]),
        );
        (p, id)
    }

    fn screen() -> RawInput {
        RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 500.0))), ..Default::default() }
    }

    fn state() -> TranscriptUiState {
        TranscriptUiState {
            fillers: transcribe::FILLER_WORDS.iter().map(|s| s.to_string()).collect(),
            filler_pad_ms: 120,
            ..Default::default()
        }
    }

    /// 30 idle frames with a populated transcript, nothing pressed: no repaint, no edit, no seek.
    #[test]
    fn transcript_ui_headless_no_input_no_repaint() {
        let (mut p, id) = project();
        let mut st = state();
        let palette = Palette::new(true, egui::Color32::WHITE);
        let mut playhead = 2.2;
        let ctx = egui::Context::default();
        for _ in 0..30 {
            let _ = ctx.run(screen(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undone = false;
                    let mut undo = |_: &Project| panic!("no undo without edits");
                    let r = show(ui, &mut st, &mut p, &mut playhead, &[id], &palette, &mut undone, &mut undo);
                    assert!(!r.edited && !r.seeked && r.cut == 0);
                });
            });
        }
        assert!(!ctx.has_requested_repaint(), "idle Transcript section requested a repaint");
        assert_eq!(st.clip, Some(id), "the selected clip's transcript is shown");
        assert_eq!(p.transcript(id).unwrap().words.len(), 5);
        assert!(st.want_transcribe.is_none() && st.tts_request.is_none());
    }

    /// The "View transcript" window: 30 idle frames open over a transcript request no repaint; with no
    /// transcript it shows the "No transcript yet" hint instead of an empty list.
    #[test]
    fn transcript_window_no_idle_repaint_and_empty_hint() {
        let (p, id) = project();
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut st = TranscriptWindow { open: true, clip: id, query: String::new() };
        for _ in 0..30 {
            let _ = ctx.run(screen(), |ctx| {
                let r = window(ctx, &mut st, &p, 1.1);
                assert!(r.seek.is_none() && !r.transcribe);
            });
        }
        assert!(!ctx.has_requested_repaint(), "idle transcript window requested a repaint");
        assert!(st.open);
        // a clip without a transcript
        let mut st = TranscriptWindow { open: true, clip: 424242, query: String::new() };
        let full = ctx.run(screen(), |ctx| {
            let _ = window(ctx, &mut st, &p, 0.0);
        });
        let texts: Vec<String> = full
            .shapes
            .iter()
            .filter_map(|cs| match &cs.shape {
                Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect();
        assert!(texts.iter().any(|s| s.contains("No transcript yet")), "{texts:?}");
        assert!(texts.iter().any(|s| s == "Transcribe…"), "{texts:?}");
    }

    /// Clicking a word seeks to it; Delete over a selected word ripple-cuts it out of the clip (and
    /// the transcript) in one undo step.
    #[test]
    fn word_click_seeks_and_delete_cuts() {
        let (mut p, id) = project();
        let mut st = state();
        let palette = Palette::new(true, egui::Color32::WHITE);
        let mut playhead = 0.0;
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut undos = 0usize;
        let mut run = |events: Vec<Event>, st: &mut TranscriptUiState, p: &mut Project, playhead: &mut f64, undos: &mut usize| {
            let mut resp = TranscriptResponse::default();
            let full = ctx.run(RawInput { events, ..screen() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undone = false;
                    let mut undo = |_: &Project| *undos += 1;
                    resp = show(ui, st, p, playhead, &[id], &palette, &mut undone, &mut undo);
                });
            });
            (resp, full)
        };
        // find where "um" was painted
        let (_, full) = run(vec![], &mut st, &mut p, &mut playhead, &mut undos);
        let pos = full
            .shapes
            .iter()
            .find_map(|cs| match &cs.shape {
                Shape::Text(t) if t.galley.text() == "um" => Some(t.pos),
                _ => None,
            })
            .expect("the word 'um' is laid out");
        let click = pos + vec2(3.0, 4.0);
        let mut seeked = false;
        for pressed in [true, false] {
            let ev = Event::PointerButton { pos: click, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
            let (r, _) = run(vec![ev], &mut st, &mut p, &mut playhead, &mut undos);
            seeked |= r.seeked;
        }
        assert!(seeked, "clicking a word seeks");
        assert!((playhead - 1.0).abs() < 1e-9, "to the word's start: {playhead}");
        assert_eq!(st.sel, Some((1, 1)));
        // Delete: the word (and its 0.4 s) leave the clip, the transcript follows
        let key = |pressed| Event::Key { key: egui::Key::Delete, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
        let (r, _) = run(vec![key(true), key(false)], &mut st, &mut p, &mut playhead, &mut undos);
        assert!(r.edited && r.cut == 1, "cut: {} edited: {}", r.cut, r.edited);
        assert_eq!(undos, 1, "one undo step");
        assert!((p.duration() - 9.6).abs() < 1e-6, "{}", p.duration());
        let text: Vec<&str> = p.transcript(id).unwrap().words.iter().map(|w| w.2.as_str()).collect();
        assert_eq!(text, vec!["Welcome", "to", "the", "show"]);
        assert!((p.transcript(id).unwrap().words[1].0 - 1.6).abs() < 1e-6, "'to' moved up by 0.4 s");
        assert!(st.sel.is_none());
    }

    /// Mark-instead-first: "Mark fillers" drops a range marker per hit and only then does "Remove
    /// fillers" cut - which also takes the marks away.
    #[test]
    fn fillers_are_marked_then_removed() {
        let (mut p, id) = project();
        let mut st = state();
        st.clip = Some(id);
        let words = p.transcript(id).unwrap().words.clone();
        let fillers: Vec<&str> = st.fillers.iter().map(String::as_str).collect();
        let ranges = transcribe::filler_ranges(&words, &fillers, st.filler_pad_ms);
        assert_eq!(ranges.len(), 1, "{ranges:?}");
        st.filler_marks = mark_ranges(&mut p, &words, &ranges);
        assert_eq!(st.filler_marks.len(), 1);
        let m = p.markers.iter().find(|m| m.id == st.filler_marks[0]).unwrap();
        assert_eq!(m.note, "um");
        assert!((m.t - 0.88).abs() < 1e-9 && (m.duration - 0.64).abs() < 1e-9, "{m:?}");
        // the remove path the button runs
        for mid in st.filler_marks.drain(..) {
            p.remove_marker(mid);
        }
        let n = p.cut_word_ranges(id, &ranges);
        assert!(n > 0);
        assert!(p.markers.is_empty());
        assert_eq!(p.transcript(id).unwrap().words.len(), 4);
        assert!((p.duration() - (10.0 - 0.64)).abs() < 1e-6, "{}", p.duration());
    }

    #[test]
    fn span_and_labels() {
        let w = words(&[(0.0, 0.4, "a"), (1.0, 1.4, "b"), (2.0, 2.4, "c")]);
        assert_eq!(span(&w, (0, 2)), Some((0.0, 2.4)));
        assert_eq!(span(&w, (1, 1)), Some((1.0, 1.4)));
        assert_eq!(span(&w, (1, 7)), None);
        let (p, id) = project();
        assert_eq!(clip_label(&p, id), p.clip(id).unwrap().name);
        assert_eq!(clip_label(&p, 999), "clip #999");
    }
}
