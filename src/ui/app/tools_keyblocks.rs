//! ---- ws:keyframe-blocks ----
//! MCP surface of keyframe blocks (`model::keyblocks`): list / apply (chaining) / retime / ease / remove,
//! plus save / forget for the user's own blocks (`Settings.key_blocks`, no undo - like other presets).

use super::tools_args::Args;
use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::{builtin_blocks, find_block, Ease};

fn done(v: Value) -> Result<ToolOutcome, String> {
    Ok(ToolOutcome::Done(v))
}

/// `parse_ease`'s names plus the curve presets ("Smooth", "Overshoot", ...).
fn ease_arg(s: &str) -> Option<Ease> {
    parse_ease(s).or_else(|| Ease::PRESETS.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)).map(|p| p.1))
}

/// Apply block `name` to `ids` at timeline time `at` (a clip that does not contain it chains after its
/// last block instead); `None` = chain on every clip. Shared by the tool, the Gallery click and a
/// timeline drop. Returns how many clips took it.
pub(super) fn apply_block(
    app: &mut App,
    name: &str,
    ids: &[Id],
    at: Option<f64>,
    dur: Option<f64>,
) -> Result<usize, String> {
    let block = find_block(&app.settings.key_blocks, name).ok_or_else(|| format!("no such block '{name}'"))?;
    let mut n = 0;
    for &id in ids {
        let Some(c) = app.project.clip(id) else { continue };
        let local = at.filter(|&t| c.contains(t)).map(|t| t - c.start);
        n += app.project.apply_key_block(id, &block, local, dur).is_some() as usize;
    }
    Ok(n)
}

/// Capture clip `id`'s keys in [from, to] (clip-local, default whole clip) into `Settings.key_blocks`.
pub(super) fn save_block(app: &mut App, id: Id, name: &str, from: Option<f64>, to: Option<f64>) -> Result<(), String> {
    let (w, h) = (app.project.width as f64, app.project.height as f64);
    let c = app.project.clip(id).ok_or("no such clip")?;
    let b = c
        .capture_block(name, from.unwrap_or(0.0), to.unwrap_or(c.duration), w, h)
        .ok_or("need keys at two or more times on position / scale / rotation / opacity / volume")?;
    app.settings.key_blocks.retain(|o| !o.name.eq_ignore_ascii_case(name));
    app.settings.key_blocks.push(b);
    app.settings.save();
    Ok(())
}

