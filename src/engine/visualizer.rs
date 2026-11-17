//! ---- ws:audio-visualizer ----
//! Audio visualizer styles: any shader effect whose `// @param` header declares the audio inputs
//! `level` and/or `band1`..`band6` (`AUDIO_PARAMS`). The built-ins are compiled in (`BUILTIN`); a
//! custom one is a library effect the user saved (`library.save` / `%APPDATA%\SimpleEditor\library
//! \effects`) that declares any of those params. The audio reaches the shader as keyframes baked once
//! when the visualizer is made or restyled (`analyze` + `bake`) - playback and export read plain
//! keyframed params, no audio analysis on the playback path.

use crate::media::{AudioSource, Backend, SAMPLE_RATE};
use crate::model::{Animated, ClipKind, Effect, Id, Keyframe, Project, TrackKind};

/// The audio inputs a visualizer shader may declare: overall loudness, then six log-spaced bands
/// (about 60 Hz to 8 kHz). Each is baked as 0..1 mapped onto the param's own range.
pub const AUDIO_PARAMS: [&str; 7] = ["level", "band1", "band2", "band3", "band4", "band5", "band6"];
pub const BANDS: usize = 6;
/// Beat inputs: `beat` = phase 0..1 from one detected beat to the next (a sawtooth, 0 on the hit),
/// `beats` = how many beats have passed (0 before the first, then 1, 2, ...), `bpm` = the song's tempo
/// (constant). Baked raw (not mapped), clamped to the param's range: declare `beat 0 0 1`,
/// `beats 0 0 100000`, `bpm 120 0 300`.
pub const BEAT_PARAMS: [&str; 3] = ["beat", "beats", "bpm"];

/// (name, source) of the built-in styles, the first is the default. "Spectrum Bars" is the Source
/// monitor's original noa-style bar field, ported to a shader.
pub const BUILTIN: [(&str, &str); 3] = [
    ("Neon Orb", include_str!("../../library/effects/neon-orb.glsl")),
    ("Spectrum Bars", include_str!("../../library/effects/spectrum-bars.glsl")),
    ("Voxel Blocks", include_str!("../../library/effects/voxel-blocks.glsl")),
];

/// One pickable style.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub name: String,
    pub src: String,
    /// From the user's library rather than compiled in.
    pub custom: bool,
}

/// Does this shader read audio (declares one of `AUDIO_PARAMS`)?
pub fn is_visualizer(src: &str) -> bool {
    crate::engine::shaderlib::parse_meta(src)
        .params
        .iter()
        .any(|p| AUDIO_PARAMS.contains(&p.id.as_str()) || BEAT_PARAMS.contains(&p.id.as_str()))
}

/// The built-ins, then every installed library effect that is a visualizer (by its `@name`, else id).
/// Reads the library folder: call it when a picker opens, not every frame.
pub fn styles() -> Vec<Style> {
    styles_from(
        crate::engine::library::installed(crate::engine::library::Kind::Effect)
            .into_iter()
            .filter_map(|it| Some((it.id, std::fs::read_to_string(&it.path).ok()?))),
    )
}

/// The built-ins, then each `(id, source)` that is a visualizer, named by its `@name` (else the id).
pub fn styles_from(custom: impl IntoIterator<Item = (String, String)>) -> Vec<Style> {
    let mut out: Vec<Style> =
        BUILTIN.iter().map(|&(n, s)| Style { name: n.into(), src: s.into(), custom: false }).collect();
    for (id, src) in custom {
        if !is_visualizer(&src) {
            continue;
        }
        let name = crate::engine::shaderlib::parse_meta(&src).name;
        let name = if name.is_empty() { id } else { name };
        // an installed copy of a built-in (the online library has Neon Orb) is the same style
        if !out.iter().any(|s| s.name == name) {
            out.push(Style { name, src, custom: true });
        }
    }
    out
}

/// Find a style by name (case-insensitive), default = the first built-in.
pub fn find(name: Option<&str>) -> Result<Style, String> {
    find_in(styles(), name)
}

fn find_in(all: Vec<Style>, name: Option<&str>) -> Result<Style, String> {
    match name {
        None => Ok(all[0].clone()),
        Some(n) => all.iter().find(|s| s.name.eq_ignore_ascii_case(n.trim())).cloned().ok_or_else(|| {
            format!("no visualizer style '{n}' (have: {})", all.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "))
        }),
    }
}

/// The last-used style (`Settings::visualizer_style`), falling back to the default when it is unset or
/// no longer installed.
pub fn last_used(name: &str) -> Style {
    find((!name.is_empty()).then_some(name)).or_else(|_| find(None)).expect("a built-in style")
}

/// The style name of a visualizer effect: its shader's `@name`.
pub fn style_of(e: &Effect) -> Option<String> {
    (e.kind == crate::model::EffectKind::Shader && is_visualizer(&e.shader)).then(|| e.name())
}

/// One analysis frame: `[level, band1..band6]`, each 0..1.
pub type Frame = [f32; 1 + BANDS];

/// Samples per FFT window (~43 ms at 48 kHz, 23 Hz bins: Sub and Bass come apart).
const FFT_N: usize = 2048;

/// In-place radix-2 FFT (`re.len()` a power of two).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f32::consts::TAU / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (ang * k as f32).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                (re[b], im[b]) = (re[a] - tr, im[a] - ti);
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// Raw (un-normalised) [peak, band energies] of mono `x`: a Hann-windowed FFT of the last `FFT_N`
/// samples, each band the root of the summed power of the bins inside its edges (`AudioMix::BANDS`).
pub fn raw_frame(x: &[f32]) -> Frame {
    let peak = x.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
    let w = &x[x.len().saturating_sub(FFT_N)..];
    let mut re = vec![0f32; FFT_N];
    let mut im = vec![0f32; FFT_N];
    let n = w.len().max(1) as f32;
    for (i, &s) in w.iter().enumerate() {
        re[i] = s * (0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n).cos());
    }
    fft(&mut re, &mut im);
    let df = SAMPLE_RATE as f32 / FFT_N as f32;
    let mut out = [0.0; 1 + BANDS];
    out[0] = peak;
    for (b, &(_, lo, hi)) in crate::model::AudioMix::BANDS.iter().enumerate() {
        let (k0, k1) = ((lo / df).ceil() as usize, ((hi / df).ceil() as usize).min(FFT_N / 2));
        let p: f32 = (k0.max(1)..k1).map(|k| re[k] * re[k] + im[k] * im[k]).sum();
        out[1 + b] = p.sqrt() / n;
    }
    out
}

