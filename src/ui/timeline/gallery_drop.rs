//! Dragging Gallery cards onto the timeline: what a payload means under the pointer. One resolver
//! (`aim`) serves both the hover highlight (`paint`) and the release (the app executes the returned
//! `Aim`), so what is lit while dragging is exactly what lands. The target decides the meaning:
//! - a CLIP body takes effects, looks, LUTs, caption styles, speed ramps, keyframe blocks - the whole
//!   selection when the clip under the pointer is one of several selected, else just that clip;
//! - a CUT or clip EDGE takes a transition: the nearest edge within `EDGE_PX` of the pointer wins, else
//!   the half of the clip it is over (left half = its start, right half = its end);
//! - an EMPTY part of a video track (or the lane below the last one: a new track) takes everything that
//!   makes a clip - templates, titles, HTML layers, icons.
//!
//! Media-library payloads (assets, sequences, files) keep their own path in `mod.rs`.
use super::*;
use crate::engine::library::{split_id, Kind};
use crate::ui::gallery::GalleryTab;

/// How far from an edge (screen points) a transition drop still snaps to it.
pub(crate) const EDGE_PX: f32 = 14.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DropKind {
    OnClip,
    OnEdge,
    OnTrack,
    /// A sound: an audio track (or the new-track lane, which makes an audio one).
    OnAudio,
}

/// What kind of target a Gallery payload wants (None: not a Gallery payload, or it makes no sense on a
/// timeline - a skill).
pub(crate) fn drop_kind(p: &DragPayload) -> Option<DropKind> {
    match p {
        DragPayload::Effect(_) | DragPayload::KeyBlock(_) => Some(DropKind::OnClip),
        DragPayload::Transition(_) => Some(DropKind::OnEdge),
        DragPayload::Template(_) => Some(DropKind::OnTrack),
        DragPayload::GalleryCard(tab, _) => match tab {
            GalleryTab::Looks
            | GalleryTab::Luts
            | GalleryTab::Captions
            | GalleryTab::SpeedRamps
            | GalleryTab::Effects => Some(DropKind::OnClip),
            GalleryTab::Transitions => Some(DropKind::OnEdge),
            GalleryTab::Sounds => Some(DropKind::OnAudio),
            GalleryTab::Titles | GalleryTab::Templates => Some(DropKind::OnTrack),
            _ => None,
        },
        DragPayload::LibItem(id) => match split_id(id)?.0 {
            Kind::Effect | Kind::Lut => Some(DropKind::OnClip),
            Kind::Transition => Some(DropKind::OnEdge),
            Kind::Html | Kind::Icon => Some(DropKind::OnTrack),
            Kind::Skill => None,
        },
        _ => None,
    }
}

/// Can `c` take this payload (a pixel effect / look / LUT needs picture; an audio effect needs audio)?
fn takes(p: &DragPayload, c: &Clip) -> bool {
    match p {
        DragPayload::Effect(k) => (c.kind == ClipKind::Audio) == k.applies_to_audio() && !c.uses_graph(),
        DragPayload::GalleryCard(GalleryTab::Looks | GalleryTab::Luts | GalleryTab::Effects, _) => {
            c.is_visual() && !c.uses_graph()
        }
        DragPayload::LibItem(id) => match split_id(id) {
            Some((Kind::Effect | Kind::Lut, _)) => c.is_visual() && !c.uses_graph(),
            _ => true,
        },
        _ => true,
    }
}

/// The resolved target of a drop.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Aim {
    /// Apply to these clips; `hit` is the one under the pointer, `t` the timeline time of the pointer.
    Clips { clips: Vec<Id>, hit: Id, t: f64 },
    /// A transition on `clip`'s start (`out` false) or end edge (a cut when a clip abuts there).
    Edge { clip: Id, out: bool },
    /// Place a new clip at `t`: on video track `track`, or on a NEW track (`track` None).
    Place { track: Option<usize>, t: f64 },
}

