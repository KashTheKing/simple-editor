//! ---- ws:audio-analysis ----
//! MCP tools over `engine::analysis` + `engine::autocut` (auto-Luau-exposed via `TOOL_TABLES`):
//! audio.analyze/beats/duck/normalize/match_loudness/sync_offset, autocut.detect/mark, media.scene_cuts.
//! Every tool that writes a marker (audio.beats' as_markers, autocut.mark, media.scene_cuts' as_markers)
//! routes through `App::fire_markers_added` so `marker_added` fires exactly once per marker, identically
//! to the Auto-cut pane's buttons (autocut_ui.rs) and the hotkey Actions (audio_actions.rs).

use super::tools_args::Args;
use super::tools_helpers::*;
use super::*;
use crate::engine::analysis::{self, NormMode};
use crate::engine::autocut::{self, AutoCutParams};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::engine::visualizer;
use crate::media::waveform::Peaks;
use std::sync::Arc;

/// Per-clip (cuts, quiet ranges) via `autocut::loud_segments`/`to_timeline` - the same detection
/// `timeline.auto_cut` (tools_timeline.rs) performs, shared here by `autocut.detect` (preview) and
/// `autocut.mark` ("Mark instead" of Apply) so they never read a segment differently from each other.
fn detect_silence(
    app: &mut App,
    ids: &[Id],
    params: &AutoCutParams,
    keep_quiet: bool,
) -> Result<Vec<(Id, Vec<f64>, Vec<(f64, f64)>)>, String> {
    let mut out = Vec::new();
    for &id in ids {
        let c = app.project.clip(id).ok_or("no such clip")?.clone();
        if c.kind != ClipKind::Audio || c.reverse || c.freeze.is_some() {
            continue;
        }
        let path = app.project.asset(c.asset).ok_or("clip has no asset")?.path.clone();
        let peaks =
            app.waveforms.get(&path, c.audio_stream).ok_or("waveform still computing - try again in a moment")?;
        let segs = autocut::loud_segments(&peaks, c.src_in, c.src_len(), params);
        let (cuts, quiet) = autocut::to_timeline(&segs, c.start, c.src_in, c.duration, c.speed, keep_quiet);
        out.push((id, cuts, quiet));
    }
    Ok(out)
}

fn silence_params(a: &Args) -> AutoCutParams {
    let mut params = AutoCutParams::default();
    if let Some(v) = a.f64("threshold_db") {
        params.threshold_db = v as f32;
    }
    if let Some(v) = a.f64("min_silence") {
        params.min_silence = v;
    }
    if let Some(v) = a.f64("min_speech") {
        params.min_speech = v;
    }
    if let Some(v) = a.f64("padding") {
        params.padding = v;
    }
    params
}

