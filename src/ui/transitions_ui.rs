//! Transitions panel. Catalogue: one CARD per `TransitionKind` (same size and frame as an effect card),
//! previewing the transition half-way over the stock picture the effect thumbnails are rendered from —
//! the app hands it over with `set_stock`, and without it a card falls back to a neutral named tile.
//! A card selects the kind, and is a drag source emitting `DragPayload::Transition`. Next to the grid the
//! default duration DragValue (0.1..5 s), a colour button for FadeToColor and a direction selector for
//! Push/Wipe. "Add at start" / "Add at end" apply it to EVERY selected clip through `add_transitions`,
//! which is also what the menu, the hotkeys and MCP call — it records the choice in `TransitionsState`,
//! so Ctrl+T (Action::AddLastTransition) repeats whatever was applied last, whichever path applied it.
//! Below: the transitions touching the first selected clip (Project::transitions_of): kind combo,
//! duration, colour/direction/ease editors, ✕ remove. Returns true when the project changed (call
//! `undo` once per gesture first).

use crate::model::{Ease, Id, Project, Transition, TransitionKind, ABUT_EPS};
use crate::theme::Palette;
use crate::ui::effects_ui::CARD;
use crate::ui::{DragPayload, Gesture};
use eframe::egui::{self, pos2, vec2, Button, Color32, DragValue, Rect, Response, Stroke, StrokeKind, Vec2};
use std::cell::RefCell;

#[cfg(test)]
use crate::ui::effects_ui::test_rects;

thread_local! {
    /// The catalogue's stock picture, uploaded by the app from the same source as the effect thumbnails.
    /// The handle lives here so the texture outlives the app's own thumbnail list.
    static STOCK: RefCell<Option<egui::TextureHandle>> = const { RefCell::new(None) };
}

/// The app hands over the picture the effect catalogue renders from; the cards preview over it.
pub fn set_stock(tex: egui::TextureHandle) {
    STOCK.with(|s| *s.borrow_mut() = Some(tex));
}

