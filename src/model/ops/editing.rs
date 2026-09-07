use crate::model::ops::trim;
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
    /// Thin wrapper (byte-identical behaviour) over `insert_asset_clips_ranged` with `range: None`.
    pub fn insert_asset_clips(&mut self, asset_id: Id, at: f64, video_track: Option<usize>) -> Vec<Id> {
        self.insert_asset_clips_ranged(asset_id, at, video_track, None, None)
    }

    /// Three-point-editing primitive: like `insert_asset_clips`, but `range` (source seconds)
    /// overrides the default `[0, duration)` window, and `audio_track` prefers a specific track for
    /// stream 0 (later streams still fall back to `audio_tracks()[i]`, same as before).
    pub fn insert_asset_clips_ranged(
        &mut self,
        asset_id: Id,
        at: f64,
        video_track: Option<usize>,
        audio_track: Option<usize>,
        range: Option<(f64, f64)>,
    ) -> Vec<Id> {
        let Some(asset) = self.asset(asset_id).cloned() else { return Vec::new() };
        // ---- ws:media-library ----
        // a subclip (`Asset.range`, wave 0b) places only its own window unless the caller asked for a
        // narrower one - otherwise every subclip would start at source 0 like its parent
        let range = range.or(asset.range);
        let (src_in, dur) = match range {
            Some((s, e)) => (s, (e - s).max(MIN_CLIP)),
            None => (0.0, if asset.kind == ClipKind::Image { 5.0 } else { asset.duration.max(MIN_CLIP) }),
        };
        let n_parts = asset.has_video() as usize + asset.audio_streams.len();
        let link = if n_parts > 1 { self.new_id() } else { 0 };
        let mut ids = Vec::new();
        if asset.has_video() {
            let ti = self.find_free_track(TrackKind::Video, at, dur, video_track);
            let mut c = Clip::new(self.new_id(), asset.kind, asset.name(), at, dur);
            c.asset = asset.id;
            c.src_in = src_in;
            c.link = link;
            ids.push(c.id);
            self.tracks[ti].clips.push(c);
            self.tracks[ti].sort();
        }
        for (i, s) in asset.audio_streams.iter().enumerate() {
            let prefer = if i == 0 { audio_track } else { None }.or_else(|| self.audio_tracks().get(i).copied());
            let ti = self.find_free_track(TrackKind::Audio, at, dur, prefer);
            let name =
                if asset.audio_streams.len() > 1 { format!("{} [{}]", asset.name(), s.label()) } else { asset.name() };
            let mut c = Clip::new(self.new_id(), ClipKind::Audio, name, at, dur);
            c.asset = asset.id;
            c.audio_stream = s.index;
            c.src_in = src_in;
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
        let ids = split_tracks_at(&mut self.tracks, t, only, &mut self.next_id);
        self.tidy();
        ids
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

    /// Delete clips. With `ripple`, later clips close the gap (per track, only where no other clip
    /// overlaps the gap). `ripple` here is the pre-existing, call-site-chosen toggle (ripple-delete
    /// vs. leave-a-gap) - deliberately every track unconditionally, NOT scoped to `ripple_tracks()`:
    /// this fn is a general-purpose primitive with many pre-existing callers (autocut, the RippleDelete
    /// hotkey, timeline drag-delete, lossless-cut's segment builder) that already rely on every track
    /// staying in sync when they ask for a ripple delete, regardless of the new per-track ripple flag.
    /// Only the two call sites the plan names (`RippleDeleteInOut`/`PasteInsert`, in actions.rs) move to
    /// `ripple_tracks()`-scoped ops (`ripple_delete_range`/`ripple_open`) - this fn's own behaviour is
    /// intentionally unchanged.
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
            let all: Vec<usize> = (0..self.tracks.len()).collect();
            for (a, b) in ranges {
                self.close_gap(a, b, &all);
            }
        }
        self.tidy();
    }

    /// Shift clips starting at/after `b` left by (b-a), on `tracks` only, where [a,b) is free on that
    /// track. EXTENDED (was: every track, unconditionally) - scoped to `tracks`. Deliberately does NOT
    /// call `shift_time` itself: `delete_clips`'s ripple branch also calls this (with every track, to
    /// keep its own long-standing behaviour exactly as it was), and several pre-existing callers of
    /// `delete_clips(ids, true)` (autocut, `subtitles_ui::cut_dups`) already manage their own
    /// marker/cue ripple mapping independently - folding `shift_time` in here double-shifted them
    /// (`cut_dups`'s own cue remap on top of this one) and was caught by the existing
    /// `duplicate_takes_are_marked_then_cut_with_the_cues` test. `ripple_delete_range`/`close_gap_at`
    /// below call `shift_time` themselves after this, for exactly the two callers (the extended legacy
    /// `RippleDeleteInOut`/`PasteInsert` actions, and every new trim-model op) that want it.
    fn close_gap(&mut self, a: f64, b: f64, tracks: &[usize]) {
        let len = b - a;
        if len <= 0.0 {
            return;
        }
        for &ti in tracks {
            let Some(t) = self.tracks.get_mut(ti) else { continue };
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

    /// Like `split_at`, but scoped by TRACK INDEX instead of an id allow-list. `split_at`'s `only:
    /// &[Id]` filter is an O(k) linear `.contains()` per clip - fine for the handful of ids its
    /// existing callers pass, but the ripple ops below would need to pass every clip id on the
    /// scoped tracks, turning that into O(n) per clip and O(n^2) overall (caught by
    /// `splice_in_is_linear_on_1000_clips`). Scoping by the (small) track-index list instead keeps
    /// every step below genuinely O(n).
    pub(crate) fn split_at_scoped(&mut self, t: f64, tracks: &[usize]) -> Vec<Id> {
        let mut new_ids = Vec::new();
        let mut link_map: std::collections::HashMap<Id, Id> = std::collections::HashMap::new();
        for &ti in tracks {
            let n = self.tracks.get(ti).map(|tr| tr.clips.len()).unwrap_or(0);
            let mut added = Vec::new();
            for ci in 0..n {
                let c = &self.tracks[ti].clips[ci];
                if !c.contains(t) {
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
        new_ids
    }

    /// Remove everything in [a,b) on `tracks` and close the gap there. EXTENDED (was: every track
    /// unconditionally, and discarded the removed ids) - scoped to `tracks` and returns the removed
    /// ids, so `Project::extract_range` (trim.rs) can be a one-line wrapper instead of reimplementing
    /// split+delete+close_gap. O(n): one linear scan per step, no per-clip `move_clips` walk. Calls
    /// `shift_time` itself (unlike the private `close_gap` it uses - see that fn's doc comment).
    pub fn ripple_delete_range(&mut self, a: f64, b: f64, tracks: &[usize]) -> Vec<Id> {
        if b <= a + EPS || tracks.is_empty() {
            return Vec::new();
        }
        let set: std::collections::HashSet<usize> = tracks.iter().copied().collect();
        self.split_at_scoped(a, tracks);
        self.split_at_scoped(b, tracks);
        let ids: Vec<Id> = self
            .all_clips()
            .filter(|(ti, c)| set.contains(ti) && c.start >= a - EPS && c.end() <= b + EPS)
            .map(|(_, c)| c.id)
            .collect();
        self.delete_clips(&ids, false);
        self.close_gap(a, b, tracks);
        self.shift_time(b, -(b - a), tracks);
        self.tidy();
        ids
    }

    /// Open a gap of `span` seconds at `at` on `tracks`: split anything of theirs crossing it, then
    /// shift every one of their clips at/after `at` right by `span` in one linear pass (a uniform
    /// shift preserves every relative gap/overlap, so no per-clip collision check is needed - this
    /// is what makes it O(n) instead of the old per-clip `move_clips` walk, each of which rescanned
    /// the track). The inverse of `ripple_delete_range`, used by Paste Insert and `splice_in`.
    /// EXTENDED: was every track unconditionally; now scoped to `tracks`.
    pub fn ripple_open(&mut self, at: f64, span: f64, tracks: &[usize]) {
        if span <= EPS || tracks.is_empty() {
            return;
        }
        self.split_at_scoped(at, tracks);
        for &ti in tracks {
            let Some(t) = self.tracks.get_mut(ti) else { continue };
            for c in &mut t.clips {
                if c.start >= at - EPS {
                    c.start += span;
                }
            }
        }
        self.shift_time(at, span, tracks);
        self.tidy();
    }

    /// Finds the local gap bounds under `(track, t)` - `[previous clip's end, next clip's start)` -
    /// and closes it, scoped to `ripple_tracks()`. False if `t` isn't actually a gap, or there's
    /// nothing after it to pull left.
    pub fn close_gap_at(&mut self, track: usize, t: f64) -> bool {
        let Some(tr) = self.tracks.get(track) else { return false };
        if tr.locked || tr.clips.iter().any(|c| c.contains(t)) {
            return false;
        }
        let a = tr.clips.iter().filter(|c| c.end() <= t + EPS).map(|c| c.end()).fold(0.0_f64, f64::max);
        let b = tr
            .clips
            .iter()
            .filter(|c| c.start >= t - EPS)
            .map(|c| c.start)
            .fold(None, |acc: Option<f64>, x| Some(acc.map_or(x, |m: f64| m.min(x))));
        let Some(b) = b else { return false };
        if b <= a + EPS {
            return false;
        }
        let tracks = self.ripple_tracks();
        self.close_gap(a, b, &tracks);
        self.shift_time(b, -(b - a), &tracks);
        true
    }

    /// Shifts `Project.markers` WHERE `m.sequence == self.editing` (main-timeline markers have
    /// `sequence == None`) and `in_point`/`out_point`, at/after `from`, by `dt` seconds. While a
    /// sequence is open it never touches `Project.subtitles` (`Cue` carries no sequence tag - cues
    /// are always main-timeline-relative; ponytail: a documented ceiling, not a bug - see notes.md).
    /// `tracks` gates the whole call: nothing rippled (an empty scope) means nothing else should
    /// move either. Called by every ripple op in trim.rs, plus `close_gap`/`ripple_open` above (so
    /// the legacy `RippleDeleteInOut`/`PasteInsert` actions get marker/cue shifting for free too).
    pub fn shift_time(&mut self, from: f64, dt: f64, tracks: &[usize]) {
        if tracks.is_empty() || dt.abs() < EPS {
            return;
        }
        let editing = self.editing;
        for m in &mut self.markers {
            if m.sequence == editing && m.t >= from - EPS {
                m.t = (m.t + dt).max(0.0);
            }
        }
        if editing.is_none() {
            for c in &mut self.subtitles {
                if c.start >= from - EPS {
                    c.start = (c.start + dt).max(0.0);
                    c.end = (c.end + dt).max(0.0);
                }
            }
        }
        if let Some(ip) = self.in_point {
            if ip >= from - EPS {
                self.in_point = Some((ip + dt).max(0.0));
            }
        }
        if let Some(op) = self.out_point {
            if op >= from - EPS {
                self.out_point = Some((op + dt).max(0.0));
            }
        }
        self.sort_markers();
    }

    /// Clip ids starting at/after (`backward=false`) or strictly before (`backward=true`) `t`,
    /// optionally restricted to one track - `A` / `Shift+A`'s select-forward/backward.
    pub fn clips_from(&self, t: f64, track: Option<usize>, backward: bool) -> Vec<Id> {
        self.all_clips()
            .filter(|(ti, c)| {
                track.map(|x| x == *ti).unwrap_or(true) && if backward { c.start < t - EPS } else { c.end() > t + EPS }
            })
            .map(|(_, c)| c.id)
            .collect()
    }

    /// Clip ids covering `t` on any track - `Ctrl+Shift+D`'s select-under-playhead.
    pub fn clips_at(&self, t: f64) -> Vec<Id> {
        self.all_clips().filter(|(_, c)| c.contains(t)).map(|(_, c)| c.id).collect()
    }

    /// Nearest clip boundary to `t` (optionally restricted to one track) as an `EditPoint` - `U`'s
    /// select-nearest-edit-point. `side` is `Both` when the boundary is shared by two abutting clips,
    /// else the single side actually present (a track's own leading/trailing edge).
    pub fn nearest_edit_point(&self, t: f64, track: Option<usize>) -> Option<trim::EditPoint> {
        let tracks: Vec<usize> = match track {
            Some(ti) => vec![ti],
            None => (0..self.tracks.len()).collect(),
        };
        let mut best: Option<(usize, f64, f64)> = None; // (track, boundary_t, distance)
        for ti in tracks {
            let Some(tr) = self.tracks.get(ti) else { continue };
            for c in &tr.clips {
                for b in [c.start, c.end()] {
                    let d = (b - t).abs();
                    if best.map(|(_, _, bd)| d < bd).unwrap_or(true) {
                        best = Some((ti, b, d));
                    }
                }
            }
        }
        let (ti, bt, _) = best?;
        let tr = &self.tracks[ti];
        let has_left = tr.clips.iter().any(|c| (c.end() - bt).abs() < ABUT_EPS);
        let has_right = tr.clips.iter().any(|c| (c.start - bt).abs() < ABUT_EPS);
        let side = match (has_left, has_right) {
            (true, true) => trim::Side::Both,
            (true, false) => trim::Side::Left,
            (false, true) => trim::Side::Right,
            (false, false) => return None,
        };
        Some(trim::EditPoint { track: ti, t: bt, side })
    }

    /// Sets `in_point`/`out_point` from a clip's `[start, end)` - `clip` defaults to the clip under
    /// `playhead` when `None`. Shared by the `X` ("Mark Clip") keyboard action and the `timeline.mark`
    /// MCP tool so the logic exists once. `None` (no clip found either way) leaves in/out untouched.
    pub fn mark_from_clip(&mut self, clip: Option<Id>, playhead: f64) -> Option<(f64, f64)> {
        let id = clip.or_else(|| self.clips_at(playhead).first().copied())?;
        let c = self.clip(id)?;
        let (s, e) = (c.start, c.end());
        self.in_point = Some(s);
        self.out_point = Some(e);
        Some((s, e))
    }

    /// Keep only [a,b), moving it to start at 0. Clears in/out points. Every track (not just
    /// `ripple_tracks()`): this crops the whole project down to a range, so every track must stay in
    /// sync - a position-locked secondary track left behind would desync forever, not just for one edit.
    pub fn trim_to_range(&mut self, a: f64, b: f64) {
        let end = self.duration().max(b) + 1.0;
        let all: Vec<usize> = (0..self.tracks.len()).collect();
        self.ripple_delete_range(b, end, &all);
        self.ripple_delete_range(0.0, a, &all);
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

// ---- ws:pro-monitor ----
/// The body of `Project::split_at`, generic over any track vec (not `&mut self`) so
/// `Project::multicam_switch` (model/ops/multicam.rs) can split a nested `Sequence`'s own `tracks`
/// field directly, with no `main_stash` swap through `open_sequence`/`close_sequence`. `next_id` is a
/// `&mut Id` (the counter `Project::new_id` increments) rather than `&mut Project`/a closure, since that
/// is the one piece of `self` the original body needed. Callers over `self.tracks` (`split_at`) still
/// call `self.tidy()` themselves afterward; `multicam_switch` doesn't need to (no gaps are created).
pub(super) fn split_tracks_at(tracks: &mut Vec<Track>, t: f64, only: Option<&[Id]>, next_id: &mut Id) -> Vec<Id> {
    let mut new_ids = Vec::new();
    let mut link_map: std::collections::HashMap<Id, Id> = std::collections::HashMap::new();
    let mut gen_id = || {
        *next_id += 1;
        *next_id
    };
    for ti in 0..tracks.len() {
        let mut added = Vec::new();
        for ci in 0..tracks[ti].clips.len() {
            let c = &tracks[ti].clips[ci];
            if !c.contains(t) || only.map(|o| !o.contains(&c.id)).unwrap_or(false) {
                continue;
            }
            let old_link = c.link;
            let nid = gen_id();
            let new_link = if old_link != 0 {
                match link_map.get(&old_link) {
                    Some(&l) => l,
                    None => {
                        let l = gen_id();
                        link_map.insert(old_link, l);
                        l
                    }
                }
            } else {
                0
            };
            if let Some(mut right) = tracks[ti].clips[ci].split(t, nid) {
                right.link = new_link;
                new_ids.push(right.id);
                added.push(right);
            }
        }
        if !added.is_empty() {
            tracks[ti].clips.extend(added);
            tracks[ti].sort();
        }
    }
    new_ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipKind, TrackKind};

    fn clip(id: Id, name: &str, start: f64, dur: f64) -> Clip {
        Clip::new(id, ClipKind::Video, name, start, dur)
    }

    /// `Project::split_at` over `self.tracks` must behave identically before and after the extraction -
    /// pure refactor, pinned against the pre-extraction shape (split a clip in two, right half gets a
    /// fresh id, linked clips on other tracks split at the same point and share a fresh link id).
    #[test]
    fn split_tracks_at_matches_old_split_at_behavior() {
        let mut p = Project::new();
        let a = p.new_id();
        p.tracks[0].clips.push(clip(a, "a", 0.0, 10.0));
        let b = p.new_id();
        p.tracks[1].clips.push(clip(b, "b", 0.0, 10.0));
        p.tracks[0].clips[0].link = 5;
        p.tracks[1].clips[0].link = 5;
        let new_ids = p.split_at(4.0, None);
        assert_eq!(new_ids.len(), 2, "both linked clips split");
        let left_a = p.clip(a).unwrap();
        assert_eq!(left_a.duration, 4.0);
        let right_a = p.tracks[0].clips.iter().find(|c| c.id != a).expect("track 0 has a's right half");
        assert_eq!(right_a.duration, 6.0);
        // right halves keep a shared (fresh) link id, distinct from the original 5
        let right_ids: Vec<Id> =
            p.tracks.iter().flat_map(|t| &t.clips).map(|c| c.id).filter(|&id| id != a && id != b).collect();
        assert_eq!(right_ids.len(), 2);
        let links: Vec<Id> = right_ids.iter().map(|&id| p.clip(id).unwrap().link).collect();
        assert_eq!(links[0], links[1], "right halves share one fresh link id");
        assert_ne!(links[0], 5, "not the original link id");
    }

    /// `only` restricts which clips split - a clip whose id is not in `only` is left whole, matching
    /// `split_at`'s pre-extraction `only` semantics.
    #[test]
    fn split_tracks_at_respects_only() {
        let mut tracks = vec![Track::new(1, TrackKind::Video, "V1")];
        let a = 10;
        let b = 11;
        tracks[0].clips.push(clip(a, "a", 0.0, 10.0));
        tracks[0].clips.push(clip(b, "b", 0.0, 10.0)); // deliberately overlapping - only `only` matters here
        let mut next_id = 100;
        let ids = split_tracks_at(&mut tracks, 4.0, Some(&[a]), &mut next_id);
        assert_eq!(ids.len(), 1, "only clip a splits");
        assert_eq!(tracks[0].clips.iter().filter(|c| c.name == "b").count(), 1, "b stays whole");
    }
}