/// Mix raw frames (`[peak, band energies]` at `hz`) into what a visualizer shows, `[level,
/// band1..band6]` 0..1: each band normalised to its loudest moment in the song (floored at a tenth of
/// the loudest band, so a band with next to nothing in it stays near 0 instead of being blown up) and
/// square-rooted; `bandN` = gain x sensitivity x that; `level` = the gain-weighted sum of the bands'
/// energies, normalised to its own loudest moment, x sensitivity (all gains 1 = overall loudness); then
/// attack / release smoothing. Deterministic.
pub fn shape_mix(raw: &[Frame], hz: f64, m: &crate::model::AudioMix) -> Vec<Frame> {
    let m = m.clone().clamped();
    let mut top = [1e-6f32; BANDS];
    for f in raw {
        for b in 0..BANDS {
            top[b] = top[b].max(f[1 + b]);
        }
    }
    let floor = top.iter().fold(0.0f32, |a, &v| a.max(v)) * 0.1;
    let norm = |f: &Frame, b: usize| (f[1 + b] / top[b].max(floor)).clamp(0.0, 1.0).sqrt();
    // the level hears the bands' energies as they are (a quiet band adds little), weighted by the gains
    let mix = |f: &Frame| (0..BANDS).map(|b| m.gains[b] * f[1 + b]).sum::<f32>();
    let ltop = raw.iter().map(mix).fold(1e-6f32, f32::max);
    let dt = 1.0 / hz as f32;
    let rise = if m.attack > 0.0 { 1.0 - (-dt / m.attack).exp() } else { 1.0 };
    let fall = 1.0 - (-dt / m.release).exp();
    let mut env = [0.0f32; 1 + BANDS];
    raw.iter()
        .map(|f| {
            let mut target = [0.0f32; 1 + BANDS];
            target[0] = m.sensitivity * mix(f) / ltop;
            for b in 0..BANDS {
                target[1 + b] = m.gains[b] * m.sensitivity * norm(f, b);
            }
            for c in 0..1 + BANDS {
                let t = target[c].clamp(0.0, 1.0);
                env[c] += (t - env[c]) * if t > env[c] { rise } else { fall };
            }
            env
        })
        .collect()
}

/// The beat onsets (source seconds) the beat detection hears through `m`'s beat bands (the whole
/// signal's peaks when none are picked): `analysis::onsets`, Detect Beats' own picker.
fn beats_mix(raw: &[Frame], hz: f64, m: &crate::model::AudioMix) -> Vec<f64> {
    let env: Vec<f32> = if m.beat_bands.iter().any(|&b| b) {
        raw.iter().map(|f| (0..BANDS).filter(|&b| m.beat_bands[b]).map(|b| f[1 + b]).sum()).collect()
    } else {
        raw.iter().map(|f| f[0]).collect()
    };
    debug_assert!((hz - crate::media::waveform::PEAKS_PER_SEC as f64).abs() < 1e-9, "onsets reads 100 Hz peaks");
    let peaks = crate::media::waveform::Peaks { min: env.iter().map(|v| -v).collect(), max: env };
    let dur = raw.len() as f64 / hz;
    crate::engine::analysis::onsets(&peaks, 0.0, dur, BEAT_REFRACTORY, BEAT_SENSITIVITY)
}

/// What `analyze` found over a span: one `Frame` per key, and the beats (clip-local seconds) + tempo.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Analysis {
    pub frames: Vec<Frame>,
    pub beats: Vec<f64>,
    pub bpm: Option<f64>,
}

/// Frames a second of the whole-song analysis (`SongAnalysis`).
pub const ANALYSIS_HZ: f64 = 100.0;

/// One song file analysed once, start to end: `[level, band1..band6]` shaped (normalised over the whole
/// song, ~0.15 s release) at `ANALYSIS_HZ`, plus its beats (source seconds). Everything that draws a
/// visualizer reads this at a SOURCE TIME - the timeline bake (`rebake`), the Source monitor
/// (`at`) and so the export - so a visualized frame is a pure function of (song, source time): the
/// same after a seek, a stall or a buffering pause, in preview and in export.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SongAnalysis {
    /// Shaped through the default `AudioMix` (what the Source monitor shows).
    pub frames: Vec<Frame>,
    /// Beat onsets through the default mix, source seconds.
    pub beats: Vec<f64>,
    /// `[peak, band energies]` before any mixing, at `ANALYSIS_HZ`: what a band mixer re-mixes.
    pub raw: Vec<Frame>,
}

impl SongAnalysis {
    /// Decode `src` start to end once (sequential reads) and analyse it.
    // ponytail: naive per-band DFT at 100 Hz, ~0.3 s per minute of song in a release build; an FFT when
    // the band mixer needs real band edges.
    pub fn compute(src: &mut dyn AudioSource) -> SongAnalysis {
        let dur = src.duration().max(0.0);
        let hop = (SAMPLE_RATE as f64 / ANALYSIS_HZ).round() as usize; // = one waveform peak bucket
        let n = (dur * ANALYSIS_HZ).ceil() as usize + 1;
        let mut stereo = vec![0f32; hop * 2];
        let mut hist: Vec<f32> = vec![0.0; FFT_N];
        let mut peaks = crate::media::waveform::Peaks { min: Vec::with_capacity(n), max: Vec::with_capacity(n) };
        let raw: Vec<Frame> = (0..n)
            .map(|i| {
                src.read_at(i as f64 * hop as f64 / SAMPLE_RATE as f64, &mut stereo);
                let mono: Vec<f32> = stereo.chunks_exact(2).map(|c| 0.5 * (c[0] + c[1])).collect();
                peaks.min.push(mono.iter().fold(0.0f32, |m, &v| m.min(v)));
                peaks.max.push(mono.iter().fold(0.0f32, |m, &v| m.max(v)));
                hist.drain(..mono.len().min(hist.len()));
                hist.extend_from_slice(&mono);
                let mut f = raw_frame(&hist[hist.len() - FFT_N..]);
                f[0] = mono.iter().fold(0.0f32, |m, &s| m.max(s.abs())); // the peak of this hop only
                f
            })
            .collect();
        let _ = peaks; // ponytail: the peak column of `raw` is the envelope `beats_mix` reads
        Self::from_raw(raw)
    }

    /// The analysis of already-computed raw frames (default mix).
    pub fn from_raw(raw: Vec<Frame>) -> SongAnalysis {
        let m = crate::model::AudioMix::default();
        SongAnalysis { frames: shape_mix(&raw, ANALYSIS_HZ, &m), beats: beats_mix(&raw, ANALYSIS_HZ, &m), raw }
    }

