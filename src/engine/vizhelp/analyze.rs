//! ---- ws:visualizer-helper ----
//! Analyse Song: a port of `plans/visualizer-helper/hsv_analyze.py` (and the vocal-onset part of
//! `hsv_lyrics.py`) to Rust, so nobody needs Python. Mono signals at `SR` in, a `SongViz` out.
//!
//! Every musical signal comes from a stem when one is given (the hardstyle lesson: the kick and the screech
//! lead are as loud as the voice, so mix-based vocal or kick detection fails). Without stems it falls back
//! to band-passed copies of the mix and says so (`SongViz::stems` = false).

use super::dsp::*;
use crate::model::SongViz;

/// The analysis sample rate.
pub const SR: f64 = 24_000.0;
/// STFT size and hop for the onset envelopes.
const NFFT: usize = 1024;
const HOP: usize = 256;

/// The four Demucs stems (mono, `SR`).
#[derive(Default)]
pub struct Stems {
    pub vocals: Vec<f32>,
    pub drums: Vec<f32>,
    pub bass: Vec<f32>,
    pub other: Vec<f32>,
}

/// The knobs of Analyse Song.
#[derive(Clone, Debug)]
pub struct Opts {
    /// Tempo search range (BPM).
    pub bpm_min: f64,
    pub bpm_max: f64,
    /// Kick gate: a hit needs a 40-150 Hz thump within this many dB of the song's loudest ones.
    pub gate_db: f64,
    /// Drop times the user gave (skip detection).
    pub drops: Vec<f64>,
    /// No drop before this time.
    pub after: f64,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts { bpm_min: 130.0, bpm_max: 175.0, gate_db: 18.0, drops: Vec::new(), after: 0.0 }
    }
}

/// Progress callback: (fraction 0..1, what).
pub type Report<'a> = &'a dyn Fn(f32, &str);

/// Analyse `mix` (+ `stems` when separated). Deterministic: the same input gives the same record.
pub fn analyze(mix: &[f32], stems: Option<&Stems>, o: &Opts, report: Report) -> SongViz {
    let dur = mix.len() as f64 / SR;
    // stand-ins when there are no stems: the voice band, the low end, the mix
    let fallback;
    let st: &Stems = match stems {
        Some(s) => s,
        None => {
            report(0.02, "Splitting the mix");
            fallback = Stems { vocals: bandpass(mix, 250.0, 3500.0, SR), drums: mix.to_vec(), bass: bandpass(mix, 30.0, 160.0, SR), other: bandpass(mix, 1200.0, 9000.0, SR) };
            &fallback
        }
    };
    report(0.05, "Finding the hits");
    let h = hits(mix, o);
    report(0.35, "Finding the drops");
    let (drops, calm) = drops(st, &h, o, dur);
    let fx0 = drops.first().copied().unwrap_or(0.0);
    // the hit strengths: gated, strong onsets back after the first drop, the calm outro barely popping
    let mut out_hits = Vec::with_capacity(h.times.len());
    for i in 0..h.times.len() {
        let t = h.times[i];
        let mut w = r2(h.strength[i] * h.gate[i]);
        if t >= fx0 && w < 0.3 && h.full_on[i] > 0.35 {
            w = w.max(r2((0.3 + 0.5 * (h.full_on[i] - 0.35) / 0.65).min(0.75)));
        }
        if calm.is_some_and(|c| t >= c) {
            w = r2(w * 0.35);
        }
        out_hits.push([t as f32, w as f32]);
    }
    report(0.45, "Finding the screeches");
    let screeches: Vec<f32> = h.screech_times.iter().filter(|&&t| t >= fx0 - 0.05).map(|&t| t as f32).collect();
    let screech = synth_envelope(&st.other, fx0);
    report(0.55, "Following the voice");
    let v = voice(&st.vocals);
    report(0.95, "Tidying up");
    SongViz {
        duration: r3(dur),
        bpm: (h.bpm * 100.0).round() / 100.0,
        stems: stems.is_some(),
        hits: out_hits,
        screeches,
        drops: drops.iter().map(|&d| r3(d) as f32).collect(),
        calm: calm.map(|c| r3(c) as f32),
        vocal_onset: v.first_onset.map(|t| t as f32),
        onsets: v.onsets.iter().map(|&t| t as f32).collect(),
        voiced: v.segments.iter().map(|&(a, b)| [a as f32, b as f32]).collect(),
        vox: curve(&v.vox, 0.01, 0.03, 0.035),
        vib: curve(&v.vib, 0.01, 1.0 / 30.0, 0.09),
        screech: curve(&screech, 0.01, 0.02, 0.08),
        ..Default::default()
    }
}

fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

// ------------------------------------------------------------------ hits

struct Hits {
    bpm: f64,
    /// Hit times and their strengths before the kick gate.
    times: Vec<f64>,
    strength: Vec<f64>,
    /// Kick gate 0..1 (a real low-end thump) and the full-band onset at each hit.
    gate: Vec<f64>,
    full_on: Vec<f64>,
    /// Brightness jumps (screech candidates), before the first-drop filter.
    screech_times: Vec<f64>,
}

/// Per STFT frame: the low (35-160 Hz) and full-band (35-8000 Hz) log-power flux, and the 3-9 kHz /
/// 0.2-3 kHz band powers.
struct Spectral {
    low: Vec<f64>,
    full: Vec<f64>,
    hi: Vec<f64>,
    mid: Vec<f64>,
}

fn spectral(x: &[f32]) -> Spectral {
    // frames centred on their time (scipy's stft pads half a window each side): frame t is at t * HOP
    let mut padded = vec![0f32; x.len() + NFFT];
    padded[NFFT / 2..NFFT / 2 + x.len()].copy_from_slice(x);
    let x = &padded[..];
    let n = if x.len() >= NFFT { (x.len() - NFFT) / HOP + 1 } else { 0 };
    let win: Vec<f32> = (0..NFFT).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / NFFT as f32).cos()).collect();
    let bin = |f: f64| ((f * NFFT as f64 / SR).ceil() as usize).min(NFFT / 2 + 1);
    let (l0, l1, f0, f1) = (bin(35.0), bin(160.0), bin(35.0), bin(8000.0));
    let (h0, h1, m0, m1) = (bin(3000.0), bin(9000.0), bin(200.0), bin(3000.0));
    let mut s = Spectral { low: vec![0.0; n], full: vec![0.0; n], hi: vec![0.0; n], mid: vec![0.0; n] };
    let (mut re, mut im) = (vec![0f32; NFFT], vec![0f32; NFFT]);
    let mut prev = vec![0f64; NFFT / 2 + 1];
    let mut cur = vec![0f64; NFFT / 2 + 1];
    for t in 0..n {
        for i in 0..NFFT {
            re[i] = x[t * HOP + i] * win[i];
            im[i] = 0.0;
        }
        crate::engine::visualizer::fft(&mut re, &mut im);
        for k in 0..=NFFT / 2 {
            let p = (re[k] as f64).powi(2) + (im[k] as f64).powi(2) + 1e-10;
            cur[k] = p;
        }
        let flux = |a: usize, b: usize| -> f64 {
            if t == 0 || b <= a {
                return 0.0;
            }
            (a..b).map(|k| (cur[k].ln() - prev[k].ln()).max(0.0)).sum::<f64>() / (b - a) as f64
        };
        s.low[t] = flux(l0, l1);
        s.full[t] = flux(f0, f1);
        s.hi[t] = cur[h0..h1].iter().sum();
        s.mid[t] = cur[m0..m1].iter().sum();
        std::mem::swap(&mut prev, &mut cur);
    }
    s
}

/// Onset envelope minus its running median (one second), / its 98th percentile.
fn norm(o: &[f64], fps: f64) -> Vec<f64> {
    let med = median_filter(o, fps as usize);
    let d: Vec<f64> = o.iter().zip(&med).map(|(a, m)| (a - m).max(0.0)).collect();
    let pos: Vec<f64> = d.iter().copied().filter(|&v| v > 0.0).collect();
    let top = percentile(&pos, 98.0) + 1e-9;
    d.iter().map(|v| v / top).collect()
}

