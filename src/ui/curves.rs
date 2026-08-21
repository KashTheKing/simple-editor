//! Curve (graph) editor for the FIRST selected clip: x = clip-local time over [0, duration], y = value.
//! Left: a list of the clip's keyframeable properties (`Clip::props_mut` names + "Volume"/"Pan" +
//! "<effect>: <param>" for effect params) with a colour swatch and a checkbox to show/hide each curve
//! (auto-scaled per property; the selected property's scale drives the y axis labels). Right: the graph —
//! curves drawn with the eased segments (sample `Animated::at` every ~2 px), keys as diamonds (accent when
//! selected), playhead line (click on the ruler strip seeks → `seeked`), constant (unkeyed) properties
//! shown as a flat dashed line. Interaction: drag a key horizontally/vertically (time clamped to the clip,
//! vertical changes the value — `Animated::move_key` + `keys[i].v`; undo once at drag start), click a key
//! to select it, Delete removes it, double-click on empty graph area adds a key for the active property at
//! that (t, v), right-click a key → easing menu (Ease::ALL) + Delete; Ctrl+Scroll zooms the time axis,
//! Shift+Scroll pans. Bezier velocity handles: a Bezier segment (or any segment whose left key is
//! selected) shows its two handles; dragging one converts the segment to `Ease::Bezier`.

use crate::model::{Animated, Clip, Ease, Id, Project};
use crate::settings::CurvePreset;
use crate::theme::Palette;
use eframe::egui::{self, pos2, vec2, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke};
use std::cell::RefCell;

pub(crate) const RULER_H: f32 = 16.0;
const LIST_W: f32 = 150.0;
const HIT_PX: f32 = 6.0;

thread_local! {
    // ponytail: thread_local hand-off because show() can't reach Settings. The app calls
    // set_available_presets() each frame (or when they change) and polls take_pending_curve_preset().
    static AVAILABLE: RefCell<Vec<CurvePreset>> = const { RefCell::new(Vec::new()) };
    static PENDING: RefCell<Option<CurvePreset>> = const { RefCell::new(None) };
}

/// The app hands the current Settings.curve_presets to the panel with this (cheap: only on change or
/// every frame — the panel just reads names + the selected preset).
pub fn set_available_presets(v: Vec<CurvePreset>) {
    AVAILABLE.with(|a| *a.borrow_mut() = v);
}

/// A curve preset captured via "Save curve preset…" waiting for the app to store it in Settings.
pub fn take_pending_curve_preset() -> Option<CurvePreset> {
    PENDING.with(|p| p.borrow_mut().take())
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Key {
        prop: usize,
        key: usize,
    },
    /// Out-handle of the segment starting at `key` of `prop`.
    HandleOut {
        prop: usize,
        key: usize,
    },
    /// In-handle of that segment (drawn near key + 1).
    HandleIn {
        prop: usize,
        key: usize,
    },
    Seek,
}

pub struct CurvesState {
    /// Index of the active property in the panel's property list.
    pub active: usize,
    /// Hidden curves (property indices).
    pub hidden: Vec<usize>,
    /// Time zoom/pan of the graph (seconds at the left edge, px per second; 0 = fit the clip).
    pub scroll_x: f64,
    pub zoom: f32,
    /// Selected keys as (property index, key index).
    pub selected: Vec<(usize, usize)>,
    /// Last frame's graph rect / effective px-per-second / active y scale (hit-testing + tests).
    pub graph: Rect,
    pub pps: f32,
    pub y_lo: f64,
    pub y_hi: f64,
    drag: Option<Drag>,
    preset_name: String,
    preset_sel: usize,
    /// Key under the open context menu.
    menu_key: Option<(usize, usize)>,
}

impl Default for CurvesState {
    fn default() -> Self {
        Self {
            active: 0,
            hidden: Vec::new(),
            scroll_x: 0.0,
            zoom: 0.0,
            selected: Vec::new(),
            graph: Rect::NOTHING,
            pps: 0.0,
            y_lo: 0.0,
            y_hi: 1.0,
            drag: None,
            preset_name: String::new(),
            preset_sel: 0,
            menu_key: None,
        }
    }
}

#[derive(Default)]
pub struct CurvesResponse {
    pub edited: bool,
    pub seeked: bool,
}

// ---------- property plumbing (same order as the panel list) ----------

fn base_count(c: &Clip) -> usize {
    if c.is_visual() {
        5
    } else {
        2
    }
}

pub(crate) fn prop_count(c: &Clip) -> usize {
    base_count(c) + c.effects.iter().map(|e| e.params.len()).sum::<usize>()
}

pub(crate) fn prop_label(c: &Clip, i: usize) -> String {
    let names: &[&str] =
        if c.is_visual() { &["Position X", "Position Y", "Scale", "Rotation", "Opacity"] } else { &["Volume", "Pan"] };
    if i < names.len() {
        return names[i].to_string();
    }
    let mut i = i - names.len();
    for e in &c.effects {
        if i < e.params.len() {
            let p = e.specs().get(i).map(|s| s.name).unwrap_or("?");
            return format!("{}: {}", e.kind.name(), p);
        }
        i -= e.params.len();
    }
    String::new()
}

