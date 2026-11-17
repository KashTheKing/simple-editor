//! ---- ws:animator-templates ----
//! The two template windows, as plain egui over plain state (the App side is `ui::app::template_ctl`):
//! "Use Template" lists a template's slots so each can be given footage before it is placed, and the
//! Template Builder turns clips of the timeline into a template (name, picture time, which clips are
//! slots, whether a clip's footage rides along). Both lists are the shared `list_rows` table.

use crate::engine::templates::{SlotFit, SlotWant};
use crate::model::{ClipKind, Id};
use crate::ui::list_rows::{Ctrl, Moved, RowSpec, Rows, ROW_H};
use crate::ui::DragPayload;
use eframe::egui::{self, Rect};

// ------------------------------------------------------------------------------------------ use

pub struct UseSlot {
    pub name: String,
    pub want: SlotWant,
    pub secs: f64,
    pub fit: SlotFit,
    /// The footage chosen for it: a media library asset and its name.
    pub fill: Option<(Id, String)>,
    /// The template brings footage of its own for this slot (used when nothing is chosen).
    pub sample: bool,
}

#[derive(Default)]
pub struct UseState {
    pub open: bool,
    pub name: String,
    pub slots: Vec<UseSlot>,
}

#[derive(Debug, PartialEq)]
pub enum UseAct {
    Place,
    Cancel,
    /// Choose a file for slot i.
    Pick(usize),
    Clear(usize),
    /// Fill the slots in order with the Media Library's selection.
    FromLibrary,
    OpenLibrary,
    /// Media dragged from the library onto slot i: fills it and the ones after it, in order.
    Drop(usize, Vec<Id>),
}

const USE_CTRLS: &[Ctrl] =
    &[Ctrl { glyph: "…", hint: "Choose a file" }, Ctrl { glyph: "✕", hint: "Empty this slot" }];

fn secs(s: f64) -> String {
    format!("{s:.1} s")
}

/// The slot rows (shared by the window and its test). `selected` = how many media the library has selected.
pub fn use_rows(ui: &mut egui::Ui, st: &mut UseState, selected: usize) -> Option<UseAct> {
    let mut act = None;
    ui.horizontal(|ui| {
        let r = ui.add_enabled(selected > 0, egui::Button::new("Use selected media"));
        let r = r
            .on_hover_text("Fill the slots in order with the media selected in the Media Library")
            .on_disabled_hover_text("Select media in the Media Library first");
        if r.clicked() {
            act = Some(UseAct::FromLibrary);
        }
        if ui.button("Media Library").on_hover_text("Open the Media Library; drag media from it onto a slot").clicked() {
            act = Some(UseAct::OpenLibrary);
        }
    });
    ui.add_space(4.0);
    let dragged = egui::DragAndDrop::payload::<DragPayload>(ui.ctx()).and_then(|p| match &*p {
        DragPayload::Asset(id) => Some(vec![*id]),
        DragPayload::Assets(ids) => Some(ids.clone()),
        _ => None,
    });
    let released = ui.input(|i| i.pointer.any_released());
    let pointer = ui.ctx().pointer_interact_pos();
    let mut rows = Rows::new(ui, "template_use_slots", USE_CTRLS, &[56.0, 56.0, 190.0], false);
    for i in 0..st.slots.len() {
        let s = &st.slots[i];
        let top = ui.cursor().min;
        let row = Rect::from_min_size(top, egui::vec2(ui.available_width(), ROW_H));
        let footage = match (&s.fill, s.sample) {
            (Some((_, n)), _) => n.clone(),
            (None, true) => "Template footage".to_string(),
            (None, false) => "Empty".to_string(),
        };
        // (a stretch slot takes the length of what goes in)
        let len = if s.fit == SlotFit::Stretch && s.fill.is_some() { "Footage".to_string() } else { secs(s.secs) };
        let r = rows.row(
            ui,
            i,
            &RowSpec { key: i as u64, label: &s.name, values: &[s.want.label(), &len, &footage], ..Default::default() },
        );
        match r.ctrl {
            Some(0) => act = Some(UseAct::Pick(i)),
            Some(1) => act = Some(UseAct::Clear(i)),
            _ => {}
        }
        if let (Some(ids), Some(p)) = (&dragged, pointer) {
            if row.contains(p) {
                ui.painter().rect_stroke(row, 3.0, ui.visuals().selection.stroke, egui::StrokeKind::Inside);
                if released {
                    act = Some(UseAct::Drop(i, ids.clone()));
                }
            }
        }
    }
    let _ = rows.finish(ui); // (slots keep the template's order here)
    act
}

