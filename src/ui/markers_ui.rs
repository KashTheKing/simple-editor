//! Markers pane: every marker in timeline order (project markers and clip markers together, via
//! `Project::markers_in_timeline`). Rows: colour label dot (menu), time (editable DragValue, seconds),
//! duration for ranges, name, note (multiline when the row is selected), "Go" (seek) and X (delete).
//! Toolbar: "Add at playhead" (M), "Add on selected clip", filter by label, and "Copy as list" (markdown,
//! handy for the AI tools). Undo once per gesture; returns what changed.

use crate::model::{Id, Project};
use crate::theme::Palette;
use crate::ui::timecode;
use eframe::egui::{self, DragValue, Response, RichText};

#[derive(Default)]
pub struct MarkersState {
    pub selected: Option<Id>,
    /// 0 = every label.
    pub filter_label: u8,
}

#[derive(Default)]
pub struct MarkersResponse {
    pub edited: bool,
    pub seek: Option<f64>,
}

/// True on the first frame of an edit gesture (drag start, or a click/typing change).
fn edit_start(r: &Response) -> bool {
    r.drag_started() || (r.changed() && !r.dragged())
}

/// A small red-on-hover ✕ (same shape as the planner / inspector delete buttons).
pub(crate) fn x_button(ui: &mut egui::Ui) -> Response {
    let r = ui.small_button("✕");
    if r.hovered() {
        let c = egui::Color32::from_rgb(220, 70, 70);
        ui.painter().rect_stroke(r.rect, 2.0, egui::Stroke::new(1.0, c), egui::StrokeKind::Inside);
        ui.painter().text(
            r.rect.center(),
            egui::Align2::CENTER_CENTER,
            "✕",
            egui::TextStyle::Button.resolve(ui.style()),
            c,
        );
    }
    r
}

/// Test-only: remember a widget rect so headless tests can click the real button.
#[cfg(test)]
fn mark(ui: &egui::Ui, name: impl std::fmt::Display, r: &Response) {
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("mk", name.to_string())), r.rect));
}
#[cfg(not(test))]
fn mark(_ui: &egui::Ui, _name: impl std::fmt::Display, _r: &Response) {}

/// Where a marker lives: the project timeline, or a clip (with its start offset).
struct Row {
    id: Id,
    /// Timeline seconds.
    abs_t: f64,
    /// 0 for project markers.
    offset: f64,
    clip: Option<Id>,
}

fn rows(project: &Project, filter: u8) -> Vec<Row> {
    let mut v: Vec<Row> = project
        .markers
        .iter()
        .filter(|m| filter == 0 || m.label == filter)
        .map(|m| Row { id: m.id, abs_t: m.t, offset: 0.0, clip: None })
        .collect();
    for (_, c) in project.all_clips() {
        for m in c.markers.iter().filter(|m| filter == 0 || m.label == filter) {
            v.push(Row { id: m.id, abs_t: c.start + m.t, offset: c.start, clip: Some(c.id) });
        }
    }
    v.sort_by(|a, b| a.abs_t.total_cmp(&b.abs_t));
    v
}

