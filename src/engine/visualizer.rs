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

/// Band centre frequencies (Hz), log-spaced 60 Hz .. 8 kHz.
fn band_hz(b: usize) -> f32 {
    60.0 * (8000.0f32 / 60.0).powf(b as f32 / (BANDS - 1) as f32)
}

/// Samples per DFT window (~21 ms at 48 kHz).
const DFT_N: usize = 1024;

/// Raw (un-normalised) [peak, band magnitudes] of mono `x`: a Hann-windowed DFT of the last `DFT_N`
/// samples, evaluated at three frequencies around each band centre (the loudest wins).
// ponytail: naive DFT, 6*3*1024 ~ 18k multiply-adds per frame; an FFT only if bands or N grow a lot.
pub fn raw_frame(x: &[f32]) -> Frame {
    let peak = x.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
    let w = &x[x.len().saturating_sub(DFT_N)..];
    let n = w.len().max(1) as f32;
    let win: Vec<f32> =
        w.iter().enumerate().map(|(i, &s)| s * (0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n).cos())).collect();
    let at = |f: f32| {
        let k = std::f32::consts::TAU * f / SAMPLE_RATE as f32;
        let (re, im) = win.iter().enumerate().fold((0.0f32, 0.0f32), |(re, im), (i, &s)| {
            let a = k * i as f32;
            (re + s * a.cos(), im - s * a.sin())
        });
        (re * re + im * im).sqrt() / n
    };
    let mut out = [0.0; 1 + BANDS];
    out[0] = peak;
    for b in 0..BANDS {
        let f = band_hz(b);
        out[1 + b] = [f * 0.85, f, f * 1.18].into_iter().map(at).fold(0.0, f32::max);
    }
    out
}

/// Normalise each column to its loudest frame (so quiet files and weak highs still move), shape it
/// (sqrt for the bands, which read flat otherwise) and give it an instant attack / ~0.15 s release so
/// it pulses on hits instead of flickering. Deterministic.
pub fn shape(raw: &[Frame], fps: f64) -> Vec<Frame> {
    let mut top = [1e-4f32; 1 + BANDS];
    for f in raw {
        for (t, &v) in top.iter_mut().zip(f) {
            *t = t.max(v);
        }
    }
    let fall = (-1.0 / fps / 0.15).exp() as f32;
    let mut env = [0.0f32; 1 + BANDS];
    raw.iter()
        .map(|f| {
            for c in 0..1 + BANDS {
                let v = (f[c] / top[c]).clamp(0.0, 1.0);
                let v = if c == 0 { v } else { v.sqrt() };
                env[c] = v.max(env[c] * fall);
            }
            env
        })
        .collect()
}

/// What `analyze` found over a span: one `Frame` per key, and the beats (clip-local seconds) + tempo.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Analysis {
    pub frames: Vec<Frame>,
    pub beats: Vec<f64>,
    pub bpm: Option<f64>,
}

/// Analyse `len` seconds of `src` starting at source second `from` (played at `speed`), `fps` frames a
/// second: `len * fps + 1` frames, plus the beats `analysis::onsets` finds (the same picker as Detect
/// Beats / `audio.beats`, on a waveform envelope built from the same pass). Decodes the audio once.
// ponytail: runs on the caller's thread (an edit, never playback); a 4-minute song is about a second.
pub fn analyze(src: &mut dyn AudioSource, from: f64, len: f64, speed: f64, fps: f64) -> Analysis {
    let n = (len * fps).floor() as usize + 1;
    let speed = if speed.is_finite() && speed > 0.0 { speed } else { 1.0 };
    let hop = ((SAMPLE_RATE as f64 * speed / fps).round() as usize).max(1);
    let mut stereo = vec![0f32; hop * 2];
    let mut hist: Vec<f32> = vec![0.0; DFT_N.max(hop)];
    // the envelope for the beat picker: PEAKS_PER_SEC buckets of source time from `from`
    let bucket = (SAMPLE_RATE / crate::media::waveform::PEAKS_PER_SEC) as usize;
    let mut peaks = crate::media::waveform::Peaks { min: Vec::new(), max: Vec::new() };
    let mut pend: Vec<f32> = Vec::new();
    let raw: Vec<Frame> = (0..n)
        .map(|i| {
            src.read_at(from + i as f64 * hop as f64 / SAMPLE_RATE as f64, &mut stereo);
            let mono: Vec<f32> = stereo.chunks_exact(2).map(|c| 0.5 * (c[0] + c[1])).collect();
            pend.extend_from_slice(&mono);
            while pend.len() >= bucket {
                let b: Vec<f32> = pend.drain(..bucket).collect();
                peaks.min.push(b.iter().fold(0.0f32, |m, &v| m.min(v)));
                peaks.max.push(b.iter().fold(0.0f32, |m, &v| m.max(v)));
            }
            hist.drain(..mono.len().min(hist.len()));
            hist.extend_from_slice(&mono);
            let mut f = raw_frame(&hist[hist.len() - DFT_N..]);
            f[0] = mono.iter().fold(0.0f32, |m, &s| m.max(s.abs())); // the peak of this frame only
            f
        })
        .collect();
    let onsets = crate::engine::analysis::onsets(&peaks, 0.0, len * speed, BEAT_REFRACTORY, BEAT_SENSITIVITY);
    let bpm = crate::engine::analysis::bpm(&onsets);
    Analysis { frames: shape(&raw, fps), beats: onsets.iter().map(|t| t / speed).collect(), bpm }
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
    let mut src = crate::media::open_audio(&path, a.audio_stream, backend)?;
    // ponytail: one source time + speed for the whole span; reversed / retimed-curve clips read linearly
    let an = analyze(&mut *src, a.src_time(start), len, a.speed.abs(), FPS);
    let v = p.clip_mut(vis).ok_or("no such clip")?;
    let e = v.effects.iter_mut().find(|e| style_of(e).is_some()).ok_or("the clip has no visualizer effect")?;
    Ok(bake(e, &an, FPS))
}

/// Swap `vis`'s visualizer effect for `style` (keeping its place in the stack and its on/off) and bake
/// the audio under it again. Without audio under it the new style keeps its header defaults.
pub fn restyle(p: &mut Project, vis: Id, style: &Style, backend: Backend) -> Result<(), String> {
    let c = p.clip_mut(vis).ok_or("no such clip")?;
    let i = c.effects.iter().position(|e| style_of(e).is_some()).ok_or("the clip has no visualizer effect")?;
    let mut e = Effect::from_shader(&style.src);
    e.enabled = c.effects[i].enabled;
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