pub fn use_window(ctx: &egui::Context, st: &mut UseState, selected: usize) -> Option<UseAct> {
    if !st.open {
        return None;
    }
    let mut open = true;
    let mut act = None;
    egui::Window::new("Use Template")
        .id(egui::Id::new("template_use"))
        .open(&mut open)
        .collapsible(false)
        .default_width(520.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            ui.strong(&st.name);
            ui.add_space(2.0);
            act = use_rows(ui, st, selected);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Place").on_hover_text("Place it at the playhead; a slot left empty stays a placeholder").clicked() {
                    act = Some(UseAct::Place);
                }
                if ui.button("Cancel").clicked() {
                    act = Some(UseAct::Cancel);
                }
            });
        });
    if !open || (act.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape))) {
        act = Some(UseAct::Cancel);
    }
    act
}

// ------------------------------------------------------------------------------------------ builder

#[derive(Clone, Debug, PartialEq)]
pub struct BuildRow {
    pub clip: Id,
    pub label: String,
    /// Video / Image, or Audio (sound can ride along or be left out, never a slot).
    pub kind: ClipKind,
    pub secs: f64,
    pub slot: bool,
    pub slot_name: String,
    pub want: SlotWant,
    pub fit: SlotFit,
    /// Its footage rides along in the template (off = a placeholder; sound: left out).
    pub include: bool,
    pub open: bool,
}

#[derive(Default)]
pub struct BuilderState {
    pub open: bool,
    pub just_opened: bool,
    pub name: String,
    pub tags: String,
    /// The whole timeline (true) or the selection it was opened with.
    pub all: bool,
    /// The selection it was opened with (empty: only "Timeline" is offered).
    pub selection: Vec<Id>,
    /// The clips of the current scope and where they start on the timeline.
    pub ids: Vec<Id>,
    pub start: f64,
    pub dur: f64,
    /// Picture time, seconds into the template.
    pub thumb_t: f64,
    pub rows: Vec<BuildRow>,
    /// Clips kept as they are (text, shapes, layers).
    pub others: usize,
    /// A template of that name exists: Save replaces it.
    pub exists: bool,
}

#[derive(Debug, PartialEq)]
pub enum BuildAct {
    Save,
    Cancel,
    /// Switched between the selection and the whole timeline: the rows are rebuilt.
    Scope,
    /// The picture time moved: show that frame.
    Seek,
}

const BUILD_CTRLS: &[Ctrl] = &[
    Ctrl { glyph: "▣", hint: "Slot: filled with your footage when the template is used" },
    Ctrl { glyph: "🎞", hint: "Footage: kept in the template, or left out (a placeholder)" },
    Ctrl { glyph: "▲", hint: "Move up (Alt+Up)" },
    Ctrl { glyph: "▼", hint: "Move down (Alt+Down)" },
];

/// Apply a list move to the rows (one step).
fn move_row(rows: &mut Vec<BuildRow>, m: Moved) {
    let n = rows.len();
    let (from, to) = match m {
        Moved::Slot { from, slot, .. } => (from, if slot > from { slot - 1 } else { slot }),
        Moved::Step { from, delta } => (from, (from as i32 + delta).clamp(0, n as i32 - 1) as usize),
        Moved::Indent { .. } => return,
    };
    if from < n && to < n && from != to {
        let r = rows.remove(from);
        rows.insert(to, r);
    }
}

