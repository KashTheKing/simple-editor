//! ---- ws:registries-schema-hooks ----
//! `App::place_asset` / `DropMode`: the one placement funnel every drop, library "add", recording
//! import and three-point edit goes through. Wave 0b pre-declared the shape as a Place-only stub;
//! ---- ws:source-monitor ---- fills it: `place` maps each `DropMode` onto trim-model's ranged ops
//! (`insert_asset_clips_ranged` / `splice_in` / `overwrite_asset` + `replace_clip` / a fresh top
//! track), and `place_many` is the old `App::insert_at` chaining loop (deleted) over it.

use super::*;

/// How a drop/place interacts with what's already on the track - the drop-modifier table's rows
/// (plain = Place, Ctrl = Splice, Alt = Overwrite, Shift = OnTop).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DropMode {
    /// On the first free track of each kind at `at` (today's plain drop), never overlapping.
    Place,
    /// Insert edit: ripple tracks open by the clip's length first.
    Splice,
    /// Overwrite edit; ON A CLIP BODY it is a Replace edit (`Project::replace_clip` - duration,
    /// effects and transform of the clip under the drop point are kept), elsewhere `overwrite_asset`.
    Overwrite,
    /// On a brand-new track above every existing track of that kind.
    OnTop,
}

impl DropMode {
    /// The drop-modifier table: Ctrl = Splice, Alt = Overwrite, Shift = Place on Top, none = Place.
    pub(crate) fn from_modifiers(m: egui::Modifiers) -> Self {
        if m.ctrl || m.command {
            DropMode::Splice
        } else if m.alt {
            DropMode::Overwrite
        } else if m.shift {
            DropMode::OnTop
        } else {
            DropMode::Place
        }
    }
}

/// Place `asset`'s source `range` (None = the whole file) at timeline time `at`, video on `track` if
/// given, per `mode`. Returns the new clip ids (for a Replace edit: the replaced ones). Project-only:
/// callers own the undo step, exactly as they did around the old `insert_at`.
pub(crate) fn place(
    project: &mut Project,
    asset: Id,
    at: f64,
    track: Option<usize>,
    mode: DropMode,
    range: Option<(f64, f64)>,
) -> Vec<Id> {
    // `track` is one raw index of either kind (see the doc comment above); `splice_in`/`overwrite_asset`
    // (ws:timeline-trim-gestures) want it split into separate video/audio slots - resolve by the
    // track's actual kind rather than assuming, since Overwrite intentionally receives whichever kind
    // is under the pointer (ws:source-monitor's own audio-row fix) while Place/Splice/OnTop only ever
    // see a video-track index (pre-filtered by the caller).
    let split_track = |project: &Project, t: Option<usize>| -> (Option<usize>, Option<usize>) {
        match t.and_then(|ti| project.tracks.get(ti).map(|tr| (ti, tr.kind))) {
            Some((ti, TrackKind::Audio)) => (None, Some(ti)),
            Some((ti, _)) => (Some(ti), None),
            None => (None, None),
        }
    };
    match mode {
        DropMode::Place => project.insert_asset_clips_ranged(asset, at, track, None, range),
        DropMode::Splice => {
            let (vt, at_) = split_track(project, track);
            project.splice_in(asset, at, vt, at_, range)
        }
        DropMode::Overwrite => {
            let Some(a) = project.asset(asset) else { return Vec::new() };
            // the drop point on a clip body of the target track (given, else the first track of the
            // asset's own kind): a Replace edit keeping that clip's duration/effects/transform
            let ti = track.or_else(|| {
                if a.has_video() {
                    project.video_tracks().first().copied()
                } else {
                    project.audio_tracks().first().copied()
                }
            });
            let hit = ti
                .and_then(|ti| project.tracks.get(ti))
                .and_then(|t| t.clips.iter().find(|c| c.contains(at) && c.uses_asset()))
                .map(|c| c.id);
            match hit {
                Some(id) => {
                    let has_audio = !a.audio_streams.is_empty();
                    let has_video = a.has_video();
                    let mut done = Vec::new();
                    for cid in project.expand_links(&[id]) {
                        let Some(c) = project.clip(cid) else { continue };
                        let fits = if c.kind == ClipKind::Audio { has_audio } else { has_video };
                        if fits && project.replace_clip(cid, asset) {
                            // `replace_clip` always zeroes `src_in` - a three-point edit's marked
                            // in-point (`range`) must still apply on this clip-body-hit path, same
                            // as the gap-hit `overwrite_asset` path below already honours it.
                            if let Some(c) = project.clip_mut(cid) {
                                c.src_in = range.map(|r| r.0).unwrap_or(0.0);
                            }
                            done.push(cid);
                        }
                    }
                    done
                }
                None => {
                    let (vt, at_) = split_track(project, track);
                    project.overwrite_asset(asset, at, vt, at_, range)
                }
            }
        }
        DropMode::OnTop => {
            let Some(a) = project.asset(asset).cloned() else { return Vec::new() };
            // a fresh video track lands before the audio tracks (shifting their indices), so make it
            // first and resolve the audio track after
            let vt = a.has_video().then(|| project.add_track(TrackKind::Video));
            let at_ = (!a.audio_streams.is_empty()).then(|| project.add_track(TrackKind::Audio));
            project.insert_asset_clips_ranged(asset, at, vt, at_, range)
        }
    }
}

