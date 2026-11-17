//! ---- ws:animator ----
//! App side of the Animator (`ui::animator`): the pane, Stack with Overlays, block controls and Flow over MCP.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::{find_block, AnimLink, BlockCtl, Dir, Match};

/// Onion skin follows the Animator: off whenever it wasn't on screen last frame.
pub(super) fn frame(app: &mut App, _ctx: &egui::Context) {
    if !app.pane_drawn(Pane::Animator) {
        crate::ui::animator::set_onion(0);
    }
}

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::Animator {
        return false;
    }
    let playing = app.player.is_playing();
    let resp = {
        let App { project, selection, playhead, undo, redo, animator: st, palette, settings, .. } = app;
        let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
        crate::ui::animator::show(
            ui,
            st,
            project,
            selection,
            playhead,
            playing,
            palette,
            &settings.key_blocks,
            &mut push,
        )
    };
    if resp.seeked {
        app.player.pause();
        app.player.seek(app.playhead);
    }
    if resp.edited {
        app.after_edit();
    }
    if let Some((a, b)) = resp.play {
        app.seek(a);
        app.play_gated(1.0);
        app.play_stop_at = Some(b);
    }
    if let Some(t) = resp.toast {
        app.toast(t);
    }
    if let Some(b) = resp.save {
        let name = b.name.clone();
        let data = serde_json::to_value(&b).unwrap_or_default();
        match super::gallery_mine::put(app, crate::engine::gallery::Kind::Animate, &name, &[], data, false) {
            Ok(it) => app
                .toast(format!("Saved '{}' to Gallery ▸ Animate - share its file from ⋯ ▸ Show preset files", it.name)),
            Err(e) => app.toast(e),
        }
    }
    if resp.import {
        super::gallery_mine::import_dialog(app, false);
    }
    if resp.open_folder {
        super::gallery_mine::open_folder(Some(crate::engine::gallery::Kind::Animate));
    }
    if resp.with_overlays {
        select_with_overlays(app);
    }
    if resp.stack {
        stack(app);
    }
    if let Some((pid, turn)) = resp.follow {
        let ids = app.selection.clone();
        let snap = app.project.to_json();
        if follow_path(&mut app.project, &ids, pid, turn) > 0 {
            push_undo_json(&mut app.undo, &mut app.redo, snap);
            app.after_edit();
            app.toast("Following the path - right-click Position X / Y and Unlink to stop");
        }
    }
    if resp.draw_path {
        if app.selection.is_empty() {
            app.toast("Select the clips that should follow the path first");
        } else {
            app.path_armed = true;
            app.tools.tool = crate::ui::tools::Tool::Draw;
            app.toast("Draw the motion path in the Viewer - the selected clips follow it at your drawing's pace");
        }
    }
    true
}

/// Link `ids`' Position X / Y (and Rotation when `turn`) to saved path `pid`. Returns how many clips.
fn follow_path(p: &mut Project, ids: &[Id], pid: Id, turn: bool) -> usize {
    let mut n = 0;
    for &id in ids {
        let Some(c) = p.clip_mut(id).filter(|c| c.is_visual()) else { continue };
        for (a, l) in [(&mut c.x, AnimLink::PathX(pid)), (&mut c.y, AnimLink::PathY(pid))] {
            a.unlink();
            a.link = l;
        }
        if turn {
            c.rotation.unlink();
            c.rotation.link = AnimLink::PathAngle(pid);
        }
        n += 1;
    }
    p.refresh_links();
    n
}

/// A stroke drawn while "Draw a motion path" was armed: save it as a path and make the selection follow it.
/// True when it took the stroke (the caller then doesn't add it as a drawing).
pub(super) fn take_stroke(app: &mut App, s: &crate::model::Stroke) -> bool {
    if !std::mem::take(&mut app.path_armed) || s.points.len() < 2 {
        return false;
    }
    let snap = app.project.to_json();
    let name = format!("Motion path {}", app.project.paths.len() + 1);
    let pid = app.project.add_path(name.clone(), s.points.clone());
    let ids = app.selection.clone();
    let n = follow_path(&mut app.project, &ids, pid, false);
    push_undo_json(&mut app.undo, &mut app.redo, snap);
    app.after_edit();
    app.tools.tool = crate::ui::tools::Tool::Select;
    app.toast(format!(
        "Saved {name}: {n} clip{} follow it - Rotation, Turn along path to face its way",
        if n == 1 { "" } else { "s" }
    ));
    true
}

