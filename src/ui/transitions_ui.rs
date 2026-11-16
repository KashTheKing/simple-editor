//! Transitions: the state, apply helpers and preview painting behind the Gallery's Transitions tab
//! (`ui::gallery`; the old Transitions pane is gone). The tab shows one card per `TransitionKind`
//! previewed half-way over the stock picture the effect thumbnails are rendered from (the app hands it
//! over with `set_stock`; without it a card is a neutral named tile), then the installed / online library
//! transitions. A card click applies it at the start of the selection, a press-and-move drags it onto a
//! cut, and its right-click offers Apply to Start / End / Connection / All (`Where`). Above the grid,
//! `defaults_row`: the default duration (0.1..5 s), plus a colour button for FadeToColor and a
//! direction for Push/Wipe. Every apply goes through `add_transitions` / `apply_where`, which is also
//! what the hotkeys and MCP call - they record the choice in `TransitionsState`, so Ctrl+T
//! (Action::AddLastTransition) repeats whatever was applied last, whichever path applied it. A clip with
//! no neighbour on that side gets an EDGE transition (blend from/to nothing) instead of a cut transition,
//! so lone clips can fade in/out too.
//!
//! ws:shader-library: after the four built-ins, one card per installed library transition
//! (`library::installed(Kind::Transition)`, listed by `library_items`: read once, re-read only when the
//! folder's mtime changes). Picking one makes the next transition a `TransitionKind::Shader` running
//! that source (`TransitionsState::shader`); its named controls replace colour / direction in the row.
//!
//! A transition already on the timeline is edited where it is selected: the Inspector (kind, duration,
//! colour, direction, ease, remove) and the band's right-click (Change Type / Easing, Remove).

use crate::model::{Id, Project, TransitionKind, ABUT_EPS};
use crate::theme::Palette;
use eframe::egui::{self, pos2, vec2, Color32, DragValue, Rect, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

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
    // ---- ws:shader-library ----
    /// The library transition picked (`kind == ALL.len()`): its source, library id and control values.
    pub shader: String,
    pub shader_id: String,
    pub params: Vec<f64>,
}

impl Default for TransitionsState {
    fn default() -> Self {
        Self {
            duration: 1.0,
            kind: 0,
            color: [0, 0, 0, 255],
            direction: 0,
            hover: None,
            shader: String::new(),
            shader_id: String::new(),
            params: Vec::new(),
        }
    }
}

/// `TransitionsState::kind` of "the picked library transition" (after the built-ins).
const SHADER_SLOT: usize = TransitionKind::ALL.len();

impl TransitionsState {
    /// The selected kind - also the one Ctrl+T repeats, because every apply path records here.
    pub fn kind(&self) -> TransitionKind {
        if self.kind >= SHADER_SLOT && !self.shader.is_empty() {
            return TransitionKind::Shader;
        }
        TransitionKind::ALL[self.kind.min(TransitionKind::ALL.len() - 1)]
    }
    /// Remember what was just applied (from any path) so Ctrl+T is never stale.
    pub(crate) fn remember(&mut self, kind: TransitionKind, dur: f64) {
        self.kind = TransitionKind::ALL.iter().position(|&k| k == kind).unwrap_or(SHADER_SLOT);
        self.duration = dur;
    }
    /// Pick a library transition for the next apply (its controls start at the header defaults).
    pub(crate) fn pick_shader(&mut self, id: &str, src: &str) {
        if self.shader != src {
            self.params = crate::engine::shaderlib::parse_meta(src).params.iter().map(|p| p.default).collect();
        }
        self.kind = SHADER_SLOT;
        self.shader = src.to_string();
        self.shader_id = id.to_string();
    }
}

/// One installed library shader (effect or transition) as the catalogues list it.
pub(crate) struct LibItem {
    pub id: String,
    pub name: String,
    pub src: Rc<str>,
}

struct LibList {
    mtime: Option<std::time::SystemTime>,
    /// `ui.input().time` of the last mtime check.
    checked: f64,
    items: Rc<Vec<LibItem>>,
}

