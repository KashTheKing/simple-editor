//! ---- ws:animator ----
//! Transition spans: a bar over the timeline (the Animator's Timeline lane) that sets up the transitions
//! under it. Wherever it lies, every cut between two clips on a video track gets the span's transition; move
//! it, stretch it or change the clips under it and the transitions follow (`refresh_spans`, run after every
//! edit). The transitions it makes are ordinary ones (the renderer never sees a span).
//!
//! First version: it does NOT nest a clip's overlays into stacks. Each video track gets its own transitions,
//! so overlays change together with the footage only where their cuts line up with its cuts (Stack does the
//! grouping by hand). Gaps are closed on the footage track only (the lowest video track with clips under the
//! span): a title and a logo ten seconds apart on an overlay track are not pulled together. An overlay does
//! not follow a footage clip that closes up either.
//! ponytail: spans belong to the main timeline (`Project.spans`); while a nested sequence is open they rest.
//! Give `Sequence` / `Stash` their own list if spans are wanted inside sequences.

use crate::model::*;

/// The shortest a span gets.
pub const MIN_SPAN: f64 = 0.1;

impl Project {
    /// Add a transition span over timeline [t, t + dur] making `kind` on every cut under it (half a second
    /// each, linear; change `Project.spans` and `refresh_spans` for the rest). Returns its id.
    pub fn add_span(&mut self, t: f64, dur: f64, kind: TransitionKind) -> Id {
        let id = self.new_id();
        self.spans.push(TransitionSpan {
            id,
            t: t.max(0.0),
            dur: dur.max(MIN_SPAN),
            kind,
            cut_dur: 0.5,
            ease: Ease::Linear,
            direction: 0,
            shader: String::new(),
            params: Vec::new(),
            made: Vec::new(),
        });
        self.refresh_spans();
        id
    }

    /// Delete span `id` and the transitions it made.
    pub fn remove_span(&mut self, id: Id) -> bool {
        let Some(i) = self.spans.iter().position(|s| s.id == id) else { return false };
        for tid in self.spans.remove(i).made {
            self.remove_transition(tid);
        }
        true
    }