fn select_with_overlays(app: &mut App) {
    let ids = app.project.clip_with_overlays(app.playhead);
    if ids.is_empty() {
        app.toast("No video clip under the playhead");
        return;
    }
    let n = ids.len() - 1;
    app.selection = ids;
    app.toast(format!("Selected the clip and {n} overlay{}", if n == 1 { "" } else { "s" }));
}

/// Stack the selection (or, with one clip or none selected, the clip under the playhead and its overlays).
fn stack(app: &mut App) {
    let ids =
        if app.selection.len() > 1 { app.selection.clone() } else { app.project.clip_with_overlays(app.playhead) };
    let snap = app.project.to_json();
    match app.project.make_stack(&ids) {
        Some(id) => {
            push_undo_json(&mut app.undo, &mut app.redo, snap);
            app.selection = vec![id];
            app.after_edit();
            app.toast(format!("Stacked {} clips: a transition or block on it moves them together. Right-click ▸ Nest ▸ Unnest to undo the grouping", ids.len()));
        }
        None => app.toast("Select a footage clip and its overlays to stack"),
    }
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::StackOverlays => stack(app),
        _ => return false,
    }
    true
}

fn done(v: Value) -> Result<ToolOutcome, String> {
    Ok(ToolOutcome::Done(v))
}

