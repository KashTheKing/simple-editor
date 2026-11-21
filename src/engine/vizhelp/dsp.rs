//! ---- ws:visualizer-helper ----
//! The small signal toolbox the song analysis needs (what `hsv_analyze.py` took from numpy / scipy):
//! zero-phase Butterworth band-passes, envelope followers, percentiles, median and moving-average
//! filters, peak picking and Ramer-Douglas-Peucker curve simplification. Plain loops, no allocation in
//! the inner parts; every function is deterministic.

/// A biquad section (Direct Form I), coefficients normalised by a0.
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
}

impl Biquad {
    /// RBJ cookbook low-pass / high-pass at `f` Hz with quality `q`.
    fn new(kind_high: bool, f: f64, q: f64, sr: f64) -> Biquad {
        let w = std::f64::consts::TAU * (f / sr).clamp(1e-6, 0.499);
        let (sn, cs) = w.sin_cos();
        let alpha = sn / (2.0 * q);
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = if kind_high { ((1.0 + cs) / 2.0, -(1.0 + cs), (1.0 + cs) / 2.0) } else { ((1.0 - cs) / 2.0, 1.0 - cs, (1.0 - cs) / 2.0) };
        Biquad { b: [b0 / a0, b1 / a0, b2 / a0], a: [-2.0 * cs / a0, (1.0 - alpha) / a0] }
    }

    fn run(&self, x: &mut [f32]) {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for v in x.iter_mut() {
            let x0 = *v as f64;
            let y0 = self.b[0] * x0 + self.b[1] * x1 + self.b[2] * x2 - self.a[0] * y1 - self.a[1] * y2;
            (x2, x1, y2, y1) = (x1, x0, y1, y0);
            *v = y0 as f32;
        }
    }
}

/// The two sections of a 4th-order Butterworth (Q of each pole pair).
const BUTTER4_Q: [f64; 2] = [0.541_196_1, 1.306_563];

/// A 4th-order Butterworth band-pass `lo`..`hi` Hz run forwards and backwards (zero phase, like scipy's
/// `filtfilt`).
pub fn bandpass(x: &[f32], lo: f64, hi: f64, sr: f64) -> Vec<f32> {
    let mut y = x.to_vec();
    let mut secs = Vec::new();
    for q in BUTTER4_Q {
        secs.push(Biquad::new(true, lo, q, sr));
        secs.push(Biquad::new(false, hi, q, sr));
    }
    for s in &secs {
        s.run(&mut y);
    }
    y.reverse();
    for s in &secs {
        s.run(&mut y);
    }
    y.reverse();
    y
}

/// RMS in dB of consecutive `hop`-sample blocks (the last partial one dropped), floored at `floor`.
pub fn block_db(x: &[f32], hop: usize, floor: f64) -> Vec<f64> {
    x.chunks_exact(hop.max(1))
        .map(|c| {
            let ms = c.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / c.len() as f64;
            20.0 * (ms.sqrt() + floor).log10()
        })
        .collect()
}

/// An attack / release envelope follower over a series sampled every `dt` seconds.
pub fn follow(x: &[f64], attack: f64, release: f64, dt: f64) -> Vec<f64> {
    let (a, r) = (1.0 - (-dt / attack).exp(), 1.0 - (-dt / release).exp());
    let mut y = vec![0.0; x.len()];
    for k in 1..x.len() {
        let c = if x[k] > y[k - 1] { a } else { r };
        y[k] = y[k - 1] + (x[k] - y[k - 1]) * c;
    }
    y
}

/// The `p`-th percentile (0..100) with linear interpolation, as numpy's default. 0 for an empty input.
pub fn percentile(v: &[f64], p: f64) -> f64 {
    let mut s: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if s.is_empty() {
        return 0.0;
    }
    s.sort_by(|a, b| a.total_cmp(b));
    let pos = (p / 100.0).clamp(0.0, 1.0) * (s.len() - 1) as f64;
    let (i, f) = (pos.floor() as usize, pos.fract());
    if i + 1 < s.len() {
        s[i] + (s[i + 1] - s[i]) * f
    } else {
        s[i]
    }
}

/// A centred running median over `w` samples (edges use the samples that exist).
pub fn median_filter(v: &[f64], w: usize) -> Vec<f64> {
    let h = w / 2;
    let mut buf: Vec<f64> = Vec::with_capacity(w + 1);
    (0..v.len())
        .map(|i| {
            buf.clear();
            buf.extend_from_slice(&v[i.saturating_sub(h)..(i + h + 1).min(v.len())]);
            buf.sort_by(|a, b| a.total_cmp(b));
            buf[buf.len() / 2]
        })
        .collect()
}

/// A moving average over `w` samples, its window shifted by `shift` samples (positive = looks ahead, like
/// scipy's `origin = -shift`). Edges average what exists.
pub fn uniform(v: &[f64], w: usize, shift: isize) -> Vec<f64> {
    let n = v.len();
    let w = w.max(1) as isize;
    let mut cs = vec![0.0; n + 1];
    for i in 0..n {
        cs[i + 1] = cs[i] + v[i];
    }
    (0..n as isize)
        .map(|i| {
            let lo = (i - w / 2 + shift).clamp(0, n as isize) as usize;
            let hi = (i - w / 2 + shift + w).clamp(0, n as isize) as usize;
            if hi > lo {
                (cs[hi] - cs[lo]) / (hi - lo) as f64
            } else {
                v[i as usize]
            }
        })
        .collect()
}