fn near(o: &[f64], i: usize, w: usize) -> f64 {
    o[i.saturating_sub(w)..(i + w + 1).min(o.len())].iter().fold(0.0, |m, &v| m.max(v))
}

fn hits(mix: &[f32], o: &Opts) -> Hits {
    let fps = SR / HOP as f64;
    let sp = spectral(mix);
    let low = norm(&sp.low, fps);
    let full = norm(&sp.full, fps);
    let env: Vec<f64> = low.iter().zip(&full).map(|(l, f)| 0.7 * l + 0.3 * f).collect();
    let n = env.len();
    let mut out = Hits { bpm: 0.0, times: Vec::new(), strength: Vec::new(), gate: Vec::new(), full_on: Vec::new(), screech_times: Vec::new() };
    if n < 16 {
        return out;
    }
    // tempo: the autocorrelation peak inside the BPM range, refined to a fraction of a frame
    let mean = env.iter().sum::<f64>() / n as f64;
    let c: Vec<f64> = env.iter().map(|v| v - mean).collect();
    let (bmin, bmax) = (o.bpm_min.clamp(40.0, 300.0), o.bpm_max.clamp(41.0, 400.0).max(o.bpm_min + 1.0));
    let lag_hi = ((60.0 * fps / bmin).ceil() as usize + 2).min(n - 1);
    let ac: Vec<f64> = (0..=lag_hi).map(|l| c[..n - l].iter().zip(&c[l..]).map(|(a, b)| a * b).sum()).collect();
    let lag = (1..lag_hi)
        .filter(|&l| {
            let b = 60.0 * fps / l as f64;
            b > bmin && b < bmax
        })
        .max_by(|&a, &b| ac[a].total_cmp(&ac[b]).then(b.cmp(&a)))
        .unwrap_or(((60.0 * fps / ((bmin + bmax) / 2.0)).round() as usize).clamp(2, lag_hi - 1));
    let (y0, y1, y2) = (ac[lag - 1], ac[lag], ac[lag + 1]);
    let den = y0 - 2.0 * y1 + y2;
    let period = if den.abs() > 1e-12 { lag as f64 + (0.5 * (y0 - y2) / den).clamp(-0.5, 0.5) } else { lag as f64 };
    out.bpm = 60.0 * fps / period;
    // Ellis dynamic-programming beat tracker on that tempo
    let mut score = env.clone();
    let mut back = vec![usize::MAX; n];
    for i in 0..n {
        let lo = (i as f64 - 2.0 * period).max(0.0) as usize;
        let hi = (i as f64 - period / 2.0) as isize;
        if hi <= 0 || lo as isize >= hi {
            continue;
        }
        let (mut best, mut bj) = (f64::NEG_INFINITY, lo);
        for j in lo..hi as usize {
            let v = score[j] - 200.0 * ((i - j) as f64 / period).ln().powi(2);
            if v > best {
                (best, bj) = (v, j);
            }
        }
        score[i] = env[i] + best;
        back[i] = bj;
    }
    let tail = (period as usize).max(1).min(n);
    let mut i = (n - tail..n).rev().max_by(|&a, &b| score[a].total_cmp(&score[b]).then(b.cmp(&a))).unwrap_or(n - 1);
    let mut beats = vec![i];
    while back[i] != usize::MAX {
        i = back[i];
        beats.push(i);
    }
    beats.reverse();
    // grid beats with some low end (or a strong full-band hit)
    let mut bi = Vec::new();
    let mut bst = Vec::new();
    for &b in &beats {
        let s = near(&low, b, 3).max(0.5 * near(&full, b, 3));
        if s > 0.06 {
            bi.push(b);
            bst.push(s);
        }
    }
    let bt: Vec<f64> = bi.iter().map(|&b| b as f64 / fps).collect();
    let mut hits: Vec<(f64, f64)> = bt.iter().zip(&bst).map(|(&t, &s)| (t, s.min(1.0))).collect();
    // off-beat stabs half way between beats
    for w in bi.windows(2) {
        let (a, b) = (w[0], w[1]);
        if (b - a) as f64 > 1.6 * period {
            continue;
        }
        let c = (a + b) / 2;
        let s = near(&low, c, 4).max(0.6 * near(&full, c, 4));
        if s > 0.45 {
            hits.push((c as f64 / fps, (0.75 * s).min(1.0)));
        }
    }
    // rolls the grid missed
    for p in find_peaks(&env, 0.28, (0.085 * fps) as usize) {
        hits.push((p as f64 / fps, (0.8 * env[p]).min(1.0)));
    }
    hits.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for h in hits {
        match merged.last_mut() {
            Some(m) if h.0 - m.0 < 0.085 => m.1 = m.1.max(h.1),
            _ => merged.push(h),
        }
    }
    // downbeats: the phase (mod 4) with the most low end
    let best = (0..4usize).max_by(|&a, &b| {
        let s = |ph: usize| bst.iter().skip(ph).step_by(4).sum::<f64>();
        s(a).total_cmp(&s(b)).then(b.cmp(&a))
    });
    let down: Vec<f64> = best.map(|b| bt.iter().skip(b).step_by(4).map(|&t| r3(t)).collect()).unwrap_or_default();
    for &(a, w) in &merged {
        let mut w = 0.25 + 0.75 * w.powf(0.8);
        let g = bt.iter().copied().min_by(|x, y| (x - a).abs().total_cmp(&(y - a).abs()));
        match g {
            Some(g) if (g - a).abs() < 0.04 => w = if down.contains(&r3(g)) { 1.0 } else { w.clamp(0.55, 0.8) },
            _ => w = w.min(0.5),
        }
        out.times.push(r3(a));
        out.strength.push(r2(w));
    }
    // the kick gate: no 40-150 Hz thump in the 60 ms after a hit (a hat, a clap) -> strength 0
    let lowsig = bandpass(mix, 40.0, 150.0, SR);
    let db_after: Vec<f64> = out
        .times
        .iter()
        .map(|&t| {
            let i = ((t * SR) as usize).min(lowsig.len());
            let j = (((t + 0.06) * SR) as usize).min(lowsig.len()).max(i + 1).min(lowsig.len());
            let seg = &lowsig[i.min(j.saturating_sub(1))..j];
            let ms = seg.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / seg.len().max(1) as f64;
            20.0 * (ms.sqrt() + 1e-9).log10()
        })
        .collect();
    let p95 = percentile(&db_after, 95.0);
    for (k, &t) in out.times.iter().enumerate() {
        let f = (t * fps) as usize;
        let win = |o: &[f64]| o[f.saturating_sub(2).min(o.len())..(f + 4).min(o.len())].iter().fold(0.0f64, |m, &v| m.max(v));
        let lvl = ((db_after[k] - p95 + o.gate_db) / 6.0).clamp(0.0, 1.0);
        out.gate.push(lvl * ((win(&low) - 0.2) / 0.15).clamp(0.0, 1.0));
        out.full_on.push(win(&full));
    }
    // screeches: brightness jumps in the mix (3-9 kHz over 0.2-3 kHz, next 120 ms vs last 250 ms)
    let br0: Vec<f64> = sp.hi.iter().zip(&sp.mid).map(|(h, m)| h.ln() - m.ln()).collect();
    let br = uniform(&br0, 3, 0);
    let (w12, w25, w10) = ((0.12 * fps) as usize, (0.25 * fps) as usize, (0.1 * fps) as usize);
    let ahead = uniform(&br, w12, (w12 / 2) as isize);
    let behind = uniform(&br, w25, -((w25 / 2) as isize));
    let jump: Vec<f64> = ahead.iter().zip(&behind).map(|(a, b)| a - b).collect();
    let lhi: Vec<f64> = sp.hi.iter().map(|v| v.ln()).collect();
    let loud = uniform(&lhi, w10, (w10 / 2) as isize);
    let p40 = percentile(&loud, 40.0);
    out.screech_times = find_peaks(&jump, 0.5, (0.15 * fps) as usize).into_iter().filter(|&p| loud[p] > p40).map(|p| r3(p as f64 / fps)).collect();
    out
}

