//! ---- ws:zones-markers ----
//! The effect-zone lane above the tracks: each zone of the open timeline as a bar (`model::zone`), the
//! Animator's span bars over the main timeline. Click picks it (the Inspector shows its card,
//! `ui::zones_ui`), drag moves it, its ends resize it, all snapped like a cue and one undo per gesture.
//! Overlapping zones stack in rows. The lane only takes room while the timeline has a zone.
use super::*;
use crate::model::EffectZone;
use crate::ui::{menu, timecode, zones_ui};
use std::cell::RefCell;

/// Height of one row of bars.
pub(super) const ROW_H: f32 = 18.0;

/// What a drag does: move (grab = press time minus the zone's start) or one end.
#[derive(Clone, Copy)]
enum Op {
    Move { grab: f64 },
    End { right: bool },
}

struct Drag {
    id: Id,
    op: Op,
    before: Project,
    changed: bool,
}

thread_local! {
    static DRAG: RefCell<Option<Drag>> = const { RefCell::new(None) };
}

/// Each zone's row: greedy, in start order, so overlapping zones never share one.
fn rows(zones: &[&EffectZone]) -> Vec<(Id, usize)> {
    let mut order: Vec<&&EffectZone> = zones.iter().collect();
    order.sort_by(|a, b| a.t.total_cmp(&b.t));
    let mut ends: Vec<f64> = Vec::new();
    let mut out = Vec::new();
    for z in order {
        let r = ends.iter().position(|&e| e <= z.t + 1e-9).unwrap_or(ends.len());
        if r == ends.len() {
            ends.push(z.end());
        } else {
            ends[r] = z.end();
        }
        out.push((z.id, r));
    }
    out
}

