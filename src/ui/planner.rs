//! Planner pane: tabs "Plan", "Notes" and "Timer".
//! Plan = a nested to-do tree (Project.plan): each row = checkbox (done), editable title, colour label dot
//! (click → LABEL_COLORS menu), a collapse triangle, a done/total progress fraction when it has children,
//! "+" add sub-task, delete, reorder among siblings, indent/outdent; a non-empty `requirements` checklist
//! renders compactly underneath. A selected item's details panel shows its notes (multiline), its
//! requirements (checkbox + label + delete, "Add requirement") and its moodboard: the asset ids in
//! `assets` as rows (name, kind tag, thumbnail via ThumbCache if available, per-asset note in `asset_notes`,
//! "Add to timeline", delete); assets are added by dropping library items (DragPayload::Asset) onto the item or
//! onto the moodboard area. Progress bar "done/total" at the top; "Add task" at the top level;
//! "Clear completed". Undo once per gesture.
//!
//! Notes tab = `Project.notes`, a list of titled markdown entries ("describe your process, ideas, style
//! while you edit — the style summary / AI tools read this"): one collapsed-by-default
//! `CollapsingHeader` per note (colour label dot, title field, Edit/Preview toggle, delete), body shown
//! as rendered markdown (`egui_commonmark`) or, in edit mode, a plain multiline `TextEdit`.
//!
//! Timer tab = a session-scoped stopwatch/countdown (`PlannerState::timer`, not project data): a round
//! dial (`dial()`), Start/Pause/Reset, and an optional link to one plan item — while running, elapsed
//! seconds accumulate into that item's `PlanItem::tracked_seconds` (`timer_tick`, the pure, testable
//! half of the tab).
//!
//! Returns what changed.

use crate::media::thumbs::ThumbCache;
use crate::model::{ClipKind, Id, PlanItem, Project, LABEL_COLORS};
use crate::theme::Palette;
use crate::ui::markers_ui::x_button;
use crate::ui::tools::{glyph_text_button, Dir, Glyph};
use crate::ui::{edit_start, label_color, once, DragPayload};
use eframe::egui::{self, Response, RichText, TextEdit};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct PlannerState {
    pub tab: usize,
    pub selected: Option<Id>,
    pub collapsed: Vec<Id>,
    /// Notes tab: which notes (by id) are in edit mode (plain `TextEdit`) rather than markdown preview.
    /// Open/closed state of each note's `CollapsingHeader` is persisted by egui itself, not here.
    pub notes_editing: Vec<Id>,
    pub md_cache: CommonMarkCache,
    pub timer: TimerState,
}

#[derive(Default)]
pub struct PlannerResponse {
    pub edited: bool,
    /// Asset ids the user asked to place on the timeline at the playhead.
    pub add_to_timeline: Vec<Id>,
}

/// Stopwatch / countdown mode for the Timer tab.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum TimerMode {
    #[default]
    Stopwatch,
    Countdown,
}

/// Session-scoped Timer tab state — not project data (a running timer is UI state, like the playhead),
/// which is why it lives here rather than on `PlanItem`.
pub struct TimerState {
    pub mode: TimerMode,
    pub running: bool,
    /// Stopwatch: seconds elapsed. Countdown: seconds remaining.
    pub seconds: f64,
    /// Countdown duration, and what Reset restores `seconds` to in that mode.
    pub minutes: u32,
    pub secs: u32,
    pub last_tick: Option<Instant>,
    /// Plan item this timer banks elapsed seconds into (`PlanItem::tracked_seconds`) while running.
    pub linked: Option<Id>,
    /// A countdown reached zero and hasn't been acknowledged yet (Start/Reset clears it) — drives the
    /// dial's flash.
    pub done_flash: bool,
}

impl Default for TimerState {
    fn default() -> Self {
        Self {
            mode: TimerMode::default(),
            running: false,
            seconds: 0.0,
            minutes: 5,
            secs: 0,
            last_tick: None,
            linked: None,
            done_flash: false,
        }
    }
}

// ---------- pure tree helpers ----------

fn find_mut(items: &mut [PlanItem], id: Id) -> Option<&mut PlanItem> {
    for it in items {
        if it.id == id {
            return Some(it);
        }
        if let Some(f) = find_mut(&mut it.children, id) {
            return Some(f);
        }
    }
    None
}