fn clip_block(args: &Value) -> Result<(Id, usize), String> {
    Ok((req(arg_u64(args, "clip_id"), "clip_id")?, req(arg_u64(args, "index"), "index")? as usize))
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "keyblocks.list",
        desc: "Keyframe blocks (builtins, then Settings.key_blocks): name, default seconds, animated properties. With clip_id: the blocks applied to that clip (index, name, clip-local t, dur).",
        args: &["clip_id:integer:false:omit=the catalogue"],
        kind: ToolKind::Read,
        run: |app, args| {
            if let Some(id) = arg_u64(args, "clip_id") {
                let c = app.project.clip(id).ok_or("no such clip")?;
                let v: Vec<Value> = c
                    .blocks
                    .iter()
                    .enumerate()
                    .map(|(i, b)| json!({"index": i, "name": b.name, "t": b.t, "dur": b.dur, "props": b.props}))
                    .collect();
                return done(json!(v));
            }
            let row = |b: &crate::model::KeyBlock, user: bool| {
                let props: Vec<&String> = b.tracks.iter().map(|t| &t.prop).collect();
                json!({"name": b.name, "dur": b.dur, "user": user, "props": props})
            };
            let mut v: Vec<Value> = builtin_blocks().iter().map(|b| row(b, false)).collect();
            v.extend(app.settings.key_blocks.iter().map(|b| row(b, true)));
            done(json!(v))
        },
    },
    ToolDef {
        name: "keyblocks.apply",
        desc: "Apply a keyframe block to clip_ids (default selection) at timeline time `at` (default playhead; a time inside an applied block chains after it). chain:true appends after each clip's last block instead. dur overrides the block's length. Only writes ordinary keys.",
        args: &[
            "name:string:true:",
            "clip_ids:array:false:defaults to selection",
            "at:number:false:timeline seconds, default playhead",
            "chain:boolean:false:append after the last block",
            "dur:number:false:seconds",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?.to_string();
            let ids = Args(args).ids("clip_ids").unwrap_or_else(|| app.selection.clone());
            let at = (!arg_bool(args, "chain").unwrap_or(false)).then(|| Args(args).t_or_playhead("at", app));
            let n = apply_block(app, &name, &ids, at, arg_f64(args, "dur"))?;
            if n == 0 {
                return Err("no clip took the block (select a clip with room for it)".into());
            }
            done(json!({"ok": true, "count": n}))
        },
    },
    ToolDef {
        name: "keyblocks.retime",
        desc: "Move / stretch applied block `index` of clip_id: new clip-local start t and length dur (either optional); its keys move with it.",
        args: &["clip_id:integer:true:", "index:integer:true:", "t:number:false:", "dur:number:false:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let (id, i) = clip_block(args)?;
            let b = app.project.clip(id).and_then(|c| c.blocks.get(i)).ok_or("no such block")?.clone();
            let (t, d) = (arg_f64(args, "t").unwrap_or(b.t), arg_f64(args, "dur").unwrap_or(b.dur));
            app.project.retime_key_block(id, i, t, d);
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "keyblocks.ease",
        desc: "Set the easing of every segment inside applied block `index` (Linear|EaseIn|EaseOut|EaseInOut|Hold|cubic-bezier(..)|a curve preset name like Smooth/Snap/Overshoot).",
        args: &["clip_id:integer:true:", "index:integer:true:", "ease:string:true:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let (id, i) = clip_block(args)?;
            let e = req(arg_str(args, "ease").and_then(ease_arg), "ease")?;
            if !app.project.ease_key_block(id, i, e) {
                return Err("no such block".into());
            }
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "keyblocks.remove",
        desc: "Delete applied block `index` from clip_id together with the keys it wrote (a seam key shared with a chained neighbour stays).",
        args: &["clip_id:integer:true:", "index:integer:true:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let (id, i) = clip_block(args)?;
            if !app.project.remove_key_block(id, i) {
                return Err("no such block".into());
            }
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "keyblocks.save",
        desc: "Save a clip's keys in clip-local [from, to] (default: the whole clip) as your own keyframe block in Settings.key_blocks (replaces one of the same name). No undo.",
        args: &[
            "name:string:true:",
            "clip_id:integer:false:default first selected",
            "from:number:false:",
            "to:number:false:",
        ],
        kind: ToolKind::Ui,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?.trim().to_string();
            if name.is_empty() {
                return Err("name is empty".into());
            }
            let id = arg_u64(args, "clip_id").or(app.selection.first().copied()).ok_or("select a clip")?;
            save_block(app, id, &name, arg_f64(args, "from"), arg_f64(args, "to"))?;
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "keyblocks.forget",
        desc: "Delete one of your saved keyframe blocks (Settings.key_blocks). No undo.",
        args: &["name:string:true:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?;
            let n = app.settings.key_blocks.len();
            app.settings.key_blocks.retain(|b| !b.name.eq_ignore_ascii_case(name));
            if n == app.settings.key_blocks.len() {
                return Err(format!("no saved block '{name}'"));
            }
            app.settings.save();
            done(json!({"ok": true}))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_arg_takes_kinds_and_presets() {
        assert_eq!(ease_arg("EaseOut"), Some(Ease::EaseOut));
        assert_eq!(ease_arg("overshoot"), Some(Ease::PRESETS[4].1));
        assert_eq!(ease_arg("nope"), None);
    }

    #[test]
    fn saved_blocks_shadow_builtins_by_name() {
        let mut mine = builtin_blocks()[0].clone();
        mine.dur = 9.0;
        assert_eq!(find_block(&[mine], "fade in").unwrap().dur, 9.0);
        assert!(find_block(&[], "Shake").is_some());
    }
}
