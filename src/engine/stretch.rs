//! Pitch-keeping time stretch (WSOLA). The one stretch path: the mixer runs every Keep Pitch clip through it
//! (playback, scrubbing, export and the audio bakes all mix through `Mixer`), speed ramps included since the
//! grains are placed from the clip's own source clock, and the editor's shuttle (JKL) runs the timeline mix
//! through it.
//!
//! Output is cut into Hann grains of `GRAIN` frames every `HOP` (half a grain, so the windows sum to 1). Grain
//! `k` starts at output frame `k * HOP` and reads `GRAIN` source frames at the speed of 1 (so the pitch stays)
//! from where the source clock says it should start, nudged by up to `SEEK` frames to the spot that best
//! continues the waveform of the grain before it. The chosen positions are remembered (`Grains`), so blocks
//! of any size and alignment join without a seam; a seek or an edit only loses the nudge for one grain.
//!
//! Chosen over a crate: no small, permissively licensed Rust stretcher exists (Rubber Band is GPL, the
//! Signalsmith binding needs a C++ build), and this is ~150 lines. Cost: about 60 correlations of 128 samples per
//! grain (~94 grains a second), a few ms of the audio thread per second of a stretched clip.

use crate::model::Clip;

/// Frames per grain (~21 ms at 48 kHz).
pub const GRAIN: i64 = 1024;
/// Output frames between grain starts.
pub const HOP: i64 = GRAIN / 2;
/// How far (frames, either way) a grain may move to line up with the one before (~5 ms: down to ~100 Hz).
const SEEK: i64 = 240;
/// Above this speed the sound is muted rather than stretched: grains would skip most of the source.
pub const MAX_RATE: f64 = 4.0;

/// Chosen source frame of the most recent grains, by grain index, and the source frames read from `src_at`
/// on (kept, so each block reads only the frames after them: the decoder sees one contiguous read, as at 1x).
#[derive(Default)]
pub struct Grains {
    sig: u64,
    hist: Vec<(i64, i64)>,
    src: Vec<f32>,
    src_at: i64,
}

/// Periodic Hann window: shifted by `HOP` it sums to exactly 1.
fn hann(i: i64) -> f32 {
    (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / GRAIN as f64).cos()) as f32
}

impl Grains {
    fn cached(&self, k: i64) -> Option<i64> {
        self.hist.iter().find(|h| h.0 == k).map(|h| h.1)
    }

