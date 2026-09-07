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
            // ---- ws:snap-engine ----
            "timeline.snap_get" => Ok(json!({"enabled": app.settings.snap})),
            "timeline.snap_set" => {
                app.settings.snap = req(arg_bool(args, "enabled"), "enabled")?;
                Ok(json!({"ok": true, "enabled": app.settings.snap}))
            }
            "timeline.snap_query" => {
                let t = req(arg_f64(args, "t"), "t")?;
                let exclude = arg_ids(args, "exclude_ids").unwrap_or_default();
                let thr = crate::ui::timeline::snap_thr(app.timeline.zoom, app.project.fps);
                let hit = crate::ui::timeline::target(
                    &app.project,
                    t,
                    thr,
                    app.playhead,
                    &exclude,
                    &app.selection,
                    None,
                    app.settings.snap_markers,
                );
                Ok(match hit {
                    Some((x, kind)) => json!({"t": x, "kind": format!("{kind:?}")}),
                    None => json!({"t": Value::Null, "kind": Value::Null}),
                })
            }
            "timeline.zones" => {
                let x = req(arg_f64(args, "x"), "x")? as f32;
                let y = req(arg_f64(args, "y"), "y")? as f32;
                Ok(json!({"zone": format!("{:?}", timeline_zone_at(app, x, y))}))
            }
            "timeline.set_in_out" => {
                let (snap_on, zoom, ph) = (app.settings.snap, app.timeline.zoom, app.playhead);
                let mut changed = false;
                if let Some(v) = arg_f64(args, "in") {
                    let v = crate::ui::timeline::snap_time(v.max(0.0), snap_on, zoom, &app.project, ph, &[]);
                    let v = v.min(app.project.out_point.unwrap_or(f64::INFINITY));
                    if app.project.in_point != Some(v) {
                        app.project.in_point = Some(v);
                        changed = true;
                    }
                }
                if let Some(v) = arg_f64(args, "out") {
                    let v = crate::ui::timeline::snap_time(v.max(0.0), snap_on, zoom, &app.project, ph, &[]);
                    let v = v.max(app.project.in_point.unwrap_or(0.0));
                    if app.project.out_point != Some(v) {
                        app.project.out_point = Some(v);
                        changed = true;
                    }
                }
                Ok(json!({"ok": true, "changed": changed, "in": app.project.in_point, "out": app.project.out_point}))
            }
            _ => unreachable!(),
        }
    }
    Some(run(app, name, args))
}

// ---- ws:snap-engine ----
/// Debug hit-test for `timeline.zones`: which `arm::Zone` a screen point would land on, approximated
/// from the timeline's current layout state (no live egui frame available to an MCP caller). Ruler
/// (including the in/out handles), lane gaps, clip bodies (top/bottom split on tall rows), edges and
/// seams are covered; Drop/Fade/VolumeLine/Key/Marker/TransitionEdge need an active drag/dnd payload
/// and are not reachable from this static point-in-time query.
fn timeline_zone_at(app: &App, x: f32, y: f32) -> crate::ui::timeline::Zone {
    use crate::ui::timeline::Zone;
    let state = &app.timeline;
    if y < state.lanes_rect.top() {
        return Zone::RulerInOut;
    }
    let Some(ti) = state.track_at(y, &app.project) else { return Zone::Lane };
    let track = &app.project.tracks[ti];
    let t = state.time_at(x);
    const EDGE_PX_PAD: f32 = 6.0; // matches EDGE_W's on-screen 6 px, converted per-call via zoom
    let edge_thr = (EDGE_PX_PAD / state.zoom.max(0.01)) as f64;
    let mut ordered: Vec<&crate::model::Clip> = track.clips.iter().collect();
    ordered.sort_by(|a, b| a.start.total_cmp(&b.start));
    for w in ordered.windows(2) {
        if (w[0].end() - w[1].start).abs() < crate::model::ABUT_EPS && (t - w[0].end()).abs() * state.zoom as f64 <= 3.0
        {
            return Zone::Seam;
        }
    }
    for cl in &track.clips {
        if !cl.contains(t) {
            continue;
        }
        if (t - cl.start).abs() <= edge_thr {
            return Zone::EdgeStart;
        }
        if (t - cl.end()).abs() <= edge_thr {
            return Zone::EdgeEnd;
        }
        let row_top = crate::ui::timeline::row_top(state, &app.project, ti).unwrap_or(0.0);
        let split = track.height >= 2.0 * crate::ui::timeline::MIN_TRACK_H;
        return if split && y >= row_top + track.height / 2.0 { Zone::BodyBottom } else { Zone::Body };
    }
    Zone::Lane
}

// ---- ws:registries-schema-hooks ----
// One `ToolDef` per tool above, wired to `dispatch` by name — bodies untouched. Kind mirrors the old
// hand-kept mutating-tool name list exactly (project.new/open/save self-manage the undo stack via
// `set_project`/plain file I/O, so they stay `Read` here just as they were absent from that list).
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};