// ------------------------------------------------------------------ drops and the calm part

/// A drop = the bass back after >= 1 s of silence, with kicks in it. The first one counts; a later one only
/// when the 4 s after it are louder (bass + drums) than any earlier section. Calm = the first point after
/// the last drop where the next 8 s are 4 dB under the drop's.
fn drops(st: &Stems, h: &Hits, o: &Opts, dur: f64) -> (Vec<f64>, Option<f64>) {
    let hop = (SR * 0.05) as usize;
    let mut bdb = block_db(&st.bass, hop, 1e-7);
    let mut ddb = block_db(&st.drums, hop, 1e-7);
    let m = bdb.len().min(ddb.len());
    bdb.truncate(m);
    ddb.truncate(m);
    if m < 40 {
        return (o.drops.clone(), None);
    }
    let td = |k: usize| k as f64 * 0.05;
    let pk = percentile(&bdb, 99.0);
    let mut cs = vec![0usize; m + 1];
    for k in 0..m {
        cs[k + 1] = cs[k] + (bdb[k] < pk - 35.0) as usize;
    }
    let quiet = |k: usize| k >= 30 && cs[k - 10] - cs[k - 30] >= 18;
    let energy: Vec<f64> = bdb.iter().zip(&ddb).map(|(b, d)| 10.0 * (10f64.powf(b / 10.0) + 10f64.powf(d / 10.0)).log10()).collect();
    let mean = |a: usize, b: usize| -> f64 {
        let b = b.min(m);
        if b <= a {
            return energy[a.min(m - 1)];
        }
        energy[a..b].iter().sum::<f64>() / (b - a) as f64
    };
    let mut drops: Vec<f64> = Vec::new();
    if !o.drops.is_empty() {
        drops = o.drops.clone();
    } else {
        let mut best = f64::NEG_INFINITY;
        let mut k = 40;
        while k < m {
            if quiet(k) && bdb[k] > pk - 15.0 {
                let e4 = mean(k, k + 80);
                let t = td(k);
                let kicks = h.times.iter().zip(&h.gate).filter(|(&ht, &g)| ht >= t - 0.3 && ht <= t + 4.0 && g > 0.3).count();
                if kicks >= 6 && (drops.is_empty() || e4 > best + 2.0) {
                    let mut k0 = k;
                    while k0 > 0 && bdb[k0 - 1] > pk - 55.0 {
                        k0 -= 1;
                    }
                    let t0 = td(k0) - 0.05;
                    let snap = h.times.iter().copied().filter(|&ht| (ht - t0).abs() <= 0.15).min_by(|a, b| (a - t0).abs().total_cmp(&(b - t0).abs()));
                    drops.push(snap.unwrap_or(t0));
                }
                best = best.max(e4);
                k += 80;
            }
            k += 1;
        }
    }
    drops.retain(|&d| d > 2.0_f64.max(o.after - 1.0));
    let calm = drops.last().map(|&last| {
        let k0 = ((last / 0.05) as usize).min(m - 1);
        let reference = mean(k0, k0 + 160);
        let loud = (0..m).filter(|&k| td(k) > last && mean(k, k + 160) >= reference - 4.0).last();
        let c = loud.map_or(dur, |k| td(k) + 8.0);
        if c > dur - 4.0 {
            dur
        } else {
            c
        }
    });
    (drops, calm)
}

