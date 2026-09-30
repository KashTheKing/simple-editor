//! Transitions panel - the catalogue only. One CARD per `TransitionKind` (same size and frame as an
//! effect card), previewing the transition half-way over the stock picture the effect thumbnails are
//! rendered from - the app hands it over with `set_stock`, and without it a card falls back to a neutral
//! named tile. A card picks the kind, a press-and-move drags `DragPayload::Transition` onto a cut, and its
//! right-click applies it straight away (start / end of every selected clip, or every cut on the track).
//! Above the grid, one row: the default duration (0.1..5 s), plus a colour button for FadeToColor and a
//! direction for Push/Wipe. Every apply goes through `add_transitions`, which is also what the menu, the
//! hotkeys and MCP call - it records the choice in `TransitionsState`, so Ctrl+T
//! (Action::AddLastTransition) repeats whatever was applied last, whichever path applied it. A clip with
//! no neighbour on that side gets an EDGE transition (blend from/to nothing) instead of a cut transition,
//! so lone clips can fade in/out too.
//!
//! A transition already on the timeline is edited where it is selected: the Inspector (kind, duration,
//! colour, direction, ease, remove) and the band's right-click (Change Type / Easing, Remove).

use crate::model::{Id, Project, TransitionKind, ABUT_EPS};
use crate::theme::Palette;
use crate::ui::effects_ui::CARD;
use crate::ui::{menu, DragPayload};
use eframe::egui::{self, pos2, vec2, Color32, DragValue, Rect, Response, Stroke, StrokeKind, Vec2};
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
    /// Catalogue settings for the next transition to add - and the memory of the last one added.
    pub duration: f64,
    pub kind: usize,
    pub color: [u8; 4],
    pub direction: u8,
    // ---- ws:registries-schema-hooks ----
    /// Index of the catalogue card the pointer is hovering, mirroring `EffectsResponse::hover` for a
    /// future async GPU hover preview (ws:inspector-gallery, wave 2). Unread this wave.
    #[allow(dead_code)]
    pub hover: Option<usize>,
}

impl Default for TransitionsState {
    fn default() -> Self {
        Self { duration: 1.0, kind: 0, color: [0, 0, 0, 255], direction: 0, hover: None }
    }
}

