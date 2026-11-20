//! ---- ws:animator ----
//! The Animator pane: the animation timeline for the selected clips, the Animate page's main panel.
//! - Blocks: one lane per selected clip, a bar per applied block (drag to move, drag an end to stretch, ▸ opens
//!   its keys), a ⚡ / ✓ badge wherever one move hands over to the next (click: match their speeds).
//! - Curves: each clip's keys as curves, every segment's bezier handles draggable, box / Shift select, ease
//!   presets, value or velocity; the clip's channel list sits in the label column.
//! - Flow: how fast each clip's picture moves and the cut into the next one, every hand-off with its two
//!   speeds and a Smooth button.
//! - Speed: each clip's speed ramp.
//!
//! All four are the same timeline (`lanes_view`): a label column, one ruler and playhead, a row (group) per
//! selected clip, the range toggle, the zoom bar, Ctrl+wheel zoom / Shift+wheel pan and a scrollbar under the
//! lanes. The view only changes what a clip's row shows.
//!
//! Blocks also has the Timeline lane above the clips' own: transition spans (`model::ops::spans`), bars that
//! put their transition on every cut under them; and the Path tool, which turns the next stroke drawn in the
//! Viewer into a Path block (`PathCtl`) on the selected clips.
//!
//! Narrow (a docked panel), it turns into a list of the focused clip's blocks with their controls in place.
//! The selected block's controls also show at the top of the Inspector (`block_section`).

use crate::model::*;
use crate::theme::Palette;
use crate::ui::menu;
use eframe::egui::{self, pos2, vec2, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, Ui};
use std::cell::Cell;

thread_local! {
    static SEL: Cell<Option<(Id, usize)>> = const { Cell::new(None) };
    static SPAN: Cell<Option<Id>> = const { Cell::new(None) };
    static TRANS: Cell<Option<Id>> = const { Cell::new(None) };
    static ONION: Cell<u8> = const { Cell::new(0) };
    static ONION_TEX: Cell<Option<(egui::TextureId, [u32; 2])>> = const { Cell::new(None) };
}
/// The selected layer's own picture for the onion skin's ghosts (None while it is still loading).
pub fn onion_tex() -> Option<(egui::TextureId, [u32; 2])> {
    ONION_TEX.with(|o| o.get())
}
pub fn set_onion_tex(t: Option<(egui::TextureId, [u32; 2])>) {
    ONION_TEX.with(|o| o.set(t));
}
/// Onion-skin ghosts each side the Preview draws around the selected layer (0 = off). Set while the
/// Animator is on screen (`animator_ctl` clears it when it isn't).
pub fn onion() -> u8 {
    ONION.with(|o| o.get())
}
pub fn set_onion(n: u8) {
    ONION.with(|o| o.set(n));
}
/// The block selected in the Animator (the Inspector shows its controls).
pub fn selected_block() -> Option<(Id, usize)> {
    SEL.with(|s| s.get())
}
/// The transition span selected in the Animator's Timeline lane (the Inspector shows its controls).
pub fn selected_span() -> Option<Id> {
    SPAN.with(|s| s.get())
}
/// The timeline transition picked in a clip's Transitions row (the Inspector shows its controls).
pub fn selected_transition() -> Option<Id> {
    TRANS.with(|s| s.get())
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum View {
    #[default]
    Blocks,
    Curves,
    Flow,
    Speed,
}

/// The Animator's own tools (its lanes are not the timeline's): Select picks, moves and stretches blocks;
/// Cut splits the block you click, where you click it; Path turns the next stroke drawn in the Viewer into a
/// Path block on the selected clips (the app holds the Viewer's Draw tool while it is picked).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum AnimTool {
    #[default]
    Select,
    Cut,
    Path,
}

#[derive(Default)]
pub struct AnimState {
    pub tool: AnimTool,
    /// The Animator's own snapping, off (it is on by default).
    pub no_snap: bool,
    /// The range the lanes cover (set each frame): Play in the Animator runs it and stops at its end.
    pub range: (f64, f64),
    /// Play in the Animator repeats `range` until stopped; `looping` = the player's loop is the Animator's.
    pub loop_play: bool,
    pub looping: bool,
    pub view: View,
    pub sel: Option<(Id, usize)>,
    open: Vec<(Id, usize)>,
    /// Curves: selected keys as (property, key index) of clip `key_clip` (keys are picked on one clip at a time).
    keys: Vec<(&'static str, usize)>,
    key_clip: Option<Id>,
    focus: Option<&'static str>,
    velocity: bool,
    pub mode: Match,
    /// The clip as it was when a drag started (a drag re-applies from it every frame: one undo step).
    orig: Option<Clip>,
    guide: Option<f64>,
    boxsel: Option<(Pos2, Pos2)>,
    ranges: Option<Vec<(&'static str, f64, f64)>>,
    /// Speed: the picked point, as (clip, key index).
    speed_sel: Option<(Id, usize)>,
    /// Onion-skin ghosts each side (`onion`); `None` = the default, off.
    pub ghosts: Option<u8>,
    /// Width of the lanes' label column (the splitter between labels and lanes); `None` = `HEAD_W`.
    pub head_w: Option<f32>,
    /// Curves: properties hidden on the graph, folded channel groups, every key of the clip (not just the
    /// selected block's), the key / handle a drag holds, where a right-click on empty space landed.
    hidden: Vec<&'static str>,
    /// Folded channel groups (`Settings.anim_folded`, by group name: the same on every clip).
    pub folded: Vec<String>,
    /// Properties left out of the channel list and the graph (`Settings.anim_hidden_props`, by kind: the
    /// same on every clip). A view choice: their keys stay; the list's right-click brings them back.
    pub props_off: Vec<String>,
    /// Clips whose "Properties - <clip>" row is folded to its title.
    clip_folded: Vec<Id>,
    /// `props_off` / `folded` were read from Settings (the app does it once).
    pub view_loaded: bool,
    /// The property rows drawn last frame, as (clip, property).
    #[cfg(test)]
    ch_rows: Vec<(Id, &'static str)>,
    /// Where a Gallery card could land last frame: (lane, its clip; None = the Timeline lane).
    #[cfg(test)]
    drop_pos: Vec<(Rect, Option<Id>)>,
    whole_clip: bool,
    grab: Option<(&'static str, usize, i8)>,
    /// Curves: the channel list's order as the user dragged it - group names and property names in one list
    /// (groups in this order, each group's properties in this order). Empty = `CHANNELS`. A view order only.
    ch_order: Vec<&'static str>,
    /// Curves: the axis a key drag is locked to while Shift is held (true = time only, false = value only).
    axis: Option<bool>,
    /// Where each key was drawn last frame, as (clip, property, key index, screen position).
    #[cfg(test)]
    key_pos: Vec<(Id, &'static str, usize, Pos2)>,
    ctx_t: Option<f64>,
    /// The time window shown (timeline seconds) after a zoom / pan; `None` = fit.
    pub win: Option<(f64, f64)>,
    /// The clips `win` was framed for: another selection frames its own blocks again.
    win_for: Vec<Id>,
    /// ws:forum-ui: the playhead last frame - when it moves out of `win` (a seek, playback), the window
    /// pages after it (`follow_playhead`); a pan alone never moves it, so panning away is not fought.
    last_ph: Option<f64>,
    /// Blocks: the lanes' row height (Alt+wheel); `None` = `LANE_H`.
    pub lane_h: Option<f32>,
    /// Curves / Flow / Speed: a clip's graph height (Alt+wheel, or drag a row's bottom edge in the label
    /// column); `None` = the clips share the panel's height.
    pub graph_h: Option<f32>,
    /// Blocks: where a selection box dragged over empty lane space began.
    lasso: Option<Pos2>,
    /// Blocks: the lanes span the whole timeline (else just the selected clips).
    pub whole: bool,
    /// More blocks picked with Shift / Ctrl-click (`sel` is the main one, the Inspector's).
    pub multi: Vec<(Id, usize)>,
    /// `!no_snap`, set each frame: off = a dragged block snaps to nothing.
    pub snap: bool,
    /// Curves: width of the label column, which holds each clip's channel list; `None` = `CH_W`.
    pub ch_w: Option<f32>,
    /// The last press was in the Animator: Delete / Backspace / Split act on its block or keys, not the clip.
    pub here: bool,
    /// The Timeline lane's picked transition spans (the first is the Inspector's).
    pub spans: Vec<Id>,
    /// A dragged span as its drag started: (t, dur, the end held: -1 start, 1 end, 0 the bar).
    span_orig: Option<(f64, f64, i8)>,
    /// Where each span's bar was drawn last frame.
    #[cfg(test)]
    span_pos: Vec<(Id, Rect)>,
    /// The timeline transition picked in a clip's Transitions row.
    pub trans: Option<Id>,
    /// The end of a transition's bar a drag holds (-1 start, 1 end, 0 the middle: no change).
    trans_held: i8,
    /// Where each transition's bar was drawn last frame, as (transition, lane's clip, rect).
    #[cfg(test)]
    trans_pos: Vec<(Id, Id, Rect)>,
}

/// What a click in the lanes asked for (applied after they are drawn, as one undo step).
type Act = Box<dyn FnOnce(&mut Project, &mut AnimState, &mut AnimResp)>;

/// Every picked block: the main one first.
fn picked(st: &AnimState) -> Vec<(Id, usize)> {
    st.sel.into_iter().chain(st.multi.iter().copied()).collect()
}

/// Delete the picked blocks (highest index first, so the rest keep theirs).
fn remove_picked(p: &mut Project, st: &mut AnimState) {
    let mut v = picked(st);
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for (id, i) in v {
        p.remove_key_block(id, i);
    }
    (st.sel, st.multi, st.open) = (None, Vec::new(), Vec::new());
}

/// Link the picked blocks into one (they must be on one clip).
fn link_act(sel: Vec<(Id, usize)>) -> Act {
    Box::new(move |p, s, o| {
        let idx: Vec<usize> = sel.iter().map(|b| b.1).collect();
        match p.link_key_blocks(sel[0].0, &idx) {
            Some(i) => {
                (s.sel, s.multi, s.open) = (Some((sel[0].0, i)), Vec::new(), Vec::new());
                o.toast = Some("Linked into one block: its properties move, stretch and ease together".into());
            }
            None => o.toast = Some("Pick two or more blocks of one clip to link".into()),
        }
    })
}
/// The picked blocks when there are several, all on one clip (what Link needs).
fn linkable(st: &AnimState) -> Option<Vec<(Id, usize)>> {
    let v = picked(st);
    (v.len() > 1 && v.iter().all(|b| b.0 == v[0].0)).then_some(v)
}

/// Add `block` to clip `id` at clip-local `at` (None = after its last block) and pick it.
/// `exact`: start right at `at` (a Gallery drop) even over another block; else chain after the blocks there.
fn add_act(id: Id, block: KeyBlock, at: Option<f64>, exact: bool) -> Act {
    Box::new(move |p, s, o| {
        let placed = match (exact, at) {
            (true, Some(t)) => p.place_key_block(id, &block, t, None),
            _ => p.apply_key_block(id, &block, at, None),
        };
        if let Some((t, _)) = placed {
            let i = p.clip(id).and_then(|c| c.blocks.iter().position(|x| (x.t - t).abs() < 1e-9));
            (s.sel, s.multi) = (i.map(|i| (id, i)), Vec::new());
            o.toast = Some(format!("Added {}", block.name));
        } else {
            o.toast = Some(format!("{} has nothing to animate on this clip", block.name));
        }
    })
}

fn flow_act(sel: Vec<(Id, usize)>, mode: Match) -> Act {
    Box::new(move |p, _, o| {
        let n = p.flow_key_blocks(&sel, mode);
        o.toast = Some(match n {
            0 => "Nothing to flow: pick blocks on one clip, on layers that overlap, or either side of a cut".into(),
            _ => format!("Flowed {n} pair{} of blocks together", if n == 1 { "" } else { "s" }),
        });
    })
}

/// Delete or Backspace, taken from the frame's input so the app's own Delete never also removes the clip.
fn del_pressed(ui: &Ui) -> bool {
    use egui::{Key, Modifiers};
    !ui.ctx().wants_keyboard_input()
        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete) | i.consume_key(Modifiers::NONE, Key::Backspace))
}

/// The key whose ease shapes the move picked key `k` belongs to: itself, or the key before it when no move
/// follows (a key's ease is of the segment after it, so on a block's last key it would change nothing).
fn ease_slot(a: &Animated, k: usize, end: f64) -> usize {
    if k > 0 && (k + 1 >= a.keys.len() || a.keys[k].t >= end - 1e-4) {
        k - 1
    } else {
        k
    }
}

#[derive(Default)]
pub struct AnimResp {
    pub seeked: bool,
    pub edited: bool,
    /// Play this timeline range once (after a change, to show it).
    pub play: Option<(f64, f64)>,
    pub toast: Option<String>,
    /// Save this block as a preset (the Gallery's Animate folder).
    pub save: Option<KeyBlock>,
    pub import: bool,
    pub open_folder: bool,
    /// Group the selection into a stack (a nested sequence) / select the clip under the playhead + its overlays.
    pub stack: bool,
    /// "Make transitionable": close the gaps between the selected clips (all of them when none are selected).
    pub transitionable: bool,
    pub with_overlays: bool,
    /// Make the selected clips follow this saved path (and turn along it when true).
    pub follow: Option<(Id, bool)>,
}

const HEAD_W: f32 = 132.0;
/// Curves' label column: wide enough for a channel row (name, value, ◀ ◆ ▶ 👁).
const CH_W: f32 = 300.0;
/// The lowest a clip's graph row goes (Curves / Flow / Speed).
const GRAPH_MIN: f32 = 120.0;
/// The lanes' horizontal scrollbar.
const SCROLL_H: f32 = 12.0;
/// "Drop with collisions": (name, bounce, gravity), the first is what the button does.
const COLLIDE: [(&str, f64, f64); 4] =
    [("Normal", 0.45, 0.55), ("Bouncy", 0.9, 0.5), ("Heavy", 0.05, 0.9), ("Floaty", 0.5, 0.1)];
const LANE_H: f32 = 30.0;
/// An open block's row: its move as a string from origin to target.
const STRING_H: f32 = 72.0;
const SNAP_PX: f32 = 7.0;
/// Narrower than this the pane is a list (`compact`).
pub const COMPACT_W: f32 = 560.0;
pub const PROPS: [&str; 10] =
    ["x", "y", "scale", "scale_x", "scale_y", "rotation", "opacity", "volume", "pan", "speed"];

pub fn prop_label(p: &str) -> &'static str {
    match p {
        "x" => "Position X",
        "y" => "Position Y",
        "scale" => "Scale",
        "scale_x" => "Width",
        "scale_y" => "Height",
        "rotation" => "Rotation",
        "opacity" => "Opacity",
        "pan" => "Pan",
        "speed" => "Speed",
        _ => "Volume",
    }
}
pub(crate) fn prop_color(p: &str) -> Color32 {
    match p {
        "x" => Color32::from_rgb(0x33, 0x66, 0xFF),
        "y" => Color32::from_rgb(0x2B, 0xB3, 0xA3),
        "scale" | "scale_x" | "scale_y" => Color32::from_rgb(0xF2, 0xC1, 0x4E),
        "rotation" => Color32::from_rgb(0x9B, 0x5D, 0xE5),
        "opacity" => Color32::from_rgb(0xE5, 0x48, 0x7A),
        "pan" => Color32::from_rgb(0x6C, 0x8E, 0xBF),
        "speed" => Color32::from_rgb(0xE0, 0x60, 0x2A),
        _ => Color32::from_rgb(0x4C, 0xC0, 0x6A),
    }
}
const GOOD: Color32 = Color32::from_rgb(0x4C, 0xC0, 0x6A);
const BAD: Color32 = Color32::from_rgb(0xEF, 0x55, 0x50);

fn unit(p: &str) -> &'static str {
    match p {
        "x" | "y" => "px",
        "rotation" => "°",
        _ => "%",
    }
}
/// A value as shown: scale / opacity / volume in %.
fn shown(p: &str, v: f64) -> f64 {
    if matches!(p, "x" | "y" | "rotation") {
        v
    } else {
        v * 100.0
    }
}
pub fn speed_text(p: &str, v: f64) -> String {
    format!("{:.0} {}/s", v, if p.starts_with("scale") { "%" } else { unit(p) })
}

/// The selected clips, top track first (as the timeline stacks them).
fn lanes(project: &Project, selection: &[Id]) -> Vec<Id> {
    let mut v: Vec<(usize, f64, Id)> =
        selection.iter().filter_map(|&id| Some((project.track_of(id)?, project.clip(id)?.start, id))).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    v.dedup_by_key(|x| x.2);
    v.into_iter().map(|x| x.2).collect()
}

/// The selected clips and the clips their transitions hand over from / to, in lane order: a transition's
/// two sides lie one over the other in time, so its overlap shows.
fn with_neighbours(project: &Project, ids: &[Id]) -> Vec<Id> {
    let mut v = ids.to_vec();
    for &id in ids {
        for (ti, tr) in project.transitions_of(id) {
            if let Some((l, r)) = project.tracks[ti].transition_clips(tr) {
                v.extend([l, r].into_iter().flatten().map(|c| c.id));
            }
        }
    }
    lanes(project, &v)
}

/// The timeline transitions on clip `id`: each with the window it plays in and "left → right" names.
fn clip_transitions(project: &Project, id: Id) -> Vec<(Transition, (f64, f64), String)> {
    let mut v = Vec::new();
    for (ti, tr) in project.transitions_of(id) {
        let Some((l, r)) = project.tracks[ti].transition_clips(tr) else { continue };
        let Some((c, h)) = tr.cut_half(l, r) else { continue };
        let name = |c: Option<&Clip>| c.map_or("nothing".to_string(), |c| c.name.clone());
        v.push((tr.clone(), (c - h, c + h), format!("{} → {}", name(l), name(r))));
    }
    v
}

fn ticks(t0: f64, t1: f64, px: f32) -> Vec<f64> {
    let span = (t1 - t0).max(1e-3);
    let step = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0]
        .into_iter()
        .find(|s| px as f64 / (span / s) >= 46.0)
        .unwrap_or(60.0);
    let mut out = Vec::new();
    let mut x = (t0 / step).ceil() * step;
    while x <= t1 + 1e-9 {
        out.push((x * 1000.0).round() / 1000.0);
        x += step;
    }
    out
}

// ---------- the look (the approved preview's: dark cards, a colour per kind of block, pills) ----------

/// Card, track, line and text greys of the Animator (the 1.3.0 Performance window's family).
pub(crate) mod look {
    use eframe::egui::Color32;
    pub const CARD: Color32 = Color32::from_rgb(0x1B, 0x1D, 0x23);
    pub const TRACK: Color32 = Color32::from_rgb(0x2C, 0x2F, 0x37);
    pub const LINE: Color32 = Color32::from_rgb(0x26, 0x29, 0x31);
    pub const DEEP: Color32 = Color32::from_rgb(0x10, 0x12, 0x16);
    pub const TEXT: Color32 = Color32::from_gray(232);
    pub const MUTED: Color32 = Color32::from_gray(140);
    pub const DIM: Color32 = Color32::from_rgb(0x5F, 0x63, 0x6D);
    pub const HANDLE: Color32 = Color32::from_rgb(0xF2, 0xC1, 0x4E);
}

/// The kind of a block and its colour. Every kind has one colour of its own, used for its bar in the lanes,
/// its Inspector pill, its Gallery card and its dot in the "+ Animation" menu (which so doubles as the legend).
pub fn block_kind(b: &AppliedBlock) -> (&'static str, Color32) {
    if b.ctl.as_ref().is_some_and(|c| c.path.is_some()) {
        return ("Path", kind_color("Path"));
    }
    let gen = b.ctl.as_ref().and_then(|c| c.src.gen);
    kind_of(&b.name, gen)
}
/// A kind's colour ("Custom" = hand keys, linked and split blocks, saved presets).
pub fn kind_color(kind: &str) -> Color32 {
    let c = |r, g, b| Color32::from_rgb(r, g, b);
    match kind {
        "In" => c(0x2B, 0xB3, 0xA3),       // teal
        "Out" => c(0xE0, 0x60, 0x2A),      // orange
        "Physics" => c(0x9B, 0x5D, 0xE5),  // purple
        "Collide" => c(0xFF, 0x5C, 0xCB),  // pink
        "Emphasis" => c(0xF2, 0xC1, 0x4E), // yellow
        "Camera" => c(0x33, 0x66, 0xFF),   // blue
        "Text" => c(0x7F, 0xD0, 0xFF),     // sky
        "Sound" => c(0x4C, 0xC0, 0x6A),    // green
        "Speed" => c(0xC0, 0x22, 0x3B),    // red
        "Path" => c(0x96, 0xDC, 0x32),     // lime
        "Transition" => c(0xFF, 0x80, 0x80), // salmon: a span over the timeline
        _ => c(0x9A, 0xA3, 0xB2),          // Custom: slate
    }
}
pub fn kind_of(name: &str, gen: Option<Gen>) -> (&'static str, Color32) {
    let n = name.to_ascii_lowercase();
    let has = |w: &str| n.contains(w);
    let kind = if let Some(g) = gen {
        if g == Gen::Shake {
            "Camera"
        } else {
            "Physics"
        }
    } else if has("collide") {
        "Collide"
    } else if has("speed") || has("slow-mo") {
        "Speed"
    } else if has("sound") || has("duck") || has("pan ") || has("tremolo") {
        "Sound"
    } else if has("typewriter") {
        "Text"
    } else if has(" in") || has("drop") || has("elastic") {
        "In"
    } else if has(" out") || has("whip") {
        "Out"
    } else if has("ken burns") || has("drift") || has("zoom") || has("camera") {
        "Camera"
    } else if ["pulse", "shake", "wiggle", "bounce", "spin", "flash"].iter().any(|w| has(w)) {
        "Emphasis"
    } else {
        "Custom"
    };
    (kind, kind_color(kind))
}

/// `text` on one line, cut with "…" to fit `max_w` (the list rule: a label fills its column and truncates).
pub(crate) fn elided(ui: &Ui, text: &str, font: FontId, color: Color32, max_w: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping {
        max_width: max_w.max(8.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.fonts_mut(|f| f.layout_job(job))
}

/// A pill: a chip that is on (accent tint + accent edge) or off (card + line).
fn chip(ui: &mut Ui, on: bool, text: &str) -> egui::Response {
    let accent = ui.visuals().selection.stroke.color;
    let b = egui::Button::new(egui::RichText::new(text).size(11.5).color(if on { Color32::WHITE } else { look::TEXT }))
        .fill(if on { accent.gamma_multiply(0.28) } else { look::CARD })
        .stroke(Stroke::new(1.0, if on { accent } else { look::LINE }))
        .corner_radius(6.0);
    ui.add(b)
}

/// The accent-filled main button (+ Block).
fn primary(ui: &mut Ui, text: &str) -> egui::Response {
    let accent = ui.visuals().selection.stroke.color;
    ui.add(
        egui::Button::new(egui::RichText::new(text).size(12.0).color(crate::ui::tools::on_accent(accent))).fill(accent).corner_radius(6.0),
    )
}

/// The segmented control (the Performance window's): a track with the picked segment filled in accent.
pub(crate) fn segmented<T: PartialEq + Copy>(ui: &mut Ui, cur: &mut T, items: &[(T, &str)]) -> bool {
    let accent = ui.visuals().selection.stroke.color;
    let mut changed = false;
    egui::Frame::new().fill(look::TRACK).corner_radius(7.0).inner_margin(2.0).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for &(v, n) in items {
            let on = *cur == v;
            let t = egui::RichText::new(n).size(12.0).color(if on { crate::ui::tools::on_accent(accent) } else { look::MUTED });
            if ui
                .add(
                    egui::Button::new(t)
                        .fill(if on { accent } else { Color32::TRANSPARENT })
                        .stroke(Stroke::NONE)
                        .corner_radius(5.0),
                )
                .clicked()
            {
                *cur = v;
                changed = true;
            }
        }
    });
    changed
}

/// An ease's shape as a tiny curve in `r`.
fn ease_icon(p: &egui::Painter, r: Rect, e: Ease, col: Color32) {
    let pts: Vec<Pos2> = (0..=12)
        .map(|i| {
            let f = i as f64 / 12.0;
            pos2(r.left() + f as f32 * r.width(), r.bottom() - e.apply(f) as f32 * r.height())
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(1.4, col)));
}

/// A pill with the ease's curve in front of its name.
fn ease_chip(ui: &mut Ui, on: bool, name: &str, e: Ease) -> egui::Response {
    let accent = ui.visuals().selection.stroke.color;
    let font = FontId::proportional(11.5);
    let tw = ui.fonts_mut(|f| f.layout_no_wrap(name.to_string(), font.clone(), Color32::WHITE).size().x);
    let (r, resp) = ui.allocate_exact_size(vec2(tw + 34.0, 22.0), Sense::click());
    let hover = resp.hovered();
    let p = ui.painter();
    p.rect(
        r,
        6.0,
        if on { accent.gamma_multiply(0.28) } else { look::CARD },
        Stroke::new(
            1.0,
            if on {
                accent
            } else if hover {
                look::DIM
            } else {
                look::LINE
            },
        ),
        egui::StrokeKind::Inside,
    );
    ease_icon(
        p,
        Rect::from_min_size(r.min + vec2(7.0, 5.0), vec2(13.0, 12.0)),
        e,
        if on { Color32::WHITE } else { look::TEXT },
    );
    p.text(
        pos2(r.left() + 26.0, r.center().y),
        Align2::LEFT_CENTER,
        name,
        font,
        if on { Color32::WHITE } else { look::TEXT },
    );
    resp
}

/// "IN" / "CAMERA" … in its colour.
fn kind_pill(ui: &mut Ui, kind: &str, col: Color32) {
    let t = egui::RichText::new(kind.to_uppercase()).size(10.0).strong().color(col.lerp_to_gamma(Color32::WHITE, 0.35));
    egui::Frame::new()
        .fill(col.gamma_multiply(0.22))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(t);
        });
}

/// The four arrows; Some(side) when one was clicked.
fn side_pad(ui: &mut Ui, cur: Option<Dir>, only_lr: bool) -> Option<Dir> {
    let accent = ui.visuals().selection.stroke.color;
    let mut out = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (d, a) in [(Dir::Left, "←"), (Dir::Up, "↑"), (Dir::Down, "↓"), (Dir::Right, "→")] {
            if only_lr && matches!(d, Dir::Up | Dir::Down) {
                continue;
            }
            let on = cur == Some(d);
            let b = egui::Button::new(egui::RichText::new(a).size(13.0).color(if on {
                Color32::WHITE
            } else {
                look::TEXT
            }))
            .min_size(vec2(28.0, 24.0))
            .fill(if on { accent.gamma_multiply(0.3) } else { look::CARD })
            .stroke(Stroke::new(1.0, if on { accent } else { look::LINE }))
            .corner_radius(5.0);
            if ui.add(b).on_hover_text(d.name()).clicked() {
                out = Some(d);
            }
        }
    });
    out
}

