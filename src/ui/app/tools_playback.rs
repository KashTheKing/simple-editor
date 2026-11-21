use super::playback_ctl;
use super::thumbs::*;
use super::tools_helpers::*;
use super::*;

pub(super) fn dispatch(app: &mut App, name: &str, args: &Value) -> Option<Result<Value, String>> {
    let prefix = name.split('.').next().unwrap_or("");
    if !matches!(prefix, "playback" | "frame" | "render" | "sequence" | "templates") {
        return None;
    }
    pub(super) fn run(app: &mut App, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "sequence.list" => {
                let list: Vec<Value> = app
                    .project
                    .sequences
                    .iter()
                    .map(|s| {
                        // the open one's live length / Max length are swapped into the project
                        let open = app.project.editing == Some(s.id);
                        let max = if open { app.project.max_duration } else { s.max_duration };
                        json!({"id": s.id, "name": s.name, "width": s.width, "height": s.height,
                               "fps": s.fps, "duration": app.project.sequence_duration(s.id), "max_duration": max})
                    })
                    .collect();
                Ok(json!(list))
            }
            "sequence.open" => {
                // ws:timeline-tabs: opens (or shows) the sequence's tab; no id = the main timeline's
                let id = arg_u64(args, "id");
                if !super::timeline_tabs::switch(app, id) {
                    return Err("no such sequence".into());
                }
                Ok(json!({"ok": true, "editing": app.project.editing}))
            }
            "render.frame" => {
                let t = req(arg_f64(args, "t"), "t")?;
                let w = arg_u64(args, "width").map(|w| w as u32).unwrap_or(640).clamp(16, 3840);
                let frame = app.render_frame_now(t, w).ok_or("render timed out")?;
                let png = mcp::png_encode(&frame);
                Ok(json!({
                    "width": frame.width, "height": frame.height,
                    "data_url": format!("data:image/png;base64,{}", base64(&png)),
                }))
            }
            "playback.seek" => {
                let t = req(arg_f64(args, "t"), "t")?;
                app.seek(t);
                Ok(json!({"ok": true, "t": app.playhead}))
            }
            "playback.play" => {
                app.play_gated(1.0); // ws:play-gate: returns at once; `playback.status` says when it has started
                let mut out = app.transport_json();
                out["ok"] = json!(true);
                Ok(out)
            }
            "playback.pause" => {
                app.player.pause();
                app.playhead = app.player.time();
                Ok(json!({"ok": true, "t": app.playhead}))
            }
            "templates.list" => {
                let templates: Vec<&str> = app.settings.templates.iter().map(|t| t.name.as_str()).collect();
                let motions: Vec<String> = crate::engine::presets::builtin_motions()
                    .iter()
                    .map(|m| m.name.clone())
                    .chain(app.settings.motion_presets.iter().map(|m| m.name.clone()))
                    .collect();
                Ok(json!({"templates": templates, "builtin": crate::engine::templates::BUILTIN_NAMES, "motion_presets": motions}))
            }
            "templates.apply" => {
                let name = req(arg_str(args, "name"), "name")?;
                let at = req(arg_f64(args, "at"), "at")?;
                // (place_clips, with the template's layers and transitions; slots: templates.use)
                let d = super::template_ctl::find(app, name).ok_or_else(|| format!("no template '{name}'"))?;
                let ids = crate::engine::templates::apply(&mut app.project, d, at, &[]);
                Ok(json!({"ok": true, "clip_ids": ids}))
            }
            // ---------------- round 3 ----------------
            "frame.export" => {
                let out = PathBuf::from(req(arg_str(args, "path"), "path")?);
                let t = arg_f64(args, "t").unwrap_or(app.playhead);
                let (pw, ph) = (app.project.width.max(16), app.project.height.max(16));
                let w = arg_u64(args, "width").map(|w| w as u32).unwrap_or(pw).clamp(16, 7680);
                let h = arg_u64(args, "height")
                    .map(|h| h as u32)
                    .unwrap_or_else(|| ((ph as u64 * w as u64) / pw as u64).max(1) as u32)
                    .clamp(16, 4320);
                let opts = frame_ui::FrameExport {
                    out: out.clone(),
                    size: (w, h),
                    scaler: app.project.scaler,
                    resize: arg_str(args, "resize").unwrap_or(&app.settings.export_scaler).to_string(),
                    with_effects: arg_bool(args, "with_effects").unwrap_or(true),
                    quality: arg_u64(args, "quality").map(|q| q as u32).unwrap_or(app.settings.frame_quality),
                };
                let (rw, _) = frame_render_size((pw, ph), opts.size);
                let frame = app.render_frame_sized(t, rw).ok_or("render timed out")?;
                write_image(&frame, &opts)?;
                Ok(json!({"ok": true, "path": out.to_string_lossy(), "width": w, "height": h}))
            }
            // ---- ws:player-rate-loop ----
            "playback.rate" => {
                let rate = req(arg_f64(args, "rate"), "rate")?;
                if rate == 0.0 {
                    return Err("rate must not be 0 (use playback.pause)".into());
                }
                let rate = rate.clamp(-8.0, 8.0);
                app.play_gated(rate); // starts at `rate` (after buffering), or sets it if already going
                Ok(json!({"ok": true, "rate": app.transport_rate()}))
            }
            "playback.step" => {
                let frames = req(arg_f64(args, "frames"), "frames")?.round() as i64;
                app.player.step(frames, app.project.fps);
                app.playhead = app.player.time();
                Ok(json!({"ok": true, "t": app.playhead}))
            }
            "playback.loop" => {
                let on = req(arg_bool(args, "on"), "on")?;
                if on {
                    let a = arg_f64(args, "in").or(app.project.in_point).unwrap_or(0.0);
                    let b = arg_f64(args, "out").or(app.project.out_point).unwrap_or(app.project.duration());
                    if b <= a {
                        return Err("'out' must be after 'in'".into());
                    }
                    app.player.set_loop(Some((a, b)));
                } else {
                    app.player.set_loop(None);
                }
                Ok(json!({"ok": true, "loop_range": app.player.loop_range()}))
            }
            "playback.play_range" => {
                let mode = req(arg_str(args, "mode"), "mode")?;
                if !matches!(mode, "in_out" | "around" | "to_out") {
                    return Err("mode must be 'in_out' | 'around' | 'to_out'".into());
                }
                playback_ctl::start_play_range(app, mode);
                Ok(json!({"ok": true}))
            }
            "playback.scrub" => {
                let t = req(arg_f64(args, "t"), "t")?;
                if app.transport_playing() {
                    return Err("scrub only works while paused".into());
                }
                app.player.scrub(t);
                Ok(json!({"ok": true}))
            }
            "playback.trace" => {
                let secs = arg_f64(args, "secs").unwrap_or(3.0).clamp(0.1, 60.0);
                let path = std::env::temp_dir().join(format!("simple-editor-trace-{}.json", std::process::id()));
                let _ = std::fs::remove_file(&path);
                crate::perf::start();
                let p = path.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs_f64(secs));
                    crate::perf::stop();
                    let tmp = p.with_extension("tmp");
                    if std::fs::write(&tmp, crate::perf::trace_json()).is_ok() {
                        let _ = std::fs::rename(&tmp, &p);
                    }
                });
                Ok(json!({"ok": true, "path": path, "ready_in_s": secs}))
            }
            "playback.status" => {
                let (published, worst_gap_ms) = app.player.publish_stats();
                let mut out = json!({
                    "rate": app.transport_rate(),
                    "playing": app.player.is_playing(),
                    "time": app.player.time(),
                    "published_frames": published,
                    "worst_gap_ms": worst_gap_ms,
                    "dropped_frames": app.player.dropped_frames(),
                    "loop_range": app.player.loop_range(),
                    // ---- ws:perf-baseline ---- (the HUD's numbers; ms, rolling over ~120 UI frames)
                    "ui_frame_ms": app.hud.frame.avg(),
                    "ui_frame_p95_ms": app.hud.frame.p95(),
                    "gl_composite_ms": app.hud.composite.avg(),
                    "upload_ms": app.hud.upload.avg(),
                    "decode_ms": app.hud.decode_avg_ms,
                    "decode_worst_ms": app.hud.decode_worst_ms,
                    "cache_hit_rate": app.hud.hit_rate,
                });
                // ---- ws:play-gate ---- state / buffering / buffer_fraction / settled / ...
                if let (Some(o), Value::Object(t)) = (out.as_object_mut(), app.transport_json()) {
                    o.extend(t);
                }
                Ok(out)
            }
            // ---- ws:perf-baseline ----
            "playback.hud" => {
                app.settings.hud = arg_bool(args, "on").unwrap_or(!app.settings.hud);
                app.settings.save();
                Ok(json!({"ok": true, "hud": app.settings.hud}))
            }
            "render.layers_async" => {
                let t = req(arg_f64(args, "t"), "t")?;
                let max_w = arg_u64(args, "max_w").map(|w| w as u32).unwrap_or(640).clamp(16, 3840);
                let id = app.player.request_layers(t, max_w);
                Ok(json!({"ok": true, "id": id}))
            }
            "render.poll_layers" => {
                let id = req(arg_u64(args, "id"), "id")?;
                match app.player.take_layers_reply() {
                    Some((rid, set)) if rid == id => Ok(json!({"ready": true, "layers": set.layers.len()})),
                    _ => Ok(json!({"ready": false})),
                }
            }
            _ => unreachable!(),
        }
    }
    Some(run(app, name, args))
}

