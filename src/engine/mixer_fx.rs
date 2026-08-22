//! Audio filters and bus routing: the DSP behind the mixer panel.
//!
//! `BusGraph` resolves `Project.buses` (Main first) into an evaluation order and owns one `FilterState`
//! per (bus, filter) so IIR/delay state survives across blocks. Clips sum into their bus
//! (`Project::bus_of`), each bus runs its filter chain, then gain/pan/mono, then sums into its output bus.
//! Interleaved stereo f32 @ 48 kHz, processed in place, block-continuous.
//!
//! Filters (`model::FilterKind`): Eq (5-band RBJ: low shelf, three peaks, high shelf — `EQ_BANDS` maps
//! bands to parameter indices), HighPass/LowPass (RBJ),
//! Reverb (Freeverb comb+allpass), Echo (delay line + feedback, optional ping-pong), Distortion (soft clip
//! + tone), Compressor (peak detector, attack/release, makeup), NoiseGate, Noise (white/pink/tone), Gain.
//!
//! Parameters: every `AudioFilter` param is an `Animated` read **once per block** at the block-start
//! timeline time `t` (`AudioFilter::at(i, t)`). Coefficients / delay lengths / thresholds are recomputed
//! from those values and held constant for the block, so automation steps at block boundaries
//! (≤ ~100 ms). The two params where a step is audible as a click — the Gain filter's gain and the Noise
//! filter's level — are ramped linearly from the previous block's value across the block. All buffers
//! (delay lines, comb/allpass memories, bus scratch) are sized at construction / first block, so steady
//! state allocates nothing.

use crate::media::SAMPLE_RATE;
use crate::model::{AudioFilter, Bus, FilterKind, Id, Project};

/// Linear amplitude of a dB value.
pub fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// dB of a linear amplitude (floored at -120 dB).
pub fn lin_to_db(v: f32) -> f32 {
    20.0 * v.abs().max(1e-6).log10()
}

/// One-pole smoothing coefficient for a time constant in ms.
fn coef(ms: f32, sr: f32) -> f32 {
    let n = (ms.max(0.01) * 0.001 * sr).max(1.0);
    1.0 - (-1.0 / n).exp()
}

// ---------- biquad ----------

/// RBJ band shapes used by the EQ and the pass filters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Band {
    LowPass,
    HighPass,
    Peak,
    LowShelf,
    HighShelf,
}

/// Normalised RBJ coefficients `[b0, b1, b2, a1, a2]` (a0 divided out). Shelves take Q like the peaks
/// (Q = 0.707 is the classic slope-1 shelf).
pub fn coeffs(band: Band, f0: f32, q: f32, gain_db: f32, sr: f32) -> [f32; 5] {
    let f0 = f0.clamp(10.0, sr * 0.45);
    let q = q.clamp(0.05, 20.0);
    let w0 = std::f32::consts::TAU * f0 / sr;
    let (sn, cs) = w0.sin_cos();
    let alpha = sn / (2.0 * q);
    let a = 10f32.powf(gain_db / 40.0);
    let tsa = 2.0 * a.sqrt() * alpha;
    let (b0, b1, b2, a0, a1, a2) = match band {
        Band::LowPass => ((1.0 - cs) * 0.5, 1.0 - cs, (1.0 - cs) * 0.5, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
        Band::HighPass => ((1.0 + cs) * 0.5, -(1.0 + cs), (1.0 + cs) * 0.5, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
        Band::Peak => (1.0 + alpha * a, -2.0 * cs, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cs, 1.0 - alpha / a),
        Band::LowShelf => (
            a * ((a + 1.0) - (a - 1.0) * cs + tsa),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
            a * ((a + 1.0) - (a - 1.0) * cs - tsa),
            (a + 1.0) + (a - 1.0) * cs + tsa,
            -2.0 * ((a - 1.0) + (a + 1.0) * cs),
            (a + 1.0) + (a - 1.0) * cs - tsa,
        ),
        Band::HighShelf => (
            a * ((a + 1.0) + (a - 1.0) * cs + tsa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
            a * ((a + 1.0) + (a - 1.0) * cs - tsa),
            (a + 1.0) - (a - 1.0) * cs + tsa,
            2.0 * ((a - 1.0) - (a + 1.0) * cs),
            (a + 1.0) - (a - 1.0) * cs - tsa,
        ),
    };
    if a0.abs() < 1e-12 {
        return [1.0, 0.0, 0.0, 0.0, 0.0];
    }
    [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]
}

/// |H(e^jw)| in dB at `freq` for one set of normalised coefficients.
pub fn response_db(c: &[f32; 5], freq: f32, sr: f32) -> f32 {
    let w = std::f32::consts::TAU * freq.clamp(1.0, sr * 0.5) / sr;
    let (s1, c1) = (-w).sin_cos();
    let (s2, c2) = (-2.0 * w).sin_cos();
    let nr = c[0] + c[1] * c1 + c[2] * c2;
    let ni = c[1] * s1 + c[2] * s2;
    let dr = 1.0 + c[3] * c1 + c[4] * c2;
    let di = c[3] * s1 + c[4] * s2;
    let num = (nr * nr + ni * ni).sqrt();
    let den = (dr * dr + di * di).sqrt().max(1e-12);
    lin_to_db(num / den)
}

/// The EQ's five bands, low to high, as `(shape, gain param, freq param, Q param)`. The first seven
/// `F_EQ` slots keep the meaning the 3-band EQ gave them, so projects saved before the extra peaks
/// still load — hence the scattered indices.
pub const EQ_BANDS: [(Band, usize, usize, usize); 5] = [
    (Band::LowShelf, 0, 1, 7),
    (Band::Peak, 9, 10, 11),
    (Band::Peak, 2, 3, 4),
    (Band::Peak, 12, 13, 14),
    (Band::HighShelf, 5, 6, 8),
];

/// The bands an EQ / pass filter is made of at time `t`, as `(band, freq, q, gain_db)`.
/// Empty for every other kind — the UI draws a response curve exactly when this is non-empty.
pub fn filter_bands(f: &AudioFilter, t: f64) -> Vec<(Band, f32, f32, f32)> {
    let p = |i: usize| f.at(i, t) as f32;
    match f.kind {
        FilterKind::Eq => EQ_BANDS.iter().map(|&(b, gi, fi, qi)| (b, p(fi), p(qi), p(gi))).collect(),
        FilterKind::HighPass => vec![(Band::HighPass, p(0), p(1), 0.0)],
        FilterKind::LowPass => vec![(Band::LowPass, p(0), p(1), 0.0)],
        _ => Vec::new(),
    }
}

/// Total response of an EQ / pass filter at `freq`, in dB (0 for other kinds).
pub fn filter_response_db(f: &AudioFilter, t: f64, freq: f32) -> f32 {
    let sr = SAMPLE_RATE as f32;
    filter_bands(f, t).iter().map(|&(b, f0, q, g)| response_db(&coeffs(b, f0, q, g, sr), freq, sr)).sum()
}

/// Transposed direct-form II biquad, one per channel per band.
#[derive(Clone, Copy, Default)]
struct Biquad {
    c: [f32; 5],
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.c[0] * x + self.z1;
        self.z1 = self.c[1] * x - self.c[3] * y + self.z2;
        self.z2 = self.c[2] * x - self.c[4] * y;
        if !y.is_finite() {
            self.z1 = 0.0;
            self.z2 = 0.0;
            return 0.0;
        }
        y
    }
}

// ---------- reverb (Freeverb) ----------

const COMB_LEN: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLP_LEN: [usize; 4] = [556, 441, 341, 225];
/// Right-channel stereo spread, in samples at 44.1 kHz (Freeverb's `stereospread`).
const SPREAD: usize = 23;
/// Freeverb's `fixedgain`: how much of the dry signal is pushed into the comb bank.
const REV_IN_GAIN: f32 = 0.015;

struct Comb {
    buf: Vec<f32>,
    pos: usize,
    store: f32,
}

impl Comb {
    fn new(n: usize) -> Self {
        Self { buf: vec![0.0; n.max(1)], pos: 0, store: 0.0 }
    }
    fn tick(&mut self, x: f32, fb: f32, damp: f32) -> f32 {
        let y = self.buf[self.pos];
        self.store = y * (1.0 - damp) + self.store * damp;
        let v = x + self.store * fb;
        self.buf[self.pos] = if v.is_finite() { v } else { 0.0 };
        self.pos = (self.pos + 1) % self.buf.len();
        y
    }
}

struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(n: usize) -> Self {
        Self { buf: vec![0.0; n.max(1)], pos: 0 }
    }
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.buf[self.pos];
        let v = x + y * 0.5;
        self.buf[self.pos] = if v.is_finite() { v } else { 0.0 };
        self.pos = (self.pos + 1) % self.buf.len();
        y - x
    }
}