    /// This song heard through `m` (`m` = default: `self` itself, no work).
    pub fn mixed(&self, m: &crate::model::AudioMix) -> SongAnalysis {
        if *m == crate::model::AudioMix::default() {
            return self.clone();
        }
        SongAnalysis { frames: shape_mix(&self.raw, ANALYSIS_HZ, m), beats: beats_mix(&self.raw, ANALYSIS_HZ, m), raw: Vec::new() }
    }

    /// Each band's average energy over source seconds [from, to], 0..1 of the loudest band: the band
    /// mixer's spectrum graph.
    pub fn spectrum(&self, from: f64, to: f64) -> [f32; BANDS] {
        let (i0, i1) = ((from * ANALYSIS_HZ).max(0.0) as usize, ((to * ANALYSIS_HZ).max(0.0) as usize).min(self.raw.len()));
        let mut sum = [0.0f32; BANDS];
        for f in self.raw.get(i0..i1).unwrap_or(&[]) {
            for b in 0..BANDS {
                sum[b] += f[1 + b];
            }
        }
        let top = sum.iter().fold(1e-9f32, |a, &v| a.max(v));
        sum.map(|v| v / top)
    }

    /// The frame at source second `t` (the loudest over [t, t + span) so a sparse reader never steps
    /// over a hit); silence outside the song.
    pub fn at_span(&self, t: f64, span: f64) -> Frame {
        let i0 = (t * ANALYSIS_HZ).floor();
        if !i0.is_finite() || i0 < 0.0 || i0 as usize >= self.frames.len() {
            return [0.0; 1 + BANDS];
        }
        let i0 = i0 as usize;
        let i1 = ((t + span.max(0.0)) * ANALYSIS_HZ).ceil().max(i0 as f64 + 1.0) as usize;
        let mut out = [0.0f32; 1 + BANDS];
        for f in &self.frames[i0..i1.min(self.frames.len())] {
            for (o, &v) in out.iter_mut().zip(f) {
                *o = o.max(v);
            }
        }
        out
    }

    /// The frame shown at source second `t`: what one 1/`FPS` timeline frame there bakes to.
    pub fn at(&self, t: f64) -> Frame {
        self.at_span(t, 1.0 / FPS)
    }

    /// The beat inputs at source second `t`, exactly as `bake` keys them for a clip starting at the song's
    /// start: (`beat` phase 0..1 since the last beat, `beats` passed, `bpm`).
    pub fn beat_at(&self, t: f64) -> (f64, f64, Option<f64>) {
        let bpm = crate::engine::analysis::bpm(&self.beats);
        let n = self.beats.partition_point(|&b| b <= t);
        if n == 0 {
            return (0.0, 0.0, bpm);
        }
        let last = self.beats[n - 1];
        let next = self.beats.get(n).copied().unwrap_or(last + bpm.map_or(0.5, |b| 60.0 / b));
        (((t - last) / (next - last).max(1e-6)).clamp(0.0, 1.0), n as f64, bpm)
    }

    /// The span a clip plays: `len` timeline seconds from source second `from` at `speed`, `fps` keys a
    /// second, beats in clip-local seconds.
    pub fn span(&self, from: f64, len: f64, speed: f64, fps: f64) -> Analysis {
        let speed = if speed.is_finite() && speed > 0.0 { speed } else { 1.0 };
        let n = (len * fps).floor() as usize + 1;
        let frames = (0..n).map(|i| self.at_span(from + i as f64 / fps * speed, speed / fps)).collect();
        let end = from + len * speed;
        let beats: Vec<f64> = self.beats.iter().filter(|&&b| b >= from && b <= end).map(|b| (b - from) / speed).collect();
        let bpm = crate::engine::analysis::bpm(&beats);
        Analysis { frames, beats, bpm }
    }
}

/// path + stream -> its analysis (None while a background compute runs).
type Cache = std::collections::HashMap<(String, usize), Option<std::sync::Arc<SongAnalysis>>>;
static SONGS: std::sync::Mutex<Option<Cache>> = std::sync::Mutex::new(None);
/// Songs kept analysed (a minute of song is ~170 KB).
const SONGS_KEPT: usize = 24;

fn songs() -> std::sync::MutexGuard<'static, Option<Cache>> {
    SONGS.lock().unwrap_or_else(|e| e.into_inner())
}

fn remember(key: (String, usize), a: Option<std::sync::Arc<SongAnalysis>>) {
    let mut g = songs();
    let m = g.get_or_insert_with(Default::default);
    if m.len() >= SONGS_KEPT && !m.contains_key(&key) {
        m.retain(|_, v| v.is_none()); // ponytail: drop the finished ones wholesale; an LRU if it ever thrashes
    }
    m.insert(key, a);
}

/// The song's analysis, computed on this thread when not cached yet (an edit: Add / restyle / re-bake).
pub fn analysis(path: &str, stream: usize, backend: Backend) -> Result<std::sync::Arc<SongAnalysis>, String> {
    let key = (path.to_string(), stream);
    if let Some(Some(a)) = songs().as_ref().and_then(|m| m.get(&key)) {
        return Ok(a.clone());
    }
    let mut src = crate::media::open_audio(path, stream, backend)?;
    let a = std::sync::Arc::new(SongAnalysis::compute(&mut *src));
    remember(key, Some(a.clone()));
    Ok(a)
}

/// The song's analysis if ready, else start it on a background thread (once) and answer None; `done`
/// runs when it lands (a repaint). For the Source monitor: never blocks the UI thread.
pub fn analysis_async(path: &str, stream: usize, backend: Backend, done: impl FnOnce() + Send + 'static) -> Option<std::sync::Arc<SongAnalysis>> {
    let key = (path.to_string(), stream);
    {
        let mut g = songs();
        let m = g.get_or_insert_with(Default::default);
        match m.get(&key) {
            Some(Some(a)) => return Some(a.clone()),
            Some(None) => return None,
            None => {
                m.insert(key.clone(), None);
            }
        }
    }
    std::thread::spawn(move || {
        let a = crate::media::open_audio(&key.0, key.1, backend).map(|mut s| std::sync::Arc::new(SongAnalysis::compute(&mut *s)));
        // a file that will not open still resolves (to silence), never stays pending
        remember(key, Some(a.unwrap_or_default()));
        done();
    });
    None
}

/// Analyse `len` seconds of `src` from source second `from` at `speed`, `fps` frames a second (the whole
/// source is analysed, then the span read): see `SongAnalysis`.
pub fn analyze(src: &mut dyn AudioSource, from: f64, len: f64, speed: f64, fps: f64) -> Analysis {
    SongAnalysis::compute(src).span(from, len, speed, fps)
}

