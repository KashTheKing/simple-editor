use crate::model::*;

impl Project {
    // ---------- editing ----------
    /// Re-sort clips and drop transitions whose cuts no longer exist. Call after any layout change.
    pub fn tidy(&mut self) {
        for t in &mut self.tracks {
            t.sort();
            t.prune_transitions();
        }
    }

    /// Place an asset on the timeline at `at`: video/image clip on a video track (preferring `video_track`)
    /// plus one linked audio clip per audio stream (stream N preferring track A(N+1)). Returns new clip ids.
    pub fn insert_asset_clips(&mut self, asset_id: Id, at: f64, video_track: Option<usize>) -> Vec<Id> {
        let Some(asset) = self.asset(asset_id).cloned() else { return Vec::new() };
        let dur = if asset.kind == ClipKind::Image { 5.0 } else { asset.duration.max(MIN_CLIP) };
        let n_parts = asset.has_video() as usize + asset.audio_streams.len();
        let link = if n_parts > 1 { self.new_id() } else { 0 };
        let mut ids = Vec::new();
        if asset.has_video() {
            let ti = self.find_free_track(TrackKind::Video, at, dur, video_track);
            let mut c = Clip::new(self.new_id(), asset.kind, asset.name(), at, dur);
            c.asset = asset.id;
            c.link = link;
            ids.push(c.id);
            self.tracks[ti].clips.push(c);
            self.tracks[ti].sort();
        }
        for (i, s) in asset.audio_streams.iter().enumerate() {
            let prefer = self.audio_tracks().get(i).copied();
            let ti = self.find_free_track(TrackKind::Audio, at, dur, prefer);
            let name =
                if asset.audio_streams.len() > 1 { format!("{} [{}]", asset.name(), s.label()) } else { asset.name() };
            let mut c = Clip::new(self.new_id(), ClipKind::Audio, name, at, dur);
            c.asset = asset.id;
            c.audio_stream = s.index;
            c.link = link;
            ids.push(c.id);
            self.tracks[ti].clips.push(c);
            self.tracks[ti].sort();
        }
        ids
    }

    /// Add a text clip on the topmost video track that has room (or a new one).
    pub fn add_text_clip(&mut self, at: f64, dur: f64) -> Id {
        let prefer = self.video_tracks().last().copied();
        let ti = self.find_free_track(TrackKind::Video, at, dur, prefer);
        let c = Clip::new(self.new_id(), ClipKind::Text, "Text", at, dur);
        let id = c.id;
        self.tracks[ti].clips.push(c);
        self.tracks[ti].sort();
        id
    }

    /// Split every clip crossing t (or only the given ids). Right halves of a linked group stay linked
    /// to each other under a fresh link id. Returns the new (right-half) ids.
    pub fn split_at(&mut self, t: f64, only: Option<&[Id]>) -> Vec<Id> {
        let mut new_ids = Vec::new();
        let mut link_map: std::collections::HashMap<Id, Id> = std::collections::HashMap::new();
        for ti in 0..self.tracks.len() {
            let mut added = Vec::new();
            for ci in 0..self.tracks[ti].clips.len() {
                let c = &self.tracks[ti].clips[ci];
                if !c.contains(t) || only.map(|o| !o.contains(&c.id)).unwrap_or(false) {
                    continue;
                }
                let old_link = c.link;
                let nid = self.new_id();
                let new_link = if old_link != 0 {
                    match link_map.get(&old_link) {
                        Some(&l) => l,
                        None => {
                            let l = self.new_id();
                            link_map.insert(old_link, l);
                            l
                        }
                    }
                } else {
                    0
                };
                if let Some(mut right) = self.tracks[ti].clips[ci].split(t, nid) {
                    right.link = new_link;
                    new_ids.push(right.id);
                    added.push(right);
                }
            }
            if !added.is_empty() {
                self.tracks[ti].clips.extend(added);
                self.tracks[ti].sort();
            }
        }
        self.tidy();
        new_ids
    }

    /// Freeze-frame the clips at timeline time t: each clip is split at t and its right part becomes a
    /// still of the frame at t (linked clips of the same time too). Returns the frozen clip ids.
    pub fn freeze_at(&mut self, t: f64, ids: &[Id]) -> Vec<Id> {
        let ids = self.expand_links(ids);
        let mut frozen = Vec::new();
        for id in ids {
            let Some(c) = self.clip(id) else { continue };
            if !c.contains(t) {
                continue; // freezing a clip the playhead is not over would store an out-of-range source time
            }
            let src = c.src_time(t);
            let target =
                if t > c.start + MIN_CLIP { self.split_at(t, Some(&[id])).first().copied().unwrap_or(id) } else { id };
            if let Some(c) = self.clip_mut(target) {
                c.freeze = Some(src);
                frozen.push(target);
            }
        }
        frozen
    }

