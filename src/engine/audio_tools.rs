//! ---- ws:parity-audio ----
//! Shotcut-parity audio helpers that are not filters:
//! - `measure_lufs`: the analysis pass of the two-pass Normalize filter (a clip's integrated loudness,
//!   through the filters ahead of the Normalize in its chain). Runs on a worker thread, never the UI.
//! - `tone_wav`: the tone generator (sine / square / sawtooth / triangle / white / pink noise) as a
//!   48 kHz stereo WAV the app imports like any other media.
//! - `ScopeFeed` / `Scope`: the Audio Scopes pane's data - the audio thread copies every mixed block into
//!   a fixed ring (`SCOPE_FEED`, no allocation), the UI thread takes the newest frames and computes a
//!   log-frequency spectrum, the goniometer (vectorscope) points and the L/R correlation.

use crate::engine::mixer_fx::{FilterState, Lufs};
use crate::media::{AudioSource, SAMPLE_RATE};
use crate::model::{AudioFilter, Clip};
use std::sync::Mutex;

// ---------- two-pass normalize: the analysis pass ----------

/// Longest stretch of a clip the analysis reads (seconds): an hour of 48 kHz audio is ~2 s of work.
const MEASURE_MAX_S: f64 = 3600.0;

/// Integrated loudness (LUFS, BS.1770-shaped like the Mixer's meter) of `clip`'s source window read from
/// `src`, after the clip's enabled filters before index `upto` (the Normalize being analysed). None for
/// silence (nothing above the -70 LUFS gate). `cancel` is polled between blocks.
pub fn measure_lufs(
    src: &mut dyn AudioSource,
    clip: &Clip,
    upto: usize,
    cancel: &dyn Fn() -> bool,
) -> Option<f32> {
    let chain: Vec<&AudioFilter> = clip.audio_filters.iter().take(upto).filter(|f| f.enabled).collect();
    let mut states: Vec<FilterState> = chain.iter().map(|f| FilterState::new(f, SAMPLE_RATE)).collect();
    let speed = clip.speed.max(1e-3);
    let span = (clip.duration * speed).min(MEASURE_MAX_S);
    let frames = (span * SAMPLE_RATE as f64) as usize;
    let mut lufs = Lufs::new();
    let mut buf = vec![0.0f32; 4800 * 2];
    let mut done = 0;
    while done < frames {
        if cancel() {
            return None;
        }
        let n = (frames - done).min(4800);
        let b = &mut buf[..n * 2];
        src.read_at(clip.src_in + done as f64 / SAMPLE_RATE as f64, b);
        // the filters see clip-local time, as in the mixer (a speed change only moves where in the
        // clip a source second lands)
        let lt = done as f64 / SAMPLE_RATE as f64 / speed;
        for (f, st) in chain.iter().zip(states.iter_mut()) {
            st.process(f, lt, b);
        }
        lufs.push_block(b);
        done += n;
    }
    let v = lufs.integrated();
    v.is_finite().then_some(v)
}

// ---------- tone generator ----------

/// The tone generator's waveforms.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wave {
    Sine,
    Square,
    Sawtooth,
    Triangle,
    WhiteNoise,
    PinkNoise,
}

impl Wave {
    pub const ALL: [Wave; 6] = [Wave::Sine, Wave::Square, Wave::Sawtooth, Wave::Triangle, Wave::WhiteNoise, Wave::PinkNoise];
    pub fn name(self) -> &'static str {
        match self {
            Wave::Sine => "Sine",
            Wave::Square => "Square",
            Wave::Sawtooth => "Sawtooth",
            Wave::Triangle => "Triangle",
            Wave::WhiteNoise => "White noise",
            Wave::PinkNoise => "Pink noise",
        }
    }
    /// By name or a short form ("saw", "white", "pink"), ignoring case.
    pub fn parse(s: &str) -> Option<Wave> {
        let s = s.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|w| {
            let n = w.name().to_ascii_lowercase();
            n == s || n.replace(' ', "") == s.replace([' ', '_', '-'], "") || n.starts_with(&s) && s.len() >= 3
        })
    }
    /// Is the frequency meaningful for this wave (not for noise)?
    pub fn pitched(self) -> bool {
        !matches!(self, Wave::WhiteNoise | Wave::PinkNoise)
    }
}

/// Longest tone the generator makes, in seconds.
pub const TONE_MAX_S: f64 = 3600.0;
/// Fade at each end of a generated tone, so it never starts or stops with a click.
const TONE_FADE_S: f64 = 0.005;