/// The lane's height: a row per stack of overlapping zones, none without a zone.
pub(super) fn height(p: &Project) -> f32 {
    let z: Vec<&EffectZone> = p.zones_here().collect();
    if z.is_empty() {
        return 0.0;
    }
    (rows(&z).iter().map(|(_, r)| r + 1).max().unwrap_or(1)) as f32 * ROW_H + 2.0
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    ui: &mut egui::Ui,
    c: &mut TimelineCtx<'_>,
    state: &mut TimelineState,
    out: &mut TimelineResponse,
    painter: &egui::Painter,
    lane: Rect,
    pal: &Palette,
    small: &FontId,
    thin: Stroke,
    full: Rect,
    id: egui::Id,
    pointer: Option<Pos2>,
    primary_down: bool,
) {
    if lane.height() <= 0.0 {
        return;
    }
    let zones: Vec<EffectZone> = c.project.zones_here().cloned().collect();
    let picked = zones_ui::selected();
    let lane_resp = ui.interact(lane, id.with("zone_lane"), Sense::click());
    let sp = painter.with_clip_rect(lane);
    sp.rect_filled(lane, 0, pal.header.gamma_multiply(0.5));
    sp.hline(lane.x_range(), lane.bottom() - 0.5, thin);
    painter.text(pos2(full.left() + 6.0, lane.top() + ROW_H * 0.5), Align2::LEFT_CENTER, "Zones", small.clone(), pal.text_dim);
    if lane_resp.clicked() {
        zones_ui::select(None);
    }
    let mut add: Option<Option<Id>> = None;
    let mut del: Option<Id> = None;
    let mut retarget: Option<(Id, Option<Id>)> = None;
    let mut toggle: Option<Id> = None;
    menu::context(&lane_resp, |ui| {
        if menu::row(ui, None, "Add Effect Zone", "").on_hover_text("Over In/Out, the selected clips or 5 s from the playhead").clicked() {
            add = Some(None);
        }
    });

    let refs: Vec<&EffectZone> = zones.iter().collect();
    let row_of = rows(&refs);
    let tracks: Vec<(Id, String)> =
        c.project.tracks.iter().filter(|t| t.kind == TrackKind::Video).map(|t| (t.id, t.name.clone())).collect();
    for z in &zones {
        let (xa, xb) = (state.x_at(z.t), state.x_at(z.end()));
        if xb < lane.left() || xa > lane.right() {
            continue;
        }
        let row = row_of.iter().find(|(i, _)| *i == z.id).map(|(_, r)| *r).unwrap_or(0) as f32;
        let top = lane.top() + 1.0 + row * ROW_H;
        let rect = Rect::from_min_max(pos2(xa, top + 1.0), pos2(xb.max(xa + 4.0), top + ROW_H - 1.0));
        let r = ui.interact(rect.intersect(lane), id.with(("zone", z.id)), Sense::click_and_drag());
        let sel = picked == Some(z.id);
        // teal over the whole timeline, violet on one track
        let base = if z.track.is_none() { Color32::from_rgb(38, 166, 154) } else { Color32::from_rgb(149, 117, 205) };
        let a = if !z.enabled {
            0.18
        } else if sel {
            0.85
        } else if r.hovered() {
            0.65
        } else {
            0.45
        };
        sp.rect_filled(rect, 3.0, base.gamma_multiply(a));
        sp.rect_stroke(rect, 3.0, if sel { Stroke::new(1.5, pal.accent) } else { thin }, StrokeKind::Inside);
        let track_name = z.track.and_then(|t| tracks.iter().find(|(i, _)| *i == t)).map(|(_, n)| n.as_str());
        let text = match track_name {
            Some(n) => format!("{} · {n}", z.label()),
            None => z.label(),
        };
        sp.with_clip_rect(rect.intersect(lane)).text(
            pos2(rect.left() + 4.0, rect.center().y),
            Align2::LEFT_CENTER,
            &text,
            small.clone(),
            pal.text,
        );
        if r.clicked() || r.secondary_clicked() {
            zones_ui::select(Some(z.id));
            c.selection.clear();
        }
        if r.double_clicked() {
            *c.playhead = z.t;
            out.seeked = true;
        }
        let busy = DRAG.with(|d| d.borrow().is_some());
        if r.drag_started_by(egui::PointerButton::Primary) && !busy {
            let press_x = ui.input(|i| i.pointer.press_origin()).map(|p| p.x);
            let grab = press_x.map(|x| state.time_at(x)).unwrap_or(z.t) - z.t;
            zones_ui::select(Some(z.id));
            DRAG.with(|d| *d.borrow_mut() = Some(Drag { id: z.id, op: Op::Move { grab }, before: c.project.clone(), changed: false }));
        }
        for right in [false, true] {
            let er = Rect::from_center_size(pos2(if right { xb } else { xa }, rect.center().y), vec2(7.0, rect.height()));
            let e = ui
                .interact(er.intersect(lane), id.with(("zone_e", z.id, right)), Sense::drag())
                .on_hover_cursor(CursorIcon::ResizeHorizontal);
            if e.drag_started_by(egui::PointerButton::Primary) && !DRAG.with(|d| d.borrow().is_some()) {
                zones_ui::select(Some(z.id));
                DRAG.with(|d| *d.borrow_mut() = Some(Drag { id: z.id, op: Op::End { right }, before: c.project.clone(), changed: false }));
            }
        }
        let zid = z.id;
        let (on, cur) = (z.enabled, z.track);
        menu::context(&r.on_hover_text(format!("{text}\n{} - {}", timecode(z.t, c.project.fps), timecode(z.end(), c.project.fps))), |ui| {
            menu::sub(ui, None, "Applies to", |ui| {
                if menu::check(ui, cur.is_none(), "Whole timeline", "").clicked() {
                    retarget = Some((zid, None));
                }
                for (tid, name) in &tracks {
                    if menu::check(ui, cur == Some(*tid), name, "").clicked() {
                        retarget = Some((zid, Some(*tid)));
                    }
                }
            });
            if menu::check(ui, on, "On", "").clicked() {
                toggle = Some(zid);
            }
            ui.separator();
            if menu::row(ui, Some(Glyph::Cross), "Delete Zone", "").clicked() {
                del = Some(zid);
            }
        });
    }

    // the drag in progress: snapped every frame, one undo on release if anything moved
    if let Some(mut drag) = DRAG.with(|d| d.borrow_mut().take()) {
        if primary_down {
            if let Some(p) = pointer {
                let raw_t = state.time_at(p.x).max(0.0);
                let thr = snap_thr(state.zoom, c.project.fps);
                let hit =
                    c.snap.then(|| snap::target(c.project, raw_t, thr, *c.playhead, &[], &[], None, c.snap_markers)).flatten();
                let t = hit.map(|(x, _)| x).unwrap_or(raw_t);
                if let Some(z) = c.project.zone_mut(drag.id) {
                    let (old_t, old_d) = (z.t, z.dur);
                    let min = crate::model::ops::zones::MIN_ZONE;
                    match drag.op {
                        Op::Move { grab } => z.t = (t - grab).max(0.0),
                        Op::End { right: true } => z.dur = (t - z.t).max(min),
                        Op::End { right: false } => {
                            let end = z.end();
                            z.t = t.clamp(0.0, end - min);
                            z.dur = end - z.t;
                        }
                    }
                    if (z.t - old_t).abs() > 1e-9 || (z.dur - old_d).abs() > 1e-9 {
                        drag.changed = true;
                        out.edited = true;
                    }
                }
            }
            DRAG.with(|d| *d.borrow_mut() = Some(drag));
        } else if drag.changed {
            (c.undo)(&drag.before, "");
            out.edited = true;
        }
    }

    if add.is_some() || del.is_some() || retarget.is_some() || toggle.is_some() {
        (c.undo)(c.project, "");
        if let Some(track) = add {
            let (t, d) = c.project.zone_range(*c.playhead, c.selection, track);
            let z = c.project.add_zone(t, d, track, Vec::new());
            zones_ui::select(Some(z));
            c.selection.clear();
        }
        if let Some(z) = del {
            c.project.remove_zone(z);
        }
        if let Some((z, t)) = retarget {
            if let Some(z) = c.project.zone_mut(z) {
                z.track = t;
            }
        }
        if let Some(z) = toggle.and_then(|z| c.project.zone_mut(z)) {
            z.enabled = !z.enabled;
        }
        out.edited = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_zones_stack_in_rows() {
        let mut p = Project::new();
        p.add_zone(0.0, 10.0, None, vec![]);
        p.add_zone(5.0, 10.0, None, vec![]);
        p.add_zone(10.0, 2.0, None, vec![]);
        let z: Vec<&EffectZone> = p.zones_here().collect();
        let r: Vec<usize> = rows(&z).into_iter().map(|(_, r)| r).collect();
        assert_eq!(r, vec![0, 1, 0], "the third starts where the first ends: back on row 0");
        assert_eq!(height(&p), 2.0 * ROW_H + 2.0);
        assert_eq!(height(&Project::new()), 0.0, "no zone, no lane");
    }
}