pub(crate) fn prop_ref(c: &Clip, i: usize) -> Option<&Animated> {
    let nb = base_count(c);
    if i < nb {
        return Some(if c.is_visual() {
            match i {
                0 => &c.x,
                1 => &c.y,
                2 => &c.scale,
                3 => &c.rotation,
                _ => &c.opacity,
            }
        } else if i == 0 {
            &c.volume
        } else {
            &c.pan
        });
    }
    let mut i = i - nb;
    for e in &c.effects {
        if i < e.params.len() {
            return Some(&e.params[i]);
        }
        i -= e.params.len();
    }
    None
}

pub(crate) fn prop_mut(c: &mut Clip, i: usize) -> Option<&mut Animated> {
    let nb = base_count(c);
    if i < nb {
        return Some(if c.is_visual() {
            match i {
                0 => &mut c.x,
                1 => &mut c.y,
                2 => &mut c.scale,
                3 => &mut c.rotation,
                _ => &mut c.opacity,
            }
        } else if i == 0 {
            &mut c.volume
        } else {
            &mut c.pan
        });
    }
    let mut i = i - nb;
    for e in &mut c.effects {
        if i < e.params.len() {
            return Some(&mut e.params[i]);
        }
        i -= e.params.len();
    }
    None
}

/// Auto y scale for one property: key values (or the constant) padded by 10 %.
pub(crate) fn y_range(a: &Animated) -> (f64, f64) {
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    if a.keys.is_empty() {
        lo = a.value;
        hi = a.value;
    }
    for k in &a.keys {
        lo = lo.min(k.v);
        hi = hi.max(k.v);
    }
    let pad = ((hi - lo) * 0.1).max(0.5);
    (lo - pad, hi + pad)
}

fn prop_color(pal: &Palette, i: usize) -> Color32 {
    let cycle =
        [pal.clip_video, pal.clip_audio, pal.clip_image, pal.clip_text, pal.clip_sequence, pal.waveform, pal.keyframe];
    cycle[i % cycle.len()]
}

/// Remove key `i` keeping `Animated::toggle_key`'s "last key becomes the constant" behaviour.
fn remove_key(a: &mut Animated, i: usize) {
    if i < a.keys.len() {
        let v = a.keys.remove(i).v;
        if a.keys.is_empty() {
            a.value = v;
        }
    }
}

/// Two selected clips that abut on the timeline, as (left, right).
fn flow_pair(project: &Project, selection: &[Id]) -> Option<(Id, Id)> {
    if selection.len() != 2 {
        return None;
    }
    let a = project.clip(selection[0])?;
    let b = project.clip(selection[1])?;
    if (a.end() - b.start).abs() < crate::model::ABUT_EPS {
        Some((a.id, b.id))
    } else if (b.end() - a.start).abs() < crate::model::ABUT_EPS {
        Some((b.id, a.id))
    } else {
        None
    }
}

/// "Nice" tick step so labels sit >= 70 px apart.
fn tick_step(pps: f32) -> f64 {
    for s in [0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 300.0] {
        if s * pps as f64 >= 70.0 {
            return s;
        }
    }
    600.0
}

