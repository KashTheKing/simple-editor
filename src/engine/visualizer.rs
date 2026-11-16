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

/// The beat-synced sprite shader: it shows a cell of the image clip it sits on, so it is placed by
/// `add_sprite`, never offered as a style for the plain visualizer layer.
pub const BEAT_SPRITE: &str = include_str!("../../library/effects/beat-sprite.glsl");

/// The built-ins, then each `(id, source)` that is a visualizer, named by its `@name` (else the id).
pub fn styles_from(custom: impl IntoIterator<Item = (String, String)>) -> Vec<Style> {
    let mut out: Vec<Style> =
        BUILTIN.iter().map(|&(n, s)| Style { name: n.into(), src: s.into(), custom: false }).collect();
    for (id, src) in custom {
        // a "sprite" shader animates the picture it sits on: no style for an empty visualizer layer
        if !is_visualizer(&src) || crate::engine::shaderlib::parse_meta(&src).tags.iter().any(|t| t == "sprite") {
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

/// The audio clip a visualizer layer listens to: the audio-track clip overlapping it the most.
pub fn source_of(p: &Project, vis: Id) -> Option<Id> {
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
/// one-click visualizer (clip menu ▸ Audio, `audio.visualizer`). Returns the new clip.
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
    Ok(id)
}

/// A sprite sheet: the image and its grid (`cols` x `rows`, the first `frames` cells used).
#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    pub path: std::path::PathBuf,
    pub cols: u32,
    pub rows: u32,
    pub frames: u32,
}

/// Frames a video / image sequence is sampled to.
pub const SHEET_FRAMES: u32 = 16;

/// `name_4x2.png` / `dance 8x1.png`: the grid written in an image's file name, else None.
pub fn grid_in_name(path: &std::path::Path) -> Option<(u32, u32)> {
    let stem = path.file_stem()?.to_string_lossy().to_ascii_lowercase();
    let tail = stem.rsplit(|c: char| c == '_' || c == ' ' || c == '-').next()?;
    let (c, r) = tail.split_once('x')?;
    let (c, r) = (c.parse::<u32>().ok()?, r.parse::<u32>().ok()?);
    (1..=16).contains(&c).then_some(())?;
    (1..=16).contains(&r).then_some((c, r))
}

/// The sprite sheet for `input`: an image is a sheet already (grid from its name, else one cell); a
/// video, or a folder of numbered images, is sampled to `SHEET_FRAMES` evenly spaced frames tiled 4x4
/// into a PNG (alpha kept) under `out_dir`, by ffmpeg, once.
pub fn make_sheet(input: &std::path::Path, out_dir: &std::path::Path) -> Result<Sheet, String> {
    const IMAGES: [&str; 6] = ["png", "jpg", "jpeg", "bmp", "webp", "gif"];
    let ext = |p: &std::path::Path| p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if input.is_file() && IMAGES.contains(&ext(input).as_str()) {
        let (cols, rows) = grid_in_name(input).unwrap_or((1, 1));
        return Ok(Sheet { path: input.to_path_buf(), cols, rows, frames: cols * rows });
    }
    let ffmpeg = crate::media::ffpipe::ffmpeg_exe().ok_or("ffmpeg not found")?;
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let (cols, rows) = (4, SHEET_FRAMES / 4);
    let name = crate::engine::library::slug(&input.file_stem().unwrap_or_default().to_string_lossy());
    let out = out_dir.join(format!("{}-{cols}x{rows}.png", if name.is_empty() { "loop" } else { &name }));
    let tile = format!("scale=256:256:force_original_aspect_ratio=decrease,format=rgba,tile={cols}x{rows}");
    let mut cmd = crate::media::ffpipe::command(&ffmpeg);
    cmd.args(["-v", "error", "-y"]);
    if input.is_dir() {
        // a numbered image sequence: every Nth file so SHEET_FRAMES span the whole loop
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(input)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .filter(|p| IMAGES.contains(&ext(p).as_str()))
            .collect();
        files.sort();
        if files.is_empty() {
            return Err("the folder has no images".into());
        }
        let pick: Vec<&std::path::PathBuf> =
            (0..SHEET_FRAMES as usize).map(|i| &files[i * files.len() / SHEET_FRAMES as usize]).collect();
        let list = out_dir.join(format!("{name}-frames.txt"));
        let text: String = pick.iter().map(|p| format!("file '{}'\n", p.display().to_string().replace('\'', "'\\''"))).collect();
        std::fs::write(&list, text).map_err(|e| e.to_string())?;
        cmd.args(["-f", "concat", "-safe", "0", "-i"]).arg(&list).args(["-vf", &tile]);
    } else {
        let dur = crate::media::probe(&input.to_string_lossy(), Backend::Auto).map(|a| a.duration).unwrap_or(0.0);
        if dur <= 0.0 {
            return Err(format!("{}: not a video, image or image folder", input.display()));
        }
        let vf = format!("fps={:.6},{tile}", SHEET_FRAMES as f64 / dur);
        cmd.arg("-i").arg(input).args(["-vf", &vf]);
    }
    let st = cmd.args(["-frames:v", "1"]).arg(&out).stdin(std::process::Stdio::null()).output().map_err(|e| e.to_string())?;
    if !st.status.success() || !out.is_file() {
        return Err(format!("ffmpeg could not tile the loop: {}", String::from_utf8_lossy(&st.stderr).trim()));
    }
    Ok(Sheet { path: out, cols, rows, frames: SHEET_FRAMES })
}

/// The beat-synced clip: image asset `sheet_asset` (a sprite sheet) over audio clip `audio`'s span with
/// the Beat Sprite effect, its grid set and its beats baked from the audio. Returns the new clip.
pub fn add_sprite(p: &mut Project, audio: Id, sheet_asset: Id, sheet: &Sheet, backend: Backend) -> Result<Id, String> {
    let (at, dur) = p.clip(audio).filter(|c| c.kind == ClipKind::Audio).map(|c| (c.start, c.duration)).ok_or("not an audio clip")?;
    let vt = p.video_tracks().last().copied();
    let ti = p.find_free_track(TrackKind::Video, at, dur, vt);
    let mut c = crate::model::Clip::new(p.new_id(), ClipKind::Image, "Beat-Synced Clip", at, dur);
    c.asset = sheet_asset;
    // a dancer beside whatever is under it: lower right, half height (move / scale it like any clip)
    // ponytail: assumes x/y are project px from the canvas centre, as the Inspector's Position shows
    (c.scale, c.x, c.y) = (Animated::new(0.5), Animated::new(p.width as f64 * 0.3), Animated::new(p.height as f64 * 0.22));
    let mut e = Effect::from_shader(BEAT_SPRITE);
    let meta = crate::engine::shaderlib::parse_meta(BEAT_SPRITE);
    for (i, prm) in meta.params.iter().enumerate() {
        let v = match prm.id.as_str() {
            "cols" => sheet.cols as f64,
            "rows" => sheet.rows as f64,
            "frames" => sheet.frames as f64,
            _ => continue,
        };
        e.params[i] = Animated::new(v);
    }
    c.effects.push(e);
    let id = c.id;
    p.tracks[ti].clips.push(c);
    p.tracks[ti].sort();
    if let Err(e) = rebake(p, id, audio, backend) {
        p.delete_clips(&[id], false);
        return Err(e);
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn sheet_grids_come_from_the_name() {
        use std::path::Path;
        assert_eq!(grid_in_name(Path::new("C:/a/dance-loop_4x2.png")), Some((4, 2)));
        assert_eq!(grid_in_name(Path::new("walk 8x1.png")), Some((8, 1)));
        assert_eq!(grid_in_name(Path::new("photo.png")), None);
        assert_eq!(grid_in_name(Path::new("big_40x2.png")), None);
        let s = make_sheet(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tools/visualizer-sample/dance-loop_4x2.png")), Path::new(".")).unwrap();
        assert_eq!((s.cols, s.rows, s.frames), (4, 2, 8));
        assert!(!styles_from([("beat-sprite".to_string(), BEAT_SPRITE.to_string())]).iter().any(|s| s.name == "Beat Sprite"), "not a layer style");
    }

    /// The beat-synced clip: an image clip over the song carrying Beat Sprite with the sheet's grid.
    #[test]
    fn add_sprite_places_the_sheet_over_the_song() {
        let mut p = Project::new();
        let at = p.audio_tracks()[0];
        p.tracks[at].clips.push(crate::model::Clip::new(900, ClipKind::Audio, "song", 2.0, 6.0));
        let sheet = Sheet { path: "x_4x2.png".into(), cols: 4, rows: 2, frames: 8 };
        // no media behind the audio clip: placement fails cleanly and leaves nothing behind
        let before = p.tracks.iter().map(|t| t.clips.len()).sum::<usize>();
        assert!(add_sprite(&mut p, 900, 77, &sheet, Backend::Auto).is_err());
        assert_eq!(p.tracks.iter().map(|t| t.clips.len()).sum::<usize>(), before);
        assert!(add_sprite(&mut p, 12345, 77, &sheet, Backend::Auto).is_err(), "not an audio clip");
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
