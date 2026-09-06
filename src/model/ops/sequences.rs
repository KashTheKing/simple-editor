use crate::model::*;

impl Project {
    // ---------- sequences (nested timelines) ----------
    pub fn sequence(&self, id: Id) -> Option<&Sequence> {
        self.sequences.iter().find(|s| s.id == id)
    }
    pub fn sequence_mut(&mut self, id: Id) -> Option<&mut Sequence> {
        self.sequences.iter_mut().find(|s| s.id == id)
    }
    /// New empty sequence (V1 + A1) with the given format; returns its id.
    pub fn new_sequence(&mut self, name: impl Into<String>, width: u32, height: u32, fps: f64) -> Id {
        let id = self.new_id();
        let mut seq = Sequence { id, name: name.into(), width, height, fps, tracks: Vec::new() };
        let v = Track::new(self.new_id(), TrackKind::Video, "V1");
        let a = Track::new(self.new_id(), TrackKind::Audio, "A1");
        seq.tracks.push(v);
        seq.tracks.push(a);
        self.sequences.push(seq);
        id
    }
    /// Duration of a sequence (the live tracks when it is the one being edited).
    pub fn sequence_duration(&self, id: Id) -> f64 {
        if self.editing == Some(id) {
            return self.duration();
        }
        self.sequence(id).map(|s| s.duration()).unwrap_or(0.0)
    }
    /// Tracks of a sequence for rendering (its own, or the live `tracks` while it is being edited).
    pub fn sequence_tracks(&self, id: Id) -> Option<&[Track]> {
        if self.editing == Some(id) {
            return Some(&self.tracks);
        }
        self.sequence(id).map(|s| s.tracks.as_slice())
    }
    /// True if sequence `outer` contains `inner` directly or through nested sequence clips.
    pub fn sequence_contains(&self, outer: Id, inner: Id) -> bool {
        fn walk(p: &Project, tracks: &[Track], inner: Id, depth: u32) -> bool {
            if depth > 32 {
                return true; // treat runaway nesting as a cycle
            }
            tracks.iter().flat_map(|t| t.clips.iter()).any(|c| {
                c.kind == ClipKind::Sequence
                    && (c.sequence == inner
                        || p.sequence_tracks(c.sequence).map(|tr| walk(p, tr, inner, depth + 1)).unwrap_or(false))
            })
        }
        outer == inner || self.sequence_tracks(outer).map(|t| walk(self, t, inner, 0)).unwrap_or(false)
    }
    /// Swap sequence `id` into `tracks` for editing (closing any other open sequence first).
    pub fn open_sequence(&mut self, id: Id) -> bool {
        if self.editing == Some(id) {
            return true;
        }
        if self.sequence(id).is_none() {
            return false;
        }
        self.close_sequence();
        let stash = Stash {
            tracks: std::mem::take(&mut self.tracks),
            width: self.width,
            height: self.height,
            fps: self.fps,
            in_point: self.in_point.take(),
            out_point: self.out_point.take(),
        };
        let seq = self.sequence_mut(id).unwrap();
        let tracks = std::mem::take(&mut seq.tracks);
        let (w, h, fps) = (seq.width, seq.height, seq.fps);
        self.main_stash = Some(stash);
        self.tracks = tracks;
        self.width = w;
        self.height = h;
        self.fps = fps;
        self.editing = Some(id);
        true
    }
    /// Put the edited sequence back and restore the main timeline.
    pub fn close_sequence(&mut self) {
        let Some(id) = self.editing.take() else { return };
        let tracks = std::mem::take(&mut self.tracks);
        let (w, h, fps) = (self.width, self.height, self.fps);
        if let Some(seq) = self.sequence_mut(id) {
            seq.tracks = tracks;
            seq.width = w;
            seq.height = h;
            seq.fps = fps;
        }
        if let Some(st) = self.main_stash.take() {
            self.tracks = st.tracks;
            self.width = st.width;
            self.height = st.height;
            self.fps = st.fps;
            self.in_point = st.in_point;
            self.out_point = st.out_point;
        }
    }
    /// Place a sequence as a clip at `at` (video track `video_track` preferred). None on cycles / unknown id.
    pub fn insert_sequence_clip(&mut self, seq: Id, at: f64, video_track: Option<usize>) -> Option<Id> {
        let name = self.sequence(seq)?.name.clone();
        // a sequence can't contain itself: the timeline being edited (or main) must not be inside `seq`
        if let Some(cur) = self.editing {
            if self.sequence_contains(seq, cur) {
                return None;
            }
        }
        let dur = self.sequence_duration(seq).max(MIN_CLIP);
        let ti = self.find_free_track(TrackKind::Video, at, dur, video_track);
        let mut c = Clip::new(self.new_id(), ClipKind::Sequence, name, at, dur);
        c.sequence = seq;
        let id = c.id;
        self.tracks[ti].clips.push(c);
        self.tracks[ti].sort();
        Some(id)
    }
    /// Move the selected clips (+ linked) into a new sequence and replace them with one Sequence clip.
    /// Returns the new sequence id.
    pub fn nest_selection(&mut self, ids: &[Id], name: impl Into<String>) -> Option<Id> {
        let ids = self.expand_links(ids);
        if ids.is_empty() {
            return None;
        }
        let start = ids.iter().filter_map(|&id| self.clip(id)).map(|c| c.start).fold(f64::INFINITY, f64::min);
        let end = ids.iter().filter_map(|&id| self.clip(id)).map(|c| c.end()).fold(0.0, f64::max);
        if !start.is_finite() || end <= start {
            return None;
        }
        let (w, h, fps) = (self.width, self.height, self.fps);
        let seq_id = self.new_sequence(name, w, h, fps);
        // move clips: keep their track kind and relative order of tracks
        let mut moved: Vec<(TrackKind, usize, Clip)> = Vec::new(); // (kind, index within kind, clip)
        let mut top_video: Option<usize> = None;
        for &id in &ids {
            let Some((ti, ci)) = self.find(id) else { continue };
            let kind = self.tracks[ti].kind;
            let list = if kind == TrackKind::Video { self.video_tracks() } else { self.audio_tracks() };
            let pos = list.iter().position(|&x| x == ti).unwrap_or(0);
            if kind == TrackKind::Video {
                top_video = Some(top_video.map_or(ti, |t: usize| t.max(ti)));
            }
            let mut c = self.tracks[ti].clips.remove(ci);
            c.start -= start;
            moved.push((kind, pos, c));
        }
        self.tidy();
        let next_ids: Vec<Id> = (0..64).map(|_| self.new_id()).collect();
        let mut nid = next_ids.into_iter();
        if let Some(seq) = self.sequence_mut(seq_id) {
            for (kind, pos, c) in moved {
                // ensure track `pos` of this kind exists
                loop {
                    let have = seq.tracks.iter().filter(|t| t.kind == kind).count();
                    if have > pos {
                        break;
                    }
                    let id = nid.next().unwrap_or(0);
                    let n = have + 1;
                    let name = format!("{}{}", if kind == TrackKind::Video { "V" } else { "A" }, n);
                    let t = Track::new(id, kind, name);
                    let idx = if kind == TrackKind::Video {
                        seq.tracks.iter().rposition(|t| t.kind == TrackKind::Video).map(|i| i + 1).unwrap_or(0)
                    } else {
                        seq.tracks.len()
                    };
                    seq.tracks.insert(idx, t);
                }
                let ti =
                    seq.tracks.iter().enumerate().filter(|(_, t)| t.kind == kind).nth(pos).map(|(i, _)| i).unwrap_or(0);
                seq.tracks[ti].clips.push(c);
                seq.tracks[ti].sort();
            }
        }
        let vt = top_video.or_else(|| self.video_tracks().last().copied());
        self.insert_sequence_clip(seq_id, start, vt);
        Some(seq_id)
    }
}
