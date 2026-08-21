//! Retime window (Ctrl+R): speed / reverse / freeze for the selected clips (linked clips follow).
//! egui::Window "Speed / Retime": speed as a percent DragValue (1..10000 %, 100 = normal) + preset buttons
//! (25 % 50 % 100 % 200 % 400 %), "Reverse" checkbox, "Freeze frame at playhead" button
//! (Project::freeze_at(playhead, selection)) and "Unfreeze" (clear `freeze`), "Apply" → Project::set_speed
//! (shows a note when the new length collides with a neighbour and nothing changed). Shows the resulting
//! duration live. Undo once per applied change. Returns true when the project changed.

use crate::model::{Id, Project};
use crate::ui::duration_text;
use eframe::egui;

#[derive(Default)]
pub struct RetimeUi {
    pub open: bool,
    /// Percent being edited (synced from the first selected clip when the window opens / selection changes).
    pub percent: f64,
    pub reverse: bool,
    pub last_clip: Option<Id>,
    pub note: String,
}

const NO_ROOM: &str = "No room: the new length would collide with a neighbouring clip.";

/// Apply speed/reverse to the selection with a single undo. False (and a note) when nothing changed.
fn apply_speed(
    project: &mut Project,
    selection: &[Id],
    speed: f64,
    reverse: bool,
    note: &mut String,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let pre = project.clone();
    if project.set_speed(selection, speed, reverse) {
        undo(&pre);
        note.clear();
        true
    } else {
        *note = NO_ROOM.into();
        false
    }
}

/// Freeze only the clips the playhead is actually over: `freeze_at` stores `src_time(playhead)`, which is
/// an out-of-range (often negative) source time for a clip elsewhere on the timeline.
fn apply_freeze(project: &mut Project, selection: &[Id], playhead: f64, undo: &mut dyn FnMut(&Project)) -> bool {
    let sel: Vec<Id> =
        selection.iter().copied().filter(|&id| project.clip(id).is_some_and(|c| c.contains(playhead))).collect();
    if sel.is_empty() {
        return false;
    }
    let pre = project.clone();
    if project.freeze_at(playhead, &sel).is_empty() {
        false
    } else {
        undo(&pre);
        true
    }
}

fn apply_unfreeze(project: &mut Project, selection: &[Id], undo: &mut dyn FnMut(&Project)) -> bool {
    let ids = project.expand_links(selection);
    if !ids.iter().any(|&id| project.clip(id).is_some_and(|c| c.freeze.is_some())) {
        return false;
    }
    let pre = project.clone();
    undo(&pre);
    for id in ids {
        if let Some(c) = project.clip_mut(id) {
            c.freeze = None;
        }
    }
    true
}

