use crate::model::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Track {
    pub id: Id,
    pub name: String,
    pub kind: TrackKind,
    /// Audio: muted. Video: hidden (the "V" visibility toggle).
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub solo: bool,
    /// UI height in points.
    #[serde(default = "crate::model::dh")]
    pub height: f32,
    #[serde(default)]
    pub clips: Vec<Clip>,
    #[serde(default)]
    pub transitions: Vec<Transition>,
    /// Audio tracks: the bus every clip feeds unless the clip overrides it (0 = Main).
    #[serde(default)]
    pub bus: Id,
}

impl Track {
    pub fn new(id: Id, kind: TrackKind, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            kind,
            muted: false,
            solo: false,
            height: if kind == TrackKind::Video { 64.0 } else { 56.0 },
            clips: Vec::new(),
            transitions: Vec::new(),
            bus: 0,
        }
    }
    pub fn sort(&mut self) {
        self.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    }
    pub fn end(&self) -> f64 {
        self.clips.iter().map(|c| c.end()).fold(0.0, f64::max)
    }
    /// True if [start, start+dur) is free on this track, ignoring clips in `ignore`.
    pub fn fits(&self, start: f64, dur: f64, ignore: &[Id]) -> bool {
        start >= -EPS
            && !self
                .clips
                .iter()
                .any(|c| !ignore.contains(&c.id) && c.start < start + dur - EPS && start < c.end() - EPS)
    }
    /// The clip ending exactly where `right` starts (the left side of that cut).
    pub fn left_of(&self, right: &Clip) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id != right.id && (c.end() - right.start).abs() < ABUT_EPS)
    }
    /// The (left, right) sides of a transition, if it is still valid. Edge transitions have one side
    /// missing: `In` blends nothing → clip (no left), `Out` blends clip → nothing (no right).
    pub fn transition_clips(&self, tr: &Transition) -> Option<(Option<&Clip>, Option<&Clip>)> {
        let c = self.clips.iter().find(|c| c.id == tr.right)?;
        match tr.edge {
            TransitionEdge::Cut => Some((Some(self.left_of(c)?), Some(c))),
            TransitionEdge::In => Some((None, Some(c))),
            TransitionEdge::Out => Some((Some(c), None)),
        }
    }
    /// The transition playing at timeline time t (clamped window), with its clips.
    pub fn transition_at(&self, t: f64) -> Option<(&Transition, Option<&Clip>, Option<&Clip>)> {
        self.transitions.iter().find_map(|tr| {
            let (l, r) = self.transition_clips(tr)?;
            let (cut, h) = tr.cut_half(l, r)?;
            (t >= cut - h && t < cut + h).then_some((tr, l, r))
        })
    }
    /// Drop transitions whose clips no longer abut (edge transitions only need their clip to exist).
    pub fn prune_transitions(&mut self) {
        let keep: Vec<Id> =
            self.transitions.iter().filter(|t| self.transition_clips(t).is_some()).map(|t| t.id).collect();
        self.transitions.retain(|t| keep.contains(&t.id));
    }
}
