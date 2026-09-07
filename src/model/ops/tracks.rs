use crate::model::*;

/// Selects which of a track's three trim-model flags `Project::set_track_flag` touches - avoids
/// three near-identical setters (and is the `track.set` MCP tool's field selector).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackFlag {
    Locked,
    Ripple,
    Magnetic,
}

impl Project {
    // ---------- tracks ----------
    // ---- ws:trim-model ----
    /// True when edits on track `ti` are refused. Query used as a guard at the top of every new
    /// mutating op (ripple_trim/roll_edit/slip/... in trim.rs). Takes a track INDEX, matching every
    /// other track-addressing fn in this file (`find_free_track`, `video_tracks`, `move_clips`'s
    /// `dtrack`, `TimelineState.last_track`) rather than a stable `Track.id` - the plan's own text
    /// typed this as `id: Id`, but nothing else in the codebase looks a track up by its `id` field
    /// (it exists for JSON stability, not addressing), so an index keeps every call site symmetric
    /// with `ripple_tracks()`'s `Vec<usize>` and `self.find(clip_id)`'s `(ti, ci)`. Out-of-range = false.
    pub fn locked_of(&self, ti: usize) -> bool {
        self.tracks.get(ti).map(|t| t.locked).unwrap_or(false)
    }
    /// Indices of tracks with `ripple == true` (an unresolved `None` - only possible on a bare,
    /// not-yet-constructed `Track` - counts as `false`, the safe default). The scope for every ripple
    /// shift, including the extended legacy `close_gap`/`ripple_delete_range`/`ripple_open`.
    pub fn ripple_tracks(&self) -> Vec<usize> {
        (0..self.tracks.len()).filter(|&i| self.tracks[i].ripple.unwrap_or(false)).collect()
    }
    /// Header toggle + `track.set` MCP tool backend.
    pub fn set_track_flag(&mut self, ti: usize, flag: TrackFlag, on: bool) -> bool {
        let Some(t) = self.tracks.get_mut(ti) else { return false };
        match flag {
            TrackFlag::Locked => t.locked = on,
            TrackFlag::Ripple => t.ripple = Some(on),
            TrackFlag::Magnetic => t.magnetic = on,
        }
        true
    }
    /// Rename track `ti` (header double-click / `track.set` MCP tool). Refuses a blank name.
    pub fn rename_track(&mut self, ti: usize, name: String) -> bool {
        if name.trim().is_empty() {
            return false;
        }
        let Some(t) = self.tracks.get_mut(ti) else { return false };
        t.name = name;
        true
    }
    /// Set (or clear) track `ti`'s header swatch colour - consumes `Track.color: Option<[u8;3]>`
    /// (registries-schema-hooks, wave 0b); sole mutator, canonical over pro-timeline's independently
    /// proposed `Track.color: u8` (see the PR body / plan risk notes).
    pub fn set_track_color(&mut self, ti: usize, color: Option<[u8; 3]>) -> bool {
        let Some(t) = self.tracks.get_mut(ti) else { return false };
        t.color = color;
        true
    }
    /// Reorder track `ti` one slot up/down within its own kind (video tracks and audio tracks each
    /// stay contiguous - `add_track`'s invariant). False, unchanged, at a kind boundary.
    // Safe against the playback cache: video_dirty_spans (playback.rs) treats any Track.id mismatch at
    // an index as unbounded-dirty (player-rate-loop's track_id_reorder_full_clears), so a reorder here
    // always forces a full cache clear rather than producing stale spans.
    pub fn move_track(&mut self, ti: usize, up: bool) -> bool {
        let Some(t) = self.tracks.get(ti) else { return false };
        let list = if t.kind == TrackKind::Video { self.video_tracks() } else { self.audio_tracks() };
        let pos = list.iter().position(|&x| x == ti).unwrap();
        let npos = if up { pos.checked_sub(1) } else { (pos + 1 < list.len()).then_some(pos + 1) };
        let Some(npos) = npos else { return false };
        self.tracks.swap(ti, list[npos]);
        true
    }
    /// Adds a track of `kind` (video tracks stay before audio tracks) and returns its index.
    pub fn add_track(&mut self, kind: TrackKind) -> usize {
        let n = self.tracks.iter().filter(|t| t.kind == kind).count() + 1;
        let id = self.new_id();
        let name = format!("{}{}", if kind == TrackKind::Video { "V" } else { "A" }, n);
        let mut track = Track::new(id, kind, name);
        track.ripple = Track::default_ripple(kind, n - 1); // n is 1-based; index_within_kind is 0-based
        let idx = match kind {
            TrackKind::Video => self.video_tracks().last().map(|i| i + 1).unwrap_or(0),
            TrackKind::Audio => self.tracks.len(),
        };
        self.tracks.insert(idx, track);
        idx
    }
    pub fn remove_track(&mut self, idx: usize) {
        if idx < self.tracks.len() {
            self.tracks.remove(idx);
            self.rename_tracks();
        }
    }
    fn rename_tracks(&mut self) {
        let (mut v, mut a) = (0, 0);
        for t in &mut self.tracks {
            match t.kind {
                TrackKind::Video => {
                    v += 1;
                    t.name = format!("V{v}");
                }
                TrackKind::Audio => {
                    a += 1;
                    t.name = format!("A{a}");
                }
            }
        }
    }
    /// First track of `kind` (preferring `prefer`) where [start,start+dur) is free; adds one if needed.
    pub fn find_free_track(&mut self, kind: TrackKind, start: f64, dur: f64, prefer: Option<usize>) -> usize {
        if let Some(p) = prefer {
            if p < self.tracks.len() && self.tracks[p].kind == kind && self.tracks[p].fits(start, dur, &[]) {
                return p;
            }
        }
        let list = if kind == TrackKind::Video { self.video_tracks() } else { self.audio_tracks() };
        for i in list {
            if self.tracks[i].fits(start, dur, &[]) {
                return i;
            }
        }
        self.add_track(kind)
    }
}
