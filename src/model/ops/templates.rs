use crate::model::*;

impl Project {
    // ---------- templates ----------
    /// Place a saved group of clips (times relative to the group start) at `at`. Assets are re-added by
    /// path (ids remapped); clip/link ids are fresh; tracks chosen by kind in order (new ones when blocked).
    pub fn place_clips(&mut self, clips: Vec<Clip>, assets: Vec<Asset>, at: f64) -> Vec<Id> {
        self.place_template(clips, assets, Vec::new(), &[], at).0
    }

    /// `place_clips` for a whole template: `layers[i]` is clip i's track among the template's tracks of its
    /// kind (0 = the lowest video track / the first audio track), so text stays over footage; empty (an
    /// older template) = each clip on the first track with room. `transitions` name their clip by its id in
    /// the template. Returns the new ids in clip order and old id -> new id.
    pub fn place_template(
        &mut self,
        clips: Vec<Clip>,
        assets: Vec<Asset>,
        transitions: Vec<Transition>,
        layers: &[u32],
        at: f64,
    ) -> (Vec<Id>, std::collections::HashMap<Id, Id>) {
        use std::collections::HashMap;
        let mut asset_map: HashMap<Id, Id> = HashMap::new();
        for a in assets {
            let old = a.id;
            let new = self.add_asset(a);
            asset_map.insert(old, new);
        }
        let layered = layers.len() == clips.len() && !clips.is_empty();
        let mut link_map: HashMap<Id, Id> = HashMap::new();
        let mut group_map: HashMap<Id, Id> = HashMap::new();
        let mut ids: HashMap<Id, Id> = HashMap::new();
        let mut out = Vec::new();
        let mut pictures: Vec<(Id, Id)> = Vec::new(); // (link, sequence) of Sequence clips placed on video
        let mut ready: Vec<(Clip, TrackKind, u32)> = Vec::new();
        for (i, mut c) in clips.into_iter().enumerate() {
            let (old_id, old_link) = (c.id, c.link);
            c.id = self.new_id();
            // clip markers carry ids too: a copy that kept them would shadow the original in `marker_mut`
            for i in 0..c.markers.len() {
                c.markers[i].id = self.new_id();
            }
            if c.link != 0 {
                let l = *link_map.entry(c.link).or_insert_with(|| 0);
                c.link = if l == 0 {
                    let nl = self.new_id();
                    link_map.insert(c.link, nl);
                    nl
                } else {
                    l
                };
            }
            // a pasted group is a new group, not more members of the copied one
            if c.group != 0 {
                c.group = match group_map.get(&c.group) {
                    Some(&g) => g,
                    None => {
                        let g = self.new_id();
                        group_map.insert(c.group, g);
                        g
                    }
                };
            }
            if c.uses_asset() && c.asset != 0 {
                match asset_map.get(&c.asset) {
                    Some(&a) => c.asset = a,
                    None => continue,
                }
            }
            if c.kind == ClipKind::Sequence && self.sequence(c.sequence).is_none() {
                continue;
            }
            c.start += at;
            // a nested sequence's sound half follows its picture half (captured in track order, video
            // first) back onto an audio track; an audio-only sequence has no picture half
            let sound = c.kind == ClipKind::Sequence
                && ((old_link != 0 && pictures.contains(&(old_link, c.sequence)))
                    || !self.sequence_halves(c.sequence).0);
            if c.kind == ClipKind::Sequence && !sound {
                pictures.push((old_link, c.sequence));
            }
            let kind = if c.kind == ClipKind::Audio || sound { TrackKind::Audio } else { TrackKind::Video };
            ids.insert(old_id, c.id);
            out.push(c.id);
            if layered {
                ready.push((c, kind, layers[i]));
                continue;
            }
            let ti = match c.kind {
                // ws:browser-layer: a browser window only makes sense above what it shows
                ClipKind::Browser => self.track_above_all(c.start, c.duration),
                _ => self.find_free_track(kind, c.start, c.duration, None),
            };
            self.tracks[ti].clips.push(c);
            self.tracks[ti].sort();
        }
        // layered: one template track at a time, bottom first, each on a track above the one before it
        for kind in [TrackKind::Video, TrackKind::Audio] {
            let mut ls: Vec<u32> = ready.iter().filter(|r| r.1 == kind).map(|r| r.2).collect();
            ls.sort_unstable();
            ls.dedup();
            let mut floor = 0usize; // position among this kind's tracks the next layer may start at
            for l in ls {
                let (mine, rest): (Vec<_>, Vec<_>) = ready.into_iter().partition(|r| r.1 == kind && r.2 == l);
                ready = rest;
                let list = if kind == TrackKind::Video { self.video_tracks() } else { self.audio_tracks() };
                let hit = (floor..list.len())
                    .find(|&p| mine.iter().all(|(c, ..)| self.tracks[list[p]].fits(c.start, c.duration, &[])));
                let (pos, ti) = match hit {
                    Some(p) => (p, list[p]),
                    None => (list.len(), self.add_track(kind)),
                };
                floor = pos + 1;
                self.tracks[ti].clips.extend(mine.into_iter().map(|r| r.0));
                self.tracks[ti].sort();
            }
        }
        for mut t in transitions {
            let Some(&right) = ids.get(&t.right) else { continue };
            let Some(ti) = self.track_of(right) else { continue };
            t.id = self.new_id();
            t.right = right;
            self.tracks[ti].transitions.push(t);
        }
        (out, ids)
    }

