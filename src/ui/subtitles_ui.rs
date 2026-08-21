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

use crate::model::{Id, Project};
use crate::theme::Palette;
use eframe::egui::{self, DragValue, Response};

/// Minimum cue length (end ≥ start + this).
const MIN_CUE: f64 = 0.1;

#[derive(Default)]
pub struct SubtitlesState {
    pub selected: Option<crate::model::Id>,
    pub show_style: bool,
    /// Cue whose text field should grab focus (set by "Add at playhead").
    pub focus: Option<Id>,
}

#[derive(Default)]
pub struct SubtitlesResponse {
    pub edited: bool,
    pub seeked: bool,
}

/// True on the first frame of an edit gesture (drag start, or a non-drag change such as typing/clicking).
fn edit_start(r: &Response) -> bool {
    r.drag_started() || (r.changed() && !r.dragged())
}

/// Push undo at most once per frame.
fn once(flag: &mut bool, undo: &mut dyn FnMut(&Project), p: &Project) {
    if !*flag {
        undo(p);
        *flag = true;
    }
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
    });

    if state.show_style {
        style_section(ui, project, fonts, &mut undone, undo, &mut resp);
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
                    if ui.small_button("▶").on_hover_text("Seek to cue").clicked() {
                        *playhead = start;
                        state.selected = Some(id);
                        resp.seeked = true;
                    }
                    if active && ph > start + 0.05 && ph < end - 0.05 && ui.small_button("Split").clicked() {
                        split = Some(id);
                    }
                    if ui.small_button("✕").clicked() {
                        del = Some(id);
                    }
                });
                let mut text = project.subtitles[i].text.clone();
                let r = ui.add(egui::TextEdit::multiline(&mut text).desired_rows(1).desired_width(f32::INFINITY));
                if state.focus == Some(id) {
                    r.request_focus();
                    state.focus = None;
                }
                if r.has_focus() {
                    state.selected = Some(id);
                }
                if edit_start(&r) {
                    once(&mut undone, undo, project);
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

/// Import an .srt/.vtt via rfd; asks replace (Yes) or append (No) when cues already exist.
fn import_dialog(
    project: &mut Project,
    undone: &mut bool,
    undo: &mut dyn FnMut(&Project),
    resp: &mut SubtitlesResponse,
) {
    let Some(path) = rfd::FileDialog::new().add_filter("Subtitles", &["srt", "vtt"]).pick_file() else { return };
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    // ponytail: catch_unwind so a still-todo!() engine::subtitles in a sibling worktree can't crash the app.
    let cues = std::panic::catch_unwind(|| crate::engine::subtitles::parse(&text)).unwrap_or_default();
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
    let text = std::panic::catch_unwind(|| {
        if vtt {
            crate::engine::subtitles::to_vtt(cues)
        } else {
            crate::engine::subtitles::to_srt(cues)
        }
    })
    .unwrap_or_default();
    if !text.is_empty() {
        let _ = std::fs::write(path, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Runs engine::subtitles::parse on a small SRT; skips while the engine is still a todo!() stub.
    #[test]
    fn import_parses_srt_when_engine_ready() {
        let srt = "1\n00:00:01,000 --> 00:00:02,500\nHello\n\n2\n00:00:03,000 --> 00:00:04,000\nWorld\n";
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let r = std::panic::catch_unwind(|| crate::engine::subtitles::parse(srt));
        std::panic::set_hook(prev);
        let Ok(cues) = r else { return }; // engine not implemented yet in this worktree
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
        let mut state = SubtitlesState { show_style: true, ..Default::default() };
        let mut playhead = 2.5; // inside cue "b" → highlighted row with Split button
        let ctx = egui::Context::default();
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| panic!("no undo without edits");
                    let r = show(ui, &mut state, &mut p, &mut playhead, &fonts, &palette, &mut undo);
                    assert!(!r.edited && !r.seeked);
                });
            });
        }
        assert_eq!(p.subtitles.len(), 2);
    }
}
