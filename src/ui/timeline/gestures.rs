//! Gesture start + live-drag handling, extracted from `show()`.
//!
//! ws:timeline-trim-gestures — every body/edge press is routed through `arm()` (the frozen modifier
//! table, `arm.rs`) and the returned `GestureKind` picks the drag: Move/MagneticMove, Slip, Slide,
//! Segment on a body; Trim, RippleTrim, Roll, RateStretch, MultiRippleTrim on an edge. Roll/Slip/Slide
//! edit the live project per frame (they touch at most three clips); RippleTrim/Segment only carry a
//! delta, `paint_ghost` draws the outcome, and `release` makes the single model call.
use super::*;

/// Does this gesture keep the pre-existing press-time selection step ("an unselected clip becomes the
/// selection, Ctrl adds it")? False for the kinds whose Ctrl bit means something else now — Slide
/// (Ctrl+Alt), Segment (Ctrl+Shift), RippleTrim (Ctrl+edge), MultiRippleTrim (Ctrl+Alt+edge) — so
/// arming them on an unselected clip no longer also replays the old plain-Ctrl toggle as a side effect.
pub(super) fn arm_selects(kind: GestureKind) -> bool {
    !matches!(kind, GestureKind::Slide | GestureKind::Segment | GestureKind::RippleTrim | GestureKind::MultiRippleTrim)
}

/// ws:pro-timeline — asymmetric multi-roller trim: `ids`/`start` is the pressed clip (+ same-edge
/// linked clips, unchanged from before this workstream); `rollers` is `TimelineState.rollers`, seams
/// Shift-clicked onto the roller set before the drag started (possibly other clips, other tracks,
/// either side). Each roller keeps its OWN press-time edge (`edge0`), so a live drag applies the same
/// delta to every roller from its own edge rather than assuming one shared edge value.
fn build_trim(p: &Project, ids: &[Id], start: bool, rollers: &[(Id, bool)]) -> Gesture {
    let mut set: Vec<(Id, bool)> = ids.iter().map(|&id| (id, start)).collect();
    for &(rid, rstart) in rollers {
        if !set.iter().any(|&(id, s)| id == rid && s == rstart) {
            set.push((rid, rstart));
        }
    }
    let mut fids = Vec::with_capacity(set.len());
    let mut edge0 = Vec::with_capacity(set.len());
    for (id, s) in set {
        if let Some(cl) = p.clip(id) {
            fids.push((id, s));
            edge0.push(if s { cl.start } else { cl.end() });
        }
    }
    Gesture::Trim { ids: fids, edge0, changed: false }
}

/// The empty-lane gap under `(track, t)`: `[previous clip's end (or 0), next clip's start)`. None when
/// `t` is inside a clip, past the last clip, or the track doesn't exist.
pub(super) fn gap_at(p: &Project, ti: usize, t: f64) -> Option<(f64, f64)> {
    let tr = p.tracks.get(ti)?;
    if tr.clips.iter().any(|c| c.contains(t)) {
        return None;
    }
    let a = tr.clips.iter().filter(|c| c.end() <= t + ABUT_EPS).map(|c| c.end()).fold(0.0_f64, f64::max);
    let b = tr.clips.iter().filter(|c| c.start >= t - ABUT_EPS).map(|c| c.start).fold(f64::INFINITY, f64::min);
    (b.is_finite() && b > a + ABUT_EPS).then_some((a, b))
}

/// `Project::delete_clips`, except a clip on a magnetic track takes its gap with it — that track only
/// (`ripple_delete_range` scoped to it), the frozen table's "Track.magnetic → Delete closes the gap on
/// that track only". Everything else keeps the plain leave-a-gap (or caller-chosen ripple) delete.
pub(crate) fn delete_clips_magnetic(p: &mut Project, ids: &[Id], ripple: bool) {
    let mut mag: Vec<(usize, f64, f64)> = Vec::new();
    let mut rest: Vec<Id> = Vec::new();
    for &id in ids {
        match p.find(id) {
            Some((ti, ci)) if p.tracks[ti].magnetic => {
                let cl = &p.tracks[ti].clips[ci];
                mag.push((ti, cl.start, cl.end()));
            }
            _ => rest.push(id),
        }
    }
    mag.sort_by(|x, y| y.1.total_cmp(&x.1)); // right to left, so the ranges still to go stay valid
    for (ti, a, b) in mag {
        p.ripple_delete_range(a, b, &[ti]);
    }
    p.delete_clips(&rest, ripple);
}