/// The ease shape: drag the two yellow handles; a dot runs along the curve while the pointer is over it (the
/// live preview - it repaints only then, so an idle Inspector stays idle). Returns the new ease when dragged
/// and whether a drag just started (one undo step).
fn ease_editor(ui: &mut Ui, id: egui::Id, e: Ease) -> (Option<Ease>, bool) {
    let accent = ui.visuals().selection.stroke.color;
    let (r, resp) = ui.allocate_exact_size(vec2(170.0, 130.0), Sense::click());
    let resp = resp.on_hover_text("Ctrl+wheel zooms out and in, double-click resets the zoom");
    let p = ui.painter_at(r);
    p.rect(r, 6.0, look::DEEP, Stroke::new(1.0, look::LINE), egui::StrokeKind::Inside);
    let plot = Rect::from_min_max(r.min + vec2(12.0, 26.0), r.max - vec2(34.0, 24.0));
    let (x1, y1, x2, y2) = e.handles();
    // the view grows to keep both handles in sight (an overshoot goes past 0..1) and shrinks back with them,
    // never tighter than 0..1. `extra` is the user's own zoom out; a drag works in the view it started in, so
    // the handle doesn't run away as the view follows it
    let (zoom_id, view_id) = (id.with("zoom"), id.with("view"));
    let mut extra: f32 = ui.ctx().data(|d| d.get_temp(zoom_id).unwrap_or(0.0));
    if ui.rect_contains_pointer(r) {
        let z = ui.input(|i| i.zoom_delta());
        if z != 1.0 {
            extra = (extra - (z - 1.0) * 1.5).clamp(0.0, 3.0);
        }
    }
    if resp.double_clicked() {
        extra = 0.0;
    }
    ui.ctx().data_mut(|d| d.insert_temp(zoom_id, extra));
    let fit = (y1.min(y2).min(0.0) - extra, y1.max(y2).max(1.0) + extra);
    let held: Option<(f32, f32)> = ui.ctx().data(|d| d.get_temp(view_id));
    let (lo, hi) = held.unwrap_or(fit);
    let at = |x: f32, y: f32| pos2(plot.left() + x * plot.width(), plot.bottom() - (y - lo) / (hi - lo) * plot.height());
    // the 0..1 box: the whole plot at normal size, a band inside it when the view is stretched
    p.rect_stroke(plot, 0.0, Stroke::new(1.0, look::LINE), egui::StrokeKind::Inside);
    p.rect_stroke(Rect::from_two_pos(at(0.0, 0.0), at(1.0, 1.0)), 0.0, Stroke::new(1.0, look::TRACK), egui::StrokeKind::Inside);
    let stretched = hi - lo > 1.001;
    if stretched {
        p.text(
            pos2(plot.right(), r.top() + 7.0),
            Align2::RIGHT_TOP,
            format!("{:.1}× tall", hi - lo),
            FontId::proportional(10.0),
            look::HANDLE,
        );
    }
    p.line_segment([at(0.0, 0.0), at(x1, y1)], Stroke::new(1.0, look::MUTED));
    p.line_segment([at(1.0, 1.0), at(x2, y2)], Stroke::new(1.0, look::MUTED));
    let pts: Vec<Pos2> = (0..=40)
        .map(|i| {
            let f = i as f64 / 40.0;
            at(f as f32, e.apply(f) as f32)
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(2.0, accent)));
    let mut out = None;
    let mut started = false;
    for (k, hx, hy) in [(1, x1, y1), (2, x2, y2)] {
        let c = at(hx, hy);
        let hr = ui.interact(Rect::from_center_size(c, vec2(14.0, 14.0)), id.with(k), Sense::drag());
        started |= hr.drag_started();
        if hr.drag_started() {
            ui.ctx().data_mut(|d| d.insert_temp(view_id, (lo, hi)));
        }
        if hr.drag_stopped() {
            ui.ctx().data_mut(|d| d.remove::<(f32, f32)>(view_id));
        }
        if let Some(pp) = hr.interact_pointer_pos().filter(|_| hr.dragged()) {
            // past the box's edge the value keeps going: the view catches up on release
            let fx = ((pp.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
            let fy = (lo + (plot.bottom() - pp.y) / plot.height() * (hi - lo)).clamp(-3.0, 4.0);
            out = Some(if k == 1 {
                Ease::Bezier { x1: fx, y1: fy, x2, y2 }
            } else {
                Ease::Bezier { x1, y1, x2: fx, y2: fy }
            });
        }
        p.circle_filled(c.clamp(r.min + vec2(5.0, 5.0), r.max - vec2(5.0, 5.0)), 5.5, look::HANDLE);
    }
    // where the 0..1 band sits in the view (the whole bar at normal size), and on hover the value running it
    let bar = Rect::from_min_max(pos2(r.right() - 20.0, plot.top()), pos2(r.right() - 10.0, plot.bottom()));
    p.rect_filled(bar, 3.0, look::CARD);
    if stretched {
        let band = Rect::from_min_max(pos2(bar.left(), at(0.0, 1.0).y), pos2(bar.right(), at(0.0, 0.0).y));
        p.rect_filled(band, 3.0, look::TRACK);
    }
    if resp.hovered() || ui.rect_contains_pointer(r) {
        let ph = ((ui.input(|i| i.time) % 1.6) / 1.2).min(1.0);
        let v = e.apply(ph) as f32;
        p.circle_filled(at(ph as f32, v), 4.0, Color32::WHITE);
        p.rect_filled(Rect::from_center_size(pos2(bar.center().x, at(0.0, v).y), vec2(8.0, 6.0)), 2.0, accent);
        ui.ctx().request_repaint();
    }
    p.text(
        r.left_top() + vec2(10.0, 7.0),
        Align2::LEFT_TOP,
        "Drag the handles",
        FontId::proportional(10.5),
        look::MUTED,
    );
    (out, started)
}

/// The Animator. `undo` once per gesture, before the first change.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    st: &mut AnimState,
    project: &mut Project,
    selection: &[Id],
    playhead: &mut f64,
    playing: bool,
    pal: &Palette,
    presets: &[KeyBlock],
    undo: &mut dyn FnMut(&Project),
) -> AnimResp {
    let mut out = AnimResp::default();
    let ids = lanes(project, selection);
    // the lanes: the selection and the clips its transitions join it to
    let rows = with_neighbours(project, &ids);
    // a selected block that went away (undo, delete, another clip) is forgotten
    st.sel = st.sel.filter(|&(id, i)| rows.contains(&id) && project.clip(id).is_some_and(|c| i < c.blocks.len()));
    if st.sel.is_none() {
        st.multi.clear();
    }
    // a picked transition that went away, or whose clips left the lanes, is forgotten
    st.trans = st.trans.filter(|&t| rows.iter().any(|&c| project.transitions_of(c).iter().any(|x| x.1.id == t)));
    TRANS.with(|s| s.set(st.trans));
    #[cfg(test)]
    st.trans_pos.clear();
    // selected keys belong to one clip: forgotten when it leaves the selection
    if st.key_clip.is_none_or(|k| !rows.contains(&k)) {
        (st.keys, st.key_clip) = (Vec::new(), None);
    }
    #[cfg(test)]
    st.key_pos.clear();
    #[cfg(test)]
    st.ch_rows.clear();
    let main = st.sel;
    st.multi.retain(|&(id, i)| Some((id, i)) != main && project.clip(id).is_some_and(|c| i < c.blocks.len()));
    SEL.with(|s| s.set(st.sel));
    // a picked span that went away (undo, delete) is forgotten
    st.spans.retain(|id| project.spans.iter().any(|s| s.id == *id));
    SPAN.with(|s| s.set(st.spans.first().copied()));
    #[cfg(test)]
    st.span_pos.clear();
    set_onion(st.ghosts.unwrap_or(0));
    let compact = ui.available_width() < COMPACT_W;
    header(ui, st, project, &ids, *playhead, compact, presets, undo, &mut out);
    ui.separator();
    if ids.is_empty() {
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("Select clips on the timeline to animate them.").color(pal.text_dim));
            if ui.button("Select the clip under the playhead and its overlays").clicked() {
                out.with_overlays = true;
            }
        });
        // the Timeline lane (transitions over the clips) needs no selection
        if compact || project.editing.is_some() || project.duration() <= 0.0 {
            return out;
        }
        ui.add_space(8.0);
    }
    match st.view {
        _ if compact => compact_list(ui, st, project, &ids, playhead, pal, undo, &mut out),
        // one timeline for all four views: the view only changes what a clip's row shows
        _ => lanes_view(ui, st, project, &ids, &rows, playhead, presets, pal, undo, &mut out),
    }
    // one kind of thing is picked at a time: a block or a span picked now drops the transition
    if st.sel.is_some() || !st.spans.is_empty() {
        st.trans = None;
    }
    // Delete / Backspace: the picked blocks and only them (selected keys or a speed point took the key first)
    let any = st.sel.is_some() || !st.spans.is_empty() || st.trans.is_some();
    if !playing && any && (st.here || ui.ui_contains_pointer()) && del_pressed(ui) {
        undo(project);
        remove_picked(project, st);
        for id in std::mem::take(&mut st.spans) {
            project.remove_span(id);
        }
        if let Some(t) = st.trans.take() {
            project.remove_transition(t);
        }
        out.edited = true;
    }
    SEL.with(|s| s.set(st.sel));
    SPAN.with(|s| s.set(st.spans.first().copied()));
    TRANS.with(|s| s.set(st.trans));
    out
}

#[allow(clippy::too_many_arguments)]
fn header(
    ui: &mut Ui,
    st: &mut AnimState,
    project: &mut Project,
    ids: &[Id],
    playhead: f64,
    compact: bool,
    presets: &[KeyBlock],
    undo: &mut dyn FnMut(&Project),
    out: &mut AnimResp,
) {
    // wraps: a narrow Animator gets two or three rows instead of controls drawn over each other
    ui.horizontal_wrapped(|ui| {
        if !compact {
            segmented(ui, &mut st.view, &[(View::Blocks, "Blocks"), (View::Curves, "Curves"), (View::Flow, "Flow"), (View::Speed, "Speed")]);
            ui.add_space(6.0);
        }
        let target = st.sel.map(|s| s.0).or(ids.first().copied());
        // the accent-filled main button of the preview (a menu button wearing it)
        let accent = ui.visuals().selection.stroke.color;
        let saved = ui.visuals().widgets.clone();
        let ws = &mut ui.visuals_mut().widgets;
        for w in [&mut ws.inactive, &mut ws.hovered, &mut ws.open] {
            w.weak_bg_fill = accent;
            w.bg_fill = accent;
            w.corner_radius = 6.into();
            w.bg_stroke = Stroke::NONE;
        }
        // (spans belong to the main timeline: none while a nested sequence is open)
        let (mut span_pick, main) = (None, project.editing.is_none());
        let r = menu::button(ui, egui::RichText::new("+ Animation").color(crate::ui::tools::on_accent(accent)), |ui| {
            anim_menu(ui, presets, main.then_some(&mut span_pick))
        });
        ui.visuals_mut().widgets = saved;
        // a transition over the timeline: a span across the range the lanes show
        if let Some(k) = span_pick {
            undo(project);
            let (a, b) = if st.range.1 > st.range.0 { st.range } else { (0.0, project.duration().max(1.0)) };
            span_add_act(a, b - a, k)(project, st, out);
            out.edited = true;
        }
        if let (Some(Some(name)), Some(id)) = (r.inner, target) {
            if let Some(b) = find_block(presets, &name) {
                undo(project);
                let at = project.clip(id).filter(|c| c.contains(playhead)).map(|c| playhead - c.start);
                if let Some((t, _)) = project.apply_key_block(id, &b, at, None) {
                    let c = project.clip(id).unwrap();
                    st.sel = Some((id, c.blocks.iter().position(|x| (x.t - t).abs() < 1e-9).unwrap_or(0)));
                    out.edited = true;
                    out.toast = Some(format!("Added {name} to {}", c.name));
                }
            }
        }
        let on = st.ghosts.unwrap_or(0) > 0;
        if chip(ui, on, "Onion skin").on_hover_text("Ghost outlines of the selected layer a few frames before (blue) and after (orange), and where the selected block starts (dashed)").clicked() {
            st.ghosts = Some(if on { 0 } else { 3 });
        }
        if chip(ui, false, "Stack").on_hover_text("Group the selected clip and its overlays so a transition or block moves them as one").clicked() {
            out.stack = true;
        }
        if chip(ui, false, "Make transitionable")
            .on_hover_text("Close the gaps between the selected clips (every clip when none are selected) so each pair shares a cut a transition can sit on")
            .clicked()
        {
            out.transitionable = true;
        }
        let rec = crate::model::recording();
        // ponytail: the dot is painted over the chip's leading spaces instead of a chip that takes an icon
        let r = chip(ui, rec, "    Record").on_hover_text(
            "On: changing a value at the playhead adds a key there. Off: keys are only placed by hand (◆, double-click)",
        );
        ui.painter().circle_filled(pos2(r.rect.left() + 11.0, r.rect.center().y), 3.5, if rec { BAD } else { look::DIM });
        if r.clicked() {
            crate::model::set_recording(!rec);
        }
        let mut collide = None;
        menu::button(ui, "⋯", |ui| {
            menu::sub(ui, None, "Drop with collisions", |ui| {
                for (name, bounce, gravity) in COLLIDE {
                    if menu::row(ui, None, name, "").clicked() {
                        collide = Some((bounce, gravity));
                    }
                }
            });
            ui.separator();
            if menu::row(ui, None, "Select clip + overlays", "").clicked() {
                out.with_overlays = true;
            }
            if menu::row(ui, None, "Stack selection (moves as one in transitions)", "").clicked() {
                out.stack = true;
            }
            ui.separator();
            if menu::row(ui, None, "Draw a motion path…", "").on_hover_text("The Path tool: draw in the Viewer and the selected clips travel your stroke").clicked() {
                st.tool = AnimTool::Path;
            }
            menu::sub(ui, None, "Follow a path", |ui| {
                if project.paths.is_empty() {
                    ui.weak("No saved paths");
                }
                for p in &project.paths {
                    menu::sub(ui, None, &p.name, |ui| {
                        if menu::row(ui, None, "Follow it", "").clicked() {
                            out.follow = Some((p.id, false));
                        }
                        if menu::row(ui, None, "Follow it and turn along it", "").clicked() {
                            out.follow = Some((p.id, true));
                        }
                    });
                }
            });
            ui.separator();
            let sel = st.sel.and_then(|(id, i)| project.clip(id).and_then(|c| c.blocks.get(i).map(|b| (c, b))));
            let r = ui.add_enabled_ui(sel.is_some(), |ui| menu::row(ui, None, "Save block as preset", "")).inner;
            if r.clicked() {
                if let Some((c, b)) = sel {
                    out.save = Some(capture(c, b, project.width as f64, project.height as f64));
                }
            }
            if menu::row(ui, None, "Import presets…", "").clicked() {
                out.import = true;
            }
            if menu::row(ui, None, "Show preset files (to share)", "").clicked() {
                out.open_folder = true;
            }
        });
        if let Some(k) = collide {
            drop_collide(project, ids, playhead, k, undo, out);
        }
        if st.view == View::Flow || compact {
            // in the flow, not pinned right: pinned, it was drawn over the chips once the row ran out of room
            ui.add_space(6.0);
            mode_seg(ui, st);
        }
    });
}

/// The animations to add, by kind (the "+ Animation" button and a lane's right-click): the picked name.
/// A transition for a span: a built-in kind, or an installed library transition (`Shader` and its source).
type SpanKind = (TransitionKind, String);

/// The transitions a span can make, as menu rows (the built-in kinds, then the installed library ones): the
/// picked one.
fn span_kinds(ui: &mut Ui) -> Option<SpanKind> {
    let mut pick = None;
    for k in TransitionKind::ALL {
        if menu::row(ui, None, k.name(), "").clicked() {
            pick = Some((k, String::new()));
        }
    }
    let now = ui.input(|i| i.time);
    for it in crate::ui::transitions_ui::library_items(crate::engine::library::Kind::Transition, now).iter() {
        if menu::row(ui, None, &it.name, "").clicked() {
            pick = Some((TransitionKind::Shader, it.src.to_string()));
        }
    }
    pick
}

/// Make span `s` produce transition `k` (a shader's controls start at its header's values).
pub(crate) fn set_span_kind(s: &mut TransitionSpan, (kind, src): SpanKind) {
    s.kind = kind;
    if kind == TransitionKind::Shader {
        if s.shader != src {
            s.params = crate::engine::shaderlib::parse_meta(&src).params.iter().map(|p| p.default).collect();
        }
        s.shader = src;
    }
}

/// Add a span over timeline [t, t + dur] making transition `k`, and pick it.
fn span_add_act(t: f64, dur: f64, k: SpanKind) -> Act {
    Box::new(move |p, s, o| {
        let id = p.add_span(t, dur, k.0);
        if let Some(sp) = p.spans.iter_mut().find(|s| s.id == id) {
            set_span_kind(sp, k);
        }
        p.refresh_spans();
        (s.spans, s.sel, s.multi) = (vec![id], None, Vec::new());
        let n = p.spans.iter().find(|s| s.id == id).map_or(0, |s| s.made.len());
        o.toast = Some(if n == 0 {
            "Added to the Timeline lane: no cut under it yet - move or stretch it over two clips".into()
        } else {
            "Added to the Timeline lane: every cut under it has the transition, and follows the bar".into()
        });
    })
}

/// The kinds of animation, in the "+ Animation" menu's order (the Gallery's Blocks tab uses the same): In and
/// Out are how a clip arrives or leaves, the rest move it while it is on screen.
pub const BLOCK_GROUPS: [(&str, &[&str]); 8] = [
    ("In", &["Fade In", "Pop In", "Slide In Left", "Drop In", "Swing In", "Elastic"]),
    ("Out", &["Fade Out", "Pop Out", "Slide Out Right", "Whip"]),
    ("Physics", &["Gravity Fall", "Throw & Bounce", "Bounce Drop", "Spring In", "Throw Out", "Wobble"]),
    ("Emphasis", &["Pulse", "Zoom Punch", "Shake", "Wiggle", "Bounce", "Spin", "Flash"]),
    ("Camera", &["Ken Burns", "Drift", "Camera Shake"]),
    ("Text", &["Typewriter"]),
    ("Sound", &["Sound Swell", "Sound Fade", "Duck", "Pan Sweep", "Pan Swing", "Tremolo"]),
    ("Speed", &["Speed Ramp In", "Speed Ramp Out", "Slow-mo Hit", "Speed Up"]),
];

/// The Gallery's Blocks tab sections, in the "+ Animation" menu's order: the timeline's transitions (its
/// Transitions ▸ Timeline), `BLOCK_GROUPS`, then your own blocks.
pub fn block_sections() -> Vec<&'static str> {
    std::iter::once("Transitions").chain(BLOCK_GROUPS.iter().map(|g| g.0)).chain(["Mine"]).collect()
}

/// The `BLOCK_GROUPS` group block `name` belongs to ("Mine" for one of yours): the menu's own rows first,
/// then by name (the other slides), else Emphasis.
pub fn block_group(name: &str, mine: bool) -> &'static str {
    if mine {
        return "Mine";
    }
    if let Some(g) = BLOCK_GROUPS.iter().find(|g| g.1.contains(&name)) {
        return g.0;
    }
    match name {
        n if n.starts_with("Slide In") => "In",
        n if n.starts_with("Slide Out") => "Out",
        _ => "Emphasis",
    }
}

/// (`span` = where a pick under Transitions ▸ Timeline goes; None leaves that row out.)
fn anim_menu(ui: &mut Ui, presets: &[KeyBlock], mut span: Option<&mut Option<SpanKind>>) -> Option<String> {
    let mut pick = None;
    for (n, (g, names)) in BLOCK_GROUPS.into_iter().enumerate() {
        // a transition is how a clip arrives or leaves; everything else moves it while it is on screen
        match n {
            0 => {
                ui.weak("Transitions");
                // over the timeline: a span in the Timeline lane, on every cut under it
                if let Some(span) = span.as_deref_mut() {
                    let y0 = ui.cursor().top();
                    menu::sub(ui, None, "Timeline", |ui| {
                        if let Some(k) = span_kinds(ui) {
                            *span = Some(k);
                        }
                    });
                    kind_dot(ui, y0, "Transition");
                }
            }
            2 => {
                ui.separator();
                ui.weak("Motion and effects");
            }
            _ => {}
        }
        let y0 = ui.cursor().top();
        menu::sub(ui, None, g, |ui| {
            for n in names {
                if menu::row(ui, None, n, "").clicked() {
                    pick = Some(n.to_string());
                }
            }
        });
        kind_dot(ui, y0, g);
    }
    if !presets.is_empty() {
        let y0 = ui.cursor().top();
        menu::sub(ui, None, "Mine", |ui| {
            for b in presets {
                if menu::row(ui, None, &b.name, "").clicked() {
                    pick = Some(b.name.clone());
                }
            }
        });
        kind_dot(ui, y0, "Custom");
    }
    pick
}

/// The kind's colour as a dot in the gutter of the menu row that began at `y0` (the menu is the legend).
fn kind_dot(ui: &Ui, y0: f32, kind: &str) {
    let y1 = ui.cursor().top() - ui.spacing().item_spacing.y;
    let x = ui.min_rect().left() + ui.spacing().button_padding.x + 9.0;
    ui.painter().circle_filled(pos2(x, (y0 + y1) / 2.0), 4.0, kind_color(kind));
}

/// A block as a preset: its controls' result when it has them (so a simulation saves its keys), else its keys.
fn capture(c: &Clip, b: &AppliedBlock, w: f64, h: f64) -> KeyBlock {
    match &b.ctl {
        Some(ctl) if !ctl.edited => {
            let mut k = ctl.resolve(b.dur);
            k.name = format!("{} (mine)", b.name);
            k
        }
        _ => c.capture_block(&format!("{} (mine)", b.name), b.t, b.end(), w, h).unwrap_or_default(),
    }
}

/// "Smooth by easing" and its three choices, left to right, in one row that wraps as a unit.
fn mode_seg(ui: &mut Ui, st: &mut AnimState) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Smooth by easing").size(11.0).weak());
        for (m, n, tip) in [
            (Match::Next, "the next move", "The next move starts at the speed the earlier one ends at"),
            (Match::Earlier, "the earlier move", "The earlier move ends at the speed the next one starts at"),
            (Match::Both, "both", "Both moves change to meet halfway"),
        ] {
            if chip(ui, st.mode == m, n).on_hover_text(tip).clicked() {
                st.mode = m;
            }
        }
    });
}

/// The lanes a clip's blocks sit in, by what they animate.
const GROUPS: [&str; 4] = ["Motion", "Opacity", "Speed", "Sound"];
fn group_of(b: &AppliedBlock) -> usize {
    let only = |set: &[&str]| !b.props.is_empty() && b.props.iter().all(|p| set.contains(&p.as_str()));
    if only(&["speed"]) {
        2
    } else if only(&["volume", "pan"]) {
        3
    } else if only(&["opacity"]) {
        1
    } else {
        0
    }
}

/// Each block's row and each row's label: a lane per kind of property a clip's blocks animate (Motion,
/// Opacity, Speed, Sound - only the ones in use), and inside a lane another row only where two blocks overlap.
fn lane_rows(blocks: &[AppliedBlock]) -> (Vec<usize>, Vec<&'static str>) {
    let mut rows = vec![0; blocks.len()];
    let mut labels = Vec::new();
    for (g, name) in GROUPS.iter().enumerate() {
        let base = labels.len();
        let mut ends: Vec<f64> = Vec::new();
        for (i, b) in blocks.iter().enumerate().filter(|(_, b)| group_of(b) == g) {
            let r = match ends.iter().position(|&e| e <= b.t + 1e-4) {
                Some(r) => {
                    ends[r] = b.end();
                    r
                }
                None => {
                    ends.push(b.end());
                    ends.len() - 1
                }
            };
            rows[i] = base + r;
        }
        labels.extend(std::iter::repeat_n(*name, ends.len()));
    }
    (rows, labels)
}

/// The timeline range the lanes show zoomed all the way out: exactly the selected clips, from the start of
/// the earliest to the end of the latest.
fn range(project: &Project, ids: &[Id]) -> (f64, f64) {
    ids.iter()
        .filter_map(|&id| project.clip(id))
        .fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c.start), b.max(c.end())))
}

/// `w` kept inside `fit`: no longer than it, and slid back in when it hangs over an end.
/// ws:forum-ui: the shown window after the playhead moved from `last` to `ph`: unchanged while the
/// playhead stays in view or did not move; else paged so the playhead sits a tenth in from the left.
pub(crate) fn follow_playhead(w: (f64, f64), last: Option<f64>, ph: f64) -> (f64, f64) {
    let moved = last.is_some_and(|l| (l - ph).abs() > 1e-9);
    if !moved || (w.0..=w.1).contains(&ph) {
        return w;
    }
    let len = w.1 - w.0;
    let a = ph - len * 0.1;
    (a, a + len)
}

fn clamp_win(w: (f64, f64), fit: (f64, f64)) -> (f64, f64) {
    let len = (w.1 - w.0).min(fit.1 - fit.0);
    let a = w.0.clamp(fit.0, (fit.1 - len).max(fit.0));
    (a, a + len)
}

/// The window the lanes open on: the clips' blocks with a little air, never so wide that the shortest block
/// is a sliver (at most 14 of it across), inside `fit`. No blocks: `fit`.
fn block_fit(project: &Project, ids: &[Id], fit: (f64, f64)) -> (f64, f64) {
    let spans: Vec<(f64, f64)> = ids
        .iter()
        .filter_map(|&id| project.clip(id))
        .flat_map(|c| c.blocks.iter().map(move |b| (c.start + b.t, c.start + b.end())))
        // (and the transitions they arrive / leave by)
        .chain(ids.iter().flat_map(|&id| clip_transitions(project, id)).map(|x| x.1))
        .collect();
    if spans.is_empty() {
        return fit;
    }
    let (a, b) = spans.iter().fold((f64::MAX, f64::MIN), |(a, b), s| (a.min(s.0), b.max(s.1)));
    let short = spans.iter().map(|s| s.1 - s.0).fold(f64::MAX, f64::min).max(0.05);
    let pad = ((b - a) * 0.15).max(0.3);
    let (a, b) = (a - pad, (b + pad).min(a - pad + short * 14.0));
    let len = (b - a).min(fit.1 - fit.0);
    let a = a.clamp(fit.0, (fit.1 - len).max(fit.0));
    (a, a + len)
}

/// A keyed block that wiggles and comes back to where it started (a shake, a pulse): it has no side and no
/// origin / target, and its picture is its real curve.
fn wiggles(ctl: &BlockCtl) -> bool {
    let t = &ctl.src.tracks;
    ctl.src.gen.is_none()
        && t.iter().any(|t| t.keys.len() > 2)
        && t.iter().all(|t| t.keys.first().map(|k| k.v) == t.keys.last().map(|k| k.v))
}

fn ruler(ui: &mut Ui, rect: Rect, t0: f64, t1: f64, playhead: &mut f64, pal: &Palette, out: &mut AnimResp) {
    // the range has no air around it: a label on the first / last tick draws inside the ruler instead of
    // hanging over its edge
    let p = ui.painter_at(rect);
    let x = |t: f64| rect.left() + ((t - t0) / (t1 - t0)) as f32 * rect.width();
    for t in ticks(t0, t1, rect.width()) {
        p.line_segment([pos2(x(t), rect.bottom() - 5.0), pos2(x(t), rect.bottom())], Stroke::new(1.0, pal.border));
        let (px, align) = if x(t) - rect.left() < 14.0 {
            (x(t) + 2.0, Align2::LEFT_TOP)
        } else if rect.right() - x(t) < 14.0 {
            (x(t) - 2.0, Align2::RIGHT_TOP)
        } else {
            (x(t), Align2::CENTER_TOP)
        };
        p.text(
            pos2(px, rect.top() + 3.0),
            align,
            format!("{t}s"),
            FontId::proportional(10.0),
            pal.text_dim,
        );
    }
    let r = ui.interact(rect, ui.id().with("anim-ruler"), Sense::click_and_drag());
    if let Some(pos) = r.interact_pointer_pos().filter(|_| r.clicked() || r.dragged()) {
        *playhead = (t0 + ((pos.x - rect.left()) / rect.width()) as f64 * (t1 - t0)).max(0.0);
        out.seeked = true;
    }
}

/// The lanes' zoom, like the timeline's: − slider + and fit. Zooms around the playhead while it is in view.
/// Left of it, the range toggle (Selected clips / Whole timeline): true when it was switched.
fn zoom_bar(ui: &mut Ui, pal: &Palette, win: &mut Option<(f64, f64)>, fit: (f64, f64), playhead: f64, whole: &mut bool) -> bool {
    use crate::ui::tools::{icon_button, Glyph};
    let mut switched = false;
    let (t0, t1) = win.unwrap_or(fit);
    let flen = (fit.1 - fit.0).max(1e-3);
    let z0 = flen / (t1 - t0).max(1e-3);
    let (mut z, mut fitted) = (z0, false);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(4.0);
            fitted = icon_button(ui, pal, ui.id().with("an_fit"), Glyph::Maximize, "Zoom to fit (F)", false).clicked();
            if icon_button(ui, pal, ui.id().with("an_zin"), Glyph::Letter('+'), "Zoom in (Ctrl+wheel)", false).clicked() {
                z *= 1.25;
            }
            ui.spacing_mut().slider_width = 110.0;
            ui.add(egui::Slider::new(&mut z, 1.0..=400.0).logarithmic(true).show_value(false)).on_hover_text("Zoom");
            if icon_button(ui, pal, ui.id().with("an_zout"), Glyph::Letter('−'), "Zoom out", false).clicked() {
                z *= 0.8;
            }
            ui.add_space(6.0);
            // (right to left: the segments are listed last first)
            switched = segmented(ui, whole, &[(true, "Whole timeline"), (false, "Selected clips")]);
        });
    });
    if switched {
        return true;
    }
    if fitted || z <= 1.0 + 1e-6 && z != z0 {
        *win = Some(fit); // the whole range, kept (None would frame the blocks again)
    } else if z != z0 {
        let len = flen / z.min(400.0);
        let at = if (t0..=t1).contains(&playhead) { playhead } else { (t0 + t1) / 2.0 };
        let a = at - (at - t0) / (t1 - t0) * len;
        *win = Some((a, a + len));
    }
    false
}

/// The lanes' horizontal scrollbar: the shown window inside the whole range. Drag the thumb to move through
/// time; a click beside it centres the window there.
fn time_scrollbar(ui: &mut Ui, rect: Rect, win: &mut Option<(f64, f64)>, fit: (f64, f64)) {
    let (t0, t1) = win.unwrap_or(fit);
    let flen = (fit.1 - fit.0).max(1e-6);
    let px = |t: f64| rect.left() + ((t - fit.0) / flen) as f32 * rect.width();
    let thumb = Rect::from_min_max(
        pos2(px(t0).min(rect.right() - 14.0), rect.top() + 2.0),
        pos2(px(t1).max(px(t0) + 14.0).min(rect.right()), rect.bottom() - 2.0),
    );
    let r = ui.interact(rect, ui.id().with("an-hscroll"), Sense::click_and_drag());
    let p = ui.painter();
    p.rect_filled(rect.shrink2(vec2(0.0, 2.0)), 4.0, look::TRACK.gamma_multiply(0.5));
    p.rect_filled(thumb, 4.0, if r.hovered() || r.dragged() { look::MUTED } else { look::DIM });
    if r.dragged() {
        let dt = (r.drag_delta().x / rect.width().max(1.0)) as f64 * flen;
        *win = Some(clamp_win((t0 + dt, t1 + dt), fit));
    } else if let Some(pp) = r.interact_pointer_pos().filter(|pp| r.clicked() && !thumb.contains(*pp)) {
        let mid = fit.0 + ((pp.x - rect.left()) / rect.width().max(1.0)) as f64 * flen;
        *win = Some(clamp_win((mid - (t1 - t0) / 2.0, mid + (t1 - t0) / 2.0), fit));
    }
}

/// "Drop with collisions" on the selected clips: from the playhead when two or more of them are on screen
/// there, else from where the last of them starts.
fn drop_collide(
    project: &mut Project,
    ids: &[Id],
    playhead: f64,
    (bounce, gravity): (f64, f64),
    undo: &mut dyn FnMut(&Project),
    out: &mut AnimResp,
) {
    let pics = || ids.iter().filter_map(|&id| project.clip(id)).filter(|c| c.is_visual());
    let t = if pics().filter(|c| c.contains(playhead)).count() > 1 {
        playhead
    } else {
        pics().map(|c| c.start).fold(0.0, f64::max)
    };
    if pics().filter(|c| c.contains(t)).count() < 2 {
        out.toast = Some("Select two or more pictures that are on screen at the same time".into());
        return;
    }
    undo(project);
    // a plain drop: no throw, no spin (each Collide block's Rigid body controls in the Inspector add them)
    let phys = Phys { bounce, gravity, throw: 0.0, spin: 0.0, ..Phys::default() };
    match project.collide_drop(ids, t, None, &phys) {
        0 => out.toast = Some("Those clips end too soon after that point to drop".into()),
        n => {
            out.toast = Some(format!("Dropped {n} clips: pick a clip's Collide block for its mass, elasticity, velocity and spin"));
            out.play = Some((t, t + 4.0));
        }
    }
    out.edited = true;
}