/// Markdown list of every marker in timeline order (for notes / the AI tools).
fn as_markdown(project: &Project, fps: f64) -> String {
    let mut s = String::new();
    for (_, t, dur, name, _) in project.markers_in_timeline() {
        let name = if name.is_empty() { "(marker)".to_string() } else { name };
        if dur > 0.0 {
            s.push_str(&format!("- {} → {}  {name}\n", timecode(t, fps), timecode(t + dur, fps)));
        } else {
            s.push_str(&format!("- {}  {name}\n", timecode(t, fps)));
        }
    }
    s
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut MarkersState,
    project: &mut Project,
    selection: &[Id],
    playhead: f64,
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> MarkersResponse {
    let mut out = MarkersResponse::default();
    let fps = project.fps;
    let labels: Vec<(String, [u8; 3])> = project.labels.iter().map(|l| (l.name.clone(), l.color)).collect();

    // ---- toolbar ----
    let sel_clip = selection.iter().copied().find(|&id| project.clip(id).is_some());
    ui.horizontal_wrapped(|ui| {
        let r = ui.button("Add at playhead").on_hover_text("M");
        mark(ui, "add", &r);
        if r.clicked() {
            undo(project);
            state.selected = Some(project.add_marker(playhead, ""));
            out.edited = true;
        }
        let on_clip = sel_clip.is_some();
        if ui
            .add_enabled(on_clip, egui::Button::new("Add on selected clip"))
            .on_disabled_hover_text("Select a clip first")
            .clicked()
        {
            if let Some(cid) = sel_clip {
                let local = project.clip(cid).map(|c| (playhead - c.start).clamp(0.0, c.duration)).unwrap_or(0.0);
                undo(project);
                state.selected = project.add_clip_marker(cid, local, "");
                out.edited = true;
            }
        }
        egui::ComboBox::from_id_salt("marker_filter")
            .selected_text(if state.filter_label == 0 {
                "All labels".to_string()
            } else {
                project.label_name(state.filter_label).to_string()
            })
            .width(110.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.filter_label, 0, "All labels");
                for (i, (name, [r, g, b])) in labels.iter().enumerate() {
                    let t = RichText::new(format!("● {name}")).color(egui::Color32::from_rgb(*r, *g, *b));
                    ui.selectable_value(&mut state.filter_label, i as u8 + 1, t);
                }
            });
        if ui.button("Copy as list").on_hover_text("Markdown, for notes and the AI tools").clicked() {
            ui.ctx().copy_text(as_markdown(project, fps));
        }
    });
    ui.separator();

    let rows = rows(project, state.filter_label);
    if rows.is_empty() {
        ui.weak("No markers — press M to add one at the playhead");
        return out;
    }

    let mut remove: Option<Id> = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for row in &rows {
            // ponytail: edit a clone of the one marker and write it back — lets `undo` snapshot the
            // untouched project without holding a mutable borrow across the widgets.
            let Some(orig) = project.marker_mut(row.id).map(|m| m.clone()) else { continue };
            let mut m = orig.clone();
            let selected = state.selected == Some(row.id);
            let (mut start, mut changed) = (false, false);
            ui.horizontal(|ui| {
                // label dot + menu
                let color = match project.label_color(m.label) {
                    Some([r, g, b]) => egui::Color32::from_rgb(r, g, b),
                    None => palette.text_dim,
                };
                let dot = ui.button(RichText::new("●").color(color));
                dot.context_menu(|ui| {
                    if ui.selectable_label(m.label == 0, "None").clicked() {
                        m.label = 0;
                        start = true;
                        changed = true;
                        ui.close();
                    }
                    for (i, (name, [r, g, b])) in labels.iter().enumerate() {
                        let t = RichText::new(format!("● {name}")).color(egui::Color32::from_rgb(*r, *g, *b));
                        if ui.selectable_label(m.label == i as u8 + 1, t).clicked() {
                            m.label = i as u8 + 1;
                            start = true;
                            changed = true;
                            ui.close();
                        }
                    }
                });
                let mut t = m.t;
                let r = ui.add(DragValue::new(&mut t).speed(0.05).range(0.0..=1e6).suffix(" s").fixed_decimals(2));
                if r.changed() {
                    m.t = t;
                }
                start |= edit_start(&r);
                changed |= r.changed();
                if m.duration > 0.0 {
                    let r = ui.add(DragValue::new(&mut m.duration).speed(0.05).range(0.0..=1e6).prefix("+"));
                    start |= edit_start(&r);
                    changed |= r.changed();
                }
                let w = (ui.available_width() - 70.0).max(60.0);
                let r = ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(w).hint_text("marker"));
                if r.has_focus() || r.clicked() {
                    state.selected = Some(row.id);
                }
                start |= r.gained_focus();
                changed |= r.changed();
                let go = ui.small_button("Go").on_hover_text(timecode(row.abs_t, fps));
                mark(ui, format_args!("go{}", row.id), &go);
                if go.clicked() {
                    state.selected = Some(row.id);
                    out.seek = Some(m.t + row.offset);
                }
                let del = x_button(ui).on_hover_text("Delete marker");
                mark(ui, format_args!("del{}", row.id), &del);
                if del.clicked() {
                    remove = Some(row.id);
                }
            });
            if selected {
                let r = ui.add(
                    egui::TextEdit::multiline(&mut m.note)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .hint_text("note"),
                );
                start |= r.gained_focus();
                changed |= r.changed();
            }
            if start {
                undo(project);
            }
            if changed {
                if row.clip.is_some() {
                    // clip markers stay inside their clip
                    if let Some(c) = row.clip.and_then(|cid| project.clip(cid)) {
                        m.t = m.t.clamp(0.0, c.duration);
                    }
                }
                if let Some(dst) = project.marker_mut(row.id) {
                    *dst = m;
                }
                out.edited = true;
            }
        }
    });

    if let Some(id) = remove {
        undo(project);
        project.remove_marker(id);
        if state.selected == Some(id) {
            state.selected = None;
        }
        out.edited = true;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipKind};
    use eframe::egui::{Color32, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

    struct H {
        ctx: egui::Context,
        project: Project,
        state: MarkersState,
        selection: Vec<Id>,
        undos: usize,
        time: f64,
    }

    impl H {
        fn new() -> Self {
            let mut project = Project::new();
            project.tracks[0].clips.push(Clip::new(9, ClipKind::Video, "v", 2.0, 4.0));
            Self {
                ctx: egui::Context::default(),
                project,
                state: MarkersState::default(),
                selection: vec![9],
                undos: 0,
                time: 0.0,
            }
        }
        fn frame(&mut self, events: Vec<Event>) -> MarkersResponse {
            self.time += 0.05;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(520.0, 600.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, Color32::WHITE);
            let H { ctx, project, state, selection, undos, .. } = self;
            let mut out = MarkersResponse::default();
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    out = show(ui, state, project, selection, 3.0, &pal, &mut undo);
                });
            });
            out
        }
        fn rect(&self, name: &str) -> Rect {
            self.ctx
                .data(|d| d.get_temp::<Rect>(egui::Id::new(("mk", name.to_string()))))
                .unwrap_or_else(|| panic!("no widget rect recorded for {name}"))
        }
        fn click(&mut self, pos: Pos2) -> MarkersResponse {
            self.frame(vec![Event::PointerMoved(pos)]);
            self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }]);
            let out = self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }]);
            self.frame(vec![]);
            out
        }
    }

    #[test]
    fn add_seek_and_delete() {
        let mut h = H::new();
        h.frame(vec![]);
        let add = h.rect("add");
        h.click(add.center());
        assert_eq!(h.project.markers.len(), 1, "Add at playhead adds a marker");
        assert_eq!(h.project.markers[0].t, 3.0);
        assert_eq!(h.undos, 1, "one undo per gesture");

        let id = h.project.markers[0].id;
        h.frame(vec![]);
        let go = h.rect(&format!("go{id}"));
        let out = h.click(go.center());
        assert_eq!(out.seek, Some(3.0), "Go seeks to the marker time");

        h.frame(vec![]);
        let del = h.rect(&format!("del{id}"));
        h.click(del.center());
        assert!(h.project.markers.is_empty(), "the X removes the marker");
        assert_eq!(h.undos, 2, "delete is one more undo");
    }

    #[test]
    fn clip_markers_are_offset_and_filtered() {
        let mut p = Project::new();
        p.tracks[0].clips.push(Clip::new(9, ClipKind::Video, "v", 2.0, 4.0));
        p.add_marker(3.0, "project");
        p.add_clip_marker(9, 1.0, "on clip");
        let r = rows(&p, 0);
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.abs_t == 3.0), "both land at 3 s");
        let clip_row = r.iter().find(|x| x.clip.is_some()).expect("clip marker");
        assert_eq!(clip_row.offset, 2.0, "clip markers carry their clip start");
        p.markers[0].label = 2;
        assert_eq!(rows(&p, 2).len(), 1);
        assert_eq!(rows(&p, 1).len(), 0);
        assert_eq!(rows(&p, 0).len(), 2);
    }

    #[test]
    fn markdown_list_has_a_line_per_marker() {
        let mut p = Project::new();
        p.add_marker(1.0, "a");
        p.add_marker(2.5, "");
        let md = as_markdown(&p, 30.0);
        assert_eq!(md.lines().count(), 2);
        assert!(md.contains(" a"), "{md}");
        assert!(md.contains("(marker)"), "{md}");
    }

    #[test]
    fn empty_pane_is_harmless() {
        let mut h = H::new();
        let out = h.frame(vec![]);
        assert!(!out.edited && out.seek.is_none());
        assert_eq!(h.undos, 0);
    }
}
