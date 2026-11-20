//! ---- ws:parity-audio ----
//! Shotcut-parity audio, App side:
//! - MCP tools over a clip's own filters (`Clip.audio_filters`) and a track's auto-fade / seams;
//! - the two-pass Normalize's analysis pass (a filter's Analyze button or `audio.normalize_loudness`): a worker
//!   measures the clip, `tick` writes "Measured LUFS" back as one undo step;
//! - the tone generator (Clip ▸ Add ▸ Tone Generator… / `audio.tone`): a worker writes the WAV, `tick`
//!   adds it to the Library and places it at the playhead;
//! - the Audio Scopes pane (`draw`, a PANE_DRAWERS entry).
//! UI-thread state lives in a thread-local, not on `App`.

use super::feedback::{Toast, ToastKind};
use super::tools_args::Args;
use super::tools_helpers::req;
use super::*;
use crate::engine::audio_tools::{self, Scope, Wave, SCOPE_FEED, SCOPE_FFT};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::{AudioFilter, FilterKind};
use std::cell::RefCell;
use std::sync::Mutex;

/// A running loudness analysis: the clip, the Normalize filter's index, the worker and its answer.
struct Analysis {
    clip: Id,
    index: usize,
    prog: Arc<Progress>,
    out: Arc<Mutex<Option<f32>>>,
}

/// A tone being written: the asset to add once the file is there, and where to place it.
struct ToneJob {
    prog: Arc<Progress>,
    asset: crate::model::Asset,
    at: f64,
}

/// The tone generator window's fields.
struct ToneForm {
    open: bool,
    wave: Wave,
    hz: f64,
    level_db: f64,
    secs: f64,
}

impl Default for ToneForm {
    fn default() -> Self {
        Self { open: false, wave: Wave::Sine, hz: 1000.0, level_db: -18.0, secs: 10.0 }
    }
}