/// Detect Beats' defaults (`Settings::beat_thr`, `Action::DetectBeats`' refractory window).
const BEAT_SENSITIVITY: f32 = 1.6;
const BEAT_REFRACTORY: f64 = 0.25;

/// Write `an` as keys on every audio / beat param `e` declares: the frames (one per `1/fps` s from
/// clip-local 0) mapped onto each param's range, the beats as an exact sawtooth / step at the beat
/// times, the tempo as a constant. Returns how many params got baked.
pub fn bake(e: &mut Effect, an: &Analysis, fps: f64) -> usize {
    let specs = e.specs();
    let meta = crate::engine::shaderlib::parse_meta(&e.shader);
    let mut n = 0;
    // the interval after beat i: to the next one, else one tempo period (half a second without one)
    let period = an.bpm.map_or(0.5, |b| 60.0 / b);
    let next = |i: usize| an.beats.get(i + 1).copied().unwrap_or(an.beats[i] + period);
    for (pi, p) in meta.params.iter().enumerate() {
        while e.params.len() <= pi {
            e.params.push(Animated::new(specs[e.params.len()].default));
        }
        let key = |t: f64, v: f64| Keyframe { t, v: v.clamp(p.min, p.max), ease: Default::default() };
        let keys: Vec<Keyframe> = if let Some(c) = AUDIO_PARAMS.iter().position(|a| *a == p.id) {
            an.frames.iter().enumerate().map(|(i, f)| key(i as f64 / fps, p.min + (p.max - p.min) * f[c] as f64)).collect()
        } else if p.id == "beat" || p.id == "beats" {
            let count = p.id == "beats";
            let mut k = Vec::new();
            if an.beats.first().is_some_and(|&b| b > 1e-3) {
                k.push(key(0.0, 0.0));
                k.push(key(an.beats[0] - 1e-3, 0.0));
            }
            for (i, &b) in an.beats.iter().enumerate() {
                let end = (next(i) - 1e-3).max(b + 1e-4);
                let v = if count { (i + 1) as f64 } else { 0.0 };
                k.push(key(b, v));
                k.push(key(end, if count { v } else { 1.0 }));
            }
            k
        } else if p.id == "bpm" {
            e.params[pi] = Animated::new(an.bpm.unwrap_or(p.default).clamp(p.min, p.max));
            n += 1;
            continue;
        } else {
            continue;
        };
        e.params[pi].keys = keys;
        n += 1;
    }
    n
}

/// The audio clip a visualizer layer listens to: its linked song, else the audio-track clip
/// overlapping it the most.
pub fn source_of(p: &Project, vis: Id) -> Option<Id> {
    // the song it is linked to, when it is (Add with Audio Visualizer / Add Audio Visualizer link them)
    let linked = p.linked(vis).into_iter().find(|&id| id != vis && p.clip(id).is_some_and(|c| c.kind == ClipKind::Audio));
    if linked.is_some() {
        return linked;
    }
    let v = p.clip(vis)?;
    let (a, b) = (v.start, v.start + v.duration);
    p.tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio)
        .flat_map(|t| &t.clips)
        .map(|c| (c.id, (b.min(c.start + c.duration) - a.max(c.start)).max(0.0)))
        .filter(|&(_, o)| o > 1e-6)
        .max_by(|x, y| x.1.total_cmp(&y.1))
        .map(|(id, _)| id)
}

/// Keys per second the audio is baked at.
pub const FPS: f64 = 30.0;

/// Analyse `audio`'s sound over `vis`'s span (decoded through `backend`) and bake it into `vis`'s
/// visualizer effect (the first effect that is one).
pub fn rebake(p: &mut Project, vis: Id, audio: Id, backend: Backend) -> Result<usize, String> {
    let a = p.clip(audio).ok_or("no such audio clip")?.clone();
    let path = p.asset(a.asset).ok_or("the audio clip has no media")?.path.clone();
    let v = p.clip(vis).ok_or("no such clip")?;
    let (start, len) = (v.start, v.duration);
    let song = analysis(&path, a.audio_stream, backend)?;
    let v = p.clip_mut(vis).ok_or("no such clip")?;
    let e = v.effects.iter_mut().find(|e| style_of(e).is_some()).ok_or("the clip has no visualizer effect")?;
    let song = song.mixed(&e.audio_mix.clone().unwrap_or_default());
    // ponytail: one source time + speed for the whole span; reversed / retimed-curve clips read linearly
    let an = song.span(a.src_time(start), len, a.speed.abs(), FPS);
    Ok(bake(e, &an, FPS))
}

/// Swap `vis`'s visualizer effect for `style` (keeping its place in the stack and its on/off) and bake
/// the audio under it again. Without audio under it the new style keeps its header defaults.
pub fn restyle(p: &mut Project, vis: Id, style: &Style, backend: Backend) -> Result<(), String> {
    let c = p.clip_mut(vis).ok_or("no such clip")?;
    let i = c.effects.iter().position(|e| style_of(e).is_some()).ok_or("the clip has no visualizer effect")?;
    let mut e = Effect::from_shader(&style.src);
    e.enabled = c.effects[i].enabled;
    e.audio_mix = c.effects[i].audio_mix.clone(); // the band mixer is how the song is heard, not the look
    c.effects[i] = e;
    match source_of(p, vis) {
        Some(a) => rebake(p, vis, a, backend).map(|_| ()),
        None => Ok(()),
    }
}

/// A full-frame black shape over audio clip `audio`'s span carrying `style`, baked from its sound: the
/// one-click visualizer (clip menu ▸ Audio, `audio.visualizer`). It joins the song's link group (a
/// new one when the song has none), so moving, trimming or deleting either takes the other along like a
/// video's own audio. Returns the new clip.
pub fn add(p: &mut Project, audio: Id, style: &Style, backend: Backend) -> Result<Id, String> {
    let (at, dur) = p.clip(audio).filter(|c| c.kind == ClipKind::Audio).map(|c| (c.start, c.duration)).ok_or("not an audio clip")?;
    let id = p.add_shape_clip(crate::model::ShapeKind::Rect, at, dur);
    // ShapeStyle w/h are half-sizes: the layer is exactly the frame
    let (w, h) = (p.width as f64 / 2.0, p.height as f64 / 2.0);
    let c = p.clip_mut(id).ok_or("shape clip vanished")?;
    c.name = "Audio Visualizer".into();
    if let Some(s) = c.shape.as_mut() {
        (s.w, s.h, s.fill) = (Animated::new(w), Animated::new(h), [0, 0, 0, 255]);
    }
    c.effects.push(Effect::from_shader(&style.src));
    if let Err(e) = rebake(p, id, audio, backend) {
        p.delete_clips(&[id], false);
        return Err(e);
    }
    link(p, audio, id);
    Ok(id)
}

