//! ---- ws:visualizer-helper ----
//! The App side of the Visualizer Helper plugin (`library/plugins/visualizer-helper/`): the five tools its
//! panel calls through `tool()`, the background jobs they start and the FRAME_HOOK that lands them.
//!
//! - `audio.analyze_song`: Analyse Song. Separates stems with the Stem Separation add-on when it is installed
//!   (cached stems are reused), decodes them and the mix off the UI thread, runs `engine::vizhelp::analyze`
//!   and lands the result on the project (`Project::viz`) plus drop / calm / first-vocal markers, as ONE
//!   undo step. It also warms the visualizer's own analysis cache, so the template step never decodes on
//!   the UI thread.
//! - `visualizer.template`: a visualizer (made when the song has none) with the Olympus Storm shader
//!   specialised to the knobs and every hand key written from the analysis.
//! - `lyrics.karaoke`: the pasted lyrics as timed Lyric Chrome text clips (size from the font Size; the clip
//!   only ever scales down).
//! - `visualizer.drop_fx`: a flash + Invert adjustment at each drop, the shatter into the biggest drop (the
//!   visualizer's build plus a Difference-blend shard layer) and the background grade.
//! - `media.loop`: Background Loop, a GIF / still / clip into a seamless 60 fps ping-pong loop (ffmpeg) that
//!   replaces the clip, tiled under the song.
//!
//! Every tool is `Read` or `Mutate` (a plugin may call them); nothing heavy runs on the UI thread.

use super::feedback::{Toast, ToastKind};
use super::tools_helpers::*;
use super::*;
use crate::engine::demucs::{self, Stem};
use crate::engine::export::Progress;
use crate::engine::vizhelp::{analyze, keys, loopbg, lyrics, shader};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::{Animated, BlendMode, Ease, Effect, EffectKind, Keyframe, SongViz};

type Slot<T> = Arc<Mutex<Option<Result<T, String>>>>;

struct AnalyseJob {
    asset: Id,
    stream: usize,
    name: String,
    prog: Arc<Progress>,
    out: Slot<SongViz>,
}

struct LoopJob {
    clip: Id,
    name: String,
    prog: Arc<Progress>,
    out: Slot<crate::model::Asset>,
}

// ponytail: global lists, not App fields - the helper stays out of App's shared struct (as stems_ctl does)
static ANALYSES: Mutex<Vec<AnalyseJob>> = Mutex::new(Vec::new());
static LOOPS: Mutex<Vec<LoopJob>> = Mutex::new(Vec::new());
/// The last failure per song asset / loop clip, for the panel's status line.
static FAILED: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn set_failed(key: String, err: Option<String>) {
    let mut f = lock(&FAILED);
    f.retain(|(k, _)| *k != key);
    if let Some(e) = err {
        f.push((key, e));
    }
}

fn failed(key: &str) -> Option<String> {
    lock(&FAILED).iter().find(|(k, _)| k == key).map(|(_, e)| e.clone())
}

/// Rows for the Jobs pane.
pub(super) fn progress_jobs() -> Vec<jobs_pane::ProgJob> {
    let mut v: Vec<jobs_pane::ProgJob> = lock(&ANALYSES).iter().map(|j| (crate::ui::jobs_ui::JobKind::Bake, format!("Analyse Song · {}", j.name), j.prog.clone())).collect();
    v.extend(lock(&LOOPS).iter().map(|j| (crate::ui::jobs_ui::JobKind::Bake, format!("Background Loop · {}", j.name), j.prog.clone())));
    v
}

// ------------------------------------------------------------------ the song

/// What the steps need to know about the song clip.
struct Song {
    clip: Id,
    asset: Id,
    stream: usize,
    path: String,
    name: String,
    /// Timeline start, length, the song second at its start, and its speed.
    start: f64,
    len: f64,
    from: f64,
    speed: f64,
}

impl Song {
    /// Song second -> timeline second.
    fn timeline(&self, t: f64) -> f64 {
        self.start + (t - self.from) / self.speed
    }
}

fn is_visualizer(c: &Clip) -> bool {
    c.effects.iter().any(|e| crate::engine::visualizer::style_of(e).is_some())
}

/// The song a step works on: `clip` (an audio clip, a video clip's audio, or a visualizer's song), else the
/// selection's, else the song an analysis exists for, else the longest audio clip.
fn song(p: &Project, clip: Option<Id>, selection: &[Id]) -> Result<Song, String> {
    let pick = |id: Id| -> Option<Id> {
        let c = p.clip(id)?;
        if is_visualizer(c) {
            return crate::engine::visualizer::source_of(p, id);
        }
        crate::ui::autocut_ui::audio_targets(p, &[id]).first().copied()
    };
    let audio = || p.tracks.iter().filter(|t| t.kind == TrackKind::Audio).flat_map(|t| &t.clips).filter(|c| c.enabled && p.asset(c.asset).is_some());
    let id = match clip {
        Some(id) => pick(id).ok_or("That clip has no song: pick an audio clip, a video with sound or a visualizer")?,
        None => selection
            .iter()
            .find_map(|&id| pick(id))
            .or_else(|| audio().find(|c| p.viz_of(c.asset, c.audio_stream).is_some()).map(|c| c.id))
            .or_else(|| audio().max_by(|a, b| a.duration.total_cmp(&b.duration)).map(|c| c.id))
            .ok_or("Put a song on the timeline first")?,
    };
    let c = p.clip(id).ok_or("no such clip")?;
    let a = p.asset(c.asset).ok_or("the song clip has no media")?;
    let speed = if c.speed.is_finite() && c.speed.abs() > 1e-6 { c.speed.abs() } else { 1.0 };
    Ok(Song {
        clip: id,
        asset: a.id,
        stream: c.audio_stream,
        path: a.path.clone(),
        name: Path::new(&a.path).file_stem().unwrap_or_default().to_string_lossy().into_owned(),
        start: c.start,
        len: c.duration,
        from: c.src_time(c.start),
        speed,
    })
}

fn arg_song(app: &App, args: &Value) -> Result<Song, String> {
    song(&app.project, arg_u64(args, "clip"), &app.selection)
}

fn analysis<'a>(p: &'a Project, s: &Song) -> Result<&'a SongViz, String> {
    p.viz_of(s.asset, s.stream).ok_or_else(|| format!("Analyse {} first", s.name))
}

/// A clip id that still exists.
fn alive(p: &Project, id: Id) -> Option<Id> {
    (id != 0 && p.clip(id).is_some()).then_some(id)
}

// ------------------------------------------------------------------ Analyse Song