/// Swap the item with its previous/next sibling. Returns true when the id was found (even at an edge).
fn move_item(items: &mut Vec<PlanItem>, id: Id, delta: i32) -> bool {
    if let Some(i) = items.iter().position(|x| x.id == id) {
        let j = i as i32 + delta;
        if j >= 0 && (j as usize) < items.len() {
            items.swap(i, j as usize);
        }
        return true;
    }
    items.iter_mut().any(|it| move_item(&mut it.children, id, delta))
}

/// Make the item a child of its previous sibling.
fn indent_item(items: &mut Vec<PlanItem>, id: Id) -> bool {
    if let Some(i) = items.iter().position(|x| x.id == id) {
        if i > 0 {
            let it = items.remove(i);
            items[i - 1].children.push(it);
        }
        return true;
    }
    items.iter_mut().any(|it| indent_item(&mut it.children, id))
}

/// Move the item next to its parent (top-level items stay).
fn outdent_item(items: &mut Vec<PlanItem>, id: Id) -> bool {
    for i in 0..items.len() {
        if let Some(j) = items[i].children.iter().position(|c| c.id == id) {
            let it = items[i].children.remove(j);
            items.insert(i + 1, it);
            return true;
        }
        if outdent_item(&mut items[i].children, id) {
            return true;
        }
    }
    false
}

/// (done, total) across the whole tree.
fn count_items(items: &[PlanItem]) -> (usize, usize) {
    let mut done = 0;
    let mut total = 0;
    for it in items {
        total += 1;
        done += it.done as usize;
        let (d, t) = count_items(&it.children);
        done += d;
        total += t;
    }
    (done, total)
}

/// Remove done items (their children go with them).
fn retain_incomplete(items: &mut Vec<PlanItem>) {
    items.retain(|i| !i.done);
    for i in items {
        retain_incomplete(&mut i.children);
    }
}

