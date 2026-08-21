//! Auto-cut: find "loud" segments (speech) vs "quiet" ones (ambient) in an audio clip from its waveform
//! peaks (media::waveform::Peaks, 100 buckets/s) — no extra decoding. Pure functions, unit-tested.

use crate::media::waveform::Peaks;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoCutParams {
    /// Level threshold in dBFS (e.g. -35): buckets above are "loud".
    pub threshold_db: f32,
    /// Quiet stretches shorter than this (seconds) are absorbed into the surrounding loud segment.
    pub min_silence: f64,
    /// Loud stretches shorter than this (seconds) are dropped (clicks/noise).
    pub min_speech: f64,
    /// Extra time kept before/after each loud segment (seconds).
    pub padding: f64,
}

impl Default for AutoCutParams {
    fn default() -> Self {
        Self { threshold_db: -35.0, min_silence: 0.4, min_speech: 0.2, padding: 0.1 }
    }
}

/// Loud segments as (start, end) in SOURCE seconds within [src_in, src_in + src_len) of the analysed
/// stream, merged/filtered per `params` and clamped to that window. Peak level per bucket =
/// max(|min|, |max|) → dBFS. Empty peaks → empty result.
pub fn loud_segments(_peaks: &Peaks, _src_in: f64, _src_len: f64, _params: &AutoCutParams) -> Vec<(f64, f64)> {
    todo!("engine::autocut::loud_segments")
}

/// Turn source-time loud segments into timeline cut times + ranges to remove for a clip that starts at
/// `start` with `speed` (forward only; reversed/frozen clips are not auto-cut):
/// returns (cuts, quiet_ranges) in timeline seconds, quiet ranges = the complement of the loud segments
/// inside the clip. `keep_quiet` swaps the roles (keep ambient, remove speech).
pub fn to_timeline(
    _segments: &[(f64, f64)],
    _start: f64,
    _src_in: f64,
    _duration: f64,
    _speed: f64,
    _keep_quiet: bool,
) -> (Vec<f64>, Vec<(f64, f64)>) {
    todo!("engine::autocut::to_timeline")
}