/// Decode `path` to mono at the analysis rate (sequential reads; background threads only).
fn decode(path: &str, stream: usize, backend: Backend) -> Result<Vec<f32>, String> {
    let mut src = crate::media::open_audio(path, stream, backend)?;
    let sr_in = crate::media::SAMPLE_RATE as f64;
    let n_in = (src.duration().max(0.0) * sr_in).ceil() as usize;
    const BLOCK: usize = 4800;
    let mut buf = vec![0f32; BLOCK * 2];
    let mut mono = Vec::with_capacity(n_in);
    let mut i = 0;
    while i < n_in {
        src.read_at(i as f64 / sr_in, &mut buf);
        mono.extend(buf.chunks_exact(2).take(n_in - i).map(|c| 0.5 * (c[0] + c[1])));
        i += BLOCK;
    }
    // box-average each output sample's span of input
    let ratio = sr_in / analyze::SR;
    let n_out = (mono.len() as f64 / ratio) as usize;
    Ok((0..n_out)
        .map(|k| {
            let a = (k as f64 * ratio) as usize;
            let b = (((k + 1) as f64 * ratio) as usize).max(a + 1).min(mono.len());
            mono[a..b].iter().sum::<f32>() / (b - a) as f32
        })
        .collect())
}

fn opts(args: &Value) -> analyze::Opts {
    let d = analyze::Opts::default();
    analyze::Opts {
        bpm_min: arg_f64(args, "tempo_min").unwrap_or(d.bpm_min).clamp(40.0, 300.0),
        bpm_max: arg_f64(args, "tempo_max").unwrap_or(d.bpm_max).clamp(41.0, 400.0),
        gate_db: arg_f64(args, "gate").unwrap_or(d.gate_db).clamp(3.0, 60.0),
        drops: args.get("drops").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_f64()).collect()).unwrap_or_default(),
        after: arg_f64(args, "after").unwrap_or(0.0),
    }
}

fn start_analysis(app: &mut App, s: &Song, o: analyze::Opts) -> Result<(), String> {
    if lock(&ANALYSES).iter().any(|j| j.asset == s.asset && j.stream == s.stream) {
        return Ok(());
    }
    if !Path::new(&s.path).is_file() {
        return Err(format!("{} is offline", s.path));
    }
    set_failed(format!("song:{}", s.asset), None);
    let (path, stream, backend) = (s.path.clone(), s.stream, app.backend());
    let out: Slot<SongViz> = Arc::new(Mutex::new(None));
    let slot = out.clone();
    let prog = crate::engine::export::spawn_job("vizhelp", move |p| {
        let r = (|| {
            // the stems: cached, else separated now when the add-on is installed, else the mix alone
            let src = PathBuf::from(&path);
            let dir = match demucs::cached(&src, stream) {
                Some(d) => Some(d),
                None if demucs::available() && crate::media::ffpipe::ffmpeg_exe().is_some() => {
                    p.set(0.0, "Separating stems");
                    Some(demucs::run(&src, stream, p)?)
                }
                None => None,
            };
            if p.is_cancelled() {
                return Err("cancelled".to_string());
            }
            p.set(0.0, "Reading the song");
            let mix = decode(&path, stream, backend)?;
            let stems = match &dir {
                Some(d) => {
                    let one = |s: Stem| decode(&demucs::stem_file(d, s).to_string_lossy(), 0, backend);
                    p.set(0.02, "Reading the stems");
                    Some(analyze::Stems { vocals: one(Stem::Vocals)?, drums: one(Stem::Drums)?, bass: one(Stem::Bass)?, other: one(Stem::Other)? })
                }
                None => None,
            };
            // the visualizer bake reads this: warm it here, never on the UI thread
            let _ = crate::engine::visualizer::analysis(&path, stream, backend);
            let report = |f: f32, what: &str| p.set(0.05 + 0.95 * f, what.to_string());
            Ok(analyze::analyze(&mix, stems.as_ref(), &o, &report))
        })();
        let err = r.as_ref().err().cloned();
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
        err.map_or(Ok(()), Err)
    });
    lock(&ANALYSES).push(AnalyseJob { asset: s.asset, stream: s.stream, name: s.name.clone(), prog, out });
    Ok(())
}

/// Land a finished analysis: the record, and markers at the drops, the calm part and the first vocal. One
/// labelled undo step.
fn land_analysis(app: &mut App, asset: Id, stream: usize, mut v: SongViz) -> String {
    v.asset = asset;
    v.stream = stream;
    let before = app.project.to_json();
    let old = app.project.viz_of(asset, stream).map(|o| o.made.markers.clone()).unwrap_or_default();
    for m in old {
        app.project.remove_marker(m);
    }
    let summary = format!("{} BPM, {} hits, {} drop{}", v.bpm.round(), v.hits.len(), v.drops.len(), if v.drops.len() == 1 { "" } else { "s" });
    let (drops, calm, first) = (v.drops.clone(), v.calm, v.vocal_onset);
    let dur = v.duration;
    app.project.set_viz(v);
    // markers on every placed clip of this song
    let clips: Vec<Id> = app.project.tracks.iter().flat_map(|t| &t.clips).filter(|c| c.asset == asset && c.audio_stream == stream && c.kind == ClipKind::Audio).map(|c| c.id).collect();
    let mut marks = Vec::new();
    for id in clips {
        let Ok(s) = song(&app.project, Some(id), &[]) else { continue };
        let mut put = |t: f64, name: &str| {
            let tl = s.timeline(t);
            if tl >= s.start - 1e-6 && tl <= s.start + s.len + 1e-6 {
                marks.push((tl, name.to_string()));
            }
        };
        for (i, &d) in drops.iter().enumerate() {
            put(d as f64, if i + 1 == drops.len() && drops.len() > 1 { "Biggest Drop" } else { "Drop" });
        }
        if let Some(c) = calm.filter(|&c| (c as f64) < dur - 0.5) {
            put(c as f64, "Calm");
        }
        if let Some(f) = first {
            put(f as f64, "First Vocal");
        }
    }
    let ids: Vec<Id> = marks.into_iter().map(|(t, n)| app.project.add_marker(t, n)).collect();
    if let Some(r) = app.project.viz_of_mut(asset, stream) {
        r.made.markers = ids;
    }
    app.push_undo_labeled(before, "Analyse Song");
    app.after_edit();
    summary
}

fn status_json(app: &App, s: &Song) -> Value {
    let p = &app.project;
    let running = lock(&ANALYSES).iter().find(|j| j.asset == s.asset && j.stream == s.stream).map(|j| (j.prog.fraction(), j.prog.status()));
    let v = p.viz_of(s.asset, s.stream);
    let state = match (&running, v, failed(&format!("song:{}", s.asset))) {
        (Some(_), _, _) => "running",
        (None, _, Some(_)) => "failed",
        (None, Some(_), _) => "done",
        _ => "none",
    };
    let made = v.map(|v| {
        let m = &v.made;
        json!({
            "visualizer": alive(p, m.visualizer),
            "template": m.template,
            "lyrics": m.lyrics.iter().filter(|&&id| p.clip(id).is_some()).count(),
            "fx": m.fx.iter().filter(|&&id| p.clip(id).is_some()).count(),
            "background": m.background.iter().filter(|&&id| p.clip(id).is_some()).count(),
            "shatter": m.shatter,
        })
    });
    let loops: Vec<Value> = lock(&LOOPS).iter().map(|j| json!({"clip": j.clip, "progress": j.prog.fraction(), "step": j.prog.status()})).collect();
    json!({
        "song": s.clip,
        "name": s.name,
        "state": state,
        "progress": running.as_ref().map(|r| r.0),
        "step": running.map(|r| r.1),
        "error": failed(&format!("song:{}", s.asset)),
        "stems": {"installed": demucs::available(), "separated": demucs::cached(Path::new(&s.path), s.stream).is_some()},
        "analysis": v.map(|v| json!({
            "bpm": v.bpm, "hits": v.hits.len(), "drops": v.drops, "calm": v.calm, "first_vocal": v.vocal_onset,
            "screeches": v.screeches.len(), "onsets": v.onsets.len(), "from_stems": v.stems, "duration": v.duration,
        })),
        "made": made,
        "loops": loops,
        "templates": keys::TEMPLATES.iter().map(|t| t.0).collect::<Vec<_>>(),
        "palettes": keys::PALETTES.iter().map(|t| t.0).collect::<Vec<_>>(),
    })
}

