//! ---- ws:beat-aligner ----
//! Using the Beat Aligner's marks on the timeline: timeline markers, cuts, sound effects, clip edges
//! snapped to the marks and a keyframe block on each mark. One call = one undo step at the caller.

use crate::model::*;

/// Which clips a beat cut splits.
#[derive(Clone, Debug, PartialEq)]
pub enum CutScope {
    /// Every clip on this track (index into `tracks`).
    Track(usize),
    /// Every clip on every video track (the picture) and the sound linked to it, never the song.
    AllVideo,
    /// These clips (and what is linked to them).
    Clips(Vec<Id>),
}

impl Project {
    /// Source <-> timeline for the aligner's song: through its clip (the sheet's own, else the first clip
    /// playing the song), identity when the song is not on the timeline.
    pub fn beat_map(&self) -> BeatMap {
        let s = &self.beats;
        let clip = self.clip(s.clip).filter(|c| c.asset == s.asset).or_else(|| {
            (s.asset != 0).then(|| self.all_clips().map(|(_, c)| c).find(|c| c.asset == s.asset)).flatten()
        });
        clip.map(BeatMap::of).unwrap_or_default()
    }

    /// The marks in timeline time, every `every`th one (1 = all), sorted.
    pub fn beat_times(&self, every: usize) -> Vec<f64> {
        let map = self.beat_map();
        self.beats.marks.iter().step_by(every.max(1)).map(|m| map.to_tl(m.t)).filter(|t| *t >= 0.0).collect()
    }

    /// Point the aligner at a song: a timeline clip (its asset comes along) or a library asset.
    /// Changing the song clears the sheet; picking another clip of the same song keeps it.
    pub fn set_beat_source(&mut self, clip: Option<Id>, asset: Option<Id>, voice: bool) -> bool {
        let (clip, asset) = match (clip.and_then(|id| self.clip(id)), asset) {
            (Some(c), _) => (c.id, c.asset),
            (None, Some(a)) if self.asset(a).is_some() => (0, a),
            _ => return false,
        };
        if asset != self.beats.asset || voice != self.beats.voice {
            self.beats = BeatSheet { voice, ..Default::default() };
        }
        self.beats.asset = asset;
        self.beats.clip = clip;
        true
    }

    /// Every mark as a project marker (coloured by kind) and every section as a range marker with its
    /// name and colour. Returns how many markers were added.
    pub fn beats_to_markers(&mut self, marks: bool, sections: bool) -> usize {
        let map = self.beat_map();
        let mut todo: Vec<(f64, f64, String, u8, &str)> = Vec::new();
        if marks {
            todo.extend(
                self.beats
                    .marks
                    .iter()
                    .map(|m| (map.to_tl(m.t), 0.0, m.kind.name().to_string(), m.kind.label(), "bolt")),
            );
        }
        if sections {
            todo.extend(
                self.beats
                    .sections
                    .iter()
                    .map(|s| (map.to_tl(s.a), (s.b - s.a) / map.speed, s.name.clone(), s.label, "flag")),
            );
        }
        let n = todo.len();
        for (t, d, name, label, icon) in todo {
            let id = self.add_marker(t, name);
            if let Some(m) = self.marker_mut(id) {
                m.duration = d.max(0.0);
                m.label = label;
                m.icon = icon.into();
            }
        }
        n
    }

    /// Split at every `every`th mark: the clips in `scope` only (with their linked audio for a picked
    /// clip; never the song itself). Returns the number of cuts.
    pub fn beats_cut(&mut self, scope: &CutScope, every: usize) -> usize {
        let song = self.beats.asset;
        let mut ids: Vec<Id> = match scope {
            CutScope::Track(ti) => {
                self.tracks.get(*ti).map(|t| t.clips.iter().map(|c| c.id).collect()).unwrap_or_default()
            }
            CutScope::AllVideo => {
                let v = self.video_tracks();
                let pic: Vec<Id> = self.all_clips().filter(|(ti, _)| v.contains(ti)).map(|(_, c)| c.id).collect();
                self.expand_links(&pic)
            }
            CutScope::Clips(ids) => self.expand_links(ids),
        };
        ids.retain(|id| {
            self.clip(*id).is_some_and(|c| song == 0 || c.asset != song || matches!(scope, CutScope::Track(_)))
        });
        let mut cuts = 0;
        for t in self.beat_times(every) {
            let new = self.split_at(t, Some(&ids));
            if !new.is_empty() {
                cuts += 1;
                ids.extend(new); // the right halves get cut at the next mark
            }
        }
        cuts
    }

