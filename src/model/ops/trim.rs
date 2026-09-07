//! ---- ws:trim-model ----
//! The trim primitive set: `EditPoint`/`Side` (the keyboard-trim selection unit — snap-engine's
//! `TimelineState.edit_point` imports these rather than redefining them) plus every clip-edge/gap
//! op (ripple/roll/slip/slide, splice/overwrite/lift/extract, join/duplicate/unnest/replace,
//! magnetic_move). Every op mutates on a clone and applies atomically (stable ids, same track
//! index, `Transition.id` kept — transitions key off `Clip.id`, never touched here) and honours
//! `Project::locked_of`. Ripple ops shift downstream content via the O(n) `close_gap`/
//! `ripple_delete_range`/`ripple_open` core (editing.rs) and `Project::shift_time` for markers/cues/
//! in-out — never per-drag-frame; a live gesture (timeline-trim-gestures, wave 2) must ghost-paint
//! and apply these on release only.

use crate::model::*;

/// One clip boundary selected for keyboard trimming (`U` / `Shift+U`). Lives here (the trim
/// primitives' natural home); `snap-engine`'s `TimelineState.edit_point: Option<EditPoint>` imports
/// this type rather than redefining it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditPoint {
    pub track: usize,
    pub t: f64,
    pub side: Side,
}

/// Which side of a boundary an edit point refers to: the outgoing clip's end (`Left`), the incoming
/// clip's start (`Right`), or both when a cut is shared by two abutting clips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Both,
}

impl Project {
    // ---------- trim primitives ----------

    /// Shift every clip (except `exclude`) starting at/after `at`, on `tracks`, by `dt` seconds — no
    /// collision check (the caller has already decided this is a shove, not a move). O(n): one pass
    /// per track.
    fn shift_clips_from(&mut self, at: f64, dt: f64, exclude: Id, tracks: &[usize]) {
        for &ti in tracks {
            let Some(t) = self.tracks.get_mut(ti) else { continue };
            for c in &mut t.clips {
                if c.id != exclude && c.start >= at - EPS {
                    c.start = (c.start + dt).max(0.0);
                }
            }
        }
    }

    /// Trim one edge of `id`. `ripple=false` keeps today's fits()-guarded all-or-nothing plain trim
    /// (refuses on a same-track collision, exactly like the timeline's existing edge-drag gesture).
    /// `ripple=true` on the END edge additionally shifts every later clip on ripple tracks by the
    /// same delta `end()` moved (and skips the same-track collision guard, since downstream shifts
    /// out of the way) — the standard "ripple trim". A START-edge trim never moves `end()`
    /// (`Clip::trim_start` keeps the right edge fixed by construction), so there is nothing
    /// downstream to shift; `ripple` there only means "also move markers/cues/in-out with the edge"
    /// via `shift_time` — the same-track collision guard against the *preceding* clip still applies.
    /// ponytail: a start-edge ripple trim never shoves the preceding clip out of the way either (that
    /// is Slide's job) — refuses (no-op) exactly like the plain path when it would collide upstream.
    pub fn ripple_trim(&mut self, id: Id, start_edge: bool, new_edge: f64, ripple: bool) -> bool {
        let Some((ti, ci)) = self.find(id) else { return false };
        if self.locked_of(ti) {
            return false;
        }
        let old = self.tracks[ti].clips[ci].clone();
        let mut c = old.clone();
        if start_edge {
            let hr = self.head_room(&c);
            c.trim_start(new_edge, hr);
        } else {
            let md = self.max_clip_duration(&c);
            c.trim_end(new_edge, md);
        }
        if (c.start - old.start).abs() < EPS && (c.duration - old.duration).abs() < EPS {
            return false; // clamped straight back: nothing changed
        }
        let skip_fits = ripple && !start_edge;
        if !skip_fits && !self.tracks[ti].fits(c.start, c.duration, &[id]) {
            return false;
        }
        self.tracks[ti].clips[ci] = c.clone();
        if ripple {
            let tracks = self.ripple_tracks();
            if start_edge {
                self.shift_time(old.start, c.start - old.start, &tracks);
            } else {
                let dend = c.end() - old.end();
                if dend.abs() > EPS {
                    self.shift_clips_from(old.end(), dend, id, &tracks);
                }
                self.shift_time(old.end(), dend, &tracks);
            }
        }
        self.tidy();
        true
    }

