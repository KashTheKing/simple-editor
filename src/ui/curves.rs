//! Curve (graph) editor for the FIRST selected clip: x = clip-local time over [0, duration], y = value.
//! Left: a list of the clip's keyframeable properties (`Clip::props_mut` names + "Volume"/"Pan"/"Speed" +
//! "<effect>: <param>" for effect params) with a colour swatch and a checkbox to show/hide each curve
//! (auto-scaled per property; the selected property's scale drives the y axis labels). Right: the graph —
//! curves drawn with the eased segments (sample `Animated::at` every ~2 px), keys as diamonds (accent when
//! selected), playhead line (click on the ruler strip seeks → `seeked`), constant (unkeyed) properties
//! shown as a flat dashed line. Interaction: drag a key horizontally/vertically (time clamped to the clip,
//! vertical changes the value; undo once at drag start) — the whole selection moves with it, click a key
//! to select it, Ctrl+click adds/removes one, dragging empty graph space rubber-bands a group (like the
//! timeline's clip band), Delete removes the selection and Ctrl+C / Ctrl+V copy it and paste it at the
//! playhead keeping the relative times, double-click on empty graph area adds a key for the active
//! property at that (t, v), right-click a key → easing menu (Ease::ALL) + Delete; Ctrl+Scroll zooms the
//! time axis, Shift+Scroll pans, plain Scroll zooms the value axis around the pointer and "Fit" frames
//! every key. Bezier velocity handles: a Bezier segment (or any segment whose left key is
//! selected) shows its two handles; dragging one converts the segment to `Ease::Bezier`.

use crate::model::{Animated, Clip, Ease, Id, Keyframe, Project};
use crate::settings::{CurvePreset, MotionPreset};
use crate::theme::Palette;
use eframe::egui::{self, pos2, vec2, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke};
use std::cell::RefCell;

pub(crate) const RULER_H: f32 = 16.0;
const LIST_W: f32 = 150.0;
const LIST_MIN: f32 = 90.0;
const LIST_MAX: f32 = 320.0;
/// Grab width of the splitter between the property list and the graph.
const SPLIT_W: f32 = 5.0;
const HIT_PX: f32 = 6.0;
/// Two keys closer than this in time are the same key (model::KEY_EPS, which is private).
const T_EPS: f64 = 1e-4;

thread_local! {
    // ponytail: thread_local hand-off because show() can't reach Settings. The app calls
    // set_available_presets() each frame (or when they change) and polls take_pending_curve_preset().
    static AVAILABLE: RefCell<Vec<CurvePreset>> = const { RefCell::new(Vec::new()) };
    static PENDING: RefCell<Option<CurvePreset>> = const { RefCell::new(None) };
    // motion presets (all properties of a clip at once): built-ins + Settings.motion_presets
    static MOTIONS: RefCell<Vec<MotionPreset>> = const { RefCell::new(Vec::new()) };
    static PENDING_MOTION: RefCell<Option<MotionPreset>> = const { RefCell::new(None) };
}

/// The app hands the applicable motion presets (built-ins + saved) to the panel.
pub fn set_available_motions(v: Vec<MotionPreset>) {
    MOTIONS.with(|m| *m.borrow_mut() = v);
}