pub fn show(
    ctx: &egui::Context,
    state: &mut RetimeUi,
    project: &mut Project,
    selection: &[Id],
    playhead: f64,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    if !state.open {
        state.last_clip = None;
        return false;
    }
    let first = selection.iter().copied().find(|&id| project.clip(id).is_some());
    if state.last_clip != first {
        state.last_clip = first;
        state.note.clear();
        if let Some(c) = first.and_then(|id| project.clip(id)) {
            state.percent = c.speed * 100.0;
            state.reverse = c.reverse;
        }
    }
    let mut changed = false;
    let mut open = state.open;
    egui::Window::new("Speed / Retime").open(&mut open).resizable(false).show(ctx, |ui| {
        let Some(clip) = first.and_then(|id| project.clip(id)).cloned() else {
            ui.weak("Select a clip in the timeline.");
            return;
        };
        ui.horizontal(|ui| {
            ui.label("Speed");
            ui.add(egui::DragValue::new(&mut state.percent).range(1.0..=10000.0).suffix(" %").speed(1.0));
        });
        ui.horizontal(|ui| {
            for p in [25.0, 50.0, 100.0, 200.0, 400.0] {
                if ui.small_button(format!("{p:.0} %")).clicked() {
                    state.percent = p;
                }
            }
        });
        ui.checkbox(&mut state.reverse, "Reverse");
        let new_dur =
            if clip.freeze.is_some() { clip.duration } else { clip.src_len() / (state.percent / 100.0).max(0.0001) };
        ui.weak(format!("Duration: {} → {}", duration_text(clip.duration), duration_text(new_dur)));
        if let Some(f) = clip.freeze {
            ui.weak(format!("Frozen at source {}", duration_text(f)));
        }
        ui.horizontal(|ui| {
            if ui.button("Apply").clicked() {
                changed |= apply_speed(project, selection, state.percent / 100.0, state.reverse, &mut state.note, undo);
            }
            if ui
                .add_enabled(clip.contains(playhead), egui::Button::new("Freeze frame at playhead"))
                .on_disabled_hover_text("Move the playhead over the clip to freeze a frame.")
                .clicked()
                && apply_freeze(project, selection, playhead, undo)
            {
                changed = true;
            }
            if clip.freeze.is_some() && ui.button("Unfreeze").clicked() && apply_unfreeze(project, selection, undo) {
                changed = true;
            }
        });
        if !state.note.is_empty() {
            ui.colored_label(ui.visuals().warn_fg_color, &state.note);
        }
    });
    state.open = open;
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, ClipKind};

    fn project_10s() -> (Project, Id) {
        let a = Asset {
            id: 0,
            path: r"C:\media\a.mp4".into(),
            kind: ClipKind::Video,
            duration: 10.0,
            width: 1280,
            height: 720,
            fps: 30.0,
            audio_streams: Vec::new(),
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
        };
        let p = Project::from_media(a);
        let id = p.tracks[0].clips[0].id;
        (p, id)
    }

    #[test]
    fn apply_2x_halves_duration_one_undo() {
        let (mut p, id) = project_10s();
        let mut undos = 0;
        let mut note = String::new();
        let mut undo = |pre: &Project| {
            undos += 1;
            assert!((pre.clip(id).unwrap().duration - 10.0).abs() < 1e-9, "undo sees the pre-change project");
        };
        assert!(apply_speed(&mut p, &[id], 2.0, false, &mut note, &mut undo));
        assert_eq!(undos, 1);
        assert!(note.is_empty());
        assert!((p.clip(id).unwrap().duration - 5.0).abs() < 1e-9);
    }

    #[test]
    fn collision_leaves_note_and_no_undo() {
        let (mut p, id) = project_10s();
        // blocker right after the clip on the same track: half speed would double the duration into it
        p.tracks[0].clips.push(crate::model::Clip::new(999, ClipKind::Text, "block", 10.0, 2.0));
        let mut undos = 0;
        let mut note = String::new();
        assert!(!apply_speed(&mut p, &[id], 0.5, false, &mut note, &mut |_| undos += 1));
        assert_eq!(undos, 0);
        assert_eq!(note, NO_ROOM);
        assert!((p.clip(id).unwrap().duration - 10.0).abs() < 1e-9);
    }

    #[test]
    fn freeze_and_unfreeze() {
        let (mut p, id) = project_10s();
        let mut undos = 0;
        assert!(apply_freeze(&mut p, &[id], 4.0, &mut |_| undos += 1));
        assert_eq!(undos, 1);
        let frozen: Vec<Id> = p.all_clips().filter(|(_, c)| c.freeze.is_some()).map(|(_, c)| c.id).collect();
        assert!(!frozen.is_empty());
        assert!(apply_unfreeze(&mut p, &frozen, &mut |_| undos += 1));
        assert_eq!(undos, 2);
        assert!(p.all_clips().all(|(_, c)| c.freeze.is_none()));
        // nothing frozen → no-op, no undo
        assert!(!apply_unfreeze(&mut p, &[id], &mut |_| undos += 1));
        assert_eq!(undos, 2);
    }

    /// The playhead outside the clip is not a freeze: no undo, no negative source time.
    #[test]
    fn freeze_outside_clip_is_a_no_op() {
        let (mut p, id) = project_10s();
        p.clip_mut(id).unwrap().start = 10.0; // clip spans 10..20
        let mut undos = 0;
        assert!(!apply_freeze(&mut p, &[id], 0.0, &mut |_| undos += 1));
        assert_eq!(undos, 0);
        assert!(p.all_clips().all(|(_, c)| c.freeze.is_none()));
        // inside the clip still works
        assert!(apply_freeze(&mut p, &[id], 12.0, &mut |_| undos += 1));
        assert_eq!(undos, 1);
        assert!(p.all_clips().any(|(_, c)| c.freeze.is_some_and(|f| f >= 0.0)));
    }

    /// Headless: window lays out (with and without a selection) and reports no change.
    #[test]
    fn show_headless() {
        let (mut p, id) = project_10s();
        let ctx = egui::Context::default();
        for sel in [vec![], vec![id]] {
            let mut state = RetimeUi { open: true, ..Default::default() };
            for _ in 0..2 {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    let mut undo = |_: &Project| {};
                    assert!(!show(ctx, &mut state, &mut p, &sel, 0.0, &mut undo));
                });
            }
            if !sel.is_empty() {
                assert_eq!(state.last_clip, Some(id));
                assert!((state.percent - 100.0).abs() < 1e-9);
            }
        }
    }
}