    /// Move the shared cut between `right` and its left neighbour on the same track, keeping the sum
    /// of their durations (and every other clip's position) unchanged. Both sides are clamped to
    /// their own asset boundaries (like a plain trim); refuses all-or-nothing if either side can't
    /// actually reach `new_cut` or the result would overlap anything else.
    pub fn roll_edit(&mut self, right: Id, new_cut: f64) -> bool {
        let Some(ti) = self.track_of(right) else { return false };
        if self.locked_of(ti) {
            return false;
        }
        let t = &self.tracks[ti];
        let Some(r) = t.clips.iter().find(|c| c.id == right) else { return false };
        let Some(l) = t.left_of(r) else { return false };
        let (mut left, mut rt) = (l.clone(), r.clone());
        let old_cut = rt.start;
        let cut = new_cut.clamp(left.start + MIN_CLIP, rt.end() - MIN_CLIP);
        if (cut - old_cut).abs() < EPS {
            return false;
        }
        let md = self.max_clip_duration(&left);
        left.trim_end(cut, md);
        let hr = self.head_room(&rt);
        rt.trim_start(cut, hr);
        if (left.end() - cut).abs() > EPS || (rt.start - cut).abs() > EPS {
            return false; // an asset boundary clamped one side short of the requested cut
        }
        let ids = [left.id, rt.id];
        if !self.tracks[ti].fits(left.start, left.duration, &ids) || !self.tracks[ti].fits(rt.start, rt.duration, &ids)
        {
            return false;
        }
        let li = self.tracks[ti].clips.iter().position(|c| c.id == left.id).unwrap();
        let ri = self.tracks[ti].clips.iter().position(|c| c.id == rt.id).unwrap();
        self.tracks[ti].clips[li] = left;
        self.tracks[ti].clips[ri] = rt;
        self.tidy();
        true
    }

    /// Change the source window in place (start/duration untouched, only `src_in` moves), clamped to
    /// `[0, asset.duration - src_len]` — the occupied source window either direction plays it, so the
    /// bound is the same for a reversed clip as a forward one. Per-clip clamp (not all-or-nothing):
    /// returns true iff at least one clip actually moved.
    pub fn slip(&mut self, ids: &[Id], dsrc: f64) -> bool {
        let mut changed = false;
        for &id in ids {
            let Some((ti, ci)) = self.find(id) else { continue };
            if self.locked_of(ti) {
                continue;
            }
            let c = &self.tracks[ti].clips[ci];
            if !matches!(c.kind, ClipKind::Video | ClipKind::Audio) || c.freeze.is_some() {
                continue;
            }
            let Some(a) = self.asset(c.asset) else { continue };
            let max_in = (a.duration - c.src_len()).max(0.0);
            let new_in = (c.src_in + dsrc).clamp(0.0, max_in);
            if (new_in - c.src_in).abs() > EPS {
                self.tracks[ti].clips[ci].src_in = new_in;
                changed = true;
            }
        }
        changed
    }