/// A motion preset captured via "Save motion" waiting for the app to store it in Settings.
pub fn take_pending_motion_preset() -> Option<MotionPreset> {
    PENDING_MOTION.with(|p| p.borrow_mut().take())
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
    /// Width of the property list in points (drag the splitter; remembered across frames).
    pub list_w: f32,
    /// Hidden curves (property indices).
    pub hidden: Vec<usize>,
    /// Time zoom/pan of the graph (seconds at the left edge, px per second; 0 = fit the clip).
    pub scroll_x: f64,
    pub zoom: f32,
    /// Value-axis view on top of each property's auto range: 1 = the whole range, pan in range units.
    pub y_zoom: f64,
    pub y_pan: f64,
    /// Selected keys as (property index, key index).
    pub selected: Vec<(usize, usize)>,
    /// Last frame's graph rect / effective px-per-second / active y scale (hit-testing + tests).
    pub graph: Rect,
    pub pps: f32,
    pub y_lo: f64,
    pub y_hi: f64,
    drag: Option<Drag>,
    /// y scales latched per property for the running key/handle drag: the per-frame auto scale is derived
    /// from the very values the drag writes, so a live scale would feed the dragged value back into itself.
    drag_y: Vec<(usize, (f64, f64))>,
    /// The selected keys as they were when the drag started, per property: (prop, keys, selected indices).
    /// The whole gesture is re-derived from this snapshot, so nothing accumulates rounding.
    drag_keys: Vec<(usize, Vec<Keyframe>, Vec<usize>)>,
    /// Pointer position at drag start — the group moves by the delta from it.
    drag_from: Option<Pos2>,
    /// Rubber-band press origin while a band select is in progress.
    band: Option<Pos2>,
    /// Copied keys as (property, key) with times relative to the earliest of them.
    clipboard: Vec<(usize, Keyframe)>,
    preset_name: String,
    preset_sel: usize,
    motion_sel: usize,
    /// Key under the open context menu.
    menu_key: Option<(usize, usize)>,
}

impl Default for CurvesState {
    fn default() -> Self {
        Self {
            active: 0,
            list_w: LIST_W,
            hidden: Vec::new(),
            scroll_x: 0.0,
            zoom: 0.0,
            y_zoom: 1.0,
            y_pan: 0.0,
            selected: Vec::new(),
            graph: Rect::NOTHING,
            pps: 0.0,
            y_lo: 0.0,
            y_hi: 1.0,
            drag: None,
            drag_y: Vec::new(),
            drag_keys: Vec::new(),
            drag_from: None,
            band: None,
            clipboard: Vec::new(),
            preset_name: String::new(),
            preset_sel: 0,
            motion_sel: 0,
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
        6
    } else {
        3
    }
}

pub(crate) fn prop_count(c: &Clip) -> usize {
    base_count(c) + c.effects.iter().map(|e| e.params.len()).sum::<usize>()
}

pub(crate) fn prop_label(c: &Clip, i: usize) -> String {
    let names: &[&str] = if c.is_visual() {
        &["Position X", "Position Y", "Scale", "Rotation", "Opacity", "Speed"]
    } else {
        &["Volume", "Pan", "Speed"]
    };
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
                4 => &c.opacity,
                _ => &c.speed_curve,
            }
        } else {
            match i {
                0 => &c.volume,
                1 => &c.pan,
                _ => &c.speed_curve,
            }
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
                4 => &mut c.opacity,
                _ => &mut c.speed_curve,
            }
        } else {
            match i {
                0 => &mut c.volume,
                1 => &mut c.pan,
                _ => &mut c.speed_curve,
            }
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

/// The value-axis view applied to a property's auto range (`pan` in units of that range, so one pair of
/// numbers drives every property's lane whatever its units are).
fn zoomed((lo, hi): (f64, f64), zoom: f64, pan: f64) -> (f64, f64) {
    let span = hi - lo;
    let c = (lo + hi) * 0.5 + pan * span;
    let h = span * 0.5 / zoom.max(0.01);
    (c - h, c + h)
}

/// Re-lay `orig` with the keys at `sel` shifted by (dt, dv) and write it back; returns their new indices.
/// A moved key landing on an unmoved one replaces it, exactly like `Animated::move_key`.
fn move_group(a: &mut Animated, orig: &[Keyframe], sel: &[usize], dt: f64, dv: f64) -> Vec<usize> {
    let mut v: Vec<(Keyframe, bool)> = orig
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let m = sel.contains(&i);
            (Keyframe { t: k.t + if m { dt } else { 0.0 }, v: k.v + if m { dv } else { 0.0 }, ease: k.ease }, m)
        })
        .collect();
    v.sort_by(|a, b| a.0.t.total_cmp(&b.0.t));
    // ponytail: O(n²) collision scan — a property with thousands of keys would notice, nothing else will
    let moved: Vec<f64> = v.iter().filter(|(_, m)| *m).map(|(k, _)| k.t).collect();
    v.retain(|(k, m)| *m || !moved.iter().any(|t| (t - k.t).abs() < T_EPS));
    a.keys = v.iter().map(|(k, _)| *k).collect();
    v.iter().enumerate().filter(|(_, (_, m))| *m).map(|(i, _)| i).collect()
}

