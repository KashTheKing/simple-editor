//! ---- ws:audio-analysis ----
//! 9 MCP tools over `engine::analysis` + `engine::autocut` (auto-Luau-exposed via `TOOL_TABLES`):
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
                    analysis::detect_beat_markers(project, &ids, refractory, sensitivity, &mut asset_peaks(waveforms))
                };
                app.fire_markers_added(&marker_ids);
                Ok(ToolOutcome::Done(json!({"ok": true, "marker_ids": marker_ids, "bpm": bpm})))
            } else if a.bool("split").unwrap_or(false) {
                let n = {
                    let App { project, waveforms, .. } = app;
                    analysis::split_beats(project, &ids, refractory, sensitivity, &mut asset_peaks(waveforms))
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
];

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
        assert_eq!(TOOLS.len(), 9);
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
}