struct Freeverb {
    combs: [[Comb; 8]; 2],
    allp: [[Allpass; 4]; 2],
    /// Interleaved stereo pre-delay line, sized for the 200 ms parameter maximum.
    pre: Vec<f32>,
    pre_pos: usize,
}

impl Freeverb {
    fn new(sr: f32) -> Self {
        let scale = sr / 44100.0;
        let n = |base: usize, off: usize| ((base + off) as f32 * scale) as usize;
        Self {
            combs: [
                std::array::from_fn(|i| Comb::new(n(COMB_LEN[i], 0))),
                std::array::from_fn(|i| Comb::new(n(COMB_LEN[i], SPREAD))),
            ],
            allp: [
                std::array::from_fn(|i| Allpass::new(n(ALLP_LEN[i], 0))),
                std::array::from_fn(|i| Allpass::new(n(ALLP_LEN[i], SPREAD))),
            ],
            pre: vec![0.0; ((sr * 0.201) as usize + 2) * 2],
            pre_pos: 0,
        }
    }

    /// One frame: returns the wet (l, r).
    fn tick(&mut self, l: f32, r: f32, fb: f32, damp: f32, w1: f32, w2: f32, pre_frames: usize) -> (f32, f32) {
        let cap = self.pre.len() / 2;
        let wp = self.pre_pos * 2;
        self.pre[wp] = l;
        self.pre[wp + 1] = r;
        let d = pre_frames.min(cap - 1);
        let rp = (self.pre_pos + cap - d) % cap;
        let (dl, dr) = (self.pre[rp * 2], self.pre[rp * 2 + 1]);
        self.pre_pos = (self.pre_pos + 1) % cap;

        let input = (dl + dr) * REV_IN_GAIN;
        let mut wet = [0.0f32; 2];
        for ch in 0..2 {
            let mut acc = 0.0;
            for c in self.combs[ch].iter_mut() {
                acc += c.tick(input, fb, damp);
            }
            for a in self.allp[ch].iter_mut() {
                acc = a.tick(acc);
            }
            wet[ch] = acc;
        }
        (wet[0] * w1 + wet[1] * w2, wet[1] * w1 + wet[0] * w2)
    }
}

// ---------- per-filter state ----------

/// Interleaved-stereo delay line used by Echo.
struct Delay {
    buf: Vec<f32>,
    pos: usize,
}

/// The DSP memory a filter kind needs. Sized once in `FilterState::new`.
enum Dsp {
    /// 1 band (pass filters) or `EQ_BANDS.len()` bands (EQ) × 2 channels, band-major.
    Biquads(Vec<Biquad>),
    Reverb(Box<Freeverb>),
    Echo(Delay),
    /// Distortion tone one-pole, per channel.
    Tone([f32; 2]),
    /// Compressor / gate: linked peak envelope + smoothed gain.
    Dyn {
        env: f32,
        gain: f32,
    },
    Noise {
        rng: u32,
        pink: [[f32; 3]; 2],
        phase: f32,
        prev: f32,
    },
    Gain {
        prev: f32,
    },
}

/// Per-filter DSP state (biquad histories, delay lines, envelopes).
pub struct FilterState {
    kind: FilterKind,
    sr: f32,
    dsp: Dsp,
}

/// Longest Echo delay the parameter allows, in seconds (`F_ECHO`'s "Delay ms" max).
const MAX_ECHO_S: f32 = 2.0;

impl FilterState {
    pub fn new(f: &AudioFilter, sample_rate: u32) -> Self {
        let sr = if sample_rate == 0 { SAMPLE_RATE as f32 } else { sample_rate as f32 };
        let dsp = match f.kind {
            FilterKind::Eq => Dsp::Biquads(vec![Biquad::default(); EQ_BANDS.len() * 2]),
            FilterKind::HighPass | FilterKind::LowPass => Dsp::Biquads(vec![Biquad::default(); 2]),
            FilterKind::Reverb => Dsp::Reverb(Box::new(Freeverb::new(sr))),
            FilterKind::Echo => Dsp::Echo(Delay { buf: vec![0.0; ((sr * MAX_ECHO_S) as usize + 2) * 2], pos: 0 }),
            FilterKind::Distortion => Dsp::Tone([0.0; 2]),
            FilterKind::Compressor | FilterKind::NoiseGate => Dsp::Dyn { env: 0.0, gain: 1.0 },
            FilterKind::Noise => Dsp::Noise { rng: 0x1234_5678, pink: [[0.0; 3]; 2], phase: 0.0, prev: f32::NAN },
            FilterKind::Gain => Dsp::Gain { prev: f32::NAN },
        };
        Self { kind: f.kind, sr, dsp }
    }

