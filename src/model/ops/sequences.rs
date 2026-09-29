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
        let mut v = Track::new(self.new_id(), TrackKind::Video, "V1");
        let mut a = Track::new(self.new_id(), TrackKind::Audio, "A1");
        // both are the first (only) track of their kind in a fresh sequence
        v.ripple = Track::default_ripple(TrackKind::Video, 0);
        a.ripple = Track::default_ripple(TrackKind::Audio, 0);
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
    /// Place a sequence as a clip at `at` (track `track` preferred). Like a media file, a sequence with
    /// audio becomes a picture clip on a video track plus a linked twin on an audio track: the mixer
    /// only plays Sequence clips from audio tracks, so the sound mutes, cuts and routes like any audio
    /// clip. An audio-only sequence gets just the audio clip. Every insert path (library drop, MCP,
    /// nest, multicam) comes through here. Returns the first clip's id (the video one when there is
    /// one). None on cycles / unknown id.
    pub fn insert_sequence_clip(&mut self, seq: Id, at: f64, track: Option<usize>) -> Option<Id> {
        let name = self.sequence(seq)?.name.clone();
        // a sequence can't contain itself: the timeline being edited (or main) must not be inside `seq`
        if let Some(cur) = self.editing {
            if self.sequence_contains(seq, cur) {
                return None;
            }
        }
        let dur = self.sequence_duration(seq).max(MIN_CLIP);
        let (video, audio) = self.sequence_halves(seq);
        let kind = if video { TrackKind::Video } else { TrackKind::Audio };
        let ti = self.find_free_track(kind, at, dur, track);
        let mut c = Clip::new(self.new_id(), ClipKind::Sequence, name, at, dur);
        c.sequence = seq;
        let id = c.id;
        self.tracks[ti].clips.push(c);
        self.tracks[ti].sort();
        if video && audio {
            self.add_audio_twin(id, track);
        }
        Some(id)
    }
    /// Which clips a placed sequence gets: (picture, sound). An empty one still shows as a picture clip.
    pub(crate) fn sequence_halves(&self, seq: Id) -> (bool, bool) {
        let has = |k| self.sequence_tracks(seq).is_some_and(|ts| ts.iter().any(|t| t.kind == k && !t.clips.is_empty()));
        (has(TrackKind::Video) || !has(TrackKind::Audio), has(TrackKind::Audio))
    }
    /// Link Sequence clip `id` to a new twin on a free audio track (`prefer` first): same sequence,
    /// timing, retime and audio settings, none of the picture-only state. Returns the twin's id.
    /// ponytail: the timeline draws no waveform on it (no asset to read peaks from); sum the nested
    /// clips' peaks if nested audio ever needs one.
    fn add_audio_twin(&mut self, id: Id, prefer: Option<usize>) -> Option<Id> {
        let (ti, ci) = self.find(id)?;
        if self.tracks[ti].clips[ci].link == 0 {
            self.tracks[ti].clips[ci].link = self.new_id();
        }
        let v = self.tracks[ti].clips[ci].clone();
        // markers carry ids of their own; effects/mask/graph only shape pixels
        let a = Clip { id: self.new_id(), effects: Vec::new(), mask: None, graph: None, markers: Vec::new(), ..v };
        let ai = self.find_free_track(TrackKind::Audio, a.start, a.duration, prefer);
        let aid = a.id;
        self.tracks[ai].clips.push(a);
        self.tracks[ai].sort();
        Some(aid)
    }
    /// v2 -> v3 (`from_json`): Sequence clips used to play their audio from the video track. Give each
    /// one whose sequence made sound a linked audio twin, in every track list (main, each sequence, the
    /// stash), so an old project sounds the same under the audio-tracks-only mixer rule.
    /// ponytail: a transition between two such clips stays picture-only, so their audio now hard-cuts
    /// there (`add_transition` mirrors new ones onto the twins); mirror old ones here if that bites.
    pub(crate) fn migrate_sequence_audio(&mut self) {
        // the v2 rule: any audio-track clip, or a Sequence clip on a video track whose sequence has sound
        fn loud(p: &Project, seq: Id, depth: u32) -> bool {
            depth < 32
                && p.sequence_tracks(seq).is_some_and(|ts| {
                    ts.iter().any(|t| {
                        t.clips.iter().any(|c| {
                            t.kind == TrackKind::Audio
                                || (c.kind == ClipKind::Sequence && loud(p, c.sequence, depth + 1))
                        })
                    })
                })
        }
        // list 0 is `tracks`, 1..=n the sequences', n+1 the stash: swapping one into `tracks` lets
        // `add_audio_twin` place into it (`loud` is settled first, while nothing is swapped)
        fn swap(p: &mut Project, i: usize) {
            let other = if i == 0 {
                return;
            } else if i <= p.sequences.len() {
                &mut p.sequences[i - 1].tracks
            } else if let Some(st) = &mut p.main_stash {
                &mut st.tracks
            } else {
                return;
            };
            std::mem::swap(&mut p.tracks, other);
        }
        let loud: Vec<Id> = self.sequences.iter().map(|s| s.id).filter(|&s| loud(self, s, 0)).collect();
        for i in 0..=self.sequences.len() + self.main_stash.is_some() as usize {
            swap(self, i);
            let ids: Vec<Id> = (self.tracks.iter().filter(|t| t.kind == TrackKind::Video))
                .flat_map(|t| &t.clips)
                .filter(|c| c.kind == ClipKind::Sequence && loud.contains(&c.sequence))
                .map(|c| c.id)
                .collect();
            for id in ids {
                self.add_audio_twin(id, None);
            }
            swap(self, i);
        }
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
                    let mut t = Track::new(id, kind, name);
                    t.ripple = Track::default_ripple(kind, have);
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
