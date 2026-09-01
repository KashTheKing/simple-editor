//! Markers pane: every marker in timeline order (project markers and clip markers together, via
//! `rows`), scoped to the sequence currently open (`Project.editing`) — a project-level marker only
//! shows on the sequence it was made on; clip markers scope naturally through their clip and are
//! unaffected. One `egui::CollapsingHeader` per marker (collapsed by default): the header shows the
//! icon glyph, a dim timecode, the name, "Go" (seek) and delete; expanding it reveals the editable
//! name/time/duration/note and a small icon picker.
//!
//! Multi-select (`state.selected: Vec<Id>`): click a name to select (replacing), Ctrl/Shift+click to
//! toggle it in/out — same pattern as clip and subtitle-cue selection elsewhere. A mini time strip
//! above the list plots every marker as a tick; dragging over empty strip space rubber-bands a time
//! range (Shift adds to the selection). With 2+ selected, bulk buttons appear: Delete, "Snap to
//! Nearest Clip" (project markers only, moves `t` to the nearest clip's start) and "Link to Closest
//! Clip" (converts a project marker into a clip-local one on the nearest clip). Each bulk op is one
//! undo for the whole batch. Toolbar: "Add at playhead" (M), "Add on selected clip", filter by label,
//! "Copy as list" (markdown, handy for the AI tools, unscoped — every marker in the project).

use crate::model::{Id, Project};
use crate::theme::Palette;
use crate::ui::tools::Glyph;
use crate::ui::{edit_start, timecode};
use eframe::egui::{self, DragValue, Response, RichText};

/// Small, non-exhaustive set of glyphs relevant to a marker (the full picker lives in Settings ▸
/// Appearance ▸ Icons if someone wants an exotic one — this is just the quick picks).
const ICON_CHOICES: &[Glyph] = &[
    Glyph::Flag,
    Glyph::Bookmark,
    Glyph::Star,
    Glyph::Diamond,
    Glyph::Dot,
    Glyph::Camera,
    Glyph::MusicNote,
    Glyph::Target,
];

fn marker_glyph(name: &str) -> Glyph {
    Glyph::from_name(name).unwrap_or(Glyph::Flag)
}

#[derive(Default)]
pub struct MarkersState {
    pub selected: Vec<Id>,
    /// 0 = every label.
    pub filter_label: u8,
    /// Drag-select band in progress on the mini time strip: (press time in seconds, "Shift held").
    band: Option<(f64, bool)>,
}

#[derive(Default)]
pub struct MarkersResponse {
    pub edited: bool,
    pub seek: Option<f64>,
}

