//! ---- ws:canvas-handles-monitor ----
//! MCP tools for this workstream's canvas/monitor capabilities: clip.crop, clip.fit, clip.mask_target,
//! preview.hover, preview.view, preview.drop, timeline.reframe, playhead.set_timecode.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::preview::{self, MaskTarget};

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "clip.crop",
        desc: "Find-or-append a Crop effect on the clip and set its fraction params (0..0.5 each) at time `at`.",
        args: &[
            "clip_id:integer:true:",
            "left:number:false:0..0.5",
            "right:number:false:0..0.5",
            "top:number:false:0..0.5",
            "bottom:number:false:0..0.5",
            "feather:number:false:0..0.5",
            "at:number:false:default playhead",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = req(arg_u64(args, "clip_id"), "clip_id")?;
            let at = arg_f64(args, "at").unwrap_or(app.playhead);
            let c = app.project.clip_mut(id).ok_or("no such clip")?;
            let lt = c.local(at);
            let e = preview::crop_effect(c);
            for (i, k) in ["left", "right", "top", "bottom", "feather"].iter().enumerate() {
                if let Some(v) = arg_f64(args, k) {
                    e.params[i].set_at(lt, v.clamp(0.0, 0.5));
                }
            }
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
    ToolDef {
        name: "clip.fit",
        desc: "Fit (contain, native aspect) or stretch (fill canvas, non-uniform) the clip.",
        args: &["clip_id:integer:true:", "mode:string:true:fit|stretch"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = req(arg_u64(args, "clip_id"), "clip_id")?;
            let mode = req(arg_str(args, "mode"), "mode")?;
            let stretch = match mode {
                "fit" => false,
                "stretch" => true,
                _ => return Err("mode: fit|stretch".into()),
            };
            if app.project.fit_clip_to_screen(id, stretch) {
                Ok(ToolOutcome::Done(json!({"ok": true})))
            } else {
                Err("no such clip, or it has no native size to fit".into())
            }
        },
    },
    ToolDef {
        name: "clip.mask_target",
        desc: "Point the existing mask-tool canvas drag at the clip's own mask, or one effect's.",
        args: &["effect:integer:false:omit = the clip's own mask"],
        kind: ToolKind::Ui,
        run: |app, args| {
            app.preview.mask_target = match arg_u64(args, "effect") {
                Some(i) => MaskTarget::Effect(i as usize),
                None => MaskTarget::Clip,
            };
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
    ToolDef {
        name: "preview.hover",
        desc: "Manually drive the monitor's alt-render preview. Same App.alt_render pipeline inspector-gallery's gallery.hover targets via AltRequest::Gallery.",
        args: &["kind:string:false:effect|transition|off", "name:string:false:EffectKind/TransitionKind name"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let kind = arg_str(args, "kind").unwrap_or("off");
            let want = match kind {
                "off" => None,
                "effect" => {
                    let n = req(arg_str(args, "name"), "name")?;
                    let k = EffectKind::ALL
                        .into_iter()
                        .find(|k| k.name().eq_ignore_ascii_case(n))
                        .ok_or_else(|| format!("unknown effect '{n}'"))?;
                    Some(monitor::AltRequest::Effect(k))
                }
                "transition" => {
                    let n = req(arg_str(args, "name"), "name")?;
                    let k = TransitionKind::ALL
                        .into_iter()
                        .find(|k| k.name().eq_ignore_ascii_case(n))
                        .ok_or_else(|| format!("unknown transition '{n}'"))?;
                    Some(monitor::AltRequest::Transition(k))
                }
                _ => return Err("kind: effect|transition|off".into()),
            };
            app.alt_render.request(want);
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
    ToolDef {
        name: "preview.view",
        desc: "Read or set the canvas zoom/pan.",
        args: &["zoom:number:false:", "pan_x:number:false:", "pan_y:number:false:", "fit:boolean:false:reset to 1.0/0,0"],
        kind: ToolKind::Ui,
        run: |app, args| {
            if arg_bool(args, "fit") == Some(true) {
                app.preview.view = monitor::fit_view();
            } else {
                if let Some(z) = arg_f64(args, "zoom") {
                    app.preview.view.0 = z as f32;
                }
                if let Some(x) = arg_f64(args, "pan_x") {
                    app.preview.view.1.x = x as f32;
                }
                if let Some(y) = arg_f64(args, "pan_y") {
                    app.preview.view.1.y = y as f32;
                }
            }
            let (zoom, pan) = app.preview.view;
            Ok(ToolOutcome::Done(json!({"zoom": zoom, "pan_x": pan.x, "pan_y": pan.y})))
        },
    },
    ToolDef {
        name: "preview.drop",
        desc: "Place assets on a free video track at the playhead, as if dropped on the monitor.",
        args: &["asset_ids:array:true:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let ids = req(arg_ids(args, "asset_ids"), "asset_ids")?;
            let playhead = app.playhead;
            app.place_assets(&ids, playhead, None, DropMode::Place);
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
    ToolDef {
        name: "timeline.reframe",
        desc: "Auto-reframe using the clip's EXISTING tracked box only (no auto-detect). Errors cleanly when none exists.",
        args: &["clip_id:integer:true:", "ratio:string:false:reserved — no anchor field to drive it yet, unused this wave"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = req(arg_u64(args, "clip_id"), "clip_id")?;
            let App { project, tracking, selection, .. } = app;
            monitor::reframe(project, tracking, selection, Some(id))
                .map(|()| ToolOutcome::Done(json!({"ok": true})))
                .map_err(|e| e.to_string())
        },
    },
    ToolDef {
        name: "playhead.set_timecode",
        desc: "Parse a timecode/relative string and seek — same parser as the transport label's click-to-edit.",
        args: &["text:string:true:hh:mm:ss:ff | mm:ss | +N | -N | +1.5s"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let text = req(arg_str(args, "text"), "text")?;
            let (fps, playhead) = (app.project.fps, app.playhead);
            match crate::ui::parse_timecode(text, fps, playhead) {
                Some(t) => {
                    app.seek(t);
                    Ok(ToolOutcome::Done(json!({"ok": true, "t": t})))
                }
                None => Err(format!("couldn't parse timecode '{text}'")),
            }
        },
    },
];