    /// The kind this state was built for — the graph rebuilds when the user changes a slot's filter.
    pub fn kind(&self) -> FilterKind {
        self.kind
    }

    /// Process one block in place. `t` = timeline seconds at the block start (keyframed params).
    pub fn process(&mut self, f: &AudioFilter, t: f64, buf: &mut [f32]) {
        if f.kind != self.kind || buf.len() < 2 {
            return;
        }
        let sr = self.sr;
        let p = |i: usize| f.at(i, t) as f32;
        match (&mut self.dsp, f.kind) {
            (Dsp::Biquads(bq), FilterKind::Eq) => {
                for (b, &(band, gi, fi, qi)) in bq.chunks_exact_mut(2).zip(EQ_BANDS.iter()) {
                    let c = coeffs(band, p(fi), p(qi).max(0.1), p(gi), sr);
                    b[0].c = c;
                    b[1].c = c;
                }
                for fr in buf.chunks_exact_mut(2) {
                    for b in bq.chunks_exact_mut(2) {
                        fr[0] = b[0].tick(fr[0]);
                        fr[1] = b[1].tick(fr[1]);
                    }
                }
            }
            (Dsp::Biquads(bq), FilterKind::HighPass | FilterKind::LowPass) => {
                let band = if f.kind == FilterKind::HighPass { Band::HighPass } else { Band::LowPass };
                let c = coeffs(band, p(0), p(1), 0.0, sr);
                bq[0].c = c;
                bq[1].c = c;
                for fr in buf.chunks_exact_mut(2) {
                    fr[0] = bq[0].tick(fr[0]);
                    fr[1] = bq[1].tick(fr[1]);
                }
            }
            (Dsp::Reverb(rev), _) => {
                let room = p(0).clamp(0.0, 1.0);
                let damp = p(1).clamp(0.0, 1.0) * 0.4;
                let width = p(2).clamp(0.0, 1.0);
                let mix = p(3).clamp(0.0, 1.0);
                let pre = ((p(4).clamp(0.0, 200.0) * 0.001) * sr) as usize;
                let fb = room * 0.28 + 0.7;
                let (w1, w2) = (width * 0.5 + 0.5, (1.0 - width) * 0.5);
                for fr in buf.chunks_exact_mut(2) {
                    let (wl, wr) = rev.tick(fr[0], fr[1], fb, damp, w1, w2, pre);
                    fr[0] = fr[0] * (1.0 - mix) + wl * mix;
                    fr[1] = fr[1] * (1.0 - mix) + wr * mix;
                }
            }
            (Dsp::Echo(d), _) => {
                let cap = d.buf.len() / 2;
                let frames = ((p(0).clamp(1.0, MAX_ECHO_S * 1000.0) * 0.001) * sr) as usize;
                let frames = frames.clamp(1, cap - 1);
                let fb = p(1).clamp(0.0, 0.95);
                let mix = p(2).clamp(0.0, 1.0);
                let ping = p(3) >= 0.5;
                for fr in buf.chunks_exact_mut(2) {
                    let rp = (d.pos + cap - frames) % cap;
                    let (dl, dr) = (d.buf[rp * 2], d.buf[rp * 2 + 1]);
                    let (yl, yr) = if ping { (dr, dl) } else { (dl, dr) };
                    d.buf[d.pos * 2] = fr[0] + yl * fb;
                    d.buf[d.pos * 2 + 1] = fr[1] + yr * fb;
                    d.pos = (d.pos + 1) % cap;
                    fr[0] = fr[0] * (1.0 - mix) + yl * mix;
                    fr[1] = fr[1] * (1.0 - mix) + yr * mix;
                }
            }
            (Dsp::Tone(lp), _) => {
                let drive = p(0).clamp(1.0, 50.0);
                let tone = p(1).clamp(0.0, 1.0);
                let mix = p(2).clamp(0.0, 1.0);
                let norm = 1.0 / drive.tanh();
                // one-pole low-pass at ~3 kHz; `tone` blends it back towards the raw clip
                let a = coef(0.053, sr);
                for fr in buf.chunks_exact_mut(2) {
                    for ch in 0..2 {
                        let x = fr[ch];
                        let y = (x * drive).tanh() * norm;
                        lp[ch] += (y - lp[ch]) * a;
                        let y = lp[ch] + (y - lp[ch]) * tone;
                        fr[ch] = x * (1.0 - mix) + y * mix;
                    }
                }
            }
            (Dsp::Dyn { env, gain }, FilterKind::Compressor) => {
                let thr = db_to_lin(p(0).clamp(-60.0, 0.0));
                let ratio = p(1).max(1.0);
                let at = coef(p(2), sr);
                let rl = coef(p(3), sr);
                let makeup = db_to_lin(p(4));
                let ex = 1.0 - 1.0 / ratio;
                for fr in buf.chunks_exact_mut(2) {
                    let peak = fr[0].abs().max(fr[1].abs());
                    let c = if peak > *env { at } else { rl };
                    *env += (peak - *env) * c;
                    *gain = if *env > thr { (thr / *env).powf(ex) } else { 1.0 };
                    fr[0] *= *gain * makeup;
                    fr[1] *= *gain * makeup;
                }
            }
            (Dsp::Dyn { env, gain }, FilterKind::NoiseGate) => {
                let thr = db_to_lin(p(0).clamp(-80.0, 0.0));
                let at = coef(p(1), sr);
                let rl = coef(p(2), sr);
                for fr in buf.chunks_exact_mut(2) {
                    let peak = fr[0].abs().max(fr[1].abs());
                    let c = if peak > *env { at } else { rl };
                    *env += (peak - *env) * c;
                    let target = if *env > thr { 1.0 } else { 0.0 };
                    *gain += (target - *gain) * if target > *gain { at } else { rl };
                    fr[0] *= *gain;
                    fr[1] *= *gain;
                }
            }
            (Dsp::Noise { rng, pink, phase, prev }, _) => {
                let level = db_to_lin(p(0).clamp(-80.0, 0.0));
                let ty = p(1).round().clamp(0.0, 2.0) as u8;
                let step = std::f32::consts::TAU * p(2).clamp(20.0, sr * 0.45) / sr;
                let n = (buf.len() / 2) as f32;
                let g0 = if prev.is_finite() { *prev } else { level };
                let dg = (level - g0) / n;
                *prev = level;
                for (i, fr) in buf.chunks_exact_mut(2).enumerate() {
                    let g = g0 + dg * i as f32;
                    match ty {
                        0 => {
                            fr[0] += white(rng) * g;
                            fr[1] += white(rng) * g;
                        }
                        1 => {
                            for ch in 0..2 {
                                fr[ch] += pink_tick(&mut pink[ch], white(rng)) * g;
                            }
                        }
                        _ => {
                            let s = phase.sin() * g;
                            fr[0] += s;
                            fr[1] += s;
                            *phase = (*phase + step) % std::f32::consts::TAU;
                        }
                    }
                }
            }
            (Dsp::Gain { prev }, _) => {
                let g1 = db_to_lin(p(0).clamp(-60.0, 24.0));
                let n = (buf.len() / 2) as f32;
                let g0 = if prev.is_finite() { *prev } else { g1 };
                let dg = (g1 - g0) / n;
                *prev = g1;
                for (i, fr) in buf.chunks_exact_mut(2).enumerate() {
                    let g = g0 + dg * i as f32;
                    fr[0] *= g;
                    fr[1] *= g;
                }
            }
            _ => {}
        }
    }
}