/// PolyBLEP correction for a discontinuity at phase 0 (`t` in [0, 1), `dt` = phase step): keeps the
/// square and sawtooth from aliasing into audible whistles at high pitches.
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// `secs` of `wave` at `hz` and `level_db` dBFS (peak), interleaved stereo at `SAMPLE_RATE`.
pub fn tone_samples(wave: Wave, hz: f64, level_db: f64, secs: f64) -> Vec<f32> {
    let sr = SAMPLE_RATE as f64;
    let n = (secs.clamp(0.0, TONE_MAX_S) * sr) as usize;
    let amp = 10f64.powf(level_db.clamp(-80.0, 0.0) / 20.0) as f32;
    let dt = (hz.clamp(1.0, sr * 0.45) / sr) as f32;
    let fade = ((TONE_FADE_S * sr) as usize).min(n / 2).max(1);
    let (mut ph, mut rng, mut pink, mut tri) = (0.0f32, 0x9e37_79b9u32, [0.0f32; 3], 0.0f32);
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        let s = match wave {
            Wave::Sine => (std::f32::consts::TAU * ph).sin(),
            Wave::Square => {
                let sq = if ph < 0.5 { 1.0 } else { -1.0 };
                sq + poly_blep(ph, dt) - poly_blep((ph + 0.5).fract(), dt)
            }
            Wave::Sawtooth => 2.0 * ph - 1.0 - poly_blep(ph, dt),
            Wave::Triangle => {
                // a leaky-integrated band-limited square: no aliasing corners
                let sq = (if ph < 0.5 { 1.0 } else { -1.0 }) + poly_blep(ph, dt) - poly_blep((ph + 0.5).fract(), dt);
                tri = dt * 4.0 * sq + (1.0 - 0.001) * tri;
                tri
            }
            Wave::WhiteNoise => white(&mut rng),
            Wave::PinkNoise => pink_tick(&mut pink, white(&mut rng)),
        };
        ph = (ph + dt).fract();
        let edge = (i.min(n - 1 - i) as f32 / fade as f32).min(1.0);
        let v = (s * amp * edge).clamp(-1.0, 1.0);
        out.push(v);
        out.push(v);
    }
    out
}

fn white(rng: &mut u32) -> f32 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 17;
    *rng ^= *rng << 5;
    (*rng >> 8) as f32 / 8_388_608.0 - 1.0
}

/// Paul Kellet's economy pink filter, scaled to peak near full scale.
fn pink_tick(s: &mut [f32; 3], w: f32) -> f32 {
    s[0] = 0.99765 * s[0] + w * 0.0990460;
    s[1] = 0.96300 * s[1] + w * 0.2965164;
    s[2] = 0.57000 * s[2] + w * 1.0526913;
    ((s[0] + s[1] + s[2] + w * 0.1848) * 0.25).clamp(-1.0, 1.0)
}

/// A 16-bit PCM stereo WAV of interleaved `samples` at `SAMPLE_RATE`.
pub fn wav_stereo(samples: &[f32]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + data as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&2u16.to_le_bytes()); // stereo
    v.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    v.extend_from_slice(&(SAMPLE_RATE * 4).to_le_bytes()); // byte rate
    v.extend_from_slice(&4u16.to_le_bytes()); // block align
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        v.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    v
}

/// The tone generator's output: a WAV file's bytes and a file name describing it ("Tone 1 kHz sine -18 dB").
pub fn tone_wav(wave: Wave, hz: f64, level_db: f64, secs: f64) -> (Vec<u8>, String) {
    let pitch = if !wave.pitched() {
        String::new()
    } else if hz >= 1000.0 {
        format!(" {} kHz", trim_num(hz / 1000.0))
    } else {
        format!(" {} Hz", trim_num(hz))
    };
    let name = format!("Tone{pitch} {} {} dB", wave.name().to_lowercase(), trim_num(level_db));
    (wav_stereo(&tone_samples(wave, hz, level_db, secs)), name)
}

