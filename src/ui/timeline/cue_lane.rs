//! Subtitle-cue lane: the burned-in cues drawn as small clips above the video rows.
use super::*;

/// Draws and handles interaction for the subtitle-cue lane. Mutates `c.project`/`state`/`out`
/// directly rather than returning an `Act` — none of its gestures go through the deferred-apply path.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    ui: &mut egui::Ui,
    c: &mut TimelineCtx<'_>,
    state: &mut TimelineState,
    out: &mut TimelineResponse,
    painter: &egui::Painter,
    sub_h: f32,
    subs_lane: Rect,
    pal: &Palette,
    small: &FontId,
    thin: Stroke,
    full: Rect,
    id: egui::Id,
    mods: egui::Modifiers,
    pointer: Option<Pos2>,
    primary_down: bool,
) {
    // ---- subtitle lane: the burned-in cues as little clips above the video rows ----
    state.sub_sel.retain(|id| c.project.subtitles.iter().any(|q| q.id == *id));
    if sub_h > 0.0 {
        enum SubAct {
            Convert(Vec<Id>),
            Delete(Vec<Id>),
            Split(Id),
            Range,
            Clear,
        }
        let mut sub_act: Option<SubAct> = None;
        let inout = match (c.project.in_point, c.project.out_point) {
            (Some(a), Some(b)) if b > a => Some((a, b)),
            _ => None,
        };
        // lane background first so the cue rects (registered later) win hit-testing
        let lane_resp = ui.interact(subs_lane, id.with("subs_lane"), Sense::click_and_drag());
        let sp = painter.with_clip_rect(subs_lane);
        sp.rect_filled(subs_lane, 0, pal.header.gamma_multiply(0.5));
        sp.hline(subs_lane.x_range(), subs_lane.bottom() - 0.5, thin);
        painter.text(
            pos2(full.left() + 6.0, subs_lane.center().y),
            Align2::LEFT_CENTER,
            "Subtitles",
            small.clone(),
            pal.text_dim,
        );
        // drag on empty lane = band-select over time (Shift adds); click = deselect
        if lane_resp.drag_started_by(egui::PointerButton::Primary) {
            if let Some(o) = lane_resp.interact_pointer_pos() {
                state.sub_band = Some((state.time_at(o.x), mods.shift));
            }
        }
        if lane_resp.clicked() {
            state.sub_sel.clear();
        }
        let band_range = state.sub_band.and_then(|(t0, _)| {
            let p = pointer?;
            let t1 = state.time_at(p.x);
            Some((t0.min(t1), t0.max(t1)))
        });
        if let Some((a, b)) = band_range {
            sp.rect_filled(
                Rect::from_x_y_ranges(state.x_at(a)..=state.x_at(b), subs_lane.y_range()),
                0,
                pal.selection.gamma_multiply(0.2),
            );
        }
        if state.sub_band.is_some() && !primary_down {
            let (_, add) = state.sub_band.take().expect("checked above");
            if let Some((a, b)) = band_range {
                let hit: Vec<Id> =
                    c.project.subtitles.iter().filter(|q| q.end > a && q.start < b).map(|q| q.id).collect();
                if !add {
                    state.sub_sel.clear();
                }
                for h in hit {
                    if !state.sub_sel.contains(&h) {
                        state.sub_sel.push(h);
                    }
                }
            }
        }
        for cue in &c.project.subtitles {
            let (xa, xb) = (state.x_at(cue.start), state.x_at(cue.end));
            if xb < subs_lane.left() || xa > subs_lane.right() {
                continue;
            }
            let rect =
                Rect::from_min_max(pos2(xa, subs_lane.top() + 2.0), pos2(xb.max(xa + 2.0), subs_lane.bottom() - 2.0));
            let r = ui.interact(rect.intersect(subs_lane), id.with(("sub", cue.id)), Sense::click());
            let sel = state.sub_sel.contains(&cue.id);
            let a = if sel {
                0.7
            } else if r.hovered() {
                0.55
            } else {
                0.3
            };
            sp.rect_filled(rect, 3.0, pal.selection.gamma_multiply(a));
            sp.rect_stroke(rect, 3.0, if sel { Stroke::new(1.5, pal.accent) } else { thin }, StrokeKind::Inside);
            sp.with_clip_rect(rect.intersect(subs_lane)).text(
                pos2(rect.left() + 3.0, rect.center().y),
                Align2::LEFT_CENTER,
                &cue.text,
                small.clone(),
                pal.text,
            );
            if r.clicked() {
                if mods.ctrl || mods.shift {
                    // Ctrl/Shift+click toggles membership, like clips
                    if sel {
                        state.sub_sel.retain(|q| *q != cue.id);
                    } else {
                        state.sub_sel.push(cue.id);
                    }
                } else {
                    state.sub_sel = vec![cue.id];
                    *c.playhead = cue.start;
                    out.seeked = true;
                }
            }
            // trim handles on both edges, like a clip's
            for right in [false, true] {
                let er = Rect::from_center_size(
                    pos2(if right { xb } else { xa }, rect.center().y),
                    vec2(6.0, rect.height()),
                );
                let e = ui
                    .interact(er.intersect(subs_lane), id.with(("sub_e", cue.id, right)), Sense::drag())
                    .on_hover_cursor(CursorIcon::ResizeHorizontal);
                if e.drag_started_by(egui::PointerButton::Primary) {
                    (c.undo)(c.project);
                    state.sub_trim = Some((cue.id, right));
                }
            }
            // right-click outside the selection retargets it, like every explorer
            if r.secondary_clicked() && !sel {
                state.sub_sel = vec![cue.id];
            }
            let targets: Vec<Id> = if sel && state.sub_sel.len() > 1 { state.sub_sel.clone() } else { vec![cue.id] };
            let n = targets.len();
            let plural = |what: &str| if n > 1 { format!("{what} ({n})") } else { what.to_string() };
            let cid = cue.id;
            let ph_in = *c.playhead > cue.start + 0.05 && *c.playhead < cue.end - 0.05;
            r.on_hover_text(&cue.text).context_menu(|ui| {
                if ui.add_enabled(ph_in, egui::Button::new("Split at Playhead")).on_hover_text("Ctrl+B").clicked() {
                    sub_act = Some(SubAct::Split(cid));
                    ui.close();
                }
                if ui.button(plural("Convert to Text Clip")).clicked() {
                    sub_act = Some(SubAct::Convert(targets.clone()));
                    ui.close();
                }
                if ui.button(plural("Delete Cue")).clicked() {
                    sub_act = Some(SubAct::Delete(targets.clone()));
                    ui.close();
                }
                ui.separator();
                if ui.add_enabled(inout.is_some(), egui::Button::new("Delete Cues in In/Out Range")).clicked() {
                    sub_act = Some(SubAct::Range);
                    ui.close();
                }
                if ui.button("Clear All Subtitles").clicked() {
                    sub_act = Some(SubAct::Clear);
                    ui.close();
                }
            });
        }
        // active edge drag: follow the pointer while held, sort on release
        if let Some((tid, right)) = state.sub_trim {
            if primary_down {
                if let (Some(p), Some(q)) = (pointer, c.project.subtitles.iter_mut().find(|q| q.id == tid)) {
                    let t = state.time_at(p.x).max(0.0);
                    if right {
                        q.end = t.max(q.start + 0.1);
                    } else {
                        q.start = t.clamp(0.0, q.end - 0.1);
                    }
                    out.edited = true;
                }
            } else {
                state.sub_trim = None;
                c.project.sort_cues();
                out.edited = true;
            }
        }
        if let Some(a) = sub_act {
            (c.undo)(c.project);
            match a {
                SubAct::Convert(ids) => {
                    c.project.cues_to_text_clips(Some(&ids));
                }
                SubAct::Split(id) => {
                    c.project.split_cue(id, *c.playhead);
                }
                SubAct::Delete(ids) => c.project.subtitles.retain(|q| !ids.contains(&q.id)),
                SubAct::Range => {
                    if let Some((a, b)) = inout {
                        c.project.subtitles.retain(|q| q.end <= a || q.start >= b);
                    }
                }
                SubAct::Clear => c.project.subtitles.clear(),
            }
            state.sub_sel.clear();
            out.edited = true;
        }
    }
}