/// Premiere-style insert move, applied once on release: each moved clip is lifted out of its own
/// track (the gap closes behind it — `ripple_delete_range` on that track only), `ripple_open` makes
/// room `dt` later and the clip goes back, id intact. False (project untouched) on a locked track;
/// the caller restores `before` on a false from the middle of the loop.
/// ponytail: only the moved clips' own tracks ripple, not every ripple track — B-roll elsewhere stays
/// put; union in `ripple_tracks()` once a range-scoped `close_gap` is public.
pub(super) fn segment_move(p: &mut Project, ids: &[Id], spans: &[(usize, f64, f64)], dt: f64) -> bool {
    if dt.abs() < 1e-9 || spans.iter().any(|&(ti, _, _)| p.locked_of(ti)) {
        return false;
    }
    for (&id, &(ti, s, e)) in ids.iter().zip(spans) {
        let Some(mut cl) = p.clip(id).cloned() else { return false };
        let len = e - s;
        if !p.ripple_delete_range(s, e, &[ti]).contains(&id) {
            return false;
        }
        // target in the pre-move layout; past its own old end, closing the gap has already pulled that
        // content back by `len`
        let target = s + dt;
        let dest = if target >= e - ABUT_EPS { target - len } else { target }.max(0.0);
        p.ripple_open(dest, len, &[ti]);
        cl.start = dest;
        p.tracks[ti].clips.push(cl);
        p.tracks[ti].sort();
    }
    p.tidy();
    true
}

/// The one model call of a release-applied gesture (RippleTrim / Segment / a blocked magnetic Move).
/// Returns whether the project changed; a refused ripple/segment puts `d.before` back so a half-applied
/// linked trim never survives.
pub(super) fn release(p: &mut Project, d: &Drag, edited: bool) -> bool {
    match &d.g {
        Gesture::RippleTrim { ids, start, edge0, dt, multi } => {
            let ok = if *multi {
                // ponytail: trim_edges has no downstream shift (all-or-nothing multi-trim); pro-timeline's
                // asymmetric edit-point set composes ripple_trim per edge when a true ripple is wanted
                let edges: Vec<(Id, bool)> = ids.iter().map(|&id| (id, *start)).collect();
                p.trim_edges(&edges, *dt, false)
            } else {
                // the pressed clip ripples the ripple tracks; linked clips sharing the edge follow as
                // plain trims into the room that shift just made (a second ripple would shift twice)
                ids.iter().enumerate().all(|(i, &id)| p.ripple_trim(id, *start, *edge0 + *dt, i == 0))
            };
            if !ok {
                *p = d.before.clone();
            }
            ok
        }
        Gesture::Segment { ids, spans, dt } => {
            let ok = segment_move(p, ids, spans, *dt);
            if !ok {
                *p = d.before.clone();
            }
            ok
        }
        // the plain move stopped short of the pointer on a magnetic track: shove instead of refusing
        Gesture::Move { magnetic: true, ids, dt, dtrack, want, .. }
            if (want.0 - dt).abs() > 1e-9 || want.1 != *dtrack =>
        {
            p.magnetic_move(ids, want.0 - dt, want.1 - dtrack) || edited
        }
        _ => edited,
    }
}

impl Gesture {
    /// History row label for the gestures this workstream added; "" = the pre-existing derived label.
    pub(super) fn label(&self) -> &'static str {
        match self {
            Gesture::Roll { .. } => "Roll edit",
            Gesture::Slip { .. } => "Slip",
            Gesture::Slide { .. } => "Slide",
            Gesture::RippleTrim { multi: true, .. } => "Trim edges",
            Gesture::RippleTrim { .. } => "Ripple trim",
            Gesture::Segment { .. } => "Segment move",
            Gesture::Move { magnetic: true, .. } => "Move clips",
            _ => "",
        }
    }
}

fn ghost_rect(lp: &egui::Painter, r: Rect, pal: &Palette) {
    lp.rect_filled(r, 0, pal.accent.gamma_multiply(0.18));
    lp.rect_stroke(r, 0, Stroke::new(1.0, pal.accent), StrokeKind::Inside);
}

/// Translucent outline of every clip `off(track, clip)` says will move, drawn that many seconds from
/// where it is — pure screen-space arithmetic, no model call (the ripple / segment / splice-drop ghosts).
pub(super) fn paint_offsets(
    lp: &egui::Painter,
    state: &TimelineState,
    pal: &Palette,
    p: &Project,
    lanes: Rect,
    off: impl Fn(usize, &Clip) -> Option<f64>,
) {
    let mut tops = vec![None; p.tracks.len()];
    let mut top = lanes.top() - state.scroll_y;
    for i in row_order(p) {
        tops[i] = Some(top);
        top += p.tracks[i].height;
    }
    for (ti, cl) in p.all_clips() {
        let Some(d) = off(ti, cl).filter(|d| d.abs() > 1e-9) else { continue };
        let Some(top) = tops[ti] else { continue };
        let h = p.tracks[ti].height;
        let r = Rect::from_min_max(
            pos2(state.x_at(cl.start + d), top + 1.0),
            pos2(state.x_at(cl.end() + d), top + h - 1.0),
        )
        .intersect(lanes);
        if r.is_positive() {
            ghost_rect(lp, r, pal);
        }
    }
}