fn trim_num(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

// ---------- spectrum + vectorscope ----------

/// Frames the scope ring holds (~0.17 s): the spectrum's FFT window plus room for the UI to fall behind.
pub const SCOPE_RING: usize = 8192;
/// FFT size of the spectrum (~85 ms at 48 kHz, 11.7 Hz bins).
pub const SCOPE_FFT: usize = 4096;

/// The newest mixed output, copied in by the audio thread after every block it plays.
/// ponytail: a mutex around a fixed ring (like `BusMeterFeed`); the copy is 8 KB per block.
pub struct ScopeFeed {
    q: Mutex<ScopeRing>,
}

struct ScopeRing {
    buf: Vec<f32>,
    pos: usize,
    /// Frames ever written: the UI redraws only when this moves.
    total: u64,
}

pub static SCOPE_FEED: ScopeFeed = ScopeFeed::new();

impl ScopeFeed {
    pub const fn new() -> Self {
        Self { q: Mutex::new(ScopeRing { buf: Vec::new(), pos: 0, total: 0 }) }
    }
    /// One interleaved stereo block (audio thread). Allocates once, on the first block.
    pub fn push(&self, block: &[f32]) {
        let mut g = self.q.lock().unwrap_or_else(|e| e.into_inner());
        if g.buf.len() != SCOPE_RING * 2 {
            g.buf = vec![0.0; SCOPE_RING * 2];
            g.pos = 0;
        }
        for fr in block.chunks_exact(2) {
            let p = g.pos;
            g.buf[p * 2] = fr[0];
            g.buf[p * 2 + 1] = fr[1];
            g.pos = (p + 1) % SCOPE_RING;
        }
        g.total += (block.len() / 2) as u64;
    }
    /// Frames written so far.
    pub fn total(&self) -> u64 {
        self.q.lock().unwrap_or_else(|e| e.into_inner()).total
    }
    /// The newest `frames` frames, oldest first, into `out` (interleaved; zeros before anything played).
    pub fn latest(&self, frames: usize, out: &mut Vec<f32>) {
        let g = self.q.lock().unwrap_or_else(|e| e.into_inner());
        let frames = frames.min(SCOPE_RING);
        out.clear();
        out.resize(frames * 2, 0.0);
        if g.buf.is_empty() {
            return;
        }
        let start = (g.pos + SCOPE_RING - frames) % SCOPE_RING;
        for i in 0..frames {
            let j = (start + i) % SCOPE_RING;
            out[i * 2] = g.buf[j * 2];
            out[i * 2 + 1] = g.buf[j * 2 + 1];
        }
    }
}

/// Bars of the spectrum: third-octave-ish, log-spaced from 20 Hz to 20 kHz.
pub const SPECTRUM_BARS: usize = 31;
/// The spectrum's floor, in dB.
pub const SPECTRUM_FLOOR: f32 = -90.0;

/// What the Audio Scopes pane draws, recomputed when new audio arrives.
#[derive(Clone, Debug)]
pub struct Scope {
    /// dB per bar (`SPECTRUM_BARS`, low to high), with a slow fall so the bars read steadily.
    pub bars: Vec<f32>,
    /// The highest each bar reached recently (falls ~20 dB/s).
    pub peaks: Vec<f32>,
    /// Goniometer points: (side, mid) per frame, -1..1 (mono = a vertical line).
    pub points: Vec<(f32, f32)>,
    /// L/R correlation over the window: +1 mono, 0 wide / unrelated, -1 out of phase.
    pub correlation: f32,
    /// Peak level per channel over the window (linear).
    pub peak: (f32, f32),
}

impl Default for Scope {
    fn default() -> Self {
        Self {
            bars: vec![SPECTRUM_FLOOR; SPECTRUM_BARS],
            peaks: vec![SPECTRUM_FLOOR; SPECTRUM_BARS],
            points: Vec::new(),
            correlation: 0.0,
            peak: (0.0, 0.0),
        }
    }
}

/// Bar `i`'s frequency edges (Hz).
pub fn bar_edges(i: usize) -> (f32, f32) {
    let f = |k: f32| 20.0 * 1000f32.powf(k / SPECTRUM_BARS as f32);
    (f(i as f32), f(i as f32 + 1.0))
}

/// Bar `i`'s centre frequency (Hz).
pub fn bar_hz(i: usize) -> f32 {
    let (a, b) = bar_edges(i);
    (a * b).sqrt()
}

impl Scope {
    /// Recompute from `frames` (interleaved stereo, newest last). `dt` = seconds since the last update,
    /// for the bars' fall. Pure: the UI calls it only when `SCOPE_FEED.total()` moved.
    pub fn update(&mut self, frames: &[f32], dt: f32) {
        let n = frames.len() / 2;
        if n == 0 {
            return;
        }
        // ---- spectrum: Hann-windowed FFT of the mono sum of the newest SCOPE_FFT frames ----
        let w = n.min(SCOPE_FFT);
        let tail = &frames[(n - w) * 2..];
        let mut re = vec![0f32; SCOPE_FFT];
        let mut im = vec![0f32; SCOPE_FFT];
        for (i, fr) in tail.chunks_exact(2).enumerate() {
            let hann = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / w as f32).cos();
            re[i] = (fr[0] + fr[1]) * 0.5 * hann;
        }
        crate::engine::visualizer::fft(&mut re, &mut im);
        let bin_hz = SAMPLE_RATE as f32 / SCOPE_FFT as f32;
        // Hann's coherent gain is 0.5: a full-scale sine reads 0 dB
        let norm = 4.0 / w as f32;
        let fall = 30.0 * dt.clamp(0.0, 1.0);
        for b in 0..SPECTRUM_BARS {
            let (lo, hi) = bar_edges(b);
            let (k0, k1) = (((lo / bin_hz).floor() as usize).max(1), ((hi / bin_hz).ceil() as usize).min(SCOPE_FFT / 2));
            // the loudest bin in the band (a narrow low band still lands on one bin)
            let mut m = 0f32;
            for k in k0..k1.max(k0 + 1) {
                m = m.max((re[k] * re[k] + im[k] * im[k]).sqrt());
            }
            let db = (20.0 * (m * norm).max(1e-9).log10()).max(SPECTRUM_FLOOR);
            self.bars[b] = db.max(self.bars[b] - fall);
            self.peaks[b] = db.max(self.peaks[b] - fall * 0.66);
        }
        // ---- goniometer + correlation over the newest 1024 frames ----
        let g = n.min(1024);
        let recent = &frames[(n - g) * 2..];
        self.points.clear();
        let (mut lr, mut ll, mut rr) = (0f64, 0f64, 0f64);
        let mut peak = (0f32, 0f32);
        for fr in recent.chunks_exact(2) {
            let (l, r) = (fr[0], fr[1]);
            self.points.push((((r - l) * std::f32::consts::FRAC_1_SQRT_2).clamp(-1.0, 1.0), ((l + r) * std::f32::consts::FRAC_1_SQRT_2).clamp(-1.0, 1.0)));
            lr += (l * r) as f64;
            ll += (l * l) as f64;
            rr += (r * r) as f64;
            peak = (peak.0.max(l.abs()), peak.1.max(r.abs()));
        }
        let den = (ll * rr).sqrt();
        self.correlation = if den > 1e-12 { (lr / den).clamp(-1.0, 1.0) as f32 } else { 0.0 };
        self.peak = peak;
    }

    /// Let the bars fall to the floor (playback stopped).
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, amp: f32, n: usize, right: f32) -> Vec<f32> {
        (0..n)
            .flat_map(|i| {
                let s = amp * (std::f32::consts::TAU * hz * i as f32 / SAMPLE_RATE as f32).sin();
                [s, s * right]
            })
            .collect()
    }

    #[test]
    fn spectrum_peaks_at_the_tone() {
        let mut sc = Scope::default();
        sc.update(&sine(1000.0, 1.0, SCOPE_FFT, 1.0), 0.0);
        let best = (0..SPECTRUM_BARS).max_by(|&a, &b| sc.bars[a].total_cmp(&sc.bars[b])).unwrap();
        let (lo, hi) = bar_edges(best);
        assert!(lo <= 1000.0 && 1000.0 < hi, "loudest bar {best} spans {lo}..{hi}");
        assert!(sc.bars[best].abs() < 1.5, "a full-scale sine reads about 0 dB: {}", sc.bars[best]);
        assert!(sc.bars[2] < -40.0, "far below the tone is quiet: {}", sc.bars[2]);
        // silence falls, not jumps
        sc.update(&vec![0.0; SCOPE_FFT * 2], 0.1);
        assert!(sc.bars[best] < 0.0 && sc.bars[best] > -10.0, "{}", sc.bars[best]);
        sc.clear();
        assert!(sc.bars.iter().all(|b| *b == SPECTRUM_FLOOR));
    }

    #[test]
    fn correlation_and_goniometer() {
        let mut sc = Scope::default();
        sc.update(&sine(440.0, 0.5, 2048, 1.0), 0.0);
        assert!(sc.correlation > 0.99, "mono: {}", sc.correlation);
        assert!(sc.points.iter().all(|(x, _)| x.abs() < 1e-5), "mono draws a vertical line");
        sc.update(&sine(440.0, 0.5, 2048, -1.0), 0.0);
        assert!(sc.correlation < -0.99, "out of phase: {}", sc.correlation);
        assert!(sc.points.iter().all(|(_, y)| y.abs() < 1e-5), "out of phase draws a horizontal line");
        assert!((sc.peak.0 - 0.5).abs() < 0.01);
    }

    #[test]
    fn scope_feed_keeps_the_newest_frames() {
        let f = ScopeFeed::new();
        let mut out = Vec::new();
        f.latest(4, &mut out);
        assert_eq!(out, vec![0.0; 8], "nothing played yet");
        let block: Vec<f32> = (0..SCOPE_RING + 10).flat_map(|i| [i as f32, -(i as f32)]).collect();
        f.push(&block);
        assert_eq!(f.total(), (SCOPE_RING + 10) as u64);
        f.latest(3, &mut out);
        let last = (SCOPE_RING + 9) as f32;
        assert_eq!(out, vec![last - 2.0, -(last - 2.0), last - 1.0, -(last - 1.0), last, -last]);
    }

    #[test]
    fn tones_have_the_right_pitch_and_level() {
        for w in Wave::ALL {
            let s = tone_samples(w, 1000.0, -6.0, 0.5);
            assert_eq!(s.len(), 24000 * 2, "{w:?}");
            assert_eq!(s[0], 0.0, "{w:?} starts silent (no click)");
            assert!(s.iter().all(|v| v.abs() <= 1.0));
            let peak = s.iter().fold(0f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.2, "{w:?} is audible: {peak}");
            assert_eq!(Wave::parse(w.name()), Some(w));
        }
        // a sine at -6 dBFS peaks at 0.5 and crosses zero 2000 times a second
        let s = tone_samples(Wave::Sine, 1000.0, -6.0206, 1.0);
        let peak = s.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.5).abs() < 0.002, "{peak}");
        let left: Vec<f32> = s.iter().step_by(2).copied().collect();
        let crossings = left.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        assert!((1995..=2001).contains(&crossings), "{crossings}");
        assert_eq!(Wave::parse("saw"), Some(Wave::Sawtooth));
        assert_eq!(Wave::parse("white"), Some(Wave::WhiteNoise));
        assert_eq!(Wave::parse("nope"), None);
    }

    #[test]
    fn tone_wav_header_and_name() {
        let (bytes, name) = tone_wav(Wave::Sine, 1000.0, -18.0, 0.25);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2, "stereo");
        assert_eq!(u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]), SAMPLE_RATE);
        assert_eq!(bytes.len(), 44 + 12000 * 4);
        assert_eq!(name, "Tone 1 kHz sine -18 dB");
        assert_eq!(tone_wav(Wave::PinkNoise, 1000.0, -20.0, 0.1).1, "Tone pink noise -20 dB");
        assert_eq!(tone_wav(Wave::Square, 440.0, -12.5, 0.1).1, "Tone 440 Hz square -12.5 dB");
    }

    /// A steady source for `measure_lufs`.
    struct Tone(f32);
    impl AudioSource for Tone {
        fn duration(&self) -> f64 {
            60.0
        }
        fn read_at(&mut self, t: f64, out: &mut [f32]) {
            for (i, fr) in out.chunks_exact_mut(2).enumerate() {
                let tt = t + i as f64 / SAMPLE_RATE as f64;
                let s = self.0 * (std::f64::consts::TAU * 997.0 * tt).sin() as f32;
                fr[0] = s;
                fr[1] = s;
            }
        }
    }

    #[test]
    fn measure_lufs_reads_the_clip_through_earlier_filters() {
        use crate::model::{ClipKind, FilterKind};
        let mut clip = Clip::new(1, ClipKind::Audio, "a", 0.0, 4.0);
        let base = measure_lufs(&mut Tone(0.1), &clip, 0, &|| false).unwrap();
        // a -20 dBFS sine: about -23 LUFS (K-weighting adds ~+0.7 dB at 1 kHz, stereo sums +3 dB)
        assert!((base + 20.0).abs() < 1.5, "{base}");
        let mut g = AudioFilter::new(FilterKind::Gain);
        g.params[0].value = -10.0;
        clip.audio_filters = vec![g, AudioFilter::new(FilterKind::Normalize)];
        let after = measure_lufs(&mut Tone(0.1), &clip, 1, &|| false).unwrap();
        assert!((after - (base - 10.0)).abs() < 0.3, "the gain before the Normalize counts: {after} vs {base}");
        assert_eq!(measure_lufs(&mut Tone(0.0), &clip, 1, &|| false), None, "silence has no loudness");
        assert_eq!(measure_lufs(&mut Tone(0.1), &clip, 1, &|| true), None, "cancelled");
    }
}