fn get_peaks(app: &mut App, id: Id) -> Option<Arc<Peaks>> {
    let c = app.project.clip(id)?;
    let (asset, stream) = (c.asset, c.audio_stream);
    let path = app.project.asset(asset)?.path.clone();
    app.waveforms.get(&path, stream)
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "audio.analyze",
        desc: "Peak/RMS dBFS and onset count/BPM per clip (from cached waveform peaks); 'waveform still \
               computing' if peaks aren't ready yet.",
        args: &["clip_ids:array:false:defaults to selection"],
        kind: ToolKind::Read,
        run: |app, args| {
            let ids = Args(args).ids_or_selection("clip_ids", app);
            let sensitivity = app.settings.beat_thr;
            let mut out = Vec::new();
            for id in ids {
                let Some(c) = app.project.clip(id) else { continue };
                let (name, src_in, src_end) = (c.name.clone(), c.src_in, c.src_in + c.duration * c.speed);
                match get_peaks(app, id) {
                    Some(peaks) => {
                        let lv = analysis::levels(&peaks, src_in, src_end);
                        let onsets = analysis::onsets(&peaks, src_in, src_end, 0.25, sensitivity);
                        out.push(json!({
                            "clip_id": id, "name": name, "peak_db": lv.peak_db, "rms_db": lv.rms_db,
                            "onset_count": onsets.len(), "bpm": analysis::bpm(&onsets),
                        }));
                    }
                    None => out.push(json!({"clip_id": id, "name": name, "status": "waveform still computing"})),
                }
            }
            Ok(ToolOutcome::Done(json!(out)))
        },
    },
    ToolDef {
        name: "audio.beats",
        desc: "Detect beats on audio clip(s). With neither flag: pure detection, returns onset times \
               (source secs) + BPM, no mutation. as_markers adds a clip marker per onset (fires \
               marker_added); split cuts each target clip's link group at every beat.",
        args: &[
            "clip_ids:array:false:defaults to selection",
            "refractory_s:number:false:default 0.25",
            "as_markers:boolean:false:default false",
            "split:boolean:false:default false",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let ids = a.ids_or_selection("clip_ids", app);
            let refractory = a.f64("refractory_s").unwrap_or(0.25);
            let sensitivity = app.settings.beat_thr;
            if a.bool("as_markers").unwrap_or(false) {
                let (marker_ids, bpm) = {
                    let App { project, waveforms, .. } = app;
                    let (per_clip, bpm) = analysis::detect_beats(project, &ids, refractory, sensitivity, &mut asset_peaks(waveforms));
                    (analysis::beat_markers(project, &per_clip), bpm)
                };
                app.fire_markers_added(&marker_ids);
                Ok(ToolOutcome::Done(json!({"ok": true, "marker_ids": marker_ids, "bpm": bpm})))
            } else if a.bool("split").unwrap_or(false) {
                let n = {
                    let App { project, waveforms, .. } = app;
                    let (per_clip, _bpm) = analysis::detect_beats(project, &ids, refractory, sensitivity, &mut asset_peaks(waveforms));
                    analysis::split_beats(project, &per_clip)
                };
                Ok(ToolOutcome::Done(json!({"ok": true, "cuts": n})))
            } else {
                let mut per_clip = Vec::new();
                let mut all = Vec::new();
                for &id in &ids {
                    let Some(c) = app.project.clip(id) else { continue };
                    let (src_in, src_end) = (c.src_in, c.src_in + c.duration * c.speed);
                    match get_peaks(app, id) {
                        Some(peaks) => {
                            let onsets = analysis::onsets(&peaks, src_in, src_end, refractory, sensitivity);
                            per_clip.push(json!({"clip_id": id, "onsets": onsets}));
                            all.extend(onsets);
                        }
                        None => per_clip.push(json!({"clip_id": id, "status": "waveform still computing"})),
                    }
                }
                Ok(ToolOutcome::Done(json!({"clips": per_clip, "bpm": analysis::bpm(&all)})))
            }
        },
    },
    ToolDef {
        name: "audio.duck",
        desc: "Duck the music clip's volume under every loud (speech) window found on the dialogue \
               clips' peaks, with a ramp. dry_run returns the computed key count without writing.",
        args: &[
            "music_id:integer:true:",
            "dialogue_ids:array:true:",
            "depth_db:number:false:default Settings.duck_depth_db",
            "ramp_s:number:false:default Settings.duck_ramp_ms/1000",
            "dry_run:boolean:false:default false",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let music = req(a.id("music_id"), "music_id")?;
            let dialogue = req(a.ids("dialogue_ids"), "dialogue_ids")?;
            let depth_db = a.f64("depth_db").unwrap_or(app.settings.duck_depth_db as f64);
            let ramp_s = a.f64("ramp_s").unwrap_or(app.settings.duck_ramp_ms as f64 / 1000.0);
            let dry_run = a.bool("dry_run").unwrap_or(false);
            let n = if dry_run {
                // ponytail: dry-run scales onto a scratch clone instead of adding a bool param to the
                // pure engine fn - see the workstream's ponytail notes.
                let mut scratch = app.project.clone();
                let App { waveforms, .. } = app;
                analysis::duck(&mut scratch, music, &dialogue, depth_db, ramp_s, &mut asset_peaks(waveforms))
            } else {
                let App { project, waveforms, .. } = app;
                analysis::duck(project, music, &dialogue, depth_db, ramp_s, &mut asset_peaks(waveforms))
            };
            Ok(ToolOutcome::Done(json!({"ok": true, "dry_run": dry_run, "keys": n})))
        },
    },
    ToolDef {
        name: "audio.normalize",
        desc: "Set each clip's constant gain so its own peak/RMS reaches target_dbfs. Clips with \
               keyframed volume are skipped (reported, not silently ignored).",
        args: &[
            "clip_ids:array:false:defaults to selection",
            "target_dbfs:number:false:default -1.0",
            "mode:string:false:peak|rms, default peak",
            "dry_run:boolean:false:default false",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let ids = a.ids_or_selection("clip_ids", app);
            let target = a.f64("target_dbfs").unwrap_or(-1.0);
            let mode = match a.str("mode") {
                Some("rms") => NormMode::Rms,
                _ => NormMode::Peak,
            };
            let skipped: Vec<Id> = ids
                .iter()
                .copied()
                .filter(|&id| app.project.clip(id).is_some_and(|c| c.volume.is_animated()))
                .collect();
            let dry_run = a.bool("dry_run").unwrap_or(false);
            let n = if dry_run {
                let mut scratch = app.project.clone();
                let App { waveforms, .. } = app;
                analysis::normalize(&mut scratch, &ids, target, mode, &mut asset_peaks(waveforms))
            } else {
                let App { project, waveforms, .. } = app;
                analysis::normalize(project, &ids, target, mode, &mut asset_peaks(waveforms))
            };
            Ok(ToolOutcome::Done(json!({"ok": true, "dry_run": dry_run, "changed": n, "skipped": skipped})))
        },
    },
    ToolDef {
        name: "audio.match_loudness",
        desc: "Scale every listed clip's gain toward the selection's average (bucket-)RMS.",
        args: &["clip_ids:array:true:>=2 clips", "dry_run:boolean:false:default false"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let ids = req(a.ids("clip_ids"), "clip_ids")?;
            if ids.len() < 2 {
                return Err("clip_ids needs at least 2 clips".into());
            }
            let dry_run = a.bool("dry_run").unwrap_or(false);
            let n = if dry_run {
                let mut scratch = app.project.clone();
                let App { waveforms, .. } = app;
                analysis::match_loudness(&mut scratch, &ids, &mut asset_peaks(waveforms))
            } else {
                let App { project, waveforms, .. } = app;
                analysis::match_loudness(project, &ids, &mut asset_peaks(waveforms))
            };
            Ok(ToolOutcome::Done(json!({"ok": true, "dry_run": dry_run, "changed": n})))
        },
    },
    ToolDef {
        name: "audio.sync_offset",
        desc: "Best lag (seconds) that aligns clip_b to clip_a via cross-correlation of their waveform \
               envelopes. For multicam sync; the angle-grid UI is a later (pro-monitor) workstream.",
        args: &["clip_a:integer:true:", "clip_b:integer:true:", "max_lag_s:number:false:default 60, hard cap 120"],
        kind: ToolKind::Read,
        run: |app, args| {
            let a = Args(args);
            let ca = req(a.id("clip_a"), "clip_a")?;
            let cb = req(a.id("clip_b"), "clip_b")?;
            let max_lag = a.f64("max_lag_s").unwrap_or(60.0);
            let pa = get_peaks(app, ca).ok_or("clip_a: waveform still computing")?;
            let pb = get_peaks(app, cb).ok_or("clip_b: waveform still computing")?;
            Ok(ToolOutcome::Done(json!({"offset_s": analysis::xcorr_offset(&pa, &pb, max_lag)})))
        },
    },
    ToolDef {
        name: "autocut.detect",
        desc: "Silence-detection preview: returns cut times + kept/removed ranges per clip without \
               mutating the project (never applies).",
        args: &[
            "clip_ids:array:false:defaults to audio_targets(selection)",
            "threshold_db:number:false:",
            "min_silence:number:false:",
            "min_speech:number:false:",
            "padding:number:false:",
            "keep_quiet:boolean:false:",
        ],
        kind: ToolKind::Read,
        run: |app, args| {
            let a = Args(args);
            let params = silence_params(&a);
            let keep_quiet = a.bool("keep_quiet").unwrap_or(false);
            let ids =
                a.ids("clip_ids").unwrap_or_else(|| crate::ui::autocut_ui::audio_targets(&app.project, &app.selection));
            let per_clip = detect_silence(app, &ids, &params, keep_quiet)?;
            let out: Vec<Value> = per_clip
                .into_iter()
                .map(|(id, cuts, quiet)| json!({"clip_id": id, "cuts": cuts, "quiet_ranges": quiet}))
                .collect();
            Ok(ToolOutcome::Done(json!(out)))
        },
    },
    ToolDef {
        name: "autocut.mark",
        desc: "'Mark instead' of Apply: adds a project range marker (start,end) per detected silence \
               segment (the one Apply would have cut/removed), leaving every clip untouched, and fires \
               marker_added per marker.",
        args: &[
            "clip_ids:array:false:defaults to audio_targets(selection)",
            "threshold_db:number:false:",
            "min_silence:number:false:",
            "min_speech:number:false:",
            "padding:number:false:",
            "keep_quiet:boolean:false:",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let params = silence_params(&a);
            let keep_quiet = a.bool("keep_quiet").unwrap_or(false);
            let ids =
                a.ids("clip_ids").unwrap_or_else(|| crate::ui::autocut_ui::audio_targets(&app.project, &app.selection));
            let per_clip = detect_silence(app, &ids, &params, keep_quiet)?;
            let ranges: Vec<(f64, f64)> = per_clip.into_iter().flat_map(|(_, _, quiet)| quiet).collect();
            let marker_ids = crate::ui::autocut_ui::mark_ranges(&mut app.project, &ranges, "Silence");
            app.fire_markers_added(&marker_ids);
            Ok(ToolOutcome::Done(json!({"ok": true, "marker_ids": marker_ids})))
        },
    },
    ToolDef {
        name: "media.scene_cuts",
        desc: "ffmpeg select='gt(scene,T)' shot-change detection over the clip's source. Neither flag: \
               pure detection, returns cut times (source secs). as_markers adds one point marker \
               (duration 0) per cut and fires marker_added; split cuts the clip at each one. \
               Synchronous (blocks the UI thread for the ffmpeg pass) - documented ceiling.",
        args: &[
            "clip_id:integer:false:defaults to selection's first video clip",
            "threshold:number:false:default 0.3",
            "split:boolean:false:default false",
            "as_markers:boolean:false:default false",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let clip = match a.id("clip_id") {
                Some(id) => id,
                None => app
                    .selection
                    .iter()
                    .copied()
                    .find(|&id| app.project.clip(id).is_some_and(|c| c.kind == ClipKind::Video))
                    .ok_or("no video clip selected")?,
            };
            let thr = a.f64("threshold").unwrap_or(0.3) as f32;
            let asset = app.project.clip(clip).ok_or("no such clip")?.asset;
            let path = app.project.asset(asset).ok_or("clip has no asset")?.path.clone();
            let cuts = analysis::scene_cuts(std::path::Path::new(&path), thr)?;
            if a.bool("as_markers").unwrap_or(false) {
                let marker_ids = analysis::scene_cut_markers(&mut app.project, clip, &cuts);
                app.fire_markers_added(&marker_ids);
                Ok(ToolOutcome::Done(json!({"ok": true, "marker_ids": marker_ids})))
            } else if a.bool("split").unwrap_or(false) {
                let n = analysis::split_scene_cuts(&mut app.project, clip, &cuts);
                Ok(ToolOutcome::Done(json!({"ok": true, "cuts": n})))
            } else {
                Ok(ToolOutcome::Done(json!({"cut_times": cuts})))
            }
        },
    },
    ToolDef {
        name: "audio.bake_keys",
        desc: "Bake an audio clip's loudness into keyframes on another clip's effect parameter (Blender's \
               'Bake Sound to F-Curves'): one key per frame over the target clip, 0..1 loudness mapped onto \
               the parameter's range. Drives audio visualizers such as the Neon Orb effect's `level`.",
        args: &[
            "clip_id:integer:true:the audio clip to listen to",
            "target_id:integer:true:the clip whose effect gets the keys",
            "effect:integer:false:effect index, default the last effect",
            "param:string:false:parameter name, default level",
            "fps:number:false:keys per second, default 30",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let (src, dst) = (req(a.id("clip_id"), "clip_id")?, req(a.id("target_id"), "target_id")?);
            let fps = a.f64("fps").unwrap_or(30.0).clamp(1.0, 120.0);
            let peaks = get_peaks(app, src).ok_or("waveform still computing - try again in a moment")?;
            let ei = a.f64("effect").map(|v| v as usize);
            let n = bake_level(&mut app.project, &peaks, src, dst, ei, a.str("param").unwrap_or("level"), fps)?;
            Ok(ToolOutcome::Done(json!({"ok": true, "keys": n})))
        },
    },
    ToolDef {
        name: "audio.visualizer",
        desc: "Add an audio visualizer over an audio clip: a full-frame layer spanning it whose shader follows \
               the music (level + 6 spectrum bands baked to keyframes once, so playback stays cheap), LINKED to \
               the song so moving / trimming / deleting one does the other. Same as right-click > Audio > Add \
               Audio Visualizer. asset_id instead: places that audio file at `at` (default the playhead) with \
               its linked visualizer, as Library > Add with Audio Visualizer does. Given a visualizer clip, \
               switches its style (when `style` is given) and bakes the audio again. style: a name from \
               audio.visualizer_styles (default the last used, else Neon Orb). Band mixer (which frequencies \
               drive it): preset EDM|Hip-hop|Rock|Vocal|Lo-fi|Classical|Reset, gains = 6 numbers 0..3 \
               (Sub, Bass, Low-mid, Mid, High-mid, Air) or {band: gain}, sensitivity 0.25..4, attack / \
               release seconds, beat_bands = band names the beat detection hears ([] = the whole sound).",
        args: &[
            "clip_id:integer:false:an audio clip (adds a visualizer) or a visualizer clip (restyles it)",
            "asset_id:integer:false:an audio file in the library: place it with a visualizer",
            "at:number:false:with asset_id: timeline seconds, default the playhead",
            "style:string:false:style name, default the last used",
            "preset:string:false:band-mixer preset, or Reset",
            "gains:object:false:6 gains 0..3 (array) or {band name: gain}",
            "sensitivity:number:false:0.25..4",
            "attack:number:false:seconds to rise",
            "release:number:false:seconds to fall",
            "beat_bands:array:false:band names the beats follow; [] = the whole sound",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let a = Args(args);
            let mix = mix_args(args)?;
            let style = match a.str("style") {
                Some(n) => visualizer::find(Some(n))?,
                None => visualizer::last_used(&app.settings.visualizer_style),
            };
            let backend = app.backend();
            if a.str("style").is_some() && app.settings.visualizer_style != style.name {
                app.settings.visualizer_style = style.name.clone();
                app.settings.save();
            }
            if let Some(asset) = a.id("asset_id") {
                let at = a.f64("at").unwrap_or(app.playhead);
                let (song, vis) = visualizer::add_song(&mut app.project, asset, at, &style, backend)?;
                apply_mix(app, vis, mix, backend)?;
                return Ok(ToolOutcome::Done(json!({"ok": true, "clip_id": vis, "audio_clip_id": song, "style": style.name})));
            }
            let id = req(a.id("clip_id"), "clip_id or asset_id")?;
            let clip = app.project.clip(id).ok_or("no such clip_id")?;
            if let Some(cur) = clip.effects.iter().find_map(visualizer::style_of) {
                // a visualizer: restyle only when asked to (band settings alone keep the look)
                let name = if a.str("style").is_some() {
                    visualizer::restyle(&mut app.project, id, &style, backend)?;
                    style.name
                } else {
                    cur
                };
                apply_mix(app, id, mix, backend)?;
                return Ok(ToolOutcome::Done(json!({"ok": true, "clip_id": id, "style": name})));
            }
            let vid = visualizer::add(&mut app.project, id, &style, backend)?;
            apply_mix(app, vid, mix, backend)?;
            Ok(ToolOutcome::Done(json!({"ok": true, "clip_id": vid, "style": style.name})))
        },
    },
    ToolDef {
        name: "audio.visualizer_styles",
        desc: "The audio visualizer styles audio.visualizer and the Inspector offer: the built-ins, then every \
               library effect that reads audio (its header declares `level` or `band1`..`band6`; save one \
               with library.save kind=effect).",
        args: &[],
        kind: ToolKind::Read,
        run: |_, _| {
            let rows: Vec<Value> =
                visualizer::styles().iter().map(|s| json!({"name": s.name, "custom": s.custom})).collect();
            Ok(ToolOutcome::Done(json!({"styles": rows})))
        },
    },
];

