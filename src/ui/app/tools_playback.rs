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
                        json!({"id": s.id, "name": s.name, "width": s.width, "height": s.height,
                               "fps": s.fps, "duration": s.duration()})
                    })
                    .collect();
                Ok(json!(list))
            }
            "sequence.open" => {
                match arg_u64(args, "id") {
                    Some(id) => {
                        if !app.project.open_sequence(id) {
                            return Err("no such sequence (or already open / cycle)".into());
                        }
                    }
                    None => app.project.close_sequence(),
                }
                app.after_edit();
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
                app.player.play();
                Ok(json!({"ok": true}))
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
                Ok(json!({"templates": templates, "motion_presets": motions}))
            }
            "templates.apply" => {
                let name = req(arg_str(args, "name"), "name")?;
                let at = req(arg_f64(args, "at"), "at")?;
                let tpl = app
                    .settings
                    .templates
                    .iter()
                    .find(|t| t.name.eq_ignore_ascii_case(name))
                    .cloned()
                    .ok_or_else(|| format!("no template '{name}'"))?;
                let (clips, assets) = crate::engine::presets::decode_template(&tpl).ok_or("template is corrupted")?;
                let ids = app.project.place_clips(clips, assets, at);
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
                let frame = app.render_frame_now(t, rw).ok_or("render timed out")?;
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
                if !app.player.is_playing() {
                    app.player.play();
                }
                app.player.set_rate(rate);
                Ok(json!({"ok": true, "rate": app.player.rate()}))
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
                if app.player.is_playing() {
                    return Err("scrub only works while paused".into());
                }
                app.player.scrub(t);
                Ok(json!({"ok": true}))
            }
            "playback.status" => Ok(json!({
                "rate": app.player.rate(),
                "dropped_frames": app.player.dropped_frames(),
                "buffering": app.player.is_buffering(),
                "loop_range": app.player.loop_range(),
            })),
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
    row!("sequence.list", ToolKind::Read, "Sequences (id, name, format, duration).", &[]),
    // not in the old hand-kept mutating-tool name list (switching the edited sequence isn't itself an undoable edit)
    row!(
        "sequence.open",
        ToolKind::Read,
        "Edit a sequence (swap it into the timeline); omit id to go back to the main timeline.",
        &["id:integer:false:"]
    ),
    row!(
        "render.frame",
        ToolKind::Read,
        "Render the timeline at time t as a PNG (base64 data url), max `width` px wide (default 640).",
        &["t:number:true:", "width:integer:false:"]
    ),
    row!("playback.seek", ToolKind::Read, "Move the playhead.", &["t:number:true:"]),
    row!("playback.play", ToolKind::Read, "Start playback.", &[]),
    row!("playback.pause", ToolKind::Read, "Pause playback.", &[]),
    row!("templates.list", ToolKind::Read, "Saved clip templates and motion presets.", &[]),
    row!(
        "templates.apply",
        ToolKind::Mutate,
        "Place a saved template at a time.",
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
    row!("playback.status", ToolKind::Read, "Current rate, dropped-frame count, buffering flag, and loop range.", &[]),
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