/// The old `App::insert_at` loop verbatim over `place`: each asset lands at `at`, then `at` advances
/// to that asset's first new clip's end - a library multi-select or a multi-stream import chains end
/// to end. Every former `insert_at` call site routes here.
pub(crate) fn place_many(
    project: &mut Project,
    ids: &[Id],
    mut at: f64,
    track: Option<usize>,
    mode: DropMode,
) -> Vec<Id> {
    let mut out = Vec::new();
    for &id in ids {
        let new = place(project, id, at, track, mode, None);
        if let Some(c) = new.first().and_then(|c| project.clip(*c)) {
            at = c.end();
        }
        out.extend(new);
    }
    out
}

impl App {
    /// Single-asset placement (see `place`). `range` = the source in/out a three-point edit carries
    /// (None = the whole file) - one arg more than the plan's signature, because `source.insert` /
    /// `timeline.place` have a marked range to pass and a second rangeless entry point would be a
    /// duplicate.
    pub(crate) fn place_asset(
        &mut self,
        asset: Id,
        at: f64,
        track: Option<usize>,
        mode: DropMode,
        range: Option<(f64, f64)>,
    ) -> Vec<Id> {
        place(&mut self.project, asset, at, track, mode, range)
    }
    /// Multi-asset placement, chained end to end (see `place_many`).
    pub(crate) fn place_assets(&mut self, ids: &[Id], at: f64, track: Option<usize>, mode: DropMode) -> Vec<Id> {
        place_many(&mut self.project, ids, at, track, mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, AudioStreamInfo, Effect, EffectKind};

    fn asset(path: &str, dur: f64, streams: usize) -> Asset {
        Asset {
            id: 0,
            path: path.into(),
            kind: ClipKind::Video,
            duration: dur,
            width: 320,
            height: 240,
            fps: 25.0,
            audio_streams: (0..streams).map(|i| AudioStreamInfo { index: i, ..Default::default() }).collect(),
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
        }
    }

    /// An audio-only asset (no video track), for Overwrite-onto-an-audio-track regression coverage.
    fn audio_asset(path: &str, dur: f64) -> Asset {
        Asset {
            id: 0,
            path: path.into(),
            kind: ClipKind::Audio,
            duration: dur,
            width: 0,
            height: 0,
            fps: 0.0,
            audio_streams: vec![AudioStreamInfo::default()],
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
        }
    }

    /// (track, start, duration, asset, kind) of every clip - the layout a placement produced.
    fn layout(p: &Project) -> Vec<(usize, f64, f64, Id, ClipKind)> {
        p.all_clips().map(|(ti, c)| (ti, c.start, c.duration, c.asset, c.kind)).collect()
    }

    /// The old `App::insert_at` loop (`insert_asset_clips` per id, `t = first new clip's end`), kept
    /// here verbatim as the regression oracle for `place_many(.., DropMode::Place)`.
    fn old_insert_at(p: &mut Project, ids: &[Id], mut t: f64, vt: Option<usize>) {
        for &id in ids {
            let new = p.insert_asset_clips(id, t, vt);
            if let Some(c) = new.first().and_then(|c| p.clip(*c)) {
                t = c.end();
            }
        }
    }

    #[test]
    fn place_asset_place_matches_insert_at() {
        // a multi-stream asset: video + two audio streams -> V1, A1, A2
        let mut a = Project::new();
        let id = a.add_asset(asset("C:/x.mp4", 6.0, 2));
        let mut b = a.clone();
        old_insert_at(&mut a, &[id], 1.5, None);
        place_many(&mut b, &[id], 1.5, None, DropMode::Place);
        assert_eq!(layout(&a), layout(&b));
        assert_eq!(a.tracks.len(), 3, "V1 A1 A2");
        assert_eq!(layout(&a).len(), 3);
    }

    #[test]
    fn place_assets_chains_multiple_ids_like_insert_at() {
        let mut a = Project::new();
        let x = a.add_asset(asset("C:/x.mp4", 4.0, 1));
        let y = a.add_asset(asset("C:/y.mp4", 2.0, 1));
        let z = a.add_asset(asset("C:/z.mp4", 3.0, 1));
        let mut b = a.clone();
        old_insert_at(&mut a, &[x, y, z], 1.0, Some(0));
        let ids = place_many(&mut b, &[x, y, z], 1.0, Some(0), DropMode::Place);
        assert_eq!(layout(&a), layout(&b));
        assert_eq!(ids.len(), 6, "every new clip id comes back (video + audio per asset)");
        let starts: Vec<f64> = b.tracks[0].clips.iter().map(|c| c.start).collect();
        assert_eq!(starts, vec![1.0, 5.0, 7.0], "each lands at the previous one's end");
    }

    #[test]
    fn place_asset_splice_ripples_only_flagged_tracks() {
        let mut p = Project::from_media(asset("C:/a.mp4", 4.0, 0)); // V1 (ripple by default)
        let a = p.tracks[0].clips[0].asset;
        let v2 = p.add_track(TrackKind::Video); // V2: ripple=false by default
        p.insert_asset_clips(a, 0.0, Some(v2));
        p.tracks[v2].locked = true;
        let b = p.add_asset(asset("C:/b.mp4", 2.0, 0));
        let ids = place(&mut p, b, 0.0, Some(0), DropMode::Splice, None);
        assert_eq!(p.clip(ids[0]).unwrap().start, 0.0);
        assert_eq!(p.tracks[0].clips.iter().map(|c| c.start).collect::<Vec<_>>(), vec![0.0, 2.0], "V1 rippled");
        assert_eq!(p.tracks[v2].clips[0].start, 0.0, "the locked / non-ripple track did not move");
        // splicing onto a locked track is refused
        assert!(place(&mut p, b, 0.0, Some(v2), DropMode::Splice, None).is_empty());
    }

    #[test]
    fn place_asset_overwrite_on_clip_body_replaces() {
        let mut p = Project::from_media(asset("C:/a.mp4", 10.0, 1)); // V1 + A1, linked
        let target = p.tracks[0].clips[0].id;
        p.clip_mut(target).unwrap().effects.push(Effect::new(EffectKind::Blur));
        p.clip_mut(target).unwrap().x.value = 0.25;
        let b = p.add_asset(asset("C:/b.mp4", 3.0, 1));
        // drop point on the clip body -> replace edit on the video clip AND its linked audio
        let ids = place(&mut p, b, 4.0, None, DropMode::Overwrite, None);
        assert_eq!(ids.len(), 2, "both linked clips swapped");
        let c = p.clip(target).unwrap();
        assert_eq!(c.asset, b);
        assert_eq!(c.duration, 10.0, "duration kept");
        assert_eq!(c.effects.len(), 1, "effects kept");
        assert_eq!(c.x.value, 0.25, "transform kept");
        assert_eq!(p.tracks[0].clips.len(), 1, "no extra clip");
        // drop point in a gap -> a real overwrite edit (new clips there)
        let ids = place(&mut p, b, 12.0, None, DropMode::Overwrite, Some((0.0, 1.0)));
        let c = p.clip(ids[0]).unwrap();
        assert_eq!((c.start, c.duration), (12.0, 1.0));
        assert_eq!(p.tracks[0].clips.len(), 2);
    }

    /// Regression: a three-point Overwrite on a clip body must apply the marked in-point, not just
    /// keep `replace_clip`'s hardcoded `src_in = 0.0`.
    #[test]
    fn place_asset_overwrite_on_clip_body_applies_marked_range() {
        let mut p = Project::from_media(asset("C:/a.mp4", 10.0, 1)); // V1 + A1, linked
        let target = p.tracks[0].clips[0].id;
        let b = p.add_asset(asset("C:/b.mp4", 8.0, 1));
        let ids = place(&mut p, b, 4.0, None, DropMode::Overwrite, Some((2.5, 6.0)));
        assert_eq!(ids.len(), 2, "both linked clips swapped");
        let c = p.clip(target).unwrap();
        assert_eq!(c.asset, b);
        assert_eq!(c.duration, 10.0, "duration still kept (a Replace edit, not a resize)");
        assert_eq!(c.src_in, 2.5, "the marked in-point, not 0.0");
    }

    /// Regression: an Alt-drop (Overwrite) on an audio-only asset must land on the audio track the
    /// pointer is actually over, not always fall back to the first audio track (A1).
    #[test]
    fn place_asset_overwrite_targets_track_under_pointer_not_first_audio_track() {
        let mut p = Project::new(); // V1 (0), A1 (1)
        let a2 = p.add_track(TrackKind::Audio); // A2 (2) - not the first audio track
        let base = p.add_asset(audio_asset("C:/base.wav", 5.0));
        let placed = p.insert_asset_clips_ranged(base, 0.0, None, Some(a2), None);
        let target = placed[0];
        assert_eq!(p.track_of(target), Some(a2), "test setup: a clip body sits on A2");
        let repl = p.add_asset(audio_asset("C:/repl.wav", 5.0));
        // `drops.rs` must pass A2 (the raw track under the pointer) through for Overwrite instead of
        // nulling it because it isn't a video track - `place()` itself already honours whatever
        // track it's given.
        let out = place(&mut p, repl, 1.0, Some(a2), DropMode::Overwrite, None);
        assert_eq!(out, vec![target]);
        assert_eq!(p.track_of(target), Some(a2), "landed on A2, not A1");
        assert_eq!(p.clip(target).unwrap().asset, repl);
    }

    #[test]
    fn place_asset_on_top_adds_new_track() {
        let mut p = Project::from_media(asset("C:/a.mp4", 4.0, 1)); // V1 A1
        let empty_v2 = p.add_track(TrackKind::Video); // free, but OnTop must not reuse it
        let a = p.tracks[0].clips[0].asset;
        let n = p.tracks.len();
        let ids = place(&mut p, a, 0.0, None, DropMode::OnTop, None);
        assert_eq!(p.tracks.len(), n + 2, "a new video AND a new audio track");
        let vt = p.track_of(ids[0]).unwrap();
        assert_eq!(vt, *p.video_tracks().last().unwrap(), "on the topmost video track");
        assert_ne!(vt, empty_v2);
        assert!(p.tracks[empty_v2].clips.is_empty());
        let at = p.track_of(ids[1]).unwrap();
        assert_eq!(at, *p.audio_tracks().last().unwrap());
        // again: another fresh pair, never the one just made
        let ids2 = place(&mut p, a, 0.0, None, DropMode::OnTop, None);
        assert_eq!(p.tracks.len(), n + 4);
        assert_ne!(p.track_of(ids2[0]), Some(vt));
    }

    #[test]
    fn drop_mode_from_modifiers_follows_the_table() {
        use egui::Modifiers;
        assert_eq!(DropMode::from_modifiers(Modifiers::NONE), DropMode::Place);
        assert_eq!(DropMode::from_modifiers(Modifiers::CTRL), DropMode::Splice);
        assert_eq!(DropMode::from_modifiers(Modifiers::ALT), DropMode::Overwrite);
        assert_eq!(DropMode::from_modifiers(Modifiers::SHIFT), DropMode::OnTop);
    }
}