/// The band-mixer arguments of `audio.visualizer`, applied on top of a preset (or the clip's current mix
/// when only some are given: `apply_mix` merges). None when no band argument was passed.
type MixArgs = Option<Box<dyn Fn(crate::model::AudioMix) -> crate::model::AudioMix>>;

fn mix_args(args: &Value) -> Result<MixArgs, String> {
    use crate::model::AudioMix;
    let keys = ["preset", "gains", "sensitivity", "attack", "release", "beat_bands"];
    if !keys.iter().any(|k| args.get(*k).is_some_and(|v| !v.is_null())) {
        return Ok(None);
    }
    let preset = match args.get("preset").and_then(Value::as_str) {
        Some(p) if p.eq_ignore_ascii_case("reset") => Some(AudioMix::default()),
        Some(p) => Some(AudioMix::preset(p).ok_or_else(|| format!("no preset '{p}' (have: {})", AudioMix::PRESETS.join(", ")))?),
        None => None,
    };
    let mut gains: Vec<(usize, f32)> = Vec::new();
    match args.get("gains") {
        Some(Value::Array(v)) => {
            if v.len() != 6 {
                return Err("gains: give 6 numbers (Sub, Bass, Low-mid, Mid, High-mid, Air)".into());
            }
            for (i, g) in v.iter().enumerate() {
                gains.push((i, g.as_f64().ok_or("gains: numbers")? as f32));
            }
        }
        Some(Value::Object(o)) => {
            for (k, g) in o {
                let b = AudioMix::band(k).ok_or_else(|| format!("gains: no band '{k}'"))?;
                gains.push((b, g.as_f64().ok_or("gains: numbers")? as f32));
            }
        }
        Some(Value::Null) | None => {}
        Some(_) => return Err("gains: an array of 6 numbers or {band: gain}".into()),
    }
    let beat_bands: Option<[bool; 6]> = match args.get("beat_bands") {
        Some(Value::Array(v)) => {
            let mut b = [false; 6];
            for n in v {
                let n = n.as_str().ok_or("beat_bands: band names")?;
                b[AudioMix::band(n).ok_or_else(|| format!("beat_bands: no band '{n}'"))?] = true;
            }
            Some(b)
        }
        _ => None,
    };
    let num = |k: &str| args.get(k).and_then(Value::as_f64).map(|v| v as f32);
    let (sens, att, rel) = (num("sensitivity"), num("attack"), num("release"));
    Ok(Some(Box::new(move |cur: AudioMix| {
        let mut m = preset.clone().unwrap_or(cur);
        let hand = !gains.is_empty() || beat_bands.is_some() || sens.is_some() || att.is_some() || rel.is_some();
        for &(b, g) in &gains {
            m.gains[b] = g;
        }
        if let Some(b) = beat_bands {
            m.beat_bands = b;
        }
        m.sensitivity = sens.unwrap_or(m.sensitivity);
        m.attack = att.unwrap_or(m.attack);
        m.release = rel.unwrap_or(m.release);
        if hand {
            m.preset.clear();
        }
        m.clamped()
    })))
}

