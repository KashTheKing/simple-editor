//! ---- ws:time-remap ----
//! Time remapping UI: the speed lane (the clip's speed over its own time, keys as diamonds) shared by the
//! Inspector's Time Remap section and the Animator's Speed view, plus the section's controls (add key,
//! hold, reverse, a segment's speed / length / easing). Every edit goes through `Project::time_remap_*`,
//! so a length change ripples the track, and is one undo step (a drag: one step for the whole drag).

use crate::model::*;
use crate::theme::Palette;
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui};

/// What a lane did this frame.
#[derive(Default)]
pub struct LaneResp {
    pub changed: bool,
    /// Clicked off the keys: seek here (clip-local seconds).
    pub seek: Option<f64>,
}

/// Picked segment of a clip's map (egui memory, per clip).
fn sel_id(id: Id) -> egui::Id {
    egui::Id::new(("time_remap_sel", id))
}
pub fn selected(ui: &Ui, id: Id) -> Option<usize> {
    ui.ctx().data(|d| d.get_temp(sel_id(id)))
}
fn set_selected(ui: &Ui, id: Id, i: Option<usize>) {
    ui.ctx().data_mut(|d| match i {
        Some(i) => d.insert_temp(sel_id(id), i),
        None => d.remove::<usize>(sel_id(id)),
    });
}

/// Speed on the lane's vertical axis: signed, log-like, 0 (a hold) in the middle band.
fn g(v: f64) -> f64 {
    v.signum() * (1.0 + v.abs()).log2()
}
const G_LO: f64 = -2.4; // about -4x
const G_HI: f64 = 3.2; // about 8x