    /// Overwrite `out` (interleaved stereo) with output frames `[j0, j0 + out.len()/2)`.
    /// - `sig`: what maps output to source; when it changes the remembered grains are dropped.
    /// - `dir`: 1 plays the source forwards, -1 backwards (each grain read backwards too).
    /// - `nominal(k)`: the source frame grain `k` starts at before the nudge.
    /// - `read(f0, buf)`: add source frames `f0..` into `buf` (zeroed). Forwards, each call starts where
    ///   the last one ended unless the output jumped.
    pub fn render(
        &mut self,
        sig: u64,
        j0: i64,
        out: &mut [f32],
        dir: i64,
        nominal: impl Fn(i64) -> i64,
        read: impl FnOnce(i64, &mut [f32]),
    ) {
        out.fill(0.0);
        let n = (out.len() / 2) as i64;
        if n == 0 {
            return;
        }
        if sig != self.sig {
            self.sig = sig;
            self.hist.clear();
            self.src.clear();
        }
        let (k_lo, k_hi) = (j0.div_euclid(HOP) - 1, (j0 + n - 1).div_euclid(HOP));
        // the source span every grain (and the previous grain the first one continues) can touch
        let first = if self.cached(k_lo - 1).is_some() { k_lo - 1 } else { k_lo };
        let (mut lo, mut hi) = (i64::MAX, i64::MIN);
        for k in first..=k_hi {
            let (a, b) = match self.cached(k) {
                Some(p) => (p, p),
                None => {
                    let c = nominal(k);
                    (c - SEEK, c + SEEK)
                }
            };
            let (a, b) = if dir > 0 { (a, b + GRAIN - 1) } else { (a - GRAIN + 1, b) };
            (lo, hi) = (lo.min(a), hi.max(b));
        }
        let have = (self.src.len() / 2) as i64;
        if dir > 0 && lo >= self.src_at && lo <= self.src_at + have {
            self.src.drain(..((lo - self.src_at) * 2) as usize);
            self.src_at = lo;
            let (have, need) = (self.src.len(), ((hi - lo + 1) * 2) as usize);
            if need > have {
                self.src.resize(need, 0.0);
                read(lo + (have / 2) as i64, &mut self.src[have..]);
            }
        } else {
            // a jump (or backwards, which reads a block at a time like the resampler): read the span afresh
            self.src.clear();
            self.src.resize(((hi - lo + 1) * 2) as usize, 0.0);
            self.src_at = lo;
            read(lo, &mut self.src);
        }
        let src = &self.src;
        // mono sample `i` frames along a grain that starts at source frame `f`
        let at = |f: i64, i: i64| -> f32 {
            let x = ((f + dir * i - lo) * 2) as usize;
            src.get(x..x + 2).map_or(0.0, |s| s[0] + s[1])
        };
        self.hist.retain(|h| h.0 >= k_lo - 1);
        let mut prev = self.cached(k_lo - 1);
        for k in k_lo..=k_hi {
            let p = match self.cached(k) {
                Some(p) => p,
                None => {
                    let c = nominal(k);
                    let p = match prev {
                        // the natural continuation of the previous grain, over the frames they overlap
                        Some(pp) => {
                            let nat = pp + dir * HOP;
                            let (mut best, mut score) = (c, f32::MIN);
                            // ponytail: every 2nd lag, every 4th sample - a frame of lag error is inaudible
                            for d in (-SEEK..=SEEK).step_by(2) {
                                let (mut xy, mut yy) = (0.0f32, 1e-9f32);
                                for i in (0..HOP).step_by(4) {
                                    let y = at(c + d, i);
                                    xy += at(nat, i) * y;
                                    yy += y * y;
                                }
                                let s = xy / yy.sqrt();
                                if s > score {
                                    (best, score) = (c + d, s);
                                }
                            }
                            best
                        }
                        None => c,
                    };
                    self.hist.push((k, p));
                    p
                }
            };
            prev = Some(p);
            let g0 = k * HOP;
            for j in g0.max(j0)..(g0 + GRAIN).min(j0 + n) {
                let i = j - g0;
                let x = ((p + dir * i - lo) * 2) as usize;
                if let Some(s) = src.get(x..x + 2) {
                    let w = hann(i);
                    let o = ((j - j0) * 2) as usize;
                    out[o] += s[0] * w;
                    out[o + 1] += s[1] * w;
                }
            }
        }
        self.hist.retain(|h| h.0 >= k_hi - 2);
    }
}

/// The mixer stretches this clip's sound (Keep Pitch on and its speed is not 1).
// ponytail: a time-remapped clip stays on the varispeed path (`mixer::resample_mapped`, pitch follows speed);
// stretching it means driving grains through `map.speed` with holds/backward muted, as the time-remap hand-off describes.
pub fn stretches(clip: &Clip) -> bool {
    clip.keep_pitch && clip.freeze.is_none() && clip.time_map.is_none() && ((clip.speed - 1.0).abs() > 1e-9 || clip.speed_curve.is_animated())
}

/// A stretched clip plays too fast to hear somewhere in timeline `[t0, t1]` (muted for that block).
pub fn too_fast(clip: &Clip, t0: f64, t1: f64) -> bool {
    clip.rate(clip.local(t0)) > MAX_RATE || clip.rate(clip.local(t1)) > MAX_RATE
}

