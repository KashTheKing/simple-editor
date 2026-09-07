//! Tiered snap engine: playhead > cursor > selected-clip edges > adjacent-clip edges > markers >
//! transition edges > in/out > zero. `target()` is the single tiered entry point; `snap_target` /
//! `snap_time` / `snap_playhead` are thin back-compat wrappers kept at their pre-existing signatures so
//! every call site that already used them (gestures.rs's Move/Trim/Stretch/Marker/Spacer, mod.rs's
//! razor/marker-tool/ruler-scrub/dnd-drop) compiles unchanged and keeps passing its pinned tests, while
//! gaining markers/transition edges as candidates for free.
use super::*;

/// Which tier a snap hit came from — surfaced to scripts/tests via `timeline.snap_query` and used
/// internally to stop at the first tier with a hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapKind {
    Zero,
    Playhead,
    Cursor,
    SelectedEdge,
    ClipEdge,
    Marker,
    TransitionEdge,
    InOut,
}

/// Nearest candidate within `thr` of `t`.
pub(super) fn nearest(t: f64, thr: f64, candidates: impl Iterator<Item = f64>) -> Option<f64> {
    let mut best: Option<f64> = None;
    for x in candidates {
        let d = (x - t).abs();
        if d <= thr && best.map_or(true, |b| d < (b - t).abs()) {
            best = Some(x);
        }
    }
    best
}

/// Project markers (open-sequence only, same filter the ruler paints) + every clip's local markers, as
/// absolute timeline times.
fn marker_candidates(p: &Project) -> impl Iterator<Item = f64> + '_ {
    let proj = p.markers.iter().filter(|m| m.sequence == p.editing).map(|m| m.t);
    let clip = p.all_clips().flat_map(|(_, c)| c.markers.iter().map(move |m| c.start + m.t));
    proj.chain(clip)
}

/// Every transition's played-window edges (cut - half, cut + half) across every track.
fn transition_edge_candidates(p: &Project) -> impl Iterator<Item = f64> + '_ {
    p.tracks
        .iter()
        .flat_map(|t| {
            t.transitions.iter().filter_map(move |tr| {
                let (l, r) = t.transition_clips(tr)?;
                let (cut, half) = tr.cut_half(l, r)?;
                Some([cut - half, cut + half])
            })
        })
        .flatten()
}

/// Tiered engine: tries playhead, then `cursor` (a caller-supplied secondary reference time, e.g. a
/// second monitor's own playhead in a later wave — `None` from every call site today), then selected-
/// clip edges, then every other clip's edges (excluding `exclude`), then markers, then transition edges,
/// then in/out and 0. First tier with a hit inside `thr` wins (nearest candidate within that tier).
/// `snap_markers` gates the marker tier only (`Settings.snap_markers`); every other tier is always live.
///
/// ponytail: `snap_markers` is an 8th parameter rather than folded into `Project`/`Settings` reads —
/// arm.rs and this module stay free of any `Settings` dependency, and the 5 pre-existing gesture call
/// sites (routed through the 5-arg `snap_target` wrapper below) always pass `true`, matching today's
/// behaviour of "markers/transitions come for free" rather than needing Settings threaded through every
/// existing gesture. Only the new code this workstream adds (guide line, seam, ruler in/out, cue drag,
/// `timeline.snap_query`) reads the real `TimelineCtx::snap_markers` / `Settings.snap_markers` value.
pub(crate) fn target(
    p: &Project,
    t: f64,
    thr: f64,
    playhead: f64,
    exclude: &[Id],
    selected: &[Id],
    cursor: Option<f64>,
    snap_markers: bool,
) -> Option<(f64, SnapKind)> {
    if let Some(x) = nearest(t, thr, [playhead].into_iter()) {
        return Some((x, SnapKind::Playhead));
    }
    if let Some(c) = cursor {
        if let Some(x) = nearest(t, thr, [c].into_iter()) {
            return Some((x, SnapKind::Cursor));
        }
    }
    let sel_edges = p.all_clips().filter(|(_, c)| selected.contains(&c.id)).flat_map(|(_, c)| [c.start, c.end()]);
    if let Some(x) = nearest(t, thr, sel_edges) {
        return Some((x, SnapKind::SelectedEdge));
    }
    let adj_edges = p.all_clips().filter(|(_, c)| !exclude.contains(&c.id)).flat_map(|(_, c)| [c.start, c.end()]);
    if let Some(x) = nearest(t, thr, adj_edges) {
        return Some((x, SnapKind::ClipEdge));
    }
    if snap_markers {
        if let Some(x) = nearest(t, thr, marker_candidates(p)) {
            return Some((x, SnapKind::Marker));
        }
    }
    if let Some(x) = nearest(t, thr, transition_edge_candidates(p)) {
        return Some((x, SnapKind::TransitionEdge));
    }
    if let Some(x) = nearest(t, thr, p.in_point.into_iter().chain(p.out_point)) {
        return Some((x, SnapKind::InOut));
    }
    nearest(t, thr, [0.0].into_iter()).map(|x| (x, SnapKind::Zero))
}