/// Set visualizer `vis`'s band mix from `audio.visualizer`'s arguments and bake again.
fn apply_mix(app: &mut App, vis: Id, mix: MixArgs, backend: crate::media::Backend) -> Result<(), String> {
    let Some(f) = mix else { return Ok(()) };
    let e = app
        .project
        .clip_mut(vis)
        .and_then(|c| c.effects.iter_mut().find(|e| visualizer::style_of(e).is_some()))
        .ok_or("not a visualizer clip")?;
    let m = f(e.audio_mix.clone().unwrap_or_default());
    e.audio_mix = (m != crate::model::AudioMix::default()).then_some(m);
    if let Some(a) = visualizer::source_of(&app.project, vis) {
        visualizer::rebake(&mut app.project, vis, a, backend)?;
    }
    Ok(())
}

/// Bake audio clip `src`'s loudness into keyframes on `dst`'s effect `ei` (default: the last one),
/// parameter `name`: one key per `1/fps` s over `dst`, 0..1 loudness mapped onto the parameter's range.
/// Used by `audio.bake_keys`. Returns the key count.
pub(super) fn bake_level(
    p: &mut Project,
    peaks: &Peaks,
    src: Id,
    dst: Id,
    ei: Option<usize>,
    name: &str,
    fps: f64,
) -> Result<usize, String> {
    let audio = p.clip(src).ok_or("no such clip_id")?.clone();
    let target = p.clip_mut(dst).ok_or("no such target_id")?;
    let (start, len) = (target.start, target.duration);
    let ei = ei.unwrap_or(target.effects.len().wrapping_sub(1));
    let fx = target.effects.get_mut(ei).ok_or("target has no such effect")?;
    let specs = fx.specs();
    let pi = specs
        .iter()
        .position(|p| p.name.replace(' ', "_").eq_ignore_ascii_case(name))
        .ok_or(format!("effect has no param '{name}'"))?;
    let (lo, hi) = (specs[pi].min, specs[pi].max);
    while fx.params.len() <= pi {
        fx.params.push(crate::model::Animated::new(specs[fx.params.len()].default));
    }
    let level = loudness_keys(peaks, |lt| audio.src_time(start + lt), len, fps);
    fx.params[pi].keys = level
        .iter()
        .map(|&(t, v)| crate::model::Keyframe { t, v: lo + (hi - lo) * v, ease: Default::default() })
        .collect();
    Ok(level.len())
}