/// Frame every key of the visible properties on the time axis and reset the value view.
fn fit_view(state: &mut CurvesState, c: &Clip, n_props: usize) {
    state.y_zoom = 1.0;
    state.y_pan = 0.0;
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for p in 0..n_props {
        if state.hidden.contains(&p) {
            continue;
        }
        for k in prop_ref(c, p).map(|a| a.keys.as_slice()).unwrap_or(&[]) {
            lo = lo.min(k.t);
            hi = hi.max(k.t);
        }
    }
    let w = state.graph.width() as f64;
    if hi > lo && w > 20.0 {
        let pad = (hi - lo) * 0.05;
        state.zoom = (w / (hi - lo + 2.0 * pad)).clamp(2.0, 10_000.0) as f32;
        state.scroll_x = lo - pad;
    } else {
        state.zoom = 0.0; // one key or none to frame: back to fitting the whole clip
        state.scroll_x = 0.0;
    }
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

/// Add (or move) a key of the active property at clip-local time `t` and value `v`, selecting it.
/// Used by both the double-click and the right-click-on-empty-graph gestures.
fn add_key_at(
    project: &mut Project,
    id: Id,
    state: &mut CurvesState,
    out: &mut CurvesResponse,
    undo: &mut dyn FnMut(&Project),
    t: f64,
    v: f64,
) {
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

    // ---- motion presets (every animated property of the clip at once) ----
    ui.horizontal(|ui| {
        let motions: Vec<String> = MOTIONS.with(|m| m.borrow().iter().map(|p| p.name.clone()).collect());
        ui.label("Motion");
        if motions.is_empty() {
            ui.weak("(none)");
        } else {
            state.motion_sel = state.motion_sel.min(motions.len() - 1);
            egui::ComboBox::from_id_salt("motion_preset").selected_text(motions[state.motion_sel].clone()).show_ui(
                ui,
                |ui| {
                    for (i, n) in motions.iter().enumerate() {
                        ui.selectable_value(&mut state.motion_sel, i, n);
                    }
                },
            );
            let mut apply: Option<bool> = None; // Some(scaled)
            if ui.small_button("Apply").on_hover_text("Stretch the preset to this clip's length").clicked() {
                apply = Some(true);
            }
            if ui.small_button("Apply exact").on_hover_text("Keep the preset's own timing").clicked() {
                apply = Some(false);
            }
            let merge = ui
                .small_button("Merge")
                .on_hover_text("Add the preset's keys from the playhead on, keeping what is already there")
                .clicked();
            if apply.is_some() || merge {
                let preset = MOTIONS.with(|m| m.borrow().get(state.motion_sel).cloned());
                if let Some(preset) = preset {
                    undo(project);
                    let ph = *playhead;
                    if let Some(c) = project.clip_mut(id) {
                        match apply {
                            Some(scaled) => crate::engine::presets::apply_motion(&preset, c, scaled),
                            None => {
                                let off = c.local(ph).clamp(0.0, c.duration);
                                crate::engine::presets::merge_motion(&preset, c, off);
                            }
                        }
                        out.edited = true;
                    }
                }
            }
        }
        let can_save = !state.preset_name.trim().is_empty();
        if ui
            .add_enabled(can_save, egui::Button::new("Save motion"))
            .on_hover_text("Save every animated property of this clip under the name on the left")
            .clicked()
        {
            if let Some(c) = project.clip(id) {
                PENDING_MOTION.with(|s| {
                    *s.borrow_mut() = Some(crate::engine::presets::capture_motion(state.preset_name.trim(), c))
                });
                state.preset_name.clear();
            }
        }
    });

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
        if ui
            .small_button("Fit")
            .on_hover_text("Frame every keyframe (Ctrl+Scroll zooms time, Scroll zooms values)")
            .clicked()
        {
            if let Some(c) = project.clip(id) {
                fit_view(state, c, n_props);
            }
        }
    });

    // ---- layout: property list | graph ----
    let avail = ui.available_rect_before_wrap();
    state.list_w = state.list_w.clamp(LIST_MIN, LIST_MAX.min((avail.width() - 40.0).max(LIST_MIN)));
    let list_rect = Rect::from_min_max(avail.min, pos2(avail.left() + state.list_w, avail.bottom()));
    let split =
        Rect::from_min_max(pos2(list_rect.right(), avail.top()), pos2(list_rect.right() + SPLIT_W, avail.bottom()));
    let graph = Rect::from_min_max(pos2(split.right(), avail.top()), avail.max);
    ui.allocate_rect(avail, Sense::hover());

    // splitter: drag to resize the property list so long "<Effect>: <Param>" names stay readable
    let sr = ui.interact(split, ui.id().with("curve_split"), Sense::drag());
    if sr.hovered() || sr.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if sr.dragged() {
        state.list_w = (state.list_w + sr.drag_delta().x).clamp(LIST_MIN, LIST_MAX);
    }
    ui.painter().rect_filled(
        split.shrink2(vec2(1.0, 0.0)),
        0,
        if sr.hovered() || sr.dragged() { pal.accent } else { pal.border },
    );
    let clip_data = match project.clip(id) {
        Some(c) => c.clone(),
        None => return out,
    };
    let dur = clip_data.duration.max(crate::model::MIN_CLIP);

    // property list (child ui so labels wrap/scroll naturally)
    let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(list_rect));
    // scrolled: a clip with a few multi-param effects overflows the short bottom-tile pane
    egui::ScrollArea::vertical().id_salt("curve_props").show(&mut list_ui, |ui| {
        for i in 0..n_props {
            let a = prop_ref(&clip_data, i);
            ui.horizontal(|ui| {
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
                if ui.selectable_label(state.active == i, prop_label(&clip_data, i)).clicked() {
                    state.active = i;
                }
                // a keyframe diamond marks the properties that actually carry keys
                if a.is_some_and(|a| a.is_animated()) {
                    crate::ui::tools::glyph_label(ui, crate::ui::tools::Glyph::Diamond, pal.text_dim);
                }
            });
        }
    });

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
        let auto = prop_ref(&clip_data, i).map(y_range).unwrap_or((0.0, 1.0));
        scales.push(zoomed(auto, state.y_zoom, state.y_pan));
    }
    // freeze the dragged properties' scales for the whole gesture (see CurvesState::drag_y)
    for &(p, ys) in &state.drag_y {
        if let Some(s) = scales.get_mut(p) {
            *s = ys;
        }
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
        } else if delta.y != 0.0 {
            // plain wheel zooms the value axis, keeping the value under the pointer put:
            // u = centre + half * (2f - 1) must not move, so the centre takes up the slack
            let f = mpos.map(|m| ((plot.bottom() - m.y) / plot.height()).clamp(0.0, 1.0) as f64).unwrap_or(0.5);
            let h0 = 0.5 / state.y_zoom;
            state.y_zoom = (state.y_zoom * (1.0 + delta.y as f64 * 0.01)).clamp(0.05, 100.0);
            state.y_pan += (h0 - 0.5 / state.y_zoom) * (2.0 * f - 1.0);
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
                state.drag_y = match h {
                    Drag::HandleOut { prop, .. } | Drag::HandleIn { prop, .. } => {
                        scales.get(prop).map(|&s| (prop, s)).into_iter().collect()
                    }
                    _ => Vec::new(),
                };
            } else if let Some((p, k)) = key_hit(pos) {
                if !state.selected.contains(&(p, k)) {
                    state.selected.clear();
                    state.selected.push((p, k));
                }
                undo(project);
                state.drag = Some(Drag::Key { prop: p, key: k });
                state.drag_from = Some(pos);
                // snapshot every property holding a selected key: the whole selection moves as one
                let mut props: Vec<usize> = state.selected.iter().map(|s| s.0).collect();
                props.sort_unstable();
                props.dedup();
                state.drag_y = props.iter().filter_map(|&p| scales.get(p).map(|&s| (p, s))).collect();
                state.drag_keys = props
                    .iter()
                    .filter_map(|&p| {
                        let a = prop_ref(&clip_data, p)?;
                        let idx: Vec<usize> = state.selected.iter().filter(|s| s.0 == p).map(|s| s.1).collect();
                        (!idx.is_empty()).then(|| (p, a.keys.clone(), idx))
                    })
                    .collect();
            } else {
                state.band = Some(pos); // empty graph space: rubber-band a group, like the timeline's clips
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
                    None if !ctrl => state.selected.clear(),
                    None => {}
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
                    let o = state.drag_from.unwrap_or(pos);
                    let dx = ((pos.x - o.x) / pps) as f64;
                    let dy = (o.y - pos.y.clamp(plot.top(), plot.bottom())) / plot.height();
                    let (mut sel, mut anchor) = (Vec::new(), key);
                    for (p, orig, idx) in state.drag_keys.clone() {
                        // clamp the shift so the group keeps its shape when it hits the clip's edges
                        let (lo, hi) = idx
                            .iter()
                            .filter_map(|&i| orig.get(i))
                            .fold((f64::MAX, f64::MIN), |(a, b), k| (a.min(k.t), b.max(k.t)));
                        let dt = dx.clamp(-lo, (dur - hi).max(-lo));
                        let dv = dy as f64 * (scales[p].1 - scales[p].0);
                        let Some(a) = project.clip_mut(id).and_then(|c| prop_mut(c, p)) else { continue };
                        let new = move_group(a, &orig, &idx, dt, dv);
                        if p == prop {
                            if let Some(j) = idx.iter().position(|&i| i == key) {
                                anchor = new.get(j).copied().unwrap_or(key);
                            }
                        }
                        sel.extend(new.into_iter().map(|k| (p, k)));
                        out.edited = true;
                    }
                    state.drag = Some(Drag::Key { prop, key: anchor });
                    state.selected = sel;
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
        // rubber band: everything inside it joins the selection (Ctrl keeps what was already selected)
        if let Some(o) = state.band.take() {
            let br = Rect::from_two_pos(o, pointer.unwrap_or(o));
            if !ui.input(|i| i.modifiers.ctrl) {
                state.selected.clear();
            }
            for p in (0..n_props).filter(|&p| visible(p)) {
                let Some(a) = prop_ref(&clip_data, p) else { continue };
                for (k, key) in a.keys.iter().enumerate() {
                    if br.contains(pos2(x_at(key.t, sx), y_at(key.v, scales[p]))) && !state.selected.contains(&(p, k)) {
                        state.selected.push((p, k));
                    }
                }
            }
        }
        state.drag = None;
        state.drag_y.clear();
        state.drag_keys.clear();
        state.drag_from = None;
    }
    if resp.double_clicked() {
        if let Some(pos) = pointer {
            if pos.y >= plot.top() && key_hit(pos).is_none() {
                let t = t_at(pos.x, sx).clamp(0.0, dur);
                let v = v_at(pos.y, scales[state.active]);
                add_key_at(project, id, state, &mut out, undo, t, v);
            }
        }
    }
    // Delete removes the selected keys (only while the pointer is over the graph, so the timeline's
    // Delete keeps working elsewhere)
    // consuming (not just peeking) so the app's global Delete cannot also fire and remove the clip
    if !state.selected.is_empty()
        && ui.rect_contains_pointer(graph)
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete))
    {
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

    // ---- copy / paste the selected keys (consumed, and only over the graph: Ctrl+V elsewhere is not ours) ----
    if ui.rect_contains_pointer(graph) {
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::C)) && !state.selected.is_empty() {
            let key_of =
                |&(p, k): &(usize, usize)| prop_ref(&clip_data, p).and_then(|a| a.keys.get(k)).map(|x| (p, *x));
            let picked: Vec<(usize, Keyframe)> = state.selected.iter().filter_map(key_of).collect();
            let t0 = picked.iter().map(|(_, k)| k.t).fold(f64::MAX, f64::min);
            state.clipboard = picked.into_iter().map(|(p, k)| (p, Keyframe { t: k.t - t0, ..k })).collect();
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::V)) && !state.clipboard.is_empty() {
            undo(project);
            let t0 = clip_data.local(*playhead).clamp(0.0, dur);
            let mut at: Vec<(usize, f64)> = Vec::new();
            for (p, key) in state.clipboard.clone() {
                let t = (t0 + key.t).clamp(0.0, dur);
                if let Some(a) = project.clip_mut(id).and_then(|c| prop_mut(c, p)) {
                    if a.key_index_at(t).is_none() {
                        a.toggle_key(t); // makes an unkeyed property animated too
                    }
                    a.set_at(t, key.v);
                    a.set_ease_at(t, key.ease);
                    at.push((p, t));
                }
            }
            // indices only settle once every paste is in
            if let Some(c) = project.clip(id) {
                state.selected =
                    at.iter().filter_map(|&(p, t)| prop_ref(c, p)?.key_index_at(t).map(|k| (p, k))).collect();
            }
            out.edited = true;
        }
    }

    // ---- context menu on a key / right-click adds a key ----
    if resp.secondary_clicked() {
        state.menu_key = pointer.and_then(key_hit);
        // empty graph area: right-click adds a key for the active property right there (same as
        // double-click, which stays as the second way in)
        if state.menu_key.is_none() {
            if let Some(pos) = pointer {
                if pos.y >= plot.top() {
                    add_key_at(
                        project,
                        id,
                        state,
                        &mut out,
                        undo,
                        t_at(pos.x, sx).clamp(0.0, dur),
                        v_at(pos.y, scales[state.active]),
                    );
                }
            }
        }
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
    // y grid for the active scale, with the lane's range spelled out in the ruler
    let (lo, hi) = scales[state.active];
    painter.text(
        pos2(graph.right() - 3.0, graph.top()),
        Align2::RIGHT_TOP,
        format!("{} {lo:.2} … {hi:.2}", prop_label(&clip_data, state.active)),
        font.clone(),
        pal.text_dim,
    );
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
    // rubber band in progress
    if let Some(o) = state.band {
        let br = Rect::from_two_pos(o, pointer.unwrap_or(o)).intersect(plot);
        plot_painter.rect_filled(br, 0, pal.accent.gamma_multiply(0.15));
        plot_painter.rect_stroke(br, 0, Stroke::new(1.0, pal.accent), egui::StrokeKind::Inside);
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
    fn splitter_drag_resizes_the_property_list() {
        let mut h = Harness::new();
        let w0 = h.state.list_w;
        assert_eq!(w0, LIST_W);
        // grab the splitter strip just right of the list and drag it right
        let y = h.state.graph.center().y;
        let x = h.state.graph.left() - SPLIT_W / 2.0;
        h.press(pos2(x, y));
        for i in 1..=4 {
            h.frame(vec![Event::PointerMoved(pos2(x + 15.0 * i as f32, y))]);
        }
        h.release(pos2(x + 60.0, y));
        assert!(h.state.list_w > w0 + 40.0, "list grew: {} -> {}", w0, h.state.list_w);
        assert!(h.state.graph.left() > w0, "the graph moved right with it");
        // and it clamps
        h.state.list_w = 1000.0;
        h.frame(vec![]);
        assert!(h.state.list_w <= LIST_MAX);
        h.state.list_w = 1.0;
        h.frame(vec![]);
        assert!(h.state.list_w >= LIST_MIN);
    }

    #[test]
    fn right_click_on_empty_graph_adds_a_key() {
        let mut h = Harness::new();
        h.state.active = 0; // Position X — not animated yet
        h.frame(vec![]);
        assert!(!h.clip().x.is_animated());
        let plot = Rect::from_min_max(pos2(h.state.graph.left(), h.state.graph.top() + RULER_H), h.state.graph.max);
        let pos = pos2(plot.left() + plot.width() * 0.5, plot.center().y);
        h.frame(vec![Event::PointerMoved(pos)]);
        h.frame(vec![Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }]);
        let r = h.frame(vec![Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
        assert!(r.edited, "right-click on empty graph edits");
        let keys = &h.clip().x.keys;
        assert_eq!(keys.len(), 1, "one key added");
        assert!(keys[0].t > 1.5 && keys[0].t < 2.5, "at the clicked time: {}", keys[0].t);
        assert_eq!(h.undos, 1, "one undo for the gesture");
    }

    #[test]
    fn prop_plumbing_matches_labels() {
        let mut c = Clip::new(1, ClipKind::Video, "v", 0.0, 2.0);
        c.effects.push(crate::model::Effect::new(crate::model::EffectKind::Blur));
        assert_eq!(prop_count(&c), 7);
        assert_eq!(prop_label(&c, 0), "Position X");
        assert_eq!(prop_label(&c, 2), "Scale");
        assert_eq!(prop_label(&c, 5), "Speed");
        assert_eq!(prop_label(&c, 6), "Blur: Radius");
        c.scale.value = 2.5;
        assert_eq!(prop_ref(&c, 2).map(|a| a.value), Some(2.5));
        prop_mut(&mut c, 5).map(|a| a.value = 0.5);
        assert_eq!(c.speed_curve.value, 0.5);
        prop_mut(&mut c, 6).map(|a| a.value = 9.0);
        assert_eq!(c.effects[0].params[0].value, 9.0);
        let a = Clip::new(2, ClipKind::Audio, "a", 0.0, 2.0);
        assert_eq!(prop_count(&a), 3);
        assert_eq!(prop_label(&a, 0), "Volume");
        assert_eq!(prop_label(&a, 1), "Pan");
        assert_eq!(prop_label(&a, 2), "Speed");
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
    fn held_key_drag_does_not_run_away() {
        let mut h = Harness::new();
        let from = h.scale_key_pos(1); // key at t = 3, v = 3
        h.press(from);
        h.frame(vec![Event::PointerMoved(from + vec2(0.0, -12.0))]);
        h.frame(vec![]);
        let v = h.clip().scale.keys[1].v;
        for _ in 0..30 {
            h.frame(vec![]); // no pointer events at all — the drag is still held
        }
        let after = h.clip().scale.keys[1].v;
        assert!((after - v).abs() < 1e-6, "value drifted while the pointer was still: {v} -> {after}");
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
    fn rubber_band_selects_a_group_that_then_drags_together() {
        let mut h = Harness::new();
        let plot = Rect::from_min_max(pos2(h.state.graph.left(), h.state.graph.top() + RULER_H), h.state.graph.max);
        // band across the whole plot from an empty bottom-left corner
        let from = pos2(plot.left() + 4.0, plot.bottom() - 4.0);
        let to = pos2(plot.right() - 4.0, plot.top() + 4.0);
        h.press(from);
        for i in 1..=4 {
            h.frame(vec![Event::PointerMoved(from + (to - from) * (i as f32 / 4.0))]);
        }
        h.release(to);
        assert_eq!(h.state.selected, vec![(2, 0), (2, 1)], "the band caught both keys");
        assert_eq!(h.undos, 0, "selecting is not an edit");
        // now drag one of them: the other comes along, keeping the gap between them
        let k0 = h.scale_key_pos(0);
        h.press(k0);
        for i in 1..=4 {
            h.frame(vec![Event::PointerMoved(k0 + vec2(10.0 * i as f32, 0.0))]);
        }
        h.release(k0 + vec2(40.0, 0.0));
        let dt = 40.0 / h.state.pps as f64;
        let keys = &h.clip().scale.keys;
        assert_eq!(keys.len(), 2);
        assert!((keys[0].t - (1.0 + dt)).abs() < 0.05, "first key followed the drag: {}", keys[0].t);
        assert!((keys[1].t - (3.0 + dt)).abs() < 0.05, "second key came with it: {}", keys[1].t);
        assert!((keys[0].v - 1.0).abs() < 1e-6 && (keys[1].v - 3.0).abs() < 1e-6, "a flat drag keeps the values");
        assert_eq!(h.undos, 1, "one gesture = one undo");
    }

    #[test]
    fn copy_paste_lands_at_the_playhead_keeping_relative_times() {
        let mut h = Harness::new();
        let ctrl =
            |key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::CTRL };
        h.frame(vec![Event::PointerMoved(h.scale_key_pos(0))]); // the shortcuts only fire over the graph
        h.state.selected = vec![(2, 0), (2, 1)];
        h.frame(vec![ctrl(egui::Key::C)]);
        assert_eq!(h.state.clipboard.len(), 2);
        assert!((h.state.clipboard[0].1.t).abs() < 1e-9, "times are relative to the earliest key");
        h.playhead = 0.5;
        let r = h.frame(vec![ctrl(egui::Key::V)]);
        assert!(r.edited);
        let ts: Vec<f64> = h.clip().scale.keys.iter().map(|k| k.t).collect();
        assert_eq!(ts.len(), 4, "pasted alongside the originals: {ts:?}");
        assert!((ts[0] - 0.5).abs() < 1e-6 && (ts[2] - 2.5).abs() < 1e-6, "{ts:?}");
        assert_eq!(h.state.selected, vec![(2, 0), (2, 2)], "the pasted keys are the selection");
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn scroll_zooms_the_value_axis_and_fit_frames_the_keys() {
        let mut h = Harness::new();
        let span0 = h.state.y_hi - h.state.y_lo;
        let mid = pos2(h.state.graph.center().x, h.state.graph.center().y);
        h.frame(vec![Event::PointerMoved(mid)]);
        h.frame(vec![Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, 4.0),
            modifiers: Modifiers::NONE,
        }]);
        h.frame(vec![]); // y_lo/y_hi are published before the scroll is read, so look one frame later
        assert!(h.state.y_zoom > 1.0, "a plain wheel zooms the value axis: {}", h.state.y_zoom);
        assert!(h.state.y_hi - h.state.y_lo < span0, "the visible value range shrank");
        assert!(h.state.zoom == 0.0, "the time axis is untouched");
        // Fit: the keys span 1..3 s of a 4 s clip, so time zooms in and the value view resets
        let c = h.project.clip(7).unwrap().clone();
        fit_view(&mut h.state, &c, prop_count(&c));
        assert_eq!((h.state.y_zoom, h.state.y_pan), (1.0, 0.0));
        assert!((h.state.scroll_x - 0.9).abs() < 1e-6, "framed just before the first key: {}", h.state.scroll_x);
        h.frame(vec![]);
        let w = h.state.graph.width() as f64;
        assert!((h.state.pps as f64 * 2.2 - w).abs() < 1.0, "2.2 s fill the graph: {} px/s of {w}", h.state.pps);
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
