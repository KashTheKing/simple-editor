//! ---- ws:source-monitor ----
//! ACT_HANDLERS entry for the Source monitor: Match Frame / Reveal in Library, the four smart edits,
//! Source Tape, Show/Hide Source, and transport-focus routing (Space/K/step/Home/End/I/O/Alt+X drive
//! the source player while it holds focus; J/L do the same via `playback_ctl`'s one-line hand-off).
//! The edits themselves are plain fns over `Project` (testable without a live `App`) composed from
//! trim-model's ranged ops — `splice_in`/`extract_range`/`overwrite_asset`/`replace_clip` are called
//! directly, never through a second MCP tool name.

use super::edit_ops::{place, DropMode};
use super::*;
use crate::ui::source_ui::SourceState;

/// Smart-indicator reach in UI pixels (converted to seconds with the timeline zoom by the caller).
pub(super) const SMART_PX: f64 = 12.0;

/// Mark In at `t`; an out mark at/before it is dropped (same rule as the timeline's `MarkIn`).
pub(crate) fn mark_in(st: &mut SourceState, t: f64) {
    st.src_in = Some(t);
    if st.src_out.is_some_and(|o| o <= t) {
        st.src_out = None;
    }
}
pub(crate) fn mark_out(st: &mut SourceState, t: f64) {
    st.src_out = Some(t);
    if st.src_in.is_some_and(|i| i >= t) {
        st.src_in = None;
    }
}

/// Match Frame: the clip (`clip`, else the first under `playhead`, visual clips first) and the source
/// time it shows at `playhead` — `Clip::src_time` (speed/reverse/freeze applied), clamped to the asset.
pub(crate) fn match_frame_target(project: &Project, clip: Option<Id>, playhead: f64) -> Option<(Id, f64)> {
    let id = clip.or_else(|| {
        let at = project.clips_at(playhead);
        at.iter().copied().find(|&id| project.clip(id).is_some_and(|c| c.is_visual())).or(at.first().copied())
    })?;
    let c = project.clip(id)?;
    if !c.uses_asset() {
        return None;
    }
    let a = project.asset(c.asset)?;
    let t = if c.contains(playhead) { c.src_time(playhead) } else { c.src_in };
    Some((c.asset, t.clamp(0.0, a.duration.max(0.0))))
}

/// Append at End: the marked range goes after the last clip of the whole timeline, on the first
/// track of the asset's own kind that is free there (`insert_asset_clips_ranged`'s rule).
pub(crate) fn append_at_end(project: &mut Project, asset: Id, range: Option<(f64, f64)>) -> Vec<Id> {
    let at = project.duration();
    place(project, asset, at, None, DropMode::Place, range)
}

/// Ripple Overwrite (Resolve's smart edit): the clip under `at` on the target track (given, else
/// the first track of the asset's kind) is extracted (ripple tracks) and the marked range spliced in
/// at its start, so a longer/shorter source ripples everything after it by the difference. No clip
/// under `at` = a plain splice there.
pub(crate) fn ripple_overwrite(
    project: &mut Project,
    asset: Id,
    at: f64,
    track: Option<usize>,
    range: Option<(f64, f64)>,
) -> Vec<Id> {
    let Some(a) = project.asset(asset) else { return Vec::new() };
    let ti = track.or_else(|| {
        if a.has_video() {
            project.video_tracks().first().copied()
        } else {
            project.audio_tracks().first().copied()
        }
    });
    let under = ti
        .and_then(|ti| project.tracks.get(ti))
        .and_then(|t| t.clips.iter().find(|c| c.contains(at)))
        .map(|c| (c.start, c.end()));
    let start = match under {
        Some((s, e)) => {
            project.extract_range(s, e, None);
            s
        }
        None => at,
    };
    project.splice_in(asset, start, ti, range)
}