#[derive(Default)]
struct State {
    analyses: Vec<Analysis>,
    tones: Vec<ToneJob>,
    form: ToneForm,
    scope: Scope,
    frames: Vec<f32>,
    /// `SCOPE_FEED.total()` the scope was last computed at, and when.
    seen: u64,
    at: Option<std::time::Instant>,
    was_playing: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

// ---------- two-pass normalize ----------

/// Start measuring `clip` for its Normalize filter at `index` (through the filters before it). Err when
/// the clip has no audio file of its own to read.
fn start_analysis(app: &mut App, clip: Id, index: usize) -> Result<(), String> {
    let c = app.project.clip(clip).ok_or("no such clip")?.clone();
    if c.audio_filters.get(index).map(|f| f.kind) != Some(FilterKind::Normalize) {
        return Err("that filter is not a Normalize".into());
    }
    if c.kind != crate::model::ClipKind::Audio {
        return Err("Analyze reads an audio clip's file (use it on the audio clip, not a nested sequence)".into());
    }
    let path = app.project.asset(c.asset).map(|a| a.path.clone()).ok_or("the clip's media is missing")?;
    let backend = app.backend();
    let out: Arc<Mutex<Option<f32>>> = Arc::default();
    let sink = out.clone();
    let prog = crate::engine::export::spawn_job("loudness", move |prog| {
        prog.set(0.1, "Measuring loudness…");
        let mut src = crate::media::open_audio(&path, c.audio_stream, backend)?;
        let v = audio_tools::measure_lufs(src.as_mut(), &c, index, &|| prog.is_cancelled())
            .ok_or("the clip is silent (nothing to measure)")?;
        *sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
        prog.set(1.0, "Done");
        Ok(())
    });
    let mut t = Toast::new("Measuring loudness…");
    t.progress = Some(prog.clone());
    app.push_toast(t);
    STATE.with(|s| s.borrow_mut().analyses.push(Analysis { clip, index, prog, out }));
    Ok(())
}

/// Ensure each clip ends its chain with a Normalize filter aiming at `target` (an existing one is
/// retargeted) and start its analysis. Returns how many started.
fn normalize_clips(app: &mut App, ids: &[Id], target: f64) -> Result<usize, String> {
    let mut started = 0;
    for &id in ids {
        let Some(c) = app.project.clip_mut(id) else { continue };
        let index = match c.audio_filters.iter().position(|f| f.kind == FilterKind::Normalize) {
            Some(i) => i,
            None => {
                c.audio_filters.push(AudioFilter::new(FilterKind::Normalize));
                c.audio_filters.len() - 1
            }
        };
        c.audio_filters[index].fill_params();
        c.audio_filters[index].params[0].value = target.clamp(-40.0, -5.0);
        start_analysis(app, id, index)?;
        started += 1;
    }
    Ok(started)
}

fn finish_analysis(app: &mut App, a: Analysis) {
    if let Some(e) = a.prog.error() {
        app.push_toast(Toast::new(format!("Loudness analysis failed: {e}")).kind(ToastKind::Error));
        return;
    }
    let Some(v) = a.out.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    let ok = app
        .project
        .clip(a.clip)
        .and_then(|c| c.audio_filters.get(a.index))
        .is_some_and(|f| f.kind == FilterKind::Normalize);
    if !ok {
        return; // the clip or its filter went while it was measured
    }
    let before = app.project.to_json();
    let mut gain = 0.0;
    if let Some(f) = app.project.clip_mut(a.clip).and_then(|c| c.audio_filters.get_mut(a.index)) {
        f.fill_params();
        f.params[1] = crate::model::Animated::new((v as f64).clamp(-70.0, -0.1));
        gain = crate::engine::mixer_fx::normalize_gain_db(f.at(0, 0.0), f.at(1, 0.0), f.at(2, 0.0));
    }
    app.push_undo_labeled(before, "Analyze Loudness");
    app.after_edit();
    app.push_toast(Toast::new(format!("Measured {v:.1} LUFS: {gain:+.1} dB applied")).kind(ToastKind::Success));
}

// ---------- tone generator ----------

/// Folder generated tones are written to (kept: projects point at them).
fn tone_dir() -> PathBuf {
    crate::settings::Settings::dir().join("generated")
}

/// Write a tone on a worker and place it at `at` when written. The asset is known up front (a 48 kHz
/// stereo WAV of an exact length), so nothing is probed.
fn start_tone(app: &mut App, wave: Wave, hz: f64, level_db: f64, secs: f64, at: f64) -> Result<(Arc<Progress>, PathBuf), String> {
    if !(secs > 0.0) {
        return Err("the length must be above 0 seconds".into());
    }
    let secs = secs.min(audio_tools::TONE_MAX_S);
    let (_, name) = audio_tools::tone_wav(wave, hz, level_db, 0.0);
    let path = tone_dir().join(format!("{name} {}.wav", crate::settings::Settings::now()));
    let mut asset = crate::engine::import::placeholder(&path.to_string_lossy());
    asset.kind = crate::model::ClipKind::Audio;
    asset.duration = (secs * crate::media::SAMPLE_RATE as f64).floor() / crate::media::SAMPLE_RATE as f64;
    asset.audio_streams = vec![crate::model::AudioStreamInfo {
        index: 0,
        channels: 2,
        sample_rate: crate::media::SAMPLE_RATE,
        codec: "pcm_s16le".into(),
        ..Default::default()
    }];
    asset.codec = "pcm_s16le".into();
    let out = path.clone();
    let prog = crate::engine::export::spawn_job("tone", move |prog| {
        prog.set(0.1, "Generating tone…");
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let (bytes, _) = audio_tools::tone_wav(wave, hz, level_db, secs);
        std::fs::write(&out, bytes).map_err(|e| format!("{}: {e}", out.display()))?;
        prog.set(1.0, "Done");
        Ok(())
    });
    STATE.with(|s| s.borrow_mut().tones.push(ToneJob { prog: prog.clone(), asset, at: at.max(0.0) }));
    Ok((prog, path))
}

fn finish_tone(app: &mut App, job: ToneJob) {
    if let Some(e) = job.prog.error() {
        app.push_toast(Toast::new(format!("Tone generator failed: {e}")).kind(ToastKind::Error));
        return;
    }
    let path = PathBuf::from(&job.asset.path);
    let before = app.project.to_json();
    let aid = app.project.add_asset(job.asset);
    let ids = app.project.insert_asset_clips(aid, job.at, None);
    app.push_undo_labeled(before, "Tone Generator");
    app.selection = ids;
    app.after_edit();
    app.toast_with_folder("Tone placed on the timeline", path);
}

/// The Tone Generator window (Clip ▸ Add ▸ Tone Generator…).
fn tone_window(app: &mut App, ctx: &egui::Context) {
    let mut make = None;
    let open = STATE.with(|s| s.borrow().form.open);
    if !open {
        return;
    }
    let mut keep = true;
    egui::Window::new("Tone Generator").open(&mut keep).resizable(false).collapsible(false).show(ctx, |ui| {
        STATE.with(|s| {
            let f = &mut s.borrow_mut().form;
            crate::ui::prop_field(ui, "Wave", |ui| {
                egui::ComboBox::from_id_salt("tone_wave").selected_text(f.wave.name()).width(140.0).show_ui(ui, |ui| {
                    for w in Wave::ALL {
                        ui.selectable_value(&mut f.wave, w, w.name());
                    }
                });
            });
            ui.add_enabled_ui(f.wave.pitched(), |ui| {
                crate::ui::prop_field_val(
                    ui,
                    "Frequency",
                    egui::DragValue::new(&mut f.hz).range(20.0..=20000.0).speed(5.0).suffix(" Hz"),
                )
                .on_hover_text("1 kHz is the usual line-up tone");
            });
            crate::ui::prop_field_val(ui, "Level", egui::DragValue::new(&mut f.level_db).range(-60.0..=0.0).speed(0.2).suffix(" dB"))
                .on_hover_text("Peak level in dBFS (-18 dB is the EBU line-up level)");
            crate::ui::prop_field_val(ui, "Length", egui::DragValue::new(&mut f.secs).range(0.1..=3600.0).speed(0.5).suffix(" s"));
            ui.add_space(4.0);
            if ui.button("Add at Playhead").on_hover_text("Writes a WAV, adds it to the Library and places it").clicked() {
                make = Some((f.wave, f.hz, f.level_db, f.secs));
            }
        });
    });
    if let Some((wave, hz, db, secs)) = make {
        let at = app.playhead;
        match start_tone(app, wave, hz, db, secs, at) {
            Ok(_) => keep = false,
            Err(e) => app.toast(e),
        }
    }
    if !keep {
        STATE.with(|s| s.borrow_mut().form.open = false);
    }
}

// ---------- registry entries ----------

/// FRAME_HOOKS: a filter's Analyze click, finished analyses and tones, and the Tone Generator window.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    if let Some((owner, i)) = crate::ui::mixer_ui::take_analyze() {
        if app.project.clip(owner).is_some() {
            if let Err(e) = start_analysis(app, owner, i) {
                app.toast(e);
            }
        } else {
            app.toast("Analyze works on a clip's own Normalize filter (Inspector ▸ Audio ▸ Filters); on a bus, set Measured LUFS by hand");
        }
    }
    let (done_a, done_t) = STATE.with(|s| {
        let s = &mut *s.borrow_mut();
        let (a, keep_a): (Vec<_>, Vec<_>) = std::mem::take(&mut s.analyses).into_iter().partition(|a| a.prog.is_done());
        s.analyses = keep_a;
        let (t, keep_t): (Vec<_>, Vec<_>) = std::mem::take(&mut s.tones).into_iter().partition(|t| t.prog.is_done());
        s.tones = keep_t;
        (a, t)
    });
    for a in done_a {
        finish_analysis(app, a);
    }
    for t in done_t {
        finish_tone(app, t);
    }
    if STATE.with(|s| !s.borrow().tones.is_empty() || !s.borrow().analyses.is_empty()) {
        ctx.request_repaint_after(std::time::Duration::from_millis(100)); // only while a job runs
    }
    tone_window(app, ctx);
}