/// (clip-local time, 0..1 loudness) every `1/fps` s over `len` s; `src(lt)` maps to source time. Peak
/// level per frame, normalised to the loudest frame, with an instant attack and a ~0.15 s release so it
/// pulses on hits instead of flickering.
fn loudness_keys(peaks: &Peaks, src: impl Fn(f64) -> f64, len: f64, fps: f64) -> Vec<(f64, f64)> {
    let step = 1.0 / fps;
    let raw: Vec<f64> = (0..=(len * fps).floor() as usize)
        .map(|i| {
            let s = src(i as f64 * step);
            let (lo, hi) = peaks.range(s, s + step);
            lo.abs().max(hi.abs()) as f64
        })
        .collect();
    let top = raw.iter().fold(1e-4f64, |m, &v| m.max(v));
    let fall = (-step / 0.15).exp();
    let mut env = 0.0f64;
    raw.iter()
        .enumerate()
        .map(|(i, &v)| {
            env = (v / top).max(env * fall);
            (i as f64 * step, env.clamp(0.0, 1.0))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `every_arg_spec_parses`-style check scoped to just this file's 9 rows, plus a name/kind sanity
    /// pass - the crate-wide `mutate_rows_roll_back_on_error`/`every_arg_spec_parses` structural tests
    /// (tools_registry_tests.rs / mcp/tools.rs) already exercise these generically over every
    /// `ToolKind::Mutate` row via `mcp::tools::all()`; a live `App` isn't buildable in a test (see the
    /// App-construction deviation note in tools_registry_tests.rs) so this stays at the registry level.
    #[test]
    fn tools_audio_args_parse_and_round_trip() {
        assert_eq!(TOOLS.len(), 12);
        let names = [
            "audio.analyze",
            "audio.beats",
            "audio.duck",
            "audio.normalize",
            "audio.match_loudness",
            "audio.sync_offset",
            "autocut.detect",
            "autocut.mark",
            "media.scene_cuts",
            "audio.bake_keys",
            "audio.visualizer",
            "audio.visualizer_styles",
        ];
        for (t, want) in TOOLS.iter().zip(names) {
            assert_eq!(t.name, want);
            let schema = crate::mcp::tools::input_schema(t.args);
            assert_eq!(schema["type"], "object");
        }
        // registered in TOOL_TABLES and reachable through the flattened catalogue
        for name in names {
            assert!(crate::mcp::tools::find(name).is_some(), "{name} missing from mcp::tools::all()");
        }
    }

    /// audio.visualizer's band-mixer arguments: a preset as the base, single fields on top (which makes it
    /// hand-edited), names checked, nothing passed = no change.
    #[test]
    fn band_mixer_args_parse() {
        use crate::model::AudioMix;
        assert!(mix_args(&json!({"clip_id": 3})).unwrap().is_none());
        let f = mix_args(&json!({"preset": "rock", "gains": {"sub": 2.5}, "beat_bands": ["mid"]})).unwrap().unwrap();
        let m = f(AudioMix::default());
        let rock = AudioMix::preset("Rock").unwrap();
        assert_eq!(m.gains[0], 2.5);
        assert_eq!(m.gains[1..], rock.gains[1..]);
        assert_eq!(m.beat_bands, [false, false, false, true, false, false]);
        assert_eq!(m.preset, "", "edited by hand");
        let p = mix_args(&json!({"preset": "EDM"})).unwrap().unwrap()(AudioMix::default());
        assert_eq!(p, AudioMix::preset("EDM").unwrap(), "a bare preset keeps its name");
        let r = mix_args(&json!({"preset": "reset"})).unwrap().unwrap()(rock);
        assert_eq!(r, AudioMix::default());
        let s = mix_args(&json!({"sensitivity": 99, "gains": [0, 1, 1, 1, 1, 9]})).unwrap().unwrap()(AudioMix::default());
        assert_eq!((s.sensitivity, s.gains[5]), (4.0, 3.0), "clamped to the UI ranges");
        assert!(mix_args(&json!({"gains": [1, 2]})).is_err());
        assert!(mix_args(&json!({"beat_bands": ["kick"]})).is_err());
        assert!(mix_args(&json!({"preset": "polka"})).is_err());
    }

    /// Loud bursts give keys near 1 that fall off smoothly; silence stays at 0; deterministic.
    #[test]
    fn loudness_keys_pulse_on_hits() {
        // 2 s: silent, except a hit at 0.5 s and a softer one at 1.5 s
        let max: Vec<f32> = (0..200).map(|i| if i == 50 { 0.8 } else if i == 150 { 0.4 } else { 0.0 }).collect();
        let p = Peaks { min: max.iter().map(|m| -m).collect(), max };
        let k = loudness_keys(&p, |t| t, 2.0, 30.0);
        assert_eq!(k.len(), 61);
        assert_eq!(k, loudness_keys(&p, |t| t, 2.0, 30.0));
        assert_eq!(k[0].1, 0.0, "silence is 0");
        assert!(k[15].1 > 0.99, "loudest hit reaches 1: {}", k[15].1);
        assert!(k[18].1 > 0.3 && k[18].1 < k[15].1, "then decays, no snap to 0");
        assert!((k[45].1 - 0.5).abs() < 0.05, "half as loud, half as high: {}", k[45].1);
        assert!(k.iter().all(|&(_, v)| (0.0..=1.0).contains(&v)));
    }
}