macro_rules! row {
    ($name:literal, $kind:expr, $desc:literal, $args:expr) => {
        ToolDef {
            name: $name,
            desc: $desc,
            args: $args,
            kind: $kind,
            run: |a, v| dispatch(a, $name, v).unwrap().map(ToolOutcome::Done),
        }
    };
}

pub const TOOLS: &[ToolDef] = &[
    row!("project.summary", ToolKind::Read, "Project overview: format, duration, tracks, clips per track, assets, sequences, subtitles count, planner progress, notes — and the markdown style summary.", &[]),
    row!("project.get", ToolKind::Read, "Full project JSON (the .sedit document).", &[]),
    row!("project.new", ToolKind::Read, "New empty project (discards unsaved changes).", &["width:integer:false:default 1920", "height:integer:false:default 1080", "fps:number:false:default 30"]),
    row!("project.open", ToolKind::Read, "Open a .sedit project or a media file (creates a project around it).", &["path:string:true:absolute path"]),
    row!("project.save", ToolKind::Read, "Save the project (.sedit). Without a path: the current project file (error if none).", &["path:string:false:.sedit path"]),
    row!("project.set", ToolKind::Mutate, "Change project format/name.", &["name:string:false:", "width:integer:false:", "height:integer:false:", "fps:number:false:"]),
    row!("timeline.list", ToolKind::Read, "Tracks and clips of the timeline being edited (main or the open sequence): ids, kind, asset, start, duration, src_in, speed, effects, label.", &[]),
    row!("timeline.add_clip", ToolKind::Mutate, "Place an asset, a sequence, or a new text clip at a time.", &["asset_id:integer:false:", "sequence_id:integer:false:", "text:string:false:creates a text clip", "at:number:true:timeline seconds", "track:integer:false:video track index", "duration:number:false:text/image length"]),
    row!("timeline.split", ToolKind::Mutate, "Split clips at t (all clips crossing t when clip_ids is omitted).", &["t:number:true:", "clip_ids:array:false:"]),
    row!("timeline.delete", ToolKind::Mutate, "Delete clips (linked clips follow).", &["clip_ids:array:true:", "ripple:boolean:false:close the gap"]),
    row!("timeline.move", ToolKind::Mutate, "Move clips by dt seconds (and dtrack tracks within their kind).", &["clip_ids:array:true:", "dt:number:true:", "dtrack:integer:false:"]),
    row!("timeline.trim", ToolKind::Mutate, "Trim a clip's edges to new timeline times.", &["clip_id:integer:true:", "start:number:false:new start", "end:number:false:new end"]),
    row!("timeline.add_transition", ToolKind::Mutate, "Transition at the cut on the left of a clip (a fade-in from nothing when no clip abuts there).", &["right_clip_id:integer:true:", "kind:string:true:CrossFade|FadeToColor|Push|Wipe", "duration:number:false:default 1"]),
    row!("timeline.auto_cut", ToolKind::Mutate, "Silence-based auto-cut of audio clips (+ linked video).", &["clip_ids:array:true:", "threshold_db:number:false:default -35", "min_silence:number:false:", "min_speech:number:false:", "padding:number:false:", "keep_quiet:boolean:false:", "ripple:boolean:false:default true"]),
    row!("timeline.nest", ToolKind::Mutate, "Nest clips into a new sequence; returns the sequence id.", &["clip_ids:array:true:", "name:string:false:"]),
    row!("timeline.import", ToolKind::Read, "Import a timeline from another editor (FCP7 XML, EDL, .prproj); returns the report and opens it in the app (replace=true swaps the project in).", &["path:string:true:", "replace:boolean:false:"]),
    // ---- ws:snap-engine ----
    row!("timeline.snap_get", ToolKind::Read, "Current snapping-enabled state.", &[]),
    row!("timeline.snap_set", ToolKind::Ui, "Toggle snapping (mirrors the bare-S hotkey); not project data, no undo.", &["enabled:boolean:true:turn snapping on/off"]),
    row!("timeline.snap_query", ToolKind::Read, "Runs the tiered snap engine (playhead > cursor > selected edge > adjacent edge > marker > transition edge > in/out > 0) and returns the hit and its tier.", &["t:number:true:pointer time to test", "exclude_ids:array:false:clip ids to exclude from candidates"]),
    row!("timeline.zones", ToolKind::Read, "Debug hit-test: which arm.rs Zone a point would land on (Body/BodyBottom/Edge/Seam/Lane/RulerInOut/...).", &["x:number:true:screen x", "y:number:true:screen y"]),
    row!("timeline.set_in_out", ToolKind::Mutate, "Sets in/out, snapped, clamped in<=out; one undo pushed only if changed.", &["in:number:false:new in point (seconds)", "out:number:false:new out point (seconds)"]),
];
