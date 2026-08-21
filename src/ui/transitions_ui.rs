//! Transitions panel. Catalogue: TransitionKind::ALL with a default duration DragValue (0.1..5 s) and,
//! for FadeToColor a colour button, for Push/Wipe a direction selector. "Add at start of selected clip"
//! (Project::add_transition(right = selected clip)) and "Add at end of selected clip" (right = the clip
//! that starts where the selected one ends, on the same track) — disabled with a hint when the cut has no
//! abutting neighbour. Below: the transitions touching the selected clip (Project::transitions_of):
//! kind combo, duration, colour/direction/ease editors, ✕ remove. Returns true when the project changed
//! (call `undo` once per gesture first).

use crate::model::{Ease, Id, Project, Transition, TransitionKind, ABUT_EPS};
use crate::theme::Palette;
use eframe::egui::{self, Button, DragValue, Response};

#[cfg(test)]
use crate::ui::effects_ui::test_rects;

pub struct TransitionsState {
    /// Catalogue settings for the next transition to add.
    pub duration: f64,
    pub kind: usize,
    pub color: [u8; 4],
    pub direction: u8,
}

impl Default for TransitionsState {
    fn default() -> Self {
        Self { duration: 1.0, kind: 0, color: [0, 0, 0, 255], direction: 0 }
    }
}

fn edit_start(r: &Response) -> bool {
    r.drag_started() || (r.changed() && !r.dragged())
}

#[derive(Default)]
struct Gesture {
    start: bool,
    changed: bool,
}

impl Gesture {
    fn note(&mut self, r: &Response) {
        self.start |= edit_start(r);
        self.changed |= r.changed();
    }
    fn click(&mut self) {
        self.start = true;
        self.changed = true;
    }
}

/// The clip starting exactly where `id` ends, on the same track.
pub(crate) fn right_neighbor(project: &Project, id: Id) -> Option<Id> {
    let ti = project.track_of(id)?;
    let c = project.clip(id)?;
    project.tracks[ti].clips.iter().find(|o| o.id != id && (o.start - c.end()).abs() < ABUT_EPS).map(|o| o.id)
}