impl TransitionsState {
    /// The selected kind - also the one Ctrl+T repeats, because every apply path records here.
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
/// and direction from `st`, and record the choice in `st`. A clip with no neighbour on that side gets
/// an edge transition instead (blend from/to nothing). THE funnel: panel, menu, hotkeys and MCP all
/// come through here (or call `remember`), which is what keeps Ctrl+T on the last transition actually
/// used. Returns how many were added - a transition always belongs to the clip on the RIGHT of the cut,
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
        if project.clip(id).is_none() {
            continue;
        }
        let tid = if at_end {
            match right_neighbor(project, id) {
                Some(right) => project.add_transition(right, kind, dur),
                None => project.add_edge_transition(id, kind, dur, true),
            }
        } else if has_left(project, id) {
            project.add_transition(id, kind, dur)
        } else {
            project.add_edge_transition(id, kind, dur, false)
        };
        if let Some(tid) = tid {
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

/// One catalogue card: the preview tile with the name underneath, selected/hover border. Clicking it
/// selects the kind, right-clicking opens its quick actions; a deliberate press-and-move drags
/// `DragPayload::Transition` (`ui::drag_source`).
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
    let r = crate::ui::drag_source(ui, id, DragPayload::Transition(kind), |ui| {
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
    let tile = Rect::from_min_size(r.rect.min, vec2(CARD.0, CARD.1));
    let border = if selected || r.hovered() { palette.accent } else { palette.border };
    ui.painter().rect_stroke(tile, 2.0, Stroke::new(if selected { 2.0 } else { 1.0 }, border), StrokeKind::Inside);
    r.on_hover_text(format!("{} - click to pick, drag onto a cut", kind.name()))
}

// ---- ws:inspector-gallery ----
/// Authored here (wave 0's registries-schema-hooks did not land it - verified on merged main before
/// writing this, per the plan's own risk note): `EffectsResponse` already carries a real `hover: Option`
/// wave-0 stub, but `transitions_ui::show` returned a plain `bool` until now. `hover` is the catalogue
/// card the pointer has sat on for >=150ms, for `App.alt_render`'s `AltRequest::Transition` preview.
#[derive(Default)]
pub struct TransitionsResponse {
    pub edited: bool,
    pub hover: Option<TransitionKind>,
}

/// Directions a Push / Wipe travels from, in `Transition::direction` order.
const DIRECTIONS: [&str; 4] = ["Left", "Right", "Up", "Down"];

/// The catalogue: one row of defaults for the next transition (duration, plus colour / direction only
/// where the picked kind uses them), then the cards. Everything about a transition already on the
/// timeline lives in the Inspector and the band's right-click (simplify: one home per function).
pub fn show(
    ui: &mut egui::Ui,
    state: &mut TransitionsState,
    project: &mut Project,
    selection: &[Id],
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> TransitionsResponse {
    #[cfg(test)]
    test_rects::clear();
    state.kind = state.kind.min(TransitionKind::ALL.len() - 1);
    let kind = state.kind();
    ui.horizontal(|ui| {
        ui.label("Duration");
        ui.add(DragValue::new(&mut state.duration).range(0.1..=5.0).speed(0.02).suffix(" s"))
            .on_hover_text("Length of the next transition you add");
        if kind == TransitionKind::FadeToColor {
            ui.color_edit_button_srgba_unmultiplied(&mut state.color);
        }
        if kind.has_direction() {
            let cur = DIRECTIONS[(state.direction as usize).min(3)];
            egui::ComboBox::from_id_salt("tr_direction").selected_text(format!("From {cur}")).show_ui(ui, |ui| {
                for (i, name) in DIRECTIONS.iter().enumerate() {
                    ui.selectable_value(&mut state.direction, i as u8, *name);
                }
            });
        }
    });

    // the cuts the selection offers, for the cards' quick-action menus
    let ids: Vec<Id> = selection.iter().copied().filter(|&id| project.clip(id).is_some()).collect();
    let mut hover = None;
    let mut pick = None;
    let mut apply: Option<bool> = None; // Some(at_end)
    let mut every_cut = false;
    egui::ScrollArea::vertical().id_salt("transitions_pane").auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (i, k) in TransitionKind::ALL.into_iter().enumerate() {
                // a card is drawn inside its own scope (`drag_source`), which never wraps the row on its
                // own - break it here, or the last card runs off a narrow pane
                if i > 0 && ui.available_size_before_wrap().x < CARD.0 {
                    ui.end_row();
                }
                let r = transition_card(ui, k, state, palette, state.kind == i);
                #[cfg(test)]
                test_rects::push(format!("card_{}", k.name()), r.rect);
                if crate::ui::hover_after(ui, r.id, &r, 150.0) {
                    hover = Some(k);
                }
                if r.clicked() {
                    pick = Some(i);
                }
                // a quick action also picks the kind, so Ctrl+T repeats what the menu just applied
                r.context_menu(|ui| {
                    let rows = [
                        ("Add at start of selected clip(s)", Some(false)),
                        ("Add at end of selected clip(s)", Some(true)),
                        ("Add at every cut on this track", None),
                    ];
                    for (label, at_end) in rows {
                        let r = ui.add_enabled_ui(!ids.is_empty(), |ui| menu::row(ui, None, label, "")).inner;
                        if r.on_disabled_hover_text("Select a clip first").clicked() {
                            pick = Some(i);
                            match at_end {
                                Some(e) => apply = Some(e),
                                None => every_cut = true,
                            }
                        }
                    }
                });
            }
        });
    });
    if let Some(i) = pick {
        state.kind = i;
    }
    let kind = state.kind();
    let dur = state.duration;
    let mut changed = false;
    if let Some(at_end) = apply {
        undo(project);
        changed |= add_transitions(project, &ids, state, kind, dur, at_end) > 0;
    }
    if let (true, Some(&sel)) = (every_cut, ids.first()) {
        // every clip on the track that has something abutting its left edge is a cut
        let ti = project.track_of(sel);
        let clips: Vec<Id> = ti.map(|ti| project.tracks[ti].clips.iter().map(|c| c.id).collect()).unwrap_or_default();
        let cuts: Vec<Id> = clips.into_iter().filter(|&id| has_left(project, id)).collect();
        if !cuts.is_empty() {
            undo(project);
            changed |= add_transitions(project, &cuts, state, kind, dur, false) > 0;
        }
    }
    TransitionsResponse { edited: changed, hover }
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
        shapes: Vec<egui::epaint::ClippedShape>,
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
                rel_path: None,
                parent: None,
                range: None,
                effects: Vec::new(),
            });
            project.insert_asset_clips(aid, 0.0, Some(0));
            project.insert_asset_clips(aid, 10.0, Some(0));
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::theme::test_fonts()); // size-diet: no default_fonts feature anymore
            Self {
                ctx,
                state: TransitionsState::default(),
                project,
                selection: Vec::new(),
                undos: 0,
                time: 0.0,
                shapes: Vec::new(),
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
            let Harness { ctx, state, project, selection, undos, shapes, .. } = self;
            let mut changed = false;
            let full = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    changed |= show(ui, state, project, selection, &pal, &mut undo).edited;
                });
            });
            *shapes = full.shapes;
            changed
        }
        /// Centre of the first painted text containing `label` - how a popup's entries are found.
        fn text_at(&self, label: &str) -> Option<Pos2> {
            self.shapes.iter().find_map(|c| match &c.shape {
                egui::epaint::Shape::Text(t) if t.galley.text().contains(label) => {
                    Some(t.visual_bounding_rect().center())
                }
                _ => None,
            })
        }
        fn button(&mut self, pos: Pos2, button: PointerButton, pressed: bool) -> bool {
            self.frame(vec![Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE }])
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
        /// Right-click the `card` and click its `label` entry; true when that edited the project.
        fn menu_pick(&mut self, card: &str, label: &str) -> bool {
            self.frame(vec![]);
            let at = test_rects::get(&format!("card_{card}")).expect("card recorded").center();
            self.frame(vec![Event::PointerMoved(at)]);
            self.button(at, PointerButton::Secondary, true);
            self.button(at, PointerButton::Secondary, false);
            self.frame(vec![]);
            let item = self.text_at(label).unwrap_or_else(|| panic!("no '{label}' in the card menu"));
            self.time += 1.0; // a separate gesture, not a double-click
            self.click(item)
        }
    }

    #[test]
    fn add_at_start_creates_transition_and_audio_mirror() {
        let mut h = Harness::new();
        // second video clip (starts at 10) - its cut with the first is at its start
        let v2 = h.project.tracks[0].clips[1].id;
        let a2 = h.project.tracks[1].clips[1].id;
        h.selection = vec![v2];
        assert!(h.menu_pick("Cross Fade", "Add at start"));
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
    fn add_at_end_uses_right_neighbor() {
        let mut h = Harness::new();
        let v1 = h.project.tracks[0].clips[0].id;
        let v2 = h.project.tracks[0].clips[1].id;
        h.selection = vec![v1];
        assert!(h.menu_pick("Wipe", "Add at end"));
        assert_eq!(h.project.tracks[0].transitions.len(), 1);
        assert_eq!(h.project.tracks[0].transitions[0].right, v2, "end of v1 = cut whose right side is v2");
        assert_eq!(h.project.tracks[0].transitions[0].kind, TransitionKind::Wipe, "the card that was right-clicked");
        assert_eq!(h.state.kind(), TransitionKind::Wipe, "the menu picks the kind it applied");
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn no_neighbor_adds_an_edge_transition() {
        let mut h = Harness::new();
        // select the FIRST clip: nothing ends at its start (t = 0), so it blends in from nothing
        let v1 = h.project.tracks[0].clips[0].id;
        h.selection = vec![v1];
        assert!(h.menu_pick("Cross Fade", "Add at start"));
        assert_eq!(h.project.tracks[0].transitions.len(), 1);
        let tr = &h.project.tracks[0].transitions[0];
        assert_eq!((tr.right, tr.edge), (v1, crate::model::TransitionEdge::In));
        assert_eq!(h.undos, 1);
    }

    /// With nothing selected the card's quick actions are greyed out: a click applies nothing.
    #[test]
    fn card_menu_needs_a_selection() {
        let mut h = Harness::new();
        assert!(!h.menu_pick("Push", "Add at start"));
        assert!(h.project.tracks.iter().all(|t| t.transitions.is_empty()));
        assert_eq!(h.undos, 0);
    }

    /// The last clip of the track gets an Out edge from "Add at end" (nothing abuts it).
    #[test]
    fn add_at_end_without_neighbor_fades_out() {
        let mut h = Harness::new();
        let v2 = h.project.tracks[0].clips[1].id;
        h.selection = vec![v2];
        assert_eq!(add_transitions(&mut h.project, &[v2], &mut h.state, TransitionKind::CrossFade, 1.0, true), 1);
        let tr = &h.project.tracks[0].transitions[0];
        assert_eq!((tr.right, tr.edge), (v2, crate::model::TransitionEdge::Out));
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
        assert!(h.menu_pick("Cross Fade", "Add at start"));
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

    /// The pane lists no per-transition editors any more (Inspector's job), so drawing it next to a
    /// transition whose duration is out of range can never fake an edit.
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

    /// A card is clickable first and a drag source second: a stationary press - even one held long
    /// past egui's click timeout - arms nothing, and only real pointer travel hands the payload over.
    #[test]
    fn a_card_only_drags_once_the_pointer_moves() {
        let mut h = Harness::new();
        h.selection = vec![h.project.tracks[0].clips[1].id];
        h.frame(vec![]);
        let card = test_rects::get("card_Wipe").expect("card recorded").center();
        h.frame(vec![Event::PointerMoved(card)]);
        h.button(card, PointerButton::Primary, true);
        for _ in 0..20 {
            h.frame(vec![]);
        }
        assert!(!egui::DragAndDrop::has_any_payload(&h.ctx), "holding still is not a drag");
        h.frame(vec![Event::PointerMoved(card + vec2(24.0, 0.0))]);
        let p = egui::DragAndDrop::payload::<DragPayload>(&h.ctx).expect("moving must arm the payload");
        assert!(matches!(*p, DragPayload::Transition(TransitionKind::Wipe)), "the card under the press");
    }

    /// Right-clicking a card applies it instead of dragging it: the menu's "every cut" entry fills the
    /// whole track in one undo.
    #[test]
    fn card_right_click_menu_fills_every_cut() {
        let mut h = Harness::new();
        let aid = h.project.assets[0].id;
        h.project.insert_asset_clips(aid, 20.0, Some(0)); // cuts at 10 and 20
        h.selection = vec![h.project.tracks[0].clips[0].id];
        h.frame(vec![]);
        let card = test_rects::get("card_Push").expect("card recorded").center();
        h.frame(vec![Event::PointerMoved(card)]);
        h.button(card, PointerButton::Secondary, true);
        h.frame(vec![]);
        assert!(!egui::DragAndDrop::has_any_payload(&h.ctx), "the secondary button must never drag a card");
        h.button(card, PointerButton::Secondary, false);
        h.frame(vec![]);
        let item = h.text_at("Add at every cut on this track").expect("quick action menu");
        assert!(h.click(item));
        assert_eq!(h.project.tracks[0].transitions.len(), 2, "both cuts of the track");
        assert!(h.project.tracks[0].transitions.iter().all(|t| t.kind == TransitionKind::Push));
        assert_eq!(h.state.kind(), TransitionKind::Push, "the menu picks the kind it applied");
        assert_eq!(h.undos, 1);
    }

    /// A pane narrower than the four cards wraps them onto a second row instead of clipping the last.
    #[test]
    fn cards_wrap_in_a_narrow_pane() {
        let mut h = Harness::new();
        let pal = Palette::new(true, Color32::WHITE);
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(3.5 * CARD.0, 400.0))),
            ..Default::default()
        };
        let Harness { ctx, state, project, .. } = &mut h;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, state, project, &[], &pal, &mut |_| {});
            });
        });
        let first = test_rects::get("card_Cross Fade").expect("card recorded");
        let last = test_rects::get("card_Wipe").expect("card recorded");
        assert!(last.right() <= 3.5 * CARD.0, "the last card fits the pane: {last:?}");
        assert!(last.top() > first.bottom() - 1.0, "... on a row of its own: {last:?} vs {first:?}");
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
