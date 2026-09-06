use crate::model::*;

impl Project {
    // ---------- queries ----------
    pub fn duration(&self) -> f64 {
        self.tracks.iter().map(|t| t.end()).fold(0.0, f64::max)
    }
    pub fn is_empty(&self) -> bool {
        self.tracks.iter().all(|t| t.clips.is_empty())
    }
    pub fn frame_dur(&self) -> f64 {
        1.0 / self.fps.max(1.0)
    }
    pub fn snap_frame(&self, t: f64) -> f64 {
        (t * self.fps).round() / self.fps
    }
    /// (track index, clip index) of a clip.
    pub fn find(&self, id: Id) -> Option<(usize, usize)> {
        for (ti, t) in self.tracks.iter().enumerate() {
            if let Some(ci) = t.clips.iter().position(|c| c.id == id) {
                return Some((ti, ci));
            }
        }
        None
    }
    pub fn clip(&self, id: Id) -> Option<&Clip> {
        self.find(id).map(|(t, c)| &self.tracks[t].clips[c])
    }
    pub fn clip_mut(&mut self, id: Id) -> Option<&mut Clip> {
        let (t, c) = self.find(id)?;
        Some(&mut self.tracks[t].clips[c])
    }
    pub fn track_of(&self, clip: Id) -> Option<usize> {
        self.find(clip).map(|(t, _)| t)
    }
    pub fn all_clips(&self) -> impl Iterator<Item = (usize, &Clip)> {
        self.tracks.iter().enumerate().flat_map(|(i, t)| t.clips.iter().map(move |c| (i, c)))
    }
    /// All clip ids linked with `id` (including itself).
    pub fn linked(&self, id: Id) -> Vec<Id> {
        match self.clip(id) {
            Some(c) if c.link != 0 => {
                let l = c.link;
                self.all_clips().filter(|(_, c)| c.link == l).map(|(_, c)| c.id).collect()
            }
            Some(_) => vec![id],
            None => Vec::new(),
        }
    }
    /// Expand a selection to include linked clips.
    pub fn expand_links(&self, ids: &[Id]) -> Vec<Id> {
        let mut out: Vec<Id> = Vec::new();
        for &id in ids {
            for l in self.linked(id) {
                if !out.contains(&l) {
                    out.push(l);
                }
            }
        }
        out
    }
    pub fn video_tracks(&self) -> Vec<usize> {
        (0..self.tracks.len()).filter(|&i| self.tracks[i].kind == TrackKind::Video).collect()
    }
    pub fn audio_tracks(&self) -> Vec<usize> {
        (0..self.tracks.len()).filter(|&i| self.tracks[i].kind == TrackKind::Audio).collect()
    }
    /// UI-only (preview decoration): true if a visual clip covers `t` on an active video track.
    pub fn has_video_at(&self, t: f64) -> bool {
        self.video_tracks().into_iter().any(|ti| {
            self.active(ti) && self.tracks[ti].clips.iter().any(|c| c.enabled && c.is_visual() && c.contains(t))
        })
    }
    /// UI-only: (asset id, audio_stream) for every enabled audio clip covering `t` on an active audio track.
    pub fn audio_clips_at(&self, t: f64) -> Vec<(Id, usize)> {
        self.audio_tracks()
            .into_iter()
            .filter(|&ti| self.active(ti))
            .flat_map(|ti| {
                self.tracks[ti].clips.iter().filter(|c| c.enabled && c.contains(t)).map(|c| (c.asset, c.audio_stream))
            })
            .collect()
    }
    /// Sorted distinct clip boundaries (plus 0) for prev/next-cut navigation.
    pub fn cut_points(&self) -> Vec<f64> {
        let mut v = vec![0.0];
        for (_, c) in self.all_clips() {
            v.push(c.start);
            v.push(c.end());
        }
        v.sort_by(f64::total_cmp);
        v.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        v
    }
    /// Mute/solo resolution: a track is audible/visible if (no solo among its kind && !muted) || solo.
    pub fn active(&self, tidx: usize) -> bool {
        let t = &self.tracks[tidx];
        let any_solo = self.tracks.iter().any(|o| o.kind == t.kind && o.solo);
        if any_solo {
            t.solo
        } else {
            !t.muted
        }
    }
    /// Source seconds available before the clip's left edge (for `Clip::trim_start`).
    pub fn head_room(&self, clip: &Clip) -> f64 {
        if !matches!(clip.kind, ClipKind::Video | ClipKind::Audio) || clip.freeze.is_some() {
            return f64::INFINITY;
        }
        let Some(a) = self.asset(clip.asset) else { return f64::INFINITY };
        if clip.reverse {
            (a.duration - clip.src_end()).max(0.0)
        } else {
            clip.src_in.max(0.0)
        }
    }
    /// Longest duration the clip may have (right edge), given its source window, speed and direction.
    pub fn max_clip_duration(&self, clip: &Clip) -> f64 {
        if !matches!(clip.kind, ClipKind::Video | ClipKind::Audio) || clip.freeze.is_some() {
            return f64::INFINITY;
        }
        let Some(a) = self.asset(clip.asset) else { return f64::INFINITY };
        let extra = if clip.reverse { clip.src_in } else { a.duration - clip.src_end() };
        (clip.duration + extra.max(0.0) / clip.speed).max(MIN_CLIP)
    }

    // ---------- usage ----------
    /// Asset ids referenced by any clip (main timeline, stash, every sequence).
    pub fn used_assets(&self) -> std::collections::HashSet<Id> {
        let mut s = std::collections::HashSet::new();
        let mut add = |tracks: &[Track]| {
            for c in tracks.iter().flat_map(|t| t.clips.iter()) {
                if c.uses_asset() {
                    s.insert(c.asset);
                }
            }
        };
        add(&self.tracks);
        if let Some(st) = &self.main_stash {
            add(&st.tracks);
        }
        for seq in &self.sequences {
            add(&seq.tracks);
        }
        s
    }
    /// Remove assets no clip uses (planner moodboard assets are kept). Returns how many were removed.
    pub fn remove_unused_assets(&mut self) -> usize {
        let used = self.used_assets();
        let planned = self.plan_assets();
        let before = self.assets.len();
        self.assets.retain(|a| used.contains(&a.id) || planned.contains(&a.id));
        before - self.assets.len()
    }
}