/// xorshift32 white noise in [-1, 1).
fn white(rng: &mut u32) -> f32 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 17;
    *rng ^= *rng << 5;
    (*rng >> 8) as f32 / 8_388_608.0 - 1.0
}

/// Paul Kellet's economy pink filter (~ -3 dB/oct over the audio band).
fn pink_tick(s: &mut [f32; 3], w: f32) -> f32 {
    s[0] = 0.99765 * s[0] + w * 0.0990460;
    s[1] = 0.96300 * s[1] + w * 0.2965164;
    s[2] = 0.57000 * s[2] + w * 1.0526913;
    (s[0] + s[1] + s[2] + w * 0.1848) * 0.2
}

// ---------- bus graph ----------

struct Slot {
    id: Id,
    /// Resolved output bus; `id` itself means terminal (Main / a broken cycle's root).
    dest: Id,
    /// Hops to Main — the evaluation order is this, descending.
    depth: u32,
    audible: bool,
    buf: Vec<f32>,
    states: Vec<FilterState>,
    meter: (f32, f32),
}

/// Bus routing + per-bus filter state, rebuilt when the bus set changes.
#[derive(Default)]
pub struct BusGraph {
    slots: Vec<Slot>,
    order: Vec<Id>,
    main: Id,
    have_main: bool,
}

/// Deepest routing chain we follow before declaring a cycle.
const MAX_HOPS: u32 = 64;

impl BusGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuild for `project` (cheap when nothing changed). Cycles fall back to routing into Main.
    pub fn sync(&mut self, project: &Project) {
        let buses = &project.buses;
        self.have_main = !buses.is_empty();
        self.main = buses.first().map(|b| b.id).unwrap_or(0);
        // drop slots for buses that are gone, add slots for new ones (keeping filter state otherwise)
        self.slots.retain(|s| buses.iter().any(|b| b.id == s.id));
        for b in buses {
            if !self.slots.iter().any(|s| s.id == b.id) {
                self.slots.push(Slot {
                    id: b.id,
                    dest: b.id,
                    depth: 0,
                    audible: true,
                    buf: Vec::new(),
                    states: Vec::new(),
                    meter: (0.0, 0.0),
                });
            }
        }
        let any_solo = buses.iter().any(|b| b.solo);
        for b in buses {
            let dest = resolve(buses, b, self.main);
            let depth = hops(buses, b.id, self.main);
            // Main is the master: solo elsewhere must not silence the path everything sums into.
            let audible = !b.muted && (b.id == self.main || !any_solo || b.solo);
            if let Some(s) = self.slots.iter_mut().find(|s| s.id == b.id) {
                s.dest = dest;
                s.depth = depth;
                s.audible = audible;
            }
        }
        let Self { slots, order, .. } = self;
        order.clear();
        order.extend(buses.iter().map(|b| b.id));
        order.sort_by_key(|id| std::cmp::Reverse(slots.iter().find(|s| s.id == *id).map(|s| s.depth).unwrap_or(0)));
    }

    /// Buses in evaluation order (leaves first, Main last).
    pub fn order(&self) -> Vec<Id> {
        self.order.clone()
    }

    /// The same order without the allocation, for the mixer's per-block loop.
    pub fn order_ref(&self) -> &[Id] {
        &self.order
    }

    /// Zero every bus buffer for a new block of `frames` frames.
    pub fn begin(&mut self, frames: usize) {
        for s in &mut self.slots {
            s.buf.clear();
            s.buf.resize(frames * 2, 0.0);
        }
    }

    /// Scratch buffer for a bus (created on demand, zeroed for the block).
    pub fn buffer(&mut self, bus: Id, frames: usize) -> &mut [f32] {
        let i = match self.slots.iter().position(|s| s.id == bus) {
            Some(i) => i,
            None => {
                self.slots.push(Slot {
                    id: bus,
                    dest: bus,
                    depth: 0,
                    audible: true,
                    buf: Vec::new(),
                    states: Vec::new(),
                    meter: (0.0, 0.0),
                });
                self.slots.len() - 1
            }
        };
        let b = &mut self.slots[i].buf;
        if b.len() != frames * 2 {
            b.clear();
            b.resize(frames * 2, 0.0);
        }
        b
    }

    /// Run one bus's filters + gain/pan/mono and add the result into its output bus (or `out` for Main).
    pub fn flush(&mut self, bus: &Bus, t: f64, out: &mut [f32]) {
        let Some(i) = self.slots.iter().position(|s| s.id == bus.id) else { return };
        if !self.slots[i].audible {
            self.slots[i].buf.fill(0.0);
            self.slots[i].meter = (0.0, 0.0);
            return;
        }
        let mut buf = std::mem::take(&mut self.slots[i].buf);
        let dest = self.slots[i].dest;
        {
            let states = &mut self.slots[i].states;
            // keep one state per filter slot; rebuild a slot whose kind changed
            if states.len() > bus.filters.len() {
                states.truncate(bus.filters.len());
            }
            for (j, f) in bus.filters.iter().enumerate() {
                match states.get(j) {
                    Some(s) if s.kind() == f.kind => {}
                    Some(_) => states[j] = FilterState::new(f, SAMPLE_RATE),
                    None => states.push(FilterState::new(f, SAMPLE_RATE)),
                }
                if f.enabled {
                    states[j].process(f, t, &mut buf);
                }
            }
        }

        let g = bus.gain.at(t) as f32;
        let pan = bus.pan.at(t).clamp(-1.0, 1.0) as f32;
        let (gl, gr) = (g * (1.0 - pan).min(1.0), g * (1.0 + pan).min(1.0));
        let mut peak = (0.0f32, 0.0f32);
        for fr in buf.chunks_exact_mut(2) {
            let (mut l, mut r) = (fr[0] * gl, fr[1] * gr);
            if bus.mono {
                let m = (l + r) * 0.5;
                l = m;
                r = m;
            }
            fr[0] = l;
            fr[1] = r;
            peak.0 = peak.0.max(l.abs());
            peak.1 = peak.1.max(r.abs());
        }

        let frames = buf.len() / 2;
        let decay = (-(frames as f32) / (0.25 * SAMPLE_RATE as f32)).exp();
        let m = self.slots[i].meter;
        self.slots[i].meter = ((m.0 * decay).max(peak.0), (m.1 * decay).max(peak.1));

        let terminal = dest == bus.id || !self.have_main;
        if terminal {
            add_into(out, &buf);
        } else if let Some(j) = self.slots.iter().position(|s| s.id == dest) {
            let mut dbuf = std::mem::take(&mut self.slots[j].buf);
            if dbuf.len() < buf.len() {
                dbuf.resize(buf.len(), 0.0);
            }
            add_into(&mut dbuf, &buf);
            self.slots[j].buf = dbuf;
        }
        self.slots[i].buf = buf;
    }

    /// Peak level (L, R) of the last block, for the mixer meters.
    pub fn meter(&self, bus: Id) -> (f32, f32) {
        self.slots.iter().find(|s| s.id == bus).map(|s| s.meter).unwrap_or((0.0, 0.0))
    }
}