/// ACT_HANDLERS: Tone Generator… opens (or brings back) the window.
pub(super) fn act(_app: &mut App, a: Action) -> bool {
    if a != Action::ToneGenerator {
        return false;
    }
    STATE.with(|s| s.borrow_mut().form.open = true);
    true
}

/// PANE_DRAWERS: the Audio Scopes pane. The scope is recomputed only when the audio thread has
/// published new frames, and cleared on the playing -> stopped edge.
pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::AudioScopes {
        return false;
    }
    let playing = app.player.is_playing();
    let palette = app.palette.clone();
    STATE.with(|s| {
        let s = &mut *s.borrow_mut();
        let total = SCOPE_FEED.total();
        if playing && total != s.seen {
            let now = std::time::Instant::now();
            let dt = s.at.map(|t| (now - t).as_secs_f32()).unwrap_or(0.0);
            SCOPE_FEED.latest(SCOPE_FFT, &mut s.frames);
            s.scope.update(&s.frames, dt);
            (s.seen, s.at) = (total, Some(now));
        } else if !playing && s.was_playing {
            s.scope.clear();
            s.at = None;
        }
        s.was_playing = playing;
        crate::ui::audio_scopes_ui::body(ui, &s.scope, playing, &palette);
    });
    true
}

/// A filter as the tools report it: kind, enabled and every parameter by name (its value at clip time 0
/// and whether it is keyed).
fn filter_json(f: &AudioFilter) -> Value {
    let params: serde_json::Map<String, Value> = f
        .kind
        .params()
        .iter()
        .enumerate()
        .map(|(i, s)| (s.name.to_string(), json!(f.params.get(i).map(|a| a.value).unwrap_or(s.default))))
        .collect();
    let keyed: Vec<&str> = f
        .kind
        .params()
        .iter()
        .zip(&f.params)
        .filter(|(_, a)| a.is_animated())
        .map(|(s, _)| s.name)
        .collect();
    json!({"kind": format!("{:?}", f.kind), "name": f.kind.name(), "enabled": f.enabled, "params": params, "keyed": keyed})
}