/// Loop block `i` of clip `id` `n` times (1 = off).
fn loop_act(id: Id, i: usize, n: u32, pingpong: bool) -> Act {
    Box::new(move |p, _, o| {
        if p.loop_key_block(id, i, n, pingpong) {
            o.toast = Some(match (n, pingpong) {
                (1, _) => "Loop off: the block plays once".into(),
                (_, true) => format!("Loops {n} times, back and forth"),
                _ => format!("Loops {n} times"),
            });
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn lanes_view(
    ui: &mut Ui,
    st: &mut AnimState,
    project: &mut Project,
    ids: &[Id],
    rows: &[Id],
    playhead: &mut f64,
    presets: &[KeyBlock],
    pal: &Palette,
    undo: &mut dyn FnMut(&Project),
    out: &mut AnimResp,
) {
    let seams = project.block_seams();
    let (mut fit, mut flow_now, mut link_now, mut smooth_all, mut collide) = ((0.0, 1.0), false, false, false, false);
    st.snap = !st.no_snap;
    let view = st.view;
    // the range the lanes cover when zoomed all the way out: exactly the selected clips, or the timeline (its
    // In-Out range when one is set)
    // (nothing selected: the timeline, for the Timeline lane)
    let range_of = |whole: bool, p: &Project| match (whole || ids.is_empty(), p.in_point, p.out_point) {
        (true, Some(a), Some(b)) if b > a => (a, b),
        (true, ..) => (0.0, p.duration().max(1.0)),
        // exactly the selected clips (`rows` also lists the clips their transitions lead into)
        _ => range(p, ids),
    };
    ui.horizontal(|ui| {
        if view == View::Blocks {
            // the lanes' own tools (the timeline's keys pick them while this panel is the one in use)
            use crate::ui::tools::{icon_button, Glyph};
            for (t, g, tip) in [
                (AnimTool::Select, Glyph::Cursor, "Select (V): click or drag a box to pick blocks, drag one to move it, an end to stretch it"),
                (AnimTool::Cut, Glyph::Razor, "Cut (C): click a block to split it there (Ctrl+B splits the picked block at the playhead)"),
                (AnimTool::Path, Glyph::Pencil, "Path (D): draw in the Viewer and the selected clips travel your stroke, as a Path block from the playhead"),
            ] {
                if icon_button(ui, pal, ui.id().with(("an_tool", t as u8)), g, tip, st.tool == t).clicked() {
                    st.tool = t;
                }
            }
            let tip = "Snapping (S): a dragged block snaps to the playhead, other blocks and clip edges";
            if icon_button(ui, pal, ui.id().with("an_snap"), Glyph::Magnet, tip, st.snap).clicked() {
                st.no_snap = !st.no_snap;
            }
            ui.separator();
        }
        let tip = "Play repeats the range shown here until you stop it (Play in the Animator runs just that range)";
        if chip(ui, st.loop_play, "Loop").on_hover_text(tip).clicked() {
            st.loop_play = !st.loop_play;
        }
        match view {
            View::Blocks => {
                if picked(st).len() > 1 {
                    let tip = "Join the picked blocks. One after another: each starts where the one before ends, at its speed. On layers at the same time: they start and end together";
                    flow_now = primary(ui, "Flow together").on_hover_text(tip).clicked();
                }
                if linkable(st).is_some() {
                    let tip = "Make the picked blocks one block with all their properties: they move, stretch and ease together";
                    link_now = chip(ui, false, "Link into one block").on_hover_text(tip).clicked();
                }
                if ids.iter().filter(|&&id| project.clip(id).is_some_and(|c| c.is_visual())).count() > 1 {
                    let tip = "Physics: the selected pictures fall from where they are, land on the bottom of the frame and push each other apart (more kinds in the ⋯ menu)";
                    collide = chip(ui, false, "Drop with collisions").on_hover_text(tip).clicked();
                }
            }
            View::Curves => {}
            View::Flow => {
                let rough = seams.iter().any(|s| !s.smooth && (ids.contains(&s.left.0) || ids.contains(&s.right.0)));
                let tip = "Match the speeds at every rough hand-off of the selected clips";
                smooth_all = ui.add_enabled(rough, egui::Button::new("Smooth all")).on_hover_text(tip).clicked();
            }
            View::Speed => {
                ui.label(egui::RichText::new("Drag points · double-click adds · Del removes").size(11.0).weak());
            }
        }
        fit = range_of(st.whole, project);
        st.range = fit;
        if st.win_for != rows {
            (st.win, st.win_for) = (None, rows.to_vec());
        }
        st.win = st.win.filter(|w| w.1 > fit.0 && w.0 < fit.1).or_else(|| Some(block_fit(project, rows, fit)));
        st.win = st.win.map(|w| clamp_win(w, fit)); // zoomed all the way out = exactly the range
        if let Some(w) = st.win {
            st.win = Some(clamp_win(follow_playhead(w, st.last_ph, *playhead), fit));
        }
        st.last_ph = Some(*playhead);
        // on the right: the range toggle, then the zoom
        if zoom_bar(ui, pal, &mut st.win, fit, *playhead, &mut st.whole) {
            fit = range_of(st.whole, project);
        st.range = fit;
            st.win = Some(fit); // switching shows the whole of the new range
        }
    });
    if view == View::Curves {
        curves_bar(ui, st, project, ids, undo, out);
    }
    if smooth_all {
        undo(project);
        let mine = seams.iter().filter(|s| ids.contains(&s.left.0) || ids.contains(&s.right.0));
        let n = mine.filter(|s| project.match_seam(s.left, s.right, st.mode).is_ok()).count();
        out.edited = true;
        out.toast = Some(format!("Smoothed {n} hand-off{}", if n == 1 { "" } else { "s" }));
    }
    if collide {
        drop_collide(project, ids, *playhead, (COLLIDE[0].1, COLLIDE[0].2), undo, out);
    }
    let ph = *playhead;
    let (t0, t1) = st.win.unwrap_or(fit);
    // the lanes scroll up and down; under them a bar scrolls the shown time through the range
    let avail = ui.available_rect_before_wrap();
    let head_w = if view == View::Curves { st.ch_w.unwrap_or(CH_W).min(480.0) } else { st.head_w.unwrap_or(HEAD_W) };
    let head_w = head_w.clamp(80.0, (avail.width() * 0.5).max(80.0));
    let view_h = (avail.height() - SCROLL_H - 4.0).max(40.0);
    // Curves / Flow / Speed: a graph per clip; they share the height until one is dragged taller or shorter
    let graph_h = st.graph_h.unwrap_or(((view_h - 26.0) / rows.len().max(1) as f32).floor()).clamp(GRAPH_MIN, 900.0);
    egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(view_h).show(ui, |ui| {
        let full = ui.available_rect_before_wrap();
        let body_x0 = full.left() + head_w;
        let bw = (full.right() - body_x0).max(50.0);
        let x = |t: f64| body_x0 + ((t - t0) / (t1 - t0)) as f32 * bw;
        let pps = bw as f64 / (t1 - t0);
        let (rr, _) = ui.allocate_exact_size(vec2(full.width(), 20.0), Sense::hover());
        ruler(ui, Rect::from_min_max(pos2(body_x0, rr.top()), rr.max), t0, t1, playhead, pal, out);
        let top = rr.bottom();
        let mut act: Option<Act> = flow_now.then(|| flow_act(picked(st), st.mode));
        let mut rects: Vec<(Rect, (Id, usize))> = Vec::new(); // every block bar drawn, for the selection box
        let mut rects_tr: Vec<Rect> = Vec::new(); // every transition bar drawn (a drag on one is not a box)
        let mut heads: Vec<(Rect, Id, usize)> = Vec::new(); // every clip's label cell and how many blocks it has
        let mut drops: Vec<(Rect, Option<Id>)> = Vec::new(); // where a Gallery card lands: a clip's lane, None = Timeline
        // Alt+wheel: taller / shorter lanes, like the timeline's tracks (taken before the list scrolls by it)
        let over = ui.rect_contains_pointer(full);
        let dz = ui.input_mut(|i| if over && i.modifiers.alt { std::mem::take(&mut i.smooth_scroll_delta.y) } else { 0.0 });
        if dz != 0.0 && view == View::Blocks {
            st.lane_h = Some((st.lane_h.unwrap_or(LANE_H) * (1.0 + dz * 0.004)).clamp(22.0, 90.0));
        } else if dz != 0.0 {
            st.graph_h = Some((graph_h * (1.0 + dz * 0.004)).clamp(GRAPH_MIN, 900.0));
        }
        let lane_h = st.lane_h.unwrap_or(LANE_H);
        if link_now {
            act = linkable(st).map(link_act);
        }
        // the Timeline lane, above the clips' own: transition spans (Blocks view, the main timeline). A bar
        // puts its transition on every cut under it; Select moves and stretches it like a block
        let mut span_rects: Vec<(Rect, Id)> = Vec::new(); // every span bar drawn, for the selection box
        if view == View::Blocks && project.editing.is_none() {
            let (row, _) = ui.allocate_exact_size(vec2(full.width(), lane_h), Sense::hover());
            let head = Rect::from_min_max(row.min, pos2(body_x0, row.bottom()));
            let body = Rect::from_min_max(pos2(body_x0, row.top() - 1.0), pos2(full.right(), row.bottom() + 1.0));
            let col = kind_color("Transition");
            {
                let p = ui.painter_at(row.expand(1.0));
                p.line_segment([row.left_bottom(), row.right_bottom()], Stroke::new(1.0, look::LINE));
                p.line_segment([head.right_top(), head.right_bottom()], Stroke::new(1.0, look::LINE));
                p.circle_filled(pos2(head.left() + 11.0, row.center().y), 4.0, col);
                let g = elided(ui, "Timeline", FontId::proportional(12.0), look::TEXT, head.width() - 30.0);
                p.galley(pos2(head.left() + 22.0, row.center().y - g.size().y / 2.0), g, look::TEXT);
            }
            drops.push((body, None));
            ui.interact(head, ui.id().with("span-head"), Sense::hover()).on_hover_text(
                "Transitions over the clips: a bar here puts its transition on every cut under it, wherever you move it. Right-click the lane to add one",
            );
            // right-click the lane's empty space: a span from there (under the bars, so theirs win)
            let lane_bg = ui.interact(body, ui.id().with("span-bg"), Sense::click());
            if let Some(pp) = lane_bg.interact_pointer_pos().filter(|_| lane_bg.secondary_clicked()) {
                st.ctx_t = Some(t0 + ((pp.x - body_x0) / bw) as f64 * (t1 - t0));
            }
            let at = st.ctx_t.unwrap_or(t0).max(0.0);
            menu::context(&lane_bg, |ui| {
                ui.weak("Add transition");
                if let Some(k) = span_kinds(ui) {
                    act = Some(span_add_act(at, ((t1 - t0) / 4.0).max(1.0), k));
                }
            });
            let p = ui.painter_at(body);
            for (si, sp) in project.spans.clone().into_iter().enumerate() {
                let r = Rect::from_min_max(pos2(x(sp.t), row.top() + 5.0), pos2(x(sp.t + sp.dur), row.bottom() - 5.0));
                #[cfg(test)]
                st.span_pos.push((sp.id, r));
                if !r.intersects(body) {
                    continue;
                }
                let sel = st.spans.contains(&sp.id);
                p.rect(
                    r,
                    6.0,
                    look::CARD.lerp_to_gamma(col, if sel { 0.45 } else { 0.24 }),
                    Stroke::new(1.0, if sel { Color32::WHITE } else { col.gamma_multiply(0.75) }),
                    egui::StrokeKind::Inside,
                );
                let name = sp.name();
                p.with_clip_rect(r.shrink(1.0)).text(
                    pos2(r.left() + 8.0, r.center().y),
                    Align2::LEFT_CENTER,
                    &name,
                    FontId::proportional(11.0),
                    Color32::WHITE,
                );
                let hit = r.intersect(body);
                span_rects.push((hit, sp.id));
                let resp = ui.interact(hit, ui.id().with(("aspan", sp.id)), Sense::click_and_drag());
                let hp = resp.hover_pos().or(resp.interact_pointer_pos());
                let edge_at = |p: Pos2| -> i8 {
                    if p.x < r.left() + 6.0 {
                        -1
                    } else if p.x > r.right() - 6.0 {
                        1
                    } else {
                        0
                    }
                };
                let edge = hp.map_or(0, edge_at);
                let resp = resp
                    .on_hover_cursor(if edge != 0 { CursorIcon::ResizeHorizontal } else { CursorIcon::Default })
                    .on_hover_text(format!(
                        "{name} · {:.2} s · on every cut under it. Drag to move, drag an end to stretch, Shift-click to pick more",
                        sp.dur
                    ));
                if resp.clicked() || resp.drag_started() {
                    let more = resp.clicked() && ui.input(|i| i.modifiers.shift || i.modifiers.command);
                    if more {
                        match st.spans.iter().position(|s| *s == sp.id) {
                            Some(j) => drop(st.spans.remove(j)),
                            None => st.spans.push(sp.id),
                        }
                    } else {
                        if !sel || resp.clicked() {
                            st.spans = vec![sp.id];
                        }
                        (st.sel, st.multi) = (None, Vec::new());
                    }
                    st.here = true;
                }
                if resp.drag_started() {
                    undo(project);
                    // (the end held is where the press was, not where the pointer is by now)
                    let held = ui.input(|i| i.pointer.press_origin()).map_or(edge, edge_at);
                    st.span_orig = Some((sp.t, sp.dur, held));
                }
                if resp.dragged() {
                    if let (Some((ot, od, held)), Some(pos), Some(press)) =
                        (st.span_orig, resp.interact_pointer_pos(), ui.input(|i| i.pointer.press_origin()))
                    {
                        let dt = ((pos.x - press.x) / bw) as f64 * (t1 - t0);
                        // snapping on: the playhead, every clip's edges, the other spans' ends
                        let mut cands = Vec::new();
                        if st.snap {
                            cands.push(ph);
                            cands.extend(project.all_clips().flat_map(|(_, o)| [o.start, o.end()]));
                            cands.extend(project.spans.iter().filter(|o| o.id != sp.id).flat_map(|o| [o.t, o.t + o.dur]));
                        }
                        let snap = |t: f64| {
                            cands
                                .iter()
                                .copied()
                                .filter(|c| ((c - t) * pps).abs() < SNAP_PX as f64)
                                .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
                        };
                        let min = crate::model::ops::spans::MIN_SPAN;
                        let (mut bt, mut bd) = (ot, od);
                        st.guide = None;
                        match held {
                            0 => {
                                bt = (ot + dt).max(0.0);
                                if let Some(s) = snap(bt).or_else(|| snap(bt + bd).map(|s| s - bd)) {
                                    st.guide = Some(if snap(bt).is_some() { s } else { s + bd });
                                    bt = s.max(0.0);
                                }
                            }
                            -1 => {
                                bt = (ot + dt).clamp(0.0, ot + od - min);
                                if let Some(s) = snap(bt).filter(|s| *s <= ot + od - min) {
                                    st.guide = Some(s);
                                    bt = s;
                                }
                                bd = ot + od - bt;
                            }
                            _ => {
                                let mut e = (ot + od + dt).max(ot + min);
                                if let Some(s) = snap(e).filter(|s| *s >= ot + min) {
                                    st.guide = Some(s);
                                    e = s;
                                }
                                bd = e - ot;
                            }
                        }
                        if let Some(slot) = project.spans.get_mut(si) {
                            (slot.t, slot.dur) = (bt, bd);
                        }
                    }
                }
                if resp.drag_stopped() {
                    (st.span_orig, st.guide) = (None, None);
                    // the transitions follow on release, not while it is dragged: a bar passing over a gap
                    // would close it on the way
                    out.edited = true;
                }
                menu::context(&resp, |ui| {
                    let id = sp.id;
                    menu::sub(ui, Some(crate::ui::tools::Glyph::Transition), "Transition", |ui| {
                        if let Some(k) = span_kinds(ui) {
                            act = Some(Box::new(move |p, _, _| {
                                if let Some(s) = p.spans.iter_mut().find(|s| s.id == id) {
                                    set_span_kind(s, k);
                                }
                            }));
                        }
                    });
                    if menu::row(ui, Some(crate::ui::tools::Glyph::Cross), "Delete", "").clicked() {
                        act = Some(Box::new(move |p, s, _| {
                            if !s.spans.contains(&id) {
                                s.spans = vec![id];
                            }
                            for id in std::mem::take(&mut s.spans) {
                                p.remove_span(id);
                            }
                        }));
                    }
                });
            }
        }
        for &id in rows {
            let Some(c) = project.clip(id).cloned() else { continue };
            // Curves / Flow / Speed: the clip's row is its graph on the same time axis, its controls in the
            // label column
            if view != View::Blocks {
                let folded = view == View::Curves && st.clip_folded.contains(&id);
                let h = if folded { 26.0 } else { graph_h };
                let (row, _) = ui.allocate_exact_size(vec2(full.width(), h), Sense::hover());
                let head = Rect::from_min_max(row.min, pos2(body_x0, row.bottom()));
                let body = Rect::from_min_max(pos2(body_x0, row.top()), row.max);
                ui.painter_at(body).rect_filled(body, 0.0, look::DEEP);
                drops.push((body, Some(id)));
                match view {
                    View::Curves => curve_row(ui, st, project, &c, head, body, (t0, t1), playhead, pal, undo, out),
                    View::Flow => {
                        flow_row(ui, st, project, &c, ids, &seams, head, body, (t0, t1), playhead, pal, undo, out)
                    }
                    _ => speed_row(ui, st, project, &c, head, body, (t0, t1), playhead, pal, undo, out),
                }
                let p = ui.painter_at(row.expand(1.0));
                p.line_segment([row.left_bottom(), row.right_bottom()], Stroke::new(1.0, look::LINE));
                p.line_segment([head.right_top(), head.right_bottom()], Stroke::new(1.0, look::LINE));
                // the bottom edge of the label cell: drag for taller / shorter graphs (Alt+wheel does it too)
                let grip = Rect::from_min_max(pos2(head.left(), row.bottom() - 3.0), pos2(head.right(), row.bottom() + 3.0));
                let gr = ui
                    .interact(grip, ui.id().with(("an-row-h", id)), Sense::drag())
                    .on_hover_cursor(CursorIcon::ResizeVertical);
                if gr.dragged() && !folded {
                    st.graph_h = Some((graph_h + gr.drag_delta().y).clamp(GRAPH_MIN, 900.0));
                }
                continue;
            }
            // a lane per kind of property (Motion / Opacity / Speed / Sound), more rows only where blocks
            // overlap, and a last row of keys made by hand (click one to make it a block)
            let (pack, labels) = lane_rows(&c.blocks);
            let loose = c.loose_key_spans();
            let keys_row = labels.len();
            // and a row of the timeline transitions it arrives / leaves by (its neighbour's lane shows them too)
            let trs = clip_transitions(project, id);
            let trans_row = labels.len() + (!loose.is_empty()) as usize;
            let nrows = (trans_row + (!trs.is_empty()) as usize).max(1);
            let (row, _) = ui
                .allocate_exact_size(vec2(full.width(), lane_h + (nrows - 1) as f32 * (lane_h - 8.0)), Sense::hover());
            let p = ui.painter_at(row.expand(1.0));
            let on = st.sel.is_some_and(|s| s.0 == id);
            let head = Rect::from_min_max(row.min, pos2(row.left() + head_w, row.bottom()));
            if on {
                p.rect_filled(head, 0.0, pal.accent.gamma_multiply(0.12));
            }
            p.line_segment([row.left_bottom(), row.right_bottom()], Stroke::new(1.0, look::LINE));
            p.line_segment([head.right_top(), head.right_bottom()], Stroke::new(1.0, look::LINE));
            let dot = match c.kind {
                ClipKind::Audio => pal.clip_audio,
                ClipKind::Image => pal.clip_image,
                ClipKind::Text => pal.clip_text,
                ClipKind::Sequence => pal.clip_sequence,
                _ => pal.clip_video,
            };
            let hy = row.top() + lane_h / 2.0;
            p.circle_filled(pos2(head.left() + 11.0, hy), 4.0, dot);
            // [dot][name: fills, truncates with "…"][track tag: fixed, right-aligned] - nothing passes the column
            let tag = project.track_of(id).map_or(String::new(), |t| project.tracks[t].name.clone());
            let tag_g = elided(ui, &tag, FontId::proportional(10.0), look::DIM, 34.0);
            let name_w = head.width() - 22.0 - tag_g.size().x - 14.0;
            let name_g =
                elided(ui, &c.name, FontId::proportional(12.0), if on { Color32::WHITE } else { look::TEXT }, name_w);
            p.galley(pos2(head.left() + 22.0, hy - name_g.size().y / 2.0), name_g, look::TEXT);
            p.galley(pos2(head.right() - 8.0 - tag_g.size().x, hy - tag_g.size().y / 2.0), tag_g, look::DIM);
            // the other rows' names: the lane they are (the first row carries the clip's name)
            for r in 1..nrows {
                let label = if r == trans_row && !trs.is_empty() {
                    "Transitions"
                } else {
                    labels.get(r).copied().unwrap_or("Hand keys")
                };
                let ry = row.top() + lane_h / 2.0 + r as f32 * (lane_h - 8.0);
                let same = r < labels.len() && labels[r] == labels[r - 1];
                if !same {
                    p.text(
                        pos2(head.left() + 22.0, ry),
                        Align2::LEFT_CENTER,
                        label,
                        FontId::proportional(10.5),
                        look::MUTED,
                    );
                }
            }
            let hr = ui.interact(head, ui.id().with(("lane-head", id)), Sense::click()).on_hover_text(format!(
                "{}{}",
                c.name,
                labels.first().map_or(String::new(), |l| format!(" - first lane: {l}"))
            ));
            heads.push((head, id, c.blocks.len()));
            // a click on the name picks every block of the clip (Shift / Ctrl adds them to the pick)
            if hr.clicked() {
                let all = (0..c.blocks.len()).map(|i| (id, i));
                if ui.input(|i| i.modifiers.shift || i.modifiers.command) {
                    let have = picked(st);
                    st.multi.extend(all.filter(|b| !have.contains(b)));
                } else {
                    st.multi = all.skip(1).collect();
                    st.sel = (!c.blocks.is_empty()).then_some((id, 0));
                }
                st.here = true;
            }
            menu::context(&hr, |ui| {
                menu::sub(ui, None, "Add animation", |ui| {
                    if let Some(b) = anim_menu(ui, presets, None).and_then(|n| find_block(presets, &n)) {
                        act = Some(add_act(id, b, None, false));
                    }
                });
                let r = ui
                    .add_enabled_ui(!loose.is_empty(), |ui| menu::row(ui, None, "Turn hand keys into blocks", ""))
                    .inner;
                if r.clicked() {
                    act = Some(Box::new(move |p, _, o| {
                        let n = p.block_from_keys(id, None, "Custom");
                        o.toast = Some(format!("Made {n} block{} from hand keys", if n == 1 { "" } else { "s" }));
                    }));
                }
            });
            // everything in time draws and clicks only inside the lanes, never over the label column when zoomed
            let body = Rect::from_min_max(pos2(body_x0, row.top() - 1.0), pos2(full.right(), row.bottom() + 1.0));
            drops.push((body, Some(id)));
            let p = ui.painter_at(body);
            // right-click the lane's empty space: add an animation where the pointer is (under the blocks, so
            // theirs win)
            let lane_bg = ui.interact(body, ui.id().with(("lane-bg", id)), Sense::click());
            if let Some(pp) = lane_bg.interact_pointer_pos().filter(|_| lane_bg.secondary_clicked()) {
                st.ctx_t = Some(t0 + ((pp.x - body_x0) / bw) as f64 * (t1 - t0) - c.start);
            }
            let at = st.ctx_t.map(|t| t.clamp(0.0, c.duration));
            menu::context(&lane_bg, |ui| {
                ui.weak("Add animation here");
                if let Some(b) = anim_menu(ui, presets, None).and_then(|n| find_block(presets, &n)) {
                    act = Some(add_act(id, b, at, false));
                }
            });
            // keys made by hand: a dashed "Hand keys" bar per cluster; a click makes it a block with controls
            for &(a, b) in &loose {
                let ktop = row.top() + 5.0 + keys_row as f32 * (lane_h - 8.0);
                let kr = Rect::from_min_max(
                    pos2(x(c.start + a) - 4.0, ktop),
                    pos2(x(c.start + b) + 4.0, ktop + lane_h - 10.0),
                );
                if !kr.intersects(body) {
                    continue;
                }
                let mut e = vec![kr.left_top(), kr.right_top(), kr.right_bottom(), kr.left_bottom()];
                e.push(e[0]);
                p.rect_filled(kr, 6.0, pal.keyframe.gamma_multiply(0.08));
                p.extend(egui::Shape::dashed_line(&e, Stroke::new(1.0, pal.keyframe.gamma_multiply(0.7)), 4.0, 3.0));
                p.with_clip_rect(kr.shrink(1.0)).text(
                    pos2(kr.left() + 8.0, kr.center().y),
                    Align2::LEFT_CENTER,
                    "Hand keys · click to make a block",
                    FontId::proportional(10.5),
                    look::TEXT,
                );
                let r = ui
                    .interact(kr.intersect(body), ui.id().with(("loose", id, (a * 1000.0) as i64)), Sense::click())
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .on_hover_text(
                        "Turn these keys into a block with controls (strength, ease, side) - the motion stays the same",
                    );
                if r.clicked() {
                    act = Some(Box::new(move |p, s, o| {
                        if p.block_from_keys(id, Some((a, b)), "Custom") > 0 {
                            s.sel = p
                                .clip(id)
                                .and_then(|c| c.blocks.iter().position(|bl| (bl.t - a).abs() < 1e-6))
                                .map(|i| (id, i));
                            o.toast = Some("Made a block from the hand keys: its controls are in the Inspector".into());
                        }
                    }));
                }
            }
            // the clip's own span: a faint dashed box the blocks live in
            let span = Rect::from_min_max(
                pos2(x(c.start).max(body_x0), row.top() + 4.0),
                pos2(x(c.end()), row.bottom() - 4.0),
            );
            p.rect_filled(span, 4.0, Color32::from_white_alpha(6));
            let mut edge = vec![span.left_top(), span.right_top(), span.right_bottom(), span.left_bottom()];
            edge.push(edge[0]);
            p.extend(egui::Shape::dashed_line(&edge, Stroke::new(1.0, Color32::from_white_alpha(18)), 4.0, 3.0));
            // its timeline transitions: a bar over the window each plays in (a cut's straddles the cut, the
            // neighbour's lane has the same bar). Click picks it (the Inspector shows its controls), an end
            // drags its length, right-click changes its kind
            let col = kind_color("Transition");
            for (tr, (wa, wb), sides) in &trs {
                let top = row.top() + 5.0 + trans_row as f32 * (lane_h - 8.0);
                let r = Rect::from_min_max(pos2(x(*wa), top), pos2(x(*wb).max(x(*wa) + 6.0), top + lane_h - 10.0));
                #[cfg(test)]
                st.trans_pos.push((tr.id, id, r));
                if !r.intersects(body) {
                    continue;
                }
                let sel = st.trans == Some(tr.id);
                p.rect(
                    r,
                    6.0,
                    look::CARD.lerp_to_gamma(col, if sel { 0.45 } else { 0.24 }),
                    Stroke::new(1.0, if sel { Color32::WHITE } else { col.gamma_multiply(0.75) }),
                    egui::StrokeKind::Inside,
                );
                // the cut itself
                if tr.edge == TransitionEdge::Cut {
                    let cx = x((wa + wb) / 2.0);
                    p.line_segment([pos2(cx, r.top() + 2.0), pos2(cx, r.bottom() - 2.0)], Stroke::new(1.0, Color32::from_white_alpha(90)));
                }
                p.with_clip_rect(r.shrink(1.0)).text(
                    pos2(r.left() + 8.0, r.center().y),
                    Align2::LEFT_CENTER,
                    tr.name(),
                    FontId::proportional(11.0),
                    Color32::WHITE,
                );
                let hit = r.intersect(body);
                rects_tr.push(hit);
                let resp = ui.interact(hit, ui.id().with(("atrans", tr.id, id)), Sense::click_and_drag());
                let edge_at = |hp: Pos2| -> i8 {
                    if hp.x < r.left() + 6.0 {
                        -1
                    } else if hp.x > r.right() - 6.0 {
                        1
                    } else {
                        0
                    }
                };
                let edge = resp.hover_pos().or(resp.interact_pointer_pos()).map_or(0, edge_at);
                let resp = resp
                    .on_hover_cursor(if edge != 0 { CursorIcon::ResizeHorizontal } else { CursorIcon::Default })
                    .on_hover_text(format!(
                        "{} · {:.2} s · {sides}. Drag an end to change its length, right-click for its kind",
                        tr.name(),
                        wb - wa
                    ));
                if resp.clicked() || resp.drag_started() {
                    (st.trans, st.sel, st.multi, st.spans) = (Some(tr.id), None, Vec::new(), Vec::new());
                    st.here = true;
                }
                // (the end held is where the press was: the bar grows under the pointer while it drags)
                if resp.drag_started() {
                    st.trans_held = ui.input(|i| i.pointer.press_origin()).map_or(edge, edge_at);
                    if st.trans_held != 0 {
                        undo(project);
                    }
                }
                // an end sets the length: a cut's window grows both ways from the cut, an In / Out one from
                // its clip's edge
                if let Some(pos) = resp.interact_pointer_pos().filter(|_| resp.dragged() && st.trans_held != 0) {
                    let t = t0 + ((pos.x - body_x0) / bw) as f64 * (t1 - t0);
                    let d = match tr.edge {
                        TransitionEdge::Cut => 2.0 * (t - (wa + wb) / 2.0).abs(),
                        TransitionEdge::In => t - wa,
                        TransitionEdge::Out => wb - t,
                    };
                    if let Some(x) = project.transition_mut(tr.id) {
                        x.duration = (d.clamp(0.1, 5.0) * 100.0).round() / 100.0;
                        out.edited = true;
                    }
                }
                menu::context(&resp, |ui| {
                    let tid = tr.id;
                    menu::sub(ui, Some(crate::ui::tools::Glyph::Transition), "Transition", |ui| {
                        if let Some((kind, src)) = span_kinds(ui) {
                            act = Some(Box::new(move |p, s, _| {
                                if let Some(x) = p.transition_mut(tid) {
                                    if kind == TransitionKind::Shader && x.shader != src {
                                        x.params = crate::engine::shaderlib::parse_meta(&src).params.iter().map(|p| p.default).collect();
                                        x.shader = src;
                                    }
                                    x.kind = kind;
                                }
                                s.trans = Some(tid);
                            }));
                        }
                    });
                    if menu::row(ui, Some(crate::ui::tools::Glyph::Cross), "Delete", "").clicked() {
                        act = Some(Box::new(move |p, s, _| {
                            p.remove_transition(tid);
                            s.trans = None;
                        }));
                    }
                });
            }
            for (i, b) in c.blocks.iter().enumerate() {
                let top = row.top() + 5.0 + pack[i] as f32 * (lane_h - 8.0);
                let r =
                    Rect::from_min_max(pos2(x(c.start + b.t), top), pos2(x(c.start + b.end()), top + lane_h - 10.0));
                if !r.intersects(body) {
                    continue;
                }
                let sel = st.sel == Some((id, i)) || st.multi.contains(&(id, i));
                let (_, col) = block_kind(b);
                p.rect(
                    r,
                    6.0,
                    look::CARD.lerp_to_gamma(col, if sel { 0.45 } else { 0.24 }),
                    Stroke::new(1.0, if sel { Color32::WHITE } else { col.gamma_multiply(0.75) }),
                    egui::StrokeKind::Inside,
                );
                let open = st.open.contains(&(id, i));
                let edited = b.ctl.as_ref().is_some_and(|c| c.edited) || b.ctl.is_none();
                let inner = p.with_clip_rect(r.shrink(1.0));
                inner.text(
                    pos2(r.left() + 8.0, r.center().y),
                    Align2::CENTER_CENTER,
                    if open { "▾" } else { "▸" },
                    FontId::proportional(10.0),
                    look::TEXT,
                );
                let mut tx0 = r.left() + 16.0;
                if r.width() > 70.0 {
                    if let Some(a) = b.props.first().and_then(|pr| prop_ref(&c, pr)) {
                        let vs: Vec<f64> = (0..=16).map(|k| a.at(b.t + b.dur * k as f64 / 16.0)).collect();
                        let (lo, hi) = vs.iter().fold((f64::MAX, f64::MIN), |(l, h), &v| (l.min(v), h.max(v)));
                        let g = Rect::from_min_size(pos2(tx0, r.center().y - 6.0), vec2(22.0, 12.0));
                        let pts: Vec<Pos2> = vs
                            .iter()
                            .enumerate()
                            .map(|(k, v)| {
                                let fy = if hi - lo > 1e-9 { ((v - lo) / (hi - lo)) as f32 } else { 0.5 };
                                pos2(g.left() + g.width() * k as f32 / 16.0, g.bottom() - fy * g.height())
                            })
                            .collect();
                        inner.add(egui::Shape::line(pts, Stroke::new(1.3, Color32::WHITE.gamma_multiply(0.8))));
                        tx0 += 26.0;
                    }
                }
                // a looped block: "×3" after its name and a tick where each cycle starts
                let cycles = b.ctl.as_ref().map_or(1, |c| c.cycles());
                let name = if cycles > 1 { format!("{} ×{cycles}", b.name) } else { b.name.clone() };
                inner.text(pos2(tx0, r.center().y), Align2::LEFT_CENTER, name, FontId::proportional(11.0), Color32::WHITE);
                for j in (1..cycles).filter(|_| r.width() / cycles as f32 > 4.0) {
                    let cx = r.left() + r.width() * j as f32 / cycles as f32;
                    inner.line_segment(
                        [pos2(cx, r.top() + 2.0), pos2(cx, r.bottom() - 2.0)],
                        Stroke::new(1.0, Color32::WHITE.gamma_multiply(0.4)),
                    );
                }
                if edited && r.width() > 30.0 {
                    inner.circle_filled(pos2(r.right() - 8.0, r.center().y), 3.0, look::HANDLE);
                }
                rects.push((r.intersect(body), (id, i)));
                if st.tool == AnimTool::Cut {
                    let resp = ui
                        .interact(r.intersect(body), ui.id().with(("ablk-cut", id, i)), Sense::click())
                        .on_hover_cursor(CursorIcon::Crosshair);
                    if let Some(hp) = resp.hover_pos() {
                        ui.painter().line_segment([pos2(hp.x, r.top()), pos2(hp.x, r.bottom())], Stroke::new(1.5, Color32::WHITE));
                    }
                    if let Some(pp) = resp.interact_pointer_pos().filter(|_| resp.clicked()) {
                        let t = t0 + ((pp.x - body_x0) / bw) as f64 * (t1 - t0) - c.start;
                        act = Some(Box::new(move |p, s, o| {
                            if p.split_key_block(id, i, t) {
                                (s.sel, s.multi) = (Some((id, i)), Vec::new());
                            } else {
                                o.toast = Some("Too close to the block's end to cut there".into());
                            }
                        }));
                    }
                    continue;
                }
                let resp = ui.interact(r.intersect(body), ui.id().with(("ablk", id, i)), Sense::click_and_drag());
                let hp = resp.hover_pos().or(resp.interact_pointer_pos());
                let edge = hp.map_or(0, |p| {
                    if p.x < r.left() + 6.0 {
                        -1
                    } else if p.x > r.right() - 6.0 {
                        1
                    } else {
                        0
                    }
                });
                let resp =
                    resp.on_hover_cursor(if edge != 0 { CursorIcon::ResizeHorizontal } else { CursorIcon::Default });
                let resp = resp.on_hover_text(format!(
                    "{} · {:.2} s · drag to move, drag an end to stretch, click ▸ to shape how it moves, Shift-click to pick more",
                    b.name, b.dur
                ));
                if resp.clicked() {
                    let more = ui.input(|i| i.modifiers.shift || i.modifiers.command);
                    if more && st.sel.is_some_and(|s| s != (id, i)) {
                        match st.multi.iter().position(|m| *m == (id, i)) {
                            Some(j) => drop(st.multi.remove(j)),
                            None => st.multi.push((id, i)),
                        }
                    } else {
                        st.sel = Some((id, i));
                        if !more {
                            st.multi.clear();
                            st.spans.clear();
                        }
                    }
                    if !more && hp.is_some_and(|p| p.x < r.left() + 16.0) {
                        if open {
                            st.open.retain(|o| *o != (id, i));
                        } else {
                            st.open.push((id, i));
                        }
                    }
                }
                if resp.drag_started() {
                    st.sel = Some((id, i));
                    undo(project);
                    st.orig = Some(c.clone());
                }
                if resp.dragged() {
                    if let (Some(orig), Some(pos), Some(press)) =
                        (st.orig.clone(), resp.interact_pointer_pos(), ui.input(|i| i.pointer.press_origin()))
                    {
                        let ob = &orig.blocks[i];
                        let dt = ((pos.x - press.x) / bw) as f64 * (t1 - t0);
                        let (mut bt, mut bd) = (ob.t, ob.dur);
                        // snapping on: the playhead, the other blocks, and every clip's edges on the timeline
                        let mut cands = Vec::new();
                        if st.snap {
                            cands.push(ph - orig.start);
                            cands.extend(
                                orig.blocks.iter().enumerate().filter(|(j, _)| *j != i).flat_map(|(_, o)| [o.t, o.end()]),
                            );
                            cands.extend(project.all_clips().flat_map(|(_, o)| [o.start - orig.start, o.end() - orig.start]));
                        }
                        let snap = |t: f64| {
                            cands
                                .iter()
                                .copied()
                                .filter(|c| ((c - t) * pps).abs() < SNAP_PX as f64)
                                .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
                        };
                        st.guide = None;
                        match edge {
                            0 => {
                                bt = ob.t + dt;
                                if let Some(s) = snap(bt).or_else(|| snap(bt + bd).map(|s| s - bd)) {
                                    st.guide = Some(orig.start + if snap(bt).is_some() { s } else { s + bd });
                                    bt = s;
                                }
                            }
                            -1 => {
                                bt = (ob.t + dt).min(ob.end() - 0.05);
                                if let Some(s) = snap(bt) {
                                    st.guide = Some(orig.start + s);
                                    bt = s;
                                }
                                bd = ob.end() - bt;
                            }
                            _ => {
                                let mut e = (ob.end() + dt).max(ob.t + 0.05);
                                if let Some(s) = snap(e) {
                                    st.guide = Some(orig.start + s);
                                    e = s;
                                }
                                bd = e - ob.t;
                            }
                        }
                        if let Some(slot) = project.clip_mut(id) {
                            *slot = orig;
                        }
                        project.retime_key_block(id, i, bt, bd);
                        out.edited = true;
                    }
                }
                if resp.drag_stopped() {
                    st.orig = None;
                    st.guide = None;
                }
                menu::context(&resp, |ui| {
                    menu::sub(ui, Some(crate::ui::tools::Glyph::CurveIcon), "Ease", |ui| {
                        for (n, e) in Ease::ALL.iter().map(|e| (e.name(), *e)).chain(Ease::PRESETS) {
                            if menu::row(ui, None, n, "").clicked() {
                                act = Some(Box::new(move |p, _, _| {
                                    p.ease_key_block(id, i, e);
                                }));
                            }
                        }
                    });
                    let cur = b.ctl.as_ref().map_or((1, false), |c| (c.cycles(), c.pingpong));
                    menu::sub(ui, None, "Loop", |ui| {
                        for n in [1u32, 2, 3, 4, 8, 16] {
                            let label = if n == 1 { "Off".to_string() } else { format!("{n} times") };
                            if menu::check(ui, cur.0 == n, &label, "").clicked() {
                                act = Some(loop_act(id, i, n, cur.1));
                            }
                        }
                        ui.separator();
                        if menu::check(ui, cur.1, "Ping-pong", "").clicked() {
                            act = Some(loop_act(id, i, cur.0.max(2), !cur.1));
                        }
                    });
                    if menu::row(ui, None, "Split at playhead", "Ctrl+B").clicked() {
                        let t = ph - c.start;
                        act = Some(Box::new(move |p, s, o| {
                            s.multi.clear();
                            if !p.split_key_block(id, i, t) {
                                o.toast = Some("Move the playhead inside the block to split it there".into());
                            }
                        }));
                    }
                    if picked(st).len() > 1 && menu::row(ui, None, "Flow together", "").clicked() {
                        act = Some(flow_act(picked(st), st.mode));
                    }
                    if let Some(v) = linkable(st) {
                        if menu::row(ui, None, "Link into one block", "").clicked() {
                            act = Some(link_act(v));
                        }
                    }
                    if b.ctl.as_ref().is_some_and(|c| c.edited)
                        && menu::row(ui, None, "Reset to its controls", "").clicked()
                    {
                        let ctl = b.ctl.clone().unwrap();
                        act = Some(Box::new(move |p, _, _| {
                            p.set_block_ctl(id, i, ctl);
                        }));
                    }
                    if menu::row(ui, None, "Save as preset", "").clicked() {
                        let k = capture(&c, b, 1920.0, 1080.0);
                        act = Some(Box::new(move |_, _, o| o.save = Some(k)));
                    }
                    if menu::row(ui, Some(crate::ui::tools::Glyph::Cross), "Delete", "").clicked() {
                        act = Some(Box::new(move |p, s, _| {
                            if !picked(s).contains(&(id, i)) {
                                (s.sel, s.multi) = (Some((id, i)), Vec::new());
                            }
                            remove_picked(p, s);
                        }));
                    }
                });
            }
            // hand-offs on this lane: ⚡ a speed jump, ✓ smooth
            for s in seams.iter().filter(|s| s.left.0 == id || s.right.0 == id) {
                if s.left.0 != id && project.clip(s.left.0).is_some() && ids.contains(&s.left.0) {
                    continue; // drawn on the earlier clip's lane already
                }
                let c0 = pos2(x(s.t), row.top() + lane_h / 2.0);
                let br = Rect::from_center_size(c0, vec2(16.0, 16.0));
                if !body.contains(c0) {
                    continue;
                }
                let col = if s.smooth { GOOD } else { BAD };
                p.circle_filled(c0, 8.0, col);
                p.circle_stroke(c0, 8.0, Stroke::new(2.0, pal.panel));
                p.text(
                    c0,
                    Align2::CENTER_CENTER,
                    if s.smooth { "✓" } else { "⚡" },
                    FontId::proportional(10.0),
                    Color32::WHITE,
                );
                let r =
                    ui.interact(br, ui.id().with(("seam", s.left, s.right)), Sense::click()).on_hover_text(format!(
                        "{}: ends at {} · the next move starts at {}{}",
                        prop_label(&s.prop),
                        speed_text(&s.prop, s.v_left),
                        speed_text(&s.prop, s.v_right),
                        if s.smooth { " (smooth)" } else { " - click to match the speeds" }
                    ));
                if r.clicked() {
                    let (l, rr, mode, t) = (s.left, s.right, st.mode, s.t);
                    act = Some(Box::new(move |p, _, o| match p.match_seam(l, rr, mode) {
                        Ok(v) => {
                            o.toast = Some(format!("Speeds matched at {v:.0}/s"));
                            o.play = Some(((t - 0.8).max(0.0), t + 0.9));
                        }
                        Err(e) => o.toast = Some(e),
                    }));
                }
            }
            // open blocks: the move as a string from its origin (bottom) to its target (top). No keys here (Curves
            // has them): pull the two middle points to shape the ease, an end point sideways to stretch it
            for (i, b) in c.blocks.iter().enumerate().filter(|(i, _)| st.open.contains(&(id, *i))) {
                let (sub, _) = ui.allocate_exact_size(vec2(full.width(), STRING_H), Sense::hover());
                ui.painter_at(sub).rect_filled(sub, 0.0, look::DEEP);
                let live = b.ctl.as_ref().filter(|c| !c.edited && !c.is_sim() && !wiggles(c));
                let (xa, xb) = (x(c.start + b.t), x(c.start + b.end()));
                let (y0, y1) = (sub.bottom() - 18.0, sub.top() + 18.0);
                let at = |fx: f32, fy: f32| pos2(xa + fx * (xb - xa), y0 + fy * (y1 - y0));
                let pts: Vec<Pos2> = match live {
                    Some(ctl) => {
                        let e = current_ease(ctl);
                        (0..=48).map(|k| at(k as f32 / 48.0, e.apply(k as f64 / 48.0) as f32)).collect()
                    }
                    // simulated or edited by hand: what its first property really does
                    None => {
                        let a = b.props.first().and_then(|pr| prop_ref(&c, pr));
                        let vs: Vec<f64> =
                            a.map_or(Vec::new(), |a| (0..=48).map(|k| a.at(b.t + b.dur * k as f64 / 48.0)).collect());
                        let (lo, hi) = vs.iter().fold((f64::MAX, f64::MIN), |(l, h), &v| (l.min(v), h.max(v)));
                        let fy = |v: f64| if hi - lo > 1e-9 { ((v - lo) / (hi - lo)) as f32 } else { 0.5 };
                        vs.iter().enumerate().map(|(k, &v)| at(k as f32 / 48.0, fy(v))).collect()
                    }
                };
                let (Some(&first), Some(&last)) = (pts.first(), pts.last()) else { continue };
                let sb = Rect::from_min_max(pos2(body_x0, sub.top()), sub.max);
                let p = ui.painter_at(sb);
                for (y, name) in [(y0, "origin"), (y1, "target")] {
                    p.line_segment([pos2(xa, y), pos2(xb, y)], Stroke::new(1.0, look::LINE));
                    ui.painter_at(sub).text(
                        pos2(sub.left() + 22.0, y),
                        Align2::LEFT_CENTER,
                        name,
                        FontId::proportional(10.5),
                        look::MUTED,
                    );
                }
                p.add(egui::Shape::line(pts, Stroke::new(2.0, block_kind(b).1)));
                let key = ("astr", id, i);
                if let Some(ctl) = live {
                    let (x1, y1h, x2, y2h) = current_ease(ctl).handles();
                    for (k, hx, hy, from) in [(1u8, x1, y1h, first), (2, x2, y2h, last)] {
                        let hc = at(hx, hy);
                        p.line_segment([from, hc], Stroke::new(1.0, look::MUTED));
                        p.circle_filled(hc, 5.0, look::HANDLE);
                        let hr = ui
                            .interact(Rect::from_center_size(hc, vec2(14.0, 14.0)).intersect(sb), ui.id().with((key, k)), Sense::drag())
                            .on_hover_cursor(CursorIcon::Grab)
                            .on_hover_text("Pull to shape how the block moves from its origin to its target");
                        if hr.drag_started() {
                            undo(project);
                        }
                        if let Some(pp) = hr.interact_pointer_pos().filter(|_| hr.dragged()) {
                            let fx = ((pp.x - xa) / (xb - xa).max(1.0)).clamp(0.0, 1.0);
                            let fy = ((pp.y - y0) / (y1 - y0)).clamp(-0.4, 1.4);
                            let mut ctl = ctl.clone();
                            // the pulled end is the user's now, not the speed Flow gave it
                            if k == 1 {
                                (ctl.ease, ctl.in_speed) = (Some(Ease::Bezier { x1: fx, y1: fy, x2, y2: y2h }), None);
                            } else {
                                (ctl.ease, ctl.out_speed) = (Some(Ease::Bezier { x1, y1: y1h, x2: fx, y2: fy }), None);
                            }
                            project.set_block_ctl(id, i, ctl);
                            out.edited = true;
                        }
                    }
                }
                for (k, hc) in [(0u8, first), (3, last)] {
                    p.circle(hc, 5.0, Color32::WHITE, Stroke::new(1.5, Color32::BLACK));
                    let hr = ui
                        .interact(Rect::from_center_size(hc, vec2(14.0, 14.0)).intersect(sb), ui.id().with((key, k)), Sense::drag())
                        .on_hover_cursor(CursorIcon::ResizeHorizontal)
                        .on_hover_text(if k == 0 { "Origin: pull sideways to start earlier or later" } else { "Target: pull sideways to end earlier or later" });
                    if hr.drag_started() {
                        undo(project);
                    }
                    let dt = hr.drag_delta().x as f64 / pps;
                    if hr.dragged() && dt != 0.0 {
                        let (t, d) = if k == 0 {
                            let t = (b.t + dt).clamp(0.0, b.end() - 0.05);
                            (t, b.end() - t)
                        } else {
                            (b.t, b.dur + dt)
                        };
                        out.edited |= project.retime_key_block(id, i, t, d);
                    }
                }
            }
        }
        let bottom = ui.min_rect().bottom();
        // Ctrl+wheel zooms the lanes' time, Shift+wheel / sideways scroll pans, F / Home fits (plain wheel scrolls lanes)
        let lanes_rect = Rect::from_min_max(pos2(body_x0, full.top()), pos2(full.right(), bottom.max(full.top() + 40.0)));
        let bg = ui.interact(lanes_rect, ui.id().with("lanes-pan"), Sense::hover());
        if let Some(w) = wheel_zoom(ui, &bg, lanes_rect, (t0, t1), true) {
            st.win = Some(w);
        }
        if ui.rect_contains_pointer(lanes_rect) && ui.input(|i| i.key_pressed(egui::Key::F) || i.key_pressed(egui::Key::Home)) {
            st.win = None;
        }
        // a drag that starts on empty lane space is a selection box, like the timeline's: every block it
        // touches is picked
        let (press, cur, down) =
            ui.input(|i| (i.pointer.press_origin(), i.pointer.latest_pos(), i.pointer.primary_down()));
        match (press, cur, down) {
            (Some(o), Some(c), true) if view == View::Blocks => {
                let in_head = o.x < body_x0 - 3.0 && heads.iter().any(|h| h.0.contains(o));
                let on_bar = rects.iter().any(|r| r.0.contains(o)) || span_rects.iter().any(|r| r.0.contains(o))
                    || rects_tr.iter().any(|r| r.contains(o));
                let free = in_head || lanes_rect.contains(o) && o.y > top && !on_bar;
                if st.lasso.is_none() && free && (c - o).length() > 4.0 && ui.ctx().dragged_id().is_none() {
                    st.lasso = Some(o);
                }
                if let Some(o) = st.lasso {
                    let r = Rect::from_two_pos(o, c);
                    ui.painter().rect(
                        r.intersect(full),
                        0.0,
                        pal.accent.gamma_multiply(0.12),
                        Stroke::new(1.0, pal.accent),
                        egui::StrokeKind::Inside,
                    );
                    // dragged down the names: every block of the clips it crosses; in the lanes: the blocks it touches
                    let hit: Vec<(Id, usize)> = if o.x < body_x0 - 3.0 {
                        let rows = heads.iter().filter(|h| h.0.y_range().intersects(r.y_range()));
                        rows.flat_map(|h| (0..h.2).map(move |i| (h.1, i))).collect()
                    } else {
                        rects.iter().filter(|b| b.0.intersects(r)).map(|b| b.1).collect()
                    };
                    st.sel = hit.first().copied();
                    st.multi = hit.into_iter().skip(1).collect();
                    // (and the spans it touches, in the lanes)
                    st.spans = span_rects.iter().filter(|b| o.x >= body_x0 - 3.0 && b.0.intersects(r)).map(|b| b.1).collect();
                    st.here = true;
                }
            }
            _ => st.lasso = None,
        }
        // left of the lanes only: a key at the very start of the range stays clickable
        let split = Rect::from_min_max(pos2(body_x0 - 6.0, top), pos2(body_x0, bottom));
        let sr =
            ui.interact(split, ui.id().with("lane-split"), Sense::drag()).on_hover_cursor(CursorIcon::ResizeHorizontal);
        if sr.dragged() {
            let w = Some((head_w + sr.drag_delta().x).clamp(80.0, (full.width() * 0.5).max(80.0)));
            *(if view == View::Curves { &mut st.ch_w } else { &mut st.head_w }) = w;
        }
        let p = ui.painter();
        if sr.hovered() || sr.dragged() {
            p.line_segment([pos2(body_x0, top), pos2(body_x0, bottom)], Stroke::new(2.0, pal.accent));
        }
        if (t0..=t1).contains(playhead) {
            p.line_segment(
                [pos2(x(*playhead), top - 20.0), pos2(x(*playhead), bottom)],
                Stroke::new(1.0, pal.playhead),
            );
        }
        if let Some(g) = st.guide {
            p.line_segment(
                [pos2(x(g), top), pos2(x(g), bottom)],
                Stroke::new(1.0, Color32::from_rgb(0xF2, 0xC1, 0x4E)),
            );
        }
        // a Gallery card dragged over the lanes: a block onto a clip's lane, a transition onto the Timeline lane
        #[cfg(test)]
        st.drop_pos.clone_from(&drops);
        let pos = ui.input(|i| i.pointer.latest_pos()).filter(|p| ui.clip_rect().contains(*p));
        let at = pos.and_then(|p| drops.iter().find(|d| d.0.contains(p)).map(|d| (p, d.0, d.1)));
        if let (Some(pl), Some((pos, lane, on))) = (egui::DragAndDrop::payload::<crate::ui::DragPayload>(ui.ctx()), at) {
            let t = t0 + ((pos.x - body_x0) / bw) as f64 * (t1 - t0);
            if let Some((dur, drop)) = card_drop(project, presets, &pl, on, t, ((t1 - t0) / 4.0).max(1.0)) {
                // where it lands and a ghost of its length
                let p = ui.painter_at(lane);
                let g = Rect::from_min_max(pos2(x(t), lane.top() + 3.0), pos2(x(t + dur), lane.bottom() - 3.0));
                p.rect(g, 4.0, pal.accent.gamma_multiply(0.25), Stroke::new(1.0, pal.accent), egui::StrokeKind::Inside);
                p.line_segment([pos2(x(t), lane.top()), pos2(x(t), lane.bottom())], Stroke::new(2.0, pal.accent));
                ui.ctx().set_cursor_icon(CursorIcon::Copy);
                if ui.input(|i| i.pointer.any_released()) {
                    egui::DragAndDrop::clear_payload(ui.ctx());
                    act = Some(drop);
                    st.here = true;
                }
            }
        }
        if let Some(f) = act {
            undo(project);
            f(project, st, out);
            out.edited = true;
        }
    });
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), SCROLL_H), Sense::hover());
    time_scrollbar(ui, Rect::from_min_max(pos2(bar.left() + head_w, bar.top()), bar.max), &mut st.win, fit);
}

/// What a Gallery card dropped at timeline time `t` on lane `on` (a clip's; None = the Timeline lane) does:
/// its length (for the ghost) and the edit. A block lands on that clip from `t`, a transition on the Timeline
/// lane is a span of `span` seconds from `t`; anything else has no place here (None).
fn card_drop(
    project: &Project,
    presets: &[KeyBlock],
    pl: &crate::ui::DragPayload,
    on: Option<Id>,
    t: f64,
    span: f64,
) -> Option<(f64, Act)> {
    use crate::ui::DragPayload as D;
    match (pl, on) {
        (D::KeyBlock(name), Some(id)) => {
            let b = find_block(presets, name)?;
            let c = project.clip(id)?;
            let at = (t - c.start).clamp(0.0, c.duration);
            Some((b.dur, add_act(id, b, Some(at), true)))
        }
        (D::Transition(k), None) if project.editing.is_none() => {
            Some((span, span_add_act(t.max(0.0), span, (*k, String::new()))))
        }
        _ => None,
    }
}

// ---------- curves ----------

/// The keys a curve shows: the selected block's span on that property (the whole clip with no block).
fn curve_span(c: &Clip, sel: Option<usize>) -> (f64, f64) {
    match sel.and_then(|i| c.blocks.get(i)) {
        Some(b) => (b.t, b.end()),
        None => (0.0, c.duration),
    }
}

/// The channel list: the properties a clip can animate, in groups (Blender's dope / graph sidebar).
const CHANNELS: [(&str, &[&str]); 4] = [
    ("Transform", &["x", "y", "scale", "scale_x", "scale_y", "rotation"]),
    ("Opacity", &["opacity"]),
    ("Sound", &["volume", "pan"]),
    ("Speed", &["speed"]),
];

/// The channel groups and their properties in the user's order (`AnimState::ch_order`; names it does not
/// list keep their `CHANNELS` place after the listed ones).
fn ch_groups(order: &[&'static str]) -> Vec<(&'static str, Vec<&'static str>)> {
    let at = |n: &str| order.iter().position(|o| *o == n).unwrap_or(usize::MAX);
    let mut groups: Vec<(&'static str, Vec<&'static str>)> = CHANNELS.iter().map(|(g, ps)| (*g, ps.to_vec())).collect();
    groups.sort_by_key(|g| at(g.0)); // stable: unlisted ones stay in CHANNELS order
    for g in &mut groups {
        g.1.sort_by_key(|p| at(p));
    }
    groups
}

/// The order after a drag or Alt+arrow in the channel list. `rows` = what each drawn row was: (its group's
/// index in `groups`, the property; None = the group's own row). A group moves among the groups, a property
/// inside its group.
fn ch_moved(
    mut groups: Vec<(&'static str, Vec<&'static str>)>,
    rows: &[(usize, Option<&'static str>)],
    mv: &crate::ui::list_rows::Moved,
) -> Vec<&'static str> {
    use crate::ui::list_rows::Moved;
    // the rows of the same kind as `from` (the group rows, or its group's property rows), and where it is
    let peers = |from: usize| -> Vec<usize> {
        let (g, p) = rows[from];
        (0..rows.len()).filter(|&i| rows[i].1.is_some() == p.is_some() && (p.is_none() || rows[i].0 == g)).collect()
    };
    // row `from` goes to place `to` among its peers
    let mut place = |from: usize, to: usize| {
        let ps = peers(from);
        let (g, p) = rows[from];
        match p {
            None => {
                let it = groups.remove(g);
                // before the group that is now at that place (the removed one no longer counts)
                let others: Vec<usize> = ps.iter().filter(|&&i| i != from).map(|&i| rows[i].0).collect();
                let at = others.get(to).map_or(groups.len(), |&gi| if gi > g { gi - 1 } else { gi });
                groups.insert(at, it);
            }
            Some(p) => {
                let list = &mut groups[g].1;
                list.retain(|q| *q != p);
                let others: Vec<&str> = ps.iter().filter(|&&i| i != from).filter_map(|&i| rows[i].1).collect();
                let at = others.get(to).and_then(|n| list.iter().position(|q| q == n)).unwrap_or(list.len());
                list.insert(at, p);
            }
        }
    };
    match *mv {
        Moved::Slot { from, slot, .. } if from < rows.len() => {
            // its place among its peers: how many of them are drawn above the slot
            let to = peers(from).iter().filter(|&&i| i != from && i < slot).count();
            place(from, to);
        }
        Moved::Step { from, delta } if from < rows.len() => {
            let ps = peers(from);
            let cur = ps.iter().position(|&i| i == from).unwrap_or(0) as i32;
            place(from, (cur + delta).clamp(0, ps.len() as i32 - 1) as usize);
        }
        _ => {}
    }
    groups.iter().flat_map(|(g, ps)| std::iter::once(*g).chain(ps.iter().copied())).collect()
}

/// A key edited by hand inside a block pauses that block's controls (they would rewrite the key).
fn touch_blocks(c: &mut Clip, p: &str, t: f64) {
    for b in &mut c.blocks {
        if b.props.iter().any(|q| q == p) && t >= b.t - 1e-4 && t <= b.end() + 1e-4 {
            if let Some(ctl) = b.ctl.as_mut() {
                ctl.edited = true;
            }
        }
    }
}

/// `win` zoomed by `zoom` (< 1 = in) around time `at`, then moved by `pan` seconds.
pub(crate) fn zoom_pan(win: (f64, f64), at: f64, zoom: f64, pan: f64) -> (f64, f64) {
    let span = ((win.1 - win.0) * zoom).clamp(0.05, 3600.0);
    let f = ((at - win.0) / (win.1 - win.0).max(1e-9)).clamp(0.0, 1.0);
    let a = at - span * f + pan;
    (a, a + span)
}

/// Mouse wheel zooms (or Ctrl+wheel with `ctrl_only`), Shift+wheel / a sideways scroll pans, middle-drag pans.
/// Returns the new window when the input changed it.
fn wheel_zoom(ui: &Ui, resp: &egui::Response, rect: Rect, win: (f64, f64), ctrl_only: bool) -> Option<(f64, f64)> {
    let pos = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p))?;
    let (sd, zd) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
    let at = win.0 + ((pos.x - rect.left()) / rect.width().max(1.0)) as f64 * (win.1 - win.0);
    let per_px = (win.1 - win.0) / rect.width().max(1.0) as f64;
    let mut zoom = 1.0 / zd as f64;
    if !ctrl_only && sd.y != 0.0 {
        zoom *= (-sd.y as f64 * 0.0025).exp();
    }
    let mut pan = -sd.x as f64 * per_px;
    if resp.dragged_by(egui::PointerButton::Middle) {
        pan -= resp.drag_delta().x as f64 * per_px;
    }
    ((zoom - 1.0).abs() > 1e-6 || pan.abs() > 1e-9).then(|| zoom_pan(win, at, zoom, pan))
}

/// What a right-click on the graph asked for.
enum KeyAct {
    Ease(Ease),
    Delete,
    Loop(u32),
    SelectProp(&'static str),
    SelectAll,
    AddAt(&'static str, f64),
    Fit,
}

/// What Curves shows of a clip: the picked block (None = every key of the clip), its span, the properties the
/// clip can animate and the ones drawn on its graph.
struct CurveCtx {
    bi: Option<usize>,
    span: (f64, f64),
    channels: Vec<&'static str>,
    shown: Vec<&'static str>,
}

fn curve_ctx(st: &AnimState, c: &Clip) -> CurveCtx {
    let bi = st.sel.filter(|s| s.0 == c.id && !st.whole_clip).map(|s| s.1).filter(|&i| i < c.blocks.len());
    // what this clip can animate (sound props only on a sound clip), and what the graph shows of it
    // (in the channel list's order, which is also the order the curves are drawn in)
    let channels: Vec<&'static str> = ch_groups(&st.ch_order)
        .into_iter()
        .flat_map(|g| g.1)
        .filter(|p| prop_ref(c, p).is_some())
        .filter(|p| matches!(*p, "speed") || matches!(*p, "volume" | "pan") != c.is_visual())
        .collect();
    let shown = channels
        .iter()
        .copied()
        .filter(|p| !st.hidden.contains(p) && !prop_off(st, p) && prop_ref(c, p).is_some_and(|a| a.is_animated()))
        .filter(|p| bi.is_none_or(|i| c.blocks[i].props.iter().any(|q| q == p)))
        .collect();
    CurveCtx { bi, span: curve_span(c, bi), channels, shown }
}

/// "N more times" rows (the Loop menus of the selected keys): the count picked.
fn loop_rows(ui: &mut Ui) -> Option<u32> {
    let mut pick = None;
    for n in [1u32, 2, 3, 4, 8, 16] {
        if menu::row(ui, None, &format!("{n} more time{}", if n == 1 { "" } else { "s" }), "").clicked() {
            pick = Some(n);
        }
    }
    pick
}

/// Repeat the selected keys (of clip `id`) `n` more times after the last of them. Returns what to tell the user.
fn loop_sel_keys(project: &mut Project, st: &AnimState, id: Id, n: u32) -> String {
    let Some(c) = project.clip(id) else { return String::new() };
    let ts = st.keys.iter().filter_map(|&(p, k)| prop_ref(c, p).and_then(|a| a.keys.get(k)).map(|k| k.t));
    let (a, b) = ts.fold((f64::MAX, f64::MIN), |(a, b), t| (a.min(t), b.max(t)));
    let mut props: Vec<String> = st.keys.iter().map(|k| k.0.to_string()).collect();
    props.sort();
    props.dedup();
    match project.loop_keys(id, &props, a, b, n) {
        0 => "Pick two or more keys of a property (its first and last) to loop what is between them".into(),
        _ => format!("Looped the selected keys {n} more time{}", if n == 1 { "" } else { "s" }),
    }
}

/// Curves' own controls, in a row under the timeline's: Whole clip, Value / Velocity, the ease presets, Loop.
fn curves_bar(
    ui: &mut Ui,
    st: &mut AnimState,
    project: &mut Project,
    ids: &[Id],
    undo: &mut dyn FnMut(&Project),
    out: &mut AnimResp,
) {
    ui.horizontal(|ui| {
        let tip = "Show every key of the clip, not just the picked block's";
        if st.sel.is_some() && chip(ui, st.whole_clip, "Clip keys").on_hover_text(tip).clicked() {
            st.whole_clip = !st.whole_clip;
        }
        segmented(ui, &mut st.velocity, &[(false, "Value"), (true, "Velocity")]);
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Ease").size(11.0).color(look::MUTED))
            .on_hover_text("Sets the ease of the picked keys, or of the keys shown when none are picked");
        for (n, e) in Ease::PRESETS.into_iter().take(5).chain([("Linear", Ease::Linear), ("Hold", Ease::Hold)]) {
            if !ease_chip(ui, false, n, e).clicked() {
                continue;
            }
            undo(project);
            for &id in ids {
                // keys picked on one clip: only they change
                if !st.keys.is_empty() && st.key_clip != Some(id) {
                    continue;
                }
                let Some(c) = project.clip(id).cloned() else { continue };
                let cx = curve_ctx(st, &c);
                let (s0, s1) = cx.span;
                // no keys picked: the whole block, through its Ease control when it has one (so it stays live)
                let live = |i: &usize| c.blocks[*i].ctl.as_ref().is_some_and(|c| !c.edited && !c.is_sim());
                if let Some(i) = cx.bi.filter(|i| st.keys.is_empty() && live(i)) {
                    project.ease_key_block(id, i, e);
                } else if let Some(cm) = project.clip_mut(id) {
                    for &p in &cx.shown {
                        let Some(a) = prop_mut(cm, p) else { continue };
                        let mut hit = Vec::new();
                        for k in 0..a.keys.len() {
                            let t = a.keys[k].t;
                            if (st.keys.is_empty() && t >= s0 - 1e-4 && t < s1 - 1e-4) || st.keys.contains(&(p, k)) {
                                let k = ease_slot(a, k, s1);
                                a.keys[k].ease = e;
                                hit.push(a.keys[k].t);
                            }
                        }
                        for t in hit {
                            touch_blocks(cm, p, t);
                        }
                    }
                }
            }
            out.edited = true;
        }
        // the selected keys: repeat them, or make them a block
        let Some(id) = st.key_clip.filter(|_| st.keys.len() > 1) else { return };
        let r = menu::button(ui, "Loop keys", loop_rows);
        r.response.on_hover_text("Repeat the selected keys after the last of them");
        if let Some(n) = r.inner.flatten() {
            undo(project);
            out.toast = Some(loop_sel_keys(project, st, id, n));
            out.edited = true;
        }
        let Some(c) = project.clip(id) else { return };
        if curve_ctx(st, c).bi.is_none() {
            let ts = st.keys.iter().filter_map(|&(p, k)| prop_ref(c, p).and_then(|a| a.keys.get(k)).map(|k| k.t));
            let (a, b) = ts.fold((f64::MAX, f64::MIN), |(a, b), t| (a.min(t), b.max(t)));
            if b > a && chip(ui, false, "Make a block from the selected keys").clicked() {
                undo(project);
                if project.block_from_keys(id, Some((a, b)), "Custom") > 0 {
                    st.keys.clear();
                    out.toast = Some("Made a block: its controls are in the Inspector".into());
                    out.edited = true;
                } else {
                    out.toast = Some("Those keys already belong to a block".into());
                }
            }
        }
    });
}

/// A graph row's label cell: the clip's dot and "<what the row is> - <clip>" on its first line; returns the
/// rest of the cell. `open` = a fold caret before the dot (▼ open, ▶ folded): its click, when it has one.
fn row_title(ui: &Ui, head: Rect, c: &Clip, what: &str, pal: &Palette, open: Option<bool>) -> (Rect, Option<egui::Response>) {
    let p = ui.painter_at(head);
    let y = head.top() + 12.0;
    let caret = open.map(|o| {
        let r = Rect::from_center_size(pos2(head.left() + 9.0, y), vec2(16.0, 18.0));
        let hint = if o { "Fold to the title" } else { "Show the properties" };
        let resp = ui.interact(r, ui.id().with(("an-fold", c.id)), Sense::click()).on_hover_text(hint);
        let col = if resp.hovered() { look::TEXT } else { look::MUTED };
        p.text(r.center(), Align2::CENTER_CENTER, if o { "▼" } else { "▶" }, FontId::proportional(9.0), col);
        resp
    });
    let head = if caret.is_some() { Rect::from_min_max(pos2(head.left() + 14.0, head.top()), head.max) } else { head };
    let dot = match c.kind {
        ClipKind::Audio => pal.clip_audio,
        ClipKind::Image => pal.clip_image,
        ClipKind::Text => pal.clip_text,
        ClipKind::Sequence => pal.clip_sequence,
        _ => pal.clip_video,
    };
    p.circle_filled(pos2(head.left() + 11.0, y), 4.0, dot);
    let title = format!("{what} - {}", c.name);
    let g = elided(ui, &title, FontId::proportional(12.0), look::TEXT, head.width() - 30.0);
    p.galley(pos2(head.left() + 22.0, y - g.size().y / 2.0), g, look::TEXT);
    (Rect::from_min_max(pos2(head.left(), head.top() + 24.0), head.max), caret)
}

/// Curves' row for one clip: its channel list in the label column, its keys as curves in the lane.
#[allow(clippy::too_many_arguments)]
fn curve_row(
    ui: &mut Ui,
    st: &mut AnimState,
    project: &mut Project,
    c: &Clip,
    head: Rect,
    body: Rect,
    win: (f64, f64),
    playhead: &mut f64,
    pal: &Palette,
    undo: &mut dyn FnMut(&Project),
    out: &mut AnimResp,
) {
    let id = c.id;
    let cx = curve_ctx(st, c);
    let focus = st.focus.filter(|f| cx.channels.contains(f)).or(cx.shown.first().copied()).or(cx.channels.first().copied());
    // the selected keys belong to one clip (`key_clip`): another clip's row sees none, and becomes that clip
    // the moment a key of its own is picked
    let stash = (st.key_clip != Some(id)).then(|| (std::mem::take(&mut st.keys), st.ranges.take()));
    // what the row is, then the clip: the picked block, else the clip's properties
    let what = cx.bi.map_or("Properties", |i| c.blocks[i].name.as_str());
    let folded = st.clip_folded.contains(&id);
    let (list, caret) = row_title(ui, head, c, what, pal, Some(!folded));
    if caret.is_some_and(|r| r.clicked()) {
        if folded {
            st.clip_folded.retain(|x| *x != id);
        } else {
            st.clip_folded.push(id);
        }
    }
    if !folded {
        let mut hui =
            ui.new_child(egui::UiBuilder::new().id_salt(("an-head", id)).max_rect(list.shrink2(vec2(2.0, 1.0))));
        hui.set_clip_rect(list.intersect(ui.clip_rect()));
        channel_list(&mut hui, st, project, id, &cx.channels, playhead, undo, out);
        let resp = ui.interact(body, ui.id().with(("an-graph", id)), Sense::click_and_drag());
        let c = project.clip(id).cloned().unwrap_or_else(|| c.clone());
        graph(ui, st, project, &c, cx.span, &cx.shown, focus, body, resp, win, playhead, pal, undo, out);
    }
    // a right-click anywhere in the label cell (the title, a group, a property): which properties show
    let c = project.clip(id).cloned().unwrap_or_else(|| c.clone());
    menu::context_area(ui, ui.id().with(("an-view", id)), head, |ui| view_menu(ui, st, &c, &cx.channels));
    if let Some((keys, ranges)) = stash {
        if st.keys.is_empty() {
            (st.keys, st.ranges) = (keys, ranges);
        } else {
            st.key_clip = Some(id);
        }
    }
}

/// A group's row of a property list (Curves' channel list and the Inspector's): its name and fold arrow.
/// `blank` = the control slots it leaves empty.
pub(crate) fn group_row(
    ui: &mut Ui,
    rows: &mut crate::ui::list_rows::Rows,
    n: usize,
    g: &str,
    open: bool,
    dim: bool,
    blank: u8,
) -> crate::ui::list_rows::RowResp {
    let key = u64::MAX - CHANNELS.iter().position(|c| c.0 == g).unwrap_or(0) as u64;
    rows.row(ui, n, &crate::ui::list_rows::RowSpec { key, label: g, open: Some(open), dim, values: &[""], blank, ..Default::default() })
}

/// A property's row of a property list (Curves' channel list and the Inspector's): the property's colour bar,
/// its name (fills, elides), `value` in the fixed value column, the list's controls. Returns the row and its
/// name's own response (click / double-click).
#[allow(clippy::too_many_arguments)]
pub(crate) fn prop_list_row(
    ui: &mut Ui,
    rows: &mut crate::ui::list_rows::Rows,
    n: usize,
    id: Id,
    p: &str,
    label: &str,
    value: &str,
    depth: u8,
    dim: bool,
    blank: u8,
) -> (crate::ui::list_rows::RowResp, egui::Response) {
    let key = (id as u64) << 8 | PROPS.iter().position(|q| *q == p).unwrap_or(0) as u64;
    let r = rows.row(ui, n, &crate::ui::list_rows::RowSpec { key, label, depth, dim, values: &[value], blank, ..Default::default() });
    // the property's colour, as on the graph
    let sw = Rect::from_min_size(pos2(r.label.left() - 7.0, r.label.center().y - 5.0), vec2(3.0, 10.0));
    ui.painter().rect_filled(sw, 1.0, prop_color(p));
    let lr = ui.interact(r.label, ui.id().with(("ch", id, key)), Sense::click());
    (r, lr)
}

/// The channel rows: [▸ group | property] [value] [◀ previous key] [◆ key at the playhead] [▶ next key] [👁].
#[allow(clippy::too_many_arguments)]
fn channel_list(
    ui: &mut Ui,
    st: &mut AnimState,
    project: &mut Project,
    id: Id,
    channels: &[&'static str],
    playhead: &mut f64,
    undo: &mut dyn FnMut(&Project),
    out: &mut AnimResp,
) {
    use crate::ui::list_rows::{Ctrl, Rows};
    const CTRLS: &[Ctrl] = &[
        Ctrl { glyph: "◀", hint: "Previous key" },
        Ctrl { glyph: "◆", hint: "Add a key at the playhead (or remove the one there)" },
        Ctrl { glyph: "▶", hint: "Next key" },
        Ctrl { glyph: "👁", hint: "Show or hide on the graph" },
    ];
    let Some(c) = project.clip(id).cloned() else { return };
    let lt = (*playhead - c.start).clamp(0.0, c.duration);
    egui::ScrollArea::vertical().id_salt(("anim-ch", id)).auto_shrink([false, false]).show(ui, |ui| {
        let mut rows = Rows::new(ui, ("anim-ch-rows", id), CTRLS, &[54.0], true);
        let mut n = 0;
        // (props, ctrl) to apply after the list: one undo step
        let mut todo: Vec<(&'static str, usize)> = Vec::new();
        // the groups and their properties in the user's order; what each drawn row is, for a drag's result
        let groups = ch_groups(&st.ch_order);
        let mut drawn: Vec<(usize, Option<&'static str>)> = Vec::new();
        for (gi, (g, props)) in groups.iter().enumerate() {
            let g = *g;
            let list: Vec<&'static str> =
                props.iter().copied().filter(|p| channels.contains(p) && !prop_off(st, p)).collect();
            if list.is_empty() {
                continue;
            }
            let open = !st.folded.iter().any(|x| x == g);
            let any_keyed = list.iter().any(|p| prop_ref(&c, p).is_some_and(|a| a.is_animated()));
            let r = group_row(ui, &mut rows, n, g, open, !any_keyed, 0);
            n += 1;
            drawn.push((gi, None));
            if r.toggled {
                if open {
                    st.folded.push(g.to_string());
                } else {
                    st.folded.retain(|x| x != g);
                }
            }
            if let Some(k) = r.ctrl {
                todo.extend(list.iter().map(|&p| (p, k)));
            }
            if !open {
                continue;
            }
            for &p in &list {
                let Some(a) = prop_ref(&c, p) else { continue };
                let v = format!("{:.1}{}", shown(p, a.at(lt)), unit(p));
                let hidden = st.hidden.contains(&p);
                let (r, lr) = prop_list_row(ui, &mut rows, n, id, p, prop_label(p), &v, 1, hidden || !a.is_animated(), 0);
                n += 1;
                drawn.push((gi, Some(p)));
                #[cfg(test)]
                st.ch_rows.push((id, p));
                // the focused one gets a bar
                if st.focus == Some(p) {
                    ui.painter().rect_filled(r.label.expand2(vec2(4.0, 1.0)), 3.0, pal_accent(ui).gamma_multiply(0.12));
                }
                // a click on the name: focus it and select its keys (Shift adds)
                // a double-click on the name: a key at the playhead
                if lr.double_clicked() {
                    todo.push((p, 9));
                } else if lr.clicked() {
                    st.focus = Some(p);
                    if !ui.input(|i| i.modifiers.shift) {
                        st.keys.clear();
                    }
                    for k in 0..a.keys.len() {
                        if !st.keys.contains(&(p, k)) {
                            st.keys.push((p, k));
                        }
                    }
                }
                if let Some(k) = r.ctrl {
                    todo.push((p, k));
                }
            }
        }
        if n == 0 && !channels.is_empty() {
            ui.weak("No properties shown").on_hover_text("Right-click for Show All");
        }
        // a drag of a row's handle (or Alt+Up / Down on the selected row) reorders the list: a group among the
        // groups, a property inside its group. A view order: the project does not change, so no undo step
        if let Some(mv) = rows.finish(ui) {
            st.ch_order = ch_moved(groups, &drawn, &mv);
        }
        if todo.is_empty() {
            return;
        }
        let edits = todo.iter().any(|t| t.1 == 1 || t.1 == 9);
        if edits {
            undo(project);
        }
        for (p, k) in todo {
            match k {
                0 | 2 => {
                    let a = prop_ref(&c, p).unwrap();
                    let t = if k == 0 {
                        a.keys.iter().rev().map(|k| k.t).find(|&t| t < lt - 1e-4)
                    } else {
                        a.keys.iter().map(|k| k.t).find(|&t| t > lt + 1e-4)
                    };
                    if let Some(t) = t {
                        *playhead = c.start + t;
                        out.seeked = true;
                        st.focus = Some(p);
                    }
                }
                // ◆ adds a key at the playhead or removes the one there; a double-click (9) only adds
                1 | 9 => {
                    let Some(cm) = project.clip_mut(id) else { continue };
                    let Some(a) = prop_mut(cm, p) else { continue };
                    match a.key_index_at(lt) {
                        Some(i) if k == 1 => {
                            let v = a.keys[i].v;
                            a.keys.remove(i);
                            if a.keys.is_empty() {
                                a.value = v; // the last key gone: the property keeps its value
                            }
                        }
                        Some(_) => {}
                        None if !a.is_animated() => a.toggle_key(lt),
                        None => {
                            let v = a.at(lt);
                            a.key_at(lt, v);
                        }
                    }
                    touch_blocks(cm, p, lt);
                    st.keys.clear();
                    out.edited = true;
                }
                _ => {
                    if st.hidden.contains(&p) {
                        st.hidden.retain(|x| *x != p);
                    } else {
                        st.hidden.push(p);
                    }
                }
            }
        }
    });
}

/// Property `p` is left out of the channel list (and the graph) by the view.
fn prop_off(st: &AnimState, p: &str) -> bool {
    st.props_off.iter().any(|x| x == p)
}

/// The channel list's right-click (a clip's title, a group or a property): which properties show, grouped
/// like the list, the folds, and the default view. A view choice: nothing in the project changes, and the
/// choice holds for every clip (the app keeps it in Settings).
fn view_menu(ui: &mut Ui, st: &mut AnimState, c: &Clip, channels: &[&'static str]) {
    for (g, props) in ch_groups(&st.ch_order) {
        let list: Vec<&'static str> = props.into_iter().filter(|p| channels.contains(p)).collect();
        if list.is_empty() {
            continue;
        }
        ui.weak(g);
        for p in list {
            if menu::check_keep(ui, !prop_off(st, p), prop_label(p)).clicked() {
                if prop_off(st, p) {
                    st.props_off.retain(|x| x != p);
                } else {
                    st.props_off.push(p.into());
                }
            }
        }
    }
    ui.separator();
    let unkeyed: Vec<&str> =
        channels.iter().copied().filter(|p| !prop_ref(c, p).is_some_and(|a| a.is_animated())).collect();
    if menu::row(ui, None, "Show All", "").clicked() {
        st.props_off.clear();
    }
    if menu::row(ui, None, "Hide Unkeyed", "").clicked() {
        for p in &unkeyed {
            if !prop_off(st, p) {
                st.props_off.push(p.to_string());
            }
        }
    }
    if menu::row(ui, None, "Only Keyed", "").clicked() {
        st.props_off.retain(|x| !channels.contains(&x.as_str()));
        st.props_off.extend(unkeyed.iter().map(|p| p.to_string()));
    }
    ui.separator();
    if menu::row(ui, None, "Collapse All", "").clicked() {
        st.folded = CHANNELS.iter().map(|g| g.0.to_string()).collect();
    }
    if menu::row(ui, None, "Expand All", "").clicked() {
        st.folded.clear();
        st.clip_folded.clear();
    }
    ui.separator();
    if menu::row(ui, None, "Reset to Default View", "").clicked() {
        st.reset_view();
    }
}

impl AnimState {
    /// The channel list as it first was: every property shown, nothing folded, the built-in order.
    pub fn reset_view(&mut self) {
        self.props_off.clear();
        self.folded.clear();
        self.clip_folded.clear();
        self.ch_order.clear();
        self.hidden.clear();
    }
}

fn pal_accent(ui: &Ui) -> Color32 {
    ui.visuals().selection.stroke.color
}

#[allow(clippy::too_many_arguments)]
fn graph(
    ui: &mut Ui,
    st: &mut AnimState,
    project: &mut Project,
    c: &Clip,
    (s0, s1): (f64, f64),
    props: &[&'static str],
    focus: Option<&'static str>,
    rect: Rect,
    resp: egui::Response,
    win: (f64, f64),
    playhead: &mut f64,
    pal: &Palette,
    undo: &mut dyn FnMut(&Project),
    out: &mut AnimResp,
) {
    let id = c.id;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, look::DEEP);
    if props.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No keys shown: ◆ in the list adds one at the playhead, or show a hidden property (👁)",
            FontId::proportional(12.0),
            look::MUTED,
        );
        return;
    }
    // time is the lanes' own (`win`, timeline seconds): every clip's graph lines up under the one ruler
    let plot = Rect::from_min_max(rect.min + vec2(0.0, 8.0), rect.max - vec2(0.0, 8.0));
    let (f0, f1) = (win.0 - c.start, win.1 - c.start);
    let x = |t: f64| plot.left() + ((t - f0) / (f1 - f0)) as f32 * plot.width();
    let tx = |px: f32| f0 + ((px - plot.left()) / plot.width()) as f64 * (f1 - f0);
    let vel = |a: &Animated, t: f64| {
        let e = 1.0 / 600.0;
        (a.at(t + e) - a.at(t - e)) / (2.0 * e)
    };
    // per-property y range (frozen while a drag runs so the curve doesn't swim under the pointer)
    let ranges: Vec<(&'static str, f64, f64)> = st.ranges.clone().unwrap_or_else(|| {
        props
            .iter()
            .map(|&p| {
                let a = prop_ref(c, p).unwrap();
                // what is on screen, sampled as finely as it is drawn, plus the keys themselves (a shake's
                // peaks fall between coarse samples)
                let (q0, q1) = (f0, f1);
                let mut vs: Vec<f64> = (0..=240)
                    .map(|i| {
                        let t = q0 + (q1 - q0) * i as f64 / 240.0;
                        if st.velocity {
                            vel(a, t)
                        } else {
                            a.at(t)
                        }
                    })
                    .collect();
                if !st.velocity {
                    vs.extend(a.keys.iter().filter(|k| k.t >= q0 && k.t <= q1).map(|k| k.v));
                }
                let (mut lo, mut hi) = vs.iter().fold((f64::MAX, f64::MIN), |(l, h), &v| (l.min(v), h.max(v)));
                if st.velocity {
                    let m = lo.abs().max(hi.abs()).max(1e-3);
                    (lo, hi) = (-m, m);
                }
                if hi - lo < 1e-6 {
                    lo -= 1.0;
                    hi += 1.0;
                }
                let g = (hi - lo) * 0.12;
                (p, lo - g, hi + g)
            })
            .collect()
    });
    let rng = |p: &str| ranges.iter().find(|r| r.0 == p).map_or((0.0, 1.0), |r| (r.1, r.2));
    let y = |p: &str, v: f64| {
        let (lo, hi) = rng(p);
        plot.bottom() - ((v - lo) / (hi - lo)) as f32 * plot.height()
    };
    let vy = |p: &str, py: f32| {
        let (lo, hi) = rng(p);
        lo + ((plot.bottom() - py) / plot.height()) as f64 * (hi - lo)
    };
    // grid, the block's span, time ticks, value ticks of the focused property
    painter.rect_filled(Rect::from_min_max(pos2(x(s0).max(plot.left()), plot.top()), pos2(x(s1).min(plot.right()), plot.bottom())), 0.0, look::TEXT.gamma_multiply(0.03));
    for t in ticks(c.start + f0, c.start + f1, plot.width()) {
        let px = x(t - c.start);
        painter.line_segment([pos2(px, rect.top()), pos2(px, rect.bottom())], Stroke::new(1.0, look::LINE));
    }
    if let Some(fp) = focus.filter(|f| props.contains(f)) {
        let (lo, hi) = rng(fp);
        for i in 0..=4 {
            let v = lo + (hi - lo) * i as f64 / 4.0;
            let py = y(fp, v);
            painter.line_segment([pos2(plot.left(), py), pos2(plot.right(), py)], Stroke::new(1.0, look::LINE.gamma_multiply(0.7)));
            let txt = if st.velocity { speed_text(fp, shown(fp, v)) } else { format!("{:.0}{}", shown(fp, v), unit(fp)) };
            painter.text(pos2(plot.left() + 5.0, py - 1.0), Align2::LEFT_BOTTOM, txt, FontId::proportional(10.0), prop_color(fp));
        }
    }
    // curves (focus last, on top)
    let mut order: Vec<&'static str> = props.iter().copied().filter(|p| Some(*p) != focus).collect();
    order.extend(focus.filter(|f| props.contains(f)));
    for &p in &order {
        let a = prop_ref(c, p).unwrap();
        let pts: Vec<Pos2> = (0..=240)
            .map(|i| {
                let t = f0 + (f1 - f0) * i as f64 / 240.0;
                pos2(x(t), y(p, if st.velocity { vel(a, t) } else { a.at(t) }))
            })
            .collect();
        let foc = Some(p) == focus;
        painter.with_clip_rect(plot).add(egui::Shape::line(pts, Stroke::new(if foc { 2.2 } else { 1.4 }, prop_color(p).gamma_multiply(if foc { 1.0 } else { 0.55 }))));
    }
    // keys + the selected keys' handles (value view only: the handles are the segment's bezier)
    let mut hits: Vec<(Rect, &'static str, usize, i8)> = Vec::new();
    if !st.velocity {
        for &p in &order {
            let a = prop_ref(c, p).unwrap();
            for (k, key) in a.keys.iter().enumerate().filter(|(_, k)| k.t >= f0 - 1e-4 && k.t <= f1 + 1e-4) {
                let kp = pos2(x(key.t), y(p, key.v));
                let on = st.keys.contains(&(p, k));
                if on {
                    if let Some(nx) = a.keys.get(k + 1) {
                        let (x1, y1, _, _) = key.ease.handles();
                        let h = pos2(x(key.t + x1 as f64 * (nx.t - key.t)), y(p, key.v + y1 as f64 * (nx.v - key.v)));
                        painter.line_segment([kp, h], Stroke::new(1.0, look::MUTED));
                        painter.circle(h, 4.5, look::DEEP, Stroke::new(1.5, look::HANDLE));
                        hits.push((Rect::from_center_size(h, vec2(12.0, 12.0)), p, k, 1));
                    }
                    if let Some(pv) = k.checked_sub(1).and_then(|j| a.keys.get(j)) {
                        let (_, _, x2, y2) = pv.ease.handles();
                        let h = pos2(x(pv.t + x2 as f64 * (key.t - pv.t)), y(p, pv.v + y2 as f64 * (key.v - pv.v)));
                        painter.line_segment([kp, h], Stroke::new(1.0, look::MUTED));
                        painter.circle(h, 4.5, look::DEEP, Stroke::new(1.5, look::HANDLE));
                        hits.push((Rect::from_center_size(h, vec2(12.0, 12.0)), p, k, -1));
                    }
                }
                let d = 5.5;
                painter.add(egui::Shape::convex_polygon(
                    vec![kp + vec2(0.0, -d), kp + vec2(d, 0.0), kp + vec2(0.0, d), kp + vec2(-d, 0.0)],
                    if on { Color32::WHITE } else { prop_color(p) },
                    Stroke::new(1.5, if on { prop_color(p) } else { Color32::BLACK }),
                ));
                hits.push((Rect::from_center_size(kp, vec2(13.0, 13.0)), p, k, 0));
                #[cfg(test)]
                st.key_pos.push((id, p, k, kp));
            }
        }
    }
    let ph = *playhead - c.start;
    if let Some((a, b)) = st.boxsel.filter(|_| resp.dragged()) {
        painter.rect(Rect::from_two_pos(a, b), 0.0, pal.accent.gamma_multiply(0.15), Stroke::new(1.0, pal.accent), egui::StrokeKind::Inside);
    }
    let at = |pos: Pos2| hits.iter().rev().find(|h| h.0.contains(pos)).copied();
    let shift = ui.input(|i| i.modifiers.shift);
    let primary = resp.dragged_by(egui::PointerButton::Primary) || resp.drag_started_by(egui::PointerButton::Primary);

    // ---- pick: a click selects (Shift adds / removes), on empty space it clears and seeks ----
    if resp.clicked() {
        let pos = resp.interact_pointer_pos();
        match pos.and_then(at) {
            Some((_, p, k, 0)) => {
                if shift {
                    if let Some(j) = st.keys.iter().position(|s| *s == (p, k)) {
                        st.keys.remove(j);
                    } else {
                        st.keys.push((p, k));
                    }
                } else {
                    st.keys = vec![(p, k)];
                }
                st.focus = Some(p);
            }
            Some(_) => {}
            None => {
                if !shift {
                    st.keys.clear();
                }
                if let Some(pp) = pos {
                    *playhead = (c.start + tx(pp.x)).max(0.0);
                    out.seeked = true;
                }
            }
        }
    }
    // ---- drag: what was under the press when it started is what moves, for the whole drag ----
    if resp.drag_started_by(egui::PointerButton::Primary) {
        let press = ui.input(|i| i.pointer.press_origin());
        st.grab = press.and_then(at).map(|h| (h.1, h.2, h.3));
        match st.grab {
            Some((p, k, side)) if !st.velocity => {
                if side == 0 && !st.keys.contains(&(p, k)) {
                    if !shift {
                        st.keys.clear();
                    }
                    st.keys.push((p, k));
                }
                st.focus = Some(p);
                undo(project);
                st.orig = Some(c.clone());
                st.ranges = Some(ranges.clone());
            }
            _ => {
                st.grab = None;
                if !shift {
                    st.keys.clear();
                }
                st.boxsel = press.map(|p| (p, p));
            }
        }
    }
    if primary && resp.dragged() {
        let cur = ui.input(|i| i.pointer.interact_pos());
        let press = ui.input(|i| i.pointer.press_origin());
        if let (Some(b), Some(cur)) = (st.boxsel.as_mut(), cur) {
            b.1 = cur;
        } else if let (Some(orig), Some(cur), Some(pp), Some((hp, hk, side))) = (st.orig.clone(), cur, press, st.grab) {
            let mut cm = orig.clone();
            if side == 0 {
                // the selected keys move together in time (frames, the playhead snaps) and in value, each on its
                // own property's scale; Ctrl = time only. Shift locks the drag to one axis: the way the pointer
                // has mostly moved since the press, decided as Shift goes down and kept until it is let go
                if !shift {
                    st.axis = None;
                } else if st.axis.is_none() {
                    st.axis = Some((cur.x - pp.x).abs() >= (cur.y - pp.y).abs());
                }
                let (move_t, move_v) = (st.axis != Some(false), st.axis != Some(true));
                let lead = prop_ref(&orig, hp).and_then(|a| a.keys.get(hk)).map_or(0.0, |k| k.t);
                let fps = project.fps.max(1.0);
                let mut to = ((lead + tx(cur.x) - tx(pp.x)) * fps).round() / fps;
                if ((ph - to) * plot.width() as f64 / (f1 - f0)).abs() < SNAP_PX as f64 {
                    to = ph;
                }
                let dt = if move_t { to - lead } else { 0.0 };
                let ctrl = ui.input(|i| i.modifiers.ctrl) || !move_v;
                for &(p, k) in &st.keys {
                    let (Some(oa), Some(a)) = (prop_ref(&orig, p), prop_mut(&mut cm, p)) else { continue };
                    let Some(ok) = oa.keys.get(k).copied() else { continue };
                    let lo = k.checked_sub(1).map_or(f64::MIN, |j| oa.keys[j].t + 1e-3);
                    let hi = oa.keys.get(k + 1).map_or(f64::MAX, |o| o.t - 1e-3);
                    a.keys[k].t = (ok.t + dt).clamp(lo.max(0.0), hi.min(orig.duration));
                    if !ctrl {
                        a.keys[k].v = ok.v + vy(p, cur.y) - vy(p, pp.y);
                    }
                }
                for &(p, k) in &st.keys {
                    if let Some(t) = prop_ref(&orig, p).and_then(|a| a.keys.get(k)).map(|k| k.t) {
                        touch_blocks(&mut cm, p, t);
                    }
                }
            } else if let (Some(oa), Some(a)) = (prop_ref(&orig, hp), prop_mut(&mut cm, hp)) {
                // a handle: rewrite the segment's bezier (Alt = this handle alone; else the key stays smooth)
                let alt = ui.input(|i| i.modifiers.alt);
                let (t, v) = (tx(cur.x), vy(hp, cur.y));
                let seg = if side > 0 { hk } else { hk - 1 };
                let (k0, k1) = (oa.keys[seg], oa.keys[seg + 1]);
                let (dt, dv) = (k1.t - k0.t, k1.v - k0.v);
                let fx = ((t - k0.t) / dt).clamp(0.0, 1.0) as f32;
                let fy = if dv.abs() > 1e-9 { ((v - k0.v) / dv) as f32 } else { 0.0 };
                let (x1, y1, x2, y2) = k0.ease.handles();
                a.keys[seg].ease = if side > 0 { Ease::Bezier { x1: fx, y1: fy, x2, y2 } } else { Ease::Bezier { x1, y1, x2: fx, y2: fy } };
                if !alt {
                    let slope = if side > 0 {
                        (fy as f64 * dv) / (fx as f64 * dt).max(1e-6)
                    } else {
                        ((1.0 - fy as f64) * dv) / ((1.0 - fx as f64) * dt).max(1e-6)
                    };
                    let other = if side > 0 { hk.checked_sub(1) } else { (hk + 1 < oa.keys.len()).then_some(hk) };
                    if let Some(o) = other.filter(|&o| o != seg) {
                        let (ka, kb) = (oa.keys[o], oa.keys[o + 1]);
                        let s = slope * (kb.t - ka.t) / (kb.v - ka.v).abs().max(1e-9).copysign(kb.v - ka.v);
                        a.keys[o].ease = if side > 0 { ease_with_out_slope(ka.ease, s) } else { ease_with_in_slope(ka.ease, s) };
                    }
                }
                touch_blocks(&mut cm, hp, k0.t);
            }
            if let Some(slot) = project.clip_mut(id) {
                *slot = cm;
            }
            out.edited = true;
        }
    }
    if resp.drag_stopped() {
        if let Some((a, b)) = st.boxsel.take() {
            let r = Rect::from_two_pos(a, b);
            for h in hits.iter().filter(|h| h.3 == 0 && r.contains(h.0.center())) {
                if !st.keys.contains(&(h.1, h.2)) {
                    st.keys.push((h.1, h.2));
                }
            }
        }
        st.orig = None;
        st.ranges = None;
        st.grab = None;
        st.axis = None;
    }
    // ---- right-click: on a key (selecting it first) or on empty space ----
    if resp.secondary_clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            match at(pos) {
                Some((_, p, k, 0)) => {
                    if !st.keys.contains(&(p, k)) {
                        st.keys = vec![(p, k)];
                    }
                    st.focus = Some(p);
                }
                _ => st.ctx_t = Some(tx(pos.x).clamp(0.0, c.duration)),
            }
        }
    }
    let mut act: Option<KeyAct> = None;
    let n = st.keys.len();
    let fname = focus.map_or("", prop_label);
    menu::context(&resp, |ui| {
        if n > 0 {
            ui.label(egui::RichText::new(format!("{n} key{}", if n == 1 { "" } else { "s" })).size(11.0).color(look::MUTED));
            menu::sub(ui, Some(crate::ui::tools::Glyph::CurveIcon), "Interpolation", |ui| {
                for (name, e) in Ease::ALL.iter().map(|e| (e.name(), *e)).chain(Ease::PRESETS) {
                    if menu::row(ui, None, name, "").clicked() {
                        act = Some(KeyAct::Ease(e));
                    }
                }
            });
            if menu::row(ui, Some(crate::ui::tools::Glyph::Cross), "Delete", "Del").clicked() {
                act = Some(KeyAct::Delete);
            }
            if n > 1 {
                menu::sub(ui, None, "Loop", |ui| {
                    if let Some(k) = loop_rows(ui) {
                        act = Some(KeyAct::Loop(k));
                    }
                });
            }
            ui.separator();
        }
        if let Some(f) = focus {
            if menu::row(ui, None, &format!("Select {fname} keys"), "").clicked() {
                act = Some(KeyAct::SelectProp(f));
            }
            if let Some(t) = st.ctx_t.filter(|_| n == 0 && !st.velocity) {
                if menu::row(ui, Some(crate::ui::tools::Glyph::Diamond), &format!("Add a {fname} key here"), "").clicked() {
                    act = Some(KeyAct::AddAt(f, t));
                }
            }
        }
        if menu::row(ui, None, "Select all", "").clicked() {
            act = Some(KeyAct::SelectAll);
        }
        if menu::row(ui, None, "Frame all", "F").clicked() {
            act = Some(KeyAct::Fit);
        }
    });
    // Delete / Backspace: the picked keys (never the block or the timeline clip)
    if n > 0 && (st.here || ui.rect_contains_pointer(rect)) && del_pressed(ui) {
        act = Some(KeyAct::Delete);
    }
    // double-click on the graph adds a key on the focused property there
    if resp.double_clicked() && !st.velocity {
        if let (Some(f), Some(pp)) = (focus, resp.interact_pointer_pos()) {
            if at(pp).is_none() {
                act = Some(KeyAct::AddAt(f, tx(pp.x).clamp(0.0, c.duration)));
            }
        }
    }
    let Some(act) = act else { return };
    match act {
        KeyAct::SelectProp(p) => {
            st.keys = (0..prop_ref(c, p).map_or(0, |a| a.keys.len())).map(|k| (p, k)).collect();
        }
        KeyAct::SelectAll => {
            st.keys = props.iter().flat_map(|&p| (0..prop_ref(c, p).map_or(0, |a| a.keys.len())).map(move |k| (p, k))).collect();
        }
        KeyAct::Fit => st.win = None,
        KeyAct::Loop(k) => {
            undo(project);
            out.toast = Some(loop_sel_keys(project, st, id, k));
            out.edited = true;
        }
        KeyAct::Ease(e) => {
            undo(project);
            if let Some(cm) = project.clip_mut(id) {
                for &(p, k) in &st.keys {
                    let t = prop_mut(cm, p).filter(|a| k < a.keys.len()).map(|a| {
                        let k = ease_slot(a, k, s1);
                        a.keys[k].ease = e;
                        a.keys[k].t
                    });
                    if let Some(t) = t {
                        touch_blocks(cm, p, t);
                    }
                }
            }
            out.edited = true;
        }
        KeyAct::Delete => {
            undo(project);
            if let Some(cm) = project.clip_mut(id) {
                let mut sel = st.keys.clone();
                sel.sort_by(|a, b| b.1.cmp(&a.1));
                for (p, k) in sel {
                    let Some(a) = prop_mut(cm, p) else { continue };
                    if k < a.keys.len() {
                        let key = a.keys.remove(k);
                        if a.keys.is_empty() {
                            a.value = key.v;
                        }
                        touch_blocks(cm, p, key.t);
                    }
                }
            }
            st.keys.clear();
            out.edited = true;
        }
        KeyAct::AddAt(p, t) => {
            undo(project);
            if let Some(cm) = project.clip_mut(id) {
                if let Some(a) = prop_mut(cm, p) {
                    if a.is_animated() {
                        let v = a.at(t);
                        a.key_at(t, v);
                    } else {
                        a.toggle_key(t);
                    }
                }
                touch_blocks(cm, p, t);
            }
            st.ctx_t = None;
            out.edited = true;
        }
    }
}

// ---------- flow ----------

/// Flow's row for one clip: how fast its picture moves over the shown time (and into the clip it cuts to, when
/// that one has no row of its own), every hand-off with its two speeds and a Smooth button.
#[allow(clippy::too_many_arguments)]
fn flow_row(
    ui: &mut Ui,
    st: &mut AnimState,
    project: &mut Project,
    c: &Clip,
    ids: &[Id],
    all: &[Seam],
    head: Rect,
    body: Rect,
    (t0, t1): (f64, f64),
    playhead: &mut f64,
    pal: &Palette,
    undo: &mut dyn FnMut(&Project),
    out: &mut AnimResp,
) {
    let id = c.id;
    let tr = project.track_of(id).unwrap_or(0);
    // this clip and the one it cuts to (Flow is about hand-offs, and the cut is the one people feel most)
    let cuts_to = |n: &&Clip| (n.start - c.end()).abs() < 1.5 / project.fps && !ids.contains(&n.id);
    let next = project.tracks[tr].clips.iter().find(cuts_to).cloned();
    // a cut's hand-off is drawn on the earlier clip's row (on the later one's when the earlier has none)
    let seams: Vec<&Seam> =
        all.iter().filter(|s| s.left.0 == id || (s.right.0 == id && !ids.contains(&s.left.0))).collect();
    let prop = seams
        .iter()
        .max_by(|a, b| (a.v_left - a.v_right).abs().total_cmp(&(b.v_left - b.v_right).abs()))
        .map(|s| s.prop.clone())
        .or_else(|| c.blocks.iter().flat_map(|b| b.props.iter()).find(|p| MOTION_PROPS.contains(&p.as_str())).cloned())
        .unwrap_or_else(|| "scale".into());
    let (rest, _) = row_title(ui, head, c, &format!("{} speed", prop_label(&prop)), pal, None);
    if let Some(n) = &next {
        let g = elided(ui, &format!("→ {}", n.name), FontId::proportional(10.5), look::MUTED, head.width() - 30.0);
        ui.painter_at(head).galley(pos2(head.left() + 22.0, rest.top()), g, look::MUTED);
    }
    let resp = ui.interact(body, ui.id().with(("an-flow", id)), Sense::click());
    let plot = Rect::from_min_max(body.min + vec2(0.0, 30.0), body.max - vec2(0.0, 46.0));
    let x = |t: f64| body.left() + ((t - t0) / (t1 - t0).max(1e-3)) as f32 * body.width();
    let samples: Vec<(f64, Option<f64>)> = (0..=300)
        .map(|i| {
            let t = t0 + (t1 - t0) * i as f64 / 300.0;
            let clip = if c.contains(t) { Some(c) } else { next.as_ref().filter(|n| n.contains(t)) };
            (t, clip.map(|cl| speed(cl, &prop, t - cl.start, 1.0)))
        })
        .collect();
    let mut m = samples.iter().filter_map(|s| s.1).fold(1.0f64, |m, v| m.max(v.abs()));
    for s in &seams {
        m = m.max(s.v_left.abs()).max(s.v_right.abs());
    }
    m *= 1.15;
    let y = |v: f64| plot.center().y - (v / m) as f32 * plot.height() / 2.0;
    let p = ui.painter_at(body);
    for v in [-m / 1.15, 0.0, m / 1.15] {
        p.line_segment(
            [pos2(plot.left(), y(v)), pos2(plot.right(), y(v))],
            Stroke::new(1.0, pal.border.gamma_multiply(if v == 0.0 { 1.0 } else { 0.4 })),
        );
        p.text(
            pos2(plot.left() + 5.0, y(v) - 1.0),
            Align2::LEFT_BOTTOM,
            speed_text(&prop, v),
            FontId::proportional(10.0),
            pal.text_dim,
        );
    }
    if next.is_some() && (t0..=t1).contains(&c.end()) {
        p.line_segment(
            [pos2(x(c.end()), plot.top() - 8.0), pos2(x(c.end()), plot.bottom())],
            Stroke::new(1.0, pal.text_dim),
        );
        p.text(
            pos2(x(c.end()) + 4.0, plot.bottom() - 4.0),
            Align2::LEFT_BOTTOM,
            "cut",
            FontId::proportional(10.0),
            pal.text_dim,
        );
    }
    let col = prop_color(&prop);
    for side in [true, false] {
        let pts: Vec<Pos2> = samples
            .iter()
            .filter(|(t, v)| v.is_some() && (*t < c.end()) == side)
            .map(|(t, v)| pos2(x(*t), y(v.unwrap())))
            .collect();
        if pts.len() > 1 {
            p.with_clip_rect(plot).add(egui::Shape::line(pts, Stroke::new(2.2, col)));
        }
    }
    // the blocks underneath, for orientation: a row each where they overlap in time (three rows at most)
    let mut ends: Vec<f64> = Vec::new();
    for cl in std::iter::once(c).chain(next.iter()) {
        for b in &cl.blocks {
            let (a, e) = (cl.start + b.t, cl.start + b.end());
            let lane = ends.iter().position(|&x| x <= a + 1e-4).unwrap_or(ends.len()).min(2);
            if lane == ends.len() {
                ends.push(e);
            } else {
                ends[lane] = e;
            }
            let row = lane as f32 * 13.0;
            let r = Rect::from_min_max(
                pos2(x(a).max(plot.left()), plot.bottom() + 6.0 + row),
                pos2(x(e).min(plot.right()), plot.bottom() + 18.0 + row),
            );
            if r.width() > 1.0 {
                p.rect_filled(r, 3.0, block_kind(b).1.gamma_multiply(0.45));
                p.with_clip_rect(r).text(
                    r.left_center() + vec2(4.0, 0.0),
                    Align2::LEFT_CENTER,
                    &b.name,
                    FontId::proportional(9.5),
                    Color32::WHITE,
                );
            }
        }
    }
    let mut act = None;
    for s in seams.iter().filter(|s| s.prop == prop) {
        let sx = x(s.t);
        if sx < body.left() || sx > body.right() {
            continue;
        }
        let c2 = if s.smooth { GOOD } else { BAD };
        p.line_segment([pos2(sx, y(s.v_left)), pos2(sx, y(s.v_right))], Stroke::new(3.0, c2));
        p.circle(pos2(sx, y(s.v_left)), 4.5, pal.panel, Stroke::new(2.0, c2));
        p.circle_filled(pos2(sx, y(s.v_right)), 4.5, c2);
        let label = if s.smooth {
            "✓ Smooth".to_string()
        } else {
            format!("⚡ {} jump · Smooth", speed_text(&s.prop, (s.v_left - s.v_right).abs()))
        };
        // a pill drawn by hand (`ui.put` would move the rows' layout cursor up to it)
        let g = p.layout_no_wrap(label, FontId::proportional(11.0), c2);
        let br = Rect::from_center_size(pos2(sx, plot.top() - 16.0), g.size() + vec2(16.0, 6.0)).intersect(body);
        let r = ui
            .interact(br, ui.id().with(("an-flow-seam", s.left, s.right)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(if s.smooth { "The speeds already match" } else { "Match the two speeds by easing" });
        let line = if r.hovered() { c2 } else { c2.gamma_multiply(0.6) };
        p.rect(br, 10.0, look::CARD, Stroke::new(1.0, line), egui::StrokeKind::Inside);
        p.galley(br.center() - g.size() / 2.0, g, c2);
        if r.clicked() {
            act = Some((s.left, s.right, s.t));
        }
    }
    if seams.is_empty() {
        p.text(
            plot.left_top() + vec2(90.0, -24.0),
            Align2::LEFT_TOP,
            "No hand-offs here. Put two blocks end to end, or end one on a cut, to match their speeds.",
            FontId::proportional(11.5),
            pal.text_dim,
        );
    }
    if let Some(pp) = resp.interact_pointer_pos().filter(|_| resp.clicked()) {
        *playhead = (t0 + ((pp.x - body.left()) / body.width()) as f64 * (t1 - t0)).max(0.0);
        out.seeked = true;
    }
    if let Some((l, r, t)) = act {
        undo(project);
        match project.match_seam(l, r, st.mode) {
            Ok(v) => {
                out.toast = Some(format!("Speeds matched at {}", speed_text(&prop, v)));
                out.play = Some(((t - 0.8).max(0.0), t + 0.9));
            }
            Err(e) => out.toast = Some(e),
        }
        out.edited = true;
    }
}

// ---------- speed ramp ----------

/// Speed's row for one clip: its speed ramp over the shown time; the ramps to start from are in the label
/// column's menu.
#[allow(clippy::too_many_arguments)]
fn speed_row(
    ui: &mut Ui,
    st: &mut AnimState,
    project: &mut Project,
    c: &Clip,
    head: Rect,
    body: Rect,
    (t0, t1): (f64, f64),
    playhead: &mut f64,
    pal: &Palette,
    undo: &mut dyn FnMut(&Project),
    out: &mut AnimResp,
) {
    let id = c.id;
    let (rest, _) = row_title(ui, head, c, "Speed", pal, None);
    let tr = project.track_of(id).unwrap_or(0);
    let next = project.tracks[tr].clips.iter().find(|n| (n.start - c.end()).abs() < 1.5 / project.fps).map(|n| n.id);
    // what the menu asked for: these keys (0..1 across the clip; none = normal speed), or a ramp through the cut
    let mut pick: Option<Result<Vec<Keyframe>, Id>> = None;
    let mut remap = None; // ws:time-remap: Some(on) from the menu
    let mut hui = ui.new_child(egui::UiBuilder::new().id_salt(("an-head", id)).max_rect(rest.shrink2(vec2(20.0, 0.0))));
    hui.set_clip_rect(rest.intersect(ui.clip_rect()));
    menu::button(&mut hui, "Ramps", |ui| {
        for pr in crate::engine::presets::builtin_speed_ramps() {
            if menu::row(ui, None, &pr.name, "").clicked() {
                pick = Some(Ok(pr.keys.clone()));
            }
        }
        if menu::row(ui, None, "Normal", "").clicked() {
            pick = Some(Ok(Vec::new()));
        }
        // ws:time-remap
        ui.separator();
        let on = c.time_map.is_some();
        let tip = "Remap time with keys: holds and backward stretches too; changing a speed changes the clip's length";
        if menu::row(ui, None, "Time Remap", if on { "On" } else { "" }).on_hover_text(tip).clicked() {
            remap = Some(!on);
        }
        if let Some(n) = next {
            ui.separator();
            let tip = "Speed up into the cut and out of it on the next clip: the classic speed-ramp transition";
            if menu::row(ui, None, "Ramp through the cut", "").on_hover_text(tip).clicked() {
                pick = Some(Err(n));
            }
        }
    });
    match pick {
        Some(Ok(keys)) => {
            undo(project);
            if let Some(cm) = project.clip_mut(id) {
                cm.speed_curve.keys = keys.iter().map(|k| Keyframe { t: k.t * cm.duration, ..*k }).collect();
            }
            out.edited = true;
        }
        Some(Err(n)) => {
            undo(project);
            let (out_b, in_b) = (find_block(&[], "Speed Ramp Out"), find_block(&[], "Speed Ramp In"));
            if let (Some(o), Some(i)) = (out_b, in_b) {
                let d = 0.5f64.min(c.duration / 2.0);
                project.place_key_block(id, &o, c.duration - d, Some(d));
                project.place_key_block(n, &i, 0.0, Some(d));
                out.play = Some(((c.end() - 1.0).max(0.0), c.end() + 1.0));
                out.toast = Some("Ramped through the cut: 4x out of this clip, 4x into the next".into());
            }
            out.edited = true;
        }
        None => {}
    }
    if let Some(on) = remap {
        undo(project);
        out.edited |= if on { project.time_remap_enable(id) } else { project.time_remap_disable(id) };
    }
    let Some(c) = project.clip(id).cloned() else { return };
    // ws:time-remap: a remapped clip's row is its speed lane (keys, holds, backward stretches)
    if c.time_map.is_some() {
        let (l, w, start) = (body.left(), body.width(), c.start);
        let x = move |t: f64| l + ((start + t - t0) / (t1 - t0).max(1e-3)) as f32 * w;
        let tx = move |px: f32| t0 + ((px - l) / w.max(1.0)) as f64 * (t1 - t0) - start;
        let plot = Rect::from_min_max(body.min + vec2(0.0, 16.0), body.max - vec2(0.0, 4.0));
        let r = crate::ui::time_remap_ui::lane(ui, plot, &x, &tx, project, id, pal, undo);
        out.edited |= r.changed;
        if let Some(s) = r.seek {
            *playhead = (start + s).max(0.0);
            out.seeked = true;
        }
        return;
    }
    let sel = st.speed_sel.filter(|s| s.0 == id).map(|s| s.1);
    let resp = ui.interact(body, ui.id().with(("an-speed", id)), Sense::click_and_drag());
    let plot = Rect::from_min_max(body.min + vec2(0.0, 16.0), body.max - vec2(0.0, 8.0));
    let (l0, l1) = (-2.4f64, 2.1f64);
    // clip-local time on the lanes' shared axis
    let x = |t: f64| body.left() + ((c.start + t - t0) / (t1 - t0).max(1e-3)) as f32 * body.width();
    let tx = |px: f32| t0 + ((px - body.left()) / body.width()) as f64 * (t1 - t0) - c.start;
    let y = |s: f64| plot.bottom() - ((s.max(0.01).log2() - l0) / (l1 - l0)) as f32 * plot.height();
    let sy = |py: f32| 2f64.powf(l0 + ((plot.bottom() - py) / plot.height()) as f64 * (l1 - l0));
    let p = ui.painter_at(body);
    let span = Rect::from_min_max(pos2(x(0.0), body.top()), pos2(x(c.duration), body.bottom()));
    p.rect_filled(span, 0.0, look::TEXT.gamma_multiply(0.03));
    for s in [0.25, 0.5, 1.0, 2.0, 4.0] {
        p.line_segment(
            [pos2(plot.left(), y(s)), pos2(plot.right(), y(s))],
            Stroke::new(1.0, pal.border.gamma_multiply(if s == 1.0 { 1.0 } else { 0.4 })),
        );
        p.text(
            pos2(plot.left() + 5.0, y(s) - 1.0),
            Align2::LEFT_BOTTOM,
            format!("{:.0}%", s * 100.0),
            FontId::proportional(10.0),
            pal.text_dim,
        );
    }
    let keys = &c.speed_curve.keys;
    let at = |t: f64| if keys.is_empty() { c.speed.max(0.01) } else { c.speed_curve.at(t) };
    let pts: Vec<Pos2> = (0..=200)
        .map(|i| {
            let t = c.duration * i as f64 / 200.0;
            pos2(x(t), y(at(t)))
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(2.2, pal.accent)));
    let mut hits = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        let kp = pos2(x(k.t), y(k.v));
        p.circle(kp, 6.0, if sel == Some(i) { Color32::WHITE } else { pal.accent }, Stroke::new(1.5, Color32::WHITE));
        p.text(
            kp - vec2(0.0, 10.0),
            Align2::CENTER_BOTTOM,
            format!("{:.0}%", k.v * 100.0),
            FontId::proportional(10.0),
            pal.text,
        );
        hits.push((Rect::from_center_size(kp, vec2(14.0, 14.0)), i));
    }
    let press = ui.input(|i| i.pointer.press_origin());
    let hit = press.and_then(|pp| hits.iter().find(|h| h.0.contains(pp)).map(|h| h.1));
    if resp.drag_started() && hit.is_some() {
        undo(project);
        st.speed_sel = hit.map(|i| (id, i));
    }
    let sel = st.speed_sel.filter(|s| s.0 == id).map(|s| s.1);
    if let (Some(i), Some(cur)) = (sel.filter(|_| resp.dragged()), resp.interact_pointer_pos()) {
        if let Some(cm) = project.clip_mut(id) {
            let ks = &mut cm.speed_curve.keys;
            let lo = i.checked_sub(1).map_or(0.0, |j| ks[j].t + 0.01);
            let hi = ks.get(i + 1).map_or(cm.duration, |k| k.t - 0.01);
            if i < ks.len() {
                ks[i].t = tx(cur.x).clamp(lo, hi);
                let mut s = sy(cur.y).clamp(0.05, 8.0);
                if (s - 1.0).abs() < 0.06 {
                    s = 1.0;
                }
                ks[i].v = s;
            }
        }
        out.edited = true;
    }
    if resp.clicked() {
        st.speed_sel = hit.map(|i| (id, i));
        if hit.is_none() {
            if let Some(pp) = press {
                *playhead = (c.start + tx(pp.x)).max(0.0);
                out.seeked = true;
            }
        }
    }
    if resp.double_clicked() && hit.is_none() {
        if let Some(pp) = press {
            undo(project);
            if let Some(cm) = project.clip_mut(id) {
                let t = tx(pp.x).clamp(0.0, cm.duration);
                if cm.speed_curve.keys.is_empty() {
                    let s = cm.speed.max(0.01);
                    cm.speed_curve.keys = vec![
                        Keyframe { t: 0.0, v: s, ease: Ease::EaseInOut },
                        Keyframe { t: cm.duration, v: s, ease: Ease::EaseInOut },
                    ];
                }
                cm.speed_curve.key_at(t, sy(pp.y).clamp(0.05, 8.0));
                if let Some(k) = cm.speed_curve.key_index_at(t) {
                    cm.speed_curve.keys[k].ease = Ease::EaseInOut;
                }
            }
            out.edited = true;
        }
    }
    if let Some(i) = sel.filter(|_| (st.here || ui.ui_contains_pointer()) && del_pressed(ui)) {
        undo(project);
        if let Some(cm) = project.clip_mut(id).filter(|cm| i < cm.speed_curve.keys.len()) {
            cm.speed_curve.keys.remove(i);
        }
        st.speed_sel = None;
        out.edited = true;
    }
}

// ---------- compact: the focused clip's blocks as a list, controls in place ----------

#[allow(clippy::too_many_arguments)]
fn compact_list(
    ui: &mut Ui,
    st: &mut AnimState,
    project: &mut Project,
    ids: &[Id],
    playhead: &mut f64,
    pal: &Palette,
    undo: &mut dyn FnMut(&Project),
    out: &mut AnimResp,
) {
    let mut id = st.sel.map(|s| s.0).unwrap_or(ids[0]);
    egui::ComboBox::from_id_salt("anim-lane")
        .selected_text(project.clip(id).map_or("", |c| c.name.as_str()).to_string())
        .width(ui.available_width() - 8.0)
        .show_ui(ui, |ui| {
            for &o in ids {
                if let Some(c) = project.clip(o) {
                    if ui.selectable_label(o == id, &c.name).clicked() {
                        id = o;
                        st.sel = None;
                    }
                }
            }
        });
    let Some(c) = project.clip(id).cloned() else { return };
    let seams = project.block_seams();
    const CTRLS: &[crate::ui::list_rows::Ctrl] = &[crate::ui::list_rows::Ctrl { glyph: "✕", hint: "Delete" }];
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let mut rows = crate::ui::list_rows::Rows::new(ui, ("anim-rows", id), CTRLS, &[96.0], false);
        let mut act: Option<Box<dyn FnOnce(&mut Project, &mut AnimState)>> = None;
        for (i, b) in c.blocks.iter().enumerate() {
            let open = st.sel == Some((id, i));
            let jump = seams.iter().any(|s| !s.smooth && (s.left == (id, i) || s.right == (id, i)));
            let when = format!("{}{:.2} s · {:.2} s", if jump { "⚡ " } else { "" }, b.t, b.dur);
            let r = rows.row(
                ui,
                i,
                &crate::ui::list_rows::RowSpec {
                    key: (id as u64) << 16 | i as u64,
                    label: &b.name,
                    open: Some(open),
                    values: &[&when],
                    ..Default::default()
                },
            );
            if r.toggled || ui.interact(r.label, ui.id().with(("crow", id, i)), Sense::click()).clicked() {
                st.sel = if open { None } else { Some((id, i)) };
                if !open {
                    *playhead = c.start + b.t + b.dur * 0.5;
                    out.seeked = true;
                }
            }
            if r.ctrl == Some(0) {
                act = Some(Box::new(move |p, s| {
                    p.remove_key_block(id, i);
                    s.sel = None;
                }));
            }
            if open {
                ui.indent(("cbody", id, i), |ui| {
                    if block_controls(ui, project, (id, i), pal, undo, &mut out.toast) {
                        out.edited = true;
                    }
                });
            }
        }
        if !c.loose_key_spans().is_empty() && chip(ui, false, "Turn hand keys into blocks").clicked() {
            act = Some(Box::new(move |p, _| {
                p.block_from_keys(id, None, "Custom");
            }));
        }
        if c.blocks.is_empty() {
            ui.label(
                egui::RichText::new("No blocks on this clip yet: + Block adds one at the playhead.")
                    .color(pal.text_dim),
            );
        }
        // reordering blocks is moving them in time: the list follows time, so a drag has nothing to apply
        let _ = rows.finish(ui);
        if let Some(f) = act {
            undo(project);
            f(project, st);
            out.edited = true;
        }
    });
}

// ---------- the block's controls (Inspector and the compact list) ----------

/// The Inspector's block section: the Animator's selected block's controls. True when it edited the project.
pub fn block_section(ui: &mut Ui, project: &mut Project, pal: &Palette, undo: &mut dyn FnMut(&Project)) -> bool {
    // (both cards draw: the picked span's, then the picked block's)
    let span = span_section(ui, project, undo);
    let trans = trans_card(ui, project, undo);
    block_card(ui, project, pal, undo) || span || trans
}

/// The Inspector's card of the timeline transition picked in the Animator: the same controls (and undo) as a
/// transition picked on the timeline.
fn trans_card(ui: &mut Ui, project: &mut Project, undo: &mut dyn FnMut(&Project)) -> bool {
    let Some(t) = selected_transition() else { return false };
    if !project.tracks.iter().any(|tr| tr.transitions.iter().any(|x| x.id == t)) {
        return false;
    }
    let mut edited = false;
    egui::Frame::new().fill(look::CARD).corner_radius(10.0).inner_margin(12.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        edited = crate::ui::inspector::transition_section(ui, project, &[t], undo);
    });
    ui.add_space(6.0);
    edited
}

/// The picked block's card.
fn block_card(ui: &mut Ui, project: &mut Project, pal: &Palette, undo: &mut dyn FnMut(&Project)) -> bool {
    let Some((id, i)) = selected_block() else { return false };
    let Some((b, cname)) = project.clip(id).and_then(|c| c.blocks.get(i).map(|b| (b.clone(), c.name.clone()))) else {
        return false;
    };
    let (kind, col) = block_kind(&b);
    let mut edited = false;
    // the preview's Inspector card: kind pill + name + Delete, "on <clip> · t - t", then the controls
    egui::Frame::new().fill(look::CARD).corner_radius(10.0).inner_margin(12.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            // say what it is: a transition (the clip arrives / leaves) or motion while it is on screen
            let what = match kind {
                "In" => "Transition · In",
                "Out" => "Transition · Out",
                "Emphasis" => "Motion · Emphasis",
                "Physics" => "Motion · Physics",
                "Collide" => "Motion · Collide",
                "Text" => "Motion · Text",
                "Camera" => "Motion · Camera",
                "Path" => "Motion · Path",
                k => k,
            };
            kind_pill(ui, what, col);
            ui.label(egui::RichText::new(&b.name).size(15.0).strong().color(look::TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let del = egui::Button::new(egui::RichText::new("Delete").size(12.0).color(look::MUTED)).frame(false);
                if ui.add(del).on_hover_text("Delete this block and its keys").clicked() {
                    undo(project);
                    project.remove_key_block(id, i);
                    SEL.with(|s| s.set(None));
                    edited = true;
                }
            });
        });
        if edited {
            return;
        }
        ui.label(
            egui::RichText::new(format!("on {cname} · {:.2} – {:.2} s", b.t, b.end())).size(11.5).color(look::MUTED),
        );
        ui.add_space(6.0);
        let mut toast = None;
        edited = block_controls(ui, project, (id, i), pal, undo, &mut toast);
        if let Some(t) = toast {
            ui.label(egui::RichText::new(t).size(11.0).color(GOOD));
        }
    });
    ui.add_space(6.0);
    edited
}

