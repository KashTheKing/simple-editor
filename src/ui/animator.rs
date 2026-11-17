//! ---- ws:animator ----
//! The Animator pane: the animation timeline for the selected clips, the Animate page's main panel.
//! - Blocks: one lane per selected clip, a bar per applied block (drag to move, drag an end to stretch, ▸ opens
//!   its keys), a ⚡ / ✓ badge wherever one move hands over to the next (click: match their speeds).
//! - Curves: the selected block's keys as curves, every segment's bezier handles draggable, box / Shift select,
//!   ease presets, value or velocity.
//! - Flow: how fast the picture moves across the selected clip and the cut into the next one, every hand-off
//!   with its two speeds and a Smooth button.
//! - Speed: the clip's speed ramp.
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

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum View {
    #[default]
    Blocks,
    Curves,
    Flow,
    Speed,
}

#[derive(Default)]
pub struct AnimState {
    pub view: View,
    pub sel: Option<(Id, usize)>,
    open: Vec<(Id, usize)>,
    /// Curves: selected keys as (property, key index) of the selected block's clip.
    keys: Vec<(&'static str, usize)>,
    focus: Option<&'static str>,
    velocity: bool,
    pub mode: Match,
    /// The clip as it was when a drag started (a drag re-applies from it every frame: one undo step).
    orig: Option<Clip>,
    guide: Option<f64>,
    boxsel: Option<(Pos2, Pos2)>,
    ranges: Option<Vec<(&'static str, f64, f64)>>,
    speed_sel: Option<usize>,
    /// Onion-skin ghosts each side (`onion`); `None` = the default, off.
    pub ghosts: Option<u8>,
    /// Width of the lanes' label column (the splitter between labels and lanes); `None` = `HEAD_W`.
    pub head_w: Option<f32>,
    /// Curves: properties hidden on the graph, folded channel groups, every key of the clip (not just the
    /// selected block's), the key / handle a drag holds, where a right-click on empty space landed.
    hidden: Vec<&'static str>,
    folded: Vec<&'static str>,
    whole_clip: bool,
    grab: Option<(&'static str, usize, i8)>,
    ctx_t: Option<f64>,
    /// The time window shown (timeline seconds) after a zoom / pan; `None` = fit.
    pub win: Option<(f64, f64)>,
    /// The clips `win` was framed for: another selection frames its own blocks again.
    win_for: Vec<Id>,
    /// Blocks: the lanes span the whole timeline (else just the selected clips).
    pub whole: bool,
    /// More blocks picked with Shift / Ctrl-click (`sel` is the main one, the Inspector's).
    pub multi: Vec<(Id, usize)>,
    /// The app's snapping setting (the caller sets it each frame): off = a dragged block snaps to nothing.
    pub snap: bool,
    /// Curves: width of the channel list (the splitter between it and the graph); `None` = 300.
    pub ch_w: Option<f32>,
    /// The last press was in the Animator: Delete / Backspace / Split act on its block or keys, not the clip.
    pub here: bool,
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

fn flow_act(sel: Vec<(Id, usize)>, mode: Match) -> Act {
    Box::new(move |p, _, o| {
        let n = p.flow_key_blocks(&sel, mode);
        o.toast = Some(match n {
            0 => "Nothing to flow: pick blocks on one clip, or either side of a cut".into(),
            _ => format!("Flowed {n} hand-off{}: each block starts where the one before ends", if n == 1 { "" } else { "s" }),
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
    pub with_overlays: bool,
    /// Make the selected clips follow this saved path (and turn along it when true).
    pub follow: Option<(Id, bool)>,
    /// The next stroke drawn in the Viewer becomes a motion path the selected clips follow.
    pub draw_path: bool,
}

const HEAD_W: f32 = 132.0;
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
fn prop_color(p: &str) -> Color32 {
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
fn block_col(b: &AppliedBlock) -> Color32 {
    let [r, g, bl] = block_color(&b.name);
    Color32::from_rgb(r, g, bl)
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

/// The kind of a block and its colour: In teal, Out orange, Emphasis yellow, Camera blue, Physics purple,
/// Sound green, Speed coral; a saved one keeps its name's colour.
pub fn block_kind(b: &AppliedBlock) -> (&'static str, Color32) {
    let gen = b.ctl.as_ref().and_then(|c| c.src.gen);
    kind_of(&b.name, gen)
}
pub fn kind_of(name: &str, gen: Option<Gen>) -> (&'static str, Color32) {
    let c = |r, g, b| Color32::from_rgb(r, g, b);
    if let Some(g) = gen {
        return if g == Gen::Shake { ("Camera", c(0x33, 0x66, 0xFF)) } else { ("Physics", c(0x9B, 0x5D, 0xE5)) };
    }
    let n = name.to_ascii_lowercase();
    let has = |w: &str| n.contains(w);
    if has("speed") || has("slow-mo") {
        ("Speed", c(0xEF, 0x7A, 0x5A))
    } else if has("sound") || has("duck") || has("pan ") || has("tremolo") {
        ("Sound", c(0x4C, 0xC0, 0x6A))
    } else if has(" in") || has("drop") || has("elastic") || has("typewriter") {
        ("In", c(0x2B, 0xB3, 0xA3))
    } else if has(" out") || has("whip") {
        ("Out", c(0xE0, 0x60, 0x2A))
    } else if has("ken burns") || has("drift") || has("zoom") || has("camera") {
        ("Camera", c(0x33, 0x66, 0xFF))
    } else if ["pulse", "shake", "wiggle", "bounce", "spin", "flash"].iter().any(|w| has(w)) {
        ("Emphasis", c(0xF2, 0xC1, 0x4E))
    } else {
        let [r, g, bl] = block_color(name);
        ("Mine", c(r, g, bl))
    }
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
        egui::Button::new(egui::RichText::new(text).size(12.0).color(Color32::WHITE)).fill(accent).corner_radius(6.0),
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
            let t = egui::RichText::new(n).size(12.0).color(if on { Color32::WHITE } else { look::MUTED });
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
    let (r, resp) = ui.allocate_exact_size(vec2(170.0, 130.0), Sense::hover());
    let p = ui.painter_at(r);
    p.rect(r, 6.0, look::DEEP, Stroke::new(1.0, look::LINE), egui::StrokeKind::Inside);
    let plot = Rect::from_min_max(r.min + vec2(12.0, 26.0), r.max - vec2(34.0, 24.0));
    let at = |x: f32, y: f32| pos2(plot.left() + x * plot.width(), plot.bottom() - y * plot.height());
    p.rect_stroke(plot, 0.0, Stroke::new(1.0, look::TRACK), egui::StrokeKind::Inside);
    let (x1, y1, x2, y2) = e.handles();
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
        let hr = ui
            .interact(Rect::from_center_size(c, vec2(14.0, 14.0)), id.with(k), Sense::drag())
            .on_hover_cursor(CursorIcon::Grab);
        started |= hr.drag_started();
        if let Some(pp) = hr.interact_pointer_pos().filter(|_| hr.dragged()) {
            let fx = ((pp.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
            let fy = ((plot.bottom() - pp.y) / plot.height()).clamp(-0.8, 1.8);
            out = Some(if k == 1 {
                Ease::Bezier { x1: fx, y1: fy, x2, y2 }
            } else {
                Ease::Bezier { x1, y1, x2: fx, y2: fy }
            });
        }
        p.circle_filled(c, 5.5, look::HANDLE);
    }
    // the live preview: a dot running the curve, and a bar showing where the value is
    if resp.hovered() || ui.rect_contains_pointer(r) {
        let ph = ((ui.input(|i| i.time) % 1.6) / 1.2).min(1.0);
        let v = e.apply(ph) as f32;
        p.circle_filled(at(ph as f32, v), 4.0, Color32::WHITE);
        let bar = Rect::from_min_max(pos2(r.right() - 20.0, plot.top()), pos2(r.right() - 10.0, plot.bottom()));
        p.rect_filled(bar, 3.0, look::CARD);
        let y = plot.bottom() - v.clamp(-0.2, 1.2) * plot.height();
        p.rect_filled(Rect::from_center_size(pos2(bar.center().x, y), vec2(8.0, 6.0)), 2.0, accent);
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
    // a selected block that went away (undo, delete, another clip) is forgotten
    st.sel = st.sel.filter(|&(id, i)| ids.contains(&id) && project.clip(id).is_some_and(|c| i < c.blocks.len()));
    if st.sel.is_none() {
        st.keys.clear();
        st.multi.clear();
    }
    let main = st.sel;
    st.multi.retain(|&(id, i)| Some((id, i)) != main && project.clip(id).is_some_and(|c| i < c.blocks.len()));
    SEL.with(|s| s.set(st.sel));
    set_onion(st.ghosts.unwrap_or(0));
    let compact = ui.available_width() < COMPACT_W;
    header(ui, st, project, &ids, *playhead, compact, presets, undo, &mut out);
    ui.separator();
    if ids.is_empty() {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("Select clips on the timeline to animate them.").color(pal.text_dim));
            if ui.button("Select the clip under the playhead and its overlays").clicked() {
                out.with_overlays = true;
            }
        });
        return out;
    }
    match st.view {
        _ if compact => compact_list(ui, st, project, &ids, playhead, pal, undo, &mut out),
        View::Blocks => blocks(ui, st, project, &ids, playhead, pal, undo, &mut out),
        View::Curves => curves(ui, st, project, &ids, playhead, pal, undo, &mut out),
        View::Flow => flow(ui, st, project, &ids, playhead, pal, undo, &mut out),
        View::Speed => speed_view(ui, st, project, &ids, playhead, pal, undo, &mut out),
    }
    // Delete / Backspace: the picked blocks and only them (selected keys or a speed point took the key first)
    if !playing && st.sel.is_some() && (st.here || ui.ui_contains_pointer()) && del_pressed(ui) {
        undo(project);
        remove_picked(project, st);
        out.edited = true;
    }
    SEL.with(|s| s.set(st.sel));
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
    ui.horizontal(|ui| {
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
        let r = menu::button(ui, egui::RichText::new("+ Animation").color(Color32::WHITE), |ui| {
            let mut pick = None;
            let groups: [(&str, &[&str]); 7] = [
                ("In", &["Fade In", "Pop In", "Slide In Left", "Drop In", "Swing In", "Elastic", "Spring In"]),
                ("Out", &["Fade Out", "Pop Out", "Slide Out Right", "Whip", "Throw Out"]),
                ("Emphasis", &["Pulse", "Zoom Punch", "Shake", "Wiggle", "Bounce", "Spin", "Flash", "Wobble"]),
                ("Camera", &["Camera Shake", "Ken Burns", "Drift", "Bounce Drop"]),
                ("Text", &["Typewriter"]),
                ("Sound", &["Sound Swell", "Sound Fade", "Duck", "Pan Sweep", "Pan Swing", "Tremolo"]),
                ("Speed", &["Speed Ramp In", "Speed Ramp Out", "Slow-mo Hit", "Speed Up"]),
            ];
            for (g, names) in groups {
                menu::sub(ui, None, g, |ui| {
                    for n in names {
                        if menu::row(ui, None, n, "").clicked() {
                            pick = Some(n.to_string());
                        }
                    }
                });
            }
            if !presets.is_empty() {
                menu::sub(ui, None, "Mine", |ui| {
                    for b in presets {
                        if menu::row(ui, None, &b.name, "").clicked() {
                            pick = Some(b.name.clone());
                        }
                    }
                });
            }
            pick
        });
        ui.visuals_mut().widgets = saved;
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
        let rec = crate::model::recording();
        // ponytail: the dot is painted over the chip's leading spaces instead of a chip that takes an icon
        let r = chip(ui, rec, "    Record").on_hover_text(
            "On: changing a value at the playhead adds a key there. Off: keys are only placed by hand (◆, double-click)",
        );
        ui.painter().circle_filled(pos2(r.rect.left() + 11.0, r.rect.center().y), 3.5, if rec { BAD } else { look::DIM });
        if r.clicked() {
            crate::model::set_recording(!rec);
        }
        menu::button(ui, "⋯", |ui| {
            if menu::row(ui, None, "Select clip + overlays", "").clicked() {
                out.with_overlays = true;
            }
            if menu::row(ui, None, "Stack selection (moves as one in transitions)", "").clicked() {
                out.stack = true;
            }
            ui.separator();
            if menu::row(ui, None, "Draw a motion path…", "").on_hover_text("Draw in the Viewer with the pencil: the selected clips follow what you draw, at your drawing's pace").clicked() {
                out.draw_path = true;
            }
            menu::sub(ui, None, "Follow a path", |ui| {
                if project.paths.is_empty() {
                    ui.weak("No saved paths yet - draw one first");
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
        if st.view == View::Flow || compact {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| mode_seg(ui, st));
        }
    });
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

fn mode_seg(ui: &mut Ui, st: &mut AnimState) {
    for (m, n, tip) in [
        (Match::Both, "both", "Both moves change to meet halfway"),
        (Match::Earlier, "the earlier move", "The earlier move ends at the speed the next one starts at"),
        (Match::Next, "the next move", "The next move starts at the speed the earlier one ends at"),
    ] {
        if chip(ui, st.mode == m, n).on_hover_text(tip).clicked() {
            st.mode = m;
        }
    }
    ui.label(egui::RichText::new("Smooth by easing").size(11.0).weak());
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

/// The timeline range the lanes show: the selected clips plus a little air.
fn range(project: &Project, ids: &[Id]) -> (f64, f64) {
    let (a, b) = ids
        .iter()
        .filter_map(|&id| project.clip(id))
        .fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c.start), b.max(c.end())));
    let pad = ((b - a) * 0.04).max(0.1);
    ((a - pad).max(0.0), b + pad)
}

/// The window the lanes open on: the clips' blocks with a little air, never so wide that the shortest block
/// is a sliver (at most 14 of it across), inside `fit`. No blocks: `fit`.
fn block_fit(project: &Project, ids: &[Id], fit: (f64, f64)) -> (f64, f64) {
    let spans: Vec<(f64, f64)> = ids
        .iter()
        .filter_map(|&id| project.clip(id))
        .flat_map(|c| c.blocks.iter().map(move |b| (c.start + b.t, c.start + b.end())))
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
    // a label centred on the first / last tick hangs over the edge: let it (else "0s" reads "s")
    let p = ui.painter_at(rect.expand2(vec2(14.0, 0.0)));
    let x = |t: f64| rect.left() + ((t - t0) / (t1 - t0)) as f32 * rect.width();
    for t in ticks(t0, t1, rect.width()) {
        p.line_segment([pos2(x(t), rect.bottom() - 5.0), pos2(x(t), rect.bottom())], Stroke::new(1.0, pal.border));
        p.text(
            pos2(x(t), rect.top() + 3.0),
            Align2::CENTER_TOP,
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
fn zoom_bar(ui: &mut Ui, pal: &Palette, win: &mut Option<(f64, f64)>, fit: (f64, f64), playhead: f64) {
    use crate::ui::tools::{icon_button, Glyph};
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
        });
    });
    if fitted || z <= 1.0 + 1e-6 && z != z0 {
        *win = Some(fit); // the whole range, kept (None would frame the blocks again)
    } else if z != z0 {
        let len = flen / z.min(400.0);
        let at = if (t0..=t1).contains(&playhead) { playhead } else { (t0 + t1) / 2.0 };
        let a = at - (at - t0) / (t1 - t0) * len;
        *win = Some((a, a + len));
    }
}

#[allow(clippy::too_many_arguments)]
fn blocks(
    ui: &mut Ui,
    st: &mut AnimState,
    project: &mut Project,
    ids: &[Id],
    playhead: &mut f64,
    pal: &Palette,
    undo: &mut dyn FnMut(&Project),
    out: &mut AnimResp,
) {
    let seams = project.block_seams();
    let (mut fit, mut flow_now, mut link_now) = ((0.0, 1.0), false, false);
    ui.horizontal(|ui| {
        if segmented(ui, &mut st.whole, &[(false, "Selected clips"), (true, "Whole timeline")]) {
            st.win = None;
        }
        if picked(st).len() > 1 {
            let tip = "Chain the picked blocks: each starts where the one before ends, at its speed (Smooth by easing sets which side gives way)";
            flow_now = primary(ui, "Flow together").on_hover_text(tip).clicked();
        }
        if linkable(st).is_some() {
            let tip = "Make the picked blocks one block with all their properties: they move, stretch and ease together";
            link_now = chip(ui, false, "Link into one block").on_hover_text(tip).clicked();
        }
        fit = if st.whole { (0.0, project.duration().max(1.0) * 1.02) } else { range(project, ids) };
        if st.win_for != ids {
            (st.win, st.win_for) = (None, ids.to_vec());
        }
        st.win = st.win.filter(|w| w.1 > fit.0 && w.0 < fit.1).or_else(|| Some(block_fit(project, ids, fit)));
        zoom_bar(ui, pal, &mut st.win, fit, *playhead);
    });
    let ph = *playhead;
    let (t0, t1) = st.win.unwrap_or(fit);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let full = ui.available_rect_before_wrap();
        let head_w = st.head_w.unwrap_or(HEAD_W).clamp(80.0, (full.width() * 0.5).max(80.0));
        let body_x0 = full.left() + head_w;
        let bw = (full.right() - body_x0).max(50.0);
        let x = |t: f64| body_x0 + ((t - t0) / (t1 - t0)) as f32 * bw;
        let pps = bw as f64 / (t1 - t0);
        let (rr, _) = ui.allocate_exact_size(vec2(full.width(), 20.0), Sense::hover());
        ruler(ui, Rect::from_min_max(pos2(body_x0, rr.top()), rr.max), t0, t1, playhead, pal, out);
        let top = rr.bottom();
        let mut act: Option<Act> = flow_now.then(|| flow_act(picked(st), st.mode));
        if link_now {
            act = linkable(st).map(link_act);
        }
        for &id in ids {
            let Some(c) = project.clip(id).cloned() else { continue };
            // a lane per kind of property (Motion / Opacity / Speed / Sound), more rows only where blocks
            // overlap, and a last row of keys made by hand (click one to make it a block)
            let (pack, labels) = lane_rows(&c.blocks);
            let loose = c.loose_key_spans();
            let keys_row = labels.len();
            let nrows = (labels.len() + (!loose.is_empty()) as usize).max(1);
            let (row, _) = ui
                .allocate_exact_size(vec2(full.width(), LANE_H + (nrows - 1) as f32 * (LANE_H - 8.0)), Sense::hover());
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
            let hy = row.top() + LANE_H / 2.0;
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
                let label = labels.get(r).copied().unwrap_or("Hand keys");
                let ry = row.top() + LANE_H / 2.0 + r as f32 * (LANE_H - 8.0);
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
            menu::context(&hr, |ui| {
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
            let p = ui.painter_at(body);
            // keys made by hand: a dashed "Hand keys" bar per cluster; a click makes it a block with controls
            for &(a, b) in &loose {
                let ktop = row.top() + 5.0 + keys_row as f32 * (LANE_H - 8.0);
                let kr = Rect::from_min_max(
                    pos2(x(c.start + a) - 4.0, ktop),
                    pos2(x(c.start + b) + 4.0, ktop + LANE_H - 10.0),
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
            for (i, b) in c.blocks.iter().enumerate() {
                let top = row.top() + 5.0 + pack[i] as f32 * (LANE_H - 8.0);
                let r =
                    Rect::from_min_max(pos2(x(c.start + b.t), top), pos2(x(c.start + b.end()), top + LANE_H - 10.0));
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
                inner.text(
                    pos2(tx0, r.center().y),
                    Align2::LEFT_CENTER,
                    &b.name,
                    FontId::proportional(11.0),
                    Color32::WHITE,
                );
                if edited && r.width() > 30.0 {
                    inner.circle_filled(pos2(r.right() - 8.0, r.center().y), 3.0, look::HANDLE);
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
                    resp.on_hover_cursor(if edge != 0 { CursorIcon::ResizeHorizontal } else { CursorIcon::Grab });
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
                let c0 = pos2(x(s.t), row.top() + LANE_H / 2.0);
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
        let split = Rect::from_min_max(pos2(body_x0 - 3.0, top), pos2(body_x0 + 3.0, bottom));
        let sr =
            ui.interact(split, ui.id().with("lane-split"), Sense::drag()).on_hover_cursor(CursorIcon::ResizeHorizontal);
        if sr.dragged() {
            st.head_w = Some((head_w + sr.drag_delta().x).clamp(80.0, (full.width() * 0.5).max(80.0)));
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
        if let Some(f) = act {
            undo(project);
            f(project, st, out);
            out.edited = true;
        }
    });
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
    SelectProp(&'static str),
    SelectAll,
    AddAt(&'static str, f64),
    Fit,
}

#[allow(clippy::too_many_arguments)]
fn curves(
    ui: &mut Ui,
    st: &mut AnimState,
    project: &mut Project,
    ids: &[Id],
    playhead: &mut f64,
    pal: &Palette,
    undo: &mut dyn FnMut(&Project),
    out: &mut AnimResp,
) {
    let id = st.sel.map(|s| s.0).unwrap_or(ids[0]);
    let Some(c) = project.clip(id).cloned() else { return };
    let picked = st.sel.filter(|s| s.0 == id).map(|s| s.1).filter(|&i| i < c.blocks.len());
    let bi = picked.filter(|_| !st.whole_clip);
    let (s0, s1) = curve_span(&c, bi);
    // what this clip can animate (sound props only on a sound clip), and what the graph shows of it
    let channels: Vec<&'static str> = PROPS
        .into_iter()
        .filter(|p| prop_ref(&c, p).is_some())
        .filter(|p| matches!(*p, "speed") || matches!(*p, "volume" | "pan") != c.is_visual())
        .collect();
    let keyed = |p: &str| prop_ref(&c, p).is_some_and(|a| a.is_animated());
    let shown: Vec<&'static str> = channels
        .iter()
        .copied()
        .filter(|p| !st.hidden.contains(p) && keyed(p))
        .filter(|p| bi.is_none_or(|i| c.blocks[i].props.iter().any(|q| q == p)))
        .collect();
    let focus = st.focus.filter(|f| channels.contains(f)).or(shown.first().copied()).or(channels.first().copied());
    st.focus = focus;
    // a window from another clip / view that misses this one is dropped
    st.win = st.win.filter(|w| w.1 > c.start && w.0 < c.end());

    // ---- header: what, Whole clip, Value / Velocity, ease presets, Fit ----
    ui.horizontal(|ui| {
        let title = match bi {
            Some(i) => format!("{} · {}", c.blocks[i].name, c.name),
            None => format!("{} · every key", c.name),
        };
        ui.label(egui::RichText::new(title).strong().color(look::TEXT));
        if picked.is_some() && chip(ui, st.whole_clip, "Whole clip").on_hover_text("Show every key of the clip, not just the block's").clicked() {
            st.whole_clip = !st.whole_clip;
            st.win = None;
        }
        segmented(ui, &mut st.velocity, &[(false, "Value"), (true, "Velocity")]);
        ui.add_space(4.0);
        ui.label(egui::RichText::new(if st.keys.is_empty() { "Ease all:" } else { "Ease selected:" }).size(11.0).color(look::MUTED));
        for (n, e) in Ease::PRESETS.into_iter().take(5).chain([("Linear", Ease::Linear), ("Hold", Ease::Hold)]) {
            if ease_chip(ui, false, n, e).clicked() {
                undo(project);
                // no keys picked: the whole block, through its Ease control when it has one (so it stays live)
                let live = |i: &usize| c.blocks[*i].ctl.as_ref().is_some_and(|c| !c.edited && !c.is_sim());
                if let Some(i) = bi.filter(|i| st.keys.is_empty() && live(i)) {
                    project.ease_key_block(id, i, e);
                } else if let Some(cm) = project.clip_mut(id) {
                    for &p in &shown {
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
                out.edited = true;
            }
        }
        if chip(ui, false, "Fit").on_hover_text("Show the whole span (F)").clicked() {
            st.win = None;
        }
        if bi.is_none() && !st.keys.is_empty() {
            let ts: Vec<f64> =
                st.keys.iter().filter_map(|&(p, k)| prop_ref(&c, p).and_then(|a| a.keys.get(k)).map(|k| k.t)).collect();
            let (a, b) = ts.iter().fold((f64::MAX, f64::MIN), |(a, b), &t| (a.min(t), b.max(t)));
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

    ui.horizontal_top(|ui| {
        let w = st.ch_w.unwrap_or(300.0).clamp(150.0, 480.0);
        ui.vertical(|ui| {
            ui.set_width(w);
            channel_list(ui, st, project, id, &channels, playhead, undo, out);
        });
        // the splitter between the list and the graph
        let (sr, sp) = ui.allocate_exact_size(vec2(6.0, ui.available_height()), Sense::drag());
        let sp = sp.on_hover_cursor(CursorIcon::ResizeHorizontal);
        if sp.dragged() {
            st.ch_w = Some((w + sp.drag_delta().x).clamp(150.0, 480.0));
        }
        let col = if sp.hovered() || sp.dragged() { pal.accent } else { look::LINE };
        ui.painter().vline(sr.center().x, sr.y_range(), Stroke::new(if col == look::LINE { 1.0 } else { 2.0 }, col));
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let c = project.clip(id).cloned().unwrap_or(c);
        graph(ui, st, project, &c, bi, (s0, s1), &shown, focus, rect, resp, playhead, pal, undo, out);
    });
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
    use crate::ui::list_rows::{Ctrl, RowSpec, Rows};
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
        for (gi, (g, props)) in CHANNELS.into_iter().enumerate() {
            let list: Vec<&'static str> = props.iter().copied().filter(|p| channels.contains(p)).collect();
            if list.is_empty() {
                continue;
            }
            let open = !st.folded.contains(&g);
            let any_keyed = list.iter().any(|p| prop_ref(&c, p).is_some_and(|a| a.is_animated()));
            let r = rows.row(
                ui,
                n,
                &RowSpec { key: u64::MAX - gi as u64, label: g, open: Some(open), dim: !any_keyed, values: &[""], ..Default::default() },
            );
            n += 1;
            if r.toggled {
                if open {
                    st.folded.push(g);
                } else {
                    st.folded.retain(|x| *x != g);
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
                let r = rows.row(
                    ui,
                    n,
                    &RowSpec {
                        key: (id as u64) << 8 | n as u64,
                        label: prop_label(p),
                        depth: 1,
                        dim: hidden || !a.is_animated(),
                        values: &[&v],
                        ..Default::default()
                    },
                );
                n += 1;
                // the property's colour, as on the graph; the focused one gets a bar
                let sw = Rect::from_min_size(pos2(r.label.left() - 7.0, r.label.center().y - 5.0), vec2(3.0, 10.0));
                ui.painter().rect_filled(sw, 1.0, prop_color(p));
                if st.focus == Some(p) {
                    ui.painter().rect_filled(r.label.expand2(vec2(4.0, 1.0)), 3.0, pal_accent(ui).gamma_multiply(0.12));
                }
                // a click on the name: focus it and select its keys (Shift adds)
                // a double-click on the name: a key at the playhead
                let lr = ui.interact(r.label, ui.id().with(("ch", id, p)), Sense::click());
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
        let _ = rows.finish(ui);
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

fn pal_accent(ui: &Ui) -> Color32 {
    ui.visuals().selection.stroke.color
}

#[allow(clippy::too_many_arguments)]
fn graph(
    ui: &mut Ui,
    st: &mut AnimState,
    project: &mut Project,
    c: &Clip,
    _bi: Option<usize>,
    (s0, s1): (f64, f64),
    props: &[&'static str],
    focus: Option<&'static str>,
    rect: Rect,
    resp: egui::Response,
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
    let plot = Rect::from_min_max(rect.min + vec2(58.0, 10.0), rect.max - vec2(12.0, 22.0));
    let pad = (s1 - s0).max(0.1) * 0.06;
    let (f0, f1) = st.win.map(|w| (w.0 - c.start, w.1 - c.start)).unwrap_or((s0 - pad, s1 + pad));
    let x = |t: f64| plot.left() + ((t - f0) / (f1 - f0)) as f32 * plot.width();
    let tx = |px: f32| f0 + ((px - plot.left()) / plot.width()) as f64 * (f1 - f0);
    let vel = |a: &Animated, t: f64| {
        let e = 1.0 / 600.0;
        (a.at(t + e) - a.at(t - e)) / (2.0 * e)
    };
    // zoom / pan the graph's own time (wheel, Shift+wheel, middle-drag), F / Home fits
    if let Some(w) = wheel_zoom(ui, &resp, plot, (c.start + f0, c.start + f1), false) {
        st.win = Some(w);
    }
    if ui.rect_contains_pointer(rect) && ui.input(|i| i.key_pressed(egui::Key::F) || i.key_pressed(egui::Key::Home)) {
        st.win = None;
    }
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
        painter.line_segment([pos2(px, plot.top()), pos2(px, plot.bottom())], Stroke::new(1.0, look::LINE));
        painter.text(pos2(px, rect.bottom() - 4.0), Align2::CENTER_BOTTOM, format!("{t}s"), FontId::proportional(10.0), look::DIM);
    }
    if let Some(fp) = focus.filter(|f| props.contains(f)) {
        let (lo, hi) = rng(fp);
        for i in 0..=4 {
            let v = lo + (hi - lo) * i as f64 / 4.0;
            let py = y(fp, v);
            painter.line_segment([pos2(plot.left(), py), pos2(plot.right(), py)], Stroke::new(1.0, look::LINE.gamma_multiply(0.7)));
            let txt = if st.velocity { speed_text(fp, shown(fp, v)) } else { format!("{:.0}{}", shown(fp, v), unit(fp)) };
            painter.text(pos2(plot.left() - 6.0, py), Align2::RIGHT_CENTER, txt, FontId::proportional(10.0), prop_color(fp));
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
            }
        }
    }
    let ph = *playhead - c.start;
    if (f0..=f1).contains(&ph) {
        painter.line_segment([pos2(x(ph), plot.top()), pos2(x(ph), plot.bottom())], Stroke::new(1.0, pal.playhead));
    }
    if let Some((a, b)) = st.boxsel {
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
                // own property's scale; Ctrl = time only
                let lead = prop_ref(&orig, hp).and_then(|a| a.keys.get(hk)).map_or(0.0, |k| k.t);
                let fps = project.fps.max(1.0);
                let mut to = ((lead + tx(cur.x) - tx(pp.x)) * fps).round() / fps;
                if ((ph - to) * plot.width() as f64 / (f1 - f0)).abs() < SNAP_PX as f64 {
                    to = ph;
                }
                let dt = to - lead;
                let ctrl = ui.input(|i| i.modifiers.ctrl);
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
            ui.separator();
        }
        if let Some(f) = focus {
            if menu::row(ui, None, &format!("Select every {fname} key"), "").clicked() {
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

#[allow(clippy::too_many_arguments)]
fn flow(
    ui: &mut Ui,
    st: &mut AnimState,
    project: &mut Project,
    ids: &[Id],
    playhead: &mut f64,
    pal: &Palette,
    undo: &mut dyn FnMut(&Project),
    out: &mut AnimResp,
) {
    let id = st.sel.map(|s| s.0).unwrap_or(ids[0]);
    let Some(c) = project.clip(id).cloned() else { return };
    let tr = project.track_of(id).unwrap_or(0);
    // this clip and the one it cuts to (Flow is about hand-offs, and the cut is the one people feel most)
    let next = project.tracks[tr].clips.iter().find(|n| (n.start - c.end()).abs() < 1.5 / project.fps).cloned();
    let (t0, t1) = (c.start, next.as_ref().map_or(c.end(), |n| n.start + n.duration.min(c.duration.max(2.0))));
    // the zoom the Blocks lanes are at, while it shows some of this
    let (t0, t1) = st.win.filter(|w| w.1 > t0 && w.0 < t1).unwrap_or((t0, t1));
    let seams: Vec<Seam> = project.block_seams().into_iter().filter(|s| s.left.0 == id || s.right.0 == id).collect();
    let prop = seams
        .iter()
        .max_by(|a, b| (a.v_left - a.v_right).abs().total_cmp(&(b.v_left - b.v_right).abs()))
        .map(|s| s.prop.clone())
        .or_else(|| c.blocks.iter().flat_map(|b| b.props.iter()).find(|p| MOTION_PROPS.contains(&p.as_str())).cloned())
        .unwrap_or_else(|| "scale".into());
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{} speed of {}{}",
                prop_label(&prop),
                c.name,
                next.as_ref().map_or(String::new(), |n| format!(" → {}", n.name))
            ))
            .strong(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let rough = seams.iter().filter(|s| !s.smooth).count();
            if ui.add_enabled(rough > 0, egui::Button::new("Smooth all")).clicked() {
                undo(project);
                let mut n = 0;
                for s in &seams {
                    n += project.match_seam(s.left, s.right, st.mode).is_ok() as usize;
                }
                out.edited = true;
                out.toast = Some(format!("Smoothed {n} hand-off{}", if n == 1 { "" } else { "s" }));
            }
        });
    });
    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click());
    let plot = Rect::from_min_max(rect.min + vec2(64.0, 30.0), rect.max - vec2(14.0, 66.0));
    if let Some(w) = wheel_zoom(ui, &resp, plot, (t0, t1), false) {
        st.win = Some(w);
    }
    let x = |t: f64| plot.left() + ((t - t0) / (t1 - t0).max(1e-3)) as f32 * plot.width();
    let samples: Vec<(f64, Option<f64>)> = (0..=300)
        .map(|i| {
            let t = t0 + (t1 - t0) * i as f64 / 300.0;
            let clip = if t < c.end() { Some(&c) } else { next.as_ref() };
            (t, clip.map(|cl| speed(cl, &prop, t - cl.start, 1.0)))
        })
        .collect();
    let mut m = samples.iter().filter_map(|s| s.1).fold(1.0f64, |m, v| m.max(v.abs()));
    for s in &seams {
        m = m.max(s.v_left.abs()).max(s.v_right.abs());
    }
    m *= 1.15;
    let y = |v: f64| plot.center().y - (v / m) as f32 * plot.height() / 2.0;
    let p = ui.painter_at(rect);
    for v in [-m / 1.15, 0.0, m / 1.15] {
        p.line_segment(
            [pos2(plot.left(), y(v)), pos2(plot.right(), y(v))],
            Stroke::new(1.0, pal.border.gamma_multiply(if v == 0.0 { 1.0 } else { 0.4 })),
        );
        p.text(
            pos2(plot.left() - 6.0, y(v)),
            Align2::RIGHT_CENTER,
            speed_text(&prop, v),
            FontId::proportional(10.0),
            pal.text_dim,
        );
    }
    for t in ticks(t0, t1, plot.width()) {
        p.text(
            pos2(x(t), rect.bottom() - 4.0),
            Align2::CENTER_BOTTOM,
            format!("{t}s"),
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
    for cl in std::iter::once(&c).chain(next.iter()) {
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
                pos2(x(a).max(plot.left()), plot.bottom() + 8.0 + row),
                pos2(x(e).min(plot.right()), plot.bottom() + 20.0 + row),
            );
            if r.width() > 1.0 {
                p.rect_filled(r, 3.0, block_col(b).gamma_multiply(0.45));
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
        let c2 = if s.smooth { GOOD } else { BAD };
        p.line_segment([pos2(sx, y(s.v_left)), pos2(sx, y(s.v_right))], Stroke::new(3.0, c2));
        p.circle(pos2(sx, y(s.v_left)), 4.5, pal.panel, Stroke::new(2.0, c2));
        p.circle_filled(pos2(sx, y(s.v_right)), 4.5, c2);
        let label = if s.smooth {
            "✓ Smooth".to_string()
        } else {
            format!("⚡ {} jump · Smooth", speed_text(&s.prop, (s.v_left - s.v_right).abs()))
        };
        let br = Rect::from_center_size(pos2(sx, plot.top() - 16.0), vec2(label.len() as f32 * 6.2 + 14.0, 20.0));
        let r = ui.put(br, egui::Button::new(egui::RichText::new(label).size(11.0).color(c2)).corner_radius(10.0));
        if r.clicked() {
            act = Some((s.left, s.right, s.t));
        }
    }
    if seams.is_empty() {
        p.text(
            plot.left_top(),
            Align2::LEFT_TOP,
            "No hand-offs here. Put two blocks end to end, or end one on a cut, to match their speeds.",
            FontId::proportional(12.0),
            pal.text_dim,
        );
    }
    if (t0..=t1).contains(playhead) {
        p.line_segment(
            [pos2(x(*playhead), plot.top()), pos2(x(*playhead), plot.bottom())],
            Stroke::new(1.0, pal.playhead),
        );
    }
    if let Some(pp) = resp.interact_pointer_pos().filter(|_| resp.clicked()) {
        *playhead = t0 + ((pp.x - plot.left()) / plot.width()) as f64 * (t1 - t0);
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

#[allow(clippy::too_many_arguments)]
fn speed_view(
    ui: &mut Ui,
    st: &mut AnimState,
    project: &mut Project,
    ids: &[Id],
    playhead: &mut f64,
    pal: &Palette,
    undo: &mut dyn FnMut(&Project),
    out: &mut AnimResp,
) {
    let id = st.sel.map(|s| s.0).unwrap_or(ids[0]);
    let Some(c) = project.clip(id).cloned() else { return };
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(format!("{} · speed ramp", c.name)).strong());
        for pr in crate::engine::presets::builtin_speed_ramps() {
            if chip(ui, false, &pr.name).clicked() {
                undo(project);
                if let Some(cm) = project.clip_mut(id) {
                    cm.speed_curve.keys = pr.keys.iter().map(|k| Keyframe { t: k.t * cm.duration, ..*k }).collect();
                }
                out.edited = true;
            }
        }
        if chip(ui, false, "Normal").clicked() {
            undo(project);
            if let Some(cm) = project.clip_mut(id) {
                cm.speed_curve.keys.clear();
            }
            out.edited = true;
        }
        let tr = project.track_of(id).unwrap_or(0);
        let next =
            project.tracks[tr].clips.iter().find(|n| (n.start - c.end()).abs() < 1.5 / project.fps).map(|n| n.id);
        if let Some(n) = next {
            if ui
                .button("Ramp through the cut")
                .on_hover_text(
                    "Speed up into the cut and out of it on the next clip: the classic speed-ramp transition",
                )
                .clicked()
            {
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
        }
        ui.label(egui::RichText::new("Drag points · double-click adds · Del removes").size(11.0).weak());
    });
    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let plot = Rect::from_min_max(rect.min + vec2(50.0, 12.0), rect.max - vec2(14.0, 22.0));
    let (l0, l1) = (-2.4f64, 2.1f64);
    let x = |t: f64| plot.left() + (t / c.duration.max(1e-3)) as f32 * plot.width();
    let tx = |px: f32| ((px - plot.left()) / plot.width()) as f64 * c.duration;
    let y = |s: f64| plot.bottom() - ((s.max(0.01).log2() - l0) / (l1 - l0)) as f32 * plot.height();
    let sy = |py: f32| 2f64.powf(l0 + ((plot.bottom() - py) / plot.height()) as f64 * (l1 - l0));
    let p = ui.painter_at(rect);
    for s in [0.25, 0.5, 1.0, 2.0, 4.0] {
        p.line_segment(
            [pos2(plot.left(), y(s)), pos2(plot.right(), y(s))],
            Stroke::new(1.0, pal.border.gamma_multiply(if s == 1.0 { 1.0 } else { 0.4 })),
        );
        p.text(
            pos2(plot.left() - 6.0, y(s)),
            Align2::RIGHT_CENTER,
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
    let mut fill = pts.clone();
    fill.push(pos2(plot.right(), y(1.0)));
    fill.push(pos2(plot.left(), y(1.0)));
    p.add(egui::Shape::line(pts, Stroke::new(2.2, pal.accent)));
    let mut hits = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        let kp = pos2(x(k.t), y(k.v));
        p.circle(
            kp,
            6.0,
            if st.speed_sel == Some(i) { Color32::WHITE } else { pal.accent },
            Stroke::new(1.5, Color32::WHITE),
        );
        p.text(
            kp - vec2(0.0, 10.0),
            Align2::CENTER_BOTTOM,
            format!("{:.0}%", k.v * 100.0),
            FontId::proportional(10.0),
            pal.text,
        );
        hits.push((Rect::from_center_size(kp, vec2(14.0, 14.0)), i));
    }
    let ph = *playhead - c.start;
    if (0.0..=c.duration).contains(&ph) {
        p.line_segment([pos2(x(ph), plot.top()), pos2(x(ph), plot.bottom())], Stroke::new(1.0, pal.playhead));
    }
    let press = ui.input(|i| i.pointer.press_origin());
    let hit = press.and_then(|pp| hits.iter().find(|h| h.0.contains(pp)).map(|h| h.1));
    if resp.drag_started() && hit.is_some() {
        undo(project);
        st.speed_sel = hit;
    }
    if let (Some(i), Some(cur)) = (st.speed_sel.filter(|_| resp.dragged()), resp.interact_pointer_pos()) {
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
        st.speed_sel = hit;
        if hit.is_none() {
            if let Some(pp) = press {
                *playhead = c.start + tx(pp.x).clamp(0.0, c.duration);
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
    if let Some(i) = st.speed_sel.filter(|_| (st.here || ui.ui_contains_pointer()) && del_pressed(ui)) {
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
            kind_pill(ui, kind, col);
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

/// The ease the block's main move has now: its control, else what the preset was designed with.
fn current_ease(ctl: &BlockCtl) -> Ease {
    ctl.ease.or_else(|| ctl.src.tracks.first().and_then(|t| t.keys.first()).map(|k| k.ease)).unwrap_or(Ease::Linear)
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
    let mut dur = b.dur;
    let mut changed = false;
    let mut gesture = false;
    // origin / target: where each property is as the block starts / ends
    let ends: Vec<(String, f64, f64)> = project
        .clip(id)
        .map(|c| b.props.iter().take(3).filter_map(|p| prop_ref(c, p).map(|a| (p.clone(), a.at(b.t), a.at(b.end())))).collect())
        .unwrap_or_default();
    egui::Grid::new(("anim-ctl", id, i)).num_columns(2).spacing([12.0, 9.0]).min_col_width(70.0).show(ui, |ui| {
        let wig = wiggles(&ctl);
        let moves_xy = ctl.src.gen.map_or(ctl.dir.is_some() && !wig, |g| g.has_dir());
        if moves_xy {
            field(ui, if b.name.contains("Out") || ctl.src.gen == Some(Gen::ThrowOut) { "Toward side" } else { "From side" });
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
        if let Some(g) = ctl.src.gen {
            let ph = &mut ctl.phys;
            for &k in g.controls() {
                let r = match k {
                    "stiffness" => slider(ui, "Stiffness", &mut ph.stiffness, 0.0, 1.0, ""),
                    "damping" => {
                        slider(ui, if g == Gen::Shake { "Fade out" } else { "Damping" }, &mut ph.damping, 0.0, 1.0, "")
                    }
                    "bounce" => slider(ui, "Bounciness", &mut ph.bounce, 0.0, 1.0, ""),
                    "gravity" => slider(ui, "Gravity", &mut ph.gravity, 0.0, 1.0, ""),
                    "spin" => slider(ui, if g == Gen::Shake { "Tilt" } else { "Spin" }, &mut ph.spin, -1.0, 1.0, ""),
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
            if g == Gen::Shake {
                field(ui, "");
                if chip(ui, false, "New shake pattern").clicked() {
                    ctl.phys.seed = ctl.phys.seed.wrapping_add(1);
                    changed = true;
                    gesture = true;
                }
                ui.end_row();
            }
        } else {
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
    use super::*;

    struct H {
        ctx: egui::Context,
        st: AnimState,
        p: Project,
        sel: Vec<Id>,
        ph: f64,
        width: f32,
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
            H { ctx: egui::Context::default(), st: AnimState::default(), p, sel: vec![1, 2], ph: 0.5, width }
        }
        fn frame(&mut self) -> AnimResp {
            let mut out = AnimResp::default();
            let w = self.width;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w, 420.0))),
                ..Default::default()
            };
            let (st, p, sel, ph) = (&mut self.st, &mut self.p, &self.sel, &mut self.ph);
            let _ = self.ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let pal = Palette::new(true, Color32::from_rgb(0x33, 0x66, 0xFF));
                    out = show(ui, st, p, sel, ph, false, &pal, &[], &mut |_| {});
                });
            });
            out
        }
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
        p.place_key_block(1, &f("Pop In"), 10.0, Some(0.4));
        p.place_key_block(1, &f("Slide Out Right"), 12.0, Some(0.6));
        let (a, b) = block_fit(&p, &[1], fit);
        assert!(a <= 10.0 && a >= fit.0 && b <= fit.1, "{a} {b}");
        // the shortest block (0.4 s) is at least a fourteenth of the window
        assert!(b - a <= 0.4 * 14.0 + 1e-9 && b - a >= 0.4, "{a} {b}");
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