/// Every plan item as (id, indented title) in tree order, for the Timer tab's "Link to task" combo.
fn flatten_plan(items: &[PlanItem]) -> Vec<(Id, String)> {
    fn walk(items: &[PlanItem], depth: usize, out: &mut Vec<(Id, String)>) {
        for i in items {
            let title = if i.title.is_empty() { "(untitled)" } else { i.title.as_str() };
            out.push((i.id, format!("{}{}", "  ".repeat(depth), title)));
            walk(&i.children, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    walk(items, 0, &mut out);
    out
}

/// mm:ss (rounded to the second — a dial has no room for tenths).
fn clock_text(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// Advance the timer by wall-clock time since the last call. Called by the App EVERY frame — not from
/// the Timer tab, which may be a hidden sibling tab — so a countdown still finishes, banks time onto
/// the linked task, and can notify while any other pane is in front. Returns
/// `(banked_time_on_a_task, countdown_just_finished)`.
pub fn tick(state: &mut PlannerState, project: &mut Project) -> (bool, bool) {
    let t = &mut state.timer;
    if !t.running {
        t.last_tick = None;
        return (false, false);
    }
    let now = Instant::now();
    let dt = t.last_tick.map(|p| now.duration_since(p).as_secs_f64()).unwrap_or(0.0);
    t.last_tick = Some(now);
    let banked = timer_tick(t, project, dt);
    (banked, !t.running) // timer_tick clears `running` exactly when a countdown reaches zero
}

/// The pure half of the tick: advance `timer` by `dt` seconds (computed by `tick` from `Instant`s so
/// this stays testable without mocking time) and, if linked, bank it onto that plan item. Returns
/// whether it touched the project (so the caller can mark it dirty).
fn timer_tick(timer: &mut TimerState, project: &mut Project, dt: f64) -> bool {
    if dt <= 0.0 {
        return false;
    }
    match timer.mode {
        TimerMode::Stopwatch => timer.seconds += dt,
        TimerMode::Countdown => {
            timer.seconds = (timer.seconds - dt).max(0.0);
            if timer.seconds <= 0.0 {
                timer.running = false;
                timer.done_flash = true;
            }
        }
    }
    match timer.linked.and_then(|id| project.plan_item_mut(id)) {
        Some(it) => {
            it.tracked_seconds += dt;
            true
        }
        None => false,
    }
}

// ---------- UI ----------

enum Op {
    Add(Option<Id>),
    Remove(Id),
    Up(Id),
    Down(Id),
    In(Id),
    Out(Id),
    ClearDone,
}

struct TreeUi<'a> {
    collapsed: &'a mut Vec<Id>,
    selected: &'a mut Option<Id>,
    op: &'a mut Option<Op>,
    start: bool,
    changed: bool,
    palette: &'a Palette,
}

impl TreeUi<'_> {
    fn note(&mut self, r: &Response) {
        self.start |= edit_start(r);
        self.changed |= r.changed();
    }
    /// Text fields: one undo entry per visit to the field, not per keystroke.
    fn note_text(&mut self, r: &Response) {
        self.start |= r.gained_focus();
        self.changed |= r.changed();
    }
    fn click(&mut self) {
        self.start = true;
        self.changed = true;
    }
}

/// Colour-dot button opening a LABEL_COLORS menu; returns the picked value.
fn color_menu(ui: &mut egui::Ui, current: u8, palette: &Palette) -> Option<u8> {
    let mut picked = None;
    let chip = crate::ui::tools::color_chip(label_color(current, palette), false, palette);
    egui::containers::menu::MenuButton::from_button(chip).ui(ui, |ui| {
        if ui.button("None").clicked() {
            picked = Some(0);
            ui.close();
        }
        for (i, (name, [r, g, b])) in LABEL_COLORS.iter().enumerate() {
            // the name carries its own colour — no swatch character needed
            let t = RichText::new(*name).color(egui::Color32::from_rgb(*r, *g, *b));
            if ui.button(t).clicked() {
                picked = Some(i as u8 + 1);
                ui.close();
            }
        }
    });
    picked
}

fn tree_rows(ui: &mut egui::Ui, items: &mut [PlanItem], depth: usize, t: &mut TreeUi) {
    for it in items {
        let closed = t.collapsed.contains(&it.id);
        let row = ui
            .horizontal(|ui| {
                ui.add_space(depth as f32 * 14.0);
                if it.children.is_empty() {
                    ui.add_space(18.0);
                } else if glyph_text_button(ui, Glyph::Tri(if closed { Dir::Right } else { Dir::Down }), "").clicked() {
                    if closed {
                        t.collapsed.retain(|&c| c != it.id);
                    } else {
                        t.collapsed.push(it.id);
                    }
                }
                let r = ui.checkbox(&mut it.done, "");
                t.note(&r);
                if let Some(c) = color_menu(ui, it.color, t.palette) {
                    it.color = c;
                    t.click();
                }
                let w = (ui.available_width() - 150.0).max(60.0);
                let r = ui.add(TextEdit::singleline(&mut it.title).desired_width(w).hint_text("task"));
                if r.has_focus() {
                    *t.selected = Some(it.id);
                }
                t.note_text(&r);
                if !it.children.is_empty() {
                    let (d, total) = count_items(&it.children);
                    ui.weak(format!("{d}/{total}"));
                }
                let mut op = |o: Op| *t.op = Some(o);
                if ui.small_button("+").on_hover_text("Add sub-task").clicked() {
                    op(Op::Add(Some(it.id)));
                }
                if glyph_text_button(ui, Glyph::Tri(Dir::Up), "").on_hover_text("Move up").clicked() {
                    op(Op::Up(it.id));
                }
                if glyph_text_button(ui, Glyph::Tri(Dir::Down), "").on_hover_text("Move down").clicked() {
                    op(Op::Down(it.id));
                }
                if glyph_text_button(ui, Glyph::Indent(false), "").on_hover_text("Outdent").clicked() {
                    op(Op::Out(it.id));
                }
                if glyph_text_button(ui, Glyph::Indent(true), "").on_hover_text("Indent").clicked() {
                    op(Op::In(it.id));
                }
                if x_button(ui).on_hover_text("Delete task").clicked() {
                    op(Op::Remove(it.id));
                }
            })
            .response;
        // drop a library asset onto the row → moodboard
        if let Some(p) = row.dnd_release_payload::<DragPayload>() {
            if let DragPayload::Asset(aid) = *p {
                if !it.assets.contains(&aid) {
                    it.assets.push(aid);
                    t.click();
                }
                *t.selected = Some(it.id);
            }
        }
        // a non-empty checklist renders compactly right under its task — no header, just small rows
        if !it.requirements.is_empty() {
            let mut rm: Option<usize> = None;
            for (ri, (label, done)) in it.requirements.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add_space(depth as f32 * 14.0 + 36.0);
                    let r = ui.checkbox(done, "");
                    t.note(&r);
                    ui.label(RichText::new(label.as_str()).small().weak());
                    if x_button(ui).on_hover_text("Remove requirement").clicked() {
                        rm = Some(ri);
                    }
                });
            }
            if let Some(i) = rm {
                it.requirements.remove(i);
                t.click();
            }
        }
        if !closed {
            tree_rows(ui, &mut it.children, depth + 1, t);
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut PlannerState,
    project: &mut Project,
    thumbs: &mut ThumbCache,
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> PlannerResponse {
    let mut resp = PlannerResponse::default();
    let mut undone = false;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.tab, 0, "Plan");
        ui.selectable_value(&mut state.tab, 1, "Notes");
        ui.selectable_value(&mut state.tab, 2, "Timer");
    });
    ui.separator();

    if state.tab == 1 {
        notes_tab(ui, state, project, palette, undo, &mut resp);
        return resp;
    }
    if state.tab == 2 {
        timer_tab(ui, state, project, palette, &mut resp);
        return resp;
    }

    let (done, total) = count_items(&project.plan);
    ui.horizontal(|ui| {
        if total > 0 {
            ui.add(
                egui::ProgressBar::new(done as f32 / total as f32).desired_width(120.0).text(format!("{done}/{total}")),
            );
        }
        ui.label("");
    });

    // ponytail: per-frame deep clone of the task tree, written back on change — keeps `undo` able to
    // snapshot the untouched project. Upgrade: drive tree_rows from &mut project.plan if a big plan shows.
    let mut plan = project.plan.clone();
    let mut op: Option<Op> = None;
    let mut t = TreeUi {
        collapsed: &mut state.collapsed,
        selected: &mut state.selected,
        op: &mut op,
        start: false,
        changed: false,
        palette,
    };
    ui.horizontal(|ui| {
        if ui.button("Add task").clicked() {
            *t.op = Some(Op::Add(None));
        }
        if done > 0 && ui.button("Clear completed").clicked() {
            *t.op = Some(Op::ClearDone);
        }
    });
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        tree_rows(ui, &mut plan, 0, &mut t);
        if let Some(sel) = *t.selected {
            let (start, changed) = details(ui, &mut plan, sel, project, thumbs, &mut resp);
            t.start |= start;
            t.changed |= changed;
        }
    });

    if t.start {
        once(&mut undone, undo, project);
    }
    if t.changed {
        project.plan = plan;
        resp.edited = true;
    }
    if let Some(op) = op {
        once(&mut undone, undo, project);
        match op {
            Op::Add(parent) => state.selected = Some(project.plan_add(parent, "Task")),
            Op::Remove(id) => {
                project.plan_remove(id);
                if state.selected == Some(id) {
                    state.selected = None;
                }
            }
            Op::Up(id) => {
                move_item(&mut project.plan, id, -1);
            }
            Op::Down(id) => {
                move_item(&mut project.plan, id, 1);
            }
            Op::In(id) => {
                indent_item(&mut project.plan, id);
            }
            Op::Out(id) => {
                outdent_item(&mut project.plan, id);
            }
            Op::ClearDone => retain_incomplete(&mut project.plan),
        }
        resp.edited = true;
    }
    resp
}