/// Ghost of the live RippleTrim / Segment drag: the trimmed or moved clips at their new extent, plus
/// every clip the release will shift, offset by exactly what the model call will do to it.
pub(super) fn paint_ghost(lp: &egui::Painter, state: &TimelineState, pal: &Palette, p: &Project, lanes: Rect) {
    let Some(Drag { g, .. }) = &state.drag else { return };
    match g {
        Gesture::RippleTrim { ids, start, edge0, dt, multi } if dt.abs() > 1e-9 => {
            for &id in ids {
                let Some((ti, ci)) = p.find(id) else { continue };
                let Some(top) = row_top(state, p, ti) else { continue };
                let cl = &p.tracks[ti].clips[ci];
                let (s, e) = if *start { (cl.start + dt, cl.end()) } else { (cl.start, cl.end() + dt) };
                let r = Rect::from_min_max(
                    pos2(state.x_at(s), top + 1.0),
                    pos2(state.x_at(e), top + p.tracks[ti].height - 1.0),
                )
                .intersect(lanes);
                if r.is_positive() {
                    ghost_rect(lp, r, pal);
                }
            }
            // downstream shift: end edge only (a start-edge ripple trim never moves clips), single-clip only
            if !*multi && !*start {
                let ripple = p.ripple_tracks();
                paint_offsets(lp, state, pal, p, lanes, |ti, cl| {
                    (ripple.contains(&ti) && !ids.contains(&cl.id) && cl.start >= *edge0 - ABUT_EPS).then_some(*dt)
                });
            }
        }
        Gesture::Segment { ids, spans, dt } if dt.abs() > 1e-9 => {
            // moved clips land dt later; on each of their tracks the content between the old slot and
            // the new one flows the other way by the clip's length (see `segment_move`)
            paint_offsets(lp, state, pal, p, lanes, |ti, cl| {
                if ids.contains(&cl.id) {
                    return Some(*dt);
                }
                let &(_, s, e) = spans.iter().find(|&&(t, _, _)| t == ti)?;
                let (len, target) = (e - s, s + dt);
                if target >= s {
                    (cl.start >= e - ABUT_EPS && cl.start < target + len - ABUT_EPS).then_some(-len)
                } else {
                    (cl.start >= target - ABUT_EPS && cl.start < s - ABUT_EPS).then_some(len)
                }
            });
        }
        _ => {}
    }
}

