//! ---- ws:zones-markers ----
//! Effect zones: an effect stack laid over a time range of one track, or of the whole timeline, instead
//! of on one clip ("grade everything from 0:30 to 0:45"). A zone is a bar in the timeline's zone lane;
//! the renderer never sees it: `engine::zones::render_view` turns each one into an ordinary adjustment
//! layer (whole timeline) or into effects on the clips under it (one track), so preview, export and
//! prerender draw it through the same effect path clips use.

use crate::model::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EffectZone {
    pub id: Id,
    /// What its bar says; empty = its first effect's name.
    pub name: String,
    /// Timeline seconds it covers: [t, t + dur].
    pub t: f64,
    pub dur: f64,
    /// None = every video track (the canvas below it); Some(track id) = that track's clips only.
    pub track: Option<Id>,
    /// The stack, run in order. Effect `start` / `len` and keyframe times are zone-local seconds
    /// (0 = the zone's start), like a clip's are clip-local.
    pub effects: Vec<Effect>,
    pub enabled: bool,
    /// The sequence it was made on (None = the main timeline), like `Marker.sequence`.
    pub sequence: Option<Id>,
}

impl Default for EffectZone {
    fn default() -> Self {
        Self { id: 0, name: String::new(), t: 0.0, dur: 1.0, track: None, effects: Vec::new(), enabled: true, sequence: None }
    }
}

impl EffectZone {
    pub fn end(&self) -> f64 {
        self.t + self.dur
    }
    /// What its bar and the Inspector call it.
    pub fn label(&self) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        match self.effects.first() {
            Some(e) if self.effects.len() > 1 => format!("{} +{}", e.name(), self.effects.len() - 1),
            Some(e) => e.name(),
            None => "Effect zone".to_string(),
        }
    }
}