/// Notes + moodboard of the selected item (edits the plan clone). Returns (gesture start, changed).
fn details(
    ui: &mut egui::Ui,
    plan: &mut [PlanItem],
    sel: Id,
    project: &Project,
    thumbs: &mut ThumbCache,
    resp: &mut PlannerResponse,
) -> (bool, bool) {
    let Some(it) = find_mut(plan, sel) else { return (false, false) };
    let mut start = false;
    let mut changed = false;
    ui.separator();
    let r = ui.add(TextEdit::multiline(&mut it.notes).desired_rows(2).desired_width(f32::INFINITY).hint_text("notes"));
    start |= r.gained_focus();
    changed |= r.changed();

    ui.weak("Requirements");
    let mut rm_req: Option<usize> = None;
    for (ri, (label, done)) in it.requirements.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let r = ui.checkbox(done, "");
            start |= edit_start(&r);
            changed |= r.changed();
            let r = ui.add(TextEdit::singleline(label).desired_width(f32::INFINITY));
            start |= r.gained_focus();
            changed |= r.changed();
            if x_button(ui).on_hover_text("Remove requirement").clicked() {
                rm_req = Some(ri);
            }
        });
    }
    if let Some(i) = rm_req {
        it.requirements.remove(i);
        start = true;
        changed = true;
    }
    if ui.small_button("Add requirement").clicked() {
        it.requirements.push(("Requirement".into(), false));
        start = true;
        changed = true;
    }

    while it.asset_notes.len() < it.assets.len() {
        it.asset_notes.push(String::new());
    }
    let mut rm: Option<usize> = None;
    let (frame_r, payload) = ui.dnd_drop_zone::<DragPayload, ()>(egui::Frame::group(ui.style()), |ui| {
        ui.weak("Moodboard — drop library assets here");
        for i in 0..it.assets.len() {
            let aid = it.assets[i];
            ui.horizontal(|ui| {
                let Some(a) = project.asset(aid) else {
                    ui.weak("(missing asset)");
                    return;
                };
                if a.has_video() {
                    if let Some((tex, [w, h])) = thumbs.texture(ui.ctx(), &a.path, 0.0, 40) {
                        let size = egui::vec2(w as f32, h as f32);
                        ui.add(egui::Image::new(egui::load::SizedTexture::new(tex, size)));
                    }
                }
                ui.label(a.name());
                ui.weak(match a.kind {
                    ClipKind::Video => "V",
                    ClipKind::Audio => "A",
                    ClipKind::Image => "I",
                    _ => "?",
                });
                if ui.small_button("Add to timeline").clicked() {
                    resp.add_to_timeline.push(aid);
                }
                if x_button(ui).on_hover_text("Remove from moodboard").clicked() {
                    rm = Some(i);
                }
            });
            let r = ui.add(TextEdit::singleline(&mut it.asset_notes[i]).desired_width(f32::INFINITY).hint_text("note"));
            start |= r.gained_focus();
            changed |= r.changed();
        }
    });
    let _ = frame_r;
    if let Some(p) = payload {
        if let DragPayload::Asset(aid) = *p {
            if !it.assets.contains(&aid) {
                it.assets.push(aid);
                it.asset_notes.push(String::new());
                start = true;
                changed = true;
            }
        }
    }
    if let Some(i) = rm {
        it.assets.remove(i);
        if i < it.asset_notes.len() {
            it.asset_notes.remove(i);
        }
        start = true;
        changed = true;
    }
    (start, changed)
}