/// Indices of local maxima at least `height` high and at least `distance` apart (the higher one wins), in
/// time order: scipy's `find_peaks(x, height, distance)`.
pub fn find_peaks(x: &[f64], height: f64, distance: usize) -> Vec<usize> {
    let mut cand: Vec<usize> = (1..x.len().saturating_sub(1)).filter(|&i| x[i] >= height && x[i] > x[i - 1] && x[i] >= x[i + 1]).collect();
    let mut order = cand.clone();
    order.sort_by(|&a, &b| x[b].total_cmp(&x[a]).then(a.cmp(&b)));
    let mut keep = vec![false; x.len()];
    let mut taken: Vec<usize> = Vec::new();
    for i in order {
        let lo = taken.partition_point(|&t| t + distance <= i);
        if taken.get(lo).is_some_and(|&t| t < i + distance) {
            continue;
        }
        taken.insert(lo, i);
        keep[i] = true;
    }
    cand.retain(|&i| keep[i]);
    cand
}

/// Ramer-Douglas-Peucker: the indices of `(ts, ys)` to keep so straight lines between them stay within
/// `tol` of every dropped sample. Iterative (a long song never overflows the stack).
pub fn rdp(ts: &[f64], ys: &[f64], tol: f64) -> Vec<usize> {
    let n = ts.len();
    if n <= 2 {
        return (0..n).collect();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((i, j)) = stack.pop() {
        if j <= i + 1 {
            continue;
        }
        let (mut worst, mut at) = (0.0, 0);
        let span = (ts[j] - ts[i]).max(1e-12);
        for k in i + 1..j {
            let line = ys[i] + (ys[j] - ys[i]) * (ts[k] - ts[i]) / span;
            let e = (ys[k] - line).abs();
            if e > worst {
                (worst, at) = (e, k);
            }
        }
        if worst > tol {
            keep[at] = true;
            stack.push((i, at));
            stack.push((at, j));
        }
    }
    (0..n).filter(|&k| keep[k]).collect()
}

/// `series` (one value every `dt` s from 0) resampled every `step` s, then simplified with RDP at `tol`:
/// `[t, v]` points, rounded to 1 ms / 0.001.
pub fn curve(series: &[f64], dt: f64, step: f64, tol: f64) -> Vec<[f32; 2]> {
    if series.is_empty() {
        return Vec::new();
    }
    let dur = (series.len() - 1) as f64 * dt;
    let n = (dur / step).floor() as usize + 1;
    let ts: Vec<f64> = (0..n).map(|i| i as f64 * step).collect();
    let vs: Vec<f64> = ts.iter().map(|&t| interp(series, t / dt)).collect();
    rdp(&ts, &vs, tol).into_iter().map(|i| [r3(ts[i]) as f32, r3(vs[i]) as f32]).collect()
}

/// `v` read at fractional index `x` (linear, clamped at the ends).
pub fn interp(v: &[f64], x: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let x = x.clamp(0.0, (v.len() - 1) as f64);
    let i = x.floor() as usize;
    let f = x - i as f64;
    if i + 1 < v.len() {
        v[i] + (v[i + 1] - v[i]) * f
    } else {
        v[i]
    }
}

/// Round to 1/1000.
pub fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bandpass_keeps_the_band_and_drops_the_rest() {
        let sr = 8000.0;
        let tone = |f: f64| -> Vec<f32> { (0..8000).map(|i| (std::f64::consts::TAU * f * i as f64 / sr).sin() as f32).collect() };
        let rms = |x: &[f32]| (x[2000..6000].iter().map(|v| v * v).sum::<f32>() / 4000.0).sqrt();
        assert!(rms(&bandpass(&tone(100.0), 40.0, 150.0, sr)) > 0.5);
        assert!(rms(&bandpass(&tone(1500.0), 40.0, 150.0, sr)) < 0.02);
    }

    #[test]
    fn peaks_keep_the_higher_one_inside_the_distance() {
        let x = [0.0, 1.0, 0.0, 3.0, 0.0, 0.0, 0.0, 2.0, 0.0];
        assert_eq!(find_peaks(&x, 0.5, 3), vec![3, 7]);
        assert_eq!(find_peaks(&x, 0.5, 1), vec![1, 3, 7]);
    }

    #[test]
    fn percentile_and_median_match_numpy() {
        let v = [4.0, 1.0, 3.0, 2.0];
        assert_eq!(percentile(&v, 50.0), 2.5);
        assert_eq!(percentile(&v, 100.0), 4.0);
        assert_eq!(median_filter(&[1.0, 9.0, 2.0, 3.0, 4.0], 3), vec![9.0, 2.0, 3.0, 3.0, 4.0]);
    }

    #[test]
    fn rdp_drops_points_on_a_line() {
        let ts: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let mut ys = ts.clone();
        ys[5] = 9.0;
        assert_eq!(rdp(&ts, &ys, 0.1), vec![0, 4, 5, 6, 9]);
    }
}
