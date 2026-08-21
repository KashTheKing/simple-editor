//! Auto-cut: find "loud" segments (speech) vs "quiet" ones (ambient) in an audio clip from its waveform
//! peaks (media::waveform::Peaks, 100 buckets/s) — no extra decoding. Pure functions, unit-tested.

use crate::media::waveform::{Peaks, PEAKS_PER_SEC};

const EPS: f64 = 1e-6;

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
pub fn loud_segments(peaks: &Peaks, src_in: f64, src_len: f64, params: &AutoCutParams) -> Vec<(f64, f64)> {
    let per = PEAKS_PER_SEC as f64;
    let b0 = ((src_in * per).floor().max(0.0)) as usize;
    let b1 = (((src_in + src_len) * per).ceil().max(0.0) as usize).min(peaks.len());
    if peaks.is_empty() || src_len <= 0.0 || b0 >= b1 {
        return Vec::new();
    }
    // compare linear peak levels against the linear threshold (no log per bucket)
    let thr = 10f32.powf(params.threshold_db / 20.0);
    let mut segs: Vec<(f64, f64)> = Vec::new();
    let mut run: Option<f64> = None;
    for i in b0..=b1 {
        let loud = i < b1 && peaks.min[i].abs().max(peaks.max[i].abs()) > thr;
        match (loud, run) {
            (true, None) => run = Some(i as f64 / per),
            (false, Some(start)) => {
                let end = i as f64 / per;
                // absorb a short quiet gap into the previous segment
                match segs.last_mut() {
                    Some(last) if start - last.1 < params.min_silence => last.1 = end,
                    _ => segs.push((start, end)),
                }
                run = None;
            }
            _ => {}
        }
    }
    let (lo, hi) = (src_in, src_in + src_len);
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (a, b) in segs {
        if b - a < params.min_speech {
            continue;
        }
        let (a, b) = ((a - params.padding).max(lo), (b + params.padding).min(hi));
        if b <= a {
            continue;
        }
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b), // padding made neighbours touch
            _ => out.push((a, b)),
        }
    }
    out
}