/// Changes whenever the clip's output -> source mapping does (speed, trim, move, reverse).
// ponytail: an edited ramp key keeps the signature; the grains it misplaces are gone within ~30 ms
pub fn sig(clip: &Clip) -> u64 {
    clip.speed.to_bits()
        ^ clip.src_in.to_bits().rotate_left(13)
        ^ clip.start.to_bits().rotate_left(29)
        ^ clip.duration.to_bits().rotate_left(43)
        ^ clip.reverse as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::SAMPLE_RATE;

    const SR: f64 = SAMPLE_RATE as f64;

    /// Dominant frequency (Hz) of a mono signal: the zero crossings counted over it.
    fn freq(x: &[f32]) -> f64 {
        let ups = x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        ups as f64 * SR / x.len() as f64
    }

    /// Stretch a 440 Hz sine at `rate` the way the mixer does (blocks of 1024 frames from a source clock),
    /// returning the left channel of `secs` of output.
    fn stretched(rate: f64, secs: f64) -> Vec<f32> {
        let sine = |f: i64| (std::f64::consts::TAU * 440.0 * f as f64 / SR).sin() as f32;
        let mut g = Grains::default();
        let frames = (secs * SR) as usize;
        let mut y = Vec::new();
        let mut block = vec![0f32; 2048];
        let mut j = 0i64;
        let mut last_read = i64::MIN;
        while y.len() < frames {
            g.render(1, j, &mut block, 1, |k| (k as f64 * HOP as f64 * rate).round() as i64, |f0, buf| {
                assert!(f0 >= last_read, "reads go forwards: {f0} after {last_read}");
                last_read = f0;
                for (i, s) in buf.chunks_exact_mut(2).enumerate() {
                    let v = sine(f0 + i as i64);
                    s[0] += v;
                    s[1] += v;
                }
            });
            y.extend(block.chunks_exact(2).map(|s| s[0]));
            j += 1024;
        }
        y.truncate(frames);
        y
    }

    #[test]
    fn stretch_keeps_the_pitch_and_the_level() {
        for rate in [1.5, 0.5, 0.25, 4.0] {
            let y = stretched(rate, 1.0);
            let body = &y[GRAIN as usize..];
            let f = freq(body);
            assert!((f - 440.0).abs() < 440.0 * 0.02, "rate {rate}: {f} Hz");
            let rms = (body.iter().map(|v| v * v).sum::<f32>() / body.len() as f32).sqrt();
            assert!((rms - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.08, "rate {rate}: rms {rms}");
        }
    }

    /// Share of `x`'s energy at `hz` (Goertzel over whole cycles): 1.0 is a pure tone at that pitch.
    fn purity(x: &[f32], hz: f64) -> f64 {
        let n = ((x.len() as f64 * hz / SR).floor() * SR / hz) as usize;
        let w = std::f64::consts::TAU * hz / SR;
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &v in &x[..n] {
            let s = v as f64 + 2.0 * w.cos() * s1 - s2;
            (s2, s1) = (s1, s);
        }
        let power = (s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2) * 2.0 / n as f64;
        let total = x[..n].iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
        power / total
    }

    /// The sine test with a spectrum check: at every speed from 0.25x to 4x nearly all the energy stays at
    /// the tone's own frequency (a resample would move it; seams between grains would spread it).
    #[test]
    fn stretch_is_clean_from_quarter_to_4x() {
        for hz in [150.0, 440.0, 1000.0] {
            for rate in [0.25, 0.5, 0.8, 1.25, 2.0, 3.0, 4.0] {
                let sine = move |f: i64| (std::f64::consts::TAU * hz * f as f64 / SR).sin() as f32;
                let mut g = Grains::default();
                let mut y = Vec::new();
                let mut block = vec![0f32; 2048];
                for b in 0..60 {
                    let nominal = |k: i64| (k as f64 * HOP as f64 * rate).round() as i64;
                    g.render(1, b * 1024, &mut block, 1, nominal, |f0, buf| {
                        for (i, s) in buf.chunks_exact_mut(2).enumerate() {
                            let v = sine(f0 + i as i64);
                            s[0] += v;
                            s[1] += v;
                        }
                    });
                    y.extend(block.chunks_exact(2).map(|s| s[0]));
                }
                // in ~170 ms windows: a seam may land a fraction of a frame off, a phase slip of well under a
                // cent that a whole-signal measure would count as lost energy
                for w in y[GRAIN as usize..].chunks_exact(8192) {
                    let p = purity(w, hz);
                    assert!(p > 0.95, "{hz} Hz at {rate}x: {:.1} % of the energy at the tone", p * 100.0);
                    // and nowhere near where a resample would have put it
                    if (rate - 1.0f64).abs() > 0.2 {
                        assert!(purity(w, hz * rate) < 0.2, "{hz} Hz at {rate}x sounds resampled");
                    }
                }
            }
        }
    }

    #[test]
    fn reverse_keeps_the_pitch() {
        let mut g = Grains::default();
        let mut y = Vec::new();
        let mut out = vec![0f32; 2048];
        let start = 200_000i64;
        for b in 0..40 {
            g.render(2, b * 1024, &mut out, -1, |k| start - (k * HOP) / 2, |f0, buf| {
                for (i, s) in buf.chunks_exact_mut(2).enumerate() {
                    s[0] = (std::f64::consts::TAU * 440.0 * (f0 + i as i64) as f64 / SR).sin() as f32;
                    s[1] = s[0];
                }
            });
            y.extend(out.chunks_exact(2).map(|s| s[0]));
        }
        let f = freq(&y[2048..]);
        assert!((f - 440.0).abs() < 9.0, "{f} Hz");
    }
}