pub struct TransitionsState {
    /// Catalogue settings for the next transition to add — and the memory of the last one added.
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

impl TransitionsState {
    /// The selected kind — also the one Ctrl+T repeats, because every apply path records here.
    pub fn kind(&self) -> TransitionKind {
        TransitionKind::ALL[self.kind.min(TransitionKind::ALL.len() - 1)]
    }
    /// Remember what was just applied (from any path) so Ctrl+T is never stale.
    pub(crate) fn remember(&mut self, kind: TransitionKind, dur: f64) {
        self.kind = TransitionKind::ALL.iter().position(|&k| k == kind).unwrap_or(0);
        self.duration = dur;
    }
}

/// The clip starting exactly where `id` ends, on the same track.
pub(crate) fn right_neighbor(project: &Project, id: Id) -> Option<Id> {
    let ti = project.track_of(id)?;
    let c = project.clip(id)?;
    project.tracks[ti].clips.iter().find(|o| o.id != id && (o.start - c.end()).abs() < ABUT_EPS).map(|o| o.id)
}

/// A clip abuts the left edge of `id`, i.e. there is a cut to put a transition on.
fn has_left(project: &Project, id: Id) -> bool {
    let Some(ti) = project.track_of(id) else { return false };
    project.clip(id).is_some_and(|c| project.tracks[ti].left_of(c).is_some())
}

/// Add a transition at the cut left of every id (or at its right edge with `at_end`), with the colour
/// and direction from `st`, and record the choice in `st`. THE funnel: panel, menu, hotkeys and MCP all
/// come through here (or call `remember`), which is what keeps Ctrl+T on the last transition actually
/// used. Returns how many were added — a transition always belongs to the clip on the RIGHT of the cut,
/// and `Project::add_transition` replaces the one already on that cut, so overlapping selections are fine.
pub(crate) fn add_transitions(
    project: &mut Project,
    ids: &[Id],
    st: &mut TransitionsState,
    kind: TransitionKind,
    dur: f64,
    at_end: bool,
) -> usize {
    st.remember(kind, dur);
    let mut added = 0;
    for &id in ids {
        let right = if at_end { right_neighbor(project, id) } else { project.clip(id).map(|c| c.id) };
        let Some(right) = right else { continue };
        if let Some(tid) = project.add_transition(right, kind, dur) {
            if let Some(t) = project.transition_mut(tid) {
                t.color = st.color;
                t.direction = st.direction;
            }
            added += 1;
        }
    }
    added
}

/// A transition card dropped on a clip at timeline time `t`: which of the clip's two cuts it means.
/// The half you drop on picks it, so the gesture reads the same as "Add at start" / "Add at end".
/// Shared so the timeline's drop highlight and the app's drop handler cannot drift apart.
pub(crate) fn drop_at_end(clip: &crate::model::Clip, t: f64) -> bool {
    t > clip.start + clip.duration / 2.0
}

/// Half-way through the transition: enough to show the wipe/push/fade on a still card.
const PREVIEW_P: f32 = 0.5;

/// Where the incoming clip travels from, mirroring `compose::dir_vec` (0 left, 1 right, 2 up, 3 down).
fn dir_vec(direction: u8, size: Vec2) -> Vec2 {
    match direction {
        0 => vec2(-size.x, 0.0),
        1 => vec2(size.x, 0.0),
        2 => vec2(0.0, -size.y),
        _ => vec2(0.0, size.y),
    }
}

/// The part of the tile the incoming clip has reached at `p` (same regions as `compose`'s wipe).
fn wipe_rect(tile: Rect, direction: u8, p: f32) -> Rect {
    let (w, h) = (tile.width() * p, tile.height() * p);
    match direction {
        0 => Rect::from_min_max(tile.min, pos2(tile.left() + w, tile.bottom())),
        1 => Rect::from_min_max(pos2(tile.right() - w, tile.top()), tile.max),
        2 => Rect::from_min_max(tile.min, pos2(tile.right(), tile.top() + h)),
        _ => Rect::from_min_max(pos2(tile.left(), tile.bottom() - h), tile.max),
    }
}

/// Draw the transition over the stock picture: A is the picture, B the same picture tinted, so the two
/// sides of the cut read apart without decoding a second image.
fn paint_preview(
    p: &egui::Painter,
    tile: Rect,
    tex: egui::TextureId,
    kind: TransitionKind,
    st: &TransitionsState,
    palette: &Palette,
) {
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let (a, b) = (Color32::WHITE, palette.accent);
    match kind {
        TransitionKind::CrossFade => {
            p.image(tex, tile, uv, a);
            p.image(tex, tile, uv, b.gamma_multiply(0.5));
        }
        TransitionKind::FadeToColor => {
            p.image(tex, tile, uv, a);
            let c = st.color;
            p.rect_filled(tile, 2.0, Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 190));
        }
        TransitionKind::Push => {
            let d = dir_vec(st.direction, tile.size());
            let p = p.with_clip_rect(tile);
            p.image(tex, tile.translate(-d * PREVIEW_P), uv, a);
            p.image(tex, tile.translate(d * (1.0 - PREVIEW_P)), uv, b);
        }
        TransitionKind::Wipe => {
            p.image(tex, tile, uv, a);
            p.with_clip_rect(wipe_rect(tile, st.direction, PREVIEW_P)).image(tex, tile, uv, b);
        }
    }
}