/// The synth stem's 1.5-8 kHz loudness, 0..2.5, 30 ms attack / 250 ms release, every 10 ms; zero before
/// the first drop (the electricity stays off for the clean build).
fn synth_envelope(other: &[f32], fx0: f64) -> Vec<f64> {
    let hs = bandpass(other, 1500.0, 8000.0, SR);
    let se = block_db(&hs, (SR * 0.01) as usize, 1e-7);
    let (lo, hi) = (percentile(&se, 50.0), percentile(&se, 99.5));
    let scr: Vec<f64> = se.iter().map(|v| ((v - lo) / (hi - lo).max(1e-9)).clamp(0.0, 1.0) * 2.5).collect();
    let mut ys = follow(&scr, 0.03, 0.25, 0.01);
    for (k, y) in ys.iter_mut().enumerate() {
        if (k as f64) * 0.01 < fx0 {
            *y = 0.0;
        }
    }
    ys
}

// ------------------------------------------------------------------ the voice

struct Voice {
    /// The vocal envelope and the vibrato, every 10 ms.
    vox: Vec<f64>,
    vib: Vec<f64>,
    onsets: Vec<f64>,
    segments: Vec<(f64, f64)>,
    first_onset: Option<f64>,
}

/// The decimated rate the pitch tracker runs at (the voice is band-passed to 80-1200 Hz first).
const PITCH_SR: f64 = 8000.0;

