use crate::model::*;

impl Project {
    // ---------- transitions ----------
    /// Add (or replace) a transition at the cut on the left of clip `right`. Linked audio clips whose
    /// left neighbour is linked with the video's left neighbour get an audio CrossFade of the same length.
    /// Returns the new transition id, or None if `right` has no abutting left neighbour.
    pub fn add_transition(&mut self, right: Id, kind: TransitionKind, duration: f64) -> Option<Id> {
        self.add_transition_at(right, kind, duration, TransitionEdge::Cut)
    }
    /// Add (or replace) an edge transition at the start (`out` false) or end (`out` true) of `clip`:
    /// the clip blends from/to nothing, no neighbour needed. Mirrors on linked clips like `add_transition`.
    pub fn add_edge_transition(&mut self, clip: Id, kind: TransitionKind, duration: f64, out: bool) -> Option<Id> {
        self.add_transition_at(clip, kind, duration, if out { TransitionEdge::Out } else { TransitionEdge::In })
    }
    fn add_transition_at(
        &mut self,
        right: Id,
        kind: TransitionKind,
        duration: f64,
        edge: TransitionEdge,
    ) -> Option<Id> {
        let (ti, _) = self.find(right)?;
        let r = self.clip(right)?.clone();
        if edge == TransitionEdge::Cut {
            self.tracks[ti].left_of(&r)?;
        }
        let duration = duration.max(MIN_CLIP);
        let id = self.new_id();
        Self::clear_transition_slot(&mut self.tracks[ti], &r, edge);
        self.tracks[ti].transitions.push(Transition {
            id,
            right,
            kind,
            duration,
            color: [0, 0, 0, 255],
            direction: 0,
            ease: Ease::Linear,
            edge,
        });
        // mirror on linked clips (audio crossfade / fade)
        if r.link != 0 {
            let partners: Vec<Id> = self.linked(right).into_iter().filter(|&p| p != right).collect();
            for p in partners {
                let Some((pti, _)) = self.find(p) else { continue };
                if pti == ti {
                    continue;
                }
                let pc = self.clip(p).unwrap().clone();
                if edge == TransitionEdge::Cut && self.tracks[pti].left_of(&pc).is_none() {
                    continue;
                }
                let pid = self.new_id();
                let pkind = if self.tracks[pti].kind == TrackKind::Audio { TransitionKind::CrossFade } else { kind };
                Self::clear_transition_slot(&mut self.tracks[pti], &pc, edge);
                self.tracks[pti].transitions.push(Transition {
                    id: pid,
                    right: p,
                    kind: pkind,
                    duration,
                    color: [0, 0, 0, 255],
                    direction: 0,
                    ease: Ease::Linear,
                    edge,
                });
            }
        }
        Some(id)
    }
    /// Remove whatever transition already occupies the spot a new one is going to: a `Cut` or `In`
    /// at the start of `c` share the start slot (plus the previous clip's `Out`); an `Out` at the end
    /// of `c` shares the end slot with the next clip's `Cut` / `In`.
    fn clear_transition_slot(track: &mut Track, c: &Clip, edge: TransitionEdge) {
        let prev = track.left_of(c).map(|l| l.id);
        let next = track.clips.iter().find(|o| o.id != c.id && (o.start - c.end()).abs() < ABUT_EPS).map(|o| o.id);
        let (start_clip, end_clip) = match edge {
            TransitionEdge::Cut | TransitionEdge::In => (Some(c.id), prev),
            TransitionEdge::Out => (next, Some(c.id)),
        };
        track.transitions.retain(|t| {
            !(start_clip.is_some_and(|s| t.right == s && t.edge != TransitionEdge::Out)
                || end_clip.is_some_and(|e| t.right == e && t.edge == TransitionEdge::Out))
        });
    }
    pub fn remove_transition(&mut self, id: Id) {
        for t in &mut self.tracks {
            t.transitions.retain(|x| x.id != id);
        }
    }
    pub fn transition_mut(&mut self, id: Id) -> Option<&mut Transition> {
        self.tracks.iter_mut().flat_map(|t| t.transitions.iter_mut()).find(|x| x.id == id)
    }
    /// Transitions touching a clip (as left or right side): (track index, transition).
    pub fn transitions_of(&self, clip: Id) -> Vec<(usize, &Transition)> {
        let mut out = Vec::new();
        for (ti, t) in self.tracks.iter().enumerate() {
            for tr in &t.transitions {
                if let Some((l, r)) = t.transition_clips(tr) {
                    if l.is_some_and(|c| c.id == clip) || r.is_some_and(|c| c.id == clip) {
                        out.push((ti, tr));
                    }
                }
            }
        }
        out
    }
}