/// Where a payload dropped at time `t` over track row `track` (`None` = not over a row) lands.
/// `below`: the pointer is in the lanes but under the last row (the "new track" lane). `thr` = the edge
/// snap distance in seconds (`EDGE_PX / zoom`).
pub(crate) fn aim(
    project: &Project,
    selection: &[Id],
    payload: &DragPayload,
    t: f64,
    track: Option<usize>,
    below: bool,
    thr: f64,
) -> Option<Aim> {
    match drop_kind(payload)? {
        DropKind::OnClip => {
            let clip = project.tracks.get(track?)?.clips.iter().find(|c| c.contains(t))?;
            if !takes(payload, clip) {
                return None;
            }
            let sel: Vec<Id> =
                selection.iter().copied().filter(|&id| project.clip(id).is_some_and(|c| takes(payload, c))).collect();
            let clips = if sel.len() > 1 && sel.contains(&clip.id) { sel } else { vec![clip.id] };
            Some(Aim::Clips { clips, hit: clip.id, t })
        }
        DropKind::OnEdge => {
            let tr = project.tracks.get(track?)?;
            // nearest clip edge within the snap distance, else the half of the clip the pointer is over
            let mut best: Option<(f64, Id, bool)> = None;
            for c in &tr.clips {
                for (x, out) in [(c.start, false), (c.end(), true)] {
                    let d = (x - t).abs();
                    if d <= thr && best.is_none_or(|(b, _, _)| d < b) {
                        best = Some((d, c.id, out));
                    }
                }
            }
            match best {
                Some((_, clip, out)) => Some(Aim::Edge { clip, out }),
                None => {
                    let c = tr.clips.iter().find(|c| c.contains(t))?;
                    Some(Aim::Edge { clip: c.id, out: crate::ui::transitions_ui::drop_at_end(c, t) })
                }
            }
        }
        DropKind::OnAudio => match track {
            Some(ti) if project.tracks.get(ti)?.kind == TrackKind::Audio => Some(Aim::Place { track: Some(ti), t }),
            Some(_) => None,
            None if below => Some(Aim::Place { track: None, t }),
            None => None,
        },
        DropKind::OnTrack => match track {
            // (a picture clip goes on a video row)
            Some(ti) if project.tracks.get(ti)?.kind == TrackKind::Video => Some(Aim::Place { track: Some(ti), t }),
            Some(_) => None,
            None if below => Some(Aim::Place { track: None, t }),
            None => None,
        },
    }
}

/// Where the lane for a NEW track would start (just under the last row), for the ghost.
pub(crate) fn row_end(state: &TimelineState, p: &Project) -> f32 {
    let mut top = state.lanes_rect.top() - state.scroll_y;
    for i in paint::row_order(p, state.show_kinds) {
        top += p.tracks[i].height;
    }
    top
}

/// Highlight what `aim` means: the clips it applies to, the cut it goes on, or a ghost clip of
/// `len` seconds (a dashed outline in the new-track lane when there is no track yet).
pub(crate) fn paint(lp: &egui::Painter, pal: &Palette, state: &TimelineState, p: &Project, aim: &Aim, len: f64) {
    let fill = pal.selection.gamma_multiply(0.35);
    let stroke = Stroke::new(2.0, pal.selection);
    let clip_rect = |ti: usize, c: &Clip| {
        let top = paint::row_top(state, p, ti)?;
        Some(Rect::from_min_max(
            pos2(state.x_at(c.start), top + 1.0),
            pos2(state.x_at(c.end()), top + p.tracks[ti].height - 1.0),
        ))
    };
    match aim {
        Aim::Clips { clips, .. } => {
            for &id in clips {
                if let Some(r) = p.track_of(id).zip(p.clip(id)).and_then(|(ti, c)| clip_rect(ti, c)) {
                    lp.rect_filled(r, 0, fill);
                    lp.rect_stroke(r, 0, stroke, StrokeKind::Inside);
                }
            }
        }
        Aim::Edge { clip, out } => {
            let Some((ti, c)) = p.track_of(*clip).zip(p.clip(*clip)) else { return };
            let Some(r) = clip_rect(ti, c) else { return };
            let x = if *out { r.right() } else { r.left() };
            let bar = Rect::from_min_max(pos2(x - 6.0, r.top()), pos2(x + 6.0, r.bottom()));
            lp.rect_filled(bar, 0, fill);
            lp.rect_stroke(bar, 0, stroke, StrokeKind::Inside);
        }
        Aim::Place { track, t } => {
            let (top, h) = match track {
                Some(ti) => match paint::row_top(state, p, *ti) {
                    Some(top) => (top, p.tracks[*ti].height),
                    None => return,
                },
                None => (row_end(state, p), 40.0),
            };
            let ghost = Rect::from_min_max(pos2(state.x_at(*t), top + 1.0), pos2(state.x_at(*t + len), top + h - 1.0));
            lp.rect_filled(ghost, 0, pal.selection.gamma_multiply(0.3));
            lp.rect_stroke(ghost, 0, Stroke::new(1.0, pal.selection), StrokeKind::Inside);
            if track.is_none() {
                lp.text(
                    ghost.left_top() + vec2(4.0, 2.0),
                    egui::Align2::LEFT_TOP,
                    "New track",
                    egui::FontId::proportional(10.0),
                    pal.text,
                );
            }
        }
    }
}