/// The Inspector's card of the span picked in the Timeline lane: the transition it makes, each one's length,
/// ease and direction. True when it edited the project (the app's edit hook then makes its transitions again).
fn span_section(ui: &mut Ui, project: &mut Project, undo: &mut dyn FnMut(&Project)) -> bool {
    let Some(sp) = selected_span().and_then(|id| project.spans.iter().find(|s| s.id == id).cloned()) else {
        return false;
    };
    let col = kind_color("Transition");
    let mut e = sp.clone();
    let mut g = crate::ui::Gesture::default();
    let mut del = false;
    egui::Frame::new().fill(look::CARD).corner_radius(10.0).inner_margin(12.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            kind_pill(ui, "Transition · Timeline", col);
            ui.label(egui::RichText::new(sp.name()).size(15.0).strong().color(look::TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let b = egui::Button::new(egui::RichText::new("Delete").size(12.0).color(look::MUTED)).frame(false);
                del = ui.add(b).on_hover_text("Delete this span and the transitions it made").clicked();
            });
        });
        let cuts = project.tracks.iter().filter(|t| t.kind == TrackKind::Video).flat_map(|t| &t.transitions);
        let n = cuts.filter(|x| sp.made.contains(&x.id)).count();
        ui.label(
            egui::RichText::new(format!("{:.2} – {:.2} s · {n} cut{}", sp.t, sp.t + sp.dur, if n == 1 { "" } else { "s" }))
                .size(11.5)
                .color(look::MUTED),
        );
        ui.add_space(6.0);
        egui::Grid::new(("anim-span", sp.id)).num_columns(2).spacing([12.0, 9.0]).min_col_width(70.0).show(ui, |ui| {
            field(ui, "Transition");
            egui::ComboBox::from_id_salt(("anim-span-kind", sp.id)).selected_text(e.name()).show_ui(ui, |ui| {
                for k in TransitionKind::ALL {
                    g.note(&ui.selectable_value(&mut e.kind, k, k.name()));
                }
                let now = ui.input(|i| i.time);
                for it in crate::ui::transitions_ui::library_items(crate::engine::library::Kind::Transition, now).iter() {
                    let on = e.kind == TransitionKind::Shader && *e.shader == *it.src;
                    if ui.selectable_label(on, &it.name).clicked() {
                        set_span_kind(&mut e, (TransitionKind::Shader, it.src.to_string()));
                        g.click();
                    }
                }
            });
            ui.end_row();
            field(ui, "Duration");
            let r = ui.add(egui::DragValue::new(&mut e.cut_dur).range(0.1..=5.0).speed(0.02).suffix(" s"));
            g.note(&r.on_hover_text("How long each transition it makes lasts"));
            ui.end_row();
            field(ui, "Ease");
            egui::ComboBox::from_id_salt(("anim-span-ease", sp.id)).selected_text(e.ease.name()).show_ui(ui, |ui| {
                for ea in Ease::ALL {
                    g.note(&ui.selectable_value(&mut e.ease, ea, ea.name()));
                }
            });
            ui.end_row();
            if e.kind.has_direction() {
                field(ui, "Direction");
                ui.horizontal(|ui| {
                    for (k, name) in ["Left", "Right", "Up", "Down"].iter().enumerate() {
                        g.note(&ui.selectable_value(&mut e.direction, k as u8, *name));
                    }
                });
                ui.end_row();
            }
            if e.kind == TransitionKind::Shader {
                let specs = crate::model::shader_specs(&e.shader);
                crate::ui::effects_ui::plain_param_rows(ui, &specs, &mut e.params, &mut g);
            }
        });
    });
    ui.add_space(6.0);
    if del {
        undo(project);
        project.remove_span(sp.id);
        SPAN.with(|s| s.set(None));
        return true;
    }
    if g.start {
        undo(project);
    }
    if e == sp {
        return false;
    }
    if let Some(slot) = project.spans.iter_mut().find(|s| s.id == sp.id) {
        *slot = e;
    }
    project.refresh_spans();
    true
}