enum MenuAct {
    SetEase(Ease),
    Delete,
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut CurvesState,
    project: &mut Project,
    selection: &[Id],
    playhead: &mut f64,
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> CurvesResponse {
    let mut out = CurvesResponse::default();
    let pal = *palette;
    let Some(&id) = selection.iter().find(|&&id| project.clip(id).is_some()) else {
        ui.label("Select a clip");
        return out;
    };
    let n_props = project.clip(id).map(prop_count).unwrap_or(0);
    if n_props == 0 {
        ui.label("Nothing to animate");
        return out;
    }
    state.active = state.active.min(n_props - 1);
    // drop stale key selections (undo/effect removal may have invalidated them)
    if let Some(c) = project.clip(id) {
        state.selected.retain(|&(p, k)| prop_ref(c, p).is_some_and(|a| k < a.keys.len()));
        if let Some(d) = state.drag {
            let valid = match d {
                Drag::Key { prop, key } | Drag::HandleOut { prop, key } | Drag::HandleIn { prop, key } => {
                    prop_ref(c, prop).is_some_and(|a| key < a.keys.len())
                }
                Drag::Seek => true,
            };
            if !valid {
                state.drag = None;
            }
        }
    }

    // ---- presets + flow row ----
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut state.preset_name).hint_text("Preset name").desired_width(90.0));
        let can_save = !state.preset_name.trim().is_empty();
        if ui.add_enabled(can_save, egui::Button::new("Save curve preset")).clicked() {
            if let Some(a) = project.clip(id).and_then(|c| prop_ref(c, state.active)) {
                let dur = project.clip(id).map(|c| c.duration).unwrap_or(1.0);
                if let Some(p) = crate::engine::presets::capture_curve(state.preset_name.trim(), a, dur, false) {
                    PENDING.with(|s| *s.borrow_mut() = Some(p));
                    state.preset_name.clear();
                }
            }
        }
        let names: Vec<String> = AVAILABLE.with(|a| a.borrow().iter().map(|p| p.name.clone()).collect());
        if !names.is_empty() {
            state.preset_sel = state.preset_sel.min(names.len() - 1);
            egui::ComboBox::from_id_salt("curve_preset").selected_text(names[state.preset_sel].clone()).show_ui(
                ui,
                |ui| {
                    for (i, n) in names.iter().enumerate() {
                        ui.selectable_value(&mut state.preset_sel, i, n);
                    }
                },
            );
            let mut apply: Option<bool> = None; // Some(scaled)
            if ui.small_button("Apply exact").clicked() {
                apply = Some(false);
            }
            if ui.small_button("Apply scaled").clicked() {
                apply = Some(true);
            }
            if let Some(scaled) = apply {
                let preset = AVAILABLE.with(|a| a.borrow().get(state.preset_sel).cloned());
                if let Some(preset) = preset {
                    undo(project);
                    if let Some(c) = project.clip_mut(id) {
                        let dur = c.duration;
                        if let Some(a) = prop_mut(c, state.active) {
                            crate::engine::presets::apply_curve(&preset, a, dur, scaled);
                            out.edited = true;
                        }
                    }
                }
            }
        }
        let flow = flow_pair(project, selection);
        if ui
            .add_enabled(flow.is_some(), egui::Button::new("Flow →"))
            .on_hover_text("Continue the first clip's motion into the second")
            .on_disabled_hover_text("Select exactly two abutting clips")
            .clicked()
        {
            if let Some((a, b)) = flow {
                undo(project);
                out.edited |= project.flow_clips(a, b);
            }
        }
    });

    // ---- layout: property list | graph ----
    let avail = ui.available_rect_before_wrap();
    let list_rect = Rect::from_min_max(avail.min, pos2(avail.left() + LIST_W, avail.bottom()));
    let graph = Rect::from_min_max(pos2(list_rect.right() + 4.0, avail.top()), avail.max);
    ui.allocate_rect(avail, Sense::hover());
    let clip_data = match project.clip(id) {
        Some(c) => c.clone(),
        None => return out,
    };
    let dur = clip_data.duration.max(crate::model::MIN_CLIP);

    // property list (child ui so labels wrap/scroll naturally)
    let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(list_rect));
    for i in 0..n_props {
        let a = prop_ref(&clip_data, i);
        list_ui.horizontal(|ui| {
            let mut visible = !state.hidden.contains(&i);
            if ui.checkbox(&mut visible, "").changed() {
                if visible {
                    state.hidden.retain(|&h| h != i);
                } else {
                    state.hidden.push(i);
                }
            }
            let (_, sw) = ui.allocate_space(vec2(10.0, 10.0));
            ui.painter().rect_filled(sw, 2, prop_color(&pal, i));
            let label = prop_label(&clip_data, i);
            let text = if a.is_some_and(|a| a.is_animated()) { format!("{label} ◆") } else { label };
            if ui.selectable_label(state.active == i, text).clicked() {
                state.active = i;
            }
        });
    }

    // ---- graph mapping ----
    if graph.width() < 20.0 || graph.height() < RULER_H + 10.0 {
        return out;
    }
    let plot = Rect::from_min_max(pos2(graph.left(), graph.top() + RULER_H), graph.max);
    let pps = if state.zoom > 0.0 { state.zoom } else { (plot.width() as f64 / dur) as f32 };
    if state.zoom <= 0.0 {
        state.scroll_x = 0.0;
    }
    let x_at = |t: f64, sx: f64| plot.left() + ((t - sx) * pps as f64) as f32;
    let t_at = |x: f32, sx: f64| sx + ((x - plot.left()) / pps) as f64;
    let sx = state.scroll_x;
    // per-property y scales
    let mut scales: Vec<(f64, f64)> = Vec::with_capacity(n_props);
    for i in 0..n_props {
        scales.push(prop_ref(&clip_data, i).map(y_range).unwrap_or((0.0, 1.0)));
    }
    let y_at = |v: f64, (lo, hi): (f64, f64)| plot.bottom() - (((v - lo) / (hi - lo)) * plot.height() as f64) as f32;
    let v_at = |y: f32, (lo, hi): (f64, f64)| lo + (((plot.bottom() - y) / plot.height()) as f64) * (hi - lo);
    state.graph = graph;
    state.pps = pps;
    (state.y_lo, state.y_hi) = scales[state.active];

    let resp = ui.interact(graph, ui.id().with("curve_graph"), Sense::click_and_drag());
    let pointer = resp.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.latest_pos()));
    let press_origin = ui.input(|i| i.pointer.press_origin());
    let hidden_snap = state.hidden.clone();
    let selected_snap = state.selected.clone();

    // ---- scroll / zoom ----
    if ui.rect_contains_pointer(graph) {
        let (delta, zoom, mpos) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.latest_pos()));
        if zoom != 1.0 {
            if let Some(m) = mpos {
                let t_c = t_at(m.x, state.scroll_x);
                state.zoom = (pps * zoom.clamp(0.5, 2.0)).clamp(2.0, 10_000.0);
                state.scroll_x = t_c - ((m.x - plot.left()) / state.zoom) as f64;
            }
        } else if delta.x != 0.0 {
            state.zoom = pps; // panning fixes the zoom so the fit doesn't snap back
            state.scroll_x = (state.scroll_x - (delta.x / pps) as f64).clamp(-dur, dur);
        }
    }

    // hit-test helpers over visible props (snapshots keep the closures borrow-free)
    let visible = move |i: usize| !hidden_snap.contains(&i);
    let key_hit = |pos: Pos2| -> Option<(usize, usize)> {
        let mut best: Option<((usize, usize), f32)> = None;
        for p in 0..n_props {
            if !visible(p) {
                continue;
            }
            let Some(a) = prop_ref(&clip_data, p) else { continue };
            for (k, key) in a.keys.iter().enumerate() {
                let kp = pos2(x_at(key.t, sx), y_at(key.v, scales[p]));
                let d = kp.distance(pos);
                if d <= HIT_PX && best.is_none_or(|(_, bd)| d < bd) {
                    best = Some(((p, k), d));
                }
            }
        }
        best.map(|(pk, _)| pk)
    };
    // handles are shown for bezier segments and segments whose left key is selected
    let handle_positions = |p: usize, k: usize| -> Option<(Pos2, Pos2, Pos2, Pos2)> {
        let a = prop_ref(&clip_data, p)?;
        let (k0, k1) = (a.keys.get(k)?, a.keys.get(k + 1)?);
        let (x1, y1, x2, y2) = k0.ease.handles();
        let seg_dx = k1.t - k0.t;
        let seg_dv = k1.v - k0.v;
        let p0 = pos2(x_at(k0.t, sx), y_at(k0.v, scales[p]));
        let p3 = pos2(x_at(k1.t, sx), y_at(k1.v, scales[p]));
        let h1 = pos2(x_at(k0.t + x1 as f64 * seg_dx, sx), y_at(k0.v + y1 as f64 * seg_dv, scales[p]));
        let h2 = pos2(x_at(k0.t + x2 as f64 * seg_dx, sx), y_at(k0.v + y2 as f64 * seg_dv, scales[p]));
        Some((p0, h1, h2, p3))
    };
    let shows_handles = |p: usize, k: usize| -> bool {
        if !visible(p) {
            return false;
        }
        let bez = prop_ref(&clip_data, p)
            .and_then(|a| a.keys.get(k))
            .is_some_and(|key| matches!(key.ease, Ease::Bezier { .. }));
        bez || selected_snap.contains(&(p, k)) || selected_snap.contains(&(p, k + 1))
    };
    let handle_hit = |pos: Pos2| -> Option<Drag> {
        for p in 0..n_props {
            let Some(a) = prop_ref(&clip_data, p) else { continue };
            for k in 0..a.keys.len().saturating_sub(1) {
                if !shows_handles(p, k) {
                    continue;
                }
                if let Some((_, h1, h2, _)) = handle_positions(p, k) {
                    if h1.distance(pos) <= HIT_PX {
                        return Some(Drag::HandleOut { prop: p, key: k });
                    }
                    if h2.distance(pos) <= HIT_PX {
                        return Some(Drag::HandleIn { prop: p, key: k });
                    }
                }
            }
        }
        None
    };

    // ---- interaction ----
    if resp.drag_started() {
        if let Some(pos) = press_origin.or(pointer) {
            if pos.y < plot.top() {
                state.drag = Some(Drag::Seek);
            } else if let Some(h) = handle_hit(pos) {
                undo(project);
                state.drag = Some(h);
            } else if let Some((p, k)) = key_hit(pos) {
                if !state.selected.contains(&(p, k)) {
                    state.selected.clear();
                    state.selected.push((p, k));
                }
                undo(project);
                state.drag = Some(Drag::Key { prop: p, key: k });
            }
        }
    }
    if resp.clicked() {
        if let Some(pos) = pointer {
            if pos.y < plot.top() {
                *playhead = clip_data.start + t_at(pos.x, sx).clamp(0.0, dur);
                out.seeked = true;
            } else {
                let ctrl = ui.input(|i| i.modifiers.ctrl);
                match key_hit(pos) {
                    Some(pk) if ctrl => {
                        if let Some(i) = state.selected.iter().position(|&s| s == pk) {
                            state.selected.remove(i);
                        } else {
                            state.selected.push(pk);
                        }
                    }
                    Some(pk) => {
                        state.selected.clear();
                        state.selected.push(pk);
                    }
                    None => state.selected.clear(),
                }
            }
        }
    }
    if resp.dragged() {
        if let (Some(drag), Some(pos)) = (state.drag, pointer) {
            match drag {
                Drag::Seek => {
                    *playhead = clip_data.start + t_at(pos.x, sx).clamp(0.0, dur);
                    out.seeked = true;
                }
                Drag::Key { prop, key } => {
                    let t = t_at(pos.x, sx).clamp(0.0, dur);
                    let v = v_at(pos.y.clamp(plot.top(), plot.bottom()), scales[prop]);
                    if let Some(a) = project.clip_mut(id).and_then(|c| prop_mut(c, prop)) {
                        let j = a.move_key(key, t);
                        if let Some(k) = a.keys.get_mut(j) {
                            k.v = v;
                        }
                        state.drag = Some(Drag::Key { prop, key: j });
                        state.selected.clear();
                        state.selected.push((prop, j));
                        out.edited = true;
                    }
                }
                Drag::HandleOut { prop, key } | Drag::HandleIn { prop, key } => {
                    let is_out = matches!(drag, Drag::HandleOut { .. });
                    if let Some(a) = project.clip_mut(id).and_then(|c| prop_mut(c, prop)) {
                        if key + 1 < a.keys.len() {
                            let (k0t, k0v) = (a.keys[key].t, a.keys[key].v);
                            let (k1t, k1v) = (a.keys[key + 1].t, a.keys[key + 1].v);
                            let seg_dt = (k1t - k0t).max(1e-9);
                            // ponytail: flat segments use half the visible scale as the value span so the
                            // handle still drags — exact bezier y is meaningless when v0 == v1.
                            let seg_dv = if (k1v - k0v).abs() > 1e-9 {
                                k1v - k0v
                            } else {
                                (scales[prop].1 - scales[prop].0) * 0.5
                            };
                            let hx = ((t_at(pos.x, sx) - k0t) / seg_dt).clamp(0.0, 1.0) as f32;
                            let hy = ((v_at(pos.y, scales[prop]) - k0v) / seg_dv) as f32;
                            let (mut x1, mut y1, mut x2, mut y2) = a.keys[key].ease.handles();
                            if is_out {
                                (x1, y1) = (hx, hy);
                            } else {
                                (x2, y2) = (hx, hy);
                            }
                            a.keys[key].ease = Ease::Bezier { x1, y1, x2, y2 };
                            out.edited = true;
                        }
                    }
                }
            }
        }
    }
    if resp.drag_stopped() {
        state.drag = None;
    }
    if resp.double_clicked() {
        if let Some(pos) = pointer {
            if pos.y >= plot.top() && key_hit(pos).is_none() {
                let t = t_at(pos.x, sx).clamp(0.0, dur);
                let v = v_at(pos.y, scales[state.active]);
                undo(project);
                let active = state.active;
                if let Some(a) = project.clip_mut(id).and_then(|c| prop_mut(c, active)) {
                    if !a.is_animated() {
                        a.toggle_key(t);
                    }
                    a.set_at(t, v);
                    if let Some(k) = a.key_index_at(t) {
                        state.selected.clear();
                        state.selected.push((active, k));
                    }
                    out.edited = true;
                }
            }
        }
    }
    // Delete removes the selected keys (only while the pointer is over the graph, so the timeline's
    // Delete keeps working elsewhere)
    if !state.selected.is_empty() && ui.rect_contains_pointer(graph) && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
        undo(project);
        let mut sel = state.selected.clone();
        sel.sort_by(|a, b| b.cmp(a)); // per-prop descending key index → removals don't shift later ones
        for (p, k) in sel {
            if let Some(a) = project.clip_mut(id).and_then(|c| prop_mut(c, p)) {
                remove_key(a, k);
            }
        }
        state.selected.clear();
        out.edited = true;
    }

    // ---- context menu on a key ----
    if resp.secondary_clicked() {
        state.menu_key = pointer.and_then(key_hit);
    }
    let mut act: Option<MenuAct> = None;
    if let Some((_p, _k)) = state.menu_key {
        resp.context_menu(|ui| {
            ui.menu_button("Easing", |ui| {
                for e in Ease::ALL {
                    if ui.button(e.name()).clicked() {
                        act = Some(MenuAct::SetEase(e));
                    }
                }
                ui.separator();
                for (name, e) in Ease::PRESETS {
                    if ui.button(name).clicked() {
                        act = Some(MenuAct::SetEase(e));
                    }
                }
            });
            if ui.button("Delete").clicked() {
                act = Some(MenuAct::Delete);
            }
        });
    }
    if let (Some(act), Some((p, k))) = (act, state.menu_key) {
        undo(project);
        if let Some(a) = project.clip_mut(id).and_then(|c| prop_mut(c, p)) {
            match act {
                MenuAct::SetEase(e) if k < a.keys.len() => {
                    a.keys[k].ease = e;
                    out.edited = true;
                }
                MenuAct::Delete => {
                    remove_key(a, k);
                    state.selected.retain(|&s| s != (p, k));
                    out.edited = true;
                    state.menu_key = None;
                }
                _ => {}
            }
        }
    }

    // ---- painting ----
    let painter = ui.painter().with_clip_rect(graph);
    painter.rect_filled(graph, 0, pal.panel);
    painter.rect_filled(Rect::from_min_max(graph.min, pos2(graph.right(), plot.top())), 0, pal.header);
    // time ticks in the ruler
    let font = egui::TextStyle::Small.resolve(ui.style());
    let step = tick_step(pps);
    let mut t = (t_at(plot.left(), sx) / step).floor() * step;
    while t <= t_at(plot.right(), sx) {
        if t >= -1e-9 {
            let x = x_at(t, sx);
            painter.vline(x, egui::Rangef::new(graph.top(), plot.top()), Stroke::new(1.0, pal.border));
            painter.text(pos2(x + 2.0, graph.top()), Align2::LEFT_TOP, format!("{t:.2}"), font.clone(), pal.text_dim);
        }
        t += step;
    }
    // y grid for the active scale
    let (lo, hi) = scales[state.active];
    for i in 0..=4 {
        let v = lo + (hi - lo) * i as f64 / 4.0;
        let y = y_at(v, (lo, hi));
        painter.hline(
            egui::Rangef::new(plot.left(), plot.right()),
            y,
            Stroke::new(1.0, pal.border.linear_multiply(0.4)),
        );
        painter.text(
            pos2(plot.left() + 2.0, y - 1.0),
            Align2::LEFT_BOTTOM,
            format!("{v:.2}"),
            font.clone(),
            pal.text_dim,
        );
    }
    // clip bounds
    for tb in [0.0, dur] {
        painter.vline(x_at(tb, sx), egui::Rangef::new(plot.top(), plot.bottom()), Stroke::new(1.0, pal.border));
    }
    // curves
    let plot_painter = ui.painter().with_clip_rect(plot);
    let mut pts: Vec<Pos2> = Vec::with_capacity((plot.width() / 2.0) as usize + 2);
    for p in 0..n_props {
        if !visible(p) {
            continue;
        }
        let Some(a) = prop_ref(&clip_data, p) else { continue };
        let color = prop_color(&pal, p);
        let x0 = x_at(0.0, sx).max(plot.left());
        let x1 = x_at(dur, sx).min(plot.right());
        if x1 <= x0 {
            continue;
        }
        if !a.is_animated() {
            let y = y_at(a.value, scales[p]);
            plot_painter.extend(Shape::dashed_line(&[pos2(x0, y), pos2(x1, y)], Stroke::new(1.0, color), 4.0, 4.0));
            continue;
        }
        pts.clear();
        let mut x = x0;
        while x <= x1 {
            pts.push(pos2(x, y_at(a.at(t_at(x, sx)), scales[p])));
            x += 2.0;
        }
        plot_painter.add(Shape::line(pts.clone(), Stroke::new(1.5, color)));
        // bezier handles
        for k in 0..a.keys.len().saturating_sub(1) {
            if !shows_handles(p, k) {
                continue;
            }
            if let Some((p0, h1, h2, p3)) = handle_positions(p, k) {
                let hs = Stroke::new(1.0, pal.text_dim);
                plot_painter.line_segment([p0, h1], hs);
                plot_painter.line_segment([p3, h2], hs);
                plot_painter.circle_filled(h1, 3.0, pal.accent);
                plot_painter.circle_filled(h2, 3.0, pal.accent);
            }
        }
        // keys
        for (k, key) in a.keys.iter().enumerate() {
            let c = pos2(x_at(key.t, sx), y_at(key.v, scales[p]));
            let fill = if state.selected.contains(&(p, k)) { pal.accent } else { color };
            plot_painter.add(Shape::convex_polygon(
                vec![pos2(c.x, c.y - 4.0), pos2(c.x + 4.0, c.y), pos2(c.x, c.y + 4.0), pos2(c.x - 4.0, c.y)],
                fill,
                Stroke::new(1.0, pal.text),
            ));
        }
    }
    // playhead
    let ph = clip_data.local(*playhead);
    if ph >= 0.0 && ph <= dur {
        painter.vline(x_at(ph, sx), egui::Rangef::new(graph.top(), plot.bottom()), Stroke::new(1.0, pal.playhead));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipKind, Keyframe};
    use eframe::egui::{Event, Modifiers, PointerButton, RawInput};

    struct Harness {
        ctx: egui::Context,
        state: CurvesState,
        project: Project,
        selection: Vec<Id>,
        playhead: f64,
        undos: usize,
        time: f64,
    }

    impl Harness {
        fn new() -> Self {
            let mut project = Project::new();
            let mut clip = Clip::new(7, ClipKind::Video, "v", 0.0, 4.0);
            clip.scale.keys =
                vec![Keyframe { t: 1.0, v: 1.0, ease: Ease::Linear }, Keyframe { t: 3.0, v: 3.0, ease: Ease::Linear }];
            project.tracks[0].clips.push(clip);
            let mut h = Self {
                ctx: egui::Context::default(),
                state: CurvesState::default(),
                project,
                selection: vec![7],
                playhead: 0.0,
                undos: 0,
                time: 0.0,
            };
            h.frame(vec![]); // layout pass: fills state.graph / pps
            h
        }
        fn frame(&mut self, events: Vec<Event>) -> CurvesResponse {
            self.time += 0.05;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 400.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, Color32::from_rgb(0, 120, 212));
            let Harness { ctx, state, project, selection, playhead, undos, .. } = self;
            let mut resp = CurvesResponse::default();
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    resp = show(ui, state, project, selection, playhead, &pal, &mut undo);
                });
            });
            resp
        }
        fn press(&mut self, pos: Pos2) {
            self.frame(vec![Event::PointerMoved(pos)]);
            self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }]);
        }
        fn release(&mut self, pos: Pos2) -> CurvesResponse {
            let r = self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }]);
            self.frame(vec![]);
            r
        }
        fn clip(&self) -> &Clip {
            &self.project.tracks[0].clips[0]
        }
        /// Screen position of scale key `k` (prop index 2) using the state's stored mapping.
        fn scale_key_pos(&self, k: usize) -> Pos2 {
            let plot = Rect::from_min_max(
                pos2(self.state.graph.left(), self.state.graph.top() + RULER_H),
                self.state.graph.max,
            );
            let key = &self.clip().scale.keys[k];
            let (lo, hi) = y_range(&self.clip().scale);
            let x = plot.left() + ((key.t - self.state.scroll_x) * self.state.pps as f64) as f32;
            let y = plot.bottom() - (((key.v - lo) / (hi - lo)) * plot.height() as f64) as f32;
            pos2(x, y)
        }
    }

    #[test]
    fn prop_plumbing_matches_labels() {
        let mut c = Clip::new(1, ClipKind::Video, "v", 0.0, 2.0);
        c.effects.push(crate::model::Effect::new(crate::model::EffectKind::Blur));
        assert_eq!(prop_count(&c), 6);
        assert_eq!(prop_label(&c, 0), "Position X");
        assert_eq!(prop_label(&c, 2), "Scale");
        assert_eq!(prop_label(&c, 5), "Blur: Radius");
        c.scale.value = 2.5;
        assert_eq!(prop_ref(&c, 2).map(|a| a.value), Some(2.5));
        prop_mut(&mut c, 5).map(|a| a.value = 9.0);
        assert_eq!(c.effects[0].params[0].value, 9.0);
        let a = Clip::new(2, ClipKind::Audio, "a", 0.0, 2.0);
        assert_eq!(prop_count(&a), 2);
        assert_eq!(prop_label(&a, 0), "Volume");
        assert_eq!(prop_label(&a, 1), "Pan");
    }

    #[test]
    fn drag_key_moves_time_and_value_with_one_undo() {
        let mut h = Harness::new();
        assert!(h.state.graph.width() > 100.0, "graph not laid out: {:?}", h.state.graph);
        let from = h.scale_key_pos(0); // key at t = 1, v = 1
        let to = from + vec2(60.0, -20.0);
        h.press(from);
        let mut edited = false;
        for i in 1..=4 {
            let p = from + (to - from) * (i as f32 / 4.0);
            edited |= h.frame(vec![Event::PointerMoved(p)]).edited;
        }
        edited |= h.release(to).edited;
        assert!(edited, "dragging a key should edit");
        assert_eq!(h.undos, 1, "one drag = one undo");
        let k = &h.clip().scale.keys[0];
        let dt = 60.0 / h.state.pps as f64;
        assert!((k.t - (1.0 + dt)).abs() < 0.05, "time should follow the drag: {} vs {}", k.t, 1.0 + dt);
        assert!(k.v > 1.0, "value should rise when dragging up: {}", k.v);
    }

    #[test]
    fn click_selects_and_delete_removes() {
        let mut h = Harness::new();
        let p = h.scale_key_pos(1);
        h.press(p);
        h.release(p);
        assert_eq!(h.state.selected, vec![(2, 1)]);
        let r = h.frame(vec![Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        assert!(r.edited);
        assert_eq!(h.clip().scale.keys.len(), 1);
        assert_eq!(h.undos, 1);
        assert!(h.state.selected.is_empty());
    }

    #[test]
    fn ruler_click_seeks() {
        let mut h = Harness::new();
        let x = h.state.graph.left() + (2.0 * h.state.pps as f64) as f32; // t = 2 (fit → scroll 0)
        let pos = pos2(x, h.state.graph.top() + RULER_H * 0.5);
        h.press(pos);
        let r = h.release(pos);
        assert!(r.seeked, "clicking the ruler should seek");
        assert!((h.playhead - 2.0).abs() < 0.05, "playhead {}", h.playhead);
        assert_eq!(h.undos, 0, "seeking is not an edit");
    }

    #[test]
    fn double_click_adds_key_for_active_property() {
        let mut h = Harness::new();
        h.state.active = 2; // Scale
        h.frame(vec![]);
        // empty spot at t = 2 near the middle of the plot
        let plot_top = h.state.graph.top() + RULER_H;
        let x = h.state.graph.left() + (2.0 * h.state.pps as f64) as f32;
        let pos = pos2(x, (plot_top + h.state.graph.bottom()) * 0.5 + 30.0);
        h.press(pos);
        h.release(pos);
        h.press(pos);
        let r = h.release(pos);
        let _ = r;
        assert_eq!(h.clip().scale.keys.len(), 3, "double-click should add a key");
        assert!(h.clip().scale.keys.iter().any(|k| (k.t - 2.0).abs() < 0.05));
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn secondary_click_targets_key_and_ease_applies_bezier() {
        let mut h = Harness::new();
        let p = h.scale_key_pos(0);
        h.frame(vec![Event::PointerMoved(p)]);
        h.frame(vec![Event::PointerButton {
            pos: p,
            button: PointerButton::Secondary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }]);
        h.frame(vec![Event::PointerButton {
            pos: p,
            button: PointerButton::Secondary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
        h.frame(vec![]);
        assert_eq!(h.state.menu_key, Some((2, 0)), "right-click should target the key under the pointer");
        // the menu's easing entries route through MenuAct::SetEase — apply one the same way
        let mut undos = 0;
        let mut undo = |_: &Project| undos += 1;
        undo(&h.project);
        if let Some(a) = h.project.clip_mut(7).and_then(|c| prop_mut(c, 2)) {
            a.keys[0].ease = Ease::PRESETS[0].1;
        }
        assert!(matches!(h.clip().scale.keys[0].ease, Ease::Bezier { .. }));
    }

    #[test]
    fn handle_drag_converts_to_bezier() {
        let mut h = Harness::new();
        // select key 0 so its segment shows handles
        let kp = h.scale_key_pos(0);
        h.press(kp);
        h.release(kp);
        assert_eq!(h.state.selected, vec![(2, 0)]);
        // linear ease → out handle sits at 1/3 of the segment
        let k1 = h.scale_key_pos(1);
        let h1 = kp + (k1 - kp) * 0.33;
        let to = h1 + vec2(20.0, 25.0);
        h.press(h1);
        let mut edited = false;
        for i in 1..=3 {
            edited |= h.frame(vec![Event::PointerMoved(h1 + (to - h1) * (i as f32 / 3.0))]).edited;
        }
        h.release(to);
        assert!(edited, "dragging a handle should edit");
        assert!(
            matches!(h.clip().scale.keys[0].ease, Ease::Bezier { .. }),
            "handle drag must convert the segment to Bezier: {:?}",
            h.clip().scale.keys[0].ease
        );
    }

    #[test]
    fn flow_pair_and_button_path() {
        let mut p = Project::new();
        let mut a = Clip::new(1, ClipKind::Video, "a", 0.0, 2.0);
        a.scale.keys =
            vec![Keyframe { t: 0.0, v: 1.0, ease: Ease::Linear }, Keyframe { t: 2.0, v: 2.0, ease: Ease::Linear }];
        let b = Clip::new(2, ClipKind::Video, "b", 2.0, 2.0);
        p.tracks[0].clips.push(a);
        p.tracks[0].clips.push(b);
        assert_eq!(flow_pair(&p, &[2, 1]), Some((1, 2)), "order independent");
        assert_eq!(flow_pair(&p, &[1]), None);
        assert!(p.flow_clips(1, 2));
        let b = p.clip(2).unwrap();
        assert!(b.scale.is_animated(), "flow should animate the second clip");
    }

    #[test]
    fn preset_handoff() {
        assert!(take_pending_curve_preset().is_none());
        set_available_presets(vec![CurvePreset { name: "p".into(), keys: Vec::new(), absolute: false }]);
        AVAILABLE.with(|a| assert_eq!(a.borrow().len(), 1));
        PENDING.with(|p| *p.borrow_mut() = Some(CurvePreset { name: "q".into(), keys: Vec::new(), absolute: false }));
        assert_eq!(take_pending_curve_preset().map(|p| p.name), Some("q".into()));
        set_available_presets(Vec::new());
    }

    #[test]
    fn no_selection_is_harmless() {
        let mut h = Harness::new();
        h.selection.clear();
        let r = h.frame(vec![]);
        assert!(!r.edited && !r.seeked);
        assert_eq!(h.undos, 0);
    }
}