/// The builder's body (shared by the window and its tests).
pub fn builder_body(ui: &mut egui::Ui, st: &mut BuilderState) -> Option<BuildAct> {
    let mut act = None;
    egui::Grid::new("template_builder_form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Name");
        let r = ui.add(egui::TextEdit::singleline(&mut st.name).desired_width(260.0));
        if std::mem::take(&mut st.just_opened) {
            r.request_focus();
        }
        ui.end_row();
        ui.label("Tags");
        ui.add(egui::TextEdit::singleline(&mut st.tags).hint_text("comma, separated").desired_width(260.0));
        ui.end_row();
        ui.label("From");
        ui.horizontal(|ui| {
            let sel = ui.add_enabled(!st.selection.is_empty(), egui::RadioButton::new(!st.all, "Selection"));
            if sel.on_disabled_hover_text("Nothing was selected").clicked() && st.all {
                st.all = false;
                act = Some(BuildAct::Scope);
            }
            if ui.radio(st.all, "Timeline").clicked() && !st.all {
                st.all = true;
                act = Some(BuildAct::Scope);
            }
        });
        ui.end_row();
        ui.label("Picture at");
        let r = ui.add(egui::Slider::new(&mut st.thumb_t, 0.0..=st.dur.max(0.01)).suffix(" s").fixed_decimals(2));
        if r.on_hover_text("The frame shown on its Gallery card").changed() {
            act = Some(BuildAct::Seek);
        }
        ui.end_row();
    });
    ui.add_space(6.0);
    ui.strong("Footage");
    if st.rows.is_empty() {
        ui.weak("No footage in these clips.");
    }
    let mut rows = Rows::new(ui, "template_builder_rows", BUILD_CTRLS, &[70.0, 86.0, 52.0], false);
    let mut step = None;
    for i in 0..st.rows.len() {
        let r = &st.rows[i];
        let sound = r.kind == ClipKind::Audio;
        let slot = if r.slot { "Slot" } else if sound { "Sound" } else { "Fixed" };
        let footage = match (r.include, sound) {
            (true, _) => "Included",
            (false, true) => "Left out",
            (false, false) => "Placeholder",
        };
        let len = secs(r.secs);
        let label = if r.slot && !r.slot_name.is_empty() { format!("{}  ({})", r.slot_name, r.label) } else { r.label.clone() };
        let resp = rows.row(
            ui,
            i,
            &RowSpec {
                key: r.clip,
                label: &label,
                open: (!sound).then_some(r.open),
                values: &[slot, footage, &len],
                ..Default::default()
            },
        );
        let r = &mut st.rows[i];
        if resp.toggled {
            r.open = !r.open;
        }
        match resp.ctrl {
            Some(0) if !sound => {
                r.slot = !r.slot;
                r.open = r.slot;
                // (a placeholder has to be a slot: turning the slot off brings the footage back)
                r.include |= !r.slot;
                if r.slot && r.slot_name.is_empty() {
                    r.slot_name = r.label.clone();
                }
            }
            Some(1) => {
                r.include = !r.include;
                if !r.include && !sound {
                    r.slot = true;
                    if r.slot_name.is_empty() {
                        r.slot_name = r.label.clone();
                    }
                }
            }
            Some(2) => step = Some(Moved::Step { from: i, delta: -1 }),
            Some(3) => step = Some(Moved::Step { from: i, delta: 1 }),
            _ => {}
        }
        if r.open && r.slot {
            ui.indent(("template_builder_row", r.clip), |ui| {
                egui::Grid::new(("template_builder_slot", r.clip)).num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                    ui.label("Slot name");
                    ui.add(egui::TextEdit::singleline(&mut r.slot_name).desired_width(200.0));
                    ui.end_row();
                    ui.label("Wants");
                    ui.horizontal(|ui| {
                        for w in SlotWant::ALL {
                            ui.selectable_value(&mut r.want, w, w.label());
                        }
                    });
                    ui.end_row();
                    ui.label("Length");
                    ui.horizontal(|ui| {
                        for f in SlotFit::ALL {
                            ui.selectable_value(&mut r.fit, f, f.label()).on_hover_text(match f {
                                SlotFit::Trim => "The slot keeps its length: longer footage is cut, shorter footage is slowed down",
                                SlotFit::Stretch => "The slot takes the length of the footage and what comes after it moves along",
                            });
                        }
                    });
                    ui.end_row();
                });
            });
        }
    }
    if let Some(m) = rows.finish(ui).or(step) {
        move_row(&mut st.rows, m);
    }
    if st.others > 0 {
        ui.weak(format!("{} more clip{} (text, shapes, layers) stay as they are.", st.others, if st.others == 1 { "" } else { "s" }));
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let ok = !st.name.trim().is_empty() && !st.ids.is_empty();
        let label = if st.exists { "Replace" } else { "Save" };
        let r = ui.add_enabled(ok, egui::Button::new(label));
        if r.on_hover_text("Saved to your templates in the Gallery").on_disabled_hover_text("Give it a name").clicked() {
            act = Some(BuildAct::Save);
        }
        if ui.button("Cancel").clicked() {
            act = Some(BuildAct::Cancel);
        }
    });
    act
}