fn add_into(dst: &mut [f32], src: &[f32]) {
    for (d, s) in dst.iter_mut().zip(src) {
        *d += *s;
    }
}

/// Where `b` really sends: its own id when terminal (Main, dangling or cyclic output).
fn resolve(buses: &[Bus], b: &Bus, main: Id) -> Id {
    if b.id == main || b.output == 0 || b.output == b.id || !buses.iter().any(|o| o.id == b.output) {
        return if b.id == main { b.id } else { main };
    }
    // walk the chain; anything that does not reach Main within MAX_HOPS is a cycle → Main
    let mut cur = b.output;
    for _ in 0..MAX_HOPS {
        if cur == main {
            return b.output;
        }
        match buses.iter().find(|o| o.id == cur) {
            Some(o) if o.output != 0 && o.output != o.id && buses.iter().any(|x| x.id == o.output) => cur = o.output,
            _ => return b.output, // dangling further down: that bus becomes terminal-into-Main itself
        }
    }
    main
}

/// Hops from `id` to Main along resolved outputs (Main = 0, a broken cycle = 1).
fn hops(buses: &[Bus], id: Id, main: Id) -> u32 {
    let mut cur = id;
    for n in 0..MAX_HOPS {
        if cur == main {
            return n;
        }
        let Some(b) = buses.iter().find(|o| o.id == cur) else { return n + 1 };
        let next = resolve(buses, b, main);
        if next == cur {
            return n + 1;
        }
        cur = next;
    }
    MAX_HOPS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AudioFilter;

    const SR: u32 = SAMPLE_RATE;

    fn filt(kind: FilterKind, params: &[(usize, f64)]) -> AudioFilter {
        let mut f = AudioFilter::new(kind);
        for &(i, v) in params {
            f.params[i].value = v;
        }
        f
    }

    /// n frames of a sine at `hz`, amplitude `amp`, interleaved stereo.
    fn sine(hz: f32, amp: f32, n: usize) -> Vec<f32> {
        let mut v = vec![0.0; n * 2];
        for (i, fr) in v.chunks_exact_mut(2).enumerate() {
            let s = amp * (std::f32::consts::TAU * hz * i as f32 / SR as f32).sin();
            fr[0] = s;
            fr[1] = s;
        }
        v
    }

    fn rms(buf: &[f32]) -> f32 {
        if buf.is_empty() {
            return 0.0;
        }
        (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt()
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0f32, |a, s| a.max(s.abs()))
    }

    /// Steady-state RMS of `hz` through `f` (first half discarded so the IIR has settled).
    fn thru_rms(f: &AudioFilter, hz: f32) -> f32 {
        let mut st = FilterState::new(f, SR);
        let mut buf = sine(hz, 0.5, 9600);
        st.process(f, 0.0, &mut buf);
        rms(&buf[buf.len() / 2..])
    }

    #[test]
    fn low_pass_attenuates_highs() {
        let f = filt(FilterKind::LowPass, &[(0, 500.0), (1, 0.707)]);
        let low = thru_rms(&f, 100.0);
        let high = thru_rms(&f, 10_000.0);
        assert!(low > 0.3, "100 Hz should pass: {low}");
        assert!(high < low * 0.05, "10 kHz {high} vs 100 Hz {low}");
    }

    #[test]
    fn high_pass_attenuates_lows() {
        let f = filt(FilterKind::HighPass, &[(0, 2000.0), (1, 0.707)]);
        let low = thru_rms(&f, 100.0);
        let high = thru_rms(&f, 10_000.0);
        assert!(high > 0.3, "10 kHz should pass: {high}");
        assert!(low < high * 0.05, "100 Hz {low} vs 10 kHz {high}");
    }

    #[test]
    fn eq_boost_raises_that_band() {
        // +12 dB peak at 1 kHz, everything else flat
        let f = filt(FilterKind::Eq, &[(2, 12.0), (3, 1000.0), (4, 1.0)]);
        let flat = AudioFilter::new(FilterKind::Eq);
        let boosted = thru_rms(&f, 1000.0);
        let plain = thru_rms(&flat, 1000.0);
        let ratio = boosted / plain;
        assert!((ratio - 4.0).abs() < 0.4, "+12 dB ≈ ×4, got ×{ratio}");
        // a far-away band is untouched
        let far = thru_rms(&f, 60.0) / thru_rms(&flat, 60.0);
        assert!((far - 1.0).abs() < 0.15, "60 Hz should be flat, got ×{far}");
        // and the analytic response agrees with the measured one
        let db = filter_response_db(&f, 0.0, 1000.0);
        assert!((db - 12.0).abs() < 0.6, "response {db} dB");
        assert!(filter_response_db(&flat, 0.0, 1000.0).abs() < 0.01);
    }

    #[test]
    fn eq_shelves() {
        let low = filt(FilterKind::Eq, &[(0, -12.0), (1, 200.0)]);
        assert!(filter_response_db(&low, 0.0, 40.0) < -10.0);
        assert!(filter_response_db(&low, 0.0, 8000.0).abs() < 0.5);
        let high = filt(FilterKind::Eq, &[(5, 6.0), (6, 4000.0)]);
        assert!(filter_response_db(&high, 0.0, 16000.0) > 5.0);
        assert!(filter_response_db(&high, 0.0, 100.0).abs() < 0.5);
    }

    #[test]
    fn eq_five_bands() {
        let flat = AudioFilter::new(FilterKind::Eq);
        for hz in [20.0, 100.0, 1000.0, 10_000.0, 20_000.0] {
            assert!(filter_response_db(&flat, 0.0, hz).abs() < 0.01, "default EQ must be flat at {hz} Hz");
        }
        // every band boosts around its own default corner (a shelf reaches full gain past the knee)
        for &(band, gi, fi, _) in &EQ_BANDS {
            let f0 = FilterKind::Eq.params()[fi].default as f32;
            let probe = match band {
                Band::LowShelf => f0 * 0.2,
                Band::HighShelf => f0 * 3.0,
                _ => f0,
            };
            let db = filter_response_db(&filt(FilterKind::Eq, &[(gi, 12.0)]), 0.0, probe);
            assert!(db > 9.0, "{band:?} at {probe} Hz: {db} dB");
        }
        // one of the added peaks measures what the analytic curve promises
        let ratio = thru_rms(&filt(FilterKind::Eq, &[(9, 12.0)]), 400.0) / thru_rms(&flat, 400.0);
        assert!((ratio - 4.0).abs() < 0.5, "+12 dB at 400 Hz ≈ ×4, got ×{ratio}");
        // shelves take a Q now (0.707 = the slope-1 shelf they were pinned to): it reshapes the knee
        // — a resonant shelf dips on the far side of the corner — without touching the plateau
        let tight = filt(FilterKind::Eq, &[(0, 12.0), (7, 2.0)]);
        let wide = filt(FilterKind::Eq, &[(0, 12.0), (7, 0.4)]);
        let (t, w) = (filter_response_db(&tight, 0.0, 240.0), filter_response_db(&wide, 0.0, 240.0));
        assert!((t - w).abs() > 2.0, "shelf Q must reshape the knee: {t} vs {w}");
        for f in [&tight, &wide] {
            let plateau = filter_response_db(f, 0.0, 20.0);
            assert!((plateau - 12.0).abs() < 1.0, "the shelf still reaches +12 dB: {plateau}");
        }
        // a project saved by the 3-band EQ is seven params long and still means the same thing
        let mut old = filt(FilterKind::Eq, &[(0, -12.0), (1, 200.0)]);
        old.params.truncate(7);
        assert!(filter_response_db(&old, 0.0, 40.0) < -10.0, "old low shelf");
        assert!(filter_response_db(&old, 0.0, 8000.0).abs() < 0.5, "old EQ flat up top");
    }

    #[test]
    fn gain_db_is_exact() {
        let f = filt(FilterKind::Gain, &[(0, -6.0)]);
        let mut st = FilterState::new(&f, SR);
        let mut buf = vec![1.0f32; 4800 * 2];
        st.process(&f, 0.0, &mut buf);
        // first block ramps from the target (no previous value) → constant
        let want = db_to_lin(-6.0);
        assert!(buf.iter().all(|s| (s - want).abs() < 1e-6), "{} vs {want}", buf[0]);
        // a second block at the same value stays put
        let mut buf2 = vec![1.0f32; 512 * 2];
        st.process(&f, 0.0, &mut buf2);
        assert!((buf2[0] - want).abs() < 1e-6);
        // and a changed value ramps instead of stepping
        let f2 = filt(FilterKind::Gain, &[(0, 0.0)]);
        let mut buf3 = vec![1.0f32; 512 * 2];
        st.process(&f2, 0.0, &mut buf3);
        assert!((buf3[0] - want).abs() < 1e-3, "ramp starts at the old gain: {}", buf3[0]);
        assert!(buf3[buf3.len() - 1] > 0.99, "ramp ends at the new gain: {}", buf3[buf3.len() - 1]);
    }

    #[test]
    fn gate_silences_below_threshold() {
        let f = filt(FilterKind::NoiseGate, &[(0, -45.0), (1, 2.0), (2, 50.0)]);
        // -60 dBFS tone: below the threshold → gated to silence
        let mut st = FilterState::new(&f, SR);
        let mut quiet = sine(440.0, db_to_lin(-60.0), 48000);
        st.process(&f, 0.0, &mut quiet);
        assert!(peak(&quiet[quiet.len() / 2..]) < 1e-6, "{}", peak(&quiet[quiet.len() / 2..]));
        // -6 dBFS tone: above → passes untouched
        let mut st = FilterState::new(&f, SR);
        let mut loud = sine(440.0, db_to_lin(-6.0), 48000);
        st.process(&f, 0.0, &mut loud);
        let p = peak(&loud[loud.len() / 2..]);
        assert!((p - db_to_lin(-6.0)).abs() < 1e-3, "{p}");
    }

    #[test]
    fn compressor_reduces_peaks_by_the_ratio() {
        // 0 dBFS in, threshold -18 dB, ratio 4 → -18 + 18/4 = -13.5 dBFS out.
        // Fast attack / slow release so the peak detector actually settles on the sine's peak.
        let f = filt(FilterKind::Compressor, &[(0, -18.0), (1, 4.0), (2, 1.0), (3, 1000.0), (4, 0.0)]);
        let mut st = FilterState::new(&f, SR);
        let mut buf = sine(440.0, 1.0, 48000);
        st.process(&f, 0.0, &mut buf);
        let out_db = lin_to_db(peak(&buf[buf.len() / 2..]));
        assert!((out_db + 13.5).abs() < 1.0, "{out_db} dB");
        // below the threshold nothing happens
        let mut st = FilterState::new(&f, SR);
        let mut quiet = sine(440.0, db_to_lin(-30.0), 48000);
        st.process(&f, 0.0, &mut quiet);
        let q = lin_to_db(peak(&quiet[quiet.len() / 2..]));
        assert!((q + 30.0).abs() < 0.5, "{q} dB");
        // makeup is exact on top
        let g = filt(FilterKind::Compressor, &[(0, -18.0), (1, 4.0), (2, 1.0), (3, 1000.0), (4, 6.0)]);
        let mut st = FilterState::new(&g, SR);
        let mut buf = sine(440.0, 1.0, 48000);
        st.process(&g, 0.0, &mut buf);
        let with_makeup = lin_to_db(peak(&buf[buf.len() / 2..]));
        assert!((with_makeup - out_db - 6.0).abs() < 0.6, "{with_makeup} vs {out_db}");
    }

    #[test]
    fn echo_delays_by_the_right_offset() {
        // 100 ms delay, no feedback, fully wet → an impulse reappears exactly 4800 frames later
        let f = filt(FilterKind::Echo, &[(0, 100.0), (1, 0.0), (2, 1.0), (3, 0.0)]);
        let mut st = FilterState::new(&f, SR);
        let mut buf = vec![0.0f32; 48000 * 2];
        buf[0] = 1.0;
        buf[1] = 1.0;
        st.process(&f, 0.0, &mut buf);
        assert!(buf[0].abs() < 1e-6, "dry is gone at mix=1: {}", buf[0]);
        assert!((buf[4800 * 2] - 1.0).abs() < 1e-6, "echo at 100 ms: {}", buf[4800 * 2]);
        assert!(buf[4799 * 2].abs() < 1e-6);
        assert!(buf[4801 * 2].abs() < 1e-6);
        // feedback repeats it, quieter
        let f = filt(FilterKind::Echo, &[(0, 100.0), (1, 0.5), (2, 1.0), (3, 0.0)]);
        let mut st = FilterState::new(&f, SR);
        let mut buf = vec![0.0f32; 48000 * 2];
        buf[0] = 1.0;
        st.process(&f, 0.0, &mut buf);
        assert!((buf[9600 * 2] - 0.5).abs() < 1e-6, "second tap: {}", buf[9600 * 2]);
        // ping-pong puts the first echo on the other channel
        let f = filt(FilterKind::Echo, &[(0, 100.0), (1, 0.0), (2, 1.0), (3, 1.0)]);
        let mut st = FilterState::new(&f, SR);
        let mut buf = vec![0.0f32; 48000 * 2];
        buf[0] = 1.0; // left only
        st.process(&f, 0.0, &mut buf);
        assert!(buf[4800 * 2].abs() < 1e-6, "L stays empty: {}", buf[4800 * 2]);
        assert!((buf[4800 * 2 + 1] - 1.0).abs() < 1e-6, "R gets it: {}", buf[4800 * 2 + 1]);
    }

    #[test]
    fn reverb_tail_decays() {
        let f = filt(FilterKind::Reverb, &[(0, 0.8), (1, 0.2), (2, 1.0), (3, 1.0), (4, 0.0)]);
        let mut st = FilterState::new(&f, SR);
        // 50 ms of noise, then 2 s of silence
        let mut buf = sine(500.0, 0.5, 2400);
        buf.resize(2400 * 2 + 48000 * 2 * 2, 0.0);
        st.process(&f, 0.0, &mut buf);
        let tail = |from: usize, to: usize| rms(&buf[from * 2..to * 2]);
        let early = tail(4800, 9600);
        let late = tail(48000, 52800);
        assert!(early > 1e-4, "tail after the input stops: {early}");
        assert!(late < early, "tail decays: {early} → {late}");
        assert!(buf.iter().all(|s| s.is_finite()));
        // pre-delay pushes the onset later
        let f = filt(FilterKind::Reverb, &[(0, 0.8), (1, 0.2), (2, 1.0), (3, 1.0), (4, 100.0)]);
        let mut st = FilterState::new(&f, SR);
        let mut buf = vec![0.0f32; 48000 * 2];
        buf[0] = 1.0;
        buf[1] = 1.0;
        st.process(&f, 0.0, &mut buf);
        assert!(rms(&buf[..2400 * 2]) < 1e-7, "silent before the pre-delay");
        assert!(rms(&buf[4800 * 2..9600 * 2]) > 1e-6, "audible after it");
    }

    #[test]
    fn distortion_clips_and_mixes() {
        let f = filt(FilterKind::Distortion, &[(0, 20.0), (1, 1.0), (2, 1.0)]);
        let mut st = FilterState::new(&f, SR);
        let mut buf = sine(220.0, 0.2, 4800);
        st.process(&f, 0.0, &mut buf);
        // a 0.2 sine driven ×20 saturates: the peak climbs towards full scale
        assert!(peak(&buf[buf.len() / 2..]) > 0.9, "{}", peak(&buf));
        // mix 0 is a bypass
        let f = filt(FilterKind::Distortion, &[(0, 20.0), (1, 1.0), (2, 0.0)]);
        let mut st = FilterState::new(&f, SR);
        let mut buf = sine(220.0, 0.2, 480);
        let orig = buf.clone();
        st.process(&f, 0.0, &mut buf);
        assert!(buf.iter().zip(&orig).all(|(a, b)| (a - b).abs() < 1e-6));
    }

    #[test]
    fn noise_kinds() {
        for (ty, name) in [(0.0, "white"), (1.0, "pink"), (2.0, "tone")] {
            let f = filt(FilterKind::Noise, &[(0, -6.0), (1, ty), (2, 1000.0)]);
            let mut st = FilterState::new(&f, SR);
            let mut buf = vec![0.0f32; 4800 * 2];
            st.process(&f, 0.0, &mut buf);
            let r = rms(&buf);
            assert!(r > 1e-3, "{name} produced nothing");
            assert!(buf.iter().all(|s| s.is_finite() && s.abs() < 4.0), "{name} blew up");
        }
        // the tone sits at the requested frequency: one period of 1 kHz is 48 samples
        let f = filt(FilterKind::Noise, &[(0, 0.0), (1, 2.0), (2, 1000.0)]);
        let mut st = FilterState::new(&f, SR);
        let mut buf = vec![0.0f32; 4800 * 2];
        st.process(&f, 0.0, &mut buf);
        let left: Vec<f32> = buf.chunks_exact(2).map(|f| f[0]).collect();
        let zero_crossings = left.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
        assert!((zero_crossings as i32 - 200).abs() <= 4, "{zero_crossings} crossings in 100 ms");
    }

    #[test]
    fn disabled_and_mismatched_kinds_are_noops() {
        let f = filt(FilterKind::Gain, &[(0, -60.0)]);
        let mut st = FilterState::new(&f, SR);
        let other = AudioFilter::new(FilterKind::LowPass);
        let mut buf = vec![1.0f32; 64];
        st.process(&other, 0.0, &mut buf);
        assert!(buf.iter().all(|s| *s == 1.0), "a kind mismatch must not touch the block");
    }

    // ---------- bus graph ----------

    fn bus_project() -> Project {
        let mut p = Project::new();
        p.main_bus();
        p
    }

    #[test]
    fn order_is_leaves_first_main_last() {
        let mut p = bus_project();
        let main = p.buses[0].id;
        let a = p.add_bus("A");
        let b = p.add_bus("B");
        p.bus_mut(b).unwrap().output = a; // B → A → Main
        let mut g = BusGraph::new();
        g.sync(&p);
        let order = g.order();
        assert_eq!(order.len(), 3);
        assert_eq!(*order.last().unwrap(), main, "Main is last");
        let pos = |id| order.iter().position(|&x| x == id).unwrap();
        assert!(pos(b) < pos(a), "B before A");
    }

    #[test]
    fn cycles_fall_back_to_main() {
        let mut p = bus_project();
        let main = p.buses[0].id;
        let a = p.add_bus("A");
        let b = p.add_bus("B");
        p.bus_mut(a).unwrap().output = b;
        p.bus_mut(b).unwrap().output = a; // A ↔ B
        let mut g = BusGraph::new();
        g.sync(&p);
        assert_eq!(g.order().len(), 3);
        assert_eq!(*g.order().last().unwrap(), main);
        // a signal put on A still reaches the output instead of looping forever
        g.begin(4);
        g.buffer(a, 4).fill(0.5);
        let mut out = vec![0.0f32; 8];
        for id in g.order() {
            let bus = p.bus(id).unwrap().clone();
            g.flush(&bus, 0.0, &mut out);
        }
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-6), "{out:?}");
    }

    #[test]
    fn flush_gain_pan_mono_and_meter() {
        let mut p = bus_project();
        let main = p.buses[0].id;
        let a = p.add_bus("A");
        p.bus_mut(a).unwrap().gain.value = 0.5;
        let mut g = BusGraph::new();
        g.sync(&p);
        let mut out = vec![0.0f32; 8];
        g.begin(4);
        g.buffer(a, 4).fill(1.0);
        for id in g.order() {
            let bus = p.bus(id).unwrap().clone();
            g.flush(&bus, 0.0, &mut out);
        }
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-6), "gain: {out:?}");
        assert!((g.meter(a).0 - 0.5).abs() < 1e-6);
        assert!((g.meter(main).0 - 0.5).abs() < 1e-6);

        // mono folds L and R
        p.bus_mut(a).unwrap().gain.value = 1.0;
        p.bus_mut(a).unwrap().mono = true;
        g.sync(&p);
        g.begin(2);
        let buf = g.buffer(a, 2);
        buf[0] = 1.0;
        buf[1] = 0.0;
        buf[2] = 1.0;
        buf[3] = 0.0;
        let mut out = vec![0.0f32; 4];
        for id in g.order() {
            let bus = p.bus(id).unwrap().clone();
            g.flush(&bus, 0.0, &mut out);
        }
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-6), "mono: {out:?}");

        // pan hard right
        p.bus_mut(a).unwrap().mono = false;
        p.bus_mut(a).unwrap().pan.value = 1.0;
        g.sync(&p);
        g.begin(2);
        g.buffer(a, 2).fill(1.0);
        let mut out = vec![0.0f32; 4];
        for id in g.order() {
            let bus = p.bus(id).unwrap().clone();
            g.flush(&bus, 0.0, &mut out);
        }
        assert!(out[0].abs() < 1e-6 && (out[1] - 1.0).abs() < 1e-6, "pan: {out:?}");
    }

    #[test]
    fn mute_and_solo() {
        let mut p = bus_project();
        let a = p.add_bus("A");
        let b = p.add_bus("B");
        let run = |g: &mut BusGraph, p: &Project| {
            g.begin(2);
            g.buffer(a, 2).fill(1.0);
            g.buffer(b, 2).fill(1.0);
            let mut out = vec![0.0f32; 4];
            for id in g.order() {
                if let Some(bus) = p.bus(id) {
                    let bus = bus.clone();
                    g.flush(&bus, 0.0, &mut out);
                }
            }
            out[0]
        };
        let mut g = BusGraph::new();
        g.sync(&p);
        assert!((run(&mut g, &p) - 2.0).abs() < 1e-6);
        // mute A → only B
        p.bus_mut(a).unwrap().muted = true;
        g.sync(&p);
        assert!((run(&mut g, &p) - 1.0).abs() < 1e-6);
        assert_eq!(g.meter(a), (0.0, 0.0));
        p.bus_mut(a).unwrap().muted = false;
        // solo B → only B, Main still passes
        p.bus_mut(b).unwrap().solo = true;
        g.sync(&p);
        assert!((run(&mut g, &p) - 1.0).abs() < 1e-6, "solo B");
    }

    #[test]
    fn filter_state_survives_removal_and_kind_change() {
        let mut p = bus_project();
        let a = p.add_bus("A");
        p.bus_mut(a).unwrap().filters.push(filt(FilterKind::Gain, &[(0, -6.0)]));
        let mut g = BusGraph::new();
        g.sync(&p);
        let mut out = vec![0.0f32; 4];
        g.begin(2);
        g.buffer(a, 2).fill(1.0);
        for id in g.order() {
            let bus = p.bus(id).unwrap().clone();
            g.flush(&bus, 0.0, &mut out);
        }
        assert!((out[0] - db_to_lin(-6.0)).abs() < 1e-5, "{}", out[0]);
        // swap the filter for another kind in the same slot: the state must be rebuilt, not reused
        p.bus_mut(a).unwrap().filters[0] = filt(FilterKind::LowPass, &[(0, 20000.0), (1, 0.707)]);
        g.sync(&p);
        let mut out = vec![0.0f32; 4];
        g.begin(2);
        g.buffer(a, 2).fill(1.0);
        for id in g.order() {
            let bus = p.bus(id).unwrap().clone();
            g.flush(&bus, 0.0, &mut out);
        }
        assert!(out[0] > 0.5, "low-pass at 20 kHz passes DC: {}", out[0]);
        // a disabled filter is skipped
        p.bus_mut(a).unwrap().filters[0] = filt(FilterKind::Gain, &[(0, -60.0)]);
        p.bus_mut(a).unwrap().filters[0].enabled = false;
        g.sync(&p);
        let mut out = vec![0.0f32; 4];
        g.begin(2);
        g.buffer(a, 2).fill(1.0);
        for id in g.order() {
            let bus = p.bus(id).unwrap().clone();
            g.flush(&bus, 0.0, &mut out);
        }
        assert!((out[0] - 1.0).abs() < 1e-6, "{}", out[0]);
    }

    #[test]
    fn removing_a_bus_drops_its_slot() {
        let mut p = bus_project();
        let a = p.add_bus("A");
        let mut g = BusGraph::new();
        g.sync(&p);
        assert_eq!(g.order().len(), 2);
        p.remove_bus(a);
        g.sync(&p);
        assert_eq!(g.order().len(), 1);
        assert_eq!(g.meter(a), (0.0, 0.0));
    }
}