    /// Put `asset` into the slot (container clip) `clip`; its linked sound half follows, or goes when the
    /// footage has no sound. A video shorter than the slot is slowed to fill it, a longer one is trimmed.
    /// `stretch`: the slot takes the footage's length instead, and the clips of `group` (the template it
    /// came with) after it move along; ones lying across its end grow or shrink with it.
    pub fn fill_slot(&mut self, clip: Id, asset: Id, stretch: bool, group: &[Id]) -> bool {
        let Some(a) = self.asset(asset).cloned() else { return false };
        let Some(c) = self.clip(clip).cloned() else { return false };
        if !c.container || c.kind == ClipKind::Audio || a.kind == ClipKind::Audio {
            return false;
        }
        let silent = a.kind != ClipKind::Video || a.audio_streams.is_empty();
        let halves: Vec<Id> = self
            .all_clips()
            .filter(|(_, o)| c.link != 0 && o.link == c.link && o.id != clip && o.kind == ClipKind::Audio)
            .map(|(_, o)| o.id)
            .collect();
        if silent && !halves.is_empty() {
            for t in &mut self.tracks {
                t.clips.retain(|o| !halves.contains(&o.id));
            }
        }
        if !self.replace_container_pair(clip, asset) {
            return false;
        }
        let pair: Vec<Id> = std::iter::once(clip).chain(halves.iter().copied().filter(|_| !silent)).collect();
        // (an image, or footage whose length is not known yet, leaves the timing alone)
        let len = if a.kind == ClipKind::Video { a.duration } else { 0.0 };
        if len <= MIN_CLIP {
            return true;
        }
        if !stretch {
            if len < c.duration {
                for id in pair {
                    if let Some(o) = self.clip_mut(id) {
                        o.speed = (len / c.duration).max(0.01);
                    }
                }
            }
            return true;
        }
        let (old_end, delta) = (c.end(), len - c.duration);
        for t in &mut self.tracks {
            for o in &mut t.clips {
                if pair.contains(&o.id) {
                    stretch_clip(o, len);
                } else if !group.contains(&o.id) {
                    // not part of the template: stays where it is
                } else if o.start >= old_end - EPS {
                    o.start += delta;
                } else if o.end() > old_end + EPS || ((o.end() - old_end).abs() <= EPS && o.start <= c.start + EPS) {
                    // lies across the slot's end, or covers the slot to its end (a caption over it)
                    let d = (o.duration + delta).max(MIN_CLIP);
                    stretch_clip(o, d);
                }
            }
            t.sort();
        }
        true
    }
}

/// Give `c` a new length and keep its animation in place: keys (and blocks) in the first half stay with the
/// start, the rest move with the end; a clip that gets shorter is scaled instead (nothing may cross).
fn stretch_clip(c: &mut Clip, dur: f64) {
    let old = c.duration;
    if old <= MIN_CLIP || (dur - old).abs() < EPS {
        return;
    }
    let map = |t: f64| {
        if dur < old {
            t * dur / old
        } else if t > old / 2.0 {
            t + dur - old
        } else {
            t
        }
    };
    for p in BLOCK_PROPS {
        if let Some(a) = prop_mut(c, p) {
            for k in &mut a.keys {
                k.t = map(k.t);
            }
        }
    }
    for b in &mut c.blocks {
        let (s, e) = (map(b.t), map(b.end()));
        b.t = s;
        b.dur = (e - s).max(0.0);
    }
    c.duration = dur;
}