    /// Move `id` by `dt`; its content (src window, duration) is fixed, and its immediate neighbours
    /// on the same track absorb the change (their own trim methods keep them asset-boundary-safe).
    /// Clamped so neither neighbour would shrink below `MIN_CLIP`; refuses if an asset boundary would
    /// leave either neighbour short of the new shared edge.
    pub fn slide(&mut self, id: Id, dt: f64) -> bool {
        let Some(ti) = self.track_of(id) else { return false };
        if self.locked_of(ti) {
            return false;
        }
        let t = &self.tracks[ti];
        let Some(c) = t.clips.iter().find(|c| c.id == id).cloned() else { return false };
        let left = t.left_of(&c).cloned();
        let right = t.clips.iter().find(|o| o.id != id && (o.start - c.end()).abs() < ABUT_EPS).cloned();
        let min_start = left.as_ref().map(|l| l.start + MIN_CLIP).unwrap_or(0.0);
        let max_start = right.as_ref().map(|r| r.end() - c.duration - MIN_CLIP).unwrap_or(f64::INFINITY);
        if max_start < min_start {
            return false; // no room to slide at all
        }
        let new_start = (c.start + dt).clamp(min_start, max_start);
        if (new_start - c.start).abs() < EPS {
            return false;
        }
        if let Some(mut l) = left {
            let md = self.max_clip_duration(&l);
            l.trim_end(new_start, md);
            if (l.end() - new_start).abs() > EPS {
                return false; // an asset boundary would leave a gap or overlap — refuse, don't desync
            }
            let li = self.tracks[ti].clips.iter().position(|o| o.id == l.id).unwrap();
            self.tracks[ti].clips[li] = l;
        }
        if let Some(mut r) = right {
            let new_end = new_start + c.duration;
            let hr = self.head_room(&r);
            r.trim_start(new_end, hr);
            if (r.start - new_end).abs() > EPS {
                return false;
            }
            let ri = self.tracks[ti].clips.iter().position(|o| o.id == r.id).unwrap();
            self.tracks[ti].clips[ri] = r;
        }
        let ci = self.tracks[ti].clips.iter().position(|o| o.id == id).unwrap();
        self.tracks[ti].clips[ci].start = new_start;
        self.tidy();
        true
    }

    /// Asymmetric multi-roller trim: every listed `(clip_id, is_start)` edge moves by the same `dt`,
    /// all-or-nothing. `ripple=true` only lifts the same-call collision guard between the listed
    /// edges (so multiple rollers can pass each other); it does not also shove a non-participant
    /// clip out of the way — call `ripple_trim` per edge for that.
    /// ponytail: no downstream shift for the ripple case here (pro-timeline's asymmetric-trim UI is
    /// expected to compose `ripple_trim` per edge when a true ripple shift across a multi-roller set
    /// is wanted); this keeps the all-or-nothing multi-clip case simple and correctly refusable.
    pub fn trim_edges(&mut self, edges: &[(Id, bool)], dt: f64, ripple: bool) -> bool {
        if edges.is_empty() {
            return false;
        }
        let participants: Vec<Id> = edges.iter().map(|(id, _)| *id).collect();
        let mut updates = Vec::with_capacity(edges.len());
        for &(id, is_start) in edges {
            let Some((ti, ci)) = self.find(id) else { return false };
            if self.locked_of(ti) {
                return false;
            }
            let mut c = self.tracks[ti].clips[ci].clone();
            if is_start {
                let hr = self.head_room(&c);
                c.trim_start(c.start + dt, hr);
            } else {
                let md = self.max_clip_duration(&c);
                c.trim_end(c.end() + dt, md);
            }
            updates.push((ti, ci, c));
        }
        if !ripple {
            for (ti, _, c) in &updates {
                if !self.tracks[*ti].fits(c.start, c.duration, &participants) {
                    return false;
                }
            }
        }
        let mut any = false;
        for (ti, ci, c) in updates {
            let old = &self.tracks[ti].clips[ci];
            if (old.start - c.start).abs() > EPS || (old.duration - c.duration).abs() > EPS {
                any = true;
            }
            self.tracks[ti].clips[ci] = c;
        }
        if any {
            self.tidy();
        }
        any
    }

