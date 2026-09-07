//! ---- ws:pro-monitor ----
//! Multicam create/switch: `multicam_make` nests a set of angle clips into a `Sequence` (one video
//! track per angle, offset-compensated so they read in sync), `multicam_switch` splits that nested
//! `Sequence`'s own tracks at a time and toggles which angle's clips are `enabled` from there onward.
//! `multicam_switch` deliberately never touches `Project.editing`/`main_stash` - it reaches the nested
//! `Sequence`'s tracks directly via `sequence_mut`, using `split_tracks_at` (extracted from
//! `Project::split_at` in `editing.rs`) instead of the `open_sequence`/`close_sequence` stash swap: a
//! swap can leak state if interrupted mid-call (panic, early return), while a direct field mutation
//! cannot leave the project half-stashed. See the issue plan's Review trail ("wrong-path") for the
//! full reasoning.

use super::editing::split_tracks_at;
use crate::model::*;

impl Project {
    // ---------- multicam ----------

    /// Nest `ids` (>= 2 asset-backed clips, one per camera angle) into a new `Sequence`, one video track
    /// per angle, offset-compensated first so the angles read frame-aligned once nested: `offsets[i]`
    /// is how far clip `i`'s content leads the sync reference (typically `ids[0]`, e.g. from
    /// `engine::analysis::xcorr_offset`) - its `start` moves EARLIER by that many seconds
    /// (`start -= offset`) before `nest_selection` re-bases the group onto the nested sequence's own
    /// t=0. If that would push any clip's shifted start negative, every shifted start is pushed forward
    /// by the same amount first (relative sync is unaffected - only the group's placement on the timeline
    /// shifts to stay non-negative). `ids.len() != offsets.len()`, fewer than 2 ids, or an unknown id all
    /// return `None` without mutating anything.
    pub fn multicam_make(&mut self, ids: &[Id], offsets: &[f64], name: impl Into<String>) -> Option<Id> {
        if ids.len() < 2 || ids.len() != offsets.len() {
            return None;
        }
        let mut shifted = Vec::with_capacity(ids.len());
        for (&id, &off) in ids.iter().zip(offsets) {
            let c = self.clip(id)?;
            shifted.push((id, c.start - off));
        }
        let min_start = shifted.iter().map(|&(_, s)| s).fold(f64::INFINITY, f64::min);
        let correction = (-min_start).max(0.0); // 0 unless the shift above went negative
        for &(id, s) in &shifted {
            self.clip_mut(id)?.start = s + correction;
        }
        self.nest_selection(ids, name)
    }

    /// Switch a multicam `Sequence` clip's active angle from timeline time `t` onward: maps `t` to the
    /// nested sequence's own local time (`Clip::src_time`, so speed/reverse retime on the outer Sequence
    /// clip itself is respected), splits every one of that sequence's OWN video tracks at that local time
    /// (`split_tracks_at`, never `self.tracks`/`main_stash`), then sets `clip.enabled` on every resulting
    /// clip at or after the split point: `true` on the `angle`-th video track (in track order), `false`
    /// on the others. Clips before the split point keep whatever `enabled` state they already had, so an
    /// earlier switch on the same sequence is never undone. `seq_clip` not a `Sequence` clip, or `angle`
    /// out of range, returns `false` and mutates nothing.
    pub fn multicam_switch(&mut self, seq_clip: Id, t: f64, angle: usize) -> bool {
        let Some(clip) = self.clip(seq_clip) else { return false };
        if clip.kind != ClipKind::Sequence {
            return false;
        }
        let local_t = clip.src_time(t);
        let seq_id = clip.sequence;
        let Project { sequences, next_id, .. } = self;
        let Some(seq) = sequences.iter_mut().find(|s| s.id == seq_id) else { return false };
        let video_idx: Vec<usize> =
            seq.tracks.iter().enumerate().filter(|(_, tr)| tr.kind == TrackKind::Video).map(|(i, _)| i).collect();
        if angle >= video_idx.len() {
            return false;
        }
        split_tracks_at(&mut seq.tracks, local_t, None, next_id);
        for (rank, &ti) in video_idx.iter().enumerate() {
            for c in seq.tracks[ti].clips.iter_mut() {
                if c.start >= local_t - EPS {
                    c.enabled = rank == angle;
                }
            }
        }
        true
    }

