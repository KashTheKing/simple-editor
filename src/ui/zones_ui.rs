//! ---- ws:zones-markers ----
//! The Inspector's card for the effect zone picked in the timeline's zone lane: what it covers (the whole
//! timeline or one track), where (from / length), and its effect stack - the very stack editor clips use
//! (`effects_ui::stack`, keyframes included), run on a stand-in adjustment clip whose time is the zone's.

use crate::model::{Clip, ClipKind, Effect, EffectKind, Id, Project, TrackKind};
use crate::theme::Palette;
use crate::ui::{menu, timecode};
use eframe::egui::{self, DragValue, RichText};
use std::cell::Cell;

thread_local! {
    static PICKED: Cell<Option<Id>> = const { Cell::new(None) };
}

/// The zone picked in the zone lane (the Inspector shows its card instead of the clip's).
pub fn selected() -> Option<Id> {
    PICKED.with(|p| p.get())
}

pub fn select(id: Option<Id>) {
    PICKED.with(|p| p.set(id));
}

/// The stand-in clip's id: any id no project hands out.
const STAND_IN: Id = u64::MAX - 7;

/// The card. True when it edited the project (one undo step per gesture, through `undo`).
pub fn card(ui: &mut egui::Ui, project: &mut Project, playhead: f64, palette: &Palette, undo: &mut dyn FnMut(&Project)) -> bool {
    let Some(z) = selected().and_then(|id| project.zone(id).cloned()) else {
        select(None);
        return false;
    };
    let mut e = z.clone();
    let mut g = crate::ui::Gesture::default();
    let mut remove = false;
    let fps = project.fps;
    let tracks: Vec<(Id, String)> =
        project.tracks.iter().filter(|t| t.kind == TrackKind::Video).map(|t| (t.id, t.name.clone())).collect();

    ui.horizontal(|ui| {
        ui.heading(RichText::new(z.label()).size(16.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::ui::markers_ui::x_button(ui).on_hover_text("Delete the zone").clicked() {
                remove = true;
            }
        });
    });
    ui.add_space(4.0);
    // two aligned columns: labels at one x, fields at one x
    egui::Grid::new("zone_form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Name");
        g.note_text(&ui.add(egui::TextEdit::singleline(&mut e.name).hint_text(z.label()).desired_width(180.0)));
        ui.end_row();

        ui.label("Applies to");
        let shown = match e.track.and_then(|id| tracks.iter().find(|(t, _)| *t == id)) {
            Some((_, n)) => n.clone(),
            None => "Whole timeline".to_string(),
        };
        let mut pick: Option<Option<Id>> = None;
        egui::ComboBox::from_id_salt("zone_track").selected_text(shown).width(180.0).show_ui(ui, |ui| {
            if ui.selectable_label(e.track.is_none(), "Whole timeline").on_hover_text("Every track below it").clicked() {
                pick = Some(None);
            }
            for (id, name) in &tracks {
                if ui.selectable_label(e.track == Some(*id), name).clicked() {
                    pick = Some(Some(*id));
                }
            }
        });
        if let Some(p) = pick.filter(|p| *p != e.track) {
            e.track = p;
            g.start = true;
            g.changed = true;
        }
        ui.end_row();

        ui.label("From");
        let r = ui.add(DragValue::new(&mut e.t).range(0.0..=1e6).speed(0.05).custom_formatter(|v, _| timecode(v, fps)).custom_parser(move |s| crate::ui::parse_timecode(s, fps, 0.0)));
        g.note(&r);
        ui.end_row();

        ui.label("Length");
        let r = ui.add(
            DragValue::new(&mut e.dur)
                .range(crate::model::ops::zones::MIN_ZONE..=1e6)
                .speed(0.05)
                .custom_formatter(|v, _| timecode(v, fps))
                .custom_parser(move |s| crate::ui::parse_timecode(s, fps, 0.0)),
        );
        g.note(&r);
        ui.end_row();

        ui.label("On");
        g.note(&ui.checkbox(&mut e.enabled, ""));
        ui.end_row();
    });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.strong("Effects");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            menu::button(ui, "Add effect", |ui| {
                for cat in crate::ui::effects_ui::categories() {
                    let kinds: Vec<EffectKind> =
                        EffectKind::ALL.iter().copied().filter(|k| k.category() == cat && !k.applies_to_audio()).collect();
                    if kinds.is_empty() {
                        continue;
                    }
                    menu::sub(ui, None, cat, |ui| {
                        for k in kinds {
                            if menu::row(ui, None, &Effect::new(k).name(), "").clicked() {
                                e.effects.push(Effect::new(k));
                                g.start = true;
                                g.changed = true;
                            }
                        }
                    });
                }
            });
        });
    });

    // the stack editor clips use, on a stand-in adjustment clip at the zone's time (zone-local = clip-local)
    let mut scratch = Project::new();
    scratch.fps = project.fps;
    let mut clip = Clip::new(STAND_IN, ClipKind::Adjustment, z.label(), e.t, e.dur);
    clip.effects = std::mem::take(&mut e.effects);
    let vi = scratch.video_tracks()[0];
    scratch.tracks[vi].clips.push(clip);
    let mut asked = false;
    let resp = crate::ui::effects_ui::stack(ui, &mut scratch, &[STAND_IN], playhead, palette, &mut |_: &Project| asked = true);
    e.effects = scratch.clip(STAND_IN).map(|c| c.effects.clone()).unwrap_or_default();
    if e.effects.is_empty() {
        ui.weak("No effects yet");
    }
    if asked {
        g.start = true;
    }
    g.changed |= resp.edited;

    if remove {
        undo(project);
        project.remove_zone(z.id);
        select(None);
        return true;
    }
    if g.start {
        undo(project);
    }
    if e != z {
        if let Some(dst) = project.zone_mut(z.id) {
            e.t = e.t.max(0.0);
            *dst = e;
        }
        return true;
    }
    false
}