/// Close Up: close the gap under `t` on `track` only (`[previous clip's end, next clip's start)`),
/// pulling that one track's later clips left — never the other tracks. Returns the closed gap.
pub(crate) fn close_up(project: &mut Project, track: usize, t: f64) -> Option<(f64, f64)> {
    let tr = project.tracks.get(track)?;
    if tr.clips.iter().any(|c| c.contains(t)) {
        return None;
    }
    let a = tr.clips.iter().filter(|c| c.end() <= t + 1e-6).map(|c| c.end()).fold(0.0_f64, f64::max);
    let b = tr.clips.iter().filter(|c| c.start >= t - 1e-6).map(|c| c.start).fold(f64::INFINITY, f64::min);
    if !b.is_finite() || b <= a + 1e-6 {
        return None;
    }
    project.extract_range(a, b, Some(&[track]));
    Some((a, b))
}

/// The nearest timeline cut to `playhead` within `thr` seconds, as a signed offset (cut − playhead)
/// — the smart-edit row's "which cut will this land on" readout. Pure read, recomputed per frame.
pub(crate) fn smart_indicator(project: &Project, playhead: f64, thr: f64) -> Option<f64> {
    project
        .cut_points()
        .into_iter()
        .map(|c| c - playhead)
        .filter(|d| d.abs() <= thr)
        .min_by(|a, b| a.abs().total_cmp(&b.abs()))
}

/// Source Tape: `assets` (library ids, in bin order) laid end to end on V1/A1 of a fresh project at
/// the parent's format. Second value = each asset's start offset (the cut ticks). Unknown ids are
/// skipped — the caller filters first so its own order list stays aligned.
pub(crate) fn source_tape(assets: &[Id], project: &Project) -> (Project, Vec<f64>) {
    let mut p = Project::new();
    p.name = "Source Tape".into();
    p.width = project.width;
    p.height = project.height;
    p.fps = project.fps;
    let mut offsets = Vec::with_capacity(assets.len());
    let mut t = 0.0;
    for &id in assets {
        let Some(a) = project.asset(id) else { continue };
        let mut a = a.clone();
        a.parent = None; // a subclip plays as its own file here (ponytail: `range` isn't applied yet)
        let aid = p.add_asset(a);
        let ids = p.insert_asset_clips(aid, t, Some(0));
        offsets.push(t);
        t = ids.first().and_then(|&c| p.clip(c)).map(|c| c.end()).unwrap_or(t);
    }
    (p, offsets)
}

/// Snapshot, run `f`, and push one labelled undo iff the project actually changed; a refused edit
/// (Err) toasts its reason and pushes nothing. New clips become the selection.
fn edit(app: &mut App, label: &'static str, f: impl FnOnce(&mut App) -> Result<Vec<Id>, String>) {
    let before = app.project.to_json();
    match f(app) {
        Ok(ids) => {
            if app.project.to_json() != before {
                app.push_undo_labeled(before, label);
                app.after_edit();
                if !ids.is_empty() {
                    app.selection = ids;
                }
            }
        }
        Err(e) => app.toast(e),
    }
}

impl App {
    /// The library asset + source range a three-point edit places (see `SourceState::three_point`).
    fn three_point(&self) -> Result<(Id, Option<(f64, f64)>), String> {
        let st = self.source.as_ref().ok_or("Nothing is open in the Source monitor")?;
        st.three_point(&self.project).ok_or_else(|| "Open a library clip in the Source monitor first".to_string())
    }

    /// Insert the open source's marked range: `place|splice|overwrite|top|append` at `at` (default:
    /// the playhead). Project-only — the caller owns the undo step (`edit` here, the Mutate wrapper
    /// for the MCP tool).
    pub(crate) fn source_insert(&mut self, mode: &str, at: Option<f64>, track: Option<usize>) -> Result<Vec<Id>, String> {
        let (asset, range) = self.three_point()?;
        let at = at.unwrap_or(self.playhead);
        let mode = match mode {
            "place" => DropMode::Place,
            "splice" => DropMode::Splice,
            "overwrite" => DropMode::Overwrite,
            "top" => DropMode::OnTop,
            "append" => return Ok(append_at_end(&mut self.project, asset, range)),
            _ => return Err("mode: place|splice|overwrite|top|append".into()),
        };
        Ok(self.place_asset(asset, at, track, mode, range))
    }