// ---- ws:perf-baseline ----
/// ACT_HANDLERS entry: Playback ▸ Performance HUD.
pub(super) fn act_hud(app: &mut App, a: Action) -> bool {
    if a != Action::ToggleHud {
        return false;
    }
    app.settings.hud = !app.settings.hud;
    app.settings.save();
    true
}

// ---- ws:registries-schema-hooks ----
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
    row!("sequence.list", ToolKind::Read, "Sequences (id, name, format, duration, max_duration).", &[]),
    // not in the old hand-kept mutating-tool name list (switching the edited sequence isn't itself an undoable edit)
    row!(
        "sequence.open",
        ToolKind::Read,
        "Edit a sequence: opens its timeline tab (or shows it when open already); omit id for the main timeline. `timeline.tabs` lists / closes tabs.",
        &["id:integer:false:"]
    ),
    // ---- ws:mcp-context ---- an image result (MCP image content; Luau still gets {width,height,data_url})
    ToolDef {
        name: "render.frame",
        desc: "Render the timeline at time t as a PNG image, max `width` px wide (default 640). `guide` draws the Platform Emulator's guide over it (default: the Settings 'draw in render' option; a device preview comes back as the device picture).",
        args: &["t:number:true:", "width:integer:false:", "guide:boolean:false:draw the platform guide (guides.set)", "playing:boolean:false:the guide's play button state (default: the editor's)"],
        kind: ToolKind::Read,
        run: super::tools_context::render_frame_image,
    },
    row!("playback.seek", ToolKind::Read, "Move the playhead.", &["t:number:true:"]),
    row!("playback.play", ToolKind::Read, "Start playback. With Settings > Performance > \"Buffer before playing\" on (default) it first waits, at most 3 s, until the next second of frames and audio is ready, so it returns at once with `state` \"buffering\" or \"playing\": poll `playback.status` until `settled` is true.", &[]),
    row!("playback.pause", ToolKind::Read, "Pause playback.", &[]),
    row!("templates.list", ToolKind::Read, "Saved clip templates, the built-in animation templates and motion presets.", &[]),
    row!(
        "templates.apply",
        ToolKind::Mutate,
        "Place a template (saved or built-in) at a time, slots left as they are (templates.use fills them).",
        &["name:string:true:", "at:number:true:"]
    ),
    row!(
        "frame.export",
        ToolKind::Read,
        "Save the frame at time t as PNG/JPG/WebP (by the path's extension).",
        &[
            "path:string:true:",
            "t:number:false:default playhead",
            "width:integer:false:",
            "height:integer:false:",
            "with_effects:boolean:false:default true",
            "quality:integer:false:1..100 for JPG/WebP",
            "resize:string:false:neighbor|bilinear|bicubic|lanczos"
        ]
    ),
    // ---- ws:player-rate-loop ----
    row!(
        "playback.rate",
        ToolKind::Ui,
        "Set shuttle/playback rate (starts playing if paused); -8..8, 0 rejected (use playback.pause).",
        &["rate:number:true:playback speed, -8..8"]
    ),
    row!(
        "playback.step",
        ToolKind::Ui,
        "Step the playhead by N frames (negative = back); pauses first.",
        &["frames:integer:true:signed frame count"]
    ),
    row!(
        "playback.loop",
        ToolKind::Ui,
        "Enable/disable Loop In->Out playback.",
        &[
            "on:boolean:true:",
            "in:number:false:defaults to Project.in_point",
            "out:number:false:defaults to Project.out_point"
        ]
    ),
    row!(
        "playback.play_range",
        ToolKind::Ui,
        "Play In->Out, Play Around Playhead, or Play to Out, auto-stopping at the target.",
        &["mode:string:true:'in_out' | 'around' | 'to_out'"]
    ),
    row!(
        "playback.scrub",
        ToolKind::Ui,
        "Emit one BLOCK (~21ms) of audio at t without moving the clock (paused only).",
        &["t:number:true:timeline seconds"]
    ),
    row!("playback.trace", ToolKind::Read, "Record timing spans (UI frame, timeline and inspector draw, render-thread frame, layer decode, GPU composite and upload) for `secs` (default 3) and write a Chrome-trace JSON (chrome://tracing, Perfetto). Returns the file `path` at once; it appears when the recording ends (`ready_in_s`).", &["secs:number:false:recording length, 0.1-60"]),
    row!("playback.status", ToolKind::Read, "Current rate, time, dropped and published frame counts, longest wait between two frames since the last call (ms), state (playing / buffering / paused), `buffering` with `buffer_fraction` (0..1 or null) and `waiting_on` while Play waits for the frames ahead or the player refills after a stall, `settled` (true when nothing is buffering: poll for it), `start_wait_ms` / `start_capped` (how the last Play started), `audio_start_lag_ms`, loop range, and the performance HUD's numbers (UI frame avg/p95, GL composite, texture upload, decode avg/worst ms, cache hit rate).", &[]),
    // ---- ws:perf-baseline ---- (the ToolDef twin of Action::ToggleHud)
    row!(
        "playback.hud",
        ToolKind::Ui,
        "Show or hide the performance HUD over the Preview; omit `on` to toggle.",
        &["on:boolean:false:"]
    ),
    row!(
        "render.layers_async",
        ToolKind::Ui,
        "Queue a non-blocking one-shot layer decode; returns a request id.",
        &["t:number:true:", "max_w:integer:false:default 640"]
    ),
    row!(
        "render.poll_layers",
        ToolKind::Read,
        "Poll for the async layer-decode reply (not ready until it matches the given id, or it was \
         superseded by a newer request).",
        &["id:integer:true:id returned by render.layers_async"]
    ),
];
