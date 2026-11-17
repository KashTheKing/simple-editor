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
    /// Onion-skin ghosts each side (`onion`); `None` = the default, 3.
    pub ghosts: Option<u8>,
    /// Width of the lanes' label column (the splitter between labels and lanes); `None` = `HEAD_W`.
    pub head_w: Option<f32>,
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
const SUB_H: f32 = 18.0;
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
fn segmented<T: PartialEq + Copy>(ui: &mut Ui, cur: &mut T, items: &[(T, &str)]) -> bool {
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
    }
    SEL.with(|s| s.set(st.sel));
    set_onion(st.ghosts.unwrap_or(3));
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
    if compact {
        compact_list(ui, st, project, &ids, playhead, pal, undo, &mut out);
        return out;
    }
    match st.view {
        View::Blocks => blocks(ui, st, project, &ids, playhead, playing, pal, undo, &mut out),
        View::Curves => curves(ui, st, project, &ids, playhead, pal, undo, &mut out),
        View::Flow => flow(ui, st, project, &ids, playhead, pal, undo, &mut out),
        View::Speed => speed_view(ui, st, project, &ids, playhead, pal, undo, &mut out),
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
                if let Some((t, d)) = project.apply_key_block(id, &b, at, None) {
                    let c = project.clip(id).unwrap();
                    st.sel = Some((id, c.blocks.iter().position(|x| (x.t - t).abs() < 1e-9).unwrap_or(0)));
                    out.edited = true;
                    out.play = Some((c.start + t, c.start + t + d + 0.25));
                    out.toast = Some(format!("Added {name} to {}", c.name));
                }
            }
        }
        let on = st.ghosts.unwrap_or(3) > 0;
        if chip(ui, on, "Onion skin").on_hover_text("Ghost outlines of the selected layer a few frames before (blue) and after (orange), and where the selected block starts (dashed)").clicked() {
            st.ghosts = Some(if on { 0 } else { 3 });
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

/// A row per block so that blocks overlapping in time never share one (in time order, first free row).
fn pack_rows(blocks: &[AppliedBlock]) -> Vec<usize> {
    let mut ends: Vec<f64> = Vec::new();
    blocks
        .iter()
        .map(|b| match ends.iter().position(|&e| e <= b.t + 1e-4) {
            Some(r) => {
                ends[r] = b.end();
                r
            }
            None => {
                ends.push(b.end());
                ends.len() - 1
            }
        })
        .collect()
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

fn ruler(ui: &mut Ui, rect: Rect, t0: f64, t1: f64, playhead: &mut f64, pal: &Palette, out: &mut AnimResp) {
    let p = ui.painter_at(rect);
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

#[allow(clippy::too_many_arguments)]
fn blocks(
    ui: &mut Ui,
    st: &mut AnimState,
    project: &mut Project,
    ids: &[Id],
    playhead: &mut f64,
    playing: bool,
    pal: &Palette,
    undo: &mut dyn FnMut(&Project),
    out: &mut AnimResp,
) {
    let (t0, t1) = range(project, ids);
    let seams = project.block_seams();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let full = ui.available_rect_before_wrap();
        let head_w = st.head_w.unwrap_or(HEAD_W).clamp(80.0, (full.width() * 0.5).max(80.0));
        let body_x0 = full.left() + head_w;
        let bw = (full.right() - body_x0).max(50.0);
        let x = |t: f64| body_x0 + ((t - t0) / (t1 - t0)) as f32 * bw;
        let tx = |px: f32| t0 + ((px - body_x0) / bw) as f64 * (t1 - t0);
        let pps = bw as f64 / (t1 - t0);
        let (rr, _) = ui.allocate_exact_size(vec2(full.width(), 20.0), Sense::hover());
        ruler(ui, Rect::from_min_max(pos2(body_x0, rr.top()), rr.max), t0, t1, playhead, pal, out);
        let top = rr.bottom();
        let mut act: Option<Box<dyn FnOnce(&mut Project, &mut AnimState, &mut AnimResp)>> = None;
        for &id in ids {
            let Some(c) = project.clip(id).cloned() else { continue };
            // blocks that overlap in time (a speed ramp during a zoom) stack in rows inside the lane
            let pack = pack_rows(&c.blocks);
            let nrows = pack.iter().copied().max().map_or(1, |m| m + 1);
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
            ui.interact(head, ui.id().with(("lane-head", id)), Sense::hover()).on_hover_text(&c.name);
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
                let sel = st.sel == Some((id, i));
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
                let resp = ui.interact(r, ui.id().with(("ablk", id, i)), Sense::click_and_drag());
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
                    "{} · {:.2} s · drag to move, drag an end to stretch, click ▸ to see its keys",
                    b.name, b.dur
                ));
                if resp.clicked() {
                    st.sel = Some((id, i));
                    if hp.is_some_and(|p| p.x < r.left() + 16.0) {
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
                        let mut cands = vec![*playhead - orig.start, 0.0, orig.duration];
                        cands.extend(
                            orig.blocks.iter().enumerate().filter(|(j, _)| *j != i).flat_map(|(_, o)| [o.t, o.end()]),
                        );
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
                            p.remove_key_block(id, i);
                            s.sel = None;
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
            // open blocks: their keys, one row per property, diamonds drag in time
            for (i, b) in c.blocks.iter().enumerate().filter(|(i, _)| st.open.contains(&(id, *i))) {
                for prop in &b.props {
                    let (sub, _) = ui.allocate_exact_size(vec2(full.width(), SUB_H), Sense::hover());
                    let p = ui.painter_at(sub);
                    p.rect_filled(sub, 0.0, look::DEEP);
                    p.text(
                        pos2(sub.left() + 22.0, sub.center().y),
                        Align2::LEFT_CENTER,
                        prop_label(prop),
                        FontId::proportional(10.5),
                        prop_color(prop),
                    );
                    let Some(a) = prop_ref(&c, prop) else { continue };
                    for (k, key) in
                        a.keys.iter().enumerate().filter(|(_, k)| k.t >= b.t - 1e-4 && k.t <= b.end() + 1e-4)
                    {
                        let kx = x(c.start + key.t);
                        let kr = Rect::from_center_size(pos2(kx, sub.center().y), vec2(10.0, 10.0));
                        p.add(egui::Shape::convex_polygon(
                            vec![
                                pos2(kx, kr.top()),
                                pos2(kr.right(), kr.center().y),
                                pos2(kx, kr.bottom()),
                                pos2(kr.left(), kr.center().y),
                            ],
                            prop_color(prop),
                            Stroke::new(1.0, Color32::BLACK),
                        ));
                        let r = ui
                            .interact(kr, ui.id().with(("akey", id, i, prop.as_str(), k)), Sense::drag())
                            .on_hover_cursor(CursorIcon::ResizeHorizontal);
                        if r.drag_started() {
                            undo(project);
                        }
                        if let Some(pos) = r.interact_pointer_pos().filter(|_| r.dragged()) {
                            let mut t = tx(pos.x) - c.start;
                            t = (t * project.fps).round() / project.fps;
                            let (lo, hi) = (
                                a.keys.get(k.wrapping_sub(1)).map_or(f64::MIN, |o| o.t + 1e-3),
                                a.keys.get(k + 1).map_or(f64::MAX, |o| o.t - 1e-3),
                            );
                            let prop = prop.clone();
                            if let Some(cm) = project.clip_mut(id) {
                                if let Some(a) = prop_mut(cm, &prop) {
                                    a.keys[k].t = t.clamp(lo, hi);
                                }
                                if let Some(ctl) = cm.blocks[i].ctl.as_mut() {
                                    ctl.edited = true;
                                }
                            }
                            out.edited = true;
                        }
                    }
                }
            }
        }
        let bottom = ui.min_rect().bottom();
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
        if !playing && ui.ctx().input(|i| i.key_pressed(egui::Key::Delete)) && ui.ui_contains_pointer() {
            if let Some((id, i)) = st.sel {
                act = Some(Box::new(move |p, s, _| {
                    p.remove_key_block(id, i);
                    s.sel = None;
                }));
            }
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
    let bi = st.sel.filter(|s| s.0 == id).map(|s| s.1);
    let (s0, s1) = curve_span(&c, bi);
    let props: Vec<&'static str> = PROPS
        .into_iter()
        .filter(|p| prop_ref(&c, p).is_some_and(|a| a.keys.iter().any(|k| k.t >= s0 - 1e-4 && k.t <= s1 + 1e-4)))
        .filter(|p| bi.is_none_or(|i| c.blocks[i].props.iter().any(|q| q == p)))
        .collect();
    let focus = st.focus.filter(|f| props.contains(f)).or(props.first().copied());
    st.focus = focus;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(match bi {
                Some(i) => format!("{} · {}", c.blocks[i].name, c.name),
                None => format!("{} · every key", c.name),
            })
            .strong(),
        );
        for p in &props {
            if chip(ui, focus == Some(*p), prop_label(p)).clicked() {
                st.focus = Some(p);
            }
        }
        ui.separator();
        if chip(ui, !st.velocity, "Value").clicked() {
            st.velocity = false;
        }
        if chip(ui, st.velocity, "Velocity").clicked() {
            st.velocity = true;
        }
        ui.separator();
        ui.label(
            egui::RichText::new(if st.keys.is_empty() { "Ease all:" } else { "Ease selected:" }).size(11.0).weak(),
        );
        for (n, e) in Ease::PRESETS.into_iter().take(5).chain([("Linear", Ease::Linear)]) {
            if chip(ui, false, n).clicked() {
                undo(project);
                if let Some(cm) = project.clip_mut(id) {
                    for p in &props {
                        let Some(a) = prop_mut(cm, p) else { continue };
                        for (k, key) in a.keys.iter_mut().enumerate() {
                            let inr = key.t >= s0 - 1e-4 && key.t < s1 - 1e-4;
                            if inr && (st.keys.is_empty() || st.keys.contains(&(*p, k))) {
                                key.ease = e;
                            }
                        }
                    }
                    if let Some(ctl) = bi.and_then(|i| cm.blocks[i].ctl.as_mut()) {
                        ctl.edited = true;
                    }
                }
                out.edited = true;
            }
        }
    });
    if props.is_empty() {
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new("No keys here yet: add a block, or key a property in the Inspector.")
                .color(pal.text_dim),
        );
        return;
    }
    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let plot = Rect::from_min_max(rect.min + vec2(56.0, 10.0), rect.max - vec2(12.0, 22.0));
    let pad = (s1 - s0).max(0.1) * 0.06;
    let (f0, f1) = (s0 - pad, s1 + pad);
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
                let a = prop_ref(&c, p).unwrap();
                let vs: Vec<f64> = (0..=80)
                    .map(|i| {
                        let t = s0 + (s1 - s0) * i as f64 / 80.0;
                        if st.velocity {
                            vel(a, t)
                        } else {
                            a.at(t)
                        }
                    })
                    .collect();
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
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        Rect::from_min_max(pos2(x(s0), plot.top()), pos2(x(s1), plot.bottom())),
        0.0,
        pal.text.gamma_multiply(0.03),
    );
    for t in ticks(c.start + f0, c.start + f1, plot.width()) {
        let px = x(t - c.start);
        painter.line_segment(
            [pos2(px, plot.top()), pos2(px, plot.bottom())],
            Stroke::new(1.0, pal.border.gamma_multiply(0.5)),
        );
        painter.text(
            pos2(px, rect.bottom() - 4.0),
            Align2::CENTER_BOTTOM,
            format!("{t}s"),
            FontId::proportional(10.0),
            pal.text_dim,
        );
    }
    if let Some(fp) = focus {
        let (lo, hi) = rng(fp);
        for i in 0..=4 {
            let v = lo + (hi - lo) * i as f64 / 4.0;
            let py = y(fp, v);
            painter.line_segment(
                [pos2(plot.left(), py), pos2(plot.right(), py)],
                Stroke::new(1.0, pal.border.gamma_multiply(0.35)),
            );
            let txt =
                if st.velocity { speed_text(fp, shown(fp, v)) } else { format!("{:.0}{}", shown(fp, v), unit(fp)) };
            painter.text(
                pos2(plot.left() - 6.0, py),
                Align2::RIGHT_CENTER,
                txt,
                FontId::proportional(10.0),
                prop_color(fp),
            );
        }
    }
    // curves (focus last, on top)
    let mut order: Vec<&'static str> = props.iter().copied().filter(|p| Some(*p) != focus).collect();
    order.extend(focus);
    for &p in &order {
        let a = prop_ref(&c, p).unwrap();
        let pts: Vec<Pos2> = (0..=240)
            .map(|i| {
                let t = f0 + (f1 - f0) * i as f64 / 240.0;
                pos2(x(t), y(p, if st.velocity { vel(a, t) } else { a.at(t) }))
            })
            .collect();
        let foc = Some(p) == focus;
        painter.add(egui::Shape::line(
            pts,
            Stroke::new(if foc { 2.2 } else { 1.4 }, prop_color(p).gamma_multiply(if foc { 1.0 } else { 0.45 })),
        ));
    }
    // keys + the selected keys' handles (value view only: the handles are the segment's bezier)
    let mut hits: Vec<(Rect, &'static str, usize, i8)> = Vec::new();
    if !st.velocity {
        for &p in &order {
            let a = prop_ref(&c, p).unwrap();
            for (k, key) in a.keys.iter().enumerate().filter(|(_, k)| k.t >= s0 - 1e-4 && k.t <= s1 + 1e-4) {
                let kp = pos2(x(key.t), y(p, key.v));
                let on = st.keys.contains(&(p, k));
                if on {
                    // out handle = this segment's (x1, y1), in handle = the previous segment's (x2, y2)
                    if let Some(nx) = a.keys.get(k + 1) {
                        let (x1, y1, _, _) = key.ease.handles();
                        let h = pos2(x(key.t + x1 as f64 * (nx.t - key.t)), y(p, key.v + y1 as f64 * (nx.v - key.v)));
                        painter.line_segment([kp, h], Stroke::new(1.0, pal.text_dim));
                        painter.circle(h, 4.5, pal.panel, Stroke::new(1.5, Color32::WHITE));
                        hits.push((Rect::from_center_size(h, vec2(12.0, 12.0)), p, k, 1));
                    }
                    if let Some(pv) = k.checked_sub(1).and_then(|j| a.keys.get(j)) {
                        let (_, _, x2, y2) = pv.ease.handles();
                        let h = pos2(x(pv.t + x2 as f64 * (key.t - pv.t)), y(p, pv.v + y2 as f64 * (key.v - pv.v)));
                        painter.line_segment([kp, h], Stroke::new(1.0, pal.text_dim));
                        painter.circle(h, 4.5, pal.panel, Stroke::new(1.5, Color32::WHITE));
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
        painter.rect(
            Rect::from_two_pos(a, b),
            0.0,
            pal.accent.gamma_multiply(0.15),
            Stroke::new(1.0, pal.accent),
            egui::StrokeKind::Inside,
        );
    }
    // interaction: what the press landed on decides the drag
    let shift = ui.input(|i| i.modifiers.shift);
    let alt = ui.input(|i| i.modifiers.alt);
    let press = ui.input(|i| i.pointer.press_origin());
    let hit = press.and_then(|pp| hits.iter().rev().find(|h| h.0.contains(pp)).copied());
    if resp.clicked() || resp.drag_started() {
        match hit {
            Some((_, p, k, 0)) => {
                if shift {
                    if let Some(j) = st.keys.iter().position(|s| *s == (p, k)) {
                        st.keys.remove(j);
                    } else {
                        st.keys.push((p, k));
                    }
                } else if !st.keys.contains(&(p, k)) {
                    st.keys = vec![(p, k)];
                }
                st.focus = Some(p);
            }
            Some(_) => {}
            None if resp.clicked() => {
                if !shift {
                    st.keys.clear();
                }
                if let Some(pp) = press {
                    *playhead = (c.start + tx(pp.x)).max(0.0);
                    out.seeked = true;
                }
            }
            None => {
                if !shift {
                    st.keys.clear();
                }
                st.boxsel = press.map(|p| (p, p));
            }
        }
    }
    if resp.drag_started() && hit.is_some() && !st.velocity {
        undo(project);
        st.orig = Some(c.clone());
        st.ranges = Some(ranges.clone());
    }
    if resp.dragged() {
        let cur = ui.input(|i| i.pointer.interact_pos());
        if let (Some(b), Some(cur)) = (st.boxsel.as_mut(), cur) {
            b.1 = cur;
        } else if let (Some(orig), Some(cur), Some(pp), Some((_, hp, hk, side))) = (st.orig.clone(), cur, press, hit) {
            let mut cm = orig.clone();
            if side == 0 {
                // move the selected keys in time (snapped to frames and the playhead) and value
                let mut dt = tx(cur.x) - tx(pp.x);
                let lead = prop_ref(&orig, hp).unwrap().keys[hk].t;
                let fps = project.fps.max(1.0);
                let mut to = ((lead + dt) * fps).round() / fps;
                if ((ph - (lead + dt)) * plot.width() as f64 / (f1 - f0)).abs() < SNAP_PX as f64 {
                    to = ph;
                }
                dt = to - lead;
                for &(p, k) in &st.keys {
                    let (Some(oa), Some(a)) = (prop_ref(&orig, p), prop_mut(&mut cm, p)) else { continue };
                    let ok = oa.keys[k];
                    let lo = k.checked_sub(1).map_or(f64::MIN, |j| oa.keys[j].t + 1e-3);
                    let hi = oa.keys.get(k + 1).map_or(f64::MAX, |o| o.t - 1e-3);
                    a.keys[k].t = (ok.t + dt).clamp(lo, hi);
                    if !ui.input(|i| i.modifiers.ctrl) {
                        a.keys[k].v = ok.v + vy(p, cur.y) - vy(p, pp.y);
                    }
                }
            } else if let (Some(oa), Some(a)) = (prop_ref(&orig, hp), prop_mut(&mut cm, hp)) {
                // a handle: rewrite the segment's bezier (Alt = this handle alone; else the other side of the key
                // keeps the line through the key smooth)
                let (t, v) = (tx(cur.x), vy(hp, cur.y));
                let seg = if side > 0 { hk } else { hk - 1 };
                let (k0, k1) = (oa.keys[seg], oa.keys[seg + 1]);
                let (dt, dv) = (k1.t - k0.t, k1.v - k0.v);
                let fx = ((t - k0.t) / dt).clamp(0.0, 1.0) as f32;
                let fy = if dv.abs() > 1e-9 { ((v - k0.v) / dv) as f32 } else { 0.0 };
                let (x1, y1, x2, y2) = k0.ease.handles();
                a.keys[seg].ease = if side > 0 {
                    Ease::Bezier { x1: fx, y1: fy, x2, y2 }
                } else {
                    Ease::Bezier { x1, y1, x2: fx, y2: fy }
                };
                if !alt {
                    // keep the key smooth: the neighbouring segment's handle points the other way at the same slope
                    let slope = if side > 0 {
                        (fy as f64 * dv) / (fx as f64 * dt).max(1e-6)
                    } else {
                        ((1.0 - fy as f64) * dv) / ((1.0 - fx as f64) * dt).max(1e-6)
                    };
                    let other = if side > 0 { hk.checked_sub(1) } else { (hk + 1 < oa.keys.len()).then_some(hk) };
                    if let Some(o) = other.filter(|&o| o != seg) {
                        let (ka, kb) = (oa.keys[o], oa.keys[o + 1]);
                        let s = slope * (kb.t - ka.t) / (kb.v - ka.v).abs().max(1e-9).copysign(kb.v - ka.v);
                        a.keys[o].ease =
                            if side > 0 { ease_with_out_slope(ka.ease, s) } else { ease_with_in_slope(ka.ease, s) };
                    }
                }
            }
            if let Some(i) = bi {
                if let Some(ctl) = cm.blocks[i].ctl.as_mut() {
                    ctl.edited = true;
                }
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
    }
    // double-click on a curve adds a key on the focused property there
    if resp.double_clicked() && hit.is_none() && !st.velocity {
        if let (Some(fp), Some(pp)) = (focus, press) {
            undo(project);
            let t = tx(pp.x).clamp(0.0, c.duration);
            if let Some(a) = project.clip_mut(id).and_then(|cm| prop_mut(cm, fp)) {
                let v = a.at(t);
                a.set_at(t, v);
            }
            out.edited = true;
        }
    }
    if ui.ui_contains_pointer() && ui.input(|i| i.key_pressed(egui::Key::Delete)) && !st.keys.is_empty() {
        undo(project);
        if let Some(cm) = project.clip_mut(id) {
            let mut sel = st.keys.clone();
            sel.sort_by(|a, b| b.1.cmp(&a.1));
            for (p, k) in sel {
                if let Some(a) = prop_mut(cm, p).filter(|a| a.keys.len() > 2 && k < a.keys.len()) {
                    a.keys.remove(k);
                }
            }
            if let Some(ctl) = bi.and_then(|i| cm.blocks[i].ctl.as_mut()) {
                ctl.edited = true;
            }
        }
        st.keys.clear();
        out.edited = true;
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
    let plot = Rect::from_min_max(rect.min + vec2(64.0, 30.0), rect.max - vec2(14.0, 40.0));
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
    if next.is_some() {
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
            p.add(egui::Shape::line(pts, Stroke::new(2.2, col)));
        }
    }
    // the blocks underneath, for orientation
    let mut row = 0.0;
    for cl in std::iter::once(&c).chain(next.iter()) {
        for b in &cl.blocks {
            let r = Rect::from_min_max(
                pos2(x(cl.start + b.t).max(plot.left()), plot.bottom() + 8.0 + row),
                pos2(x(cl.start + b.end()).min(plot.right()), plot.bottom() + 20.0 + row),
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
            row = if row == 0.0 { 13.0 } else { 0.0 };
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
                cm.speed_curve.set_at(t, sy(pp.y).clamp(0.05, 8.0));
                if let Some(k) = cm.speed_curve.key_index_at(t) {
                    cm.speed_curve.keys[k].ease = Ease::EaseInOut;
                }
            }
            out.edited = true;
        }
    }
    if let Some(i) = st.speed_sel.filter(|_| ui.ui_contains_pointer() && ui.input(|i| i.key_pressed(egui::Key::Delete)))
    {
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
    egui::Grid::new(("anim-ctl", id, i)).num_columns(2).spacing([12.0, 9.0]).min_col_width(70.0).show(ui, |ui| {
        let moves_xy = ctl.src.gen.map_or(ctl.dir.is_some(), |g| g.has_dir());
        if moves_xy {
            field(ui, if b.name.contains("Out") || ctl.src.gen == Some(Gen::ThrowOut) { "Toward" } else { "From" });
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
                        ctl.ease = e;
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
        assert_eq!(pack_rows(b), vec![0, 1, 0], "{b:?}");
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