/// FRAME_HOOKS: land finished analyses and loops.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    let done: Vec<AnalyseJob> = {
        let mut list = lock(&ANALYSES);
        let (d, pending): (Vec<_>, Vec<_>) = std::mem::take(&mut *list).into_iter().partition(|j| j.prog.is_done());
        *list = pending;
        d
    };
    let loops: Vec<LoopJob> = {
        let mut list = lock(&LOOPS);
        let (d, pending): (Vec<_>, Vec<_>) = std::mem::take(&mut *list).into_iter().partition(|j| j.prog.is_done());
        *list = pending;
        d
    };
    let busy = !lock(&ANALYSES).is_empty() || !lock(&LOOPS).is_empty();
    for j in done {
        let r = j.out.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_else(|| Err(j.prog.error().unwrap_or_else(|| "the analysis stopped".into())));
        match r {
            Ok(v) => {
                let stems = v.stems;
                let msg = land_analysis(app, j.asset, j.stream, v);
                let tail = if stems { "" } else { " (from the mix: install Stem Separation for exact vocals and kicks)" };
                app.push_toast(Toast::new(format!("Analysed {}: {msg}{tail}", j.name)).kind(ToastKind::Success).undo(Action::Undo));
            }
            Err(e) => {
                set_failed(format!("song:{}", j.asset), Some(e.clone()));
                app.push_toast(Toast::new(format!("Analyse Song failed: {e}")).kind(ToastKind::Error));
            }
        }
    }
    for j in loops {
        let r = j.out.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_else(|| Err(j.prog.error().unwrap_or_else(|| "the loop stopped".into())));
        match r.and_then(|a| land_loop(app, j.clip, a)) {
            Ok(n) => app.push_toast(Toast::new(format!("Background Loop: {} looped ({n} clips)", j.name)).kind(ToastKind::Success).undo(Action::Undo)),
            Err(e) => {
                set_failed(format!("loop:{}", j.clip), Some(e.clone()));
                app.push_toast(Toast::new(format!("Background Loop failed: {e}")).kind(ToastKind::Error));
            }
        }
    }
    if busy {
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

// ------------------------------------------------------------------ shared editing helpers

/// The library effect whose `@name` is `name` (the plugin installs Olympus Storm and Lyric Chrome).
fn library_effect(name: &str) -> Result<String, String> {
    for it in crate::engine::library::installed(crate::engine::library::Kind::Effect) {
        if let Ok(src) = std::fs::read_to_string(&it.path) {
            if crate::engine::shaderlib::parse_meta(&src).name.eq_ignore_ascii_case(name.trim()) {
                return Ok(src);
            }
        }
    }
    Err(format!("The \"{name}\" effect is not in the library: turn the Visualizer Helper plugin on"))
}

/// Set a param's keys (by its header name: never an index, a wrong index once wiped 9339 waveform keys).
fn set_param(e: &mut Effect, name: &str, keys: Vec<Keyframe>) -> bool {
    let Some(i) = e.specs().iter().position(|p| p.name.eq_ignore_ascii_case(name)) else { return false };
    let Some(a) = e.params.get_mut(i) else { return false };
    a.value = keys.first().map_or(a.value, |k| k.v);
    a.keys = keys;
    true
}

fn animated(keys: &[(f64, f64, Ease)]) -> Animated {
    let mut a = Animated::new(keys.first().map_or(0.0, |k| k.1));
    a.keys = keys.iter().map(|&(t, v, ease)| Keyframe { t: (t * 1000.0).round() / 1000.0, v, ease }).collect();
    a
}

/// The video track named `name` (made at the top when missing; `below`: just under that track instead).
fn track_named(p: &mut Project, name: &str, below: Option<&str>) -> usize {
    if let Some(i) = p.tracks.iter().position(|t| t.kind == TrackKind::Video && t.name == name) {
        return i;
    }
    let i = p.add_track(TrackKind::Video);
    p.tracks[i].name = name.to_string();
    if let Some(b) = below.and_then(|b| p.tracks.iter().position(|t| t.kind == TrackKind::Video && t.name == b)) {
        if b < i {
            let t = p.tracks.remove(i);
            p.tracks.insert(b, t);
            return b;
        }
    }
    i
}

/// Put clip `c` on track `ti`.
fn push_clip(p: &mut Project, ti: usize, c: Clip) -> Id {
    let id = c.id;
    p.tracks[ti].clips.push(c);
    p.tracks[ti].sort();
    id
}

fn delete(p: &mut Project, ids: &[Id]) {
    let live: Vec<Id> = ids.iter().copied().filter(|&id| p.clip(id).is_some()).collect();
    if !live.is_empty() {
        p.delete_clips(&live, false);
    }
}

// ------------------------------------------------------------------ Visualizer Templates

fn rgb_arg(v: &Value) -> Option<[u8; 3]> {
    if let Some(s) = v.as_str() {
        let h = s.trim().trim_start_matches('#');
        if h.len() == 6 {
            let n = u32::from_str_radix(h, 16).ok()?;
            return Some([(n >> 16) as u8, (n >> 8) as u8, n as u8]);
        }
        return None;
    }
    let a = v.as_array()?;
    Some([a.first()?.as_f64()? as u8, a.get(1)?.as_f64()? as u8, a.get(2)?.as_f64()? as u8])
}

fn knobs(args: &Value) -> Result<(String, keys::Knobs), String> {
    let mut k = keys::Knobs::default();
    let template = arg_str(args, "template").unwrap_or("Olympus Storm").to_string();
    if !k.set_template(&template) {
        return Err(format!("template: {}", keys::TEMPLATES.iter().map(|t| t.0).collect::<Vec<_>>().join(", ")));
    }
    if let Some(p) = arg_str(args, "palette") {
        if !k.set_palette(p) {
            return Err(format!("palette: {}", keys::PALETTES.iter().map(|t| t.0).collect::<Vec<_>>().join(", ")));
        }
    }
    if let Some(c) = args.get("colors").and_then(|v| v.as_object()) {
        for (n, slot) in [("metal_pre", &mut k.metal_pre), ("metal_post", &mut k.metal_post), ("elec_pre", &mut k.elec_pre), ("elec_post", &mut k.elec_post)] {
            if let Some(v) = c.get(n) {
                *slot = rgb_arg(v).ok_or_else(|| format!("colors.{n}: \"#rrggbb\" or [r, g, b]"))?;
            }
        }
    }
    k.sensitivity = arg_f64(args, "sensitivity").unwrap_or(k.sensitivity).clamp(0.25, 2.0);
    k.smoothing = arg_f64(args, "smoothing").unwrap_or(k.smoothing).clamp(0.0, 1.0);
    k.harder = arg_f64(args, "harder").unwrap_or(k.harder).clamp(0.0, 3.0);
    k.spikes = arg_f64(args, "spikes").unwrap_or(k.spikes).clamp(1.0, 6.0);
    for (n, slot) in [("ball", &mut k.ball), ("wave", &mut k.wave), ("bolts", &mut k.bolts), ("beam", &mut k.beam)] {
        if let Some(b) = arg_bool(args, n) {
            *slot = b;
        }
    }
    Ok((template, k))
}

/// The song's visualizer: the one a template made, else one linked to the song.
fn find_visualizer(p: &Project, s: &Song) -> Option<Id> {
    if let Some(id) = p.viz_of(s.asset, s.stream).and_then(|v| alive(p, v.made.visualizer)) {
        return Some(id);
    }
    let link = p.clip(s.clip)?.link;
    (link != 0).then(|| p.tracks.iter().flat_map(|t| &t.clips).find(|c| c.link == link && is_visualizer(c)).map(|c| c.id)).flatten()
}

/// The visualizer's span as the song sees it: (song second at its start, length, speed).
fn vis_window(p: &Project, s: &Song, vis: Id) -> Option<(f64, f64, f64)> {
    let c = p.clip(vis)?;
    Some((s.from + (c.start - s.start) * s.speed, c.duration, s.speed))
}

/// The shatter build in `vis`'s own seconds (its shader's `u_time` is clip-local).
fn shatter_of(v: &SongViz, from: f64, len: f64, speed: f64, build: f64) -> Option<shader::Shatter> {
    shader::Shatter::of(v, from, len * speed, build).map(|sh| shader::Shatter { big: sh.big / speed, rift0: sh.rift0 / speed, ..sh })
}

/// The lyric lines a step made, as clip-local spans of `vis`.
fn lyric_spans(p: &Project, ids: &[Id], vis: Id) -> Vec<(f64, f64)> {
    let Some(v) = p.clip(vis) else { return Vec::new() };
    let mut s: Vec<(f64, f64)> = ids.iter().filter_map(|&id| p.clip(id)).map(|c| (c.start - v.start, c.start + c.duration - v.start)).collect();
    s.sort_by(|a, b| a.0.total_cmp(&b.0));
    s
}

fn template(app: &mut App, args: &Value) -> Result<Value, String> {
    let s = arg_song(app, args)?;
    let (name, k) = knobs(args)?;
    let v = analysis(&app.project, &s)?.clone();
    let effect = arg_str(args, "effect").unwrap_or("Olympus Storm").to_string();
    let lib = library_effect(&effect)?;
    let backend = app.backend();
    let vis = match find_visualizer(&app.project, &s) {
        Some(id) => id,
        None => {
            let style = crate::engine::visualizer::Style { name: effect.clone(), src: lib.clone(), custom: true };
            crate::engine::visualizer::add(&mut app.project, s.clip, &style, backend)?
        }
    };
    let (from, len, speed) = vis_window(&app.project, &s, vis).ok_or("the visualizer vanished")?;
    let sh = (v.made.shatter > 0.0).then(|| shatter_of(&v, from, len, speed, v.made.shatter)).flatten();
    let src = shader::specialize(&lib, &k, sh);
    // the same style: swap the source only (params keep their places, the baked audio stays); another
    // style: restyle, which re-bakes the audio under it
    let same = app.project.clip(vis).and_then(|c| c.effects.iter().find_map(crate::engine::visualizer::style_of)).is_some_and(|n| n.eq_ignore_ascii_case(&effect));
    if same {
        let c = app.project.clip_mut(vis).ok_or("no visualizer")?;
        let e = c.effects.iter_mut().find(|e| crate::engine::visualizer::style_of(e).is_some()).ok_or("no visualizer effect")?;
        e.set_shader(&src);
    } else {
        let style = crate::engine::visualizer::Style { name: effect.clone(), src: src.clone(), custom: true };
        crate::engine::visualizer::restyle(&mut app.project, vis, &style, backend)?;
    }
    let lyric = keys::lyric_keys(&lyric_spans(&app.project, &v.made.lyrics, vis));
    let c = app.project.clip_mut(vis).ok_or("no visualizer")?;
    c.name = name.clone();
    let e = c.effects.iter_mut().find(|e| crate::engine::visualizer::style_of(e).is_some()).ok_or("no visualizer effect")?;
    let mut counts = serde_json::Map::new();
    for (pname, ks) in keys::song_keys(&v, &k) {
        let w = keys::window(&ks, from, len, speed);
        let n = w.len();
        if set_param(e, pname, w) {
            counts.insert(pname.into(), json!(n));
        }
    }
    if set_param(e, "lyric", lyric) {
        counts.insert("lyric".into(), json!(true));
    }
    if let Some(r) = app.project.viz_of_mut(s.asset, s.stream) {
        r.made.visualizer = vis;
        r.made.template = name.clone();
    }
    Ok(json!({"ok": true, "clip_id": vis, "template": name, "keys": counts}))
}

// ------------------------------------------------------------------ Lyrics Karaoke

/// The app's transcript of the song clip, in song seconds (empty: none).
fn transcript(p: &Project, s: &Song) -> Vec<lyrics::Said> {
    p.transcripts
        .iter()
        .find(|t| t.clip == s.clip)
        .map(|t| t.words.iter().map(|(a, _, w)| lyrics::Said { text: w.clone(), start: s.from + (a - s.start) * s.speed }).collect())
        .unwrap_or_default()
}

fn karaoke(app: &mut App, args: &Value) -> Result<Value, String> {
    let s = arg_song(app, args)?;
    let v = analysis(&app.project, &s)?.clone();
    let text = req(arg_str(args, "lyrics"), "lyrics")?;
    let (lines, drop_before) = lyrics::parse(text);
    if lines.is_empty() {
        return Err("Paste the lyrics first (one on-screen line per line)".into());
    }
    let font = arg_str(args, "font").filter(|f| !f.trim().is_empty()).unwrap_or("Palatino Linotype").to_string();
    let size = arg_f64(args, "size").unwrap_or(58.0).clamp(8.0, 400.0);
    let greek = arg_str(args, "look").is_some_and(|l| l.eq_ignore_ascii_case("greek"));
    let giant_on = arg_bool(args, "giant").unwrap_or(true);
    let src = library_effect(arg_str(args, "effect").unwrap_or("Lyric Chrome"))?;
    let (pw, ph) = (app.project.width.max(1) as f64, app.project.height.max(1) as f64);
    let y = arg_f64(args, "y").unwrap_or(330.0 * ph / 1080.0);
    let timed = lyrics::time(&lines, &v, &transcript(&app.project, &s));
    // replace what this step made before
    let old = v.made.lyrics.clone();
    delete(&mut app.project, &old);
    let ti = track_named(&mut app.project, "Lyrics", Some("Drop FX"));
    let mut ids = Vec::new();
    let text_arc = app.text.clone();
    let mut tr = text_arc.lock().unwrap_or_else(|e| e.into_inner());
    for line in &timed {
        let giant = giant_on && line.giant;
        let shown = if greek { lyrics::greek(&line.text) } else { line.text.clone() };
        let at = s.timeline(line.clip_start);
        let d = (line.clip_end - line.clip_start) / s.speed;
        if d < 0.05 || at + d <= s.start || at >= s.start + s.len {
            continue;
        }
        // pristine text: the font is rasterised at its final size (with the shader's margin) and the clip
        // only ever scales DOWN - a scale above 1 would stretch the bitmap
        let m = if giant { 2.3 * 1.25 } else { 1.25 };
        let mut c = Clip::new(app.project.new_id(), ClipKind::Text, shown.clone(), at, d);
        let mut st = c.text.take().unwrap_or_default();
        st.text = shown.clone();
        st.font = font.clone();
        st.size = Animated::new(((if giant { size * 1.8 } else { size }) * m).round());
        st.color = [255, 255, 255, 255];
        st.outline_width = Animated::new(0.0);
        st.shadow = false;
        st.box_color[3] = 0;
        let (edges, w) = tr.char_edges(&st).unwrap_or_else(|| ((0..=shown.chars().count()).map(|i| i as f32).collect(), shown.chars().count().max(1) as f32));
        c.text = Some(st);
        c.y = Animated::new(if giant { 0.0 } else { y });
        let mut e = Effect::from_shader(&src);
        let mut lt = line.clone();
        // times relative to the clip (song seconds at the song's speed)
        for t in lt.starts.iter_mut().chain(lt.ends.iter_mut()) {
            *t = line.clip_start + (*t - line.clip_start) / s.speed;
        }
        let fill: Vec<(f64, f64, Ease)> = lyrics::fill_keys(&lt, &shown, &edges, w).into_iter().map(|(t, v)| (t, v, Ease::Linear)).collect();
        set_param(&mut e, "fill", animated(&fill).keys);
        set_param(&mut e, "gold", vec![Keyframe { t: 0.0, v: line.gold as u8 as f64, ease: Ease::Hold }]);
        let io = (0.22 * d).min(0.4);
        let sx = 1.08 / m;
        c.scale_y = Animated::new(1.0);
        c.scale_x = Animated::new(sx);
        if giant {
            set_param(&mut e, "curve", vec![Keyframe { t: 0.0, v: 0.4, ease: Ease::Linear }]);
            set_param(&mut e, "roll", animated(&[(0.0, 1.0, Ease::EaseOut), (io, 0.025, Ease::Linear), (d - io, -0.025, Ease::EaseIn), (d, -1.0, Ease::Linear)]).keys);
        } else {
            // pulled on from the right, slowing through the middle, drifting off left; turned in 3D to face the
            // centre; a motion-blur smear while it moves fast. No vertical animation at all.
            let g = (40.0 * d).min(70.0) * pw / 1920.0;
            let (x_in, x_out) = (1500.0 * pw / 1920.0, -450.0 * pw / 1920.0);
            c.x = animated(&[(0.0, x_in, Ease::EaseOut), (io, g, Ease::Linear), (d - io, -g, Ease::EaseIn), (d, x_out, Ease::Linear)]);
            let gy = g / x_in.max(1.0);
            set_param(&mut e, "yaw", animated(&[(0.0, -1.0, Ease::EaseOut), (io, -gy, Ease::Linear), (d - io, gy, Ease::EaseIn), (d, 0.6, Ease::Linear)]).keys);
            set_param(&mut e, "smear", animated(&[(0.0, 1.0, Ease::EaseOut), (io, 0.0, Ease::Linear), (d - io, 0.0, Ease::EaseIn), (d, 0.45, Ease::Linear)]).keys);
        }
        c.opacity = animated(&[(0.0, 0.0, Ease::EaseOut), (io * 0.4, 1.0, Ease::Linear), ((d - 0.3).max(d - io), 1.0, Ease::EaseIn), (d, 0.0, Ease::Linear)]);
        c.effects.push(e);
        ids.push(push_clip(&mut app.project, ti, c));
    }
    drop(tr);
    // the visualizer's waveform flattens under each line
    let vis = find_visualizer(&app.project, &s);
    if let Some(vis) = vis {
        let spans = lyric_spans(&app.project, &ids, vis);
        if let Some(e) = app.project.clip_mut(vis).and_then(|c| c.effects.iter_mut().find(|e| crate::engine::visualizer::style_of(e).is_some())) {
            set_param(e, "lyric", keys::lyric_keys(&spans));
        }
    }
    let drop_after = drop_before.and_then(|i| timed.get(i.checked_sub(1)?)).map(|l| l.clip_end);
    if let Some(r) = app.project.viz_of_mut(s.asset, s.stream) {
        r.made.lyrics = ids.clone();
    }
    Ok(json!({
        "ok": true,
        "clips": ids,
        "lines": timed.iter().map(|l| json!({"text": l.text, "start": l.clip_start, "end": l.clip_end, "words": l.starts, "gold": l.gold, "giant": l.giant})).collect::<Vec<_>>(),
        "transcript": !transcript(&app.project, &s).is_empty(),
        // the lyrics say where the drop is: Analyse Song can take it as `after`
        "drop_after": drop_after,
    }))
}

// ------------------------------------------------------------------ Drop FX

fn drop_fx(app: &mut App, args: &Value) -> Result<Value, String> {
    let s = arg_song(app, args)?;
    let v = analysis(&app.project, &s)?.clone();
    if v.drops.is_empty() {
        return Err(format!("No drops were found in {}: give their times to Analyse Song", s.name));
    }
    let (flash, invert, shatter, grade) = (arg_bool(args, "flash").unwrap_or(true), arg_bool(args, "invert").unwrap_or(true), arg_bool(args, "shatter").unwrap_or(true), arg_bool(args, "grade").unwrap_or(true));
    let build = arg_f64(args, "build").unwrap_or(12.7).clamp(1.0, 60.0);
    delete(&mut app.project, &v.made.fx);
    let ti = track_named(&mut app.project, "Drop FX", None);
    let beat = if v.bpm > 1.0 { 60.0 / v.bpm } else { 0.4 };
    let mut ids = Vec::new();
    // a flash and a one-beat colour invert at each drop: one Adjustment clip (Color Correction + Invert)
    if flash || invert {
        for &d in &v.drops {
            let at = s.timeline(d as f64);
            if at < s.start || at >= s.start + s.len {
                continue;
            }
            let len = (beat + 0.01).max(0.45);
            let mut c = Clip::new(app.project.new_id(), ClipKind::Adjustment, "Drop FX", at, len);
            if flash {
                let mut e = Effect::new(EffectKind::Color);
                set_param(&mut e, "Brightness", animated(&[(0.0, 0.9, Ease::EaseOut), (0.4, 0.0, Ease::Linear)]).keys);
                c.effects.push(e);
            }
            if invert {
                let mut e = Effect::new(EffectKind::Invert);
                set_param(&mut e, "Amount", animated(&[(0.0, 1.0, Ease::Hold), (beat, 0.0, Ease::Hold)]).keys);
                c.effects.push(e);
            }
            ids.push(push_clip(&mut app.project, ti, c));
        }
    }
    // the shatter into the biggest drop: the visualizer's build, and the shard layer inverting what is below
    let vis = find_visualizer(&app.project, &s);
    let mut shatter_note = None;
    let mut used_build = 0.0;
    if let Some(vis) = vis {
        let (from, len, speed) = vis_window(&app.project, &s, vis).ok_or("the visualizer vanished")?;
        let sh = shatter.then(|| shatter_of(&v, from, len, speed, build)).flatten();
        let c = app.project.clip(vis).ok_or("no visualizer")?.clone();
        let e = c.effects.iter().find(|e| crate::engine::visualizer::style_of(e).is_some()).ok_or("no visualizer effect")?;
        let is_storm = e.shader.contains("const float BIGDROP");
        if is_storm {
            let src = shader::with_shatter(&e.shader, sh);
            if let Some(e) = app.project.clip_mut(vis).and_then(|c| c.effects.iter_mut().find(|e| crate::engine::visualizer::style_of(e).is_some())) {
                e.set_shader(&src);
            }
            if let Some(sh) = sh {
                used_build = build;
                let mask = shader::invert_layer(&src)?;
                // a copy of the visualizer's clip (its shape size: the shards must line up), on top, from the
                // build to the drop, Difference blend, keys shifted to its start
                let span = sh.big - sh.rift0;
                let mut layer = c.clone();
                layer.id = app.project.new_id();
                layer.link = 0;
                layer.name = "Shatter".into();
                layer.start = c.start + sh.rift0;
                layer.duration = span;
                layer.blend = BlendMode::Difference;
                let mut le = e.clone();
                le.set_shader(&mask);
                for a in le.params.iter_mut() {
                    if !a.keys.is_empty() {
                        a.keys = keys::window(&a.keys, sh.rift0, span, 1.0);
                    }
                }
                layer.effects = vec![le];
                ids.push(push_clip(&mut app.project, ti, layer));
            }
        } else if shatter {
            shatter_note = Some("the shatter needs the Olympus Storm template on the visualizer");
        }
    } else if shatter {
        shatter_note = Some("the shatter needs a visualizer: run Visualizer Templates first");
    }
    // the background grade: black until the first drop, a bright golden-hour look from it, a gold-only duotone
    // through the biggest drop until the song calms down
    let mut graded = 0;
    if grade {
        let bg: Vec<Id> = v.made.background.iter().copied().filter(|&id| app.project.clip(id).is_some()).collect();
        let lut = crate::engine::library::installed(crate::engine::library::Kind::Lut).into_iter().find(|it| it.id.ends_with("golden-hour")).map(|it| it.path.to_string_lossy().into_owned());
        let d1 = v.drops[0] as f64;
        let big = v.big_drop().unwrap_or(d1);
        let two = v.drops.len() > 1;
        let calm = v.calm.map_or(v.duration, |c| c as f64);
        for id in bg {
            let Some(c) = app.project.clip(id).cloned() else { continue };
            // song seconds -> this clip's local seconds
            let local = |t: f64| s.timeline(t) - c.start;
            let mut c2 = c.clone();
            c2.effects.retain(|e| !(matches!(e.kind, EffectKind::Levels | EffectKind::Color | EffectKind::Tint) || (e.kind == EffectKind::Lut && lut.as_deref() == Some(e.lut.as_str()))));
            if let Some(path) = &lut {
                let mut e = Effect::new(EffectKind::Lut);
                e.lut = path.clone();
                c2.effects.push(e);
            }
            let mut lv = Effect::new(EffectKind::Levels);
            let mut cc = Effect::new(EffectKind::Color);
            let mut tint = Effect::new(EffectKind::Tint);
            for (n, val) in [("Red", 255.0), ("Green", 176.0), ("Blue", 40.0)] {
                set_param(&mut tint, n, vec![Keyframe { t: 0.0, v: val, ease: Ease::Hold }]);
            }
            set_param(&mut cc, "Brightness", animated(&[(0.0, -0.45, Ease::Linear), (local(d1) - 0.02, -0.12, Ease::Linear), (local(d1), 0.06, Ease::Linear)]).keys);
            if two {
                set_param(&mut lv, "In white", animated(&[(0.0, 0.44, Ease::Hold), (local(big), 0.85, Ease::Hold), (local(calm), 0.44, Ease::Hold)]).keys);
                set_param(&mut cc, "Saturation", animated(&[(0.0, 1.0, Ease::Hold), (local(big), 0.1, Ease::Hold), (local(calm), 1.0, Ease::Hold)]).keys);
                set_param(&mut cc, "Contrast", animated(&[(0.0, 1.0, Ease::Hold), (local(big), 1.2, Ease::Hold), (local(calm), 1.0, Ease::Hold)]).keys);
                set_param(&mut tint, "Amount", animated(&[(0.0, 0.0, Ease::Hold), (local(big), 0.72, Ease::Hold), (local(calm), 0.0, Ease::Hold)]).keys);
            } else {
                set_param(&mut lv, "In white", vec![Keyframe { t: 0.0, v: 0.44, ease: Ease::Hold }]);
                set_param(&mut tint, "Amount", vec![Keyframe { t: 0.0, v: 0.0, ease: Ease::Hold }]);
            }
            c2.effects.extend([lv, cc, tint]);
            // black until the first drop (one drop: the scenery creeps in dark under the shatter)
            c2.opacity = if two || used_build <= 0.0 {
                animated(&[(0.0, 0.0, Ease::Hold), (local(d1), 1.0, Ease::Hold)])
            } else {
                let r0 = (big - used_build).max(0.0);
                animated(&[(0.0, 0.0, Ease::Hold), (local(r0), 0.0, Ease::EaseIn), (local(big) - 0.02, 0.75, Ease::Hold), (local(big), 1.0, Ease::Hold)])
            };
            c2.opacity.keys.retain(|k| k.t >= 0.0);
            if let Some(slot) = app.project.clip_mut(id) {
                *slot = c2;
                graded += 1;
            }
        }
    }
    if let Some(r) = app.project.viz_of_mut(s.asset, s.stream) {
        r.made.fx = ids.clone();
        r.made.shatter = used_build;
    }
    Ok(json!({"ok": true, "clips": ids, "drops": v.drops, "shatter": used_build > 0.0, "note": shatter_note, "graded": graded}))
}

// ------------------------------------------------------------------ Background Loop

fn loop_dir() -> PathBuf {
    crate::settings::Settings::dir().join("Visualizer Helper")
}

fn start_loop(app: &mut App, args: &Value) -> Result<(), String> {
    let id = arg_u64(args, "clip").or_else(|| app.selection.iter().copied().find(|&id| app.project.clip(id).is_some_and(|c| matches!(c.kind, ClipKind::Video | ClipKind::Image))));
    let id = id.ok_or("Select the GIF, picture or clip to loop")?;
    let c = app.project.clip(id).ok_or("no such clip")?;
    if !matches!(c.kind, ClipKind::Video | ClipKind::Image) {
        return Err("Background Loop needs a GIF, picture or video clip".into());
    }
    if lock(&LOOPS).iter().any(|j| j.clip == id) {
        return Ok(());
    }
    let a = app.project.asset(c.asset).ok_or("the clip has no media")?;
    let ff = crate::media::ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let input = a.path.clone();
    let still = c.kind == ClipKind::Image || a.duration <= 0.0 || a.fps <= 0.0;
    let src_secs = a.duration.max(0.05);
    let seconds = arg_f64(args, "seconds").unwrap_or(8.0).clamp(1.0, 60.0);
    let (w, h) = (app.project.width, app.project.height);
    let name = Path::new(&input).file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let dir = loop_dir();
    let stem = format!("{}-loop-{}s-{}x{}", crate::engine::library::slug(&name), seconds as u32, w, h);
    let (one, out) = (dir.join(format!("{stem}.part.mp4")), dir.join(format!("{stem}.mp4")));
    let backend = app.backend();
    set_failed(format!("loop:{id}"), None);
    let slot: Slot<crate::model::Asset> = Arc::new(Mutex::new(None));
    let o2 = slot.clone();
    let prog = crate::engine::export::spawn_job("vizhelp-loop", move |p| {
        let r = (|| {
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let run = |args: Vec<String>, what: &str| -> Result<(), String> {
                p.set(if what.starts_with("Smoothing") { 0.05 } else { 0.7 }, what.to_string());
                let o = crate::media::ffpipe::command(&ff).args(&args).stdin(std::process::Stdio::null()).output().map_err(|e| format!("ffmpeg: {e}"))?;
                if o.status.success() {
                    Ok(())
                } else {
                    Err(String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("ffmpeg failed").to_string())
                }
            };
            let (os, ot) = (one.to_string_lossy().into_owned(), out.to_string_lossy().into_owned());
            if !out.is_file() {
                run(loopbg::pass1(&input, &os, still, src_secs, seconds, w, h), "Smoothing to 60 fps")?;
                run(loopbg::pass2(&os, &ot), "Making it loop")?;
                let _ = std::fs::remove_file(&one);
            }
            crate::media::probe(&ot, backend)
        })();
        let err = r.as_ref().err().cloned();
        *o2.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
        err.map_or(Ok(()), Err)
    });
    lock(&LOOPS).push(LoopJob { clip: id, name, prog, out: slot });
    Ok(())
}

/// Swap the looped clip for the loop, tiled back to back under the song (at least the clip's own length).
/// One labelled undo step. Returns how many clips it made.
fn land_loop(app: &mut App, clip: Id, asset: crate::model::Asset) -> Result<usize, String> {
    let c = app.project.clip(clip).cloned().ok_or("the clip to loop was deleted")?;
    let ti = app.project.track_of(clip).ok_or("the clip to loop was deleted")?;
    let before = app.project.to_json();
    let len = asset.duration.max(0.5);
    let aid = app.project.add_asset(asset);
    // under the song this background belongs with (an analysed song on the timeline), else its own length
    let s = song(&app.project, None, &app.selection).ok().filter(|s| app.project.viz_of(s.asset, s.stream).is_some());
    let end = s.as_ref().map_or(c.start + c.duration, |s| (s.start + s.len).max(c.start + c.duration));
    let mut keep = c.clone();
    keep.kind = ClipKind::Video;
    keep.asset = aid;
    keep.src_in = 0.0;
    keep.speed = 1.0;
    keep.freeze = None;
    keep.link = 0;
    app.project.delete_clips(&[clip], false);
    let mut ids = Vec::new();
    let mut t = c.start;
    while t < end - 0.05 {
        let mut k = keep.clone();
        k.id = app.project.new_id();
        k.start = t;
        k.duration = len.min(end - t);
        ids.push(push_clip(&mut app.project, ti, k));
        t += len;
    }
    if let Some(s) = s {
        if let Some(r) = app.project.viz_of_mut(s.asset, s.stream) {
            r.made.background = ids.clone();
        }
    }
    app.push_undo_labeled(before, "Background Loop");
    app.after_edit();
    Ok(ids.len())
}

fn loop_status(app: &App, args: &Value) -> Value {
    let id = arg_u64(args, "clip");
    let jobs: Vec<Value> = lock(&LOOPS).iter().filter(|j| id.is_none_or(|i| i == j.clip)).map(|j| json!({"clip": j.clip, "name": j.name, "progress": j.prog.fraction(), "step": j.prog.status()})).collect();
    let made: Vec<Id> = app.project.viz.iter().flat_map(|v| v.made.background.iter().copied()).filter(|&c| app.project.clip(c).is_some()).collect();
    json!({"running": jobs, "error": id.and_then(|i| failed(&format!("loop:{i}"))), "clips": made})
}

// ------------------------------------------------------------------ the tools

pub(crate) const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "audio.analyze_song",
        desc: "Visualizer Helper ▸ Analyse Song. action=start analyses a song clip in the background: stems through the Stem Separation add-on when installed (cached stems reused; else the mix alone), hits with strengths and downbeats (tempo search tempo_min..tempo_max BPM, kick gate `gate` dB), drops (or the `drops` you give, none before `after` s), the calm part, screeches, the vocal envelope, vibrato and syllable onsets. It lands on the project (one undo step) with markers at the drops, the calm part and the first vocal. action=status (default): {state none|running|done|failed, progress, step, analysis, made, stems}. action=stems opens Settings ▸ Add-ons (Stem Separation). clip: an audio clip, a video with sound or a visualizer; default the selection's song, else the analysed one, else the longest audio clip.",
        args: &[
            "action:string:false:status (default) | start | stems",
            "clip:integer:false:the song clip",
            "tempo_min:number:false:BPM, default 130",
            "tempo_max:number:false:BPM, default 175",
            "gate:number:false:kick gate dB, default 18 (higher keeps quieter kicks)",
            "drops:array:false:drop times (song seconds) to use instead of detecting",
            "after:number:false:no drop before this song second",
        ],
        kind: ToolKind::Read,
        run: |app, args| {
            match arg_str(args, "action").unwrap_or("status") {
                "stems" => {
                    app.settings_ui.open = true;
                    app.settings_ui.tab = crate::ui::settings_ui::ADDONS_TAB;
                    return Ok(ToolOutcome::Done(json!({"ok": true})));
                }
                "start" => {
                    let s = arg_song(app, args)?;
                    start_analysis(app, &s, opts(args))?;
                    Ok(ToolOutcome::Done(status_json(app, &s)))
                }
                "status" => {
                    let s = arg_song(app, args)?;
                    Ok(ToolOutcome::Done(status_json(app, &s)))
                }
                _ => Err("action: status, start or stems".into()),
            }
        },
    },
    ToolDef {
        name: "visualizer.template",
        desc: "Visualizer Helper ▸ Visualizer Templates: give a song's visualizer (made when it has none) a template keyed from its audio.analyze_song result. template: Olympus Storm (spiky chrome ball, waveform, lightning, beams) | Chrome Waveform | Storm Lightning. Knobs: palette (Silver to Gold | Ice to Fire | Neon | Mono) or colors {metal_pre, metal_post, elec_pre, elec_post: \"#rrggbb\" or [r,g,b]}, spikes 1..6 (spines per hit), sensitivity 0.25..2 (how readily hits pop), smoothing 0..1 (voice and synth curves), harder 0..3 (kick level from the second drop to the calm part, 1.2 = no change), ball / wave / bolts / beam on-off. Writes named keys (kclock, kpow, sclock, vox, vib, kick, screech, fx, beam, lyric); the baked level / band / wave keys stay. Same inputs, same keys.",
        args: &[
            "clip:integer:false:the song (or its visualizer); default as audio.analyze_song",
            "template:string:false:Olympus Storm (default) | Chrome Waveform | Storm Lightning",
            "palette:string:false:colour set",
            "colors:object:false:{metal_pre, metal_post, elec_pre, elec_post}",
            "spikes:number:false:1..6, default 4",
            "sensitivity:number:false:0.25..2, default 1",
            "smoothing:number:false:0..1, default 0",
            "harder:number:false:0..3, default 2.2",
            "ball:boolean:false:",
            "wave:boolean:false:",
            "bolts:boolean:false:",
            "beam:boolean:false:",
            "effect:string:false:the library effect to use, default Olympus Storm",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| template(app, args).map(ToolOutcome::Done),
    },
    ToolDef {
        name: "lyrics.karaoke",
        desc: "Visualizer Helper ▸ Lyrics Karaoke: the exact lyrics (one on-screen line per line; [SECTION] headers and blank lines skipped, a [DROP] header marks the drop) as timed text clips on a \"Lyrics\" track, each word filling silver (gold from the first drop) on its sung syllable: words are put on the vocal stem's onsets (anchored on the song's transcript when it has one). Lines slide in from the right with a 3D turn and motion blur, appear 0.35 s before their first word and leave when the voice stops; the line ending into the biggest drop wraps round a cylinder (giant). Text size comes from the font size (the clip only scales down). Uses the Lyric Chrome library effect. Replaces the lines it made before; flattens the visualizer's waveform under each line. Needs audio.analyze_song first.",
        args: &[
            "lyrics:string:true:the lyrics, exactly as sung",
            "clip:integer:false:the song; default as audio.analyze_song",
            "font:string:false:default Palatino Linotype",
            "size:number:false:font size px, default 58",
            "y:number:false:vertical offset px (default 330 at 1080p)",
            "look:string:false:normal (default) | greek (A→Λ E→Σ O→Θ Y→Ψ)",
            "giant:boolean:false:the cylinder line into the biggest drop, default true",
            "effect:string:false:the library effect, default Lyric Chrome",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| karaoke(app, args).map(ToolOutcome::Done),
    },
    ToolDef {
        name: "visualizer.drop_fx",
        desc: "Visualizer Helper ▸ Drop FX at the drops audio.analyze_song found, on a \"Drop FX\" track on top: flash (a brightness flash) and invert (the Invert effect for one beat) as one Adjustment clip per drop; shatter: the Olympus Storm visualizer gets sucked in for `build` s (default 12.7) before the biggest drop while glass shards invert on every hit (a Difference-blend layer cut from the same shader); grade: the Background Loop clips get the golden-hour LUT, black until the first drop and a gold duotone through the biggest drop. Replaces the clips it made before.",
        args: &[
            "clip:integer:false:the song; default as audio.analyze_song",
            "flash:boolean:false:default true",
            "invert:boolean:false:default true",
            "shatter:boolean:false:default true",
            "build:number:false:shatter build seconds, default 12.7",
            "grade:boolean:false:default true",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| drop_fx(app, args).map(ToolOutcome::Done),
    },
    ToolDef {
        name: "media.loop",
        desc: "Visualizer Helper ▸ Background Loop. action=start turns a low-fps GIF, a picture or a clip into a seamless 60 fps loop at the project size (ffmpeg minterpolate, then forward + reversed so the loop point never jumps; a still gets a slow push-in), `seconds` one way (default 8). When ready it replaces the clip, tiled under the analysed song (one undo step). action=status (default): running loops, the last error, the loop clips.",
        args: &[
            "action:string:false:status (default) | start",
            "clip:integer:false:the GIF / picture / video clip; default the selected one",
            "seconds:number:false:one-way length, default 8",
        ],
        kind: ToolKind::Read,
        run: |app, args| match arg_str(args, "action").unwrap_or("status") {
            "start" => {
                start_loop(app, args)?;
                Ok(ToolOutcome::Done(loop_status(app, args)))
            }
            "status" => Ok(ToolOutcome::Done(loop_status(app, args))),
            _ => Err("action: status or start".into()),
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visualizer_helper_tools_are_callable_from_plugins() {
        for t in TOOLS {
            assert!(matches!(t.kind, ToolKind::Read | ToolKind::Mutate), "{}", t.name);
            assert!(script_ctl::script_may_call(t.name), "{} must be callable from a plugin", t.name);
        }
    }

    #[test]
    fn the_song_is_the_selection_then_the_longest_audio_clip() {
        let mut p = Project::new();
        p.assets.push(crate::model::Asset {
            id: 1,
            path: "a.wav".into(),
            kind: ClipKind::Audio,
            duration: 60.0,
            width: 0,
            height: 0,
            fps: 0.0,
            audio_streams: vec![],
            codec: String::new(),
            folder: String::new(),
            tags: vec![],
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            proxy: None,
            effects: vec![],
            html: None,
        });
        let ti = p.tracks.iter().position(|t| t.kind == TrackKind::Audio).unwrap();
        for (id, start, dur) in [(10, 0.0, 5.0), (11, 10.0, 30.0)] {
            let mut c = Clip::new(id, ClipKind::Audio, "song", start, dur);
            c.asset = 1;
            c.src_in = 2.0;
            p.tracks[ti].clips.push(c);
        }
        p.next_id = 100;
        assert_eq!(song(&p, None, &[]).unwrap().clip, 11);
        assert_eq!(song(&p, None, &[10]).unwrap().clip, 10);
        let s = song(&p, Some(11), &[]).unwrap();
        assert_eq!(s.timeline(s.from + 3.0), 13.0);
        assert!(song(&Project::new(), None, &[]).is_err());
    }
}