/// Starts a new drag gesture (if one of the `start_*` options fired this frame) and, while one is
/// active, applies it to `c.project` for the current pointer position. Mutates `state.drag` and
/// `c.project` directly; nothing here needs to report back through `TimelineResponse`.
#[allow(clippy::too_many_arguments)]
pub(super) fn handle(
    ui: &mut egui::Ui,
    state: &mut TimelineState,
    c: &mut TimelineCtx<'_>,
    pointer: Option<Pos2>,
    mods: egui::Modifiers,
    lanes: Rect,
    start_spacer: bool,
    start_move: Option<Id>,
    start_trim: Option<(Id, bool)>,
    start_vol: Option<Id>,
    start_fade: Option<(Id, bool)>,
    start_key: Option<(Id, f64, Option<usize>)>,
    start_trans: Option<(usize, Id)>,
    start_marker: Option<(Id, Option<Id>)>,
) {
    // ---- gestures ----
    let origin = ui.input(|i| i.pointer.press_origin()).or(pointer).unwrap_or(Pos2::ZERO);
    // with the spacer tool every lane press is a gap gesture, clip bodies and edges included
    let spacer = c.tool == Tool::Spacer && (start_spacer || start_move.is_some() || start_trim.is_some());
    // arm(): the frozen modifier table decides what this body/edge press becomes. A locked track arms
    // nothing at all (every gesture refused, no undo); an undefined modifier combo arms nothing either.
    let pressed = start_move.or(start_trim.map(|(id, _)| id)).filter(|_| !spacer);
    let armed = pressed.and_then(|cid| {
        let ti = c.project.track_of(cid)?;
        let t = &c.project.tracks[ti];
        if t.locked {
            return None;
        }
        let flags = TrackFlags { locked: t.locked, ripple: t.ripple.unwrap_or(false), magnetic: t.magnetic };
        let zone = match (start_move, start_trim) {
            (None, Some((_, true))) => Zone::EdgeStart,
            (None, Some((_, false))) => Zone::EdgeEnd,
            _ => Zone::Body,
        };
        arm(mods, zone, flags, c.tool)
    });
    if let Some((cid, kind)) = pressed.zip(armed) {
        if arm_selects(kind) && !c.selection.contains(&cid) {
            if !mods.ctrl {
                c.selection.clear();
            }
            c.selection.push(cid);
        }
    }
    if spacer {
        let t0 = state.time_at(origin.x).max(0.0);
        let ids: Vec<Id> = c.project.all_clips().filter(|(_, cl)| cl.start >= t0).map(|(_, cl)| cl.id).collect();
        // how far left the whole group can go: the tightest gap in front of it, per track
        let room = c
            .project
            .tracks
            .iter()
            .filter_map(|t| {
                let first = t.clips.iter().map(|cl| cl.start).filter(|&s| s >= t0).fold(f64::INFINITY, f64::min);
                let prev = t.clips.iter().filter(|cl| cl.start < t0).map(|cl| cl.end()).fold(0.0, f64::max);
                first.is_finite().then_some(first - prev)
            })
            .fold(f64::INFINITY, f64::min);
        let room = if room.is_finite() { room.max(0.0) } else { 0.0 };
        state.drag =
            Some(Drag { origin, before: c.project.clone(), g: Gesture::Spacer { ids, dt: 0.0, room }, snapped: None });
    } else if let Some((cid, kind)) = pressed.zip(armed) {
        let p = &*c.project;
        let g = if let Some((_, start)) = start_trim.filter(|_| start_move.is_none()) {
            p.clip(cid).map(|clip| {
                let edge = if start { clip.start } else { clip.end() };
                // linked clips trim together when their edge coincides (the pressed clip first)
                let same_edge = |id: Id| {
                    p.clip(id).map_or(false, |cl| ((if start { cl.start } else { cl.end() }) - edge).abs() < 1e-6)
                };
                let mut ids = vec![cid];
                ids.extend(p.linked(cid).into_iter().filter(|&id| id != cid && same_edge(id)));
                let stretch = || Gesture::Stretch { id: cid, start, edge, src_len: clip.src_len(), changed: false };
                match kind {
                    GestureKind::RateStretch => stretch(),
                    GestureKind::RippleTrim => Gesture::RippleTrim { ids, start, edge0: edge, dt: 0.0, multi: false },
                    GestureKind::MultiRippleTrim => {
                        let mut ids = p.expand_links(c.selection);
                        ids.retain(|&id| id != cid);
                        ids.insert(0, cid);
                        Gesture::RippleTrim { ids, start, edge0: edge, dt: 0.0, multi: true }
                    }
                    GestureKind::Roll => {
                        let t = &p.tracks[p.track_of(cid).unwrap_or(0)];
                        let (left, right) = if start {
                            (t.left_of(clip).map(|l| l.id), Some(cid))
                        } else {
                            let r = t.clips.iter().find(|o| o.id != cid && (o.start - clip.end()).abs() < ABUT_EPS);
                            (Some(cid), r.map(|r| r.id))
                        };
                        match left.zip(right) {
                            Some((left, right)) => Gesture::Roll { left, right, cut0: edge, changed: false },
                            // no abutting neighbour: the table's fallback is a plain trim
                            None => build_trim(p, &ids, start, &state.rollers),
                        }
                    }
                    _ if c.tool == Tool::Stretch => stretch(),
                    _ => build_trim(p, &ids, start, &state.rollers),
                }
            })
        } else {
            p.track_of(cid).map(|tr| match kind {
                GestureKind::Slip => {
                    let ids = p.linked(cid);
                    let src0 = ids.iter().map(|&id| p.clip(id).map_or(0.0, |cl| cl.src_in)).collect();
                    Gesture::Slip { ids, src0, changed: false }
                }
                GestureKind::Slide => {
                    Gesture::Slide { id: cid, start0: p.clip(cid).map_or(0.0, |cl| cl.start), changed: false }
                }
                GestureKind::Segment => {
                    let ids = p.linked(cid);
                    let spans = ids
                        .iter()
                        .filter_map(|&id| {
                            let (ti, ci) = p.find(id)?;
                            let cl = &p.tracks[ti].clips[ci];
                            Some((ti, cl.start, cl.end()))
                        })
                        .collect();
                    Gesture::Segment { ids, spans, dt: 0.0 }
                }
                _ => {
                    let ids = p.expand_links(c.selection);
                    let orig = ids.iter().map(|&id| p.clip(id).map(|cl| cl.start).unwrap_or(0.0)).collect();
                    Gesture::Move {
                        ids,
                        orig,
                        kind: p.tracks[tr].kind,
                        tr,
                        dt: 0.0,
                        dtrack: 0,
                        new_track: false,
                        magnetic: kind == GestureKind::MagneticMove,
                        want: (0.0, 0),
                    }
                }
            })
        };
        if let Some(g) = g {
            // ws:pro-timeline: a Trim gesture just consumed the pending roller set (build_trim folded
            // it in above); any other gesture kind starting on an edge/body drops it too, rather than
            // leaving stale seams armed for an unrelated later trim.
            state.rollers.clear();
            state.drag = Some(Drag { origin, before: c.project.clone(), g, snapped: None });
        }
    } else if let Some(cid) = start_vol {
        state.drag = Some(Drag {
            origin,
            before: c.project.clone(),
            g: Gesture::Volume { id: cid, changed: false },
            snapped: None,
        });
    } else if let Some((cid, fout)) = start_fade {
        state.drag = Some(Drag {
            origin,
            before: c.project.clone(),
            g: Gesture::Fade { id: cid, out: fout, changed: false },
            snapped: None,
        });
    } else if let Some((cid, kt, prop)) = start_key {
        // freeze the value scale at press time: dragging a key must not move the scale under itself
        let range = prop
            .and_then(|pi| c.project.clip(cid).and_then(|cl| crate::ui::curves::prop_ref(cl, pi)))
            .map(crate::ui::curves::y_range)
            .unwrap_or((0.0, 1.0));
        state.drag = Some(Drag {
            origin,
            before: c.project.clone(),
            g: Gesture::Keys { id: cid, t: kt, prop, range, changed: false },
            snapped: None,
        });
    } else if let Some((tri, tid)) = start_trans {
        state.drag = Some(Drag {
            origin,
            before: c.project.clone(),
            g: Gesture::TransDur { track: tri, id: tid, changed: false },
            snapped: None,
        });
    } else if let Some((mid, mclip)) = start_marker {
        state.drag = Some(Drag {
            origin,
            before: c.project.clone(),
            g: Gesture::Marker { id: mid, clip: mclip, changed: false },
            snapped: None,
        });
    }
    let hover_tr = pointer.and_then(|pos| state.track_at(pos.y, c.project));
    // scalar copies + closures so the gesture arms can use geometry while `state.drag` is mutably borrowed
    let (lx, sx, sy, ltop) = (lanes.left(), state.scroll_x, state.scroll_y, lanes.top());
    let zoom0 = state.zoom;
    let t_at = move |x: f32| sx + ((x - lx) / zoom0) as f64;
    let row_top_of = move |p: &Project, ti: usize| -> Option<f32> {
        let mut top = ltop - sy;
        for i in row_order(p) {
            if i == ti {
                return Some(top);
            }
            top += p.tracks[i].height;
        }
        None
    };
    if let (Some(drag), Some(pos)) = (state.drag.as_mut(), pointer) {
        let zoom = zoom0;
        let dx = (pos.x - drag.origin.x) as f64 / zoom as f64;
        let ox = drag.origin.x; // grab point (auto-scroll compensates it), for gestures that edit at one time
        let thr = snap_thr(zoom, c.project.fps);
        let p = &mut *c.project;
        let Drag { g, snapped, .. } = drag;
        match g {
            Gesture::Move { ids, orig, kind, tr, dt, dtrack, new_track, want: requested, .. } => {
                // past the first video row (up) or the last audio row (down): offer a fresh track.
                // Armed off the painted gutter bands, not the first/last row — those are scrolled away
                // once the lanes scroll, which used to make the gesture unreachable.
                *new_track = match kind {
                    TrackKind::Video => pos.y < lanes.top() + GUTTER_H,
                    TrackKind::Audio => pos.y > lanes.bottom() - GUTTER_H,
                };
                let mut want = dx;
                *snapped = None;
                if c.snap {
                    let s0 = orig.first().copied().unwrap_or(0.0);
                    want = p.snap_frame(s0 + want) - s0;
                    let mut best: Option<f64> = None;
                    for (&id, &s) in ids.iter().zip(orig.iter()) {
                        let Some(cl) = p.clip(id) else { continue };
                        for edge in [s + want, s + want + cl.duration] {
                            if let Some(tgt) = snap_target(edge, thr, p, *c.playhead, ids) {
                                let adj = tgt - edge;
                                if best.map_or(true, |b: f64| adj.abs() < b.abs()) {
                                    best = Some(adj);
                                    *snapped = Some(tgt);
                                }
                            }
                        }
                    }
                    want += best.unwrap_or(0.0);
                }
                let min_start = orig.iter().copied().fold(f64::INFINITY, f64::min);
                want = want.max(-min_start);
                // destination row = same-kind track under the pointer (keeps the last one while outside)
                let list = if *kind == TrackKind::Video { p.video_tracks() } else { p.audio_tracks() };
                let pos_of = |t: usize| list.iter().position(|&x| x == t);
                let want_tr = match (hover_tr.and_then(pos_of), pos_of(*tr)) {
                    (Some(h), Some(o)) => h as i32 - o as i32,
                    _ => *dtrack,
                };
                *requested = (want, want_tr);
                let (ddt, ddtr) = (want - *dt, want_tr - *dtrack);
                if ddt.abs() > 1e-9 || ddtr != 0 {
                    if p.move_clips(ids, ddt, ddtr, Some(*kind)) {
                        *dt = want;
                        *dtrack = want_tr;
                    } else if ddtr != 0 && ddt.abs() > 1e-9 {
                        if p.move_clips(ids, ddt, 0, Some(*kind)) {
                            *dt = want;
                        } else if p.move_clips(ids, 0.0, ddtr, Some(*kind)) {
                            *dtrack = want_tr;
                        }
                    }
                }
            }
            Gesture::Trim { ids, edge0, changed } => {
                // ws:pro-timeline — asymmetric multi-roller: snap the FIRST roller's delta (its own
                // press-time edge + dx), then apply that SAME delta to every roller from ITS OWN
                // edge0 ("trims both cuts by the same delta", not independently re-snapped each).
                let mut dt = dx;
                *snapped = None;
                if c.snap {
                    if let Some(&e0) = edge0.first() {
                        let base_ids: Vec<Id> = ids.iter().map(|&(id, _)| id).collect();
                        let mut want0 = p.snap_frame(e0 + dx);
                        if let Some(t) = snap_target(want0, thr, p, *c.playhead, &base_ids) {
                            want0 = t;
                            *snapped = Some(t);
                        }
                        dt = want0 - e0;
                    }
                }
                // all-or-nothing (like move_clips): linked/rolled clips keep identical extents when one is blocked
                let mut upd = Vec::with_capacity(ids.len());
                for (&(id, start), &e0) in ids.iter().zip(edge0.iter()) {
                    let Some((ti, ci)) = p.find(id) else { continue };
                    let mut tmp = p.tracks[ti].clips[ci].clone();
                    let want = e0 + dt;
                    if start {
                        let hr = p.head_room(&tmp);
                        tmp.trim_start(want, hr);
                    } else {
                        let md = p.max_clip_duration(&tmp);
                        tmp.trim_end(want, md);
                    }
                    if !p.tracks[ti].fits(tmp.start, tmp.duration, &[id]) {
                        upd.clear();
                        break;
                    }
                    upd.push((ti, ci, tmp));
                }
                for (ti, ci, tmp) in upd {
                    let cl = &p.tracks[ti].clips[ci];
                    if cl.start != tmp.start || cl.duration != tmp.duration {
                        p.tracks[ti].clips[ci] = tmp;
                        *changed = true;
                    }
                }
            }
            Gesture::Stretch { id, start, edge, src_len, changed } => {
                let mut want = *edge + dx;
                *snapped = None;
                if c.snap {
                    want = p.snap_frame(want);
                    if let Some(t) = snap_target(want, thr, p, *c.playhead, &[*id]) {
                        want = t;
                        *snapped = Some(t);
                    }
                }
                if let Some((ti, ci)) = p.find(*id) {
                    let cl = &p.tracks[ti].clips[ci];
                    let (new_start, dur) = if *start {
                        let w = want.clamp(0.0, cl.end() - crate::model::MIN_CLIP);
                        (w, cl.end() - w)
                    } else {
                        (cl.start, want - cl.start)
                    };
                    let dur = dur.max(crate::model::MIN_CLIP);
                    if p.tracks[ti].fits(new_start, dur, &[*id]) {
                        let cl = &mut p.tracks[ti].clips[ci];
                        if (cl.duration - dur).abs() > 1e-9 || (cl.start - new_start).abs() > 1e-9 {
                            // speed = source seconds per timeline second: the window is untouched
                            cl.set_speed(*src_len / dur);
                            cl.start = new_start;
                            *changed = true;
                        }
                    }
                }
            }
            Gesture::Roll { left, right, cut0, changed } => {
                let mut want = *cut0 + dx;
                *snapped = None;
                if c.snap {
                    want = p.snap_frame(want);
                    // the cut's own two clips would pin it where it already is
                    if let Some(t) = snap_target(want, thr, p, *c.playhead, &[*left, *right]) {
                        want = t;
                        *snapped = Some(t);
                    }
                }
                if p.roll_edit(*right, want) {
                    *changed = true;
                }
            }
            Gesture::Slip { ids, src0, changed } => {
                // pointer right = content right = an earlier source frame under the fixed left edge;
                // always from the press-time window, so a clamped clip never drifts off the pointer
                for (&id, &s0) in ids.iter().zip(src0.iter()) {
                    let Some(cl) = p.clip(id) else { continue };
                    let d = -dx * cl.speed;
                    let want = s0 + if c.snap { p.snap_frame(d) } else { d };
                    let cur = cl.src_in;
                    if (want - cur).abs() > 1e-9 && p.slip(&[id], want - cur) {
                        *changed = true;
                    }
                }
            }
            Gesture::Slide { id, start0, changed } => {
                let mut want = *start0 + dx;
                *snapped = None;
                let Some((ti, ci)) = p.find(*id) else { return };
                let cl = &p.tracks[ti].clips[ci];
                let (cur, dur) = (cl.start, cl.duration);
                if c.snap {
                    want = p.snap_frame(want);
                    // the abutting neighbours share this clip's edges: exclude them or the snap pins it
                    let t = &p.tracks[ti];
                    let mut excl = vec![*id];
                    excl.extend(t.left_of(cl).map(|l| l.id));
                    excl.extend(
                        t.clips.iter().find(|o| o.id != *id && (o.start - cl.end()).abs() < ABUT_EPS).map(|r| r.id),
                    );
                    let mut best: Option<f64> = None;
                    for edge in [want, want + dur] {
                        if let Some(tgt) = snap_target(edge, thr, p, *c.playhead, &excl) {
                            let adj = tgt - edge;
                            if best.map_or(true, |b: f64| adj.abs() < b.abs()) {
                                best = Some(adj);
                                *snapped = Some(tgt);
                            }
                        }
                    }
                    want += best.unwrap_or(0.0);
                }
                if (want - cur).abs() > 1e-9 && p.slide(*id, want - cur) {
                    *changed = true;
                }
            }
            Gesture::RippleTrim { ids, start, edge0, dt, .. } => {
                // ghost only: the model is untouched until release
                let mut want = *edge0 + dx;
                *snapped = None;
                if c.snap {
                    want = p.snap_frame(want);
                    if let Some(t) = snap_target(want, thr, p, *c.playhead, ids) {
                        want = t;
                        *snapped = Some(t);
                    }
                }
                // keep the ghost where a trim can actually go: this side of the other edge, never < 0
                if let Some(cl) = ids.first().and_then(|&id| p.clip(id)) {
                    want = if *start { want.clamp(0.0, cl.end() - MIN_CLIP) } else { want.max(cl.start + MIN_CLIP) };
                }
                *dt = want - *edge0;
            }
            Gesture::Segment { ids, spans, dt } => {
                // ghost only: the model is untouched until release
                let Some(&(_, a, _)) = spans.first() else { return };
                let mut want = a + dx;
                *snapped = None;
                if c.snap {
                    want = p.snap_frame(want);
                    if let Some(t) = snap_target(want, thr, p, *c.playhead, ids) {
                        want = t;
                        *snapped = Some(t);
                    }
                }
                *dt = want.max(0.0) - a;
            }
            Gesture::Volume { id, changed } => {
                if let Some((ti, ci)) = p.find(*id) {
                    if let Some(top) = row_top_of(p, ti) {
                        let h = (p.tracks[ti].height - 2.0).max(1.0);
                        let frac = ((top + p.tracks[ti].height - 1.0 - pos.y) / h).clamp(0.0, 1.0);
                        let db = frac_db(frac);
                        let gain = if db <= DB_BOT + 0.25 { 0.0 } else { 10f64.powf(db as f64 / 20.0) };
                        let cl = &mut p.tracks[ti].clips[ci];
                        if cl.volume.is_animated() {
                            // latch to the grab time: editing at the live x inserts a key per frame of the drag
                            let lt = (t_at(ox) - cl.start).clamp(0.0, cl.duration);
                            cl.volume.set_at(lt, gain);
                            *changed = true;
                        } else if (cl.volume.value - gain).abs() > 1e-9 {
                            cl.volume.value = gain;
                            *changed = true;
                        }
                    }
                }
            }
            Gesture::Fade { id, out: fout, changed } => {
                if let Some(cl) = p.clip_mut(*id) {
                    let t = t_at(pos.x);
                    let v = if *fout {
                        (cl.end() - t).clamp(0.0, cl.duration)
                    } else {
                        (t - cl.start).clamp(0.0, cl.duration)
                    };
                    let dst = if *fout { &mut cl.fade_out } else { &mut cl.fade_in };
                    if (*dst - v).abs() > 1e-9 {
                        *dst = v;
                        *changed = true;
                    }
                }
            }
            Gesture::Keys { id, t, prop, range, changed } => {
                let nt = p.snap_frame(t_at(pos.x));
                if let Some(cl) = p.clip_mut(*id) {
                    let lt = (nt - cl.start).clamp(0.0, cl.duration);
                    if (lt - *t).abs() > 1e-9 {
                        cl.move_keys(*t, lt);
                        *t = lt;
                        *changed = true;
                    }
                }
                // value lane: y inside the clip rect → value, through the range frozen at press time
                if let (Some(pi), Some((ti, ci))) = (*prop, p.find(*id)) {
                    if let Some(top) = row_top_of(p, ti) {
                        let h = p.tracks[ti].height;
                        let inner = (h - 2.0 - 2.0 * KEY_PAD).max(1.0);
                        let f = (((top + h - 1.0 - KEY_PAD) - pos.y) / inner).clamp(0.0, 1.0) as f64;
                        let v = range.0 + f * (range.1 - range.0);
                        // the lane auto-scales past the property's own bounds (y_range pads by 10 %) —
                        // clamp the written value the way every DragValue for it does
                        let v = prop_range(&p.tracks[ti].clips[ci], pi).map_or(v, |(lo, hi)| v.clamp(lo, hi));
                        if let Some(a) = crate::ui::curves::prop_mut(&mut p.tracks[ti].clips[ci], pi) {
                            if let Some(ki) = a.key_index_at(*t) {
                                if (a.keys[ki].v - v).abs() > 1e-9 {
                                    a.keys[ki].v = v;
                                    *changed = true;
                                }
                            }
                        }
                    }
                }
            }
            Gesture::TransDur { track, id, changed } => {
                // the project can be replaced mid-drag (undo, MCP project.open/sequence.open): index may be stale
                if let Some(tr) = p.tracks.get_mut(*track) {
                    // duration follows the dragged edge: distance from the anchor (the cut, or the
                    // clip edge the transition hangs off), capped by what the clip(s) can supply
                    let lim = tr.transitions.iter().find(|t| t.id == *id).and_then(|t| {
                        let (l, r) = tr.transition_clips(t)?;
                        Some(match t.edge {
                            crate::model::TransitionEdge::Cut => (r?.start, 2.0, 2.0 * l?.duration.min(r?.duration)),
                            crate::model::TransitionEdge::In => (r?.start, 1.0, r?.duration),
                            crate::model::TransitionEdge::Out => (l?.end(), 1.0, l?.duration),
                        })
                    });
                    if let Some((anchor, scale, max)) = lim {
                        // cap: the Transitions panel's 0.1..5 s range, and what the clip(s) can actually supply
                        let d = ((t_at(pos.x) - anchor).abs() * scale).min(max).min(5.0).max(0.1);
                        if let Some(t) = tr.transitions.iter_mut().find(|t| t.id == *id) {
                            if (t.duration - d).abs() > 1e-9 {
                                t.duration = d;
                                *changed = true;
                            }
                        }
                    }
                }
            }
            Gesture::Marker { id, clip, changed } => {
                // markers always land on a frame; snapping additionally pulls them onto clip edges
                let want = p.snap_frame(t_at(pos.x).max(0.0));
                let hit = c.snap.then(|| snap_target(want, thr, p, *c.playhead, &[])).flatten();
                *snapped = hit;
                let want = hit.unwrap_or(want);
                // clip markers are clip-local and stay inside their clip
                let nt = match clip.and_then(|cid| p.clip(cid)) {
                    Some(cl) => (want - cl.start).clamp(0.0, cl.duration),
                    None => want,
                };
                if let Some(m) = p.marker_mut(*id) {
                    if (m.t - nt).abs() > 1e-9 {
                        m.t = nt;
                        *changed = true;
                    }
                }
                if clip.is_none() && *changed {
                    p.sort_markers();
                }
            }
            Gesture::Spacer { ids, dt, room } => {
                // move_clips is all-or-nothing, so the clip in front of the group is never overrun
                let mut want = if c.snap { p.snap_frame(dx) } else { dx };
                *snapped = None;
                if c.snap {
                    // what the user watches move is the group's leading edge: snap that, not the raw delta
                    let now = ids.iter().filter_map(|&id| p.clip(id)).map(|cl| cl.start).fold(f64::INFINITY, f64::min);
                    let lead = now - *dt; // where that edge sat before the gesture
                    if lead.is_finite() {
                        if let Some(t) = snap_target(lead + want, thr, p, *c.playhead, ids) {
                            want = t - lead;
                            *snapped = Some(t);
                        }
                    }
                }
                let want = want.max(-*room);
                if (want - *dt).abs() > 1e-9 && p.move_clips(ids, want - *dt, 0, None) {
                    *dt = want;
                }
            }
            Gesture::InOut { out, changed } => {
                let mut want = p.snap_frame(t_at(pos.x).max(0.0));
                *snapped = None;
                if c.snap {
                    if let Some(t) = snap_target(want, thr, p, *c.playhead, &[]) {
                        want = t;
                        *snapped = Some(t);
                    }
                }
                if *out {
                    let want = want.max(p.in_point.unwrap_or(0.0));
                    if p.out_point != Some(want) {
                        p.out_point = Some(want);
                        *changed = true;
                    }
                } else {
                    let want = p.out_point.map_or(want, |o| want.min(o));
                    if p.in_point != Some(want) {
                        p.in_point = Some(want);
                        *changed = true;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipKind, Project, TrackKind};

    #[test]
    fn build_trim_folds_rollers_with_their_own_edges() {
        let mut p = Project::new();
        p.tracks[0].clips = vec![Clip::new(201, ClipKind::Video, "a", 0.0, 3.0)];
        p.add_track(TrackKind::Video);
        let v2 = p.video_tracks()[1];
        p.tracks[v2].clips.push(Clip::new(202, ClipKind::Video, "d", 1.0, 3.0));
        // pressed: D's start edge (true); roller set: A's end edge (false) -- asymmetric (mixed sides)
        let g = build_trim(&p, &[202], true, &[(201, false)]);
        let Gesture::Trim { ids, edge0, .. } = g else { panic!("build_trim must return a Gesture::Trim") };
        assert_eq!(ids.len(), 2);
        let d_i = ids.iter().position(|&(id, _)| id == 202).expect("D in the roller set");
        let a_i = ids.iter().position(|&(id, _)| id == 201).expect("A in the roller set");
        assert_eq!(ids[d_i], (202, true));
        assert_eq!(ids[a_i], (201, false));
        assert_eq!(edge0[d_i], 1.0, "D's own press-time start");
        assert_eq!(edge0[a_i], 3.0, "A's own press-time end");
    }

    #[test]
    fn build_trim_drops_a_roller_whose_clip_no_longer_exists() {
        let mut p = Project::new();
        p.tracks[0].clips = vec![Clip::new(201, ClipKind::Video, "a", 0.0, 3.0)];
        let g = build_trim(&p, &[201], false, &[(9999, true)]); // 9999 doesn't exist
        let Gesture::Trim { ids, edge0, .. } = g else { panic!("build_trim must return a Gesture::Trim") };
        assert_eq!(ids, vec![(201, false)]);
        assert_eq!(edge0, vec![3.0]);
    }

    #[test]
    fn build_trim_dedupes_a_roller_matching_the_pressed_edge() {
        let mut p = Project::new();
        p.tracks[0].clips = vec![Clip::new(201, ClipKind::Video, "a", 0.0, 3.0)];
        // the roller set already names the pressed clip/side -- must not appear twice
        let g = build_trim(&p, &[201], false, &[(201, false)]);
        let Gesture::Trim { ids, .. } = g else { panic!("build_trim must return a Gesture::Trim") };
        assert_eq!(ids, vec![(201, false)]);
    }
}