/// One catalogue card: the preview tile with the name underneath, selected/hover border. Dragging it
/// carries `DragPayload::Transition`; clicking it selects the kind.
fn transition_card(
    ui: &mut egui::Ui,
    kind: TransitionKind,
    st: &TransitionsState,
    palette: &Palette,
    selected: bool,
) -> Response {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let name_h = ui.text_style_height(&egui::TextStyle::Small);
    let id = ui.id().with(("tr_card", kind.name()));
    let src = ui.dnd_drag_source(id, DragPayload::Transition(kind), |ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(CARD.0, CARD.1 + name_h + 2.0), egui::Sense::hover());
        let tile = Rect::from_min_size(rect.min, vec2(CARD.0, CARD.1));
        let p = ui.painter();
        match STOCK.with(|s| s.borrow().as_ref().map(|t| t.id())) {
            Some(tex) => paint_preview(p, tile, tex, kind, st, palette),
            None => {
                // no GPU thumbnails yet: a neutral tile that still names the transition
                p.rect_filled(tile, 2.0, palette.header);
                let g = p.layout(kind.name().to_string(), font.clone(), palette.text_dim, CARD.0 - 8.0);
                p.galley(tile.center() - g.size() / 2.0, g, palette.text_dim);
            }
        }
        let label = p.layout_no_wrap(kind.name().to_string(), font, palette.text);
        let lx = (rect.left() + (CARD.0 - label.size().x) / 2.0).max(rect.left());
        p.with_clip_rect(Rect::from_min_max(pos2(rect.left(), tile.bottom()), rect.max)).galley(
            pos2(lx, tile.bottom() + 2.0),
            label,
            palette.text,
        );
    });
    let r = ui.interact(src.response.rect, id.with("click"), egui::Sense::click());
    let tile = Rect::from_min_size(src.response.rect.min, vec2(CARD.0, CARD.1));
    let border = if selected || r.hovered() { palette.accent } else { palette.border };
    ui.painter().rect_stroke(tile, 2.0, Stroke::new(if selected { 2.0 } else { 1.0 }, border), StrokeKind::Inside);
    r.on_hover_text(format!("{} — click to pick, drag onto a cut", kind.name()))
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
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    #[cfg(test)]
    test_rects::clear();
    let mut changed = false;
    let mut g = Gesture::default();

    // ---- catalogue / defaults for the next transition ----
    ui.strong("Transitions");
    state.kind = state.kind.min(TransitionKind::ALL.len() - 1);
    ui.horizontal_wrapped(|ui| {
        for (i, k) in TransitionKind::ALL.into_iter().enumerate() {
            let r = transition_card(ui, k, state, palette, state.kind == i);
            #[cfg(test)]
            test_rects::push(format!("card_{}", k.name()), r.rect);
            if r.clicked() {
                state.kind = i;
            }
        }
    });
    let kind = state.kind();
    ui.horizontal(|ui| {
        ui.label("Duration");
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

    // ---- add at the cuts around every selected clip ----
    let ids: Vec<Id> = selection.iter().copied().filter(|&id| project.clip(id).is_some()).collect();
    let Some(&sel) = ids.first() else {
        ui.label("Select a clip");
        return false;
    };
    let any_left = ids.iter().any(|&id| has_left(project, id));
    let any_right = ids.iter().any(|&id| right_neighbor(project, id).is_some());
    let mut apply: Option<bool> = None; // Some(at_end)
    ui.horizontal(|ui| {
        let r = ui
            .add_enabled(any_left, Button::new("Add at start"))
            .on_hover_text("Transition into every selected clip")
            .on_disabled_hover_text("No clip ends where the selected one starts");
        #[cfg(test)]
        test_rects::push("add_start".into(), r.rect);
        if r.clicked() {
            apply = Some(false);
        }
        let r = ui
            .add_enabled(any_right, Button::new("Add at end"))
            .on_hover_text("Transition out of every selected clip")
            .on_disabled_hover_text("No clip starts where the selected one ends");
        #[cfg(test)]
        test_rects::push("add_end".into(), r.rect);
        if r.clicked() {
            apply = Some(true);
        }
    });
    if let Some(at_end) = apply {
        undo(project);
        let dur = state.duration;
        changed |= add_transitions(project, &ids, state, kind, dur, at_end) > 0;
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
    use crate::model::{Asset, AudioStreamInfo, ClipKind};
    use eframe::egui::{Event, Modifiers, PointerButton, Pos2, RawInput};

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

    /// Every selected clip gets a transition, not just the first one, and it is one undo entry.
    #[test]
    fn add_covers_every_selected_clip_with_one_undo() {
        let mut h = Harness::new();
        let aid = h.project.assets[0].id;
        h.project.insert_asset_clips(aid, 20.0, Some(0)); // three abutting pairs: cuts at 10 and 20
        let v2 = h.project.tracks[0].clips[1].id;
        let v3 = h.project.tracks[0].clips[2].id;
        h.selection = vec![v2, v3];
        h.frame(vec![]);
        let r = test_rects::get("add_start").expect("add button recorded");
        assert!(h.click(r.center()));
        let rights: Vec<Id> = h.project.tracks[0].transitions.iter().map(|t| t.right).collect();
        assert_eq!(rights.len(), 2, "both cuts of the selection: {rights:?}");
        assert!(rights.contains(&v2) && rights.contains(&v3));
        assert_eq!(h.undos, 1, "one undo for the whole selection");
    }

    /// Applying through the shared funnel records the choice, colour and direction included.
    #[test]
    fn apply_records_the_last_used_transition() {
        let mut h = Harness::new();
        let v2 = h.project.tracks[0].clips[1].id;
        h.state.color = [1, 2, 3, 255];
        h.state.direction = 2;
        assert_eq!(add_transitions(&mut h.project, &[v2], &mut h.state, TransitionKind::Wipe, 0.4, false), 1);
        assert_eq!(h.state.kind(), TransitionKind::Wipe, "Ctrl+T would repeat what was just applied");
        assert!((h.state.duration - 0.4).abs() < 1e-9);
        let tr = h.project.tracks[0].transitions.iter().find(|t| t.right == v2).expect("transition");
        assert_eq!((tr.kind, tr.direction, tr.color), (TransitionKind::Wipe, 2, [1, 2, 3, 255]));
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

    /// Dropping a card on a clip picks the near cut, and applying it there is exactly the button's job.
    #[test]
    fn a_dropped_card_picks_the_cut_it_landed_next_to() {
        let mut h = Harness::new();
        let v2 = h.project.tracks[0].clips[1].id; // 10..20
        let clip = h.project.clip(v2).unwrap().clone();
        assert!(!drop_at_end(&clip, 11.0), "left half = the cut at its start");
        assert!(drop_at_end(&clip, 19.0), "right half = the cut at its end");
        let st = &mut h.state;
        assert_eq!(add_transitions(&mut h.project, &[v2], st, TransitionKind::Push, 0.5, false), 1);
        assert_eq!(h.project.tracks[0].transitions[0].right, v2);
    }

    #[test]
    fn right_neighbor_finds_abutting_clip() {
        let h = Harness::new();
        let v1 = h.project.tracks[0].clips[0].id;
        let v2 = h.project.tracks[0].clips[1].id;
        assert_eq!(right_neighbor(&h.project, v1), Some(v2));
        assert_eq!(right_neighbor(&h.project, v2), None);
    }

    /// The card grid is drawn (and clickable) with no stock picture, and a click picks the kind.
    #[test]
    fn cards_pick_the_kind_without_a_stock_picture() {
        let mut h = Harness::new();
        h.selection = vec![h.project.tracks[0].clips[1].id];
        h.frame(vec![]);
        let r = test_rects::get("card_Wipe").expect("card recorded");
        assert!(r.width() >= CARD.0 - 1.0 && r.height() >= CARD.1, "card is card-sized: {r:?}");
        h.click(r.center());
        assert_eq!(h.state.kind(), TransitionKind::Wipe);
        assert_eq!(h.undos, 0, "picking a kind is not an edit");
    }

    /// The preview geometry follows the direction the same way the compositor does.
    #[test]
    fn wipe_region_follows_the_direction() {
        let tile = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 50.0));
        assert_eq!(wipe_rect(tile, 0, 0.5), Rect::from_min_max(pos2(0.0, 0.0), pos2(50.0, 50.0)));
        assert_eq!(wipe_rect(tile, 1, 0.5), Rect::from_min_max(pos2(50.0, 0.0), pos2(100.0, 50.0)));
        assert_eq!(wipe_rect(tile, 2, 0.5), Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 25.0)));
        assert_eq!(wipe_rect(tile, 3, 0.5), Rect::from_min_max(pos2(0.0, 25.0), pos2(100.0, 50.0)));
        // the incoming clip travels from the named edge (compose::dir_vec)
        assert_eq!(dir_vec(0, vec2(100.0, 50.0)), vec2(-100.0, 0.0));
        assert_eq!(dir_vec(3, vec2(100.0, 50.0)), vec2(0.0, 50.0));
    }
}