/// Turn source-time loud segments into timeline cut times + ranges to remove for a clip that starts at
/// `start` with `speed` (forward only; reversed/frozen clips are not auto-cut):
/// returns (cuts, quiet_ranges) in timeline seconds, quiet ranges = the complement of the loud segments
/// inside the clip. `keep_quiet` swaps the roles (keep ambient, remove speech).
pub fn to_timeline(
    segments: &[(f64, f64)],
    start: f64,
    src_in: f64,
    duration: f64,
    speed: f64,
    keep_quiet: bool,
) -> (Vec<f64>, Vec<(f64, f64)>) {
    let speed = if speed > 0.0 { speed } else { 1.0 };
    let end = start + duration;
    let mut loud: Vec<(f64, f64)> = Vec::new();
    for &(a, b) in segments {
        let ta = (start + (a - src_in) / speed).max(start);
        let tb = (start + (b - src_in) / speed).min(end);
        if tb <= ta {
            continue;
        }
        match loud.last_mut() {
            Some(last) if ta <= last.1 => last.1 = last.1.max(tb),
            _ => loud.push((ta, tb)),
        }
    }
    let mut quiet: Vec<(f64, f64)> = Vec::new();
    let mut pos = start;
    for &(a, b) in &loud {
        if a > pos + EPS {
            quiet.push((pos, a));
        }
        pos = b;
    }
    if end > pos + EPS {
        quiet.push((pos, end));
    }
    let mut cuts: Vec<f64> =
        loud.iter().flat_map(|&(a, b)| [a, b]).filter(|&t| t > start + EPS && t < end - EPS).collect();
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() < EPS);
    (cuts, if keep_quiet { loud } else { quiet })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 10 s of peaks with the given loud (source-second) ranges at −6 dB, quiet elsewhere at −80 dB.
    fn peaks(loud: &[(f64, f64)]) -> Peaks {
        let n = 1000;
        let mut min = vec![-0.0001f32; n];
        let mut max = vec![0.0001f32; n];
        for &(a, b) in loud {
            for i in (a * 100.0) as usize..((b * 100.0) as usize).min(n) {
                min[i] = -0.5;
                max[i] = 0.5;
            }
        }
        Peaks { min, max }
    }

    fn close(got: &[(f64, f64)], want: &[(f64, f64)]) -> bool {
        got.len() == want.len()
            && got.iter().zip(want).all(|(g, w)| (g.0 - w.0).abs() < 1e-6 && (g.1 - w.1).abs() < 1e-6)
    }

    #[test]
    fn segments_merge_drop_pad_clamp() {
        let p = peaks(&[(1.0, 3.0), (3.2, 4.0), (5.0, 5.1), (8.0, 9.0)]);
        let params = AutoCutParams::default();
        // gap 3.0–3.2 (< 0.4) merges; 5.0–5.1 (< 0.2) drops; ±0.1 padding
        let segs = loud_segments(&p, 0.0, 10.0, &params);
        assert!(close(&segs, &[(0.9, 4.1), (7.9, 9.1)]), "{segs:?}");
        // window clamps both ends
        let segs = loud_segments(&p, 0.95, 8.0, &params);
        assert!(close(&segs, &[(0.95, 4.1), (7.9, 8.95)]), "{segs:?}");
        // higher threshold → nothing is loud
        let segs = loud_segments(&p, 0.0, 10.0, &AutoCutParams { threshold_db: -3.0, ..params });
        assert!(segs.is_empty(), "{segs:?}");
        // padding joins segments that touch
        let segs = loud_segments(&p, 0.0, 10.0, &AutoCutParams { padding: 2.0, ..params });
        assert!(close(&segs, &[(0.0, 10.0)]), "{segs:?}");
        // empty peaks / empty window
        assert!(loud_segments(&Peaks { min: vec![], max: vec![] }, 0.0, 10.0, &params).is_empty());
        assert!(loud_segments(&p, 0.0, 0.0, &params).is_empty());
        assert!(loud_segments(&p, 20.0, 5.0, &params).is_empty());
    }

    #[test]
    fn timeline_mapping() {
        // plain clip: start 10, src_in 0, dur 5, speed 1
        let (cuts, quiet) = to_timeline(&[(1.0, 2.0), (3.0, 4.0)], 10.0, 0.0, 5.0, 1.0, false);
        assert_eq!(cuts, vec![11.0, 12.0, 13.0, 14.0]);
        assert!(close(&quiet, &[(10.0, 11.0), (12.0, 13.0), (14.0, 15.0)]), "{quiet:?}");
        // keep_quiet swaps: remove the loud parts
        let (cuts2, remove) = to_timeline(&[(1.0, 2.0), (3.0, 4.0)], 10.0, 0.0, 5.0, 1.0, true);
        assert_eq!(cuts2, cuts);
        assert!(close(&remove, &[(11.0, 12.0), (13.0, 14.0)]), "{remove:?}");
        // speed 2, src_in 1: source [1,9) → timeline [0,4); segment reaching the clip edges makes no cut there
        let (cuts, quiet) = to_timeline(&[(1.0, 3.0), (5.0, 9.0)], 0.0, 1.0, 4.0, 2.0, false);
        assert_eq!(cuts, vec![1.0, 2.0]);
        assert!(close(&quiet, &[(1.0, 2.0)]), "{quiet:?}");
        // segment fully outside the clip is ignored
        let (cuts, quiet) = to_timeline(&[(20.0, 30.0)], 0.0, 0.0, 5.0, 1.0, false);
        assert!(cuts.is_empty());
        assert!(close(&quiet, &[(0.0, 5.0)]), "{quiet:?}");
        // loud everywhere → no quiet ranges
        let (cuts, quiet) = to_timeline(&[(0.0, 5.0)], 0.0, 0.0, 5.0, 1.0, false);
        assert!(cuts.is_empty() && quiet.is_empty());
    }
}