/// A label in the form's left column (muted, like the preview's).
fn field(ui: &mut Ui, label: &str) {
    ui.label(egui::RichText::new(label).size(12.0).color(look::MUTED));
}

fn slider(ui: &mut Ui, label: &str, v: &mut f64, lo: f64, hi: f64, suffix: &str) -> egui::Response {
    field(ui, label);
    let r = ui.add(egui::Slider::new(v, lo..=hi).suffix(suffix).max_decimals(2).trailing_fill(true));
    ui.end_row();
    r
}

/// A section rule: "EASE SHAPE ─────".
fn section(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title.to_uppercase()).size(10.5).color(look::MUTED));
        let r = ui.available_rect_before_wrap();
        ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, look::LINE));
    });
}

/// The physics controls `keys` as form rows (of block kind `g`; None = a Collide block's rigid body). Returns
/// (something changed, an undo step starts).
fn phys_rows(ui: &mut Ui, g: Option<Gen>, keys: &[&str], ph: &mut Phys) -> (bool, bool) {
    let (mut changed, mut gesture) = (false, false);
    let shake = g == Some(Gen::Shake);
    let rigid = g.is_none_or(|g| g.walls());
    for &k in keys {
        let r = match k {
            "mass" => slider(ui, "Mass", &mut ph.mass, 0.1, 10.0, "×")
                .on_hover_text("Heavier: air drag slows it less, and in a collision it is pushed less"),
            "gravity_dir" => {
                field(ui, "Gravity pulls");
                if let Some(d) = side_pad(ui, Some(ph.gravity_dir), false) {
                    ph.gravity_dir = d;
                    (changed, gesture) = (true, true);
                }
                ui.end_row();
                continue;
            }
            "friction" => slider(ui, "Friction", &mut ph.friction, 0.0, 1.0, "")
                .on_hover_text("How much an edge or another body slows what slides or spins along it"),
            "drag" => slider(ui, "Air drag", &mut ph.drag, 0.0, 1.0, ""),
            "throw" => slider(ui, "Velocity", &mut ph.throw, 0.0, 1.0, "")
                .on_hover_text("The speed it starts with, toward the side picked under Velocity toward"),
            "stiffness" => slider(ui, "Stiffness", &mut ph.stiffness, 0.0, 1.0, ""),
            "damping" => slider(ui, if shake { "Fade out" } else { "Damping" }, &mut ph.damping, 0.0, 1.0, ""),
            "bounce" => slider(ui, if rigid { "Elasticity" } else { "Bounciness" }, &mut ph.bounce, 0.0, 1.0, ""),
            "gravity" => slider(ui, "Gravity", &mut ph.gravity, 0.0, 1.0, ""),
            "spin" => slider(ui, if shake { "Tilt" } else { "Spin" }, &mut ph.spin, -1.0, 1.0, ""),
            "freq" => slider(ui, "Jolts / s", &mut ph.freq, 1.0, 40.0, ""),
            _ => {
                field(ui, "Hide edges");
                let r = ui.checkbox(&mut ph.cover, "zoom so the frame edges never show");
                ui.end_row();
                r
            }
        };
        changed |= r.changed();
        gesture |= r.drag_started() || (r.changed() && !r.dragged());
    }
    (changed, gesture)
}