    /// `E`: extend the edit point to `to`. `Side::Both` (a shared cut) rolls it; a single side is a
    /// one-edge ripple trim, rippling iff the track is magnetic (matches the plain-edge-drag rule:
    /// only a magnetic track's plain edits ripple by default).
    pub fn extend_edit(&mut self, ep: &EditPoint, to: f64) -> bool {
        let Some(tr) = self.tracks.get(ep.track) else { return false };
        match ep.side {
            Side::Both => {
                let Some(right_id) = tr.clips.iter().find(|c| (c.start - ep.t).abs() < ABUT_EPS).map(|c| c.id) else {
                    return false;
                };
                self.roll_edit(right_id, to)
            }
            Side::Left => {
                // outgoing side: the clip ENDING at this boundary — trim its end edge.
                let Some(id) = tr.clips.iter().find(|c| (c.end() - ep.t).abs() < ABUT_EPS).map(|c| c.id) else {
                    return false;
                };
                let ripple = self.tracks[ep.track].magnetic;
                self.ripple_trim(id, false, to, ripple)
            }
            Side::Right => {
                // incoming side: the clip STARTING at this boundary — trim its start edge.
                let Some(id) = tr.clips.iter().find(|c| (c.start - ep.t).abs() < ABUT_EPS).map(|c| c.id) else {
                    return false;
                };
                let ripple = self.tracks[ep.track].magnetic;
                self.ripple_trim(id, true, to, ripple)
            }
        }
    }

    /// Overwrite edit: clears [at, at+dur) on the resolved video/audio track(s) only (split at both
    /// bounds, delete what's fully inside — other tracks are untouched), then places the asset via
    /// `insert_asset_clips_ranged`. No ripple.
    pub fn overwrite_asset(
        &mut self,
        asset: Id,
        at: f64,
        video_track: Option<usize>,
        audio_track: Option<usize>,
        range: Option<(f64, f64)>,
    ) -> Vec<Id> {
        let Some(a) = self.asset(asset).cloned() else { return Vec::new() };
        let dur = clip_span(&a, range);
        let end = at + dur;
        let vt = video_track.or_else(|| self.video_tracks().first().copied());
        let at_track = audio_track.or_else(|| self.audio_tracks().first().copied());
        let mut targets: Vec<usize> = Vec::new();
        if a.has_video() {
            targets.extend(vt);
        }
        if !a.audio_streams.is_empty() {
            targets.extend(at_track);
        }
        if targets.iter().any(|&ti| self.locked_of(ti)) {
            return Vec::new();
        }
        self.split_at_scoped(at, &targets);
        self.split_at_scoped(end, &targets);
        let overlapped: Vec<Id> = targets
            .iter()
            .flat_map(|&ti| {
                self.tracks[ti]
                    .clips
                    .iter()
                    .filter(|c| c.start >= at - EPS && c.end() <= end + EPS)
                    .map(|c| c.id)
                    .collect::<Vec<_>>()
            })
            .collect();
        self.delete_clips(&overlapped, false);
        self.insert_asset_clips_ranged(asset, at, vt, at_track, range)
    }

    /// Splice (insert) edit: ripple-opens exactly `dur` seconds at `at` on the ripple tracks, then
    /// places the asset there. O(n): `ripple_open` shifts everything downstream in one pass instead
    /// of the old per-clip `move_clips` walk.
    pub fn splice_in(
        &mut self,
        asset: Id,
        at: f64,
        video_track: Option<usize>,
        audio_track: Option<usize>,
        range: Option<(f64, f64)>,
    ) -> Vec<Id> {
        if video_track.map(|t| self.locked_of(t)).unwrap_or(false)
            || audio_track.map(|t| self.locked_of(t)).unwrap_or(false)
        {
            return Vec::new();
        }
        let Some(a) = self.asset(asset).cloned() else { return Vec::new() };
        let dur = clip_span(&a, range);
        let tracks = self.ripple_tracks();
        self.ripple_open(at, dur, &tracks);
        self.insert_asset_clips_ranged(asset, at, video_track, audio_track, range)
    }