/// Pull freshly placed clips onto video track `target` (`paste_clips`'s own rule: only if the whole group
/// fits there), or - `None` - onto a track of their own (a NEW one when they share one). Returns whether they ended up there.
pub(crate) fn pull_to_track(p: &mut Project, ids: &[Id], target: Option<usize>) -> bool {
    let ti = match target {
        Some(ti) if ti < p.tracks.len() => ti,
        Some(_) => return false,
        None => {
            let rows: std::collections::HashSet<usize> = ids.iter().filter_map(|&id| p.track_of(id)).collect();
            // already alone on a track of its own (`place_clips` made one, or reused an empty one): done
            if rows.iter().all(|&ti| p.tracks[ti].clips.iter().all(|c| ids.contains(&c.id))) {
                return !rows.is_empty();
            }
            // a clip on one shared track gets a new track; a group spread over several already got fresh ones
            if rows.len() != 1 {
                return false;
            }
            p.add_track(TrackKind::Video)
        }
    };
    let kind = p.tracks[ti].kind;
    let list = if kind == TrackKind::Video { p.video_tracks() } else { p.audio_tracks() };
    let row = |t: usize| list.iter().position(|&x| x == t);
    let from = ids.iter().filter_map(|&id| p.track_of(id)).filter_map(row).min();
    match (row(ti), from) {
        (Some(to), Some(from)) if to != from => p.move_clips(ids, 0.0, to as i32 - from as i32, Some(kind)),
        (Some(_), Some(_)) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EffectKind;

    /// Video track 0: A 0-4, B 4-8, C 10-14; audio track 1: X 0-5. Ids are plain numbers.
    fn project() -> (Project, [Id; 4]) {
        let mut p = Project::new();
        let mut add = |ti: usize, kind, id, start, dur| {
            p.tracks[ti].clips.push(Clip::new(id, kind, "c", start, dur));
            id
        };
        let a = add(0, ClipKind::Video, 11, 0.0, 4.0);
        let b = add(0, ClipKind::Video, 12, 4.0, 4.0);
        let c = add(0, ClipKind::Video, 13, 10.0, 4.0);
        let x = add(1, ClipKind::Audio, 14, 0.0, 5.0);
        (p, [a, b, c, x])
    }

    fn look() -> DragPayload {
        DragPayload::GalleryCard(GalleryTab::Looks, "Warm".into())
    }

    #[test]
    fn every_payload_has_one_kind_of_target() {
        use crate::model::TransitionKind;
        let lib = |id: &str| DragPayload::LibItem(id.into());
        let table: [(DragPayload, Option<DropKind>); 15] = [
            (DragPayload::Effect(EffectKind::Blur), Some(DropKind::OnClip)),
            (DragPayload::KeyBlock("Fade In".into()), Some(DropKind::OnClip)),
            (look(), Some(DropKind::OnClip)),
            (DragPayload::GalleryCard(GalleryTab::Luts, "a.cube".into()), Some(DropKind::OnClip)),
            (DragPayload::GalleryCard(GalleryTab::Captions, "Bold".into()), Some(DropKind::OnClip)),
            (DragPayload::GalleryCard(GalleryTab::SpeedRamps, "Ease".into()), Some(DropKind::OnClip)),
            (DragPayload::Transition(TransitionKind::Wipe), Some(DropKind::OnEdge)),
            (DragPayload::Template("T".into()), Some(DropKind::OnTrack)),
            (DragPayload::GalleryCard(GalleryTab::Titles, "Big".into()), Some(DropKind::OnTrack)),
            (lib("effects/glow"), Some(DropKind::OnClip)),
            (lib("luts/warm"), Some(DropKind::OnClip)),
            (lib("transitions/zoom"), Some(DropKind::OnEdge)),
            (lib("html/card"), Some(DropKind::OnTrack)),
            (lib("skills/tidy"), None),
            (DragPayload::Asset(1), None),
        ];
        for (p, want) in table {
            assert_eq!(drop_kind(&p), want, "{p:?}");
        }
    }

    /// A clip body takes the card; with several selected and the one under the pointer among them, all of
    /// them do - otherwise only the one under the pointer.
    #[test]
    fn a_clip_drop_applies_to_the_hit_clip_or_the_whole_selection() {
        let (p, [a, b, c, _]) = project();
        let at = |sel: &[Id], t: f64| aim(&p, sel, &look(), t, Some(0), false, 0.1);
        assert_eq!(at(&[], 2.0), Some(Aim::Clips { clips: vec![a], hit: a, t: 2.0 }));
        assert_eq!(at(&[a, b], 2.0), Some(Aim::Clips { clips: vec![a, b], hit: a, t: 2.0 }), "a is selected: all");
        assert_eq!(at(&[a, b], 11.0), Some(Aim::Clips { clips: vec![c], hit: c, t: 11.0 }), "c is not: just c");
        assert_eq!(at(&[a], 2.0), Some(Aim::Clips { clips: vec![a], hit: a, t: 2.0 }), "one selected: itself");
        assert_eq!(at(&[], 9.0), None, "the gap between B and C is no clip");
        assert_eq!(aim(&p, &[], &look(), 2.0, None, false, 0.1), None, "not over a track");
    }

    #[test]
    fn a_look_or_effect_does_not_land_on_audio() {
        let (p, [a, b, _, x]) = project();
        assert_eq!(aim(&p, &[], &look(), 2.0, Some(1), false, 0.1), None, "a look needs picture");
        let fx = DragPayload::Effect(EffectKind::Blur);
        assert_eq!(aim(&p, &[], &fx, 2.0, Some(1), false, 0.1), None, "a pixel effect needs picture");
        let cap = DragPayload::GalleryCard(GalleryTab::Captions, "Bold".into());
        assert!(aim(&p, &[], &cap, 2.0, Some(1), false, 0.1).is_some(), "a caption style is project-wide");
        // selected audio does not join a look's targets
        let got = aim(&p, &[a, b, x], &look(), 2.0, Some(0), false, 0.1);
        assert_eq!(got, Some(Aim::Clips { clips: vec![a, b], hit: a, t: 2.0 }));
    }

    /// A transition goes on the edge nearest the pointer within the snap distance, else on the half of the
    /// clip it is over; the gap away from any edge takes none.
    #[test]
    fn a_transition_snaps_to_the_nearest_edge() {
        let (p, [a, b, c, _]) = project();
        let tr = DragPayload::Transition(crate::model::TransitionKind::CrossFade);
        let at = |t: f64| aim(&p, &[], &tr, t, Some(0), false, 0.2);
        assert_eq!(at(3.9), Some(Aim::Edge { clip: a, out: true }), "the A | B cut, from A's side");
        assert_eq!(at(4.15), Some(Aim::Edge { clip: a, out: true }), "(A's end and B's start are one cut)");
        assert_eq!(at(0.1), Some(Aim::Edge { clip: a, out: false }), "A's start: a fade-in");
        assert_eq!(at(2.0), Some(Aim::Edge { clip: a, out: false }), "no edge near: the left half is its start");
        assert_eq!(at(3.0), Some(Aim::Edge { clip: a, out: true }), "...the right half its end");
        assert_eq!(at(9.9), Some(Aim::Edge { clip: c, out: false }), "snaps across the gap to C's start");
        assert_eq!(at(8.1), Some(Aim::Edge { clip: b, out: true }), "and to B's end");
        assert_eq!(at(9.0), None, "mid-gap, away from every edge");
        assert_eq!(aim(&p, &[], &tr, 2.0, None, false, 0.2), None);
    }

    /// Empty video track: a place target; below the last track: a new one; an audio row: nothing.
    #[test]
    fn a_placing_drop_wants_a_video_track_or_the_new_track_lane() {
        let (p, _) = project();
        let tpl = DragPayload::Template("T".into());
        assert_eq!(aim(&p, &[], &tpl, 9.0, Some(0), false, 0.1), Some(Aim::Place { track: Some(0), t: 9.0 }));
        assert_eq!(aim(&p, &[], &tpl, 9.0, Some(1), false, 0.1), None, "an audio row takes no picture");
        assert_eq!(aim(&p, &[], &tpl, 3.0, None, true, 0.1), Some(Aim::Place { track: None, t: 3.0 }));
        assert_eq!(aim(&p, &[], &tpl, 3.0, None, false, 0.1), None, "the ruler or the header: nothing");
        let skill = DragPayload::LibItem("skills/tidy".into());
        assert_eq!(aim(&p, &[], &skill, 9.0, Some(0), false, 0.1), None);
    }

    /// Your own effects / transitions / sounds ride the same resolver as the built-ins.
    #[test]
    fn saved_effects_transitions_and_sounds_resolve_like_the_built_ins() {
        let (p, [a, _, _, x]) = project();
        let fx = DragPayload::GalleryCard(GalleryTab::Effects, "My blur".into());
        assert_eq!(drop_kind(&fx), Some(DropKind::OnClip));
        assert_eq!(aim(&p, &[], &fx, 2.0, Some(0), false, 0.1), Some(Aim::Clips { clips: vec![a], hit: a, t: 2.0 }));
        assert_eq!(aim(&p, &[], &fx, 2.0, Some(1), false, 0.1), None, "not on audio");
        let tr = DragPayload::GalleryCard(GalleryTab::Transitions, "My wipe".into());
        assert_eq!(drop_kind(&tr), Some(DropKind::OnEdge));
        assert_eq!(aim(&p, &[], &tr, 3.9, Some(0), false, 0.2), Some(Aim::Edge { clip: a, out: true }));
        // a sound wants an audio row, or the lane below the last track (a new audio track)
        let snd = DragPayload::GalleryCard(GalleryTab::Sounds, "Boom".into());
        assert_eq!(drop_kind(&snd), Some(DropKind::OnAudio));
        assert_eq!(aim(&p, &[], &snd, 6.0, Some(1), false, 0.1), Some(Aim::Place { track: Some(1), t: 6.0 }));
        assert_eq!(aim(&p, &[], &snd, 6.0, Some(0), false, 0.1), None, "a video row takes no sound");
        assert_eq!(aim(&p, &[], &snd, 6.0, None, true, 0.1), Some(Aim::Place { track: None, t: 6.0 }));
        assert_eq!(aim(&p, &[], &snd, 6.0, None, false, 0.1), None);
        let _ = x;
    }

    #[test]
    fn placed_clips_are_pulled_onto_the_target_track() {
        let (mut p, _) = project();
        // a one-clip group placed on the first free video track, then pulled onto a NEW one
        let id = 900;
        p.tracks[0].clips.push(Clip::new(id, ClipKind::Video, "t", 20.0, 3.0));
        let before = p.video_tracks().len();
        assert!(pull_to_track(&mut p, &[id], None));
        assert_eq!(p.video_tracks().len(), before + 1, "a new video track");
        let ti = p.track_of(id).unwrap();
        assert_eq!(p.tracks[ti].kind, TrackKind::Video);
        assert!(ti != 0 && p.tracks[ti].clips.len() == 1);
        // a clip already alone on its track keeps it: no extra empty track is made
        let lone = 960;
        let own = p.add_track(TrackKind::Video);
        p.tracks[own].clips.push(Clip::new(lone, ClipKind::Video, "lone", 50.0, 2.0));
        let count = p.tracks.len();
        assert!(pull_to_track(&mut p, &[lone], None));
        assert_eq!((p.tracks.len(), p.track_of(lone)), (count, Some(own)));
        p.tracks[own].clips.clear();
        // a group spread over two tracks keeps the tracks it was placed on: no extra empty track is made
        let (a, b) = (950, 951);
        p.tracks[0].clips.push(Clip::new(a, ClipKind::Video, "g1", 40.0, 2.0));
        let upper = p.add_track(TrackKind::Video);
        p.tracks[upper].clips.push(Clip::new(b, ClipKind::Video, "g2", 40.0, 2.0));
        let count = p.tracks.len();
        assert!(!pull_to_track(&mut p, &[a, b], None));
        assert_eq!(p.tracks.len(), count);
        p.tracks[0].clips.retain(|c| c.id != a);
        p.tracks[upper].clips.retain(|c| c.id != b);
        // onto track 0 where it fits...
        assert!(pull_to_track(&mut p, &[id], Some(0)));
        assert_eq!(p.track_of(id), Some(0));
        // ...but not where something already is
        p.tracks[0].clips.push(Clip::new(901, ClipKind::Video, "x", 30.0, 3.0));
        let lane = p.add_track(TrackKind::Video);
        p.tracks[lane].clips.push(Clip::new(902, ClipKind::Video, "y", 30.0, 3.0));
        assert!(!pull_to_track(&mut p, &[901], Some(lane)), "blocked: nothing moves");
        assert_eq!(p.track_of(901), Some(0));
    }
}