/// A simulated block's motion as curves over its length, one per property it moves (each on its own scale):
/// what the physics sliders do, before playing it. Drawn from the controls, so it follows a slider as it moves.
fn sim_preview(ui: &mut Ui, ctl: &BlockCtl, dur: f64) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().min(280.0), 92.0), Sense::hover());
    let p = ui.painter_at(r);
    p.rect(r, 6.0, look::DEEP, Stroke::new(1.0, look::LINE), egui::StrokeKind::Inside);
    let plot = Rect::from_min_max(r.min + vec2(8.0, 22.0), r.max - vec2(8.0, 8.0));
    let mut lx = r.left() + 8.0;
    for t in &ctl.resolve(dur).tracks {
        let mut a = Animated::new(0.0);
        a.keys = t
            .keys
            .iter()
            .map(|k| {
                let (BlockVal::Abs(v) | BlockVal::Mul(v) | BlockVal::Add(v)) = k.v;
                Keyframe { t: k.f, v, ease: k.ease }
            })
            .collect();
        let vs: Vec<f64> = (0..=96).map(|i| a.at(i as f64 / 96.0)).collect();
        let (lo, hi) = vs.iter().fold((f64::MAX, f64::MIN), |(l, h), &v| (l.min(v), h.max(v)));
        let fy = |v: f64| if hi - lo > 1e-9 { ((v - lo) / (hi - lo)) as f32 } else { 0.5 };
        let pts = vs.iter().enumerate().map(|(i, &v)| {
            pos2(plot.left() + plot.width() * i as f32 / 96.0, plot.bottom() - fy(v) * plot.height())
        });
        let col = prop_color(&t.prop);
        p.add(egui::Shape::line(pts.collect(), Stroke::new(1.6, col)));
        let g = p.layout_no_wrap(prop_label(&t.prop).to_string(), FontId::proportional(10.0), col);
        let w = g.size().x;
        p.galley(pos2(lx, r.top() + 6.0), g, col);
        lx += w + 10.0;
    }
}

