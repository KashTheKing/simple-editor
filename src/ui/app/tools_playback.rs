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
            _ => unreachable!(),
        }
    }
    Some(run(app, name, args))
}
