use super::tools_helpers::*;
use super::*;

pub(super) fn dispatch(app: &mut App, name: &str, args: &Value) -> Option<Result<Value, String>> {
    let prefix = name.split('.').next().unwrap_or("");
    if !matches!(prefix, "timeline" | "project") {
        return None;
    }
    pub(super) fn run(app: &mut App, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "project.summary" => {
                let p = &app.project;
                let clips_per_track: Vec<Value> = p
                    .tracks
                    .iter()
                    .map(|t| json!({"name": t.name, "kind": format!("{:?}", t.kind), "clips": t.clips.len()}))
                    .collect();
                pub(super) fn count_plan(items: &[crate::model::PlanItem]) -> (usize, usize) {
                    let mut done = 0;
                    let mut total = 0;
                    for i in items {
                        total += 1;
                        if i.done {
                            done += 1;
                        }
                        let (d, t) = count_plan(&i.children);
                        done += d;
                        total += t;
                    }
                    (done, total)
                }
                let (done, total) = count_plan(&p.plan);
                Ok(json!({
                    "name": p.name, "width": p.width, "height": p.height, "fps": p.fps,
                    "duration": p.duration(), "tracks": clips_per_track,
                    "assets": p.assets.len(), "sequences": p.sequences.len(),
                    "subtitles": p.subtitles.len(), "plan_done": done, "plan_total": total,
                    "notes": p.notes, "style": crate::engine::style::style_summary(p),
                    // non-null = these tracks/size are a nested sequence's, not the main timeline's
                    "editing_sequence": p.editing,
                }))
            }
            "project.get" => serde_json::to_value(&app.project).map_err(|e| e.to_string()),
            "project.new" => {
                let mut p = Project::new();
                p.width = arg_u64(args, "width").map(|w| w as u32).unwrap_or(1920);
                p.height = arg_u64(args, "height").map(|h| h as u32).unwrap_or(1080);
                p.fps = arg_f64(args, "fps").unwrap_or(30.0);
                app.set_project(p, None);
                Ok(json!({"ok": true}))
            }
            "project.open" => {
                let path = PathBuf::from(req(arg_str(args, "path"), "path")?);
                if !path.exists() {
                    return Err(format!("no such file: {}", path.display()));
                }
                if path.extension().map(|e| e.to_string_lossy().eq_ignore_ascii_case(PROJECT_EXT)).unwrap_or(false) {
                    let mut project = Project::load(&path)?;
                    relocate_assets(&mut project, path.parent());
                    app.set_project(project, Some(path));
                } else {
                    let asset = media::probe(&path.to_string_lossy(), app.backend())?;
                    app.set_project(Project::from_media(asset), None);
                }
                Ok(json!({"ok": true}))
            }
            "project.save" => {
                let path = match arg_str(args, "path") {
                    Some(p) => PathBuf::from(p),
                    None => app.project_path.clone().ok_or("no project file yet — pass a path")?,
                };
                app.project.save(&path).map_err(|e| e.to_string())?;
                app.project_path = Some(path.clone());
                app.dirty = false;
                Ok(json!({"ok": true, "path": path.to_string_lossy()}))
            }
            "project.set" => {
                if let Some(n) = arg_str(args, "name") {
                    app.project.name = n.to_string();
                }
                if let Some(w) = arg_u64(args, "width") {
                    app.project.width = w as u32;
                }
                if let Some(h) = arg_u64(args, "height") {
                    app.project.height = h as u32;
                }
                if let Some(f) = arg_f64(args, "fps") {
                    app.project.fps = f.max(1.0);
                }
                Ok(json!({"ok": true}))
            }
            "timeline.list" => {
                let p = &app.project;
                let tracks: Vec<Value> = p
                    .tracks
                    .iter()
                    .map(|t| {
                        let clips: Vec<Value> = t
                            .clips
                            .iter()
                            .map(|c| {
                                json!({
                                    "id": c.id, "kind": format!("{:?}", c.kind), "name": c.name,
                                    "asset": c.asset, "sequence": c.sequence, "start": c.start,
                                    "duration": c.duration, "src_in": c.src_in, "speed": c.speed,
                                    "reverse": c.reverse, "freeze": c.freeze, "enabled": c.enabled,
                                    "label": p.clip_label(c), "link": c.link,
                                    "effects": c.effects.iter().map(|e| e.kind.name()).collect::<Vec<_>>(),
                                })
                            })
                            .collect();
                        json!({"id": t.id, "name": t.name, "kind": format!("{:?}", t.kind), "clips": clips})
                    })
                    .collect();
                Ok(json!({"editing_sequence": p.editing, "duration": p.duration(), "tracks": tracks}))
            }
            "timeline.add_clip" => {
                let at = req(arg_f64(args, "at"), "at")?;
                let track = arg_u64(args, "track").map(|t| t as usize);
                if let Some(aid) = arg_u64(args, "asset_id") {
                    if app.project.asset(aid).is_none() {
                        return Err("no such asset".into());
                    }
                    let ids = app.project.insert_asset_clips(aid, at, track);
                    Ok(json!({"ok": true, "clip_ids": ids}))
                } else if let Some(sid) = arg_u64(args, "sequence_id") {
                    let id = app.project.insert_sequence_clip(sid, at, track).ok_or("no such sequence (or cycle)")?;
                    Ok(json!({"ok": true, "clip_ids": [id]}))
                } else if let Some(text) = arg_str(args, "text") {
                    let dur = arg_f64(args, "duration").unwrap_or(5.0).max(0.1);
                    let id = app.project.add_text_clip(at, dur);
                    if let Some(t) = app.project.clip_mut(id).and_then(|c| c.text.as_mut()) {
                        t.text = text.to_string();
                    }
                    Ok(json!({"ok": true, "clip_ids": [id]}))
                } else {
                    Err("pass asset_id, sequence_id or text".into())
                }
            }
            "timeline.split" => {
                let t = req(arg_f64(args, "t"), "t")?;
                let only = arg_ids(args, "clip_ids");
                let new = app.project.split_at(t, only.as_deref());
                Ok(json!({"ok": true, "new_clip_ids": new}))
            }
            "timeline.delete" => {
                let ids = app.project.expand_links(&req(arg_ids(args, "clip_ids"), "clip_ids")?);
                let ripple = arg_bool(args, "ripple").unwrap_or(false);
                app.project.delete_clips(&ids, ripple);
                Ok(json!({"ok": true}))
            }
            "timeline.move" => {
                let ids = app.project.expand_links(&req(arg_ids(args, "clip_ids"), "clip_ids")?);
                let dt = req(arg_f64(args, "dt"), "dt")?;
                let dtrack = args.get("dtrack").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let moved = app.project.move_clips(&ids, dt, dtrack, None);
                if moved {
                    Ok(json!({"ok": true}))
                } else {
                    Err("move blocked (overlap or out of range)".into())
                }
            }
            "timeline.trim" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let clip = app.project.clip(id).ok_or("no such clip")?.clone();
                let headroom = app.project.head_room(&clip);
                let max_dur = app.project.max_clip_duration(&clip);
                let c = app.project.clip_mut(id).ok_or("no such clip")?;
                if let Some(s) = arg_f64(args, "start") {
                    c.trim_start(s, headroom);
                }
                if let Some(e) = arg_f64(args, "end") {
                    c.trim_end(e, max_dur);
                }
                let (start, duration) = (c.start, c.duration);
                app.project.tidy();
                Ok(json!({"ok": true, "start": start, "end": start + duration}))
            }
            "timeline.add_transition" => {
                let right = req(arg_u64(args, "right_clip_id"), "right_clip_id")?;
                let kind = match req(arg_str(args, "kind"), "kind")? {
                    "CrossFade" => TransitionKind::CrossFade,
                    "FadeToColor" => TransitionKind::FadeToColor,
                    "Push" => TransitionKind::Push,
                    "Wipe" => TransitionKind::Wipe,
                    k => return Err(format!("unknown transition kind '{k}'")),
                };
                let dur = arg_f64(args, "duration").unwrap_or(1.0);
                // no abutting left neighbour → the clip blends in from nothing instead
                let id = app
                    .project
                    .add_transition(right, kind, dur)
                    .or_else(|| app.project.add_edge_transition(right, kind, dur, false))
                    .ok_or("clip not found")?;
                app.transitions_ui.remember(kind, dur); // Ctrl+T repeats this one too
                Ok(json!({"ok": true, "transition_id": id}))
            }
            "timeline.auto_cut" => {
                use crate::engine::autocut::{loud_segments, to_timeline, AutoCutParams};
                let ids = req(arg_ids(args, "clip_ids"), "clip_ids")?;
                let mut params = AutoCutParams::default();
                if let Some(v) = arg_f64(args, "threshold_db") {
                    params.threshold_db = v as f32;
                }
                if let Some(v) = arg_f64(args, "min_silence") {
                    params.min_silence = v;
                }
                if let Some(v) = arg_f64(args, "min_speech") {
                    params.min_speech = v;
                }
                if let Some(v) = arg_f64(args, "padding") {
                    params.padding = v;
                }
                let keep_quiet = arg_bool(args, "keep_quiet").unwrap_or(false);
                let ripple = arg_bool(args, "ripple").unwrap_or(true);
                let mut cuts = Vec::new();
                let mut removes = Vec::new();
                for &id in &ids {
                    let c = app.project.clip(id).ok_or("no such clip")?.clone();
                    if c.kind != ClipKind::Audio || c.reverse || c.freeze.is_some() {
                        continue;
                    }
                    let a = app.project.asset(c.asset).ok_or("clip has no asset")?;
                    let peaks = app
                        .waveforms
                        .get(&a.path, c.audio_stream)
                        .ok_or("waveform still computing — try again in a moment")?;
                    let segs = loud_segments(&peaks, c.src_in, c.src_len(), &params);
                    let (mut cs, mut rs) = to_timeline(&segs, c.start, c.src_in, c.duration, c.speed, keep_quiet);
                    cuts.append(&mut cs);
                    removes.append(&mut rs);
                }
                if cuts.is_empty() && removes.is_empty() {
                    return Err("no segments found (are the clips audio clips?)".into());
                }
                let n = app.project.auto_cut(&ids, &cuts, &removes, ripple);
                Ok(json!({"ok": true, "removed": n}))
            }
            "timeline.nest" => {
                let ids = req(arg_ids(args, "clip_ids"), "clip_ids")?;
                let name = arg_str(args, "name")
                    .map(String::from)
                    .unwrap_or_else(|| format!("Sequence {}", app.project.sequences.len() + 1));
                let id = app.project.nest_selection(&ids, name).ok_or("nothing to nest")?;
                Ok(json!({"ok": true, "sequence_id": id}))
            }
            "timeline.import" => {
                let path = PathBuf::from(req(arg_str(args, "path"), "path")?);
                let report = crate::engine::import::import_file(&path)?;
                let md = report.to_markdown();
                let (clips, tracks, missing) = (report.clips, report.tracks, report.missing_media);
                if arg_bool(args, "replace").unwrap_or(false) {
                    app.set_project(report.project, None);
                } else {
                    app.import_ui.report = Some(report);
                    app.import_ui.open = true;
                }
                Ok(json!({"ok": true, "clips": clips, "tracks": tracks, "missing_media": missing, "report": md}))
            }
            _ => unreachable!(),
        }
    }
    Some(run(app, name, args))
}