    /// Read-only (angle index, video-track name) pairs of a multicam `Sequence` clip, for the angle-grid
    /// UI and `multicam.switch`'s own range validation. Empty when `seq_clip` isn't a `Sequence` clip.
    pub fn multicam_angles(&self, seq_clip: Id) -> Vec<(usize, String)> {
        let Some(clip) = self.clip(seq_clip) else { return Vec::new() };
        if clip.kind != ClipKind::Sequence {
            return Vec::new();
        }
        let Some(seq) = self.sequence(clip.sequence) else { return Vec::new() };
        seq.tracks.iter().filter(|t| t.kind == TrackKind::Video).enumerate().map(|(i, t)| (i, t.name.clone())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(id: Id, dur: f64) -> Asset {
        Asset {
            id,
            path: format!("C:/cam{id}.mp4"),
            kind: ClipKind::Video,
            duration: dur,
            width: 1920,
            height: 1080,
            fps: 30.0,
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

    /// Three angle clips with offsets `[0, 1.2, -0.4]` nest into a sequence with one video track per
    /// angle; the PAIRWISE difference between each pair of nested (rebased) starts equals the negative
    /// of the pairwise offset difference - the frame-alignment claim in a form that doesn't depend on
    /// which clip happens to land at the sequence's own t=0.
    #[test]
    fn multicam_make_creates_one_track_per_angle_with_offsets() {
        let mut p = Project::new();
        let ids: Vec<Id> = (0..3)
            .map(|i| {
                let aid = p.add_asset(asset(100 + i, 20.0));
                let cid = p.new_id();
                let mut c = Clip::new(cid, ClipKind::Video, format!("cam{i}"), 5.0, 10.0);
                c.asset = aid;
                // each angle on its own source video track (V1 exists already; add V2/V3) - a real
                // multicam recording is one camera per track, not stacked on the same one.
                let ti = if i == 0 { 0 } else { p.add_track(TrackKind::Video) };
                p.tracks[ti].clips.push(c);
                cid
            })
            .collect();
        let offsets = [0.0, 1.2, -0.4];
        let seq_id = p.multicam_make(&ids, &offsets, "Multicam 1").expect("nests");
        let seq = p.sequence(seq_id).expect("sequence created");
        let video_tracks: Vec<&Track> = seq.tracks.iter().filter(|t| t.kind == TrackKind::Video).collect();
        assert_eq!(video_tracks.len(), 3, "one video track per angle");
        let starts: Vec<f64> = video_tracks.iter().map(|t| t.clips[0].start).collect();
        for i in 0..3 {
            for j in 0..3 {
                let got = starts[i] - starts[j];
                let want = offsets[j] - offsets[i];
                assert!((got - want).abs() < 1e-9, "angle {i} vs {j}: {got} != {want}");
            }
        }
        // the source (main-timeline) clips are gone - nest_selection moved them, not copied them
        assert!(p.tracks[0].clips.is_empty());
    }

    fn multicam_fixture() -> (Project, Id) {
        let mut p = Project::new();
        let ids: Vec<Id> = (0..2)
            .map(|i| {
                let aid = p.add_asset(asset(200 + i, 20.0));
                let cid = p.new_id();
                let mut c = Clip::new(cid, ClipKind::Video, format!("cam{i}"), 0.0, 10.0);
                c.asset = aid;
                let ti = if i == 0 { 0 } else { p.add_track(TrackKind::Video) };
                p.tracks[ti].clips.push(c);
                cid
            })
            .collect();
        let seq_id = p.multicam_make(&ids, &[0.0, 0.0], "Multicam").unwrap();
        let clip_id = p.insert_sequence_clip(seq_id, 0.0, None).unwrap();
        (p, clip_id)
    }

    #[test]
    fn multicam_switch_splits_and_toggles_enabled_only_after_t() {
        let (mut p, clip_id) = multicam_fixture();
        assert!(p.multicam_switch(clip_id, 5.0, 1));
        let seq_id = p.clip(clip_id).unwrap().sequence;
        let seq = p.sequence(seq_id).unwrap();
        let video: Vec<&Track> = seq.tracks.iter().filter(|t| t.kind == TrackKind::Video).collect();
        // clips ENDING before the split point are untouched (still their original enabled=true -
        // "non-destructive: earlier segments keep their prior enabled state", not forced to a value).
        for tr in &video {
            for c in tr.clips.iter().filter(|c| c.start < 5.0 - 1e-6) {
                assert!(c.enabled, "clip before the split point must be left as it was");
            }
        }
        // clips starting AT/AFTER the split point: enabled only on the switched-to angle (rank 1).
        for c in video[0].clips.iter().filter(|c| c.start >= 5.0 - 1e-6) {
            assert!(!c.enabled, "angle 0's segment after the switch must be disabled");
        }
        for c in video[1].clips.iter().filter(|c| c.start >= 5.0 - 1e-6) {
            assert!(c.enabled, "angle 1's segment after the switch (the one switched to) must be enabled");
        }
        // clip ids before the split point are untouched (no new id for the untouched left half)
        assert_eq!(video[0].clips[0].start, 0.0);
    }

    #[test]
    fn multicam_switch_rejects_out_of_range_angle() {
        let (mut p, clip_id) = multicam_fixture();
        let before = p.to_json();
        assert!(!p.multicam_switch(clip_id, 5.0, 2), "only 2 angles (0,1) exist");
        assert_eq!(p.to_json(), before, "a rejected switch mutates nothing");
    }

    #[test]
    fn multicam_switch_never_touches_project_editing_or_main_stash() {
        let (mut p, clip_id) = multicam_fixture();
        assert!(p.editing.is_none());
        assert!(p.main_stash.is_none());
        assert!(p.multicam_switch(clip_id, 5.0, 1));
        assert!(p.editing.is_none(), "multicam_switch must never open a sequence for editing");
        assert!(p.main_stash.is_none(), "multicam_switch must never stash the main timeline");
    }

    #[test]
    fn multicam_angles_lists_video_tracks_by_name() {
        let (p, clip_id) = multicam_fixture();
        let angles = p.multicam_angles(clip_id);
        assert_eq!(angles.iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![0, 1]);
    }
}