fn direction_row(ui: &mut egui::Ui, dir: &mut u8, g: &mut Gesture) {
    for (i, name) in ["Left", "Right", "Up", "Down"].iter().enumerate() {
        g.note(&ui.selectable_value(dir, i as u8, *name));
    }
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut TransitionsState,
    project: &mut Project,
    selection: &[Id],
    _playhead: f64,
    _palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    #[cfg(test)]
    test_rects::clear();
    let mut changed = false;
    let mut g = Gesture::default();

    // ---- catalogue / defaults for the next transition ----
    ui.strong("Transitions");
    state.kind = state.kind.min(TransitionKind::ALL.len() - 1);
    let kind = TransitionKind::ALL[state.kind];
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("tr_add_kind").selected_text(kind.name()).show_ui(ui, |ui| {
            for (i, k) in TransitionKind::ALL.iter().enumerate() {
                ui.selectable_value(&mut state.kind, i, k.name());
            }
        });
        ui.add(DragValue::new(&mut state.duration).range(0.1..=5.0).speed(0.02).suffix(" s"));
        if kind == TransitionKind::FadeToColor {
            ui.color_edit_button_srgba_unmultiplied(&mut state.color);
        }
    });
    if kind.has_direction() {
        ui.horizontal(|ui| {
            let mut dummy = Gesture::default();
            direction_row(ui, &mut state.direction, &mut dummy);
        });
    }

    // ---- add at the cuts around the selected clip ----
    let sel = selection.iter().copied().find(|&id| project.clip(id).is_some());
    let Some(sel) = sel else {
        ui.label("Select a clip");
        return false;
    };
    let kind = TransitionKind::ALL[state.kind];
    let has_left = project
        .track_of(sel)
        .and_then(|ti| project.clip(sel).map(|c| (ti, c.clone())))
        .is_some_and(|(ti, c)| project.tracks[ti].left_of(&c).is_some());
    let right_of_end = right_neighbor(project, sel);
    let mut add_right_clip: Option<Id> = None; // the clip forming the right side of the cut
    ui.horizontal(|ui| {
        let r = ui
            .add_enabled(has_left, Button::new("Add at start"))
            .on_hover_text("Transition into the selected clip")
            .on_disabled_hover_text("No clip ends where the selected one starts");
        #[cfg(test)]
        test_rects::push("add_start".into(), r.rect);
        if r.clicked() {
            add_right_clip = Some(sel);
        }
        let r = ui
            .add_enabled(right_of_end.is_some(), Button::new("Add at end"))
            .on_hover_text("Transition out of the selected clip")
            .on_disabled_hover_text("No clip starts where the selected one ends");
        #[cfg(test)]
        test_rects::push("add_end".into(), r.rect);
        if r.clicked() {
            add_right_clip = right_of_end;
        }
    });
    if let Some(right) = add_right_clip {
        undo(project);
        if let Some(tid) = project.add_transition(right, kind, state.duration) {
            if let Some(t) = project.transition_mut(tid) {
                t.color = state.color;
                t.direction = state.direction;
            }
            changed = true;
        }
    }

    // ---- transitions touching the selected clip ----
    ui.separator();
    let list: Vec<Transition> = project.transitions_of(sel).iter().map(|&(_, t)| t.clone()).collect();
    if list.is_empty() {
        ui.label("No transitions on this clip");
    }
    let mut writes: Vec<Transition> = Vec::new();
    let mut removes: Vec<Id> = Vec::new();
    for (_i, mut tr) in list.into_iter().enumerate() {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(("tr_kind", tr.id)).selected_text(tr.kind.name()).show_ui(ui, |ui| {
                for k in TransitionKind::ALL {
                    g.note(&ui.selectable_value(&mut tr.kind, k, k.name()));
                }
            });
            // clamp_existing_to_range(false): clamping an out-of-range duration counts as a change and
            // would fake an edit (undo snapshot + dirty flag) just by drawing the panel.
            let r = ui.add(
                DragValue::new(&mut tr.duration)
                    .range(0.1..=5.0)
                    .clamp_existing_to_range(false)
                    .speed(0.02)
                    .suffix(" s"),
            );
            #[cfg(test)]
            test_rects::push(format!("tr_dur{_i}"), r.rect);
            g.note(&r);
            if tr.kind == TransitionKind::FadeToColor {
                g.note(&ui.color_edit_button_srgba_unmultiplied(&mut tr.color));
            }
            egui::ComboBox::from_id_salt(("tr_ease", tr.id)).selected_text(tr.ease.name()).show_ui(ui, |ui| {
                for e in Ease::ALL {
                    g.note(&ui.selectable_value(&mut tr.ease, e, e.name()));
                }
            });
            let r = ui.add(Button::new("✕").small());
            #[cfg(test)]
            test_rects::push(format!("tr_del{_i}"), r.rect);
            if r.clicked() {
                removes.push(tr.id);
                g.click();
            }
        });
        if tr.kind.has_direction() {
            ui.horizontal(|ui| {
                direction_row(ui, &mut tr.direction, &mut g);
            });
        }
        writes.push(tr);
    }
    if g.start {
        undo(project);
    }
    if g.changed {
        for w in writes {
            if !removes.contains(&w.id) {
                if let Some(t) = project.transition_mut(w.id) {
                    *t = w;
                }
            }
        }
        for id in removes {
            project.remove_transition(id);
        }
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, AudioStreamInfo, ClipKind, TrackKind};
    use eframe::egui::{vec2, Color32, Event, Modifiers, PointerButton, Pos2, RawInput, Rect};

    struct Harness {
        ctx: egui::Context,
        state: TransitionsState,
        project: Project,
        selection: Vec<Id>,
        undos: usize,
        time: f64,
    }

    impl Harness {
        /// Two linked video+audio clip pairs abutting at t = 10.
        fn new() -> Self {
            let mut project = Project::new();
            let aid = project.add_asset(Asset {
                id: 0,
                path: "C:/t.mp4".into(),
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
            project.insert_asset_clips(aid, 0.0, Some(0));
            project.insert_asset_clips(aid, 10.0, Some(0));
            Self {
                ctx: egui::Context::default(),
                state: TransitionsState::default(),
                project,
                selection: Vec::new(),
                undos: 0,
                time: 0.0,
            }
        }
        fn frame(&mut self, events: Vec<Event>) -> bool {
            self.time += 0.05;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, Color32::WHITE);
            let Harness { ctx, state, project, selection, undos, .. } = self;
            let mut changed = false;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    changed |= show(ui, state, project, selection, 0.0, &pal, &mut undo);
                });
            });
            changed
        }
        fn click(&mut self, pos: Pos2) -> bool {
            self.frame(vec![Event::PointerMoved(pos)]);
            let mut e = self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }]);
            e |= self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }]);
            e |= self.frame(vec![]);
            e
        }
    }

    #[test]
    fn add_at_start_creates_transition_and_audio_mirror() {
        let mut h = Harness::new();
        // second video clip (starts at 10) — its cut with the first is at its start
        let v2 = h.project.tracks[0].clips[1].id;
        let a2 = h.project.tracks[1].clips[1].id;
        h.selection = vec![v2];
        h.frame(vec![]);
        let r = test_rects::get("add_start").expect("add button recorded");
        assert!(h.click(r.center()));
        assert_eq!(h.undos, 1);
        let video_tr: Vec<_> = h.project.tracks[0].transitions.iter().collect();
        assert_eq!(video_tr.len(), 1);
        assert_eq!(video_tr[0].right, v2);
        assert_eq!(video_tr[0].kind, TransitionKind::CrossFade);
        assert!((video_tr[0].duration - 1.0).abs() < 1e-9);
        let audio_tr: Vec<_> = h.project.tracks[1].transitions.iter().collect();
        assert_eq!(audio_tr.len(), 1, "linked audio clip should get the mirrored crossfade");
        assert_eq!(audio_tr[0].right, a2);
        assert_eq!(audio_tr[0].kind, TransitionKind::CrossFade);
    }

    #[test]
    fn add_at_end_uses_right_neighbor_and_remove_deletes() {
        let mut h = Harness::new();
        let v1 = h.project.tracks[0].clips[0].id;
        let v2 = h.project.tracks[0].clips[1].id;
        h.selection = vec![v1];
        h.frame(vec![]);
        let r = test_rects::get("add_end").expect("add button recorded");
        assert!(h.click(r.center()));
        assert_eq!(h.project.tracks[0].transitions.len(), 1);
        assert_eq!(h.project.tracks[0].transitions[0].right, v2, "end of v1 = cut whose right side is v2");
        assert_eq!(h.undos, 1);
        // the transition touches v1, so it is listed; remove it
        h.frame(vec![]);
        let r = test_rects::get("tr_del0").expect("remove button recorded");
        assert!(h.click(r.center()));
        assert!(h.project.tracks[0].transitions.is_empty());
        assert_eq!(h.undos, 2);
    }

    #[test]
    fn no_neighbor_disables_add() {
        let mut h = Harness::new();
        // select the FIRST clip: nothing ends at its start (t = 0), so "Add at start" is disabled
        let v1 = h.project.tracks[0].clips[0].id;
        h.selection = vec![v1];
        h.frame(vec![]);
        let r = test_rects::get("add_start").expect("add button recorded");
        assert!(!h.click(r.center()), "disabled button must not add");
        assert!(h.project.tracks[0].transitions.is_empty());
        assert_eq!(h.undos, 0);
    }

    #[test]
    fn out_of_range_duration_is_not_rewritten_by_merely_showing() {
        let mut h = Harness::new();
        let v1 = h.project.tracks[0].clips[0].id;
        let v2 = h.project.tracks[0].clips[1].id;
        let tid = h.project.add_transition(v2, TransitionKind::CrossFade, 1.0).unwrap();
        h.project.transition_mut(tid).unwrap().duration = 8.0;
        h.selection = vec![v1];
        assert!(!h.frame(vec![]), "drawing the panel must not report an edit");
        assert_eq!(h.undos, 0, "no undo snapshot without a user gesture");
        assert!((h.project.transitions_of(v1)[0].1.duration - 8.0).abs() < 1e-9);
    }

    #[test]
    fn right_neighbor_finds_abutting_clip() {
        let h = Harness::new();
        let v1 = h.project.tracks[0].clips[0].id;
        let v2 = h.project.tracks[0].clips[1].id;
        assert_eq!(right_neighbor(&h.project, v1), Some(v2));
        assert_eq!(right_neighbor(&h.project, v2), None);
    }
}