    /// One of the four smart edits at the playhead: `append|ripple_overwrite|close_up|place_on_top`.
    /// Project-only, like `source_insert`.
    pub(crate) fn smart_edit(&mut self, kind: &str) -> Result<Vec<Id>, String> {
        match kind {
            "close_up" => {
                // the track under the pointer: the last lane pressed, else the selection's, else V1
                let track = self
                    .timeline
                    .last_track
                    .or_else(|| self.selection.first().and_then(|&id| self.project.track_of(id)))
                    .or_else(|| self.project.video_tracks().first().copied())
                    .ok_or("No track to close up on")?;
                close_up(&mut self.project, track, self.playhead).ok_or("No gap under the playhead on that track")?;
                Ok(Vec::new())
            }
            "append" => self.source_insert("append", None, None),
            "place_on_top" => self.source_insert("top", None, None),
            "ripple_overwrite" => {
                let (asset, range) = self.three_point()?;
                Ok(ripple_overwrite(&mut self.project, asset, self.playhead, None, range))
            }
            _ => Err("kind: append|ripple_overwrite|close_up|place_on_top".into()),
        }
    }

    /// Match Frame: open the clip's asset (given, else the selected clip under the playhead, else any
    /// clip under it) in the Source monitor at the source time showing on the timeline. False = no
    /// clip / not a media clip.
    pub(crate) fn match_frame(&mut self, clip: Option<Id>) -> bool {
        let ph = self.playhead;
        let clip = clip.or_else(|| {
            self.selection.first().copied().filter(|&id| self.project.clip(id).is_some_and(|c| c.contains(ph)))
        });
        let Some((asset, t)) = match_frame_target(&self.project, clip, ph) else { return false };
        self.open_asset_in_source(asset, Some(t))
    }

    /// New library subclip from the source marks (unmarked = the whole clip).
    pub(crate) fn source_subclip(&mut self, name: Option<String>) {
        let Some(st) = self.source.as_ref() else { return self.toast("Nothing is open in the Source monitor") };
        let Some(asset) = st.asset.filter(|_| st.tape.is_none()) else {
            return self.toast("Subclips need an imported library clip (not a tape or an unimported file)");
        };
        let (a, b) = st.marks().unwrap_or((0.0, st.duration));
        let mut made = None;
        edit(self, "New subclip", |app| {
            made = app.project.subclip_from_marks(asset, a, b, name);
            made.map(|_| Vec::new()).ok_or_else(|| "Could not make a subclip from those marks".to_string())
        });
        if let Some(id) = made {
            self.library.selected = Some(id);
            self.library.tab = 0;
            self.toast("Subclip added to the Library");
        }
    }

    /// J/L for the source player (`playback_ctl::shuttle_rate`'s ladder over its own rate).
    pub(crate) fn source_shuttle(&mut self, back: bool) {
        self.player.pause();
        let Some(s) = self.source.as_mut() else { return };
        let cur = if s.player.is_playing() { s.player.rate() } else { 0.0 };
        let rate = super::playback_ctl::shuttle_rate(cur, back);
        if !s.player.is_playing() {
            s.player.play();
        }
        s.player.set_rate(rate);
    }