    /// Delete clips. With `ripple`, later clips close the gap (per track, only where no other clip overlaps the gap).
    pub fn delete_clips(&mut self, ids: &[Id], ripple: bool) {
        let mut ranges: Vec<(f64, f64)> = Vec::new();
        for &id in ids {
            if let Some(c) = self.clip(id) {
                ranges.push((c.start, c.end()));
            }
        }
        for t in &mut self.tracks {
            t.clips.retain(|c| !ids.contains(&c.id));
        }
        if ripple {
            ranges.sort_by(|a, b| b.0.total_cmp(&a.0)); // right to left
            ranges.dedup_by(|a, b| (a.0 - b.0).abs() < EPS && (a.1 - b.1).abs() < EPS);
            for (a, b) in ranges {
                self.close_gap(a, b);
            }
        }
        self.tidy();
    }

    /// Shift clips starting at/after `b` left by (b-a) on every track where [a,b) is free.
    fn close_gap(&mut self, a: f64, b: f64) {
        let len = b - a;
        if len <= 0.0 {
            return;
        }
        for t in &mut self.tracks {
            if !t.fits(a, len, &[]) {
                continue;
            }
            for c in &mut t.clips {
                if c.start >= b - EPS {
                    c.start -= len;
                }
            }
        }
    }

    /// Remove everything in [a,b) on all tracks and close the gap.
    pub fn ripple_delete_range(&mut self, a: f64, b: f64) {
        if b <= a + EPS {
            return;
        }
        self.split_at(a, None);
        self.split_at(b, None);
        let ids: Vec<Id> =
            self.all_clips().filter(|(_, c)| c.start >= a - EPS && c.end() <= b + EPS).map(|(_, c)| c.id).collect();
        self.delete_clips(&ids, false);
        self.close_gap(a, b);
        self.tidy();
    }

    /// Open a gap of `span` seconds at `at`: split anything crossing it, then slide every clip from
    /// there on to the right. The inverse of `ripple_delete_range`, used by Paste Insert.
    pub fn ripple_open(&mut self, at: f64, span: f64) {
        if span <= EPS {
            return;
        }
        self.split_at(at, None);
        // right to left, so a clip never lands on one that has not moved yet
        let mut ids: Vec<(Id, f64)> =
            self.all_clips().filter(|(_, c)| c.start >= at - EPS).map(|(_, c)| (c.id, c.start)).collect();
        ids.sort_by(|x, y| y.1.total_cmp(&x.1));
        for (id, _) in ids {
            self.move_clips(&[id], span, 0, None);
        }
        self.tidy();
    }

    /// Keep only [a,b), moving it to start at 0. Clears in/out points.
    pub fn trim_to_range(&mut self, a: f64, b: f64) {
        let end = self.duration().max(b) + 1.0;
        self.ripple_delete_range(b, end);
        self.ripple_delete_range(0.0, a);
        self.in_point = None;
        self.out_point = None;
    }

    /// Move clips by dt seconds; clips on tracks of `track_kind` also move by `dtrack` tracks within their kind.
    /// All-or-nothing: returns false (and changes nothing) if any destination is blocked or out of range.
    pub fn move_clips(&mut self, ids: &[Id], dt: f64, dtrack: i32, track_kind: Option<TrackKind>) -> bool {
        // (clip id, from track, to track, new start, dur)
        let mut plan: Vec<(Id, usize, usize, f64, f64)> = Vec::new();
        for &id in ids {
            let Some((ti, ci)) = self.find(id) else { return false };
            let c = &self.tracks[ti].clips[ci];
            let kind = self.tracks[ti].kind;
            let mut to = ti;
            if dtrack != 0 && track_kind == Some(kind) {
                let list = if kind == TrackKind::Video { self.video_tracks() } else { self.audio_tracks() };
                let pos = list.iter().position(|&x| x == ti).unwrap() as i32 + dtrack;
                if pos < 0 || pos >= list.len() as i32 {
                    return false;
                }
                to = list[pos as usize];
            }
            let ns = c.start + dt;
            if ns < -EPS {
                return false;
            }
            plan.push((id, ti, to, ns.max(0.0), c.duration));
        }
        // moved clips must not overlap each other (sorted per destination track: neighbours suffice)
        // or any clip that stays put
        plan.sort_by(|a, b| a.2.cmp(&b.2).then(a.3.total_cmp(&b.3)));
        for w in plan.windows(2) {
            if w[0].2 == w[1].2 && w[1].3 < w[0].3 + w[0].4 - EPS {
                return false;
            }
        }
        let moved: std::collections::HashSet<Id> = ids.iter().copied().collect();
        for (_, _, to, ns, dur) in &plan {
            let blocked = self.tracks[*to]
                .clips
                .iter()
                .any(|c| !moved.contains(&c.id) && c.start < ns + dur - EPS && *ns < c.end() - EPS);
            if blocked {
                return false;
            }
        }
        for (id, from, to, ns, _) in plan {
            let ci = self.tracks[from].clips.iter().position(|c| c.id == id).unwrap();
            let mut c = self.tracks[from].clips.remove(ci);
            c.start = ns;
            // transitions belong to the track; moving the right clip across tracks carries none
            self.tracks[to].clips.push(c);
        }
        self.tidy();
        true
    }