/// Put `vis` in `audio`'s link group, giving the song one first if it has none.
fn link(p: &mut Project, audio: Id, vis: Id) {
    let l = match p.clip(audio).map(|c| c.link) {
        Some(0) | None => {
            let l = p.new_id();
            if let Some(c) = p.clip_mut(audio) {
                c.link = l;
            }
            l
        }
        Some(l) => l,
    };
    if let Some(c) = p.clip_mut(vis) {
        c.link = l;
    }
}

/// The song itself with its visualizer: audio asset `asset` placed at `at` (the first audio track free
/// there) and a linked visualizer layer over the same span (Library ▸ Add with Audio Visualizer,
/// `audio.visualizer {asset_id}`). Nothing is left behind on failure. Returns (song clip, visualizer).
pub fn add_song(p: &mut Project, asset: Id, at: f64, style: &Style, backend: Backend) -> Result<(Id, Id), String> {
    if p.asset(asset).ok_or("no such asset")?.kind != ClipKind::Audio {
        return Err("not an audio file".into());
    }
    let ids = p.insert_asset_clips(asset, at.max(0.0), None);
    let audio = ids.iter().copied().find(|&id| p.clip(id).is_some_and(|c| c.kind == ClipKind::Audio));
    let Some(audio) = audio else {
        p.delete_clips(&ids, false);
        return Err("the file has no sound to place".into());
    };
    match add(p, audio, style, backend) {
        Ok(vis) => Ok((audio, vis)),
        Err(e) => {
            p.delete_clips(&ids, false);
            Err(e)
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A test source: stereo `f(t)` at 48 kHz.
    struct Tone(fn(f64) -> f32, f64);
    impl AudioSource for Tone {
        fn duration(&self) -> f64 {
            self.1
        }
        fn read_at(&mut self, t: f64, out: &mut [f32]) {
            for (i, c) in out.chunks_exact_mut(2).enumerate() {
                let s = t + i as f64 / SAMPLE_RATE as f64;
                let v = if s < self.1 { (self.0)(s) } else { 0.0 };
                c[0] = v;
                c[1] = v;
            }
        }
    }

    #[test]
    fn every_builtin_is_a_visualizer_within_the_param_limit() {
        for (name, src) in BUILTIN {
            assert!(is_visualizer(src), "{name}");
            let m = crate::engine::shaderlib::parse_meta(src);
            assert_eq!(m.name, name);
            assert!(m.params.len() <= crate::engine::shaderlib::MAX_PARAMS, "{name}: {} params", m.params.len());
            assert_eq!(src.lines().filter(|l| l.starts_with("// @param")).count(), m.params.len(), "{name}: every @param parsed");
            assert!(crate::engine::shaderlib::lint(src, false).is_empty(), "{name}: {:?}", crate::engine::shaderlib::lint(src, false));
        }
        assert!(!is_visualizer(crate::model::DEFAULT_SHADER), "a plain shader is not a style");
        assert_eq!(find(None).unwrap().name, "Neon Orb");
        assert_eq!(find(Some("spectrum bars")).unwrap().name, "Spectrum Bars");
        assert!(find(Some("nope")).unwrap_err().contains("Neon Orb"));
    }

    /// A low hum lights band 1, a high whine band 6; loud halves give a high level, silence 0.
    #[test]
    fn analyze_separates_bass_from_treble_and_follows_loudness() {
        let low = |t: f64| if t < 1.0 { 0.8 * (std::f64::consts::TAU * 60.0 * t).sin() as f32 } else { 0.0 };
        let f = analyze(&mut Tone(low, 2.0), 0.0, 2.0, 1.0, 30.0).frames;
        assert_eq!(f.len(), 61);
        assert!(f[15][1] > 0.9 && f[15][6] < 0.3, "bass tone: {:?}", f[15]);
        assert!(f[15][0] > 0.9, "loud: {:?}", f[15]);
        assert!(f[60][0] < 0.01, "silence settles: {:?}", f[60]);
        let high = |t: f64| 0.5 * (std::f64::consts::TAU * 8000.0 * t).sin() as f32;
        let h = analyze(&mut Tone(high, 1.0), 0.0, 1.0, 1.0, 30.0).frames;
        assert!(h[15][6] > 0.9 && h[15][1] < 0.3, "treble tone: {:?}", h[15]);
        assert_eq!(f, analyze(&mut Tone(low, 2.0), 0.0, 2.0, 1.0, 30.0).frames, "deterministic");
    }

    const RINGS: &str = include_str!("../../tools/visualizer-sample/pulse-rings.glsl");

    /// A custom library shader that reads audio joins the picker after the built-ins (by its @name); a
    /// plain effect, a nameless one and an installed copy of a built-in do not add rows.
    #[test]
    fn custom_library_shaders_join_the_styles() {
        let lib = vec![
            ("pulse-rings".to_string(), RINGS.to_string()),
            ("blur".to_string(), crate::model::DEFAULT_SHADER.to_string()),
            ("neon-orb".to_string(), BUILTIN[0].1.to_string()),
            ("my-viz".to_string(), "// @param level 0 0 1
vec4 effect(vec4 s, vec2 uv) { return vec4(level); }
".to_string()),
        ];
        let all = styles_from(lib);
        let names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Neon Orb", "Spectrum Bars", "Voxel Blocks", "Pulse Rings", "my-viz"]);
        assert!(!all[0].custom && all[3].custom);
        assert_eq!(find_in(all.clone(), Some("PULSE RINGS")).unwrap().src, RINGS);
        assert!(crate::engine::shaderlib::lint(RINGS, false).is_empty(), "{:?}", crate::engine::shaderlib::lint(RINGS, false));
    }

    /// Restyling swaps the effect in place (same slot, on/off kept), with no audio under the clip the new
    /// style keeps its defaults; a custom style works the same as a built-in.
    #[test]
    fn restyle_swaps_the_visualizer_in_place() {
        let mut p = Project::new();
        let id = p.add_shape_clip(crate::model::ShapeKind::Rect, 0.0, 2.0);
        let c = p.clip_mut(id).unwrap();
        c.effects.push(Effect::new(crate::model::EffectKind::Blur));
        c.effects.push(Effect::from_shader(BUILTIN[0].1));
        c.effects[1].enabled = false;
        let custom = styles_from([("pulse-rings".to_string(), RINGS.to_string())]).pop().unwrap();
        for s in [find_in(styles_from([]), Some("Spectrum Bars")).unwrap(), custom] {
            restyle(&mut p, id, &s, Backend::Auto).unwrap();
            let c = p.clip(id).unwrap();
            assert_eq!(c.effects.len(), 2);
            assert_eq!(c.effects[0].kind, crate::model::EffectKind::Blur, "other effects untouched");
            assert_eq!(style_of(&c.effects[1]).as_deref(), Some(s.name.as_str()));
            assert!(!c.effects[1].enabled, "on/off kept");
            assert!(c.effects[1].params.iter().all(|a| a.keys.is_empty()), "no audio under it: defaults");
        }
        assert!(restyle(&mut p, 999, &find(None).unwrap(), Backend::Auto).is_err());
    }

    /// The visualizer listens to the audio-track clip it overlaps most.
    #[test]
    fn source_is_the_audio_under_the_clip() {
        let mut p = Project::new();
        let ati = *p.audio_tracks().first().unwrap();
        for (id, at) in [(500, 0.0), (501, 1.5)] {
            p.tracks[ati].clips.push(crate::model::Clip::new(id, ClipKind::Audio, "song", at, 2.0));
        }
        let v = p.add_shape_clip(crate::model::ShapeKind::Rect, 1.4, 2.0);
        assert_eq!(source_of(&p, v), Some(501));
        let lone = p.add_shape_clip(crate::model::ShapeKind::Rect, 9.0, 1.0);
        assert_eq!(source_of(&p, lone), None);
    }

    /// A click every half second: beats on the clicks, 120 BPM, and the beat params bake as an exact
    /// sawtooth (0 on each hit, ~1 just before the next) and a step count.
    #[test]
    fn beats_are_found_and_baked_as_phase_and_count() {
        let clicks = |t: f64| if (t + 0.25) % 0.5 < 0.01 { 0.9 } else { 0.0 };
        let an = analyze(&mut Tone(clicks, 4.0), 0.0, 4.0, 1.0, 30.0);
        assert!(an.beats.len() >= 6, "{:?}", an.beats);
        let off = |b: &f64| { let r = (b + 0.25) % 0.5; r.min(0.5 - r) };
        assert!(an.beats.iter().all(|b| off(b) < 0.03), "{:?}", an.beats);
        let bpm = an.bpm.expect("a tempo");
        assert!((bpm - 120.0).abs() < 3.0, "{bpm}");
        let src = "// @name B\n// @param beat 0 0 1\n// @param beats 0 0 100000\n// @param bpm 120 0 300\nvec4 effect(vec4 s, vec2 uv) { return vec4(beat); }\n";
        let mut e = Effect::from_shader(src);
        assert_eq!(bake(&mut e, &an, 30.0), 3);
        let (b0, b1) = (an.beats[0], an.beats[1]);
        assert!(e.at(0, b0).abs() < 1e-6 && e.at(0, b1 - 0.002) > 0.99, "phase resets on the beat");
        assert!((e.at(0, (b0 + b1) / 2.0) - 0.5).abs() < 0.05, "half way: {}", e.at(0, (b0 + b1) / 2.0));
        assert_eq!(e.at(1, b0 - 0.01), 0.0);
        assert_eq!((e.at(1, b0 + 0.01), e.at(1, b1 + 0.01)), (1.0, 2.0), "count steps on each beat");
        assert!((e.at(2, 1.0) - bpm).abs() < 1e-9);
        // the Source monitor reads the same beat inputs from the whole-song analysis
        let song = SongAnalysis::compute(&mut Tone(clicks, 4.0));
        for t in [0.1, b0 + 0.05, (b0 + b1) / 2.0, 2.33, 3.6] {
            let (phase, count, _) = song.beat_at(t);
            assert!((phase - e.at(0, t)).abs() < 0.02 && count == e.at(1, t), "beat inputs at {t}: {phase} {count}");
        }
        assert!(is_visualizer(src), "beat params alone make a visualizer");
    }

    /// A 2 s mono 48 kHz WAV: a loud 220 Hz tone for the first second, nearly silent after. Written
    /// once per test under the temp dir, decoded by the real decoders like any imported song.
    pub(crate) fn song_wav(tag: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("se-visualizer-{tag}-{}.wav", std::process::id()));
        let n = 2 * SAMPLE_RATE as usize;
        let mut b: Vec<u8> = Vec::with_capacity(44 + n * 2);
        let le32 = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_le_bytes());
        b.extend_from_slice(b"RIFF");
        le32(&mut b, 36 + n as u32 * 2);
        b.extend_from_slice(b"WAVEfmt ");
        le32(&mut b, 16);
        b.extend_from_slice(&[1, 0, 1, 0]); // PCM, mono
        le32(&mut b, SAMPLE_RATE);
        le32(&mut b, SAMPLE_RATE * 2);
        b.extend_from_slice(&[2, 0, 16, 0]);
        b.extend_from_slice(b"data");
        le32(&mut b, n as u32 * 2);
        for i in 0..n {
            let t = i as f64 / SAMPLE_RATE as f64;
            let amp = if t < 1.0 { 0.8 } else { 0.02 };
            let v = (amp * (std::f64::consts::TAU * 220.0 * t).sin() * 32767.0) as i16;
            b.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(&path, b).unwrap();
        path
    }

    pub(crate) fn song_asset(p: &mut Project, path: &std::path::Path) -> Id {
        let mut a = crate::media::probe(&path.to_string_lossy(), Backend::Ffmpeg).expect("probe the test song");
        a.id = 0;
        p.add_asset(a)
    }

    /// Library ▸ Add with Audio Visualizer: the song lands at the playhead on an audio track, the
    /// visualizer over exactly its span on a video track, both in one link group, the audio baked in
    /// (loud first second, quiet after).
    #[test]
    fn add_song_places_the_song_and_a_linked_visualizer() {
        let wav = song_wav("add");
        let mut p = Project::new();
        let asset = song_asset(&mut p, &wav);
        let style = find(None).unwrap();
        let (audio, vis) = add_song(&mut p, asset, 3.0, &style, Backend::Ffmpeg).unwrap();
        let (a, v) = (p.clip(audio).unwrap().clone(), p.clip(vis).unwrap().clone());
        assert_eq!(p.tracks[p.track_of(audio).unwrap()].kind, TrackKind::Audio);
        assert_eq!(p.tracks[p.track_of(vis).unwrap()].kind, TrackKind::Video);
        assert!((a.start - 3.0).abs() < 1e-9 && (a.duration - 2.0).abs() < 0.05, "{} {}", a.start, a.duration);
        assert_eq!((v.start, v.duration), (a.start, a.duration), "same span");
        assert!(a.link != 0 && a.link == v.link, "one link group");
        let mut group = p.linked(vis);
        group.sort();
        assert_eq!(group, vec![audio, vis]);
        assert_eq!(source_of(&p, vis), Some(audio));
        let level = |t: f64| v.effects[0].at(0, t);
        assert!(level(0.5) > 0.8 && level(1.6) < 0.2, "loud then quiet: {} {}", level(0.5), level(1.6));
        // not audio / not a file: refused, nothing left on the timeline
        let clips = |p: &Project| p.tracks.iter().map(|t| t.clips.len()).sum::<usize>();
        let before = clips(&p);
        assert!(add_song(&mut p, 9999, 0.0, &style, Backend::Ffmpeg).is_err());
        assert_eq!(clips(&p), before);
        let _ = std::fs::remove_file(wav);
    }

    /// Seek anywhere: the clip's baked value at a timeline time is the song analysis at that SOURCE
    /// time - the very function the Source monitor shows (`SongAnalysis::at`) - so preview, Source
    /// monitor and export agree at every moment, and nothing depends on how playback got there.
    #[test]
    fn the_baked_value_is_the_song_analysis_at_the_source_time() {
        let wav = song_wav("seek");
        let mut p = Project::new();
        let asset = song_asset(&mut p, &wav);
        let song = analysis(&wav.to_string_lossy(), 0, Backend::Ffmpeg).unwrap();
        // placed at 0.5 s, trimmed to start 0.4 s into the song: timeline t -> source t - 0.5 + 0.4
        let (audio, vis) = add_song(&mut p, asset, 0.5, &find(None).unwrap(), Backend::Ffmpeg).unwrap();
        let group = p.linked(audio);
        assert!(p.trim_edges(&group.iter().map(|&id| (id, true)).collect::<Vec<_>>(), 0.4, false));
        rebake(&mut p, vis, audio, Backend::Ffmpeg).unwrap();
        let c = p.clip(vis).unwrap();
        let a = p.clip(audio).unwrap();
        for k in [0, 7, 15, 22, 30] {
            let local = k as f64 / FPS; // a key time
            let src = a.src_time(c.start + local);
            assert!((src - (0.4 + local)).abs() < 1e-9);
            let want = song.at(src);
            let got = c.effects[0].at(0, local);
            assert!((got - want[0] as f64).abs() < 1e-3, "level at {local}: {got} vs {}", want[0]);
        }
        let _ = std::fs::remove_file(wav);
    }

    /// The whole-song analysis is computed once and shared: a second request is the same object.
    #[test]
    fn the_song_analysis_is_cached() {
        let wav = song_wav("cache");
        let path = wav.to_string_lossy().into_owned();
        let a = analysis(&path, 0, Backend::Ffmpeg).unwrap();
        let b = analysis(&path, 0, Backend::Ffmpeg).unwrap();
        assert!(std::sync::Arc::ptr_eq(&a, &b));
        assert!(analysis_async(&path, 0, Backend::Ffmpeg, || {}).is_some_and(|c| std::sync::Arc::ptr_eq(&a, &c)));
        assert_eq!(a.frames.len(), 201, "2 s at 100 Hz");
        assert!(a.at(0.5)[0] > 0.8 && a.at(1.6)[0] < 0.1, "loud then quiet: {:?} {:?}", a.at(0.5), a.at(1.6));
        assert_eq!(a.at(5.0), [0.0; 7], "silence past the end");
        let _ = std::fs::remove_file(wav);
    }

    /// Linked like a video and its audio: what the timeline does to a link group (move, trim, delete -
    /// its gestures expand a pick to `Project::linked`) keeps the song and its visualizer together.
    #[test]
    fn the_song_and_its_visualizer_move_trim_and_delete_together() {
        let wav = song_wav("move");
        let mut p = Project::new();
        let asset = song_asset(&mut p, &wav);
        let (audio, vis) = add_song(&mut p, asset, 0.0, &find(None).unwrap(), Backend::Ffmpeg).unwrap();
        let group = p.linked(audio);
        assert!(p.move_clips(&group, 1.5, 0, None));
        assert_eq!((p.clip(audio).unwrap().start, p.clip(vis).unwrap().start), (1.5, 1.5));
        let edges: Vec<(Id, bool)> = p.linked(vis).into_iter().map(|id| (id, false)).collect();
        assert!(p.trim_edges(&edges, -0.5, false), "trim both ends");
        let (a, v) = (p.clip(audio).unwrap(), p.clip(vis).unwrap());
        assert!((a.duration - v.duration).abs() < 1e-9 && (a.duration - 1.5).abs() < 0.05, "{} {}", a.duration, v.duration);
        p.delete_clips(&p.linked(vis), false);
        assert!(p.clip(audio).is_none() && p.clip(vis).is_none(), "deleted together");
        let _ = std::fs::remove_file(wav);
    }

    /// Add Audio Visualizer on a song already on the timeline links too - into the song's existing group
    /// when it has one (a video's own audio), so the picture, its sound and the visualizer stay together.
    #[test]
    fn add_links_into_an_existing_group() {
        let wav = song_wav("group");
        let mut p = Project::new();
        let asset = song_asset(&mut p, &wav);
        let audio = p.insert_asset_clips(asset, 0.0, None)[0];
        let video = p.add_shape_clip(crate::model::ShapeKind::Rect, 0.0, 2.0);
        let l = p.new_id();
        p.clip_mut(audio).unwrap().link = l;
        p.clip_mut(video).unwrap().link = l;
        let vis = add(&mut p, audio, &find(Some("Spectrum Bars")).unwrap(), Backend::Ffmpeg).unwrap();
        assert_eq!(p.clip(vis).unwrap().link, l);
        assert_eq!(p.linked(vis).len(), 3);
        let _ = std::fs::remove_file(wav);
    }

    fn sub_then_highmid(t: f64) -> f32 {
        let f = if t < 1.0 { 60.0 } else { 5000.0 };
        (0.6 * (std::f64::consts::TAU * f * t).sin()) as f32
    }

    /// A kick (60 Hz) on every half second, a hi-hat (5 kHz) half way between: short decaying bursts.
    fn kick_and_hat(t: f64) -> f32 {
        let (k, h) = (t % 0.5, (t + 0.25) % 0.5);
        let kick = if k < 0.12 { (-k * 30.0).exp() * (std::f64::consts::TAU * 60.0 * t).sin() } else { 0.0 };
        let hat = if h < 0.05 { (-h * 80.0).exp() * (std::f64::consts::TAU * 5000.0 * t).sin() } else { 0.0 };
        (0.7 * kick + 0.5 * hat) as f32
    }

    /// The bands have real edges: a 60 Hz tone lands in Sub, a 5 kHz one in High-mid.
    #[test]
    fn tones_land_in_their_bands() {
        let tone = |f: f64| -> Vec<f32> { (0..4096).map(|i| (std::f64::consts::TAU * f * i as f64 / SAMPLE_RATE as f64).sin() as f32).collect() };
        let low = raw_frame(&tone(60.0));
        let high = raw_frame(&tone(5000.0));
        let loudest = |f: &Frame| (0..BANDS).max_by(|&a, &b| f[1 + a].total_cmp(&f[1 + b])).unwrap();
        assert_eq!(loudest(&low), 0, "60 Hz is Sub: {low:?}");
        assert_eq!(loudest(&high), 4, "5 kHz is High-mid: {high:?}");
        assert!(low[5] < low[1] * 0.01 && high[1] < high[5] * 0.01, "no leak across the spectrum");
    }

    /// The band mixer decides what drives the visualizer: a 60 Hz second then a 5 kHz second. Weighting
    /// Sub only, the level is up in the first second and down in the second; weighting High-mid only,
    /// the other way round; a band at gain 0 is ignored, and sensitivity scales.
    #[test]
    fn band_weights_drive_the_level() {
        let song = SongAnalysis::compute(&mut Tone(sub_then_highmid, 2.0));
        let only = |b: usize| crate::model::AudioMix { gains: std::array::from_fn(|i| if i == b { 1.0 } else { 0.0 }), ..Default::default() };
        let sub = song.mixed(&only(0));
        let hi = song.mixed(&only(4));
        assert!(sub.at(0.5)[0] > 0.9 && sub.at(1.5)[0] < 0.1, "Sub-driven: {} {}", sub.at(0.5)[0], sub.at(1.5)[0]);
        assert!(hi.at(0.5)[0] < 0.1 && hi.at(1.5)[0] > 0.9, "High-mid-driven: {} {}", hi.at(0.5)[0], hi.at(1.5)[0]);
        assert_eq!(sub.at(1.5)[5], 0.0, "gain 0: the High-mid band output is ignored");
        let soft = song.mixed(&crate::model::AudioMix { sensitivity: 0.5, ..only(0) });
        assert!((soft.at(0.5)[0] - 0.5).abs() < 0.05, "half the sensitivity, half the drive: {}", soft.at(0.5)[0]);
        // slow attack: the level rises gradually instead of jumping
        let slow = song.mixed(&crate::model::AudioMix { attack: 0.3, ..only(0) });
        assert!(slow.at(0.05)[0] < sub.at(0.05)[0] * 0.5, "attack smooths the rise");
    }

    /// The beat source: listening to Sub finds the kicks (every 0.5 s from 0), listening to High-mid finds
    /// the hi-hats (0.25 s later).
    #[test]
    fn the_beat_source_picks_what_the_beats_follow() {
        let song = SongAnalysis::compute(&mut Tone(kick_and_hat, 4.0));
        let src = |b: usize| crate::model::AudioMix { beat_bands: std::array::from_fn(|i| i == b), ..Default::default() };
        let phase = |beats: &[f64]| -> f64 {
            assert!(beats.len() >= 5, "{beats:?}");
            beats.iter().map(|b| b % 0.5).sum::<f64>() / beats.len() as f64
        };
        let kicks = song.mixed(&src(0)).beats;
        let hats = song.mixed(&src(4)).beats;
        assert!(phase(&kicks) < 0.08, "Sub hears the kicks: {kicks:?}");
        assert!((phase(&hats) - 0.25).abs() < 0.08, "High-mid hears the hats: {hats:?}");
    }

    /// Presets and hand-set mixes are saved with the project and come back the same; a project from
    /// before the band mixer loads with none; restyling keeps the mix.
    #[test]
    fn the_band_mix_round_trips_in_the_project() {
        let mut p = Project::new();
        let id = p.add_shape_clip(crate::model::ShapeKind::Rect, 0.0, 2.0);
        let mut e = Effect::from_shader(BUILTIN[2].1);
        e.audio_mix = crate::model::AudioMix::preset("edm");
        assert_eq!(e.audio_mix.as_ref().unwrap().preset, "EDM");
        p.clip_mut(id).unwrap().effects.push(e);
        let back = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(back.clip(id).unwrap().effects[0].audio_mix, crate::model::AudioMix::preset("EDM"));
        for name in crate::model::AudioMix::PRESETS {
            let m = crate::model::AudioMix::preset(name).unwrap();
            assert_eq!(m.clone().clamped(), m, "{name} is inside the UI ranges");
        }
        assert!(crate::model::AudioMix::preset("polka").is_none());
        let old: Effect = serde_json::from_str(r#"{"kind":"Shader","shader":"// @param level 0 0 1\nvec4 effect(vec4 s, vec2 uv){return s;}"}"#).unwrap();
        assert_eq!(old.audio_mix, None);
        restyle(&mut p, id, &find(Some("Neon Orb")).unwrap(), Backend::Auto).unwrap();
        assert_eq!(p.clip(id).unwrap().effects[0].audio_mix, crate::model::AudioMix::preset("EDM"), "kept across styles");
        assert_eq!(crate::model::AudioMix::band("high mid"), Some(4));
    }

    /// `bake` keys exactly the audio params a shader declares, on their own ranges.
    #[test]
    fn bake_keys_only_the_declared_audio_params() {
        let src = "// @name Custom\n// @param band1 0 0 2\n// @param glow 1 0 3\n// @param level 0 0 1\nvec4 effect(vec4 s, vec2 uv) { return vec4(band1 * level * glow); }\n";
        let mut e = Effect::from_shader(src);
        let an = Analysis { frames: vec![[1.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0], [0.0; 7]], ..Default::default() };
        assert_eq!(bake(&mut e, &an, 30.0), 2);
        assert_eq!(e.params[0].keys.iter().map(|k| k.v).collect::<Vec<_>>(), vec![1.0, 0.0], "band1 on 0..2");
        assert!(e.params[1].keys.is_empty(), "glow is not audio");
        assert_eq!(e.params[2].keys.iter().map(|k| k.v).collect::<Vec<_>>(), vec![1.0, 0.0]);
        assert_eq!(style_of(&e).as_deref(), Some("Custom"));
    }
}