    fn toggle_source_tape(&mut self) {
        if self.source.as_ref().is_some_and(|s| s.tape.is_some()) {
            let own = self.source_own_project();
            if let Some(s) = self.source.as_mut() {
                s.set_tape(None, &own);
            }
        } else {
            self.source_pending = Some(super::source_pane::Pending::Tape(Vec::new()));
            self.source_focus = true;
        }
    }
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    use Action::*;
    if app.source_active() {
        // last-clicked transport wins: these drive the source player instead of the timeline
        let handled = match a {
            PlayPause => {
                app.player.pause();
                if let Some(s) = app.source.as_mut() {
                    s.player.toggle();
                }
                true
            }
            Stop => {
                if let Some(s) = app.source.as_mut() {
                    s.player.pause();
                }
                true
            }
            StepBack | StepForward => {
                if let Some(s) = app.source.as_mut() {
                    s.player.step(if a == StepBack { -1 } else { 1 }, s.fps);
                }
                true
            }
            GoStart | GoEnd => {
                if let Some(s) = app.source.as_mut() {
                    s.player.pause();
                    s.player.seek(if a == GoStart { 0.0 } else { s.duration });
                }
                true
            }
            MarkIn | MarkOut | ClearInOut => {
                if let Some(s) = app.source.as_mut() {
                    let t = s.player.time();
                    match a {
                        MarkIn => mark_in(s, t),
                        MarkOut => mark_out(s, t),
                        _ => (s.src_in, s.src_out) = (None, None),
                    }
                }
                true
            }
            _ => false,
        };
        if handled {
            return true;
        }
    }
    match a {
        MatchFrame => {
            if !app.match_frame(None) {
                app.toast("Put the playhead over a media clip to match its frame");
            }
            true
        }
        RevealInLibrary => {
            let asset = app
                .selection
                .first()
                .copied()
                .or_else(|| app.project.clips_at(app.playhead).first().copied())
                .and_then(|id| app.project.clip(id))
                .filter(|c| c.uses_asset())
                .map(|c| c.asset);
            match asset {
                Some(id) => {
                    app.library.selected = Some(id);
                    app.library.tab = 0;
                    app.surface(Pane::Library);
                }
                None => app.toast("Select a media clip first"),
            }
            true
        }
        AppendAtEnd => {
            edit(app, "Append at end", |app| app.smart_edit("append"));
            true
        }
        RippleOverwrite => {
            edit(app, "Ripple overwrite", |app| app.smart_edit("ripple_overwrite"));
            true
        }
        CloseUp => {
            edit(app, "Close up", |app| app.smart_edit("close_up"));
            true
        }
        PlaceOnTop => {
            edit(app, "Place on top", |app| app.smart_edit("place_on_top"));
            true
        }
        SourceTape => {
            app.toggle_source_tape();
            true
        }
        ToggleSource => {
            app.toggle_pane(Pane::Source);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, ClipKind, Effect, EffectKind, TrackKind};

    fn asset(path: &str, dur: f64) -> Asset {
        Asset {
            id: 0,
            path: path.into(),
            kind: ClipKind::Video,
            duration: dur,
            width: 320,
            height: 240,
            fps: 25.0,
            audio_streams: Vec::new(),
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

    /// A clip trimmed mid-asset (src_in 3 s at t=0..4) — Match Frame must land at src_in + local
    /// time, not 0, and pick the asset the clip actually uses.
    #[test]
    fn match_frame_seeks_source_time() {
        let mut p = Project::from_media(asset("C:/a.mp4", 10.0));
        let other = p.add_asset(asset("C:/b.mp4", 10.0));
        let id = p.tracks[0].clips[0].id;
        let aid = {
            let c = p.clip_mut(id).unwrap();
            c.src_in = 3.0;
            c.duration = 4.0;
            c.asset
        };
        assert_ne!(aid, other);
        assert_eq!(match_frame_target(&p, None, 1.5), Some((aid, 4.5)), "src_in + local time");
        assert_eq!(match_frame_target(&p, Some(id), 0.0), Some((aid, 3.0)));
        assert_eq!(match_frame_target(&p, None, 8.0), None, "nothing under the playhead");
        // a text clip has no source to match
        let t = p.add_text_clip(20.0, 2.0);
        assert_eq!(match_frame_target(&p, Some(t), 21.0), None);
    }

    #[test]
    fn smart_edit_append_at_end_uses_project_duration() {
        let mut p = Project::from_media(asset("C:/a.mp4", 10.0));
        let b = p.add_asset(asset("C:/b.mp4", 3.0));
        let end = p.duration();
        assert_eq!(end, 10.0);
        let ids = append_at_end(&mut p, b, Some((1.0, 2.5)));
        let c = p.clip(ids[0]).unwrap();
        assert_eq!(c.start, end, "placed at the old project end");
        assert_eq!(c.duration, 1.5, "the marked range's length");
        assert_eq!(c.src_in, 1.0);
        assert_eq!(p.track_of(ids[0]), Some(0), "video asset on the video track");
        assert_eq!(p.duration(), 11.5);
    }

    /// Close Up closes the gap on the named track only; a second track with the same gap keeps it.
    #[test]
    fn smart_edit_close_up_extracts_only_under_playhead_track() {
        let mut p = Project::from_media(asset("C:/a.mp4", 4.0));
        let a = p.tracks[0].clips[0].asset;
        p.insert_asset_clips(a, 6.0, Some(0)); // V1: [0,4) gap [6,10)
        let v2 = p.add_track(TrackKind::Video);
        p.insert_asset_clips(a, 0.0, Some(v2));
        p.insert_asset_clips(a, 6.0, Some(v2)); // V2: same layout
        assert_eq!(close_up(&mut p, 0, 5.0), Some((4.0, 6.0)));
        let starts = |ti: usize| p.tracks[ti].clips.iter().map(|c| c.start).collect::<Vec<_>>();
        assert_eq!(starts(0), vec![0.0, 4.0], "V1's later clip pulled left");
        assert_eq!(starts(v2), vec![0.0, 6.0], "V2 untouched");
        assert_eq!(close_up(&mut p, 0, 2.0), None, "not a gap");
        assert_eq!(close_up(&mut p, 0, 20.0), None, "nothing after it to pull");
    }

    #[test]
    fn smart_indicator_finds_nearest_cut() {
        let mut p = Project::from_media(asset("C:/a.mp4", 4.0));
        let a = p.tracks[0].clips[0].asset;
        p.insert_asset_clips(a, 4.0, Some(0)); // cuts at 0, 4, 8
        assert_eq!(smart_indicator(&p, 3.7, 0.5), Some(4.0 - 3.7));
        assert_eq!(smart_indicator(&p, 4.3, 0.5), Some(4.0 - 4.3), "signed: the cut is behind");
        assert_eq!(smart_indicator(&p, 7.9, 0.5), Some(8.0 - 7.9), "the nearer of 4 and 8");
        assert_eq!(smart_indicator(&p, 6.0, 0.5), None, "beyond the threshold");
        assert_eq!(smart_indicator(&p, 4.0, 0.5), Some(0.0), "on the cut");
    }

    #[test]
    fn source_tape_appends_in_filter_order_on_v1() {
        let mut p = Project::new();
        let a = p.add_asset(asset("C:/a.mp4", 4.0));
        let b = p.add_asset(asset("C:/b.mp4", 2.0));
        let c = p.add_asset(asset("C:/c.mp4", 3.0));
        let (tape, offsets) = source_tape(&[c, a, b], &p);
        assert_eq!(offsets, vec![0.0, 3.0, 7.0], "cumulative durations in the given order");
        let clips = &tape.tracks[0].clips;
        assert_eq!(clips.len(), 3, "all on track 0");
        assert_eq!(clips.iter().map(|c| c.start).collect::<Vec<_>>(), offsets);
        let names: Vec<&str> = clips.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["c.mp4", "a.mp4", "b.mp4"]);
        assert_eq!(tape.duration(), 9.0);
        assert!(p.tracks.iter().all(|t| t.clips.is_empty()), "the parent project is untouched");
        assert_eq!(source_tape(&[999], &p).1, Vec::<f64>::new(), "unknown ids are skipped");
    }

    /// Ripple Overwrite replaces the clip under the playhead and ripples by the length difference;
    /// the replaced clip's effects go with it (it's a splice, not a replace edit).
    #[test]
    fn ripple_overwrite_replaces_under_playhead_and_ripples() {
        let mut p = Project::from_media(asset("C:/a.mp4", 4.0));
        let a = p.tracks[0].clips[0].asset;
        p.insert_asset_clips(a, 4.0, Some(0)); // [0,4)[4,8)
        p.tracks[0].clips[0].effects.push(Effect::new(EffectKind::Blur));
        let b = p.add_asset(asset("C:/b.mp4", 2.0));
        let ids = ripple_overwrite(&mut p, b, 1.0, None, None);
        let starts: Vec<(f64, f64)> = p.tracks[0].clips.iter().map(|c| (c.start, c.duration)).collect();
        assert_eq!(starts, vec![(0.0, 2.0), (2.0, 4.0)], "2 s source replaced a 4 s clip; the rest rippled left");
        assert_eq!(p.clip(ids[0]).unwrap().asset, b);
        assert!(p.tracks[0].clips[0].effects.is_empty());
        // no clip under the playhead: a plain splice there
        let ids = ripple_overwrite(&mut p, b, 6.0, None, None);
        assert_eq!(p.clip(ids[0]).unwrap().start, 6.0);
    }
}