fn voice(voc: &[f32]) -> Voice {
    let h10 = (SR * 0.01) as usize;
    let ldb = block_db(voc, h10, 1e-7);
    let n = ldb.len();
    let audible: Vec<f64> = ldb.iter().copied().filter(|&v| v > -80.0).collect();
    let (lo, hi) = (percentile(&audible, 55.0), percentile(&ldb, 99.5));
    let lv: Vec<f64> = ldb.iter().map(|v| ((v - lo) / (hi - lo).max(1e-9)).clamp(0.0, 1.0).powf(1.3)).collect();
    let yv = follow(&lv, 0.08, 0.4, 0.01); // held: 80 ms attack, 400 ms release
    let ft = follow(&lv, 0.03, 0.12, 0.01); // talking: fast syllables
    // pitch by autocorrelation on 40 ms windows every 10 ms (confidence > 0.8)
    let band = bandpass(voc, 80.0, 1200.0, SR);
    let dec = (SR / PITCH_SR) as usize;
    let c: Vec<f32> = band.chunks(dec).map(|ch| ch.iter().sum::<f32>() / ch.len() as f32).collect();
    let (win, step) = ((PITCH_SR * 0.04) as usize, (PITCH_SR * 0.01) as usize);
    let (lag0, lag1) = ((PITCH_SR / 1000.0) as usize, (PITCH_SR / 80.0) as usize);
    let mut cents = vec![0.0f64; n];
    let mut ok = vec![false; n];
    let mut s = vec![0f64; win];
    let mut a2 = vec![0f64; lag1 + 1];
    for k in 0..n {
        let i0 = k * step;
        if i0 + win > c.len() || ldb[k] < lo - 12.0 {
            continue;
        }
        let m = c[i0..i0 + win].iter().map(|&v| v as f64).sum::<f64>() / win as f64;
        for (d, &v) in s.iter_mut().zip(&c[i0..i0 + win]) {
            *d = v as f64 - m;
        }
        let e0: f64 = s.iter().map(|v| v * v).sum();
        if e0 < 1e-6 {
            continue;
        }
        for (l, a) in a2.iter_mut().enumerate().take(lag1 + 1).skip(lag0.saturating_sub(1)) {
            *a = s[..win - l].iter().zip(&s[l..]).map(|(x, y)| x * y).sum::<f64>() / e0;
        }
        let j = (lag0..lag1).max_by(|&a, &b| a2[a].total_cmp(&a2[b]).then(b.cmp(&a))).unwrap_or(lag0);
        if a2[j] < 0.8 {
            continue;
        }
        let mut l = j as f64;
        if j > lag0 && j + 1 < lag1 {
            let (q0, q1, q2) = (a2[j - 1], a2[j], a2[j + 1]);
            l += 0.5 * (q0 - q2) / (q0 - 2.0 * q1 + q2 + 1e-9);
        }
        cents[k] = 1200.0 * (PITCH_SR / l / 100.0).log2();
        ok[k] = true;
    }
    // length of each voiced run
    let mut tot = vec![0.0; n];
    let mut k = 0;
    while k < n {
        if ok[k] {
            let mut j = k;
            while j < n && ok[j] {
                j += 1;
            }
            for t in &mut tot[k..j] {
                *t = (j - k) as f64;
            }
            k = j;
        } else {
            k += 1;
        }
    }
    const W: usize = 50;
    let okf: Vec<f64> = ok.iter().map(|&b| b as u8 as f64).collect();
    let vf = uniform(&okf, W, 0);
    let held0: Vec<f64> = tot.iter().zip(&ok).map(|(t, &o)| (t / 25.0).clamp(0.0, 1.0) * o as u8 as f64).collect();
    let held = uniform(&held0, W, 0);
    let ph: Vec<f64> = (0..n)
        .map(|k| {
            let (a, b) = (k.saturating_sub(W / 2), (k + W / 2).min(n));
            let mut v: Vec<f64> = (a..b).filter(|&i| ok[i]).map(|i| cents[i]).collect();
            if v.is_empty() {
                return 0.0;
            }
            v.sort_by(|x, y| x.total_cmp(y));
            if v.len() % 2 == 1 {
                v[v.len() / 2]
            } else {
                0.5 * (v[v.len() / 2 - 1] + v[v.len() / 2])
            }
        })
        .collect();
    let pos: Vec<f64> = ph.iter().copied().filter(|&v| v > 0.0).collect();
    let p20 = percentile(&pos, 20.0);
    let ph_n: Vec<f64> = ph.iter().map(|&v| if pos.is_empty() { 0.0 } else { ((v - p20) / 600.0).clamp(0.0, 1.0) }).collect();
    let lvw = uniform(&lv, W, 0);
    let sing0: Vec<f64> = (0..n)
        .map(|k| {
            let s = (1.6 * held[k] * (0.4 + 0.6 * vf[k])).clamp(0.0, 1.0);
            (0.55 * s + 0.25 * ph_n[k] + 0.3 * lvw[k]).clamp(0.0, 1.0) * ((yv[k] > 0.05) as u8 as f64)
        })
        .collect();
    let sing = uniform(&sing0, 30, 0);
    let s12: Vec<f64> = sing.iter().map(|s| s.clamp(0.0, 1.0).powf(1.2)).collect();
    let vox: Vec<f64> = (0..n).map(|k| (1.0 - s12[k]) * ft[k] * 0.4 + s12[k] * yv[k]).collect();
    // vibrato: pitch minus its 0.31 s median, band-passed 3.5-8.5 Hz, /55 cents
    let note = median_filter(&cents, 31);
    let dv: Vec<f32> = (0..n).map(|k| if ok[k] { (cents[k] - note[k]).clamp(-80.0, 80.0) as f32 } else { 0.0 }).collect();
    let wob = bandpass(&dv, 3.5, 8.5, 100.0);
    let vib: Vec<f64> = (0..n).map(|k| (wob[k] as f64 / 55.0).clamp(-1.0, 1.0) * (yv[k] * 1.5).clamp(0.0, 1.0) * s12[k]).collect();
    // syllable onsets: a 10 ms log-level rise of more than 4 dB over 20 ms
    let mut d2 = vec![0.0; n];
    for k in 2..n {
        d2[k] = (ldb[k] - ldb[k - 2]).max(0.0);
    }
    let voiced: Vec<bool> = yv.iter().map(|&v| v > 0.1).collect();
    // an onset must lead into the voice (a rise inside silence is noise)
    let onsets: Vec<f64> = find_peaks(&d2, 4.0, 6).into_iter().filter(|&p| voiced[p..(p + 15).min(n)].iter().any(|&v| v)).map(|p| r3(p as f64 * 0.01)).collect();
    // voiced stretches: gaps under 0.6 s bridged, at least 0.3 s long
    let mut segments = Vec::new();
    let mut i = 0;
    while i < n {
        if voiced[i] {
            let mut j = i;
            while j < n && (voiced[j] || (j + 60 < n && voiced[j..j + 60].iter().any(|&v| v))) {
                j += 1;
            }
            if j - i >= 30 {
                segments.push((r3(i as f64 * 0.01), r3(j as f64 * 0.01)));
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    // the first sung syllable: the onset nearest the first voiced stretch's start
    let first_onset = segments.first().map(|&(s, _)| onsets.iter().copied().filter(|&o| o >= s - 0.25 && o <= s + 0.25).min_by(|a, b| (a - s).abs().total_cmp(&(b - s).abs())).unwrap_or(s));
    Voice { vox, vib, onsets, segments, first_onset }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic hardstyle-ish song at 150 BPM: a 55 Hz kick with a click on every beat for 20 s, and a
    /// sung 220 Hz voice (with harmonics, a syllable every half second) from 6.0 s.
    pub(crate) fn song() -> (Vec<f32>, Stems, Vec<f64>) {
        let n = (SR * 20.0) as usize;
        let beat = 60.0 / 150.0;
        let mut kick = vec![0f32; n];
        let mut kicks = Vec::new();
        let mut t0 = 0.5;
        while t0 < 19.5 {
            kicks.push(t0);
            let i0 = (t0 * SR) as usize;
            for i in 0..(0.38 * SR) as usize {
                let t = i as f64 / SR;
                let f = 55.0 + 120.0 * (-t * 30.0).exp();
                let env = (-t * 12.0).exp();
                let click = if i < 48 { 0.4 * (1.0 - i as f64 / 48.0) } else { 0.0 };
                kick[i0 + i] += (0.9 * env * (std::f64::consts::TAU * f * t).sin() + click) as f32;
            }
            t0 += beat;
        }
        let mut voc = vec![0f32; n];
        for (i, v) in voc.iter_mut().enumerate() {
            let t = i as f64 / SR;
            if t < 6.0 || t > 18.0 {
                continue;
            }
            let ph = (t - 6.0) % 0.5;
            let env = (ph / 0.02).min(1.0) * if ph > 0.42 { ((0.5 - ph) / 0.08).max(0.0) } else { 1.0 };
            let vib = 0.01 * (std::f64::consts::TAU * 5.5 * t).sin();
            let w = std::f64::consts::TAU * 220.0 * (t + vib / 5.5);
            *v = (0.3 * env * (w.sin() + 0.5 * (2.0 * w).sin() + 0.25 * (3.0 * w).sin())) as f32;
        }
        let mix: Vec<f32> = kick.iter().zip(&voc).map(|(a, b)| a + b).collect();
        let stems = Stems { vocals: voc, drums: kick.clone(), bass: kick, other: vec![0.0; n] };
        (mix, stems, kicks)
    }

    #[test]
    fn finds_the_kicks_and_the_vocal_onset_of_a_synthetic_song() {
        let (mix, stems, kicks) = song();
        let a = analyze(&mix, Some(&stems), &Opts::default(), &|_, _| {});
        assert!((a.bpm - 150.0).abs() < 2.0, "bpm {}", a.bpm);
        let strong: Vec<f64> = a.hits.iter().filter(|h| h[1] >= 0.55).map(|h| h[0] as f64).collect();
        let found = kicks.iter().filter(|&&k| strong.iter().any(|&h| (h - k).abs() < 0.03)).count();
        assert!(found as f64 >= 0.9 * kicks.len() as f64, "found {found} of {} kicks in {:?}", kicks.len(), a.hits);
        // the bar's boom (strength 1) is only ever on a kick
        let stray: Vec<f32> = a.hits.iter().filter(|h| h[1] >= 1.0 && kicks.iter().all(|&k| (h[0] as f64 - k).abs() > 0.06)).map(|h| h[0]).collect();
        assert!(stray.is_empty(), "downbeats off the kicks: {stray:?}");
        let v = a.vocal_onset.expect("a vocal onset") as f64;
        assert!((v - 6.0).abs() < 0.06, "vocal onset at {v}");
        assert!(a.stems);
        // the syllables are onsets too
        let syl = (0..20).map(|i| 6.0 + 0.5 * i as f64).filter(|&s| a.onsets.iter().any(|&o| (o as f64 - s).abs() < 0.05)).count();
        assert!(syl >= 16, "{syl} of 20 syllables found: {:?}", a.onsets);
        assert!(!a.vox.is_empty() && a.vox.iter().any(|p| p[1] > 0.3));
    }

    #[test]
    fn the_same_song_gives_the_same_analysis() {
        let (mix, stems, _) = song();
        let a = analyze(&mix, Some(&stems), &Opts::default(), &|_, _| {});
        let b = analyze(&mix, Some(&stems), &Opts::default(), &|_, _| {});
        assert_eq!(a, b);
    }
}