/// Write `{name: value}` overrides onto an existing filter (constant values; a keyed parameter gets the
/// value at clip time `t`).
fn set_params(f: &mut AudioFilter, params: &serde_json::Map<String, Value>, t: f64) -> Result<(), String> {
    f.fill_params();
    for (name, v) in params {
        let i = f
            .kind
            .params()
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("unknown param '{name}' for {}", f.kind.name()))?;
        let v = v.as_f64().ok_or("param values must be numbers")?;
        f.params[i].set_at(t, v);
    }
    Ok(())
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "clip.audio_filter_add",
        desc: "Add an audio filter to each clip's own chain (runs before the clip's volume, pan and fades, \
               then the clip goes to its bus). Kinds: every Mixer filter - Eq, Eq15, HighPass, LowPass, \
               BandPass, Notch, Compressor, Expander, NoiseGate, Limiter, DeEsser, DeHum, Declick, MidSide, \
               StereoWidth, Channels (Mode 0 copy L>R, 1 copy R>L, 2 swap, 3 invert L, 4 invert R, 5 invert \
               both), Downmix, Normalize (two-pass; see audio.normalize_loudness), Reverb, Echo, Pitch, Gain, ...",
        args: &[
            "clip_ids:array:true:clip ids",
            "kind:string:true:filter kind (name or variant, any case)",
            "params:object:false:{param name: value} overrides",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let ids = req(a.ids("clip_ids"), "clip_ids")?;
            let kind_s = req(a.str("kind"), "kind")?;
            let kind = FilterKind::parse(kind_s).ok_or_else(|| format!("unknown filter '{kind_s}'"))?;
            let f = AudioFilter::with_params(kind, args.get("params").and_then(|v| v.as_object()))?;
            let mut out = Vec::new();
            for id in ids {
                let c = app.project.clip_mut(id).ok_or_else(|| format!("no clip {id}"))?;
                c.audio_filters.push(f.clone());
                out.push(json!({"clip_id": id, "index": c.audio_filters.len() - 1}));
            }
            Ok(ToolOutcome::Done(json!({"ok": true, "added": out})))
        },
    },
    ToolDef {
        name: "clip.audio_filter_set",
        desc: "Change one of a clip's own audio filters: parameter values by name (a keyed parameter is \
               set at clip time t), and/or enabled.",
        args: &[
            "clip_id:integer:true:",
            "index:integer:true:position in the clip's chain (clip.audio_filters lists them)",
            "params:object:false:{param name: value}",
            "enabled:boolean:false:",
            "t:number:false:clip-local seconds for keyed parameters (default 0)",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let id = req(a.id("clip_id"), "clip_id")?;
            let i = req(a.id("index"), "index")? as usize;
            let t = a.f64("t").unwrap_or(0.0);
            let c = app.project.clip_mut(id).ok_or("no such clip")?;
            let f = c.audio_filters.get_mut(i).ok_or("no filter at that index")?;
            if let Some(p) = args.get("params").and_then(|v| v.as_object()) {
                set_params(f, p, t)?;
            }
            if let Some(e) = a.bool("enabled") {
                f.enabled = e;
            }
            Ok(ToolOutcome::Done(filter_json(f)))
        },
    },
    ToolDef {
        name: "clip.audio_filter_remove",
        desc: "Remove one of a clip's own audio filters (by its index), or all of them with index omitted.",
        args: &["clip_id:integer:true:", "index:integer:false:omit to remove every filter"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let id = req(a.id("clip_id"), "clip_id")?;
            let c = app.project.clip_mut(id).ok_or("no such clip")?;
            match a.id("index") {
                Some(i) if (i as usize) < c.audio_filters.len() => {
                    c.audio_filters.remove(i as usize);
                }
                Some(_) => return Err("no filter at that index".into()),
                None => c.audio_filters.clear(),
            }
            Ok(ToolOutcome::Done(json!({"ok": true, "remaining": c.audio_filters.len()})))
        },
    },
    ToolDef {
        name: "clip.audio_filters",
        desc: "A clip's own audio filter chain, in order: kind, enabled, parameters by name, which are keyed.",
        args: &["clip_id:integer:true:"],
        kind: ToolKind::Read,
        run: |app, args| {
            let id = req(Args(args).id("clip_id"), "clip_id")?;
            let c = app.project.clip(id).ok_or("no such clip")?;
            Ok(ToolOutcome::Done(json!(c.audio_filters.iter().map(filter_json).collect::<Vec<_>>())))
        },
    },
    ToolDef {
        name: "track.audio_edges",
        desc: "An audio track's automatic fades at clip edges (fade_ms, 0 = off; Shotcut's track auto-fade) \
               and/or smooth seams (clips that touch cross-fade over 10 ms). Omitted values are kept.",
        args: &["track_index:integer:true:", "fade_ms:number:false:0..1000, 0 = off", "seam:boolean:false:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let ti = req(a.id("track_index"), "track_index")? as usize;
            let t = app.project.tracks.get(ti).ok_or("no such track")?;
            if t.kind != crate::model::TrackKind::Audio {
                return Err("auto-fade and seams are for audio tracks".into());
            }
            app.project.set_track_audio_edges(ti, a.f64("fade_ms").map(|f| f as f32), a.bool("seam"));
            let t = &app.project.tracks[ti];
            Ok(ToolOutcome::Done(json!({"ok": true, "fade_ms": t.auto_fade_ms, "seam": t.seam})))
        },
    },
    ToolDef {
        name: "audio.normalize_loudness",
        desc: "Two-pass loudness normalize: give each clip a Normalize filter (or retarget the one it has)                and measure the clip in the background; when the measurement lands, the filter applies                target - measured (capped by its Max gain) as its own undo step. Read the result with                clip.audio_filters. Audio clips only (a nested sequence has no file to measure).",
        args: &["clip_ids:array:true:", "target_lufs:number:false:-40..-5, default -16 (streaming); -23 for broadcast"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let ids = req(a.ids("clip_ids"), "clip_ids")?;
            let n = normalize_clips(app, &ids, a.f64("target_lufs").unwrap_or(-16.0))?;
            Ok(ToolOutcome::Done(json!({"ok": true, "analyzing": n})))
        },
    },
    ToolDef {
        name: "audio.tone",
        desc: "Tone generator: write a test tone or noise as a 48 kHz stereo WAV, add it to the Library and                place it at `at` (default the playhead) once written. Replies when the file is written.",
        args: &[
            "wave:string:false:Sine (default)|Square|Sawtooth|Triangle|White noise|Pink noise",
            "hz:number:false:20..20000, default 1000",
            "level_db:number:false:peak dBFS, -60..0, default -18",
            "seconds:number:false:length, default 10, max 3600",
            "at:number:false:timeline seconds, default the playhead",
        ],
        kind: ToolKind::Job,
        run: |app, args| {
            let a = Args(args);
            let wave = match a.str("wave") {
                Some(w) => Wave::parse(w).ok_or_else(|| format!("unknown wave '{w}'"))?,
                None => Wave::Sine,
            };
            let at = a.f64("at").unwrap_or(app.playhead);
            let (prog, path) = start_tone(
                app,
                wave,
                a.f64("hz").unwrap_or(1000.0).clamp(20.0, 20000.0),
                a.f64("level_db").unwrap_or(-18.0).clamp(-60.0, 0.0),
                a.f64("seconds").unwrap_or(10.0),
                at,
            )?;
            Ok(ToolOutcome::Job(prog, path))
        },
    },
    ToolDef {
        name: "audio.scopes",
        desc: "What the Audio Scopes pane shows for the audio playing now: the spectrum (dB per band,                20 Hz-20 kHz), L/R correlation (+1 mono, 0 wide, <0 out of phase) and peak levels. Silent                (floor) while stopped.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _args| {
            let mut sc = Scope::default();
            if app.player.is_playing() {
                let mut frames = Vec::new();
                SCOPE_FEED.latest(SCOPE_FFT, &mut frames);
                sc.update(&frames, 0.0);
            }
            let bands: Vec<Value> = (0..sc.bars.len())
                .map(|i| json!({"hz": audio_tools::bar_hz(i).round(), "db": (sc.bars[i] * 10.0).round() / 10.0}))
                .collect();
            Ok(ToolOutcome::Done(json!({
                "playing": app.player.is_playing(),
                "spectrum": bands,
                "correlation": sc.correlation,
                "peak_l": sc.peak.0, "peak_r": sc.peak.1,
            })))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_audio_parity_rows_resolve() {
        for t in TOOLS {
            assert_eq!(crate::mcp::tools::input_schema(t.args)["type"], "object", "{}", t.name);
            assert!(crate::mcp::tools::find(t.name).is_some(), "{} missing from mcp::tools::all()", t.name);
        }
        assert_eq!(crate::mcp::tools::find("clip.audio_filters").unwrap().kind, ToolKind::Read);
    }

    #[test]
    fn filter_json_and_set_params_round_trip() {
        let mut f = AudioFilter::new(FilterKind::Notch);
        set_params(&mut f, json!({"frequency": 60.0, "q": 10.0}).as_object().unwrap(), 0.0).unwrap();
        let j = filter_json(&f);
        assert_eq!(j["kind"], "Notch");
        assert_eq!(j["params"]["Frequency"], 60.0);
        assert_eq!(j["params"]["Q"], 10.0);
        assert!(set_params(&mut f, json!({"Gain": 1.0}).as_object().unwrap(), 0.0).is_err());
        // an old, shorter filter is topped up before a later parameter is written
        let mut short = AudioFilter::new(FilterKind::Expander);
        short.params.truncate(2);
        set_params(&mut short, json!({"Range dB": 20.0}).as_object().unwrap(), 0.0).unwrap();
        assert_eq!(short.params.len(), 5);
    }

    #[test]
    fn set_track_audio_edges_only_on_audio_tracks() {
        let mut p = Project::new();
        let ai = p.add_track(crate::model::TrackKind::Audio);
        let vi = p.add_track(crate::model::TrackKind::Video);
        assert!(p.set_track_audio_edges(ai, Some(10.0), None));
        assert!(!p.set_track_audio_edges(ai, Some(10.0), None), "no change");
        assert!(p.set_track_audio_edges(ai, None, Some(true)));
        assert_eq!((p.tracks[ai].auto_fade_ms, p.tracks[ai].seam), (10.0, true));
        assert!(p.set_track_audio_edges(ai, Some(5000.0), None));
        assert_eq!(p.tracks[ai].auto_fade_ms, 1000.0, "clamped");
        assert!(!p.set_track_audio_edges(vi, Some(10.0), Some(true)), "video tracks have no audio edges");
        // a track saved without the fields loads as off, and one with them round-trips
        let back: crate::model::Track = serde_json::from_value(serde_json::to_value(&p.tracks[ai]).unwrap()).unwrap();
        assert_eq!((back.auto_fade_ms, back.seam), (1000.0, true));
        let mut plain = serde_json::to_value(&p.tracks[vi]).unwrap();
        assert!(plain.get("seam").is_none() && plain.get("auto_fade_ms").is_none(), "off = not written");
        plain["kind"] = json!("Audio");
        let t: crate::model::Track = serde_json::from_value(plain).unwrap();
        assert_eq!((t.auto_fade_ms, t.seam), (0.0, false));
    }
}