    /// Remove [a, b) on `tracks` (default: every track), leaving a gap — nothing shifts. Locked
    /// tracks are dropped from the scope (a mixed locked/unlocked set still lifts the unlocked ones).
    pub fn lift_range(&mut self, a: f64, b: f64, tracks: Option<&[usize]>) -> Vec<Id> {
        if b <= a + EPS {
            return Vec::new();
        }
        let owned;
        let scope: &[usize] = match tracks {
            Some(t) => t,
            None => {
                owned = (0..self.tracks.len()).collect::<Vec<_>>();
                &owned
            }
        };
        let scope: Vec<usize> = scope.iter().copied().filter(|&ti| !self.locked_of(ti)).collect();
        self.split_at_scoped(a, &scope);
        self.split_at_scoped(b, &scope);
        let set: std::collections::HashSet<usize> = scope.iter().copied().collect();
        let ids: Vec<Id> = self
            .all_clips()
            .filter(|(ti, c)| set.contains(ti) && c.start >= a - EPS && c.end() <= b + EPS)
            .map(|(_, c)| c.id)
            .collect();
        self.delete_clips(&ids, false);
        self.tidy();
        ids
    }

    /// Remove [a, b) and close the gap (default: `ripple_tracks()`) — a thin wrapper, not a
    /// reimplementation of split+delete+close_gap (that's `ripple_delete_range`, editing.rs). Locked
    /// tracks are dropped from the scope first (same rule as `lift_range`).
    pub fn extract_range(&mut self, a: f64, b: f64, tracks: Option<&[usize]>) -> Vec<Id> {
        let owned;
        let scope: &[usize] = match tracks {
            Some(t) => t,
            None => {
                owned = self.ripple_tracks();
                &owned
            }
        };
        let scope: Vec<usize> = scope.iter().copied().filter(|&ti| !self.locked_of(ti)).collect();
        self.ripple_delete_range(a, b, &scope)
    }

    /// Merge `left` with its right neighbour on the same track when they're the same asset with
    /// contiguous, non-reversed source time and matching speed — keeps `left`'s id.
    pub fn join_through(&mut self, left: Id) -> bool {
        let Some(ti) = self.track_of(left) else { return false };
        if self.locked_of(ti) {
            return false;
        }
        let t = &self.tracks[ti];
        let Some(l) = t.clips.iter().find(|c| c.id == left).cloned() else { return false };
        let Some(r) = t.clips.iter().find(|c| (c.start - l.end()).abs() < ABUT_EPS).cloned() else { return false };
        let contiguous = !l.reverse && !r.reverse && (r.src_in - (l.src_in + l.src_len())).abs() < EPS;
        let mergeable =
            l.asset != 0 && l.asset == r.asset && l.kind == r.kind && (l.speed - r.speed).abs() < EPS && contiguous;
        if !mergeable {
            return false;
        }
        let ri = self.tracks[ti].clips.iter().position(|c| c.id == r.id).unwrap();
        self.tracks[ti].clips.remove(ri);
        let li = self.tracks[ti].clips.iter().position(|c| c.id == left).unwrap();
        self.tracks[ti].clips[li].duration += r.duration;
        self.tidy();
        true
    }

    /// Duplicate `ids` (+ their linked clips) onto the first free track of each one's kind (per
    /// `find_free_track`, preferring its own track). Duplicated clips that were linked to each other
    /// stay linked to each other (a fresh link id), not to the originals.
    pub fn duplicate(&mut self, ids: &[Id]) -> Vec<Id> {
        let ids = self.expand_links(ids);
        let mut link_map: std::collections::HashMap<Id, Id> = std::collections::HashMap::new();
        let mut new_ids = Vec::new();
        let sources: Vec<(usize, Clip)> = ids
            .iter()
            .filter_map(|&id| self.find(id).map(|(ti, ci)| (ti, self.tracks[ti].clips[ci].clone())))
            .collect();
        for (ti, mut c) in sources {
            if self.locked_of(ti) {
                continue;
            }
            c.id = self.new_id();
            if c.link != 0 {
                let new_link = match link_map.get(&c.link) {
                    Some(&l) => l,
                    None => {
                        let l = self.new_id();
                        link_map.insert(c.link, l);
                        l
                    }
                };
                c.link = new_link;
            }
            let kind = self.tracks[ti].kind;
            let target = self.find_free_track(kind, c.start, c.duration, Some(ti));
            new_ids.push(c.id);
            self.tracks[target].clips.push(c);
            self.tracks[target].sort();
        }
        self.tidy();
        new_ids
    }