pub fn builder_window(ctx: &egui::Context, st: &mut BuilderState) -> Option<BuildAct> {
    if !st.open {
        return None;
    }
    let mut open = true;
    let mut act = None;
    egui::Window::new("Template Builder")
        .id(egui::Id::new("template_builder"))
        .open(&mut open)
        .collapsible(false)
        .default_width(560.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(ctx.content_rect().height() * 0.7).show(ui, |ui| {
                act = builder_body(ui, st);
            });
        });
    if !open {
        act = Some(BuildAct::Cancel);
    }
    act
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(clip: Id, label: &str, kind: ClipKind) -> BuildRow {
        BuildRow {
            clip,
            label: label.into(),
            kind,
            secs: 3.0,
            slot: false,
            slot_name: String::new(),
            want: SlotWant::Any,
            fit: SlotFit::Trim,
            include: true,
            open: false,
        }
    }

    fn builder() -> BuilderState {
        BuilderState {
            open: true,
            name: "Trip".into(),
            ids: vec![1, 2, 3],
            dur: 9.0,
            rows: vec![
                row(1, "beach.mp4", ClipKind::Video),
                row(2, "a much longer file name that has to be cut off somewhere.png", ClipKind::Image),
                row(3, "music.mp3", ClipKind::Audio),
            ],
            others: 2,
            ..Default::default()
        }
    }

    /// 30 idle frames, no input: neither window may ask for a repaint while it sits open and unchanged.
    #[test]
    fn assert_no_idle_repaint_template_builder() {
        let ctx = egui::Context::default();
        let mut st = builder();
        st.rows[0].slot = true;
        st.rows[0].open = true;
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                let _ = builder_window(ctx, &mut st);
            });
        }
        assert!(!ctx.has_requested_repaint(), "an idle Template Builder requested a repaint");
    }

    #[test]
    fn assert_no_idle_repaint_template_use_window() {
        let ctx = egui::Context::default();
        let mut st = UseState {
            open: true,
            name: "Photo Slideshow".into(),
            slots: (0..4)
                .map(|i| UseSlot {
                    name: format!("Photo {}", i + 1),
                    want: SlotWant::Image,
                    secs: 3.0,
                    fit: SlotFit::Trim,
                    fill: (i == 0).then(|| (7, "beach.png".to_string())),
                    sample: false,
                })
                .collect(),
        };
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                let _ = use_window(ctx, &mut st, 2);
            });
        }
        assert!(!ctx.has_requested_repaint(), "an idle Use Template window requested a repaint");
    }

    /// The controls sit at one x whatever the label's length (the list rule).
    #[test]
    fn builder_rows_keep_their_controls_in_one_column() {
        let ctx = egui::Context::default();
        let mut xs = Vec::new();
        let st = builder();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut rows = Rows::new(ui, "t", BUILD_CTRLS, &[70.0, 86.0, 52.0], false);
                for (i, r) in st.rows.iter().enumerate() {
                    xs.push(rows.row(ui, i, &RowSpec { key: r.clip, label: &r.label, values: &["Slot", "Included", "3.0 s"], ..Default::default() }).ctrl_x);
                }
                let _ = rows.finish(ui);
            });
        });
        assert_eq!(xs.len(), 3);
        assert!(xs.iter().all(|x| (x - xs[0]).abs() < 0.01), "{xs:?}");
    }

    #[test]
    fn rows_reorder_by_step_and_by_drop() {
        let mut rows = builder().rows;
        move_row(&mut rows, Moved::Step { from: 0, delta: 1 });
        assert_eq!(rows.iter().map(|r| r.clip).collect::<Vec<_>>(), [2, 1, 3]);
        move_row(&mut rows, Moved::Slot { from: 2, slot: 0, depth: None });
        assert_eq!(rows.iter().map(|r| r.clip).collect::<Vec<_>>(), [3, 2, 1]);
        move_row(&mut rows, Moved::Step { from: 0, delta: -1 }); // already first: stays
        assert_eq!(rows[0].clip, 3);
    }
}