    /// Place audio asset `sfx` at every `every`th mark, `lead` seconds early (so the hit lands with it),
    /// each cut short at the next placement, on one audio track. Returns the new clip ids.
    pub fn beats_sfx(&mut self, sfx: Id, every: usize, lead: f64) -> Vec<Id> {
        let Some(a) = self.asset(sfx).filter(|a| a.kind == ClipKind::Audio).cloned() else { return Vec::new() };
        let times = self.beat_times(every);
        let mut track: Option<usize> = None;
        let mut out = Vec::new();
        for (i, &t) in times.iter().enumerate() {
            let at = (t - lead).max(0.0);
            let next = times.get(i + 1).map_or(f64::INFINITY, |n| n - lead);
            let len = a.duration.min(next - at).max(MIN_CLIP);
            let ids = self.insert_asset_clips_ranged(sfx, at, None, track, Some((0.0, len)));
            if track.is_none() {
                track = ids.first().and_then(|&id| self.track_of(id));
            }
            out.extend(ids);
        }
        out
    }

    /// Move each clip's start to the nearest mark within `tol` seconds (keeping its length), then trim
    /// its end to the nearest mark within `tol` when the source has the frames. A move that would
    /// overlap another clip on its track is skipped. Returns how many edges moved.
    pub fn beats_snap_clips(&mut self, ids: &[Id], tol: f64) -> usize {
        let times = self.beat_times(1);
        let near = |t: f64| {
            times
                .iter()
                .copied()
                .filter(|m| (m - t).abs() <= tol)
                .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
        };
        let mut n = 0;
        for &id in ids {
            let (Some(ti), Some(c)) = (self.track_of(id), self.clip(id).cloned()) else { continue };
            let free = |p: &Project, s: f64, e: f64| {
                p.tracks[ti].clips.iter().all(|o| o.id == id || o.end() <= s + 1e-9 || o.start >= e - 1e-9)
            };
            let mut start = c.start;
            if let Some(m) = near(c.start).filter(|m| (m - c.start).abs() > 1e-9) {
                if free(self, m, m + c.duration) {
                    start = m;
                    n += 1;
                }
            }
            let mut dur = c.duration;
            if let Some(m) =
                near(start + c.duration).filter(|m| *m > start + MIN_CLIP && (m - start - c.duration).abs() > 1e-9)
            {
                let room = self.asset(c.asset).map_or(f64::INFINITY, |a| (a.duration - c.src_in) / c.speed.max(1e-6));
                if c.kind == ClipKind::Image || c.kind == ClipKind::Text || m - start <= room + 1e-9 {
                    if free(self, start, m) {
                        dur = m - start;
                        n += 1;
                    }
                }
            }
            if let Some(c) = self.clip_mut(id) {
                c.start = start;
                c.duration = dur;
            }
        }
        self.tidy();
        n
    }

