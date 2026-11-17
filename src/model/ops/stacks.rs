//! ---- ws:animator ----
//! Stacks: a footage clip and the overlays on it grouped into one nested sequence that sits on the footage's
//! track, so a transition to the next clip (transitions live per track) or a block on the stack moves all of
//! them as one. Unstack is `unnest`; duplicate / delete / rename are the ordinary clip ones.

use crate::model::*;

impl Project {
    /// Make `ids` (empty = every clip) ready for transitions: on each track, each of them that follows another
    /// across an empty gap moves back to start where that one ends, so every pair shares a cut a transition
    /// can sit on. Clips with something else between them stay. Returns how many gaps it closed.
    pub fn make_transitionable(&mut self, ids: &[Id]) -> usize {
        let mut n = 0;
        for ti in 0..self.tracks.len() {
            let mut on: Vec<Id> =
                self.tracks[ti].clips.iter().filter(|c| ids.is_empty() || ids.contains(&c.id)).map(|c| c.id).collect();
            on.sort_by(|a, b| self.clip(*a).unwrap().start.total_cmp(&self.clip(*b).unwrap().start));
            for w in 0..on.len().saturating_sub(1) {
                let (Some(a), Some(b)) = (self.clip(on[w]), self.clip(on[w + 1])) else { continue };
                let (end, start) = (a.end(), b.start);
                let empty = !self.tracks[ti].clips.iter().any(|c| c.start < start - 1e-6 && c.end() > end + 1e-6);
                if start - end > 1e-6 && empty {
                    let moved = self.expand_links(&[on[w + 1]]);
                    n += self.move_clips(&moved, end - start, 0, None) as usize;
                }
            }
        }
        n
    }

    /// The video clip on the lowest video track under timeline time `t`, then every clip on the video tracks
    /// above it that is mostly (over half its length) over that clip. Empty when no video clip is under `t`.
    pub fn clip_with_overlays(&self, t: f64) -> Vec<Id> {
        let vt = self.video_tracks();
        let Some((bt, base)) =
            vt.iter().find_map(|&ti| self.tracks[ti].clips.iter().find(|c| c.contains(t)).map(|c| (ti, c)))
        else {
            return Vec::new();
        };
        let mut out = vec![base.id];
        for &ti in vt.iter().filter(|&&ti| ti > bt) {
            for c in &self.tracks[ti].clips {
                let shared = c.end().min(base.end()) - c.start.max(base.start);
                if shared > c.duration * 0.5 {
                    out.push(c.id);
                }
            }
        }
        out
    }

    /// Group `ids` into a stack: the clip on the lowest video track is the footage, the rest its overlays.
    /// Stills, text and shapes that end within 1.2 s of the footage's end are lined up with it first (footage
    /// overlays keep their length: there may be no more media). The stack lands on the footage's own track
    /// when that is free. Returns the stack clip.
    pub fn make_stack(&mut self, ids: &[Id]) -> Option<Id> {
        let (bt, base) = ids
            .iter()
            .filter_map(|&id| Some((self.track_of(id)?, id)))
            .filter(|(ti, _)| self.tracks[*ti].kind == TrackKind::Video)
            .min()?;
        let track_id = self.tracks[bt].id;
        let (s, e) = self.clip(base).map(|c| (c.start, c.end()))?;
        for &id in ids.iter().filter(|&&id| id != base) {
            if let Some(c) = self.clip_mut(id) {
                let stretchy = !matches!(c.kind, ClipKind::Video | ClipKind::Audio | ClipKind::Sequence);
                let gap = e - c.end();
                if gap < -1e-9 || (stretchy && gap > 1e-9 && gap < 1.2) {
                    c.duration = (e - c.start).max(MIN_CLIP);
                }
            }
        }
        let n = self.sequences.len() + 1;
        let seq = self.nest_selection(ids, format!("Stack {n}"))?;
        let sid =
            self.all_clips().find(|(_, c)| c.sequence == seq && c.kind == ClipKind::Sequence).map(|(_, c)| c.id)?;
        // onto the footage's track (where the next clip's transition lives) when nothing else is there now
        let ti = self.tracks.iter().position(|t| t.id == track_id)?;
        let (cs, ce) = self.clip(sid).map(|c| (c.start, c.end()))?;
        let free = self.tracks[ti].clips.iter().all(|c| c.end() <= cs + 1e-9 || c.start >= ce - 1e-9);
        if free && self.track_of(sid) != Some(ti) {
            let (from, ci) = self.find(sid)?;
            let c = self.tracks[from].clips.remove(ci);
            self.tracks[ti].clips.push(c);
            self.tracks[ti].sort();
            self.tidy();
        }
        let _ = s;
        Some(sid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clip_and_its_overlays_stack_onto_the_footage_track() {
        let mut p = Project::new();
        let v1 = p.video_tracks()[0];
        p.tracks[v1].clips.push(Clip::new(1, ClipKind::Video, "A", 0.0, 6.0));
        p.tracks[v1].clips.push(Clip::new(2, ClipKind::Video, "B", 6.0, 6.0));
        let v2 = p.add_track(TrackKind::Video);
        p.tracks[v2].clips.push(Clip::new(3, ClipKind::Text, "Title", 0.5, 5.0));
        let v3 = p.add_track(TrackKind::Video);
        p.tracks[v3].clips.push(Clip::new(4, ClipKind::Image, "Sticker", 7.0, 2.0));
        let ids = p.clip_with_overlays(1.0);
        assert_eq!(ids, vec![1, 3], "the sticker is over B, not A");
        let s = p.make_stack(&ids).unwrap();
        let c = p.clip(s).unwrap();
        assert_eq!(c.kind, ClipKind::Sequence);
        assert!((c.start, c.duration) == (0.0, 6.0), "{} {}", c.start, c.duration);
        let b = p.clip(2).unwrap();
        assert_eq!(p.track_of(s), p.track_of(2), "same track as the next clip, so a transition between them works");
        assert!((b.start - 6.0).abs() < 1e-9);
        // the title was stretched to end with the footage inside the stack
        let seq = p.sequence(c.sequence).unwrap();
        let title = seq.tracks.iter().flat_map(|t| &t.clips).find(|c| c.id == 3 || c.name == "Title").unwrap();
        assert!((title.end() - 6.0).abs() < 1e-9);
        // unstack puts them back
        let back = p.unnest(s);
        assert_eq!(back.len(), 2);
    }
}
