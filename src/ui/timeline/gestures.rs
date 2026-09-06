//! Gesture start + live-drag handling, extracted from `show()`.
use super::*;

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
    if let Some(cid) = start_move.or(start_trim.map(|(id, _)| id)).filter(|_| !spacer) {
        if !c.selection.contains(&cid) {
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
        state.drag = Some(Drag { origin, before: c.project.clone(), g: Gesture::Spacer { ids, dt: 0.0, room } });
    } else if let Some(cid) = start_move {
        if let Some(tr) = c.project.track_of(cid) {
            let ids = c.project.expand_links(c.selection);
            let orig = ids.iter().map(|&id| c.project.clip(id).map(|cl| cl.start).unwrap_or(0.0)).collect();
            let kind = c.project.tracks[tr].kind;
            let g = Gesture::Move { ids, orig, kind, tr, dt: 0.0, dtrack: 0, new_track: false };
            state.drag = Some(Drag { origin, before: c.project.clone(), g });
        }
    } else if let Some((cid, start)) = start_trim {
        if let Some(clip) = c.project.clip(cid) {
            let edge = if start { clip.start } else { clip.end() };
            // linked clips trim together when their edge coincides
            let ids: Vec<Id> = c
                .project
                .linked(cid)
                .into_iter()
                .filter(|&id| {
                    c.project
                        .clip(id)
                        .map_or(false, |cl| ((if start { cl.start } else { cl.end() }) - edge).abs() < 1e-6)
                })
                .collect();
            let g = if c.tool == Tool::Stretch {
                Gesture::Stretch { id: cid, start, edge, src_len: clip.src_len(), changed: false }
            } else {
                Gesture::Trim { ids, start, edge, changed: false }
            };
            state.drag = Some(Drag { origin, before: c.project.clone(), g });
        }
    } else if let Some(cid) = start_vol {
        state.drag = Some(Drag { origin, before: c.project.clone(), g: Gesture::Volume { id: cid, changed: false } });
    } else if let Some((cid, fout)) = start_fade {
        state.drag =
            Some(Drag { origin, before: c.project.clone(), g: Gesture::Fade { id: cid, out: fout, changed: false } });
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
        });
    } else if let Some((tri, tid)) = start_trans {
        state.drag = Some(Drag {
            origin,
            before: c.project.clone(),
            g: Gesture::TransDur { track: tri, id: tid, changed: false },
        });
    } else if let Some((mid, mclip)) = start_marker {
        state.drag = Some(Drag {
            origin,
            before: c.project.clone(),
            g: Gesture::Marker { id: mid, clip: mclip, changed: false },
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
        let thr = (SNAP_PX / zoom) as f64;
        let p = &mut *c.project;
        match &mut drag.g {
            Gesture::Move { ids, orig, kind, tr, dt, dtrack, new_track } => {
                // past the first video row (up) or the last audio row (down): offer a fresh track.
                // Armed off the painted gutter bands, not the first/last row — those are scrolled away
                // once the lanes scroll, which used to make the gesture unreachable.
                *new_track = match kind {
                    TrackKind::Video => pos.y < lanes.top() + GUTTER_H,
                    TrackKind::Audio => pos.y > lanes.bottom() - GUTTER_H,
                };
                let mut want = dx;
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
            Gesture::Trim { ids, start, edge, changed } => {
                let mut want = *edge + dx;
                if c.snap {
                    want = p.snap_frame(want);
                    if let Some(t) = snap_target(want, thr, p, *c.playhead, ids) {
                        want = t;
                    }
                }
                // all-or-nothing (like move_clips): linked clips keep identical extents when one is blocked
                let mut upd = Vec::with_capacity(ids.len());
                for &id in ids.iter() {
                    let Some((ti, ci)) = p.find(id) else { continue };
                    let mut tmp = p.tracks[ti].clips[ci].clone();
                    if *start {
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
                if c.snap {
                    want = p.snap_frame(want);
                    if let Some(t) = snap_target(want, thr, p, *c.playhead, &[*id]) {
                        want = t;
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
                let want = if c.snap { snap_target(want, thr, p, *c.playhead, &[]).unwrap_or(want) } else { want };
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
                if c.snap {
                    // what the user watches move is the group's leading edge: snap that, not the raw delta
                    let now = ids.iter().filter_map(|&id| p.clip(id)).map(|cl| cl.start).fold(f64::INFINITY, f64::min);
                    let lead = now - *dt; // where that edge sat before the gesture
                    if lead.is_finite() {
                        if let Some(t) = snap_target(lead + want, thr, p, *c.playhead, ids) {
                            want = t - lead;
                        }
                    }
                }
                let want = want.max(-*room);
                if (want - *dt).abs() > 1e-9 && p.move_clips(ids, want - *dt, 0, None) {
                    *dt = want;
                }
            }
        }
    }
}