    /// Set `enabled` on the clips (linked clips too).
    pub fn set_enabled(&mut self, ids: &[Id], enabled: bool) {
        for id in self.expand_links(ids) {
            if let Some(c) = self.clip_mut(id) {
                c.enabled = enabled;
            }
        }
    }
    /// Unlink the clips if any is linked; otherwise link them together.
    pub fn toggle_link(&mut self, ids: &[Id]) {
        let linked = ids.iter().any(|&id| self.clip(id).map(|c| c.link != 0).unwrap_or(false));
        let link = if linked { 0 } else { self.new_id() };
        for &id in ids {
            if let Some(c) = self.clip_mut(id) {
                c.link = link;
            }
        }
    }
    /// Apply speed/reverse to the clips and their linked clips (keeps source windows; durations follow).
    /// Returns false if the new lengths would collide with neighbours (nothing changed).
    pub fn set_speed(&mut self, ids: &[Id], speed: f64, reverse: bool) -> bool {
        let ids = self.expand_links(ids);
        let mut tmp: Vec<(usize, usize, Clip)> = Vec::new();
        for &id in &ids {
            let Some((ti, ci)) = self.find(id) else { continue };
            let mut c = self.tracks[ti].clips[ci].clone();
            c.set_speed(speed);
            c.reverse = reverse;
            if !self.tracks[ti].fits(c.start, c.duration, &ids) {
                return false;
            }
            tmp.push((ti, ci, c));
        }
        for (ti, ci, c) in tmp {
            self.tracks[ti].clips[ci] = c;
        }
        self.tidy();
        true
    }

    // ---------- motion helpers ----------
    /// Make two abutting clips "flow": for every visual property animated on either clip, the outgoing
    /// value/velocity of `a` continues into `b` (a gets an ease-in to the cut, b an ease-out from it, and
    /// both meet at the same value at the cut). Returns false if the clips don't abut.
    pub fn flow_clips(&mut self, a: Id, b: Id) -> bool {
        let (Some(ca), Some(cb)) = (self.clip(a).cloned(), self.clip(b).cloned()) else { return false };
        if (ca.end() - cb.start).abs() > ABUT_EPS {
            return false;
        }
        let props = ["x", "y", "scale", "rotation", "opacity"];
        fn pick(c: &mut Clip, i: usize) -> &mut Animated {
            match i {
                0 => &mut c.x,
                1 => &mut c.y,
                2 => &mut c.scale,
                3 => &mut c.rotation,
                _ => &mut c.opacity,
            }
        }
        let (da, db) = (ca.duration, cb.duration);
        let mut na = ca.clone();
        let mut nb = cb.clone();
        for i in 0..props.len() {
            let va_end = pick(&mut na, i).at(da);
            let vb_start = pick(&mut nb, i).at(0.0);
            let animated = pick(&mut na, i).is_animated() || pick(&mut nb, i).is_animated();
            if !animated {
                continue;
            }
            let meet = (va_end + vb_start) / 2.0;
            let pa = pick(&mut na, i);
            if !pa.is_animated() {
                pa.toggle_key(0.0);
            }
            pa.set_at(da, meet);
            if let Some(k) = pa.keys.iter_mut().rev().nth(1) {
                k.ease = Ease::EaseIn;
            }
            let pb = pick(&mut nb, i);
            if !pb.is_animated() {
                pb.toggle_key(db);
            }
            pb.set_at(0.0, meet);
            if let Some(k) = pb.keys.first_mut() {
                k.ease = Ease::EaseOut;
            }
        }
        *self.clip_mut(a).unwrap() = na;
        *self.clip_mut(b).unwrap() = nb;
        true
    }
}