/// The speed lane of clip `id` over `rect`. `x` maps clip-local seconds to screen x, `tx` back.
#[allow(clippy::too_many_arguments)]
pub fn lane(
    ui: &mut Ui,
    rect: Rect,
    x: &dyn Fn(f64) -> f32,
    tx: &dyn Fn(f32) -> f64,
    project: &mut Project,
    id: Id,
    pal: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> LaneResp {
    let mut out = LaneResp::default();
    let Some(c) = project.clip(id).cloned() else { return out };
    let Some(m) = c.time_map.clone() else { return out };
    let resp = ui.interact(rect, ui.id().with(("time_remap_lane", id)), Sense::click_and_drag());
    let plot = rect.shrink2(vec2(0.0, 6.0));
    let y = |v: f64| plot.bottom() - ((g(v).clamp(G_LO, G_HI) - G_LO) / (G_HI - G_LO)) as f32 * plot.height();
    let p = ui.painter_at(rect);
    p.rect_filled(Rect::from_min_max(pos2(x(0.0), rect.top()), pos2(x(c.duration), rect.bottom())), 2.0, pal.text.gamma_multiply(0.04));
    for (v, label) in [(-1.0, "-100%"), (0.0, "0%"), (1.0, "100%"), (2.0, "200%"), (4.0, "400%")] {
        let yy = y(v);
        let strong = v == 1.0 || v == 0.0;
        p.line_segment([pos2(rect.left(), yy), pos2(rect.right(), yy)], Stroke::new(1.0, pal.border.gamma_multiply(if strong { 1.0 } else { 0.4 })));
        p.text(pos2(rect.left() + 4.0, yy - 1.0), Align2::LEFT_BOTTOM, label, FontId::proportional(9.0), pal.text_dim);
    }
    let sel = selected(ui, id);
    // segments: held stretches and backward ones are tinted, the picked one outlined
    for (i, w) in m.keys.windows(2).enumerate() {
        let v = (w[1].s - w[0].s) / (w[1].t - w[0].t).max(1e-12);
        let r = Rect::from_min_max(pos2(x(w[0].t), plot.top()), pos2(x(w[1].t), plot.bottom()));
        if v.abs() < STILL {
            p.rect_filled(r, 0.0, Color32::from_rgb(0x3A, 0x8E, 0xE6).gamma_multiply(0.18));
        } else if v < 0.0 {
            p.rect_filled(r, 0.0, Color32::from_rgb(0xC0, 0x22, 0x3B).gamma_multiply(0.18));
        }
        if sel == Some(i) {
            p.rect_stroke(r, 0.0, Stroke::new(1.5, pal.accent), egui::StrokeKind::Inside);
        }
    }
    let n = ((x(c.duration) - x(0.0)).abs() as usize).clamp(16, 600);
    let pts: Vec<Pos2> = (0..=n).map(|i| c.duration * i as f64 / n as f64).map(|t| pos2(x(t), y(m.speed(t)))).collect();
    p.add(egui::Shape::line(pts, Stroke::new(2.0, pal.accent)));
    let mut hits = Vec::new();
    for (i, k) in m.keys.iter().enumerate() {
        let v = m.speed(k.t + if i + 1 == m.keys.len() { -1e-6 } else { 1e-6 });
        let kp = pos2(x(k.t), y(v));
        let d = 5.0;
        let shape = vec![kp + vec2(0.0, -d), kp + vec2(d, 0.0), kp + vec2(0.0, d), kp + vec2(-d, 0.0)];
        p.add(egui::Shape::convex_polygon(shape, pal.accent, Stroke::new(1.2, Color32::WHITE)));
        hits.push((Rect::from_center_size(kp, vec2(14.0, rect.height())), i));
    }
    let press = ui.input(|i| i.pointer.press_origin());
    let key_hit = press.and_then(|pp| hits.iter().find(|h| h.0.contains(pp)).map(|h| h.1));
    let seg_at = |px: f32| m.seg(tx(px));
    // a drag: a key moves in time (what comes after moves with it), a segment's body changes its speed
    let base_id = egui::Id::new(("time_remap_drag", id));
    if resp.drag_started() {
        undo(project);
        ui.ctx().data_mut(|d| d.insert_temp(base_id, m.clone()));
        if let (None, Some(pp)) = (key_hit, press) {
            set_selected(ui, id, Some(seg_at(pp.x)));
        }
    }
    if resp.dragged() {
        if let (Some(base), Some(pp), Some(cur)) = (ui.ctx().data(|d| d.get_temp::<TimeMap>(base_id)), press, resp.interact_pointer_pos()) {
            let edit = match key_hit {
                Some(i) if i > 0 => {
                    let t = base.keys[i].t + tx(cur.x) - tx(pp.x);
                    Some(Box::new(move |mm: &mut TimeMap| {
                        *mm = base.clone();
                        mm.move_key(i, t);
                    }) as Box<dyn Fn(&mut TimeMap)>)
                }
                Some(_) => None,
                None => {
                    let i = base.seg(tx(pp.x));
                    let (a, b) = (base.keys[i], base.keys[i + 1]);
                    let v0 = (b.s - a.s) / (b.t - a.t).max(1e-12);
                    let f = 2f64.powf(((pp.y - cur.y) / 40.0) as f64);
                    let v = (v0.abs() * f).clamp(0.05, 16.0);
                    (v0.abs() >= STILL).then(|| {
                        Box::new(move |mm: &mut TimeMap| {
                            *mm = base.clone();
                            mm.set_segment_speed(i, v);
                        }) as Box<dyn Fn(&mut TimeMap)>
                    })
                }
            };
            if let Some(e) = edit {
                out.changed |= project.time_remap_edit(id, e).is_ok();
            }
        }
    }
    if resp.clicked() {
        if let Some(pp) = press.filter(|_| key_hit.is_none()) {
            set_selected(ui, id, Some(seg_at(pp.x)));
            out.seek = Some(tx(pp.x).clamp(0.0, c.duration));
        }
    }
    if resp.double_clicked() {
        if let Some(pp) = press.filter(|_| key_hit.is_none()) {
            undo(project);
            out.changed |= project.time_remap_add_key(id, c.start + tx(pp.x).clamp(0.0, c.duration)).is_ok();
        }
    }
    resp.on_hover_text("Double-click: add a key. Drag a key: move it in time. Drag a segment up or down: its speed");
    out
}

/// The Inspector's Time Remap section for clip `id`. Returns true when the project changed.
pub fn section(ui: &mut Ui, project: &mut Project, id: Id, playhead: f64, pal: &Palette, undo: &mut dyn FnMut(&Project)) -> bool {
    let Some(c) = project.clip(id).cloned() else { return false };
    let mut changed = false;
    let Some(m) = c.time_map.clone() else {
        let r = ui.button("Turn On").on_hover_text(
            "Remap this clip's time with keys: speed ramps, freeze holds and backward stretches. Starts from what it plays now",
        );
        super::inspector::mark(ui, "time_remap_on", &r);
        if r.clicked() {
            undo(project);
            changed |= project.time_remap_enable(id);
        }
        return changed;
    };
    // the lane, over the whole clip
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 72.0), Sense::hover());
    let (l, w, d) = (rect.left(), rect.width(), c.duration.max(1e-3));
    let x = move |t: f64| l + (t / d) as f32 * w;
    let tx = move |px: f32| ((px - l) / w.max(1.0)) as f64 * d;
    let r = lane(ui, rect, &x, &tx, project, id, pal, undo);
    changed |= r.changed;
    let ph = playhead.clamp(c.start, c.end());
    // playhead marker on the lane
    if c.contains(playhead) {
        let px = x(c.local(playhead));
        ui.painter().line_segment([pos2(px, rect.top()), pos2(px, rect.bottom())], Stroke::new(1.0, pal.text));
    }
    let hold_id = egui::Id::new("time_remap_hold_len");
    let mut hold = ui.ctx().data(|dd| dd.get_temp::<f64>(hold_id)).unwrap_or(1.0);
    ui.horizontal(|ui| {
        let r = ui.button("◆").on_hover_text("Add a key at the playhead (changes nothing until you move it)");
        super::inspector::mark(ui, "time_remap_key", &r);
        if r.clicked() {
            undo(project);
            changed |= project.time_remap_add_key(id, ph).is_ok();
        }
        let r = ui.button("Hold").on_hover_text("Freeze the frame at the playhead for the length beside it; the rest of the clip and its track move later");
        super::inspector::mark(ui, "time_remap_hold", &r);
        if r.clicked() {
            undo(project);
            changed |= project.time_remap_hold(id, ph, hold).is_ok();
        }
        if ui.add(egui::DragValue::new(&mut hold).range(0.05..=60.0).speed(0.02).suffix(" s")).on_hover_text("Hold length").changed() {
            ui.ctx().data_mut(|dd| dd.insert_temp(hold_id, hold));
        }
        let r = ui.button("Reverse").on_hover_text(
            "Play the stretch around the playhead (between its two keys) backwards right after itself, then forwards again",
        );
        super::inspector::mark(ui, "time_remap_reverse", &r);
        if r.clicked() {
            undo(project);
            changed |= project.time_remap_reverse(id, ph).is_ok();
        }
    });
    // the picked segment (or the one under the playhead)
    let m = project.clip(id).and_then(|c| c.time_map.clone()).unwrap_or(m);
    let i = selected(ui, id).filter(|&i| i + 1 < m.keys.len()).unwrap_or_else(|| m.seg(c.local(ph)));
    let (a, b) = (m.keys[i], m.keys[i + 1]);
    let v = (b.s - a.s) / (b.t - a.t).max(1e-12);
    egui::Grid::new(("time_remap_seg", id)).num_columns(2).spacing(vec2(8.0, 4.0)).show(ui, |ui| {
        ui.label("Segment").on_hover_text("The stretch between two keys: click one on the lane to pick it");
        ui.label(format!("{} of {}", i + 1, m.keys.len() - 1));
        ui.end_row();
        ui.label("Speed");
        if v.abs() < STILL {
            ui.label("Hold").on_hover_text("A freeze: the same frame for the whole segment");
        } else {
            let mut pct = v.abs() * 100.0;
            let r = ui.add(egui::DragValue::new(&mut pct).range(5.0..=1600.0).speed(1.0).suffix(" %")).on_hover_text(if v < 0.0 {
                "Backwards. Its length follows the speed; the rest of the track moves with it"
            } else {
                "Its length follows the speed; the rest of the track moves with it"
            });
            super::inspector::mark(ui, "time_remap_speed", &r);
            if r.drag_started() || (r.changed() && !r.dragged()) {
                undo(project);
            }
            if r.changed() {
                changed |= project.time_remap_edit(id, |mm| {
                    mm.set_segment_speed(i, pct / 100.0);
                }).is_ok();
            }
        }
        ui.end_row();
        ui.label("Length");
        let mut len = b.t - a.t;
        let r = ui.add(egui::DragValue::new(&mut len).range(MIN_SEG..=3600.0).speed(0.01).max_decimals(3).suffix(" s"));
        if r.drag_started() || (r.changed() && !r.dragged()) {
            undo(project);
        }
        if r.changed() {
            changed |= project.time_remap_edit(id, |mm| {
                mm.set_segment_len(i, len);
            }).is_ok();
        }
        ui.end_row();
        ui.label("Smooth");
        let mut sm = a.smooth;
        if ui.checkbox(&mut sm, "").on_hover_text("Ease the speed through this segment's keys instead of one constant speed").changed() {
            undo(project);
            changed |= project.time_remap_edit(id, |mm| mm.set_smooth(i, sm)).is_ok();
        }
        ui.end_row();
    });
    ui.horizontal(|ui| {
        let inner = i > 0 && i + 1 < m.keys.len();
        let r = ui.add_enabled(inner, egui::Button::new("Remove Key")).on_hover_text("Remove the key at the start of the picked segment");
        if r.clicked() {
            undo(project);
            changed |= project.time_remap_edit(id, |mm| {
                mm.remove_key(i);
            }).is_ok();
        }
        let r = ui.button("Turn Off").on_hover_text("Back to one constant speed over the same source and length");
        super::inspector::mark(ui, "time_remap_off", &r);
        if r.clicked() {
            undo(project);
            changed |= project.time_remap_disable(id);
        }
    });
    changed
}