fn clip_block(args: &Value) -> Result<(Id, usize), String> {
    Ok((req(arg_u64(args, "clip_id"), "clip_id")?, req(arg_u64(args, "index"), "index")? as usize))
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "animate.stack",
        desc: "Group a footage clip and its overlays into one stack (a nested sequence on the footage's track) so a transition or a block moves them together. clip_ids default: the clip under the playhead and the clips mostly over it.",
        args: &["clip_ids:array:false:default = the clip under the playhead + its overlays"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let ids = arg_ids(args, "clip_ids").unwrap_or_else(|| app.project.clip_with_overlays(app.playhead));
            let id = app.project.make_stack(&ids).ok_or("need a footage clip on a video track")?;
            done(json!({"ok": true, "stack_clip": id}))
        },
    },
    ToolDef {
        name: "keyblocks.set",
        desc: "Set an applied block's controls and rewrite its keys: strength (0..2, 1 = as designed), dir (left/up/down/right), ease (a preset name or 'design' for the block's own), dur seconds, physics fields (stiffness, damping, bounce, gravity, spin -1..1, freq jolts/s, seed, cover). Resets a hand-edited block.",
        args: &[
            "clip_id:integer:true:",
            "index:integer:true:",
            "strength:number:false:",
            "dir:string:false:left/up/down/right",
            "ease:string:false:",
            "dur:number:false:seconds",
            "physics:object:false:{stiffness, damping, bounce, gravity, spin, freq, seed, cover}",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let (id, i) = clip_block(args)?;
            let b = app.project.clip(id).and_then(|c| c.blocks.get(i)).cloned().ok_or("no such block")?;
            let mut ctl = b.ctl.clone().unwrap_or_else(|| BlockCtl::new(find_block(&app.settings.key_blocks, &b.name).unwrap_or_default()));
            if let Some(s) = arg_f64(args, "strength") {
                ctl.strength = s.clamp(0.0, 3.0);
            }
            if let Some(d) = arg_str(args, "dir") {
                ctl.dir = Some(Dir::parse(d).ok_or("dir: left, up, down or right")?);
            }
            if let Some(e) = arg_str(args, "ease") {
                ctl.ease = if e.eq_ignore_ascii_case("design") { None } else { Some(super::tools_keyblocks::ease_arg(e).ok_or("unknown ease")?) };
            }
            if let Some(p) = args.get("physics").filter(|p| p.is_object()) {
                let mut cur = serde_json::to_value(ctl.phys).map_err(|e| e.to_string())?;
                for (k, v) in p.as_object().unwrap() {
                    cur[k] = v.clone();
                }
                ctl.phys = serde_json::from_value(cur).map_err(|e| format!("physics: {e}"))?;
            }
            if let Some(d) = arg_f64(args, "dur") {
                app.project.retime_key_block(id, i, b.t, d);
            }
            app.project.set_block_ctl(id, i, ctl);
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "animate.link",
        desc: "Link a clip property to another clip's property (After Effects' pick whip): value = source x mul + add at the same timeline moment. Props: x, y, scale, scale_x, scale_y, rotation, opacity, volume, pan, speed. source_clip 0 unlinks.",
        args: &["clip_id:integer:true:", "prop:string:true:", "source_clip:integer:true:0 = unlink", "source_prop:string:false:default = prop", "mul:number:false:default 1", "add:number:false:default 0"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = req(arg_u64(args, "clip_id"), "clip_id")?;
            let prop = req(arg_str(args, "prop"), "prop")?.to_string();
            let src = req(arg_u64(args, "source_clip"), "source_clip")?;
            let sprop = arg_str(args, "source_prop").unwrap_or(&prop).to_string();
            if src != 0 && (src == id || app.project.clip(src).is_none()) {
                return Err("source_clip: another clip on the timeline".into());
            }
            if !crate::model::BLOCK_PROPS.contains(&sprop.as_str()) {
                return Err(format!("source_prop: one of {:?}", crate::model::BLOCK_PROPS));
            }
            let (mul, add) = (arg_f64(args, "mul").unwrap_or(1.0), arg_f64(args, "add").unwrap_or(0.0));
            let c = app.project.clip_mut(id).ok_or("no such clip")?;
            let a = crate::model::prop_mut(c, &prop).ok_or("prop: x, y, scale, scale_x, scale_y, rotation, opacity, volume, pan or speed")?;
            a.unlink();
            if src != 0 {
                a.link = AnimLink::Layer { clip: src, prop: sprop, mul, add };
            }
            app.project.refresh_links();
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "animate.follow_path",
        desc: "Make clips follow a saved path (project.paths: Position X / Y linked to it), turn: also face along it (Rotation).",
        args: &["path_id:integer:true:", "clip_ids:array:false:defaults to selection", "turn:boolean:false:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let pid = req(arg_u64(args, "path_id"), "path_id")?;
            if !app.project.paths.iter().any(|p| p.id == pid) {
                return Err("no such path".into());
            }
            let ids = arg_ids(args, "clip_ids").unwrap_or_else(|| app.selection.clone());
            let turn = args.get("turn").and_then(|v| v.as_bool()).unwrap_or(false);
            let n = follow_path(&mut app.project, &ids, pid, turn);
            done(json!({"ok": true, "count": n}))
        },
    },
    ToolDef {
        name: "flow.seams",
        desc: "Every hand-off between two animation blocks (on one clip, or across a cut): the two blocks, timeline t, the property that moves most there, its speed as the earlier block ends and as the next starts (scale in %/s, x/y px/s, rotation °/s), and whether that is smooth.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| {
            let v: Vec<Value> = app
                .project
                .block_seams()
                .into_iter()
                .map(|s| json!({"left": {"clip_id": s.left.0, "index": s.left.1}, "right": {"clip_id": s.right.0, "index": s.right.1}, "t": s.t, "prop": s.prop, "speed_left": s.v_left, "speed_right": s.v_right, "smooth": s.smooth}))
                .collect();
            done(json!(v))
        },
    },
    ToolDef {
        name: "flow.match",
        desc: "Match the speeds at a hand-off by easing: mode next (the later block starts at the earlier one's end speed, default), earlier, or both (meet halfway). Without clips: every rough hand-off.",
        args: &[
            "left_clip:integer:false:",
            "left_index:integer:false:",
            "right_clip:integer:false:",
            "right_index:integer:false:",
            "mode:string:false:next | earlier | both",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let mode = arg_str(args, "mode").map_or(Ok(Match::Next), |m| Match::parse(m).ok_or("mode: next, earlier or both"))?;
            let pair = |c: &str, i: &str| Some((arg_u64(args, c)?, arg_u64(args, i)? as usize));
            if let (Some(l), Some(r)) = (pair("left_clip", "left_index"), pair("right_clip", "right_index")) {
                let v = app.project.match_seam(l, r, mode)?;
                return done(json!({"ok": true, "speed": v}));
            }
            let mut n = 0;
            for s in app.project.block_seams().into_iter().filter(|s| !s.smooth) {
                n += app.project.match_seam(s.left, s.right, mode).is_ok() as usize;
            }
            done(json!({"ok": true, "matched": n}))
        },
    },
];