    /// Apply keyframe block `block` (a Zoom Punch, Shake, Flash ...) at every `every`th mark on each
    /// picture clip in `ids` (all video clips when empty) that plays at that moment, never running into
    /// the next mark. Returns how many blocks were placed.
    pub fn beats_effect(&mut self, block: &KeyBlock, ids: &[Id], every: usize) -> usize {
        let times = self.beat_times(every);
        let targets: Vec<Id> = if ids.is_empty() {
            let v = self.video_tracks();
            self.all_clips().filter(|(ti, c)| v.contains(ti) && c.is_visual()).map(|(_, c)| c.id).collect()
        } else {
            ids.to_vec()
        };
        let mut n = 0;
        for (i, &t) in times.iter().enumerate() {
            let gap = times.get(i + 1).map_or(f64::INFINITY, |nx| nx - t);
            for &id in &targets {
                let Some(c) = self.clip(id).filter(|c| c.is_visual() && c.contains(t)) else { continue };
                let local = t - c.start;
                let dur = block.dur.min(gap).min(c.duration - local);
                if dur > 0.02 && self.place_key_block(id, block, local, Some(dur)).is_some() {
                    n += 1;
                }
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio_asset(id: Id, path: &str, dur: f64) -> Asset {
        serde_json::from_value(serde_json::json!({
            "id": id, "path": path, "kind": "Audio", "duration": dur, "width": 0, "height": 0, "fps": 0.0,
            "audio_streams": [{"index": 0, "channels": 2, "sample_rate": 48000}]
        }))
        .unwrap()
    }

    /// V1: one 10 s video clip; A1: the song (asset 1) from 2 s, its source from 1 s; marks every second.
    fn project() -> Project {
        let mut p = Project::new();
        p.assets.push(audio_asset(1, "song.wav", 60.0));
        p.assets.push(audio_asset(2, "hit.wav", 0.4));
        let v = p.video_tracks()[0];
        let a = p.audio_tracks()[0];
        p.tracks[v].clips.push(Clip::new(10, ClipKind::Image, "pic", 0.0, 10.0));
        let mut song = Clip::new(11, ClipKind::Audio, "song", 2.0, 8.0);
        song.asset = 1;
        song.src_in = 1.0;
        p.tracks[a].clips.push(song);
        p.next_id = 100;
        assert!(p.set_beat_source(Some(11), None, false));
        for t in [2.0, 3.0, 4.0, 5.0] {
            p.beats.add_mark(t, MarkKind::Tap); // timeline 3, 4, 5, 6
        }
        p
    }

    #[test]
    fn cut_at_marks_splits_the_picture_not_the_song() {
        let mut p = project();
        assert_eq!(p.beat_times(1), vec![3.0, 4.0, 5.0, 6.0]);
        assert_eq!(p.beats_cut(&CutScope::AllVideo, 1), 4);
        let v = p.video_tracks()[0];
        let starts: Vec<f64> = p.tracks[v].clips.iter().map(|c| c.start).collect();
        assert_eq!(starts, vec![0.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(p.tracks[p.audio_tracks()[0]].clips.len(), 1, "the song is never cut");
        // the footage's own sound is cut with its picture
        let mut p = project();
        let (v, a) = (p.video_tracks()[0], p.add_track(TrackKind::Audio));
        p.tracks[v].clips[0].link = 77;
        let mut snd = Clip::new(12, ClipKind::Audio, "pic sound", 0.0, 10.0);
        snd.link = 77;
        p.tracks[a].clips.push(snd);
        assert_eq!(p.beats_cut(&CutScope::AllVideo, 1), 4);
        assert_eq!(p.tracks[a].clips.len(), 5, "linked sound follows its picture");
        assert_eq!(p.tracks[p.track_of(11).unwrap()].clips.len(), 1, "the song is never cut");
        // every 2nd mark on one track
        let mut p = project();
        let v = p.video_tracks()[0];
        assert_eq!(p.beats_cut(&CutScope::Track(v), 2), 2);
        assert_eq!(p.tracks[v].clips.iter().map(|c| c.start).collect::<Vec<_>>(), vec![0.0, 3.0, 5.0]);
    }

    #[test]
    fn sfx_at_marks_places_one_clip_per_mark_on_one_track() {
        let mut p = project();
        let ids = p.beats_sfx(2, 1, 0.05);
        assert_eq!(ids.len(), 4);
        let tracks: std::collections::HashSet<usize> = ids.iter().map(|&id| p.track_of(id).unwrap()).collect();
        assert_eq!(tracks.len(), 1, "all on one track");
        let song_track = p.track_of(11).unwrap();
        assert!(!tracks.contains(&song_track), "not on the song's track");
        let placed: Vec<(f64, f64, Id)> =
            ids.iter().map(|&id| p.clip(id).map(|c| (c.start, c.duration, c.asset)).unwrap()).collect();
        for (i, (s, d, a)) in placed.iter().enumerate() {
            assert!((s - (3.0 + i as f64 - 0.05)).abs() < 1e-9, "{placed:?}");
            assert!((d - 0.4).abs() < 1e-9 && *a == 2);
        }
        assert!(p.beats_sfx(10, 1, 0.0).is_empty(), "the picture is not a sound");
    }

    #[test]
    fn markers_and_snapping_and_effects() {
        let mut p = project();
        p.beats.add_section(2.0, 4.0, "Chorus");
        assert_eq!(p.beats_to_markers(true, true), 5);
        let sec = p.markers.iter().find(|m| m.name == "Chorus").unwrap();
        assert_eq!((sec.t, sec.duration, sec.label), (3.0, 2.0, 2));
        assert_eq!(p.markers.iter().filter(|m| m.name == "Tap" && m.label == 3).count(), 4);

        // a clip starting at 3.1 and ending at 4.9 moves to 3.0 and trims to 5.0
        let v = p.add_track(TrackKind::Video);
        p.tracks[v].clips.push(Clip::new(20, ClipKind::Image, "b", 3.1, 1.8));
        assert_eq!(p.beats_snap_clips(&[20], 0.25), 2);
        let c = p.clip(20).unwrap();
        assert_eq!((c.start, c.duration), (3.0, 2.0));

        let punch = find_block(&[], "Zoom Punch").unwrap();
        assert_eq!(p.beats_effect(&punch, &[10], 1), 4);
        assert_eq!(p.clip(10).unwrap().blocks.len(), 4);
    }
}
