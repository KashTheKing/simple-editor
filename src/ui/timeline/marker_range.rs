//! ---- ws:zones-markers ----
//! Two marker gestures on the ruler:
//! - a range marker's ends: drag the start or the end of its bar (the flag still moves the whole marker);
//! - a row dragged out of the Markers panel (`markers_ui::MarkerDragId`) and dropped over the ruler or the
//!   tracks places that marker there (a clip marker stays on its clip, clamped into it).
//! Both snap like the other ruler gestures and are one undo step.
use super::*;
use crate::ui::markers_ui::MarkerDragId;
use std::cell::RefCell;

struct EndDrag {
    id: Id,
    right: bool,
    before: Project,
    changed: bool,
}

thread_local! {
    static DRAG: RefCell<Option<EndDrag>> = const { RefCell::new(None) };
}

/// The shortest a range marker gets by dragging an end (a frame-ish).
const MIN_RANGE: f64 = 0.04;

#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    ui: &mut egui::Ui,
    c: &mut TimelineCtx<'_>,
    state: &TimelineState,
    out: &mut TimelineResponse,
    ruler: Rect,
    lanes: Rect,
    id: egui::Id,
    pointer: Option<Pos2>,
    primary_down: bool,
) {
    let snapped = |c: &TimelineCtx<'_>, x: f32| {
        let raw = state.time_at(x).max(0.0);
        let thr = snap_thr(state.zoom, c.project.fps);
        c.snap
            .then(|| snap::target(c.project, raw, thr, *c.playhead, &[], &[], None, c.snap_markers))
            .flatten()
            .map(|(t, _)| t)
            .unwrap_or(raw)
    };

    // ---- the ends of range markers ----
    let ranges: Vec<(Id, f64, f64)> = c
        .project
        .markers
        .iter()
        .filter(|m| m.sequence == c.project.editing && m.duration > 0.0)
        .map(|m| (m.id, m.t, m.t + m.duration))
        .collect();
    for (mid, a, b) in ranges {
        for (right, t) in [(false, a), (true, b)] {
            let x = state.x_at(t);
            if x < ruler.left() - 4.0 || x > ruler.right() + 4.0 {
                continue;
            }
            let r = Rect::from_center_size(pos2(x, ruler.center().y), vec2(6.0, ruler.height())).intersect(ruler);
            if !r.is_positive() {
                continue;
            }
            let h = ui
                .interact(r, id.with(("mk_end", mid, right)), Sense::drag())
                .on_hover_cursor(CursorIcon::ResizeHorizontal)
                .on_hover_text(if right { "Drag to move the end" } else { "Drag to move the start" });
            if h.drag_started_by(egui::PointerButton::Primary) && DRAG.with(|d| d.borrow().is_none()) {
                DRAG.with(|d| *d.borrow_mut() = Some(EndDrag { id: mid, right, before: c.project.clone(), changed: false }));
            }
        }
    }
    if let Some(mut d) = DRAG.with(|d| d.borrow_mut().take()) {
        if primary_down {
            if let Some(p) = pointer {
                let t = snapped(c, p.x);
                if let Some(m) = c.project.markers.iter_mut().find(|m| m.id == d.id) {
                    let (t0, d0) = (m.t, m.duration);
                    if d.right {
                        m.duration = (t - m.t).max(MIN_RANGE);
                    } else {
                        let end = m.t + m.duration;
                        m.t = t.clamp(0.0, end - MIN_RANGE);
                        m.duration = end - m.t;
                    }
                    if (m.t - t0).abs() > 1e-9 || (m.duration - d0).abs() > 1e-9 {
                        d.changed = true;
                        out.edited = true;
                    }
                }
            }
            DRAG.with(|x| *x.borrow_mut() = Some(d));
        } else if d.changed {
            (c.undo)(&d.before, "");
            c.project.sort_markers();
            out.edited = true;
        }
    }

    // ---- a row dragged in from the Markers panel ----
    let Some(payload) = egui::DragAndDrop::payload::<MarkerDragId>(ui.ctx()) else { return };
    let Some(p) = pointer.filter(|p| ruler.contains(*p) || lanes.contains(*p)) else { return };
    let t = snapped(c, p.x);
    let x = state.x_at(t);
    ui.painter().vline(x, ruler.top()..=lanes.bottom(), Stroke::new(1.5, c.palette.accent));
    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    if !ui.input(|i| i.pointer.any_released()) {
        return;
    }
    egui::DragAndDrop::clear_payload(ui.ctx());
    let mid = payload.0;
    (c.undo)(c.project, "");
    let owner: Option<(f64, f64)> =
        c.project.all_clips().find(|(_, cl)| cl.markers.iter().any(|m| m.id == mid)).map(|(_, cl)| (cl.start, cl.duration));
    if let Some(m) = c.project.markers.iter_mut().find(|m| m.id == mid) {
        m.t = t;
        c.project.sort_markers();
    } else if let Some((start, dur)) = owner {
        if let Some(m) = c.project.marker_mut(mid) {
            m.t = (t - start).clamp(0.0, dur);
        }
    }
    out.edited = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_projects_load_with_zero_length_markers() {
        let mut v: serde_json::Value = serde_json::from_str(&Project::new().to_json()).unwrap();
        v["markers"] = serde_json::json!([{"id": 5, "t": 2.0, "name": "old"}]);
        let p = Project::from_json(&v.to_string()).unwrap();
        assert_eq!((p.markers[0].t, p.markers[0].duration), (2.0, 0.0), "no duration in the file = a point marker");
    }
}
