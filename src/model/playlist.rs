//! ---- ws:music-burn ----
//! Playlists: ordered library songs (or videos), saved in the project. Ops: `model::ops::playlists`.

use crate::model::Id;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Playlist {
    pub id: Id,
    pub name: String,
    /// Library asset ids, in play order (one asset may appear more than once).
    pub items: Vec<Id>,
    /// Seconds of silence between items when it becomes a timeline / an audio CD.
    pub gap: f64,
    /// Seconds two neighbours overlap (fading out / in) on the timeline; 0 = none. Wins over `gap`.
    pub crossfade: f64,
}