/// The ease the block's main move has now: its control, else what the preset was designed with.
fn current_ease(ctl: &BlockCtl) -> Ease {
    ctl.ease.or_else(|| ctl.src.tracks.first().and_then(|t| t.keys.first()).map(|k| k.ease)).unwrap_or(Ease::Linear)
}

/// A Path block's controls: Timing (Shape / As drawn), Duration, Smoothing, Turn along path and, for Shape,
/// the ease the clip travels the path with. Any change makes the block's keys again from the stroke it keeps.
fn path_controls(
    ui: &mut Ui,
    project: &mut Project,
    (id, i): (Id, usize),
    was: f64,
    mut ctl: BlockCtl,
    mut path: PathCtl,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let (mut dur, mut changed, mut gesture) = (was, false, false);
    let note = |r: &egui::Response, changed: &mut bool, gesture: &mut bool| {
        *changed |= r.changed();
        *gesture |= r.drag_started() || (r.changed() && !r.dragged());
    };
    egui::Grid::new(("anim-path", id, i)).num_columns(2).spacing([12.0, 9.0]).min_col_width(70.0).show(ui, |ui| {
        field(ui, "Timing");
        let r = ui.horizontal(|ui| segmented(ui, &mut path.as_drawn, &[(false, "Shape"), (true, "As drawn")]));
        r.response.on_hover_text(
            "Shape: one speed along the path over the block, shaped by Ease. As drawn: your stroke's own timing, its rushes and pauses, scaled to the block",
        );
        if r.inner {
            (changed, gesture) = (true, true);
        }
        ui.end_row();
        let r = slider(ui, "Duration", &mut dur, 0.05, 10.0, " s");
        if r.changed() {
            gesture |= r.drag_started() || !r.dragged();
        }
        let r = slider(ui, "Smoothing", &mut path.smoothing, 0.0, 1.0, "")
            .on_hover_text("0: the points as you drew them. 1: a heavily smoothed curve. The two ends stay where they are");
        note(&r, &mut changed, &mut gesture);
        field(ui, "Turn along path");
        let r = ui.checkbox(&mut path.turn, "").on_hover_text("Rotation faces the way the clip travels");
        note(&r, &mut changed, &mut gesture);
        ui.end_row();
        if !path.as_drawn {
            field(ui, "Ease");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                let cur = current_ease(&ctl);
                for (n, e) in [("Linear", Ease::Linear)].into_iter().chain(Ease::PRESETS) {
                    if ease_chip(ui, cur == e, n, e).clicked() {
                        ctl.ease = Some(e);
                        (changed, gesture) = (true, true);
                    }
                }
            });
            ui.end_row();
        }
    });
    if !path.as_drawn {
        section(ui, "Ease shape");
        let (new, started) = ease_editor(ui, egui::Id::new(("anim-ease", id, i)), current_ease(&ctl));
        gesture |= started;
        if let Some(e) = new {
            ctl.ease = Some(e);
            changed = true;
        }
    }
    if gesture {
        undo(project);
    }
    let mut edited = false;
    if (dur - was).abs() > 1e-9 {
        let t = project.clip(id).map_or(0.0, |c| c.blocks[i].t);
        edited |= project.retime_key_block(id, i, t, dur);
    }
    if changed {
        ctl.path = Some(path);
        edited |= project.set_block_ctl(id, i, ctl);
    }
    edited
}

/// Side, strength, length, ease, physics and the block's hand-offs. True when it edited the project.
fn block_controls(
    ui: &mut Ui,
    project: &mut Project,
    (id, i): (Id, usize),
    pal: &Palette,
    undo: &mut dyn FnMut(&Project),
    toast: &mut Option<String>,
) -> bool {
    let Some(b) = project.clip(id).and_then(|c| c.blocks.get(i)).cloned() else { return false };
    let Some(mut ctl) = b.ctl.clone() else {
        ui.label(
            egui::RichText::new("Plain keys (from before the Animator): drag them in Curves.")
                .size(11.0)
                .color(pal.text_dim),
        );
        return false;
    };
    let mut edited = false;
    // a Collide block: its body in the shared drop; a change runs the drop again for every clip in it
    if b.name == "Collide" {
        section(ui, "Rigid body");
        let (mut changed, mut gesture) = (false, false);
        egui::Grid::new(("anim-rigid", id, i)).num_columns(2).spacing([12.0, 9.0]).min_col_width(70.0).show(ui, |ui| {
            (changed, gesture) = phys_rows(ui, None, RIGID, &mut ctl.phys);
            // (one simulation moves every clip of the drop: a change here runs it again for all of them)
            field(ui, "Velocity toward");
            if let Some(d) = side_pad(ui, ctl.dir, false) {
                ctl.dir = (ctl.dir != Some(d)).then_some(d); // the picked side again: no direction
                (changed, gesture) = (true, true);
            }
            ui.end_row();
        });
        if gesture {
            undo(project);
        }
        return changed && project.recollide(id, i, ctl.phys, ctl.dir) > 0;
    }
    if ctl.edited {
        egui::Frame::new()
            .fill(look::HANDLE.gamma_multiply(0.08))
            .stroke(Stroke::new(1.0, look::HANDLE.gamma_multiply(0.4)))
            .corner_radius(8.0)
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Keys edited by hand, so these controls are paused.").size(12.0));
                if chip(ui, false, &format!("Reset to {}", b.name)).clicked() {
                    undo(project);
                    project.set_block_ctl(id, i, ctl.clone());
                    edited = true;
                }
            });
        return edited;
    }
    if let Some(path) = ctl.path.clone() {
        return path_controls(ui, project, (id, i), b.dur, ctl, path, undo);
    }
    let mut dur = b.dur;
    let mut changed = false;
    let mut gesture = false;
    let mut looped = None;
    // origin / target: where each property is as the block starts / ends
    let ends: Vec<(String, f64, f64)> = project
        .clip(id)
        .map(|c| b.props.iter().take(3).filter_map(|p| prop_ref(c, p).map(|a| (p.clone(), a.at(b.t), a.at(b.end())))).collect())
        .unwrap_or_default();
    egui::Grid::new(("anim-ctl", id, i)).num_columns(2).spacing([12.0, 9.0]).min_col_width(70.0).show(ui, |ui| {
        let wig = wiggles(&ctl);
        let moves_xy = ctl.src.gen.map_or(ctl.dir.is_some() && !wig, |g| g.has_dir());
        if moves_xy {
            let toward = b.name.contains("Out") || ctl.src.gen == Some(Gen::ThrowOut);
            let thrown = ctl.src.gen.is_some_and(|g| g.walls());
            field(ui, if thrown { "Velocity toward" } else if toward { "Toward side" } else { "From side" });
            let lr = ctl.src.gen.is_none() && b.name == "Spin";
            if let Some(d) = side_pad(ui, ctl.dir, lr) {
                ctl.dir = Some(d);
                changed = true;
                gesture = true;
            }
            ui.end_row();
        }
        let r = slider(ui, "Strength", &mut ctl.strength, 0.0, 2.0, "×");
        changed |= r.changed();
        gesture |= r.drag_started() || (r.changed() && !r.dragged());
        let r = slider(ui, "Duration", &mut dur, 0.05, 10.0, " s");
        if r.changed() {
            gesture |= r.drag_started() || !r.dragged();
        }
        field(ui, "Loop");
        ui.horizontal(|ui| {
            let (mut n, mut back) = (ctl.cycles(), ctl.pingpong);
            let r = ui
                .add(egui::DragValue::new(&mut n).range(1..=50).speed(0.1).suffix("×"))
                .on_hover_text("How many times the block plays (1 = once): it grows to that many times its length");
            let r2 = ui.checkbox(&mut back, "ping-pong").on_hover_text("Every other time runs backwards: there and back");
            if r.changed() || r2.changed() {
                // back and forth needs a way back
                looped = Some((if back && n < 2 { 2 } else { n }, back));
                gesture |= r.drag_started() || !r.dragged();
            }
        });
        ui.end_row();
        for (label, to) in [("From", false), ("To", true)].into_iter().filter(|_| ctl.src.gen.is_none() && !wig) {
            field(ui, label);
            ui.horizontal(|ui| {
                for (p, v0, v1) in &ends {
                    let pct = !matches!(p.as_str(), "x" | "y" | "rotation");
                    let mut v = shown(p, if to { *v1 } else { *v0 });
                    let name = match p.as_str() {
                        _ if ends.len() == 1 => String::new(),
                        "x" => "X ".into(),
                        "y" => "Y ".into(),
                        _ => format!("{} ", prop_label(p)),
                    };
                    let r = ui.add(egui::DragValue::new(&mut v).speed(if pct { 0.5 } else { 1.0 }).max_decimals(1).prefix(name).suffix(unit(p))).on_hover_text(
                        if to { "The target: where the block ends" } else { "The origin: where the block starts" },
                    );
                    gesture |= r.drag_started() || (r.changed() && !r.dragged());
                    if r.changed() {
                        let list = if to { &mut ctl.to } else { &mut ctl.from };
                        list.retain(|(q, _)| q != p);
                        list.push((p.clone(), if pct { v / 100.0 } else { v }));
                        changed = true;
                    }
                }
                if to && (!ctl.from.is_empty() || !ctl.to.is_empty()) && chip(ui, false, "As designed").on_hover_text("Back to the preset's own origin and target").clicked() {
                    (ctl.from, ctl.to) = (Vec::new(), Vec::new());
                    changed = true;
                    gesture = true;
                }
            });
            ui.end_row();
        }
        if ctl.src.gen.is_none() {
            field(ui, "Ease");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                let cur = ctl.ease;
                let design = current_ease(&BlockCtl { ease: None, ..ctl.clone() });
                for (n, e, shown) in [("As designed", None, design)]
                    .into_iter()
                    .chain(Ease::PRESETS.into_iter().map(|(n, e)| (n, Some(e), e)))
                    .chain([("Linear", Some(Ease::Linear), Ease::Linear)])
                {
                    if ease_chip(ui, cur == e, n, shown).clicked() {
                        // an ease picked by hand replaces what Flow bent the ends to
                        (ctl.ease, ctl.in_speed, ctl.out_speed) = (e, None, None);
                        changed = true;
                        gesture = true;
                    }
                }
                // shaped by Flow or by hand: neither the design nor a preset
                if let Some(e) = cur.filter(|e| *e != Ease::Linear && !Ease::PRESETS.iter().any(|p| p.1 == *e)) {
                    let _ = ease_chip(ui, true, "Custom", e);
                }
            });
            ui.end_row();
        }
    });
    // a simulated block: its physics (a rigid body for the ones that live in the frame), and what they make
    // of the motion
    if let Some(g) = ctl.src.gen {
        section(ui, if g.walls() { "Rigid body" } else { "Physics" });
        egui::Grid::new(("anim-phys", id, i)).num_columns(2).spacing([12.0, 9.0]).min_col_width(70.0).show(ui, |ui| {
            let (c, s) = phys_rows(ui, Some(g), g.controls(), &mut ctl.phys);
            changed |= c;
            gesture |= s;
            if g == Gen::Shake {
                field(ui, "");
                if chip(ui, false, "New shake pattern").clicked() {
                    ctl.phys.seed = ctl.phys.seed.wrapping_add(1);
                    changed = true;
                    gesture = true;
                }
                ui.end_row();
            }
        });
        sim_preview(ui, &ctl, dur);
    }
    if ctl.src.gen.is_none() {
        section(ui, "Ease shape");
        let (new, started) = ease_editor(ui, egui::Id::new(("anim-ease", id, i)), current_ease(&ctl));
        gesture |= started;
        if let Some(e) = new {
            ctl.ease = Some(e);
            changed = true;
        }
    }
    if gesture {
        undo(project);
    }
    if (dur - b.dur).abs() > 1e-9 {
        project.retime_key_block(id, i, b.t, dur);
        edited = true;
    }
    if changed {
        project.set_block_ctl(id, i, ctl);
        edited = true;
    }
    if let Some((n, back)) = looped {
        edited |= project.loop_key_block(id, i, n, back);
    }
    // hand-offs: the speeds where this block meets its neighbours
    let seams: Vec<Seam> =
        project.block_seams().into_iter().filter(|s| s.left == (id, i) || s.right == (id, i)).collect();
    section(ui, "Flow");
    if seams.is_empty() {
        ui.label(
            egui::RichText::new(
                "Put this move end to end with another one, or end it on a cut, to match their speeds.",
            )
            .size(11.5)
            .color(look::MUTED),
        );
    }
    for s in seams {
        let into = s.right == (id, i);
        let other = if into { s.left } else { s.right };
        let oname = project.clip(other.0).and_then(|c| c.blocks.get(other.1)).map_or(String::new(), |b| b.name.clone());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 4.5, if s.smooth { GOOD } else { BAD });
            ui.label(
                egui::RichText::new(format!("{} {oname}", if into { "Comes from" } else { "Hands over to" }))
                    .size(12.0),
            );
        });
        ui.label(
            egui::RichText::new(format!(
                "ends {} → starts {}{}",
                speed_text(&s.prop, s.v_left),
                speed_text(&s.prop, s.v_right),
                if s.smooth { " · smooth" } else { " · jump" }
            ))
            .size(11.0)
            .monospace()
            .color(look::MUTED),
        );
        ui.horizontal(|ui| {
            let mut go = None;
            let label = if into { "Match its speed" } else { "Match the next speed" };
            let r = if s.smooth { chip(ui, false, label) } else { primary(ui, label) };
            if r.clicked() {
                go = Some(if into { Match::Next } else { Match::Earlier });
            }
            if !into && chip(ui, false, "Ease the next move instead").clicked() {
                go = Some(Match::Next);
            }
            if let Some(m) = go {
                undo(project);
                match project.match_seam(s.left, s.right, m) {
                    Ok(v) => *toast = Some(format!("Matched at {}", speed_text(&s.prop, v))),
                    Err(e) => *toast = Some(e),
                }
                edited = true;
            }
        });
    }
    edited
}

#[cfg(test)]
mod tests {
    // ---- ws:forum-ui ----
    #[test]
    fn lanes_follow_a_playhead_that_leaves_the_view() {
        use super::follow_playhead;
        let w = (10.0, 20.0);
        assert_eq!(follow_playhead(w, Some(12.0), 15.0), w, "still in view");
        assert_eq!(follow_playhead(w, Some(30.0), 30.0), w, "did not move: a pan away stays put");
        assert_eq!(follow_playhead(w, None, 30.0), w, "first frame");
        let f = follow_playhead(w, Some(19.0), 31.0);
        assert!((f.1 - f.0 - 10.0).abs() < 1e-9 && (f.0..=f.1).contains(&31.0), "{f:?}");
    }

    use super::*;