    /// Make every span's transitions match what is under it now. For each span: clips under it on the footage
    /// track (the lowest video track with clips under it) that are separated by an empty gap are closed up
    /// first (`make_transitionable`, for those clips only), then every cut between two abutting clips on a
    /// video track whose time lies in [t, t + dur]
    /// carries the span's transition, and transitions it made on cuts it no longer covers are removed.
    /// Nothing changes (no new ids) while the cuts under a span stay the same. Where two spans cover one
    /// cut, the later one in the list wins.
    pub fn refresh_spans(&mut self) {
        if self.editing.is_some() {
            return;
        }
        for si in 0..self.spans.len() {
            let sp = self.spans[si].clone();
            let (a, b) = (sp.t, sp.t + sp.dur);
            let vt = self.video_tracks();
            let on = |c: &&Clip| c.start < b - 1e-6 && c.end() > a + 1e-6;
            let under: Vec<Id> = vt
                .iter()
                .map(|&ti| self.tracks[ti].clips.iter().filter(on).map(|c| c.id).collect::<Vec<Id>>())
                .find(|v| !v.is_empty())
                .unwrap_or_default();
            if under.len() > 1 {
                self.make_transitionable(&under);
            }
            let mut want: Vec<Id> = Vec::new();
            for &ti in &vt {
                let tr = &self.tracks[ti];
                want.extend(
                    tr.clips.iter().filter(|c| c.start >= a - 1e-6 && c.start <= b + 1e-6 && tr.left_of(c).is_some()).map(|c| c.id),
                );
            }
            want.sort();
            // the transitions it made that are still there, and the cuts (right clips) its video ones sit on
            let cut = |t: &Track, x: &Transition| t.kind == TrackKind::Video && x.edge == TransitionEdge::Cut;
            let alive: Vec<(Id, Option<Id>)> = self
                .tracks
                .iter()
                .flat_map(|t| t.transitions.iter().map(move |x| (x.id, cut(t, x).then_some(x.right))))
                .filter(|x| sp.made.contains(&x.0))
                .collect();
            let mut have: Vec<Id> = alive.iter().filter_map(|x| x.1).collect();
            have.sort();
            let made: Vec<Id> = if have == want {
                alive.iter().map(|x| x.0).collect()
            } else {
                for &tid in &sp.made {
                    self.remove_transition(tid);
                }
                let ids = |p: &Project| -> Vec<Id> { p.tracks.iter().flat_map(|t| t.transitions.iter().map(|x| x.id)).collect() };
                let mut made = Vec::new();
                for &right in &want {
                    // (a linked sound clip gets its crossfade too: everything new is this span's)
                    let before = ids(self);
                    if self.add_transition(right, sp.kind, sp.cut_dur).is_some() {
                        made.extend(ids(self).into_iter().filter(|i| !before.contains(i)));
                    }
                }
                made
            };
            // the span's transition on each of them (sound keeps its crossfade, at the same length)
            for t in &mut self.tracks {
                let video = t.kind == TrackKind::Video;
                for x in t.transitions.iter_mut().filter(|x| made.contains(&x.id)) {
                    x.duration = sp.cut_dur.max(MIN_CLIP);
                    if video {
                        (x.kind, x.ease, x.direction) = (sp.kind, sp.ease, sp.direction);
                        if x.shader != sp.shader || x.params != sp.params {
                            (x.shader, x.params) = (sp.shader.clone(), sp.params.clone());
                        }
                    }
                }
            }
            self.spans[si].made = made;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three clips on V1 with a second of nothing between them.
    fn three() -> Project {
        let mut p = Project::new();
        let v1 = p.video_tracks()[0];
        for (id, start) in [(1, 0.0), (2, 3.0), (3, 6.0)] {
            p.tracks[v1].clips.push(Clip::new(id, ClipKind::Video, "clip", start, 2.0));
        }
        p
    }
    fn transitions(p: &Project) -> Vec<&Transition> {
        p.tracks.iter().flat_map(|t| &t.transitions).collect()
    }

    #[test]
    fn a_span_sets_up_the_transitions_under_it_and_takes_them_with_it() {
        let mut p = three();
        let s = p.add_span(0.0, 8.0, TransitionKind::CrossFade);
        // the gaps are closed and both cuts carry the transition
        let starts: Vec<f64> = [1, 2, 3].iter().map(|&id| p.clip(id).unwrap().start).collect();
        assert_eq!(starts, [0.0, 2.0, 4.0]);
        let mut rights: Vec<Id> = transitions(&p).iter().map(|t| t.right).collect();
        rights.sort();
        assert_eq!(rights, [2, 3]);
        assert_eq!(p.spans[0].made.len(), 2);
        // nothing moves and no id changes while the cuts stay the same; its controls reach the transitions
        let ids = p.spans[0].made.clone();
        (p.spans[0].kind, p.spans[0].direction, p.spans[0].cut_dur, p.spans[0].ease) = (TransitionKind::Push, 2, 0.3, Ease::EaseOut);
        p.refresh_spans();
        assert_eq!(p.spans[0].made, ids);
        assert!(transitions(&p).iter().all(|t| (t.kind, t.direction, t.duration, t.ease) == (TransitionKind::Push, 2, 0.3, Ease::EaseOut)));
        // over the first cut only: the second cut's transition goes
        (p.spans[0].t, p.spans[0].dur) = (1.5, 1.0);
        p.refresh_spans();
        assert_eq!(transitions(&p).iter().map(|t| t.right).collect::<Vec<_>>(), [2]);
        // moved off the clips: none left
        p.spans[0].t = 20.0;
        p.refresh_spans();
        assert!(transitions(&p).is_empty() && p.spans[0].made.is_empty());
        // back over them, then deleted: its transitions go with it (the clips stay closed up)
        (p.spans[0].t, p.spans[0].dur) = (0.0, 8.0);
        p.refresh_spans();
        assert_eq!(transitions(&p).len(), 2);
        assert!(p.remove_span(s) && !p.remove_span(s));
        assert!(transitions(&p).is_empty() && p.spans.is_empty());
        assert_eq!(p.clip(3).unwrap().start, 4.0);
    }

    #[test]
    fn a_span_leaves_other_transitions_and_far_clips_alone_and_old_projects_load() {
        let mut p = three();
        // a transition made by hand outside the span stays, and so does the gap before a clip it doesn't reach
        let v1 = p.video_tracks()[0];
        p.tracks[v1].clips.push(Clip::new(4, ClipKind::Video, "clip", 8.0, 2.0));
        let hand = p.add_transition(4, TransitionKind::Wipe, 0.4).unwrap();
        p.add_span(0.0, 2.5, TransitionKind::CrossFade);
        assert_eq!(p.clip(2).unwrap().start, 3.0, "only one clip is under the span: nothing to close");
        assert_eq!(transitions(&p).iter().map(|t| t.id).collect::<Vec<_>>(), [hand]);
        p.spans[0].dur = 4.0; // now it reaches the second clip
        p.refresh_spans();
        assert_eq!(p.clip(2).unwrap().start, 2.0);
        assert_eq!(p.clip(3).unwrap().start, 6.0, "the third clip is not under it");
        assert!(transitions(&p).iter().any(|t| t.id == hand) && transitions(&p).len() == 2);
        // overlays keep their places (a title and a logo far apart are not pulled together) and get the
        // transition only where two of them already share a cut
        let v2 = p.add_track(TrackKind::Video);
        for (id, start) in [(5, 0.0), (6, 3.0), (7, 3.5)] {
            p.tracks[v2].clips.push(Clip::new(id, ClipKind::Text, "overlay", start, 0.5));
        }
        p.refresh_spans();
        assert_eq!(p.clip(6).unwrap().start, 3.0);
        assert!(transitions(&p).iter().any(|t| t.right == 7) && !transitions(&p).iter().any(|t| t.right == 6));
        // save / load keeps the span and what it made; a project from before spans loads with none
        let back = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(back.spans, p.spans);
        let mut old: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
        old.as_object_mut().unwrap().remove("spans");
        assert!(Project::from_json(&old.to_string()).unwrap().spans.is_empty());
    }
}