/// The delete button used everywhere (planner, inspector, effects, mixer, …): a painted cross that
/// turns red on hover.
pub(crate) fn x_button(ui: &mut egui::Ui) -> Response {
    let r = crate::ui::tools::glyph_text_button(ui, Glyph::Cross, "");
    if r.hovered() {
        let c = egui::Color32::from_rgb(220, 70, 70);
        ui.painter().rect_stroke(r.rect, 2.0, egui::Stroke::new(1.0, c), egui::StrokeKind::Inside);
        crate::ui::tools::draw_glyph(ui.painter(), r.rect, Glyph::Cross, c);
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

/// Every marker in timeline order, project markers filtered to the sequence currently being edited
/// (clip markers are unaffected — they already scope through their clip).
fn rows(project: &Project, filter: u8) -> Vec<Row> {
    let mut v: Vec<Row> = project
        .markers
        .iter()
        .filter(|m| (filter == 0 || m.label == filter) && m.sequence == project.editing)
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

/// Markdown list of every marker in timeline order (for notes / the AI tools) — every sequence, not
/// just the one currently open; this is a project-wide export, not the pane's scoped display.
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
    let (mods, pointer, primary_down) =
        ui.input(|i| (i.modifiers, i.pointer.latest_pos(), i.pointer.primary_down()));

    let rows = rows(project, state.filter_label);
    state.selected.retain(|id| rows.iter().any(|r| r.id == *id));

    // ---- toolbar ----
    let sel_clip = selection.iter().copied().find(|&id| project.clip(id).is_some());
    ui.horizontal_wrapped(|ui| {
        let r = ui.button("Add at playhead").on_hover_text("M");
        mark(ui, "add", &r);
        if r.clicked() {
            undo(project);
            state.selected = vec![project.add_marker(playhead, "")];
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
                if let Some(id) = project.add_clip_marker(cid, local, "") {
                    state.selected = vec![id];
                }
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
                    // the name is tinted with its own colour — no swatch character needed
                    let t = RichText::new(name.clone()).color(egui::Color32::from_rgb(*r, *g, *b));
                    ui.selectable_value(&mut state.filter_label, i as u8 + 1, t);
                }
            });
        if ui.button("Copy as list").on_hover_text("Markdown, for notes and the AI tools").clicked() {
            ui.ctx().copy_text(as_markdown(project, fps));
        }
    });

    // ---- bulk ops on the multi-selection ----
    ui.horizontal_wrapped(|ui| {
        let n = state.selected.len();
        ui.label(if n > 0 { format!("{n} selected") } else { String::new() });
        let enabled = n >= 1; // bulk ops are also the ONLY route to snap/link a single marker
        let del = ui.add_enabled(enabled, egui::Button::new("Delete"));
        mark(ui, "bulk_delete", &del);
        if del.clicked() {
            undo(project);
            for id in std::mem::take(&mut state.selected) {
                project.remove_marker(id);
            }
            out.edited = true;
        }
        let snap = ui
            .add_enabled(enabled, egui::Button::new("Snap to Nearest Clip"))
            .on_hover_text("Move each selected timeline marker to its nearest clip edge (start or end)");
        mark(ui, "bulk_snap", &snap);
        if snap.clicked() {
            undo(project);
            for id in state.selected.clone() {
                project.snap_marker_to_nearest_clip(id);
            }
            out.edited = true;
        }
        let link = ui
            .add_enabled(enabled, egui::Button::new("Link to Closest Clip"))
            .on_hover_text("Turn each selected timeline marker into a marker on its nearest clip");
        mark(ui, "bulk_link", &link);
        if link.clicked() {
            undo(project);
            for id in state.selected.clone() {
                project.link_marker_to_closest_clip(id);
            }
            out.edited = true;
        }
    });
    ui.separator();

    if rows.is_empty() {
        ui.weak("No markers — press M to add one at the playhead");
        return out;
    }

    // ---- mini time strip: a tick per marker, drag to rubber-band a time range ----
    let strip_h = 20.0;
    let span = rows.iter().map(|r| r.abs_t).fold(project.duration().max(1.0), f64::max);
    let (strip_rect, strip_resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), strip_h), egui::Sense::click_and_drag());
    let x_at = |t: f64| strip_rect.left() + ((t / span).clamp(0.0, 1.0) as f32) * strip_rect.width();
    let t_at = |x: f32| ((x - strip_rect.left()) / strip_rect.width()).clamp(0.0, 1.0) as f64 * span;
    let painter = ui.painter().clone();
    painter.rect_filled(strip_rect, 2.0, palette.header);

    if strip_resp.drag_started_by(egui::PointerButton::Primary) {
        if let Some(o) = strip_resp.interact_pointer_pos() {
            state.band = Some((t_at(o.x), mods.shift));
        }
    }
    if strip_resp.clicked() {
        state.selected.clear();
    }
    let band_range = state.band.and_then(|(t0, _)| {
        let t1 = t_at(pointer?.x);
        Some((t0.min(t1), t0.max(t1)))
    });
    if let Some((a, b)) = band_range {
        painter.rect_filled(
            egui::Rect::from_x_y_ranges(x_at(a)..=x_at(b), strip_rect.y_range()),
            0.0,
            palette.selection.gamma_multiply(0.25),
        );
    }
    if state.band.is_some() && !primary_down {
        let (_, add) = state.band.take().expect("checked above");
        if let Some((a, b)) = band_range {
            let hit: Vec<Id> = rows.iter().filter(|r| r.abs_t >= a && r.abs_t <= b).map(|r| r.id).collect();
            if !add {
                state.selected.clear();
            }
            for h in hit {
                if !state.selected.contains(&h) {
                    state.selected.push(h);
                }
            }
        }
    }
    for row in &rows {
        let x = x_at(row.abs_t);
        let sel = state.selected.contains(&row.id);
        let c = if sel { palette.accent } else { palette.text_dim };
        painter.circle_filled(egui::pos2(x, strip_rect.center().y), if sel { 3.5 } else { 2.5 }, c);
    }
    ui.add_space(2.0);

    let mut remove: Option<Id> = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for row in &rows {
            // ponytail: edit a clone of the one marker and write it back — lets `undo` snapshot the
            // untouched project without holding a mutable borrow across the widgets.
            let Some(mut m) = project.marker_mut(row.id).map(|m| m.clone()) else { continue };
            let selected = state.selected.contains(&row.id);
            let (mut start, mut changed) = (false, false);
            let mut ctrl_toggle = false;
            let mut select_hit = false;

            let header_id = ui.id().with(("marker_row", row.id));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), header_id, false)
                .show_header(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let (icon_rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                        crate::ui::tools::draw_glyph(ui.painter(), icon_rect, marker_glyph(&m.icon), palette.text);

                        let color = match project.label_color(m.label) {
                            Some([r, g, b]) => egui::Color32::from_rgb(r, g, b),
                            None => palette.text_dim,
                        };
                        let dot = ui.add(crate::ui::tools::color_chip(color, false, palette));
                        dot.context_menu(|ui| {
                            if ui.selectable_label(m.label == 0, "None").clicked() {
                                m.label = 0;
                                start = true;
                                changed = true;
                                ui.close();
                            }
                            for (i, (name, [r, g, b])) in labels.iter().enumerate() {
                                let t = RichText::new(name.clone()).color(egui::Color32::from_rgb(*r, *g, *b));
                                if ui.selectable_label(m.label == i as u8 + 1, t).clicked() {
                                    m.label = i as u8 + 1;
                                    start = true;
                                    changed = true;
                                    ui.close();
                                }
                            }
                        });

                        ui.weak(timecode(row.abs_t, fps));
                        let label_text = if m.name.is_empty() { "(marker)".to_string() } else { m.name.clone() };
                        let name_r = ui.selectable_label(selected, label_text);
                        mark(ui, format_args!("name{}", row.id), &name_r);
                        if name_r.clicked() {
                            select_hit = true;
                            ctrl_toggle = mods.ctrl || mods.shift;
                            // "when I click on a marker, it should reposition my playhead" — a plain
                            // click seeks too (Ctrl/Shift keep pure multi-select, Go stays for hover-time)
                            if !ctrl_toggle {
                                out.seek = Some(m.t + row.offset);
                            }
                        }

                        let go = ui.small_button("Go").on_hover_text(timecode(row.abs_t, fps));
                        mark(ui, format_args!("go{}", row.id), &go);
                        if go.clicked() {
                            state.selected = vec![row.id];
                            out.seek = Some(m.t + row.offset);
                        }
                        let del = x_button(ui).on_hover_text("Delete marker");
                        mark(ui, format_args!("del{}", row.id), &del);
                        if del.clicked() {
                            remove = Some(row.id);
                        }
                    });
                })
                .body(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut m.name).hint_text("marker"));
                    start |= r.gained_focus();
                    changed |= r.changed();
                    ui.horizontal(|ui| {
                        let mut t = m.t;
                        let r =
                            ui.add(DragValue::new(&mut t).speed(0.05).range(0.0..=1e6).suffix(" s").fixed_decimals(2));
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
                    });
                    let r = ui.add(
                        egui::TextEdit::multiline(&mut m.note)
                            .desired_rows(2)
                            .desired_width(f32::INFINITY)
                            .hint_text("note"),
                    );
                    start |= r.gained_focus();
                    changed |= r.changed();
                    ui.horizontal_wrapped(|ui| {
                        for g in ICON_CHOICES {
                            let picked = m.icon == g.name();
                            let btn = crate::ui::tools::glyph_text_button(ui, *g, "");
                            if picked {
                                ui.painter().rect_stroke(
                                    btn.rect,
                                    2.0,
                                    egui::Stroke::new(1.5, palette.accent),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if btn.on_hover_text(g.name()).clicked() {
                                m.icon = g.name().to_string();
                                start = true;
                                changed = true;
                            }
                        }
                    });
                });

            if select_hit {
                if ctrl_toggle {
                    if selected {
                        state.selected.retain(|id| *id != row.id);
                    } else {
                        state.selected.push(row.id);
                    }
                } else {
                    state.selected = vec![row.id];
                }
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
        state.selected.retain(|s| *s != id);
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
            self.frame_mod(events, Modifiers::NONE)
        }
        /// `show()` reads the CURRENT modifier state via `ui.input(|i| i.modifiers)` (the RawInput-level
        /// field), not each event's own `modifiers` — a click event's embedded modifiers alone (as
        /// `frame` alone would send) is not enough to make Ctrl/Shift-click register.
        fn frame_mod(&mut self, events: Vec<Event>, modifiers: Modifiers) -> MarkersResponse {
            self.time += 0.05;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(520.0, 600.0))),
                time: Some(self.time),
                modifiers,
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
        fn click_mod(&mut self, pos: Pos2, modifiers: Modifiers) -> MarkersResponse {
            self.frame_mod(vec![Event::PointerMoved(pos)], modifiers);
            self.frame_mod(
                vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers }],
                modifiers,
            );
            let out = self.frame_mod(
                vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers }],
                modifiers,
            );
            self.frame(vec![]);
            out
        }
        fn click(&mut self, pos: Pos2) -> MarkersResponse {
            self.click_mod(pos, Modifiers::NONE)
        }
        fn ctrl_click(&mut self, pos: Pos2) -> MarkersResponse {
            self.click_mod(pos, Modifiers::CTRL)
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
        assert_eq!(h.state.selected, vec![id]);

        h.frame(vec![]);
        let del = h.rect(&format!("del{id}"));
        h.click(del.center());
        assert!(h.project.markers.is_empty(), "the X removes the marker");
        assert_eq!(h.undos, 2, "delete is one more undo");
        assert!(h.state.selected.is_empty());
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
    fn project_markers_scope_to_the_current_sequence() {
        let mut p = Project::new();
        p.add_marker(1.0, "main"); // editing == None
        p.editing = Some(42);
        p.add_marker(2.0, "other seq");
        assert_eq!(rows(&p, 0).len(), 1, "only the marker made on the current sequence shows");
        p.editing = None;
        assert_eq!(rows(&p, 0).len(), 1, "back on main, the other sequence's marker is hidden");
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

    #[test]
    fn multi_select_ctrl_toggle() {
        let mut h = H::new();
        let a = h.project.add_marker(1.0, "a");
        let b = h.project.add_marker(2.0, "b");

        h.frame(vec![]);
        let ra = h.rect(&format!("name{a}"));
        h.click(ra.center());
        assert_eq!(h.state.selected, vec![a], "plain click selects, replacing");

        h.frame(vec![]);
        let rb = h.rect(&format!("name{b}"));
        h.ctrl_click(rb.center());
        assert_eq!(h.state.selected, vec![a, b], "ctrl+click adds to the selection");

        h.frame(vec![]);
        let ra2 = h.rect(&format!("name{a}"));
        h.ctrl_click(ra2.center());
        assert_eq!(h.state.selected, vec![b], "ctrl+click again toggles it back off");
    }

    #[test]
    fn bulk_delete_removes_all_selected_with_one_undo() {
        let mut h = H::new();
        let a = h.project.add_marker(1.0, "a");
        let b = h.project.add_marker(2.0, "b");
        h.state.selected = vec![a, b];
        h.frame(vec![]);
        let btn = h.rect("bulk_delete");
        h.click(btn.center());
        assert!(h.project.markers.is_empty(), "both selected markers are removed");
        assert_eq!(h.undos, 1, "one undo for the whole batch");
        assert!(h.state.selected.is_empty());
    }

    #[test]
    fn bulk_snap_moves_every_selected_marker_with_one_undo() {
        let mut h = H::new();
        h.project.tracks[0].clips.push(Clip::new(10, ClipKind::Video, "c2", 20.0, 2.0));
        let a = h.project.add_marker(1.0, "a"); // nearer clip 9 (start 2.0)
        let b = h.project.add_marker(19.0, "b"); // nearer clip 10 (start 20.0)
        h.state.selected = vec![a, b];
        h.frame(vec![]);
        let btn = h.rect("bulk_snap");
        h.click(btn.center());
        assert_eq!(h.project.marker_mut(a).unwrap().t, 2.0);
        assert_eq!(h.project.marker_mut(b).unwrap().t, 20.0);
        assert_eq!(h.undos, 1, "one undo for the whole batch");
    }
}
