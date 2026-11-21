//! ---- ws:visualizer-helper ----
//! What Visualizer Helper's Analyse Song step stores on the project (`Project::viz`), one record per song
//! asset: the musical events in SOURCE seconds of the song file (so trims and moves of the song clip never
//! invalidate them), already simplified to what the other steps key from, plus the clips each step made
//! (so running a step again replaces its own clips and nothing else).

use crate::model::{Id, Project};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SongViz {
    /// The song's asset and audio stream.
    pub asset: Id,
    pub stream: usize,
    /// Seconds of song analysed.
    pub duration: f64,
    pub bpm: f64,
    /// Analysed from Demucs stems (false: the mix alone, less exact on loud genres).
    pub stems: bool,
    /// Every hit `[time, strength 0..1]`: strength 0 hits still count on the hit clock.
    pub hits: Vec<[f32; 2]>,
    /// Bright synth stabs (screeches): times.
    pub screeches: Vec<f32>,
    /// Drop times (the bass back after a silence, louder than anything before).
    pub drops: Vec<f32>,
    /// Where the song calms down after the last drop.
    pub calm: Option<f32>,
    /// The first sung syllable.
    pub vocal_onset: Option<f32>,
    /// Syllable onsets on the vocal stem.
    pub onsets: Vec<f32>,
    /// Stretches where the voice sounds `[start, end]`.
    pub voiced: Vec<[f32; 2]>,
    /// Curves `[time, value]`, simplified: the vocal envelope (held when sung, small when talked), the
    /// vibrato (-1..1) and the synth stem's brightness (0..2.5).
    pub vox: Vec<[f32; 2]>,
    pub vib: Vec<[f32; 2]>,
    pub screech: Vec<[f32; 2]>,
    /// The clips the other steps made from this analysis.
    pub made: VizMade,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VizMade {
    /// The visualizer clip a template was applied to, and the template's name.
    pub visualizer: Id,
    pub template: String,
    pub lyrics: Vec<Id>,
    /// Drop FX clips (flashes, invert layers, the shatter layer).
    pub fx: Vec<Id>,
    pub background: Vec<Id>,
}

impl SongViz {
    /// The biggest drop: the last one detected.
    pub fn big_drop(&self) -> Option<f64> {
        self.drops.last().map(|&d| d as f64)
    }
}

impl Project {
    /// The analysis of a song asset + stream.
    pub fn viz_of(&self, asset: Id, stream: usize) -> Option<&SongViz> {
        self.viz.iter().find(|v| v.asset == asset && v.stream == stream)
    }

    pub(crate) fn viz_of_mut(&mut self, asset: Id, stream: usize) -> Option<&mut SongViz> {
        self.viz.iter_mut().find(|v| v.asset == asset && v.stream == stream)
    }

    /// Store `v`, replacing an older analysis of the same song (the clips it made stay remembered).
    pub(crate) fn set_viz(&mut self, mut v: SongViz) {
        match self.viz.iter_mut().find(|o| o.asset == v.asset && o.stream == v.stream) {
            Some(o) => {
                v.made = std::mem::take(&mut o.made);
                *o = v;
            }
            None => self.viz.push(v),
        }
    }
}