/// Snap threshold in seconds: a fixed on-screen `SNAP_PX` shrinks as `zoom` (px/s) grows, floored at
/// one physical frame (`1/fps`) so a far-zoomed-in drag is frame-quantised only, never magnetic.
/// Matches the old flat `SNAP_PX / zoom` formula exactly except for the new high-zoom floor.
pub(crate) fn snap_thr(zoom: f32, fps: f64) -> f64 {
    let px_thr = (SNAP_PX / zoom.max(0.01)) as f64;
    px_thr.max(1.0 / fps.max(1.0))
}

/// Snap target for `t`: 0, playhead, in/out, edges of clips not in `exclude`, markers and transition
/// edges — within `thr` seconds. Back-compat wrapper over `target()` (selected=&[], cursor=None,
/// snap_markers=true) — every one of the 11 pre-existing call sites keeps this exact signature.
pub(super) fn snap_target(t: f64, thr: f64, p: &Project, playhead: f64, exclude: &[Id]) -> Option<f64> {
    target(p, t, thr, playhead, exclude, &[], None, true).map(|(x, _)| x)
}

/// Frame-quantise a pointer time and pull it onto the nearest snap candidate. Every tool that turns a
/// pointer x into a time goes through here, so the razor, the marker tool, marker drags and media drops
/// land on the same edges that moves and trims already snap to.
pub(crate) fn snap_time(t: f64, on: bool, zoom: f32, p: &Project, playhead: f64, exclude: &[Id]) -> f64 {
    if !on {
        return t;
    }
    let t = p.snap_frame(t);
    snap_target(t, snap_thr(zoom, p.fps), p, playhead, exclude).unwrap_or(t)
}

/// Where the playhead lands when snapping is on: clip edges, markers, transitions, in/out and 0, but
/// NOT the playhead itself (it would always be the nearest candidate and pin it where it already is) —
/// modelled by passing a playhead tier candidate that can never be within threshold.
pub(super) fn snap_playhead(t: f64, on: bool, zoom: f32, p: &Project) -> f64 {
    if !on {
        return p.snap_frame(t);
    }
    let t = p.snap_frame(t);
    target(p, t, snap_thr(zoom, p.fps), f64::INFINITY, &[], &[], None, true).map(|(x, _)| x).unwrap_or(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipKind};

    fn two_clips() -> Project {
        let mut p = Project::new();
        p.tracks[0].clips.push(Clip::new(7, ClipKind::Video, "a", 2.0, 3.0)); // [2.0, 5.0)
        p.tracks[0].clips.push(Clip::new(8, ClipKind::Video, "b", 6.0, 1.0)); // [6.0, 7.0)
        p
    }

    #[test]
    fn tiered_snap_prefers_playhead_over_far_edge() {
        let p = two_clips();
        // t=4.9 is numerically closer to clip 7's end (5.0, dist 0.1) than to the playhead (4.7,
        // dist 0.2), but the playhead tier is checked first and wins as soon as it's in range.
        let hit = target(&p, 4.9, 1.0, 4.7, &[], &[], None, true);
        assert_eq!(hit, Some((4.7, SnapKind::Playhead)), "playhead tier must win over a nearer clip edge");
    }

    #[test]
    fn snap_thr_shrinks_with_zoom_and_floors_at_one_frame() {
        let a = snap_thr(10.0, 30.0);
        let b = snap_thr(100.0, 30.0);
        assert!(b < a, "threshold should shrink as zoom grows: {a} -> {b}");
        let floored = snap_thr(100_000.0, 30.0);
        assert!((floored - 1.0 / 30.0).abs() < 1e-9, "must floor at one physical frame: {floored}");
    }
}