thread_local! {
    static LIB: RefCell<std::collections::HashMap<crate::engine::library::Kind, LibList>> = RefCell::default();
}

/// Installed library shaders of `kind`, sorted by name. Read from disk on first use and again only when
/// the folder's mtime changed - checked at most every 2 s of `now` (UI time), so a frame never waits on
/// the disk for more than one `metadata` call. `refresh_library` forces a re-read (after a save).
pub(crate) fn library_items(kind: crate::engine::library::Kind, now: f64) -> Rc<Vec<LibItem>> {
    let dir = crate::engine::library::root().join(kind.dir());
    LIB.with(|l| {
        let mut l = l.borrow_mut();
        if let Some(e) = l.get_mut(&kind) {
            if now - e.checked < 2.0 && now >= e.checked {
                return e.items.clone();
            }
            e.checked = now;
            let m = std::fs::metadata(&dir).and_then(|m| m.modified()).ok();
            if m == e.mtime {
                return e.items.clone();
            }
        }
        let mtime = std::fs::metadata(&dir).and_then(|m| m.modified()).ok();
        let mut items: Vec<LibItem> = crate::engine::library::installed(kind)
            .into_iter()
            .filter_map(|it| {
                let src = std::fs::read_to_string(&it.path).ok()?;
                let m = crate::engine::shaderlib::parse_meta(&src);
                let name = if m.name.is_empty() { it.id.clone() } else { m.name };
                Some(LibItem { id: it.id, name, src: src.into() })
            })
            .collect();
        items.sort_by(|a, b| a.name.cmp(&b.name));
        let items = Rc::new(items);
        l.insert(kind, LibList { mtime, checked: now, items: items.clone() });
        items
    })
}

