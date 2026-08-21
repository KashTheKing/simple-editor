//! Audio mixer: sums every audible audio clip at timeline time t into interleaved stereo f32.
//! Used by playback (real-time, block by block) and export (offline to WAV).

use crate::media::DecoderPool;
use crate::model::Project;

pub struct Mixer {
    #[allow(dead_code)]
    scratch: Vec<f32>,
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

impl Mixer {
    pub fn new() -> Self {
        Self { scratch: Vec::new() }
    }

    /// Mix into `out` (interleaved stereo, frames = out.len()/2) starting at timeline time `t`.
    /// `out` is zeroed first. For each audio track with `project.active(track)`, each enabled clip
    /// overlapping [t, t + frames/48000): read `pool.audio(asset.path, clip.audio_stream)` at
    /// `clip.src_time(..)` for the overlapping sub-range, apply `clip.volume` (ramped linearly from the
    /// value at the block start to the block end), add. Clamp the result to [-1, 1].
    pub fn mix(&mut self, _project: &Project, _t: f64, _pool: &mut DecoderPool, _out: &mut [f32]) {
        todo!("engine::mixer::Mixer::mix")
    }
}