/// Notes tab: `Project.notes` as a list of collapsed-by-default markdown entries.
fn notes_tab(
    ui: &mut egui::Ui,
    state: &mut PlannerState,
    project: &mut Project,
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
    resp: &mut PlannerResponse,
) {
    let mut undone = false;
    let labels: Vec<(String, [u8; 3])> = project.labels.iter().map(|l| (l.name.clone(), l.color)).collect();
    ui.horizontal(|ui| {
        if ui.button("Add note").clicked() {
            once(&mut undone, undo, project);
            let id = project.add_note("");
            state.notes_editing.push(id);
            resp.edited = true;
        }
    });
    ui.separator();
    if project.notes.is_empty() {
        ui.weak("No notes yet — describe your process, ideas or style. The style summary / AI tools read these.");
    }
    let mut remove: Option<Id> = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for i in 0..project.notes.len() {
            let id = project.notes[i].id;
            // ponytail: edit a clone and write it back — lets `undo` snapshot the untouched project
            // without holding a mutable borrow across the widgets (same trick markers_ui.rs uses).
            let mut n = project.notes[i].clone();
            let editing = state.notes_editing.contains(&id);
            let (mut start, mut changed) = (false, false);
            let mut toggle_edit = false;
            let mut del = false;

            let header_id = ui.id().with(("planner_note", id));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), header_id, false)
                .show_header(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let color = (n.label > 0)
                            .then(|| labels.get(n.label as usize - 1))
                            .flatten()
                            .map(|(_, [r, g, b])| egui::Color32::from_rgb(*r, *g, *b))
                            .unwrap_or(palette.text_dim);
                        let dot = ui.add(crate::ui::tools::color_chip(color, false, palette));
                        dot.context_menu(|ui| {
                            if ui.selectable_label(n.label == 0, "None").clicked() {
                                n.label = 0;
                                start = true;
                                changed = true;
                                ui.close();
                            }
                            for (li, (name, [r, g, b])) in labels.iter().enumerate() {
                                let t = RichText::new(name.clone()).color(egui::Color32::from_rgb(*r, *g, *b));
                                if ui.selectable_label(n.label == li as u8 + 1, t).clicked() {
                                    n.label = li as u8 + 1;
                                    start = true;
                                    changed = true;
                                    ui.close();
                                }
                            }
                        });
                        let w = (ui.available_width() - 90.0).max(60.0);
                        let r = ui.add(TextEdit::singleline(&mut n.title).desired_width(w).hint_text("(untitled)"));
                        start |= r.gained_focus();
                        changed |= r.changed();
                        let icon = if editing { Glyph::Eye } else { Glyph::Pencil };
                        if glyph_text_button(ui, icon, "")
                            .on_hover_text(if editing { "Preview" } else { "Edit" })
                            .clicked()
                        {
                            toggle_edit = true;
                        }
                        if x_button(ui).on_hover_text("Delete note").clicked() {
                            del = true;
                        }
                    });
                })
                .body(|ui| {
                    if editing {
                        let r = ui.add(
                            TextEdit::multiline(&mut n.body)
                                .desired_width(f32::INFINITY)
                                .desired_rows(6)
                                .hint_text("Markdown — **bold**, _italic_, lists, headings…"),
                        );
                        start |= r.gained_focus();
                        changed |= r.changed();
                    } else if n.body.trim().is_empty() {
                        ui.weak("(empty — click the pencil to edit)");
                    } else {
                        CommonMarkViewer::new().show(ui, &mut state.md_cache, &n.body);
                    }
                });

            if start {
                once(&mut undone, undo, project);
            }
            if changed {
                project.notes[i] = n;
                resp.edited = true;
            }
            if toggle_edit {
                if editing {
                    state.notes_editing.retain(|&x| x != id);
                } else {
                    state.notes_editing.push(id);
                }
            }
            if del {
                remove = Some(id);
            }
        }
    });
    if let Some(id) = remove {
        once(&mut undone, undo, project);
        project.remove_note(id);
        state.notes_editing.retain(|&x| x != id);
        resp.edited = true;
    }
}

