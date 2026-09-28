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
    // ---- ws:registries-schema-hooks ----
    /// Edits on this track are refused (trim-model, wave 1).
    #[serde(default)]
    pub locked: bool,
    /// Deleting/trimming shoves downstream clips on this track to close the gap. `None` only right
    /// after a bare `Track::new` - every real construction site resolves it via `default_ripple`
    /// before the track is used, so a project session never sees `None`; `from_json` resolves it too
    /// for tracks loaded from disk.
    #[serde(default)]
    pub ripple: Option<bool>,
    /// Gapless (magnetic) track: a plain edge-drag ripples and Delete closes the gap (trim-model/
    /// timeline-trim-gestures, wave 1/2).
    #[serde(default)]
    pub magnetic: bool,
    /// Header swatch colour; `None` = the theme default. Sole definition (pro-timeline, wave 3,
    /// consumes this field rather than redeclaring it).
    #[serde(default)]
    pub color: Option<[u8; 3]>,
    /// Track-level gain multiplier (1 = unity); sampled by the mixer once audio-dsp-automation
    /// (wave 1) wires it in - see `engine::mixer::mix_tracks`. Defaults to unity because `Animated`
    /// has no `Default` impl in this crate (a bare `#[serde(default)]` would not compile).
    #[serde(default = "crate::model::a1")]
    pub volume: Animated,
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
            locked: false,
            ripple: None,
            magnetic: false,
            color: None,
            volume: crate::model::a1(),
        }
    }
    /// The one resolver every Track-construction site calls so `ripple` is never left `None` outside
    /// a bare `Track::new` for more than the current statement: the first track of a kind defaults to
    /// ripple-on (V1/A1 stay in sync with edits by default), every later track of that kind defaults
    /// to position-locked (secondary tracks - B-roll, music, SFX - never silently desync).
    pub(crate) fn default_ripple(_kind: TrackKind, index_within_kind: usize) -> Option<bool> {
        Some(index_within_kind == 0)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_volume_defaults_to_unity() {
        // no `volume` field at all - an older project, or one hand-edited
        let t: Track = serde_json::from_str(r#"{"id":1,"name":"V1","kind":"Video"}"#).unwrap();
        assert_eq!(t.volume.value, 1.0, "missing volume must default to unity, not silence");
        assert!(!t.volume.is_animated());
        assert_eq!(t.ripple, None, "bare Track::new / a fresh deserialize: not yet resolved");
        assert!(!t.locked && !t.magnetic && t.color.is_none());
    }

    /// Every real Track-construction path resolves `ripple` immediately - not just on a save/reload
    /// round-trip through `Project::from_json` (see `io::tests::ripple_resolves_on_load` for that path).
    #[test]
    fn ripple_resolves_at_every_construction_site() {
        // Project::new -> add_track: V1/A1 are each the first-of-kind
        let mut p = Project::new();
        assert_eq!(p.tracks[0].ripple, Some(true), "V1");
        assert_eq!(p.tracks[1].ripple, Some(true), "A1");
        // video tracks stay before audio tracks, so the new V2 lands at index 1, not at the end
        p.add_track(TrackKind::Video);
        assert_eq!(p.video_tracks().len(), 2);
        let v2 = p.video_tracks()[1];
        assert_eq!(p.tracks[v2].ripple, Some(false), "V2 (second video track)");

        // new_sequence: V1/A1 are each the first (only) track of their kind in a fresh sequence
        let seq_id = p.new_sequence("Seq", 1920, 1080, 30.0);
        let seq = p.sequence(seq_id).unwrap();
        assert!(seq.tracks.iter().all(|t| t.ripple.is_some()), "sequence V1/A1");

        // the auto-created "Subtitles" video track (cues_to_text_clips)
        let cue_id = p.new_id();
        p.subtitles.push(Cue { id: cue_id, start: 0.0, end: 1.0, text: "hi".into() });
        p.cues_to_text_clips(None);
        let sub = p.tracks.iter().find(|t| t.name == "Subtitles").expect("Subtitles track created");
        assert!(sub.ripple.is_some(), "Subtitles auto-track");

        // the XML/EDL import "ensure track" helper (engine::import::track_slot) calls
        // Project::add_track for any track it needs, so it inherits the resolution above for free -
        // pinned by `import::tests` exercising a real import, not duplicated here.
    }
}