    /// Flatten a `Sequence` clip back onto the timeline at its original positions (the inverse of
    /// `nest_selection`); refuses (returns empty) if the clip is retimed or its source window doesn't
    /// start at zero — compositing a nested retime into the flattened children's own starts is a
    /// documented ceiling, not implemented here.
    pub fn unnest(&mut self, clip: Id) -> Vec<Id> {
        let Some((ti, ci)) = self.find(clip) else { return Vec::new() };
        if self.locked_of(ti) {
            return Vec::new();
        }
        let c = self.tracks[ti].clips[ci].clone();
        if c.kind != ClipKind::Sequence || (c.speed - 1.0).abs() > EPS || c.src_in.abs() > EPS {
            return Vec::new();
        }
        let Some(seq) = self.sequence(c.sequence).cloned() else { return Vec::new() };
        self.tracks[ti].clips.remove(ci);
        let mut new_ids = Vec::new();
        for st in &seq.tracks {
            let kind = st.kind;
            for sc in &st.clips {
                let mut nc = sc.clone();
                nc.id = self.new_id();
                nc.start += c.start;
                let target = self.find_free_track(kind, nc.start, nc.duration, None);
                new_ids.push(nc.id);
                self.tracks[target].clips.push(nc);
                self.tracks[target].sort();
            }
        }
        self.tidy();
        new_ids
    }

    /// Swap `clip`'s asset (and reset `src_in` to 0) — duration, effects, transform and label are
    /// left exactly as they were.
    pub fn replace_clip(&mut self, clip: Id, asset: Id) -> bool {
        let Some(ti) = self.track_of(clip) else { return false };
        if self.locked_of(ti) || self.asset(asset).is_none() {
            return false;
        }
        let Some(c) = self.clip_mut(clip) else { return false };
        c.asset = asset;
        c.src_in = 0.0;
        true
    }

    /// Move `ids` by `dt`/`dtrack` (see `move_clips`); on a magnetic destination track a blocked move
    /// ripple-opens exactly the moved clip's own span there first, instead of refusing.
    pub fn magnetic_move(&mut self, ids: &[Id], dt: f64, dtrack: i32) -> bool {
        let ids = self.expand_links(ids);
        if ids.is_empty() || ids.iter().any(|&id| self.track_of(id).map(|ti| self.locked_of(ti)).unwrap_or(true)) {
            return false;
        }
        // `dtrack != 0` cross-track shoving needs move_clips' track_kind filter armed with the moved
        // clips' own kind — passing None (as an earlier version of this fn did) silently made dtrack a
        // no-op, since move_clips only changes track when `track_kind == Some(kind)`.
        let kind = ids.first().and_then(|&id| self.track_of(id)).map(|ti| self.tracks[ti].kind);
        if self.move_clips(&ids, dt, dtrack, kind) {
            return true;
        }
        let Some(&first) = ids.first() else { return false };
        let Some(ti) = self.track_of(first) else { return false };
        if !self.tracks[ti].magnetic {
            return false;
        }
        let Some(c) = self.clip(first) else { return false };
        let (new_start, span) = ((c.start + dt).max(0.0), c.duration);
        self.ripple_open(new_start, span, &[ti]);
        self.move_clips(&ids, dt, dtrack, kind)
    }
}

/// Shared "how long a placed clip is" rule used by `overwrite_asset`/`splice_in`: an explicit
/// `range`'s length, else the same default `insert_asset_clips`/`insert_asset_clips_ranged` use
/// (5 s for an image, else the asset's own duration).
fn clip_span(a: &Asset, range: Option<(f64, f64)>) -> f64 {
    match range {
        Some((s, e)) => (e - s).max(MIN_CLIP),
        None => {
            if a.kind == ClipKind::Image {
                5.0
            } else {
                a.duration.max(MIN_CLIP)
            }
        }
    }
}