/// A round progress dial: a dim background ring, a progress arc (0..1, clockwise from the top) and
/// centred text. `flash` swaps the arc/text to a warning colour (countdown-reached-zero blink).
fn dial(ui: &mut egui::Ui, progress: f32, text: &str, flash: bool, palette: &Palette) {
    let size = 120.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter();
    let center = rect.center();
    let radius = size * 0.42;
    painter.circle_stroke(center, radius, egui::Stroke::new(6.0, palette.border));
    let color = if flash { egui::Color32::from_rgb(220, 70, 70) } else { palette.accent };
    let progress = progress.clamp(0.0, 1.0);
    if progress > 0.0 {
        let n = ((progress * 64.0).ceil() as i32).max(1);
        let pts: Vec<egui::Pos2> = (0..=n)
            .map(|i| {
                let f = (i as f32 / n as f32) * progress;
                let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * f;
                center + egui::vec2(a.cos(), a.sin()) * radius
            })
            .collect();
        painter.add(egui::Shape::line(pts, egui::Stroke::new(6.0, color)));
    }
    painter.text(center, egui::Align2::CENTER_CENTER, text, egui::FontId::monospace(20.0), palette.text);
}

/// Timer tab: a stopwatch or countdown dial, optionally linked to one plan item.
fn timer_tab(
    ui: &mut egui::Ui,
    state: &mut PlannerState,
    project: &mut Project,
    palette: &Palette,
    _resp: &mut PlannerResponse,
) {
    let t = &mut state.timer;
    // ticking happens App-side every frame (`tick` above), so a hidden tab can't freeze the clock
    if t.running || t.done_flash {
        // smooth dial/blink updates while this tab is actually visible
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }

    ui.horizontal(|ui| {
        if ui.selectable_label(t.mode == TimerMode::Stopwatch, "Stopwatch").clicked() && !t.running {
            t.mode = TimerMode::Stopwatch;
            t.seconds = 0.0;
            t.done_flash = false;
        }
        if ui.selectable_label(t.mode == TimerMode::Countdown, "Timer").clicked() && !t.running {
            t.mode = TimerMode::Countdown;
            t.seconds = (t.minutes * 60 + t.secs) as f64;
            t.done_flash = false;
        }
    });

    if t.mode == TimerMode::Countdown && !t.running {
        ui.horizontal(|ui| {
            ui.label("Duration");
            let mut changed = false;
            changed |= ui.add(egui::DragValue::new(&mut t.minutes).range(0..=180).suffix("m")).changed();
            changed |= ui.add(egui::DragValue::new(&mut t.secs).range(0..=59).suffix("s")).changed();
            if changed {
                t.seconds = (t.minutes * 60 + t.secs) as f64;
            }
        });
    }

    ui.add_space(8.0);
    ui.vertical_centered(|ui| {
        let progress = match t.mode {
            TimerMode::Stopwatch => ((t.seconds % 60.0) / 60.0) as f32,
            TimerMode::Countdown => {
                let total = (t.minutes * 60 + t.secs).max(1) as f64;
                (t.seconds / total) as f32
            }
        };
        let flash = t.done_flash && ui.input(|i| i.time) % 1.0 < 0.5;
        dial(ui, progress, &clock_text(t.seconds), flash, palette);
    });
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        if ui.button(if t.running { "Pause" } else { "Start" }).clicked() {
            t.running = !t.running;
            t.done_flash = false;
            t.last_tick = if t.running { Some(Instant::now()) } else { None };
        }
        if ui.button("Reset").clicked() {
            t.running = false;
            t.last_tick = None;
            t.done_flash = false;
            t.seconds = if t.mode == TimerMode::Countdown { (t.minutes * 60 + t.secs) as f64 } else { 0.0 };
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label("Link to task");
        let items = flatten_plan(&project.plan);
        let current = t.linked.and_then(|id| items.iter().find(|(i, _)| *i == id)).map(|(_, s)| s.clone());
        egui::ComboBox::from_id_salt("planner_timer_link")
            .selected_text(current.as_deref().unwrap_or("(none)"))
            .show_ui(ui, |ui| {
                if ui.selectable_label(t.linked.is_none(), "(none)").clicked() {
                    t.linked = None;
                }
                for (id, title) in &items {
                    if ui.selectable_label(t.linked == Some(*id), title.as_str()).clicked() {
                        t.linked = Some(*id);
                    }
                }
            });
    });
    if let Some(id) = t.linked {
        if let Some(it) = project.plan_item_mut(id) {
            ui.weak(format!("Tracked on this task: {}", clock_text(it.tracked_seconds)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: Id, done: bool, children: Vec<PlanItem>) -> PlanItem {
        PlanItem { id, title: format!("t{id}"), done, children, ..Default::default() }
    }

    #[test]
    fn add_check_remove_via_project() {
        let mut p = Project::new();
        let a = p.plan_add(None, "Intro");
        let b = p.plan_add(Some(a), "Hook");
        assert_eq!(count_items(&p.plan), (0, 2));
        p.plan_item_mut(b).unwrap().done = true;
        assert_eq!(count_items(&p.plan), (1, 2));
        retain_incomplete(&mut p.plan);
        assert_eq!(count_items(&p.plan), (0, 1));
        p.plan_remove(a);
        assert!(p.plan.is_empty());
    }

    #[test]
    fn reorder_indent_outdent() {
        let mut items =
            vec![item(1, false, vec![]), item(2, false, vec![item(3, false, vec![])]), item(4, false, vec![])];
        // move 4 up: [1, 4, 2]
        assert!(move_item(&mut items, 4, -1));
        assert_eq!(items[1].id, 4);
        // edges stay put
        assert!(move_item(&mut items, 1, -1));
        assert_eq!(items[0].id, 1);
        // nested move found
        assert!(move_item(&mut items, 3, 1));
        // indent 4 under 1
        assert!(move_item(&mut items, 4, 1)); // back to [1, 2, 4]
        assert!(indent_item(&mut items, 4));
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].children.last().unwrap().id, 4);
        // outdent 3 next to its parent 2
        assert!(outdent_item(&mut items, 3));
        assert_eq!(items[2].id, 3);
        // outdent a top-level item: not found in any parent
        assert!(!outdent_item(&mut items, 1));
        // first sibling can't indent
        assert!(indent_item(&mut items, 1));
        assert_eq!(items[0].id, 1);
    }

    /// Headless: Plan and Timer tabs lay out; no edits (and, for the Timer, no tick) reported without
    /// interaction — a stopped timer touches nothing.
    #[test]
    fn show_headless() {
        let mut p = Project::new();
        let a = p.plan_add(None, "Intro");
        p.plan_item_mut(a).unwrap().requirements.push(("Approved".into(), false));
        p.plan_add(Some(a), "Hook");
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        let mut thumbs = ThumbCache::new(ctx.clone(), crate::media::Backend::Auto);
        for tab in [0, 2] {
            let mut state = PlannerState { tab, ..Default::default() };
            for _ in 0..2 {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut undo = |_: &Project| panic!("no undo without edits");
                        let r = show(ui, &mut state, &mut p, &mut thumbs, &palette, &mut undo);
                        assert!(!r.edited && r.add_to_timeline.is_empty());
                    });
                });
            }
        }
        assert_eq!(count_items(&p.plan), (0, 2));
    }

    /// Headless: the Notes tab renders both a note's markdown preview (`CommonMarkViewer`) and, once
    /// its id is in `notes_editing` (what the pencil/eye button toggles), its plain edit `TextEdit` —
    /// without panicking either way. A bare `RawInput::default()` has no pointer to click with, so the
    /// toggle itself is driven directly through `state`, same as `show_headless` drives tabs.
    #[test]
    fn notes_tab_render_toggle_headless() {
        let mut p = Project::new();
        let id = p.add_note("Style");
        p.note_mut(id).unwrap().body = "**bold** ideas".into();
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        let mut thumbs = ThumbCache::new(ctx.clone(), crate::media::Backend::Auto);
        let mut state = PlannerState { tab: 1, ..Default::default() };
        let mut undo = |_: &Project| panic!("no undo without edits");
        for editing in [false, true] {
            state.notes_editing.clear();
            if editing {
                state.notes_editing.push(id);
            }
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let r = show(ui, &mut state, &mut p, &mut thumbs, &palette, &mut undo);
                    assert!(!r.edited);
                });
            });
        }
    }

    #[test]
    fn flatten_plan_indents_children() {
        let mut p = Project::new();
        let a = p.plan_add(None, "Intro");
        p.plan_add(Some(a), "Hook");
        let flat = flatten_plan(&p.plan);
        assert_eq!(flat.len(), 2);
        assert_eq!(flat[0].1, "Intro");
        assert!(flat[1].1.starts_with("  "), "{:?}", flat[1].1);
        assert!(flat[1].1.trim().eq("Hook"));
    }

    /// The pure half of the Timer tab: a stopwatch counts up and banks elapsed seconds onto the linked
    /// item; a countdown counts down, clamps at zero and stops itself (the "reached zero" flash the UI
    /// layer reads via `done_flash`).
    #[test]
    fn timer_ticks_and_links_to_item() {
        let mut p = Project::new();
        let id = p.plan_add(None, "Edit");
        let mut t = TimerState { linked: Some(id), ..Default::default() };
        assert!(timer_tick(&mut t, &mut p, 2.5));
        assert_eq!(t.seconds, 2.5);
        assert_eq!(p.plan_item_mut(id).unwrap().tracked_seconds, 2.5);
        assert!(timer_tick(&mut t, &mut p, 1.0));
        assert_eq!(p.plan_item_mut(id).unwrap().tracked_seconds, 3.5, "accumulates across ticks");

        t.mode = TimerMode::Countdown;
        t.seconds = 1.0;
        t.running = true;
        assert!(timer_tick(&mut t, &mut p, 1.5), "still banks time on the tick that crosses zero");
        assert_eq!(t.seconds, 0.0, "clamped, never negative");
        assert!(!t.running, "a countdown stops itself at zero");
        assert!(t.done_flash);

        // unlinked: still ticks the dial, just nothing to bank
        t.linked = None;
        t.mode = TimerMode::Stopwatch;
        t.seconds = 0.0;
        assert!(!timer_tick(&mut t, &mut p, 1.0));
        assert_eq!(t.seconds, 1.0);
    }
}
