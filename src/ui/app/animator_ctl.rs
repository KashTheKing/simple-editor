//! ---- ws:animator ----
//! App side of the Animator (`ui::animator`): the pane, Stack with Overlays, the Path tool's strokes, block
//! controls and Flow over MCP.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::{find_block, AnimLink, BlockCtl, Dir, Match, PathCtl};
use crate::ui::animator::AnimTool;
use crate::ui::tools::Tool;

/// Onion skin follows the Animator: off whenever it wasn't on screen last frame.
pub(super) fn frame(app: &mut App, ctx: &egui::Context) {
    if !app.pane_drawn(Pane::Animator) {
        crate::ui::animator::set_onion(0);
        app.animator.here = false;
    }
    // the Animator's loop ends with it: Loop switched off, or a press back on the timeline / Viewer
    if app.animator.looping && !(app.animator.here && app.animator.loop_play) {
        app.player.set_loop(None);
        app.animator.looping = false;
    }
    // the Path tool draws in the Viewer: while it is picked the Viewer holds its Draw tool (`path_armed`), and
    // leaving either one (another Viewer tool, another Animator tool, the pane closed) leaves both
    let path = app.animator.tool == AnimTool::Path && app.pane_drawn(Pane::Animator);
    if path && app.path_armed && app.tools.tool != Tool::Draw {
        (app.animator.tool, app.path_armed) = (AnimTool::Select, false);
    } else if path != app.path_armed {
        app.path_armed = path;
        if path {
            app.tools.tool = Tool::Draw;
        } else {
            if app.animator.tool == AnimTool::Path {
                app.animator.tool = AnimTool::Select; // the pane went away
            }
            if app.tools.tool == Tool::Draw {
                app.tools.tool = Tool::Select;
            }
        }
    }
    // Delete / Backspace / Split belong to the Animator's block or keys after a press in it, and to the
    // timeline's clips again after a press on the timeline or the Viewer
    if ctx.input(|i| i.pointer.any_pressed()) {
        match app.layout.hovered {
            Some(Pane::Animator) => app.animator.here = true,
            Some(Pane::Timeline | Pane::Preview) => app.animator.here = false,
            _ => {}
        }
    }
    // the timeline's tool keys pick the Animator's own tools while it is the panel in use. Taken here: the
    // tool strip claims its keys before the action table is polled, so `act` below only ever sees them from
    // a direct dispatch (the palette, a script)
    if app.animator.here && app.pane_drawn(Pane::Animator) && !ctx.wants_keyboard_input() {
        for (a, t) in [
            (Action::ToolSelect, AnimTool::Select),
            (Action::ToolCut, AnimTool::Cut),
            (Action::ToolDraw, AnimTool::Path),
            (Action::ToolPen, AnimTool::Path),
        ] {
            if app.hotkeys.get(a).is_some_and(|ks| ctx.input_mut(|i| crate::hotkeys::consume_exact(i, &ks))) {
                app.animator.tool = t;
            }
        }
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
    if resp.transitionable {
        let snap = app.project.to_json();
        let ids = app.selection.clone();
        match app.project.make_transitionable(&ids) {
            0 => app.toast("Nothing to close: the clips already sit end to end (or something is between them)"),
            n => {
                push_undo_json(&mut app.undo, &mut app.redo, snap);
                app.after_edit();
                app.toast(format!("Closed {n} gap{}: these clips now share cuts a transition can sit on", if n == 1 { "" } else { "s" }));
            }
        }
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

/// A stroke drawn in the Viewer with the Animator's Path tool: a Path block on every selected picture clip,
/// from the playhead (the clip's start when the playhead is not on it), as long as the stroke took to draw,
/// timed as it was drawn. True when it took the stroke (the caller then doesn't add it as a drawing).
pub(super) fn take_stroke(app: &mut App, s: &crate::model::Stroke) -> bool {
    if !app.path_armed || s.points.len() < 2 {
        return false;
    }
    let t0 = s.points[0].2;
    let points: Vec<(f32, f32, f32)> = s.points.iter().map(|p| (p.0, p.1, p.2 - t0)).collect();
    let dur = (points[points.len() - 1].2 as f64).max(0.2);
    let snap = app.project.to_json();
    let mut made = Vec::new();
    for id in app.selection.clone() {
        let Some(c) = app.project.clip(id) else { continue };
        let at = if c.contains(app.playhead) { app.playhead - c.start } else { 0.0 };
        let path = PathCtl { points: points.clone(), as_drawn: true, ..Default::default() };
        made.extend(app.project.add_path_block(id, path, at, dur).map(|i| (id, i)));
    }
    if made.is_empty() {
        // the tool stays on: select a clip and draw again
        app.toast("Select the picture clips that should travel the path, then draw it");
        return true;
    }
    push_undo_json(&mut app.undo, &mut app.redo, snap);
    app.after_edit();
    // back to Select (the frame hook hands the Viewer its Select tool back), the new block picked
    app.animator.tool = AnimTool::Select;
    (app.animator.sel, app.animator.multi) = (made.first().copied(), made[1..].to_vec());
    app.toast(format!("Path block on {} clip{}", made.len(), if made.len() == 1 { "" } else { "s" }));
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

/// Split (Ctrl+B) with a block picked in the Animator: the block at the playhead, not the clip.
fn split_block(app: &mut App, (id, i): (Id, usize)) {
    let snap = app.project.to_json();
    let t = app.playhead - app.project.clip(id).map_or(0.0, |c| c.start);
    if app.project.split_key_block(id, i, t) {
        push_undo_json(&mut app.undo, &mut app.redo, snap);
        app.animator.multi.clear();
        app.after_edit();
        app.toast("Split the block at the playhead: the first part ends there, the second starts from it");
    } else {
        app.toast("Move the playhead inside the block to split it there");
    }
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match (a, app.animator.sel) {
        (Action::Split, Some(b)) if app.animator.here => {
            split_block(app, b);
            return true;
        }
        _ => {}
    }
    // the timeline's tool keys drive the Animator's own tools while it is the panel in use
    if app.animator.here && app.pane_drawn(Pane::Animator) {
        match a {
            Action::ToolSelect => app.animator.tool = AnimTool::Select,
            Action::ToolCut => app.animator.tool = AnimTool::Cut,
            Action::ToolDraw | Action::ToolPen => app.animator.tool = AnimTool::Path,
            Action::ToggleSnap => app.animator.no_snap = !app.animator.no_snap,
            // Play runs the range the Animator shows and stops at its end (or repeats it with Loop on)
            Action::PlayPause
                if !app.player.is_playing() && !app.play_gate.buffering() && app.animator.range.1 > app.animator.range.0 =>
            {
                let (t0, t1) = app.animator.range;
                if app.playhead < t0 - 1e-6 || app.playhead >= t1 - 1e-3 {
                    app.seek(t0);
                }
                app.play_gated(1.0);
                if app.animator.loop_play {
                    app.player.set_loop(Some((t0, t1)));
                    app.animator.looping = true;
                } else {
                    app.play_stop_at = Some(t1);
                }
            }
            _ => return false,
        }
        return true;
    }
    match a {
        Action::StackOverlays => stack(app),
        // the timeline's "Show Animations": the Animator, on the clips that are selected
        Action::ShowAnimations => app.surface(Pane::Animator),
        _ => return false,
    }
    true
}

fn done(v: Value) -> Result<ToolOutcome, String> {
    Ok(ToolOutcome::Done(v))
}

/// The span fields a tool call gives, onto `sp`.
fn span_fields(sp: &mut crate::model::TransitionSpan, args: &Value) -> Result<(), String> {
    use crate::model::TransitionKind;
    if let Some(k) = arg_str(args, "kind") {
        sp.kind = TransitionKind::from_name(k).ok_or("kind: Cross Fade, Fade to Color, Push or Wipe (a shader transition: give shader)")?;
    }
    if let Some(src) = arg_str(args, "shader") {
        crate::ui::animator::set_span_kind(sp, (TransitionKind::Shader, src.to_string()));
    }
    if let Some(v) = args.get("params").and_then(|v| v.as_array()) {
        sp.params = v.iter().filter_map(|x| x.as_f64()).collect();
    }
    if let Some(t) = arg_f64(args, "t") {
        sp.t = t.max(0.0);
    }
    if let Some(d) = arg_f64(args, "dur") {
        sp.dur = d.max(crate::model::ops::spans::MIN_SPAN);
    }
    if let Some(d) = arg_f64(args, "cut_dur") {
        sp.cut_dur = d.clamp(0.05, 10.0);
    }
    if let Some(e) = arg_str(args, "ease") {
        sp.ease = super::tools_keyblocks::ease_arg(e).ok_or("unknown ease")?;
    }
    if let Some(d) = arg_str(args, "direction") {
        let i = ["left", "right", "up", "down"].iter().position(|n| n.eq_ignore_ascii_case(d));
        sp.direction = i.ok_or("direction: left, right, up or down")? as u8;
    }
    Ok(())
}

fn span_json(sp: &crate::model::TransitionSpan) -> Value {
    json!({"span_id": sp.id, "t": sp.t, "dur": sp.dur, "kind": sp.name(), "cut_dur": sp.cut_dur, "direction": sp.direction, "transitions": sp.made})
}

/// The arguments `animate.span_add` and `animate.span_set` share.
const SPAN_ARGS: [&str; 8] = [
    "t:number:false:timeline s where it starts",
    "dur:number:false:seconds it covers",
    "kind:string:false:Cross Fade | Fade to Color | Push | Wipe",
    "cut_dur:number:false:seconds each transition lasts (default 0.5)",
    "ease:string:false:",
    "direction:string:false:left | right | up | down (Push / Wipe)",
    "shader:string:false:GLSL source of a shader transition (instead of kind)",
    "params:array:false:the shader's control values, in header order",
];

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
        name: "animate.transitionable",
        desc: "Make clips ready for transitions: on each track, close the empty gaps between the given clips so they sit end to end and every pair shares a cut a transition can go on. clip_ids default: every clip on the timeline. Returns how many gaps it closed.",
        args: &["clip_ids:array:false:default = every clip"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let n = app.project.make_transitionable(&arg_ids(args, "clip_ids").unwrap_or_default());
            done(json!({"ok": true, "gaps_closed": n}))
        },
    },
    ToolDef {
        name: "animate.span_add",
        desc: "Add a transition span: a bar over the timeline (the Animator's Timeline lane) that sets up the transitions under it. Every cut between two clips on a video track whose time lies in [t, t + dur] gets the span's transition; clips under it that are separated by an empty gap are closed up first. It stays procedural: after every edit (clips moved, the span moved or stretched) the transitions it made are remade for what is under it now, and deleting it removes them. Overlays are not grouped: each video track gets its own transitions. Defaults: the whole timeline, Cross Fade, 0.5 s per cut. Returns span_id and the transitions it made.",
        args: &SPAN_ARGS,
        kind: ToolKind::Mutate,
        run: |app, args| {
            let t = arg_f64(args, "t").unwrap_or(0.0).max(0.0);
            let dur = arg_f64(args, "dur").unwrap_or((app.project.duration() - t).max(1.0));
            let id = app.project.add_span(t, dur, crate::model::TransitionKind::CrossFade);
            span_fields(app.project.spans.iter_mut().find(|s| s.id == id).ok_or("no span")?, args)?;
            app.project.refresh_spans();
            done(span_json(app.project.spans.iter().find(|s| s.id == id).ok_or("no span")?))
        },
    },
    ToolDef {
        name: "animate.span_set",
        desc: "Change a transition span (animate.span_add): move or stretch it (t, dur) or change the transition it makes (kind / shader, cut_dur, ease, direction). Its transitions are remade for what is under it now. Returns the span and the transitions it made.",
        args: &[
            "span_id:integer:true:",
            SPAN_ARGS[0],
            SPAN_ARGS[1],
            SPAN_ARGS[2],
            SPAN_ARGS[3],
            SPAN_ARGS[4],
            SPAN_ARGS[5],
            SPAN_ARGS[6],
            SPAN_ARGS[7],
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = req(arg_u64(args, "span_id"), "span_id")?;
            span_fields(app.project.spans.iter_mut().find(|s| s.id == id).ok_or("no such span")?, args)?;
            app.project.refresh_spans();
            done(span_json(app.project.spans.iter().find(|s| s.id == id).ok_or("no such span")?))
        },
    },
    ToolDef {
        name: "animate.span_remove",
        desc: "Delete a transition span and the transitions it made (the clips stay where they are).",
        args: &["span_id:integer:true:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            if !app.project.remove_span(req(arg_u64(args, "span_id"), "span_id")?) {
                return Err("no such span".into());
            }
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "animate.spans",
        desc: "The transition spans on the timeline (animate.span_add): span_id, t, dur, kind, cut_dur, direction and the ids of the transitions each one made.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| done(json!(app.project.spans.iter().map(span_json).collect::<Vec<_>>())),
    },
    ToolDef {
        name: "keyblocks.set",
        desc: "Set an applied block's controls and rewrite its keys: strength (0..2, 1 = as designed), dir (left/up/down/right), ease (a preset name or 'design' for the block's own), dur seconds, physics fields (stiffness, damping, bounce, gravity, spin -1..1, freq jolts/s, seed, cover; rigid body - Gravity Fall, Throw & Bounce, Collide: mass 0.1..10, gravity, gravity_dir Left/Up/Down/Right, bounce = elasticity, friction, drag, throw = initial velocity along dir, spin). On a Collide block it runs that drop again with the new body. Resets a hand-edited block.",
        args: &[
            "clip_id:integer:true:",
            "index:integer:true:",
            "strength:number:false:",
            "dir:string:false:left/up/down/right",
            "ease:string:false:",
            "dur:number:false:seconds",
            "physics:object:false:{stiffness, damping, bounce, gravity, spin, freq, seed, cover, throw, mass, friction, drag, gravity_dir}",
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
            // a Collide block has no keys of its own to rewrite: its rigid-body controls run the drop again
            if b.name == "Collide" {
                let n = app.project.recollide(id, i, ctl.phys, ctl.dir);
                return done(json!({"ok": true, "clips": n}));
            }
            if let Some(d) = arg_f64(args, "dur") {
                app.project.retime_key_block(id, i, b.t, d);
            }
            app.project.set_block_ctl(id, i, ctl);
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "keyblocks.loop",
        desc: "Loop an applied block: it plays count times (1 = once, up to 50) and grows to count times one cycle's length (as far as the clip lasts). pingpong: every other cycle runs backwards (back and forth). A block of plain or hand-edited keys loops the keys it has.",
        args: &["clip_id:integer:true:", "index:integer:true:", "count:integer:true:1..50", "pingpong:boolean:false:back and forth"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let (id, i) = clip_block(args)?;
            let count = req(arg_u64(args, "count"), "count")? as u32;
            let pingpong = args.get("pingpong").and_then(|v| v.as_bool()).unwrap_or(false);
            if !app.project.loop_key_block(id, i, count, pingpong) {
                return Err("no such block".into());
            }
            let b = &app.project.clip(id).ok_or("no such clip")?.blocks[i];
            done(json!({"ok": true, "t": b.t, "dur": b.dur}))
        },
    },
    ToolDef {
        name: "keyblocks.loop_keys",
        desc: "Repeat keyframes: the keys of props in clip-local [from, to] are copied count more times right after to (keys already there are replaced, copies past the clip's end are dropped). Where the span ends on another value than it starts from, the jump back takes one frame. Returns how many keys it added.",
        args: &[
            "clip_id:integer:true:",
            "from:number:true:clip-local s",
            "to:number:true:clip-local s",
            "count:integer:true:1..50 more times",
            "props:array:false:default = every keyed property (x, y, scale, scale_x, scale_y, rotation, opacity, volume, pan, speed)",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = req(arg_u64(args, "clip_id"), "clip_id")?;
            let (from, to) = (req(arg_f64(args, "from"), "from")?, req(arg_f64(args, "to"), "to")?);
            let count = req(arg_u64(args, "count"), "count")?.clamp(1, 50) as u32;
            let props: Vec<String> = match args.get("props").and_then(|v| v.as_array()) {
                Some(a) => a.iter().filter_map(|v| v.as_str()).map(String::from).collect(),
                None => crate::model::BLOCK_PROPS.iter().map(|p| p.to_string()).collect(),
            };
            match app.project.loop_keys(id, &props, from, to, count) {
                0 => Err("no keys to repeat: from..to must hold two or more keys of a property".into()),
                n => done(json!({"ok": true, "keys_added": n})),
            }
        },
    },
    ToolDef {
        name: "animate.collide",
        desc: "Drop with collisions: the given clips' pictures fall from where they are at timeline t under gravity, land on the frame's bottom edge, stay inside its sides and push each other apart (boxes sized from each clip's media or shape x scale). One simulation run now; each clip gets its Position X / Y (and Rotation when it spins) as one 'Collide' block of plain keys. Every clip is a rigid body: physics = {mass 0.1..10, gravity 0..1, gravity_dir Left/Up/Down/Right, bounce (elasticity) 0..1, friction 0..1, drag 0..1, throw (initial velocity) 0..1, spin -1..1}; heavier bodies are pushed less and slowed less by drag. keyblocks.set on a Collide block changes one clip's body and runs the drop again. Returns how many clips it animated.",
        args: &[
            "clip_ids:array:false:defaults to selection (two or more pictures under t)",
            "t:number:false:timeline s, default = the playhead",
            "dur:number:false:seconds, default 4 (at most until the first clip ends)",
            "bounce:number:false:0..1, default 0.45 (same as physics.bounce)",
            "gravity:number:false:0..1, default 0.55 (same as physics.gravity)",
            "physics:object:false:{mass, gravity, gravity_dir, bounce, friction, drag, throw, spin} for every clip",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let ids = arg_ids(args, "clip_ids").unwrap_or_else(|| app.selection.clone());
            let t = arg_f64(args, "t").unwrap_or(app.playhead);
            // a plain drop: no throw, no spin
            let mut phys = crate::model::Phys { throw: 0.0, spin: 0.0, ..Default::default() };
            if let Some(p) = args.get("physics").filter(|p| p.is_object()) {
                let mut cur = serde_json::to_value(phys).map_err(|e| e.to_string())?;
                for (k, v) in p.as_object().unwrap() {
                    cur[k] = v.clone();
                }
                phys = serde_json::from_value(cur).map_err(|e| format!("physics: {e}"))?;
            }
            phys.bounce = arg_f64(args, "bounce").unwrap_or(phys.bounce);
            phys.gravity = arg_f64(args, "gravity").unwrap_or(phys.gravity);
            match app.project.collide_drop(&ids, t, arg_f64(args, "dur"), &phys) {
                0 => Err("need two or more picture clips that are on screen at t, with at least 0.2 s left".into()),
                n => done(json!({"ok": true, "clips": n})),
            }
        },
    },
    ToolDef {
        name: "keyblocks.split",
        desc: "Split an applied block in two at clip-local t (default: the playhead), the motion unchanged: the first part ends at the value there (its target), the second starts from it (its origin) and keeps the old target. The parts are blocks index and index + 1.",
        args: &["clip_id:integer:true:", "index:integer:true:", "t:number:false:clip-local s, inside the block"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let (id, i) = clip_block(args)?;
            let start = app.project.clip(id).ok_or("no such clip")?.start;
            let t = arg_f64(args, "t").unwrap_or(app.playhead - start);
            if !app.project.split_key_block(id, i, t) {
                return Err("t must be inside the block".into());
            }
            done(json!({"ok": true, "index": [i, i + 1]}))
        },
    },
    ToolDef {
        name: "keyblocks.link",
        desc: "Link two or more applied blocks of one clip into a single block over their whole span with all their properties (motion unchanged). Returns the new block's index; the other blocks' indexes change.",
        args: &["clip_id:integer:true:", "indexes:array:true:block indexes, two or more"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = req(arg_u64(args, "clip_id"), "clip_id")?;
            let idx: Vec<usize> = args
                .get("indexes")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_u64()).map(|v| v as usize).collect())
                .unwrap_or_default();
            let i = app.project.link_key_blocks(id, &idx).ok_or("indexes: two or more blocks of that clip")?;
            done(json!({"ok": true, "index": i}))
        },
    },
    ToolDef {
        name: "flow.together",
        desc: "Chain two or more blocks in time order: each next block on the same clip moves to start where the one before ends, from the value it ends at (origin = that target), and the speeds are matched at each hand-off (mode as flow.match). Returns how many hand-offs it made.",
        args: &["blocks:array:true:[{clip_id, index}, ...]", "mode:string:false:next | earlier | both"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let mode = arg_str(args, "mode").map_or(Ok(Match::Next), |m| Match::parse(m).ok_or("mode: next, earlier or both"))?;
            let one = |b: &Value| Some((b.get("clip_id")?.as_u64()?, b.get("index")?.as_u64()? as usize));
            let sel: Vec<(Id, usize)> =
                args.get("blocks").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(one).collect()).unwrap_or_default();
            if sel.len() < 2 {
                return Err("blocks: two or more {clip_id, index}".into());
            }
            done(json!({"ok": true, "handoffs": app.project.flow_key_blocks(&sel, mode)}))
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
        name: "animate.path",
        desc: "Add a Path block to a picture clip: its Position travels the given stroke (what the Animator's Path tool makes from a drawing in the Viewer). points: [[x, y, t], ...] - x, y in project px from the frame's centre (a clip's Position), t in seconds from the stroke's start (left out: 30 points a second). timing 'shape' (default): one speed along the path over the block, shaped by ease; 'drawn': the points' own timing scaled to the block. smoothing 0..1 (0 = the points as given, 1 = a heavily smoothed curve; the ends stay). turn: Rotation faces the way it travels. The block keeps the stroke: keyblocks.set (ease, dur) and another animate.path replace its keys. Returns the block's index.",
        args: &[
            "clip_id:integer:true:",
            "points:array:true:[[x, y, t], ...] two or more",
            "timing:string:false:shape | drawn",
            "smoothing:number:false:0..1",
            "turn:boolean:false:",
            "t:number:false:clip-local start s, default = the playhead (0 when it is not on the clip)",
            "dur:number:false:seconds, default = the stroke's own length (at least 0.2)",
            "ease:string:false:shape timing only",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = req(arg_u64(args, "clip_id"), "clip_id")?;
            let raw = args.get("points").and_then(|v| v.as_array()).ok_or("points: [[x, y, t], ...]")?;
            let mut points = Vec::new();
            for (i, p) in raw.iter().enumerate() {
                let n = |k: usize| p.get(k).and_then(|v| v.as_f64());
                let (x, y) = n(0).zip(n(1)).ok_or("points: each one is [x, y] or [x, y, t]")?;
                points.push((x as f32, y as f32, n(2).unwrap_or(i as f64 / 30.0) as f32));
            }
            if points.len() < 2 {
                return Err("points: two or more".into());
            }
            let as_drawn = match arg_str(args, "timing").unwrap_or("shape") {
                "shape" => false,
                "drawn" | "as drawn" | "as_drawn" => true,
                _ => return Err("timing: shape or drawn".into()),
            };
            let c = app.project.clip(id).ok_or("no such clip")?;
            let at = arg_f64(args, "t").unwrap_or(if c.contains(app.playhead) { app.playhead - c.start } else { 0.0 });
            let span = (points[points.len() - 1].2 - points[0].2) as f64;
            let dur = arg_f64(args, "dur").unwrap_or(span).max(0.2);
            let path = PathCtl {
                points,
                as_drawn,
                smoothing: arg_f64(args, "smoothing").unwrap_or(0.0).clamp(0.0, 1.0),
                turn: args.get("turn").and_then(|v| v.as_bool()).unwrap_or(false),
            };
            let i = app.project.add_path_block(id, path, at, dur).ok_or("needs a picture clip with room for the block")?;
            if let Some(e) = arg_str(args, "ease") {
                let mut ctl = app.project.clip(id).and_then(|c| c.blocks[i].ctl.clone()).ok_or("no such block")?;
                ctl.ease = Some(super::tools_keyblocks::ease_arg(e).ok_or("unknown ease")?);
                app.project.set_block_ctl(id, i, ctl);
            }
            done(json!({"ok": true, "index": i}))
        },
    },
    ToolDef {
        name: "keyblocks.from_keys",
        desc: "Turn one clip's keys made by hand (keys no block owns) into a block with controls (strength, ease, side), keeping the motion. With from/to (clip-local seconds): the keys in that span; without: every cluster of hand keys. Returns how many blocks it made. For many clips or the whole timeline use keyblocks.convert_all (one call, named blocks where they fit, dry_run).",
        args: &["clip_id:integer:true:", "from:number:false:clip-local s", "to:number:false:clip-local s", "name:string:false:default Custom"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = req(arg_u64(args, "clip_id"), "clip_id")?;
            let span = arg_f64(args, "from").zip(arg_f64(args, "to"));
            let n = app.project.block_from_keys(id, span, arg_str(args, "name").unwrap_or("Custom"));
            done(json!({"ok": true, "blocks": n}))
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