/// Forget the cached library listings (something was just saved or installed).
pub(crate) fn refresh_library() {
    LIB.with(|l| l.borrow_mut().clear());
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
            style(project, st, tid, kind);
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
pub(crate) fn paint_preview(
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
        // ws:shader-library: a library card (no GPU pass here) reads as a dissolve
        TransitionKind::CrossFade | TransitionKind::Shader => {
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

/// Directions a Push / Wipe travels from, in `Transition::direction` order.
const DIRECTIONS: [&str; 4] = ["Left", "Right", "Up", "Down"];

/// The shortest transition the Duration control offers (0.1 s).
pub(crate) const MIN_TRANSITION: f64 = 0.1;

/// The picture a card previews over (None until the app has rendered the effect thumbnails).
pub(crate) fn stock_id() -> Option<egui::TextureId> {
    STOCK.with(|s| s.borrow().as_ref().map(|t| t.id()))
}

/// The row of defaults for the next transition: its length (0.1..5 s), plus a colour button for
/// FadeToColor, a direction for Push / Wipe, or a library shader's named controls. The Gallery's
/// Transitions tab draws it above the cards.
pub(crate) fn defaults_row(ui: &mut egui::Ui, state: &mut TransitionsState) {
    state.kind = state.kind.min(SHADER_SLOT);
    let kind = state.kind();
    ui.horizontal_wrapped(|ui| {
        ui.label("Duration");
        ui.add(DragValue::new(&mut state.duration).range(0.1..=5.0).speed(0.02).suffix(" s"))
            .on_hover_text("Length of the next transition you add");
        if kind == TransitionKind::FadeToColor {
            ui.color_edit_button_srgba_unmultiplied(&mut state.color);
        }
        if kind == TransitionKind::Shader {
            ui.weak(crate::engine::shaderlib::parse_meta(&state.shader).name);
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
    // a picked library transition's named controls (for the next one added)
    if kind == TransitionKind::Shader {
        let specs = crate::model::shader_specs(&state.shader);
        if !specs.is_empty() {
            let mut g = crate::ui::Gesture::default();
            egui::Grid::new("tr_shader_params")
                .num_columns(2)
                .show(ui, |ui| crate::ui::effects_ui::plain_param_rows(ui, &specs, &mut state.params, &mut g));
        }
    }
}

// ---- gallery: where a transition lands ----
/// Which edge(s) of the target clip(s) a Gallery transition card's menu row / `gallery.apply where`
/// aims at. A transition belongs to the clip on the RIGHT of a cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Where {
    /// The clip's start edge as an EDGE transition (blends in from nothing, even when a clip abuts).
    Start,
    /// The clip's end edge, same.
    End,
    /// The two-sided transition on the cut at the clip's start (only where a clip abuts there).
    ConnStart,
    /// The two-sided transition on the cut at the clip's end.
    ConnEnd,
    /// Every connected edge of each clip (start and/or end) - the MCP spelling of "connection".
    Connection,
    /// The beginning of each clip: the cut when a clip abuts its start, else a fade-in edge. This is what a
    /// plain click does, and what "Apply to All" means with two or more clips selected.
    All,
}

impl Where {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Where::Start => "start",
            Where::End => "end",
            Where::ConnStart => "connection_start",
            Where::ConnEnd => "connection_end",
            Where::Connection => "connection",
            Where::All => "all",
        }
    }
    pub(crate) fn from_name(s: &str) -> Option<Where> {
        [Where::Start, Where::End, Where::ConnStart, Where::ConnEnd, Where::Connection, Where::All]
            .into_iter()
            .find(|w| w.name().eq_ignore_ascii_case(s))
    }
}

/// One place a transition goes: an edge transition on a clip, or the transition on the cut left of `right`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Spot {
    Edge { clip: Id, out: bool },
    Cut { right: Id },
}

/// The spots `where_` means for `ids` (clips that exist; each spot once, in selection order).
pub(crate) fn spots(project: &Project, ids: &[Id], where_: Where) -> Vec<Spot> {
    let mut out: Vec<Spot> = Vec::new();
    let mut push = |s: Spot| {
        if !out.contains(&s) {
            out.push(s);
        }
    };
    for &id in ids.iter().filter(|&&id| project.clip(id).is_some()) {
        let start = has_left(project, id);
        let end = right_neighbor(project, id);
        match where_ {
            Where::Start => push(Spot::Edge { clip: id, out: false }),
            Where::End => push(Spot::Edge { clip: id, out: true }),
            Where::ConnStart => {
                if start {
                    push(Spot::Cut { right: id });
                }
            }
            Where::ConnEnd => {
                if let Some(r) = end {
                    push(Spot::Cut { right: r });
                }
            }
            Where::Connection => {
                if start {
                    push(Spot::Cut { right: id });
                }
                if let Some(r) = end {
                    push(Spot::Cut { right: r });
                }
            }
            Where::All => push(if start { Spot::Cut { right: id } } else { Spot::Edge { clip: id, out: false } }),
        }
    }
    out
}

/// `want` seconds, clamped so a transition never exceeds half of the shorter clip it touches (the clip
/// itself for an edge transition); below `MIN_TRANSITION` the spot cannot take one (`None`).
pub(crate) fn clamped(project: &Project, spot: Spot, want: f64) -> Option<f64> {
    let shorter = match spot {
        Spot::Edge { clip, .. } => project.clip(clip)?.duration,
        Spot::Cut { right } => {
            let ti = project.track_of(right)?;
            let r = project.clip(right)?;
            let left = project.tracks[ti].left_of(r)?;
            r.duration.min(left.duration)
        }
    };
    let d = want.min(shorter / 2.0);
    (d >= MIN_TRANSITION - 1e-9).then_some(d)
}

/// A transition already sits on `spot`.
pub(crate) fn has_transition(project: &Project, spot: Spot) -> bool {
    use crate::model::TransitionEdge;
    let (right, edge) = match spot {
        Spot::Edge { clip, out } => (clip, if out { TransitionEdge::Out } else { TransitionEdge::In }),
        Spot::Cut { right } => (right, TransitionEdge::Cut),
    };
    project
        .track_of(right)
        .is_some_and(|ti| project.tracks[ti].transitions.iter().any(|t| t.right == right && t.edge == edge))
}

/// Put `kind` on every spot `where_` means for `ids`, with the colour / direction / shader of `st`
/// (recorded there for Ctrl+T), `dur` clamped per spot. `skip_existing` leaves a spot that already has
/// a transition alone (the default replaces it). Returns how many were added.
pub(crate) fn apply_where(
    project: &mut Project,
    st: &mut TransitionsState,
    ids: &[Id],
    kind: TransitionKind,
    dur: f64,
    where_: Where,
    skip_existing: bool,
) -> usize {
    st.remember(kind, dur);
    let mut added = 0;
    for spot in spots(project, ids, where_) {
        if skip_existing && has_transition(project, spot) {
            continue;
        }
        let Some(d) = clamped(project, spot, dur) else { continue };
        let tid = match spot {
            Spot::Edge { clip, out } => project.add_edge_transition(clip, kind, d, out),
            Spot::Cut { right } => project.add_transition(right, kind, d),
        };
        if let Some(tid) = tid {
            style(project, st, tid, kind);
            added += 1;
        }
    }
    added
}

/// Colour, direction and (for a library shader) source + controls of the picked transition.
fn style(project: &mut Project, st: &TransitionsState, tid: Id, kind: TransitionKind) {
    if let Some(t) = project.transition_mut(tid) {
        t.color = st.color;
        t.direction = st.direction;
        if kind == TransitionKind::Shader {
            // ws:shader-library: the picked library source and its control values
            t.params.clone_from(&st.params);
            t.set_shader(&st.shader); // also trims / pads the values to the header
        }
    }
}

/// What a transition card's right-click menu offers for the current selection.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Offer {
    /// Why "Apply to Start" / "Apply to End" are greyed out (None = they work).
    pub start: Option<&'static str>,
    pub end: Option<&'static str>,
    /// A selected clip's start / end touches another clip on its track: the connection rows exist.
    pub conn_start: bool,
    pub conn_end: bool,
    /// Two or more clips are selected: "Apply to All" exists.
    pub all: bool,
}

pub(crate) fn offer(project: &Project, selection: &[Id], dur: f64) -> Offer {
    let ids: Vec<Id> = selection.iter().copied().filter(|&id| project.clip(id).is_some()).collect();
    let why = |w: Where| {
        if ids.is_empty() {
            Some("Select a clip first")
        } else if spots(project, &ids, w).iter().all(|&s| clamped(project, s, dur).is_none()) {
            Some("The clip is too short for a transition")
        } else {
            None
        }
    };
    Offer {
        start: why(Where::Start),
        end: why(Where::End),
        conn_start: !spots(project, &ids, Where::ConnStart).is_empty(),
        conn_end: !spots(project, &ids, Where::ConnEnd).is_empty(),
        all: ids.len() >= 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipKind, TransitionEdge};

    /// Video track: A 0-5, B 5-10 (abuts A), C 10-14 (abuts B), D 20-30 (alone), E 40-40.15 (tiny).
    struct Fx {
        p: Project,
        a: Id,
        b: Id,
        c: Id,
        d: Id,
        e: Id,
    }

    fn fx() -> Fx {
        let mut p = Project::new();
        let mut add = |start: f64, dur: f64| {
            let id = p.new_id();
            p.tracks[0].clips.push(Clip::new(id, ClipKind::Video, "v", start, dur));
            id
        };
        let (a, b, c, d, e) = (add(0.0, 5.0), add(5.0, 5.0), add(10.0, 4.0), add(20.0, 10.0), add(40.0, 0.15));
        p.tracks[0].sort();
        Fx { p, a, b, c, d, e }
    }

    #[test]
    fn where_names_round_trip() {
        for w in [Where::Start, Where::End, Where::ConnStart, Where::ConnEnd, Where::Connection, Where::All] {
            assert_eq!(Where::from_name(w.name()), Some(w));
        }
        assert_eq!(Where::from_name("START"), Some(Where::Start), "case does not matter");
        assert_eq!(Where::from_name("middle"), None);
    }

    /// Each `where` picks its own spots: edges for start / end, the cut only where a clip abuts, the
    /// beginning of every clip for "all" (the cut when it is connected, a fade-in when it is free).
    #[test]
    fn spots_follow_where() {
        let f = fx();
        let edge = |clip, out| Spot::Edge { clip, out };
        let cut = |right| Spot::Cut { right };
        assert_eq!(spots(&f.p, &[f.b], Where::Start), [edge(f.b, false)], "an edge even though A abuts");
        assert_eq!(spots(&f.p, &[f.b], Where::End), [edge(f.b, true)]);
        assert!(spots(&f.p, &[f.a], Where::ConnStart).is_empty(), "nothing abuts A's start");
        assert_eq!(spots(&f.p, &[f.b], Where::ConnStart), [cut(f.b)]);
        assert_eq!(spots(&f.p, &[f.a], Where::ConnEnd), [cut(f.b)], "the cut belongs to the clip on its right");
        assert!(spots(&f.p, &[f.d], Where::ConnEnd).is_empty());
        assert_eq!(spots(&f.p, &[f.b], Where::Connection), [cut(f.b), cut(f.c)], "both of B's cuts");
        assert_eq!(spots(&f.p, &[f.a, f.b, f.d], Where::All), [edge(f.a, false), cut(f.b), edge(f.d, false)]);
        assert_eq!(spots(&f.p, &[f.a, f.a], Where::Start), [edge(f.a, false)], "each spot once");
        assert!(spots(&f.p, &[9999], Where::All).is_empty(), "a clip that is gone is skipped");
    }

    /// The menu rows for 0 / 1 / 2+ selected clips, connected / gapped / connected both sides.
    #[test]
    fn offer_for_each_selection() {
        let f = fx();
        let none = offer(&f.p, &[], 1.0);
        assert_eq!(none.start, Some("Select a clip first"));
        assert_eq!(none.end, Some("Select a clip first"));
        assert!(!none.conn_start && !none.conn_end && !none.all, "nothing selected: no connection / All rows");
        let free = offer(&f.p, &[f.d], 1.0);
        assert_eq!((free.start, free.end), (None, None));
        assert!(!free.conn_start && !free.conn_end && !free.all, "a gapped clip has no connection");
        let first = offer(&f.p, &[f.a], 1.0);
        assert!(!first.conn_start && first.conn_end, "A: only its end touches B");
        let last = offer(&f.p, &[f.c], 1.0);
        assert!(last.conn_start && !last.conn_end);
        let mid = offer(&f.p, &[f.b], 1.0);
        assert!(mid.conn_start && mid.conn_end, "B: connected on both sides -> two rows");
        assert!(!mid.all, "All needs two clips");
        assert!(offer(&f.p, &[f.a, f.d], 1.0).all, "two selected: Apply to All");
        let tiny = offer(&f.p, &[f.e], 1.0);
        assert_eq!(tiny.start, Some("The clip is too short for a transition"));
        assert_eq!(offer(&f.p, &[f.e, f.d], 1.0).start, None, "one clip that can take it is enough");
    }

    /// Never longer than half of the shorter clip it touches; too short for 0.1 s: refused.
    #[test]
    fn duration_is_clamped_to_half_the_shorter_clip() {
        let f = fx();
        assert_eq!(clamped(&f.p, Spot::Cut { right: f.b }, 1.0), Some(1.0), "5 s and 5 s take a full second");
        assert_eq!(clamped(&f.p, Spot::Cut { right: f.b }, 4.0), Some(2.5), "half of 5 s");
        assert_eq!(clamped(&f.p, Spot::Cut { right: f.c }, 4.0), Some(2.0), "half of the 4 s clip beside it");
        assert_eq!(clamped(&f.p, Spot::Edge { clip: f.c, out: true }, 9.0), Some(2.0));
        assert_eq!(clamped(&f.p, Spot::Edge { clip: f.e, out: false }, 1.0), None, "0.075 s is below the minimum");
        let mut f = f;
        let mut st = TransitionsState::default();
        assert_eq!(apply_where(&mut f.p, &mut st, &[f.c], TransitionKind::CrossFade, 9.0, Where::Start, false), 1);
        let tr = &f.p.tracks[0].transitions[0];
        assert!((tr.duration - 2.0).abs() < 1e-9, "stored clamped: {}", tr.duration);
    }

    /// "Apply to All" over a connected start and a free one, in one call: the cut and the fade-in.
    #[test]
    fn apply_to_all_mixes_connections_and_fade_ins() {
        let mut f = fx();
        let mut st = TransitionsState::default();
        let n = apply_where(&mut f.p, &mut st, &[f.b, f.d], TransitionKind::Wipe, 1.0, Where::All, false);
        assert_eq!(n, 2);
        let trs = &f.p.tracks[0].transitions;
        let on = |id| trs.iter().find(|t| t.right == id).expect("a transition");
        assert_eq!(on(f.b).edge, TransitionEdge::Cut, "B's start abuts A: the two-sided cut");
        assert_eq!(on(f.d).edge, TransitionEdge::In, "D's start is free: a fade-in");
        assert!(trs.iter().all(|t| t.kind == TransitionKind::Wipe && (t.duration - 1.0).abs() < 1e-9));
        assert_eq!(st.kind(), TransitionKind::Wipe, "Ctrl+T repeats it");
    }

    /// The default replaces what is on the spot; the setting makes it leave it alone.
    #[test]
    fn existing_transitions_are_replaced_or_skipped() {
        let mut f = fx();
        let mut st = TransitionsState::default();
        apply_where(&mut f.p, &mut st, &[f.b], TransitionKind::CrossFade, 1.0, Where::ConnStart, false);
        assert_eq!(apply_where(&mut f.p, &mut st, &[f.b], TransitionKind::Push, 1.0, Where::ConnStart, false), 1);
        let trs = &f.p.tracks[0].transitions;
        assert_eq!(trs.len(), 1, "replaced, not stacked");
        assert_eq!(trs[0].kind, TransitionKind::Push);
        assert_eq!(apply_where(&mut f.p, &mut st, &[f.b], TransitionKind::Wipe, 1.0, Where::ConnStart, true), 0);
        assert_eq!(f.p.tracks[0].transitions[0].kind, TransitionKind::Push, "skipped: the old one stays");
    }

    /// Start / End are edge transitions however the neighbours sit; Connection is the cut.
    #[test]
    fn start_and_end_are_edges_connection_is_the_cut() {
        let mut f = fx();
        let mut st = TransitionsState::default();
        apply_where(&mut f.p, &mut st, &[f.b], TransitionKind::CrossFade, 1.0, Where::Start, false);
        apply_where(&mut f.p, &mut st, &[f.b], TransitionKind::CrossFade, 1.0, Where::End, false);
        let edges: Vec<_> = f.p.tracks[0].transitions.iter().map(|t| (t.right, t.edge)).collect();
        assert!(edges.contains(&(f.b, TransitionEdge::In)) && edges.contains(&(f.b, TransitionEdge::Out)));
        apply_where(&mut f.p, &mut st, &[f.b], TransitionKind::Push, 1.0, Where::Connection, false);
        let cuts = f.p.tracks[0].transitions.iter().filter(|t| t.edge == TransitionEdge::Cut).count();
        assert_eq!(cuts, 2, "B's two cuts (with A and with C)");
    }

    /// The old click / drop path is untouched: the cut at the start, else a fade-in, no clamping.
    #[test]
    fn add_transitions_is_unchanged() {
        let mut f = fx();
        let mut st = TransitionsState::default();
        assert_eq!(add_transitions(&mut f.p, &[f.b, f.d], &mut st, TransitionKind::CrossFade, 4.0, false), 2);
        let trs = &f.p.tracks[0].transitions;
        assert!(trs.iter().all(|t| (t.duration - 4.0).abs() < 1e-9), "stored as asked");
    }
}
