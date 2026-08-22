//! Planner pane: tabs "Plan" and "Notes".
//! Plan = a nested to-do tree (Project.plan): each row = checkbox (done), editable title, colour label dot
//! (click → LABEL_COLORS menu), ▸/▾ collapse, "+" add sub-task, "✕" remove, ↑/↓ reorder among siblings,
//! "⇥"/"⇤" indent/outdent; a selected item shows its notes (multiline) and its moodboard: the asset ids in
//! `assets` as rows (name, kind tag, thumbnail via ThumbCache if available, per-asset note in `asset_notes`,
//! "Add to timeline", "✕"); assets are added by dropping library items (DragPayload::Asset) onto the item or
//! onto the moodboard area. Progress bar "done/total" at the top; "Add task" at the top level;
//! "Clear completed". Notes tab = Project.notes as a large multiline TextEdit ("describe your process,
//! ideas, style while you edit — the style summary / AI tools read this"). Undo once per gesture.
//! Returns what changed.

use crate::media::thumbs::ThumbCache;
use crate::model::{ClipKind, Id, PlanItem, Project, LABEL_COLORS};
use crate::theme::Palette;
use crate::ui::markers_ui::x_button;
use crate::ui::{edit_start, label_color, once, DragPayload};
use eframe::egui::{self, Response, RichText, TextEdit};

#[derive(Default)]
pub struct PlannerState {
    pub tab: usize,
    pub selected: Option<Id>,
    pub collapsed: Vec<Id>,
}

#[derive(Default)]
pub struct PlannerResponse {
    pub edited: bool,
    /// Asset ids the user asked to place on the timeline at the playhead.
    pub add_to_timeline: Vec<Id>,
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
    ui.menu_button(RichText::new("●").color(label_color(current, palette)), |ui| {
        if ui.button("None").clicked() {
            picked = Some(0);
            ui.close();
        }
        for (i, (name, [r, g, b])) in LABEL_COLORS.iter().enumerate() {
            let t = RichText::new(format!("● {name}")).color(egui::Color32::from_rgb(*r, *g, *b));
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
                } else if ui.small_button(if closed { "▸" } else { "▾" }).clicked() {
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
                let mut op = |o: Op| *t.op = Some(o);
                if ui.small_button("+").on_hover_text("Add sub-task").clicked() {
                    op(Op::Add(Some(it.id)));
                }
                if ui.small_button("↑").clicked() {
                    op(Op::Up(it.id));
                }
                if ui.small_button("↓").clicked() {
                    op(Op::Down(it.id));
                }
                if ui.small_button("⇤").on_hover_text("Outdent").clicked() {
                    op(Op::Out(it.id));
                }
                if ui.small_button("⇥").on_hover_text("Indent").clicked() {
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
    });
    ui.separator();

    if state.tab == 1 {
        let mut notes = project.notes.clone();
        let r = ui.add_sized(
            ui.available_size(),
            TextEdit::multiline(&mut notes).hint_text(
                "Describe your process, ideas and style while you edit — the style summary / AI tools read this.",
            ),
        );
        // one undo entry per visit to the notes field, not per keystroke
        if r.gained_focus() {
            once(&mut undone, undo, project);
        }
        if r.changed() {
            project.notes = notes;
            resp.edited = true;
        }
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

    /// Headless: both tabs lay out; no edits reported without interaction.
    #[test]
    fn show_headless() {
        let mut p = Project::new();
        let a = p.plan_add(None, "Intro");
        p.plan_add(Some(a), "Hook");
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        let mut thumbs = ThumbCache::new(ctx.clone(), crate::media::Backend::Auto);
        for tab in [0, 1] {
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
}