    struct H {
        ctx: egui::Context,
        st: AnimState,
        p: Project,
        sel: Vec<Id>,
        ph: f64,
        width: f32,
        undos: usize,
        shapes: Vec<egui::epaint::ClippedShape>,
    }
    impl H {
        fn new(width: f32) -> H {
            let mut p = Project::new();
            p.tracks[0].clips.push(Clip::new(1, ClipKind::Video, "Gameplay A", 0.0, 6.0));
            p.tracks[0].clips.push(Clip::new(2, ClipKind::Video, "Gameplay B", 6.0, 6.0));
            let f = |n| find_block(&[], n).unwrap();
            p.apply_key_block(1, &f("Slide In Left"), Some(0.0), None);
            p.apply_key_block(1, &f("Zoom Punch"), Some(5.5), None);
            p.apply_key_block(2, &f("Camera Shake"), Some(0.0), None);
            let (st, ctx) = (AnimState::default(), egui::Context::default());
            H { ctx, st, p, sel: vec![1, 2], ph: 0.5, width, undos: 0, shapes: Vec::new() }
        }
        fn frame(&mut self) -> AnimResp {
            self.frame_with(Vec::new(), false)
        }
        /// A frame with pointer events; `shift` is held (modifiers go on the RawInput, not on the events).
        fn frame_with(&mut self, events: Vec<egui::Event>, shift: bool) -> AnimResp {
            let mut out = AnimResp::default();
            let w = self.width;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w, 420.0))),
                events,
                modifiers: egui::Modifiers { shift, ..Default::default() },
                ..Default::default()
            };
            let (st, p, sel, ph, undos) = (&mut self.st, &mut self.p, &self.sel, &mut self.ph, &mut self.undos);
            let full = self.ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let pal = Palette::new(true, Color32::from_rgb(0x33, 0x66, 0xFF));
                    out = show(ui, st, p, sel, ph, false, &pal, &[], &mut |_| *undos += 1);
                });
            });
            self.shapes = full.shapes;
            out
        }
        /// Centre of the last painted text reading exactly `label` (a menu row is painted over the list).
        fn text_at(&self, label: &str) -> Option<Pos2> {
            self.shapes.iter().rev().find_map(|c| match &c.shape {
                egui::epaint::Shape::Text(t) if t.galley.text() == label => Some(t.pos + t.galley.rect.center().to_vec2()),
                _ => None,
            })
        }
        /// A press and release of `button` at `pos`, a frame each (after a frame moving there).
        fn press(&mut self, pos: Pos2, button: egui::PointerButton) {
            self.frame_with(vec![egui::Event::PointerMoved(pos)], false);
            for pressed in [true, false] {
                let e = egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE };
                self.frame_with(vec![e], false);
            }
            self.frame();
        }
    }

    /// The channel list's view: hidden properties are left out of the list and the graph but keep their keys;
    /// folded groups and clips draw no rows; the choice survives Settings' JSON.
    #[test]
    fn the_property_view_hides_rows_not_keys_and_round_trips() {
        let mut h = H::new(1100.0);
        h.st.view = View::Curves;
        h.frame();
        for p in ["x", "y", "scale", "opacity", "speed"] {
            assert!(h.st.ch_rows.contains(&(1, p)), "{p} is listed");
        }
        let keys = h.p.clip(1).unwrap().x.keys.len();
        assert!(keys > 0);
        h.st.props_off = vec!["x".into()];
        h.frame();
        assert!(!h.st.ch_rows.contains(&(1, "x")) && !h.st.ch_rows.contains(&(2, "x")), "hidden on every clip");
        assert!(h.st.ch_rows.contains(&(1, "y")));
        assert!(!curve_ctx(&h.st, h.p.clip(1).unwrap()).shown.contains(&"x"), "and off the graph");
        assert_eq!(h.p.clip(1).unwrap().x.keys.len(), keys, "its keys stay");
        // a folded group: none of its rows; a folded clip: no rows at all
        h.st.folded = vec!["Transform".into()];
        h.frame();
        assert!(!h.st.ch_rows.iter().any(|r| r.1 == "y") && h.st.ch_rows.contains(&(1, "opacity")));
        h.st.clip_folded = vec![1];
        h.frame();
        assert!(!h.st.ch_rows.iter().any(|r| r.0 == 1) && h.st.ch_rows.contains(&(2, "opacity")));
        h.st.reset_view();
        h.frame();
        assert!(h.st.ch_rows.contains(&(1, "x")) && h.st.props_off.is_empty() && h.st.folded.is_empty());
        // Settings keep the choice by kind / group name
        let mut s = crate::settings::Settings::default();
        (s.anim_hidden_props, s.anim_folded) = (vec!["x".into(), "speed".into()], vec!["Opacity".into()]);
        let back: crate::settings::Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!((back.anim_hidden_props, back.anim_folded), (s.anim_hidden_props, s.anim_folded));
    }

    /// Right-click a clip's "Properties" title: a checkable list of its properties; a click hides one (the
    /// menu stays open for the next), another click shows it again; Only Keyed / Show All.
    #[test]
    fn the_property_menu_toggles_visibility() {
        let mut h = H::new(1100.0);
        h.st.view = View::Curves;
        h.frame();
        let title = h.text_at("Properties - Gameplay A").expect("the clip's title");
        h.press(title, egui::PointerButton::Secondary);
        // the menu's rows are painted after the list's, so the last "Position Y" is the menu's
        let row = h.text_at("Position Y").expect("Position Y in the menu");
        assert!(h.text_at("Hide Unkeyed").is_some() && h.text_at("Reset to Default View").is_some());
        h.press(row, egui::PointerButton::Primary);
        assert_eq!(h.st.props_off, ["y"]);
        assert!(!h.st.ch_rows.contains(&(1, "y")), "the row is gone");
        let row = h.text_at("Position Y").expect("the menu is still open");
        h.press(row, egui::PointerButton::Primary);
        assert!(h.st.props_off.is_empty(), "shown again");
        h.press(h.text_at("Only Keyed").unwrap(), egui::PointerButton::Primary);
        // clip 1 animates x and scale: everything else it can animate is hidden, no key changed
        assert!(h.st.props_off.iter().any(|p| p == "opacity") && !h.st.props_off.iter().any(|p| p == "x" || p == "scale"));
        assert_eq!(h.undos, 0, "a view choice, not an edit");
        let title = h.text_at("Properties - Gameplay A").unwrap();
        h.press(title, egui::PointerButton::Secondary);
        h.press(h.text_at("Show All").unwrap(), egui::PointerButton::Primary);
        assert!(h.st.props_off.is_empty());
        // the caret folds the clip to its title
        let caret = h.text_at("▼").unwrap();
        h.press(caret, egui::PointerButton::Primary);
        assert_eq!(h.st.clip_folded.len(), 1);
    }

    /// A block card dropped on a clip's lane lands on that clip at the drop time, as one undo step; a
    /// transition dropped on the Timeline lane is a span there. Every view takes a block.
    #[test]
    fn a_dropped_block_lands_on_the_lane_at_the_drop_time_in_one_undo() {
        use crate::ui::DragPayload;
        for view in [View::Blocks, View::Curves, View::Flow, View::Speed] {
            let mut h = H::new(1100.0);
            h.st.view = view;
            h.frame();
            let (lane, _) = *h.st.drop_pos.iter().find(|d| d.1 == Some(2)).expect("clip 2's lane");
            let (t0, t1) = h.st.win.unwrap();
            let at = |t: f64| pos2(lane.left() + ((t - t0) / (t1 - t0)) as f32 * lane.width(), lane.center().y);
            let want = 6.0 + 0.5 * (t1.min(12.0) - 6.0); // inside clip 2 (6-12 s) and the window
            let pos = at(want);
            let (undos, n1) = (h.undos, h.p.clip(1).unwrap().blocks.len());
            // the press began in the Gallery (off the panel); the card rides the pointer to the lane
            let off = pos2(-40.0, -40.0);
            egui::DragAndDrop::set_payload(&h.ctx, DragPayload::KeyBlock("Pulse".into()));
            h.frame_with(vec![egui::Event::PointerMoved(off), egui::Event::PointerButton { pos: off, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }], false);
            egui::DragAndDrop::set_payload(&h.ctx, DragPayload::KeyBlock("Pulse".into()));
            h.frame_with(vec![egui::Event::PointerMoved(pos)], false);
            assert!(h.p.clip(2).unwrap().blocks.iter().all(|b| b.name != "Pulse"), "{view:?}: nothing lands while dragging");
            h.frame_with(vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }], false);
            let c = h.p.clip(2).unwrap();
            let b = c.blocks.iter().find(|b| b.name == "Pulse").unwrap_or_else(|| panic!("{view:?}: Pulse on clip 2"));
            assert!((c.start + b.t - want).abs() < 0.05, "{view:?}: at {want}, got {}", c.start + b.t);
            assert_eq!(h.undos, undos + 1, "{view:?}: one undo step");
            assert_eq!(h.p.clip(1).unwrap().blocks.len(), n1, "{view:?}: only the clip under the drop");
            assert!(egui::DragAndDrop::payload::<DragPayload>(&h.ctx).is_none());
        }
        // a transition on the Timeline lane: a span from the drop time
        let mut h = H::new(1100.0);
        h.frame();
        let (lane, _) = *h.st.drop_pos.iter().find(|d| d.1.is_none()).expect("the Timeline lane");
        let (t0, t1) = h.st.win.unwrap();
        let want = (t0 + t1) / 2.0;
        let pos = pos2(lane.left() + ((want - t0) / (t1 - t0)) as f32 * lane.width(), lane.center().y);
        let k = crate::model::TransitionKind::CrossFade;
        egui::DragAndDrop::set_payload(&h.ctx, DragPayload::Transition(k));
        h.frame_with(vec![egui::Event::PointerMoved(pos)], false);
        h.frame_with(vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }], false);
        assert_eq!(h.p.spans.len(), 1);
        assert!((h.p.spans[0].t - want).abs() < 0.05, "from the drop time");
        assert_eq!(h.undos, 1);
        // a block on the Timeline lane, a transition on a clip's lane: nothing
        assert!(card_drop(&h.p, &[], &DragPayload::KeyBlock("Pulse".into()), None, 1.0, 1.0).is_none());
        assert!(card_drop(&h.p, &[], &DragPayload::Transition(k), Some(1), 1.0, 1.0).is_none());
    }

    #[test]
    fn assert_no_idle_repaint_animator_pane() {
        for (w, view) in [
            (1100.0, View::Blocks),
            (1100.0, View::Curves),
            (1100.0, View::Flow),
            (1100.0, View::Speed),
            (420.0, View::Blocks),
        ] {
            let mut h = H::new(w);
            h.st.view = view;
            h.st.sel = Some((1, 0));
            for _ in 0..30 {
                h.frame();
            }
            assert!(!h.ctx.has_requested_repaint(), "an idle Animator ({view:?}, {w} pt) must not repaint");
        }
    }

    /// Curves, Flow and Speed are the Blocks timeline with other rows: the same window, ruler and scrollbar.
    #[test]
    fn every_view_is_the_one_timeline_and_the_scrollbar_moves_its_window() {
        let mut wins = Vec::new();
        for view in [View::Blocks, View::Curves, View::Flow, View::Speed] {
            let mut h = H::new(1100.0);
            h.st.view = view;
            h.st.whole = true;
            (h.st.win, h.st.win_for) = (Some((2.0, 5.0)), vec![1, 2]);
            h.frame();
            h.frame();
            // the window survives the view (no view frames its own clip any more)
            assert_eq!(h.st.win, Some((2.0, 5.0)), "{view:?}");
            wins.push(h.st.win);
        }
        assert!(wins.iter().all(|w| *w == wins[0]));
        // the scrollbar: a drag of the thumb slides the window through the range, clamped to it
        let ctx = egui::Context::default();
        let fit = (0.0, 12.0);
        let mut win = Some((2.0, 5.0));
        let bar = Rect::from_min_size(pos2(100.0, 200.0), vec2(600.0, SCROLL_H));
        let grab = pos2(100.0 + 600.0 * 3.5 / 12.0, 206.0);
        let ev = |pos: Pos2, pressed: Option<bool>| match pressed {
            Some(p) => egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: p,
                modifiers: Default::default(),
            },
            None => egui::Event::PointerMoved(pos),
        };
        let steps: [Vec<egui::Event>; 5] = [
            vec![ev(grab, None)],
            vec![ev(grab, Some(true))],
            vec![ev(grab + vec2(100.0, 0.0), None)],
            vec![ev(grab + vec2(100.0, 0.0), Some(false))],
            vec![],
        ];
        for events in steps {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 420.0))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| time_scrollbar(ui, bar, &mut win, fit));
            });
        }
        let (a, b) = win.unwrap();
        assert!((a - 4.0).abs() < 0.05 && (b - 7.0).abs() < 0.05, "100 px of 600 = 2 s of 12: {a} {b}");
        // and it never leaves the range
        let mut win = Some((10.0, 12.0));
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| time_scrollbar(ui, bar, &mut win, fit));
        });
        assert_eq!(win, Some((10.0, 12.0)));
    }

    /// Several selected clips: a graph row each (Curves / Flow / Speed), stacked under the one ruler, and keys
    /// are picked on one clip at a time.
    #[test]
    fn curves_stack_a_graph_per_clip_and_keys_belong_to_one_clip() {
        for view in [View::Curves, View::Flow, View::Speed] {
            let mut h = H::new(1100.0);
            h.st.view = view;
            h.frame();
            // both clips' rows fit the panel between them (the default height shares it)
            assert!(h.st.graph_h.is_none());
        }
        let mut h = H::new(1100.0);
        h.st.view = View::Curves;
        (h.st.keys, h.st.key_clip) = (vec![("x", 0), ("x", 1)], Some(1));
        h.frame();
        assert_eq!((h.st.keys.len(), h.st.key_clip), (2, Some(1)), "clip 2's row leaves clip 1's keys alone");
        // looping them repeats the slide after its last key
        let before = h.p.clip(1).unwrap().x.keys.len();
        let msg = loop_sel_keys(&mut h.p, &h.st, 1, 2);
        assert!(msg.starts_with("Looped"), "{msg}");
        assert_eq!(h.p.clip(1).unwrap().x.keys.len(), before + 4);
        // the clip leaves the selection: its keys are forgotten
        h.sel = vec![2];
        h.frame();
        assert_eq!((h.st.keys.len(), h.st.key_clip), (0, None));
    }

    #[test]
    fn physics_blocks_are_in_the_menu_and_read_as_physics() {
        for name in ["Gravity Fall", "Throw & Bounce", "Bounce Drop", "Spring In"] {
            let b = find_block(&[], name).unwrap_or_else(|| panic!("{name} is not a block"));
            assert_eq!(kind_of(&b.name, b.gen).0, "Physics", "{name}");
        }
        assert_eq!(kind_of("Collide", None).0, "Collide");
        // the rigid-body controls: one set, shared by Gravity Fall, Throw & Bounce and the collision drop
        assert_eq!(Gen::GravityFall.controls(), RIGID);
        assert_eq!(Gen::ThrowBounce.controls(), RIGID);
        for k in ["mass", "gravity", "gravity_dir", "bounce", "friction", "drag", "throw", "spin"] {
            assert!(RIGID.contains(&k), "{k}");
        }
        // a Collide block's Inspector card is that rigid body, idle
        let mut h = H::new(1100.0);
        h.p.tracks[0].clips[1].start = 0.0; // both clips on screen together
        h.p.clip_mut(2).unwrap().blocks.clear();
        assert_eq!(h.p.collide_drop(&[1, 2], 1.0, Some(2.0), &Phys::default()), 2);
        let i = h.p.clip(2).unwrap().blocks.iter().position(|b| b.name == "Collide").unwrap();
        for _ in 0..3 {
            let (p, ctx) = (&mut h.p, &h.ctx);
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let pal = Palette::new(true, Color32::from_rgb(0x33, 0x66, 0xFF));
                    assert!(!block_controls(ui, p, (2, i), &pal, &mut |_| {}, &mut None), "nothing changed");
                });
            });
        }
        assert!(!h.ctx.has_requested_repaint());
        // the Inspector's Physics section (sliders + the preview curve) draws for every simulated block, idle
        for g in Gen::ALL {
            let mut h = H::new(1100.0);
            h.p.apply_key_block(2, &find_block(&[], g.name()).unwrap(), Some(2.0), None).unwrap();
            let i = h.p.clip(2).unwrap().blocks.iter().position(|b| b.name == g.name()).unwrap();
            let (p, ctx) = (&mut h.p, &h.ctx);
            for _ in 0..3 {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let pal = Palette::new(true, Color32::from_rgb(0x33, 0x66, 0xFF));
                        block_controls(ui, p, (2, i), &pal, &mut |_| {}, &mut None);
                    });
                });
            }
            assert!(!ctx.has_requested_repaint(), "{g:?}: an idle Physics section must not repaint");
        }
    }

    /// Every kind of animation has a colour of its own (the bar, the pill, the Gallery card and the menu dot
    /// all use it), and every shipped block lands in a kind.
    #[test]
    fn every_kind_of_animation_has_its_own_colour() {
        const KINDS: [&str; 12] = [
            "In", "Out", "Physics", "Collide", "Emphasis", "Camera", "Text", "Sound", "Speed", "Custom", "Path", "Transition",
        ];
        for (i, a) in KINDS.iter().enumerate() {
            for b in &KINDS[i + 1..] {
                let (ca, cb) = (kind_color(a), kind_color(b));
                let far = (0..3).map(|k| (ca.to_array()[k] as i32 - cb.to_array()[k] as i32).abs()).sum::<i32>();
                assert!(far > 90, "{a} and {b} look alike: {ca:?} {cb:?}");
            }
        }
        for (name, kind) in [
            ("Fade In", "In"),
            ("Slide Out Right", "Out"),
            ("Gravity Fall", "Physics"),
            ("Collide", "Collide"),
            ("Pulse", "Emphasis"),
            ("Ken Burns", "Camera"),
            ("Camera Shake", "Camera"),
            ("Typewriter", "Text"),
            ("Duck", "Sound"),
            ("Speed Ramp In", "Speed"),
            ("Custom", "Custom"),
        ] {
            let gen = find_block(&[], name).and_then(|b| b.gen);
            assert_eq!(kind_of(name, gen), (kind, kind_color(kind)), "{name}");
        }
        for b in builtin_blocks() {
            assert!(KINDS.contains(&kind_of(&b.name, b.gen).0) && kind_of(&b.name, b.gen).0 != "Custom", "{}", b.name);
        }
        // a Path block is its own kind whatever its name says
        let mut p = Project::new();
        p.tracks[0].clips.push(Clip::new(1, ClipKind::Video, "v", 0.0, 6.0));
        let path = PathCtl { points: vec![(0.0, 0.0, 0.0), (100.0, 0.0, 1.0)], ..Default::default() };
        let i = p.add_path_block(1, path, 0.0, 1.0).unwrap();
        assert_eq!(block_kind(&p.clip(1).unwrap().blocks[i]), ("Path", kind_color("Path")));
    }

    /// The Timeline lane: a span's bar drags like a block (its transitions follow on release), Delete removes
    /// it and what it made, and the lane is there with nothing selected - without asking for frames.
    #[test]
    fn assert_no_idle_repaint_animator_timeline_lane_and_a_span_drags_and_deletes() {
        let count = |p: &Project| p.tracks.iter().map(|t| t.transitions.len()).sum::<usize>();
        let mut h = H::new(1100.0);
        (h.st.whole, h.st.win, h.st.win_for) = (true, Some((0.0, 12.0)), vec![1, 2]);
        let s = h.p.add_span(5.0, 2.0, TransitionKind::Push);
        assert_eq!(count(&h.p), 1, "the cut at 6 s is under it");
        h.frame();
        h.frame();
        let r = h.st.span_pos.iter().find(|x| x.0 == s).expect("the bar is drawn").1;
        let (c, dx) = (r.center(), r.width() * 1.5); // 3 s to the right
        let mut edited = false;
        for ev in [
            vec![egui::Event::PointerMoved(c)],
            vec![button(c, true)],
            vec![egui::Event::PointerMoved(c + vec2(dx / 2.0, 0.0))],
            vec![egui::Event::PointerMoved(c + vec2(dx, 0.0))],
            vec![button(c + vec2(dx, 0.0), false)],
            vec![],
        ] {
            let out = h.frame_with(ev, false);
            // (the app's edit hook)
            if out.edited {
                h.p.refresh_spans();
                edited = true;
            }
        }
        let sp = h.p.spans[0].clone();
        assert!(edited && (sp.t - 8.0).abs() < 0.05 && (sp.dur - 2.0).abs() < 1e-9, "{sp:?}");
        assert_eq!((count(&h.p), h.st.spans.clone(), selected_span()), (0, vec![s], Some(s)), "off the cut, picked");
        // nothing selected: the lane is still there, idle
        h.sel.clear();
        h.frame_with(vec![egui::Event::PointerGone], false);
        for _ in 0..30 {
            h.frame();
        }
        assert!(h.st.span_pos.iter().any(|x| x.0 == s), "the lane draws without a selection");
        assert!(!h.ctx.has_requested_repaint(), "an idle Timeline lane must not repaint");
        // Delete takes the picked span
        let key = egui::Event::Key { key: egui::Key::Delete, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        assert!(h.frame_with(vec![key], false).edited);
        assert!(h.p.spans.is_empty() && h.st.spans.is_empty());
    }

    /// Three clips on V1, a cross fade on the cut at 6 s and a shader transition on the one at 12 s; only the
    /// middle clip selected (the user's report: its Animator lane showed nothing).
    fn with_transitions() -> (H, Id, Id) {
        let mut h = H::new(1100.0);
        h.p.tracks[0].clips.push(Clip::new(3, ClipKind::Video, "Gameplay C", 12.0, 6.0));
        let fade = h.p.add_transition(2, TransitionKind::CrossFade, 1.0).unwrap();
        let cube = h.p.add_transition(3, TransitionKind::CrossFade, 0.8).unwrap();
        let x = h.p.transition_mut(cube).unwrap();
        (x.kind, x.shader) = (TransitionKind::Shader, "// @name Cube\n".into());
        h.sel = vec![2];
        (h.p.clip_mut(2).unwrap().blocks, h.ph) = (Vec::new(), 6.0);
        (h, fade, cube)
    }

    #[test]
    fn a_clip_with_transitions_shows_them_and_the_clips_they_join() {
        let (mut h, fade, cube) = with_transitions();
        h.frame();
        h.frame();
        // the selected clip's lane and both neighbours' lanes each carry the transitions they share
        let mut on: Vec<(Id, Id)> = h.st.trans_pos.iter().map(|x| (x.0, x.1)).collect();
        on.sort();
        assert_eq!(on, [(fade, 1), (fade, 2), (cube, 2), (cube, 3)]);
        assert_eq!(h.st.win_for, vec![1, 2, 3], "the neighbours are lanes");
        // laid out in time: the fade's bar straddles the cut at 6 s, a second wide, the same on both lanes
        let bar = |h: &H, t: Id, c: Id| h.st.trans_pos.iter().find(|x| x.0 == t && x.1 == c).unwrap().2;
        let (a, b) = (bar(&h, fade, 1), bar(&h, fade, 2));
        assert_eq!((a.left(), a.right()), (b.left(), b.right()));
        let (t0, t1) = h.st.win.unwrap();
        assert!(t0 <= 5.5 && t1 >= 6.5, "the opening window shows it: {t0} {t1}");
        let w: Vec<(f64, f64)> = clip_transitions(&h.p, 2).iter().map(|x| x.1).collect();
        let want = [(5.5, 6.5), (11.6, 12.4)];
        assert!(w.len() == 2 && w.iter().zip(want).all(|(a, b)| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9), "{w:?}");
        // a click picks it for the Inspector
        let c = b.center();
        for ev in [vec![egui::Event::PointerMoved(c)], vec![button(c, true)], vec![button(c, false)], vec![]] {
            h.frame_with(ev, false);
        }
        assert_eq!((h.st.trans, selected_transition()), (Some(fade), Some(fade)));
        assert_eq!(h.undos, 0, "picking is not an edit");
    }

    #[test]
    fn dragging_a_transitions_end_in_the_animator_changes_its_length_in_one_undo_step() {
        let (mut h, fade, _) = with_transitions();
        h.frame();
        h.frame();
        let r = h.st.trans_pos.iter().find(|x| x.0 == fade && x.1 == 2).unwrap().2;
        let pps = r.width(); // the bar is one second wide
        let end = pos2(r.right() - 2.0, r.center().y);
        let mut edited = false;
        for ev in [
            vec![egui::Event::PointerMoved(end)],
            vec![button(end, true)],
            vec![egui::Event::PointerMoved(end + vec2(pps * 0.25, 0.0))],
            vec![egui::Event::PointerMoved(end + vec2(pps * 0.5, 0.0))],
            vec![button(end + vec2(pps * 0.5, 0.0), false)],
            vec![],
        ] {
            edited |= h.frame_with(ev, false).edited;
        }
        // the end went half a second right: the window grows both ways from the cut, 1 s -> 2 s
        let d = h.p.tracks[0].transitions.iter().find(|x| x.id == fade).unwrap().duration;
        assert!(edited && (d - 2.0).abs() < 0.05, "{d}");
        assert_eq!(h.undos, 1, "one drag, one undo step");
        assert_eq!(h.st.trans, Some(fade));
        // Delete removes the picked transition
        let key = egui::Event::Key { key: egui::Key::Delete, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        assert!(h.frame_with(vec![key], false).edited);
        assert!(h.p.tracks[0].transitions.iter().all(|x| x.id != fade) && h.st.trans.is_none());
    }

    /// Curves, one clip, the pointer over key `k` of Position X: (harness, the key's screen position).
    fn curves_on_x(sel: Option<(Id, usize)>, k: usize) -> (H, Pos2) {
        let mut h = H::new(1100.0);
        (h.st.view, h.st.sel, h.sel, h.ph) = (View::Curves, sel, vec![1], 5.0);
        h.frame();
        h.frame();
        let at = h.st.key_pos.iter().find(|p| (p.0, p.1, p.2) == (1, "x", k)).expect("the key is drawn").3;
        (h, at)
    }
    fn button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }
    fn x_keys(h: &H) -> Vec<(f64, f64)> {
        h.p.clip(1).unwrap().x.keys.iter().map(|k| (k.t, k.v)).collect()
    }

    /// The report: keys in Curves could not be dragged. With no block picked in the Animator the selected keys
    /// were thrown away at the start of every frame, so a drag had nothing to move after its first frame.
    #[test]
    fn a_key_in_curves_drags_in_time_and_value() {
        for sel in [None, Some((1, 0))] {
            let (mut h, at) = curves_on_x(sel, 1);
            let before = x_keys(&h);
            h.frame_with(vec![egui::Event::PointerMoved(at)], false);
            h.frame_with(vec![button(at, true)], false);
            let to = at + vec2(-40.0, 30.0);
            h.frame_with(vec![egui::Event::PointerMoved(at + vec2(-20.0, 15.0))], false);
            let out = h.frame_with(vec![egui::Event::PointerMoved(to)], false);
            assert!(out.edited, "{sel:?}: the drag edits the clip");
            h.frame_with(vec![button(to, false)], false);
            h.frame();
            let after = x_keys(&h);
            assert_eq!(after[0], before[0], "{sel:?}: the key that was not picked stays");
            assert!(after[1].0 < before[1].0 - 0.05, "{sel:?}: earlier in time: {before:?} -> {after:?}");
            assert!((after[1].1 - before[1].1).abs() > 50.0, "{sel:?}: another value: {before:?} -> {after:?}");
            assert_eq!((h.st.keys.clone(), h.st.key_clip), (vec![("x", 1)], Some(1)), "{sel:?}: it stays selected");
        }
    }

    /// Shift locks a key drag to the axis the pointer has mostly moved along; letting go frees it again.
    #[test]
    fn shift_locks_a_key_drag_to_one_axis() {
        // mostly sideways: time only
        let (mut h, at) = curves_on_x(None, 1);
        let before = x_keys(&h);
        h.frame_with(vec![egui::Event::PointerMoved(at)], false);
        h.frame_with(vec![button(at, true)], true);
        h.frame_with(vec![egui::Event::PointerMoved(at + vec2(-40.0, 6.0))], true);
        // the lock holds even when the pointer then wanders the other way
        h.frame_with(vec![egui::Event::PointerMoved(at + vec2(-40.0, 60.0))], true);
        let mid = x_keys(&h);
        assert!(mid[1].0 < before[1].0 - 0.05 && mid[1].1 == before[1].1, "time only: {before:?} -> {mid:?}");
        // Shift let go: free again
        h.frame_with(vec![egui::Event::PointerMoved(at + vec2(-40.0, 61.0))], false);
        assert!((x_keys(&h)[1].1 - before[1].1).abs() > 50.0, "{:?}", x_keys(&h));
        h.frame_with(vec![button(at + vec2(-40.0, 61.0), false)], false);
        // mostly up and down: value only; Shift pressed after the drag began decides from the movement so far
        let (mut h, at) = curves_on_x(None, 1);
        h.frame_with(vec![egui::Event::PointerMoved(at)], false);
        h.frame_with(vec![button(at, true)], false);
        h.frame_with(vec![egui::Event::PointerMoved(at + vec2(-5.0, 40.0))], false);
        h.frame_with(vec![egui::Event::PointerMoved(at + vec2(-6.0, 60.0))], true);
        h.frame_with(vec![egui::Event::PointerMoved(at + vec2(-60.0, 62.0))], true);
        let end = x_keys(&h);
        assert!(end[1].0 == before[1].0 && (end[1].1 - before[1].1).abs() > 50.0, "value only: {before:?} -> {end:?}");
        h.frame_with(vec![button(at + vec2(-60.0, 62.0), false)], false);
        // Shift-click still adds to the selection (no drag: no lock)
        let (mut h, at) = curves_on_x(None, 0);
        let at = at + vec2(4.0, 0.0); // the key sits on the lanes' left edge: a little inside it
        (h.st.keys, h.st.key_clip) = (vec![("x", 1)], Some(1));
        h.frame_with(vec![egui::Event::PointerMoved(at)], true);
        h.frame_with(vec![button(at, true)], true);
        h.frame_with(vec![button(at, false)], true);
        h.frame();
        assert_eq!(h.st.keys, vec![("x", 1), ("x", 0)]);
        assert_eq!(x_keys(&h), before, "a click moves nothing");
    }

    /// Dragging a row of the channel list reorders it: a property inside its group, a group among the groups;
    /// the graph draws its curves in that order.
    #[test]
    fn the_channel_list_reorders_by_drag_and_by_alt_arrows() {
        use crate::ui::list_rows::Moved;
        // as drawn for a picture clip: Transform (x y scale scale_x scale_y rotation), Opacity (opacity), Speed
        let groups = ch_groups(&[]);
        assert_eq!(groups.iter().map(|g| g.0).collect::<Vec<_>>(), ["Transform", "Opacity", "Sound", "Speed"]);
        let rows: Vec<(usize, Option<&'static str>)> = vec![
            (0, None),
            (0, Some("x")),
            (0, Some("y")),
            (0, Some("scale")),
            (0, Some("scale_x")),
            (0, Some("scale_y")),
            (0, Some("rotation")),
            (1, None),
            (1, Some("opacity")),
            (3, None),
            (3, Some("speed")),
        ];
        let props = |o: &[&'static str], g: &str| ch_groups(o).into_iter().find(|x| x.0 == g).unwrap().1;
        let names = |o: &[&'static str]| ch_groups(o).iter().map(|g| g.0).collect::<Vec<_>>();
        // Rotation dragged above Position Y (dropped before row 2)
        let o = ch_moved(groups.clone(), &rows, &Moved::Slot { from: 6, slot: 2, depth: None });
        assert_eq!(props(&o, "Transform"), ["x", "rotation", "y", "scale", "scale_x", "scale_y"]);
        assert_eq!(names(&o), ["Transform", "Opacity", "Sound", "Speed"], "the groups stay");
        // Position X dragged to the bottom of the list: the last of its own group, never into another
        let o = ch_moved(groups.clone(), &rows, &Moved::Slot { from: 1, slot: 11, depth: None });
        assert_eq!(props(&o, "Transform"), ["y", "scale", "scale_x", "scale_y", "rotation", "x"]);
        assert_eq!(props(&o, "Opacity"), ["opacity"]);
        // the Speed group dragged to the top; Opacity dragged below Speed
        let o = ch_moved(groups.clone(), &rows, &Moved::Slot { from: 9, slot: 0, depth: None });
        assert_eq!(names(&o), ["Speed", "Transform", "Opacity", "Sound"]);
        let o = ch_moved(groups.clone(), &rows, &Moved::Slot { from: 7, slot: 11, depth: None });
        assert_eq!(names(&o), ["Transform", "Sound", "Speed", "Opacity"]);
        // Alt+Down on Position X, Alt+Up on the Opacity group; at an end nothing moves
        let o = ch_moved(groups.clone(), &rows, &Moved::Step { from: 1, delta: 1 });
        assert_eq!(props(&o, "Transform")[..2], ["y", "x"]);
        let o = ch_moved(groups.clone(), &rows, &Moved::Step { from: 7, delta: -1 });
        assert_eq!(names(&o), ["Opacity", "Transform", "Sound", "Speed"]);
        let o = ch_moved(groups.clone(), &rows, &Moved::Step { from: 1, delta: -1 });
        assert_eq!(props(&o, "Transform"), props(&[], "Transform"));
        // the order reaches the list and the graph: scale before x on clip 1 once dragged there
        let mut h = H::new(1100.0);
        h.st.view = View::Curves;
        h.st.ch_order = ch_moved(groups, &rows, &Moved::Slot { from: 3, slot: 1, depth: None });
        let cx = curve_ctx(&h.st, h.p.clip(1).unwrap());
        assert_eq!(cx.channels[..3], ["scale", "x", "y"]);
        assert_eq!(cx.shown, ["scale", "x"], "clip 1 animates scale (Zoom Punch) and x (Slide In Left)");
        h.frame();
    }

    #[test]
    fn every_view_draws_and_the_selection_reaches_the_inspector() {
        for view in [View::Blocks, View::Curves, View::Flow, View::Speed] {
            let mut h = H::new(1100.0);
            h.st.view = view;
            h.st.sel = Some((2, 0));
            h.frame();
            assert_eq!(selected_block(), Some((2, 0)), "{view:?}");
        }
        // a selection that no longer exists is dropped
        let mut h = H::new(1100.0);
        h.st.sel = Some((1, 9));
        h.frame();
        assert_eq!(selected_block(), None);
    }

    #[test]
    fn overlapping_blocks_stack_in_rows_and_a_chain_stays_on_one() {
        let f = |n| find_block(&[], n).unwrap();
        let mut c = Clip::new(1, ClipKind::Video, "a", 0.0, 6.0);
        let mut p = Project::new();
        c.blocks.clear();
        p.tracks[0].clips.push(c);
        p.apply_key_block(1, &f("Fade In"), Some(0.0), None);
        p.apply_key_block(1, &f("Fade Out"), None, None);
        p.place_key_block(1, &f("Speed Ramp Out"), 0.2, Some(0.5));
        let b = &p.clip(1).unwrap().blocks;
        assert_eq!(lane_rows(b), (vec![0, 1, 0], vec!["Opacity", "Speed"]), "{b:?}");
    }

    /// The lanes open framed on the blocks, not the whole clip: short blocks on a long clip stay readable.
    #[test]
    fn the_lanes_open_on_the_blocks_and_a_shake_has_no_side() {
        let f = |n| find_block(&[], n).unwrap();
        let mut p = Project::new();
        p.tracks[0].clips.push(Clip::new(1, ClipKind::Video, "a", 0.0, 30.0));
        let fit = range(&p, &[1]);
        assert_eq!(block_fit(&p, &[1], fit), fit, "no blocks: the whole clip");
        // zoomed all the way out is exactly the selected clips: no air around them
        p.tracks[0].clips.push(Clip::new(2, ClipKind::Video, "b", 40.0, 4.0));
        p.tracks[0].clips.push(Clip::new(3, ClipKind::Video, "c", 50.0, 2.0));
        assert_eq!(range(&p, &[2]), (40.0, 44.0));
        assert_eq!(range(&p, &[3, 2]), (40.0, 52.0));
        p.place_key_block(1, &f("Pop In"), 10.0, Some(0.4));
        p.place_key_block(1, &f("Slide Out Right"), 12.0, Some(0.6));
        let (a, b) = block_fit(&p, &[1], fit);
        assert!(a <= 10.0 && a >= fit.0 && b <= fit.1, "{a} {b}");
        // the shortest block (0.4 s) is at least a fourteenth of the window
        assert!(b - a <= 0.4 * 14.0 + 1e-9 && b - a >= 0.4, "{a} {b}");
        // zoomed all the way out is exactly the range; a window hanging over an end slides back in
        assert_eq!(clamp_win((-5.0, 50.0), (2.0, 8.0)), (2.0, 8.0));
        assert_eq!(clamp_win((7.0, 9.0), (2.0, 8.0)), (6.0, 8.0));
        assert!(wiggles(&BlockCtl::new(f("Shake"))));
        assert!(!wiggles(&BlockCtl::new(f("Slide In Left"))) && !wiggles(&BlockCtl::new(f("Fade Out"))));
    }

    #[test]
    fn a_long_clip_name_is_cut_to_the_label_column() {
        let ctx = egui::Context::default();
        let long = "Rodev GTA 6 Gameswap - FINAL v2 1080p60.mp4";
        let mut widths = Vec::new();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for w in [40.0, 90.0, 140.0] {
                    let g = elided(ui, long, FontId::proportional(12.0), Color32::WHITE, w);
                    widths.push((w, g.size().x, g.rows.len(), g.text().to_string()));
                }
            });
        });
        for (w, got, rows, text) in widths {
            assert!(got <= w + 0.5, "{got} > {w}: {text}");
            assert_eq!(rows, 1, "one line");
            assert!(text.ends_with('…') || text == long, "{text}");
        }
        // and in the pane: a lane whose clip has that name still draws (the column is the clamp)
        let mut h = H::new(1100.0);
        h.p.tracks[0].clips[0].name = long.into();
        h.frame();
    }

    #[test]
    fn the_cut_between_a_and_b_shows_as_a_hand_off() {
        let h = H::new(1100.0);
        let s = h.p.block_seams();
        assert!(s.iter().any(|s| s.left.0 == 1 && s.right.0 == 2), "{s:?}");
    }
}
