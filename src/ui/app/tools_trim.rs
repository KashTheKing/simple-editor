//! ---- ws:trim-model ----
//! MCP tools for the trim primitives (model/ops/{tracks,editing,trim}.rs): track flags/reorder,
//! the ripple/roll/slip/slide/trim_edges family, splice/overwrite/lift/extract, join/duplicate/
//! unnest/replace/magnetic_move, and the keyboard-trim `timeline.edit_point`/`timeline.keyframe_nav`
//! UI-state tools. `timeline.splice`/`overwrite`/`lift`/`extract`/`replace` are the canonical
//! registrations for these names project-wide (audit fix 3/4 in the plan) — source-monitor's
//! wave-2 three-point-edit UI must call the underlying `Project::` fns directly, not re-register
//! them. Every `Mutate` row here relies on `App::handle_tool`/`run_script`'s existing generic
//! snapshot-before/push-undo-iff-changed wrapper (mcp_exec.rs) — nothing below pushes undo itself.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::ops::tracks::TrackFlag;
use crate::model::ops::trim::{EditPoint, Side};

fn parse_side(s: &str) -> Result<Side, String> {
    match s {
        "Left" => Ok(Side::Left),
        "Right" => Ok(Side::Right),
        "Both" => Ok(Side::Both),
        _ => Err("side: Left|Right|Both".into()),
    }
}
fn side_name(s: Side) -> &'static str {
    match s {
        Side::Left => "Left",
        Side::Right => "Right",
        Side::Both => "Both",
    }
}
/// `[r,g,b]` (a track swatch colour has no alpha, unlike `tools_helpers::color_arg`'s `[r,g,b,a]`).
fn color3_arg(v: &Value) -> Option<[u8; 3]> {
    let a = v.as_array()?;
    Some([a.first()?.as_u64()? as u8, a.get(1)?.as_u64()? as u8, a.get(2)?.as_u64()? as u8])
}
fn usize_array(args: &Value, k: &str) -> Option<Vec<usize>> {
    Some(args.get(k)?.as_array()?.iter().filter_map(|v| v.as_u64()).map(|v| v as usize).collect())
}

pub(super) fn dispatch(app: &mut App, name: &str, args: &Value) -> Option<Result<Value, String>> {
    let prefix = name.split('.').next().unwrap_or("");
    if !matches!(prefix, "timeline" | "track") {
        return None;
    }
    pub(super) fn run(app: &mut App, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "track.set" => {
                let ti = req(arg_u64(args, "index"), "index")? as usize;
                if ti >= app.project.tracks.len() {
                    return Err("no such track".into());
                }
                if let Some(v) = arg_bool(args, "locked") {
                    app.project.set_track_flag(ti, TrackFlag::Locked, v);
                }
                if let Some(v) = arg_bool(args, "ripple") {
                    app.project.set_track_flag(ti, TrackFlag::Ripple, v);
                }
                if let Some(v) = arg_bool(args, "magnetic") {
                    app.project.set_track_flag(ti, TrackFlag::Magnetic, v);
                }
                if let Some(n) = arg_str(args, "name") {
                    app.project.rename_track(ti, n.to_string());
                }
                if let Some(c) = args.get("color") {
                    let color = if c.is_null() { None } else { Some(color3_arg(c).ok_or("color: [r,g,b] or null")?) };
                    app.project.set_track_color(ti, color);
                }
                Ok(json!({"ok": true}))
            }
            "track.move" => {
                let ti = req(arg_u64(args, "index"), "index")? as usize;
                let up = req(arg_bool(args, "up"), "up")?;
                if app.project.move_track(ti, up) {
                    Ok(json!({"ok": true}))
                } else {
                    Err("can't move that track further (kind boundary, or no such track)".into())
                }
            }
            "track.list" => {
                let tracks: Vec<Value> = app
                    .project
                    .tracks
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        json!({
                            "index": i, "kind": format!("{:?}", t.kind), "name": t.name,
                            "locked": t.locked, "ripple": t.ripple.unwrap_or(false), "magnetic": t.magnetic,
                            "color": t.color, "clips": t.clips.len(),
                        })
                    })
                    .collect();
                Ok(json!({"tracks": tracks}))
            }
            "timeline.shift_time" => {
                let from = req(arg_f64(args, "from"), "from")?;
                let dt = req(arg_f64(args, "dt"), "dt")?;
                let tracks = usize_array(args, "tracks").unwrap_or_else(|| app.project.ripple_tracks());
                app.project.shift_time(from, dt, &tracks);
                Ok(json!({"ok": true}))
            }
            "timeline.close_gap" => {
                let track = req(arg_u64(args, "track"), "track")? as usize;
                let t = req(arg_f64(args, "t"), "t")?;
                if app.project.close_gap_at(track, t) {
                    Ok(json!({"ok": true}))
                } else {
                    Err("no gap there (or the track is locked)".into())
                }
            }
            "timeline.mark" => {
                let clip = arg_u64(args, "clip_id");
                match app.project.mark_from_clip(clip, app.playhead) {
                    Some((s, e)) => Ok(json!({"ok": true, "in": s, "out": e})),
                    None => Err("no clip there".into()),
                }
            }
            "timeline.in_out" => Ok(json!({"in": app.project.in_point, "out": app.project.out_point})),
            "timeline.ripple_trim" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let start = req(arg_bool(args, "start"), "start")?;
                let edge = req(arg_f64(args, "edge"), "edge")?;
                let ripple = arg_bool(args, "ripple").unwrap_or(false);
                if app.project.ripple_trim(id, start, edge, ripple) {
                    Ok(json!({"ok": true}))
                } else {
                    Err("trim refused (locked track, asset boundary, or blocked by a neighbour)".into())
                }
            }
            "timeline.roll" => {
                let right = req(arg_u64(args, "right_clip_id"), "right_clip_id")?;
                let cut = req(arg_f64(args, "cut"), "cut")?;
                if app.project.roll_edit(right, cut) {
                    Ok(json!({"ok": true}))
                } else {
                    Err("roll refused (no left neighbour, locked track, or asset boundary)".into())
                }
            }
            "timeline.slip" => {
                let ids = req(arg_ids(args, "clip_ids"), "clip_ids")?;
                let dsrc = req(arg_f64(args, "dsrc"), "dsrc")?;
                if app.project.slip(&ids, dsrc) {
                    Ok(json!({"ok": true}))
                } else {
                    Err("slip refused (locked track, or already at the source window's edge)".into())
                }
            }
            "timeline.slide" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let dt = req(arg_f64(args, "dt"), "dt")?;
                if app.project.slide(id, dt) {
                    Ok(json!({"ok": true}))
                } else {
                    Err("slide refused (locked track, no room, or an asset boundary)".into())
                }
            }
            "timeline.trim_edges" => {
                let arr = args.get("edges").and_then(|v| v.as_array()).ok_or("edges: [[clip_id, is_start], ...]")?;
                let mut edges = Vec::with_capacity(arr.len());
                for e in arr {
                    let pair = e.as_array().ok_or("edges: [[clip_id, is_start], ...]")?;
                    let id = pair.first().and_then(|v| v.as_u64()).ok_or("edges: [[clip_id, is_start], ...]")?;
                    let is_start = pair.get(1).and_then(|v| v.as_bool()).ok_or("edges: [[clip_id, is_start], ...]")?;
                    edges.push((id, is_start));
                }
                let dt = req(arg_f64(args, "dt"), "dt")?;
                let ripple = arg_bool(args, "ripple").unwrap_or(false);
                if app.project.trim_edges(&edges, dt, ripple) {
                    Ok(json!({"ok": true}))
                } else {
                    Err("trim refused (locked track, asset boundary, or overlap)".into())
                }
            }
            "timeline.extend" => {
                let track = req(arg_u64(args, "track"), "track")? as usize;
                let t = req(arg_f64(args, "t"), "t")?;
                let side = parse_side(req(arg_str(args, "side"), "side")?)?;
                let to = req(arg_f64(args, "to"), "to")?;
                if app.project.extend_edit(&EditPoint { track, t, side }, to) {
                    Ok(json!({"ok": true}))
                } else {
                    Err("could not extend that edit point".into())
                }
            }
            "timeline.splice" | "timeline.overwrite" => {
                let asset_id = req(arg_u64(args, "asset_id"), "asset_id")?;
                let at = req(arg_f64(args, "at"), "at")?;
                if app.project.asset(asset_id).is_none() {
                    return Err("no such asset".into());
                }
                let track = arg_u64(args, "track").map(|t| t as usize);
                let range = match (arg_f64(args, "in"), arg_f64(args, "out")) {
                    (Some(i), Some(o)) => Some((i, o)),
                    _ => None,
                };
                let ids = if name == "timeline.splice" {
                    app.project.splice_in(asset_id, at, track, range)
                } else {
                    app.project.overwrite_asset(asset_id, at, track, range)
                };
                Ok(json!({"ok": true, "clip_ids": ids}))
            }
            "timeline.lift" | "timeline.extract" => {
                let a0 = req(arg_f64(args, "a"), "a")?;
                let b0 = req(arg_f64(args, "b"), "b")?;
                let tracks = usize_array(args, "tracks");
                let ids = if name == "timeline.lift" {
                    app.project.lift_range(a0, b0, tracks.as_deref())
                } else {
                    app.project.extract_range(a0, b0, tracks.as_deref())
                };
                Ok(json!({"ok": true, "clip_ids": ids}))
            }
            "timeline.join" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                if app.project.join_through(id) {
                    Ok(json!({"ok": true}))
                } else {
                    Err("nothing to join (locked track, no right neighbour, or not contiguous)".into())
                }
            }
            "timeline.duplicate" => {
                let ids = arg_ids(args, "clip_ids").unwrap_or_else(|| app.selection.clone());
                let new_ids = app.project.duplicate(&ids);
                Ok(json!({"ok": true, "clip_ids": new_ids}))
            }
            "timeline.unnest" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let new_ids = app.project.unnest(id);
                if new_ids.is_empty() {
                    Err("not a plain (unretimed) sequence clip".into())
                } else {
                    Ok(json!({"ok": true, "clip_ids": new_ids}))
                }
            }
            "timeline.replace" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let asset_id = req(arg_u64(args, "asset_id"), "asset_id")?;
                if app.project.replace_clip(id, asset_id) {
                    Ok(json!({"ok": true}))
                } else {
                    Err("replace refused (locked track, no such clip, or no such asset)".into())
                }
            }
            "timeline.magnetic_move" => {
                let ids = req(arg_ids(args, "clip_ids"), "clip_ids")?;
                let dt = req(arg_f64(args, "dt"), "dt")?;
                let dtrack = args.get("dtrack").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                if app.project.magnetic_move(&ids, dt, dtrack) {
                    Ok(json!({"ok": true}))
                } else {
                    Err("move blocked (locked track, out of range, or the destination isn't magnetic)".into())
                }
            }
            "timeline.select_forward" => {
                let t = arg_f64(args, "t").unwrap_or(app.playhead);
                let track = arg_u64(args, "track").map(|v| v as usize);
                let backward = arg_bool(args, "backward").unwrap_or(false);
                Ok(json!({"clip_ids": app.project.clips_from(t, track, backward)}))
            }
            "timeline.clips_at" => {
                let t = arg_f64(args, "t").unwrap_or(app.playhead);
                Ok(json!({"clip_ids": app.project.clips_at(t)}))
            }
            "timeline.edit_point" => {
                if arg_bool(args, "clear").unwrap_or(false) {
                    app.timeline.edit_point = None;
                    return Ok(json!({"ok": true}));
                }
                if let (Some(track), Some(t)) = (arg_u64(args, "track"), arg_f64(args, "t")) {
                    let ep = match arg_str(args, "side") {
                        Some(s) => EditPoint { track: track as usize, t, side: parse_side(s)? },
                        None => {
                            app.project.nearest_edit_point(t, Some(track as usize)).ok_or("no edit point near there")?
                        }
                    };
                    app.timeline.edit_point = Some(ep);
                }
                Ok(match app.timeline.edit_point {
                    Some(ep) => json!({"track": ep.track, "t": ep.t, "side": side_name(ep.side)}),
                    None => Value::Null,
                })
            }
            "timeline.keyframe_nav" => {
                let dir = req(arg_str(args, "direction"), "direction")?;
                let &id = app.selection.first().ok_or("select a clip first")?;
                let c = app.project.clip(id).ok_or("no such clip")?;
                let lt = app.playhead - c.start;
                let start = c.start;
                let times = c.key_times();
                let target = match dir {
                    "prev" => times.iter().rev().find(|&&t| t < lt - 1e-6).copied(),
                    "next" => times.iter().find(|&&t| t > lt + 1e-6).copied(),
                    _ => return Err("direction: prev|next".into()),
                };
                let t = target.ok_or("no keyframe in that direction")?;
                app.seek(start + t);
                Ok(json!({"ok": true, "t": start + t}))
            }
            _ => unreachable!(),
        }
    }
    Some(run(app, name, args))
}

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
    row!("track.set", ToolKind::Mutate, "Edit one track's flags/name/colour. Colour is [r,g,b] or null (canonical form — no competing colour-index field/tool).", &["index:integer:true:", "locked:boolean:false:", "ripple:boolean:false:", "magnetic:boolean:false:", "name:string:false:", "color:array:false:[r,g,b] or null"]),
    row!("track.move", ToolKind::Mutate, "Reorder a track one slot up/down within its own kind (video tracks / audio tracks stay contiguous).", &["index:integer:true:", "up:boolean:true:"]),
    row!("track.list", ToolKind::Read, "Every track: index, kind, name, locked, ripple, magnetic, color, clip count.", &[]),
    row!("timeline.shift_time", ToolKind::Mutate, "Shift markers (matching the current sequence scope), main-timeline subtitle cues, and in/out at/after `from` by `dt` seconds.", &["from:number:true:", "dt:number:true:", "tracks:array:false:default ripple_tracks()"]),
    row!("timeline.close_gap", ToolKind::Mutate, "Close the gap under (track, t) — ripple tracks only.", &["track:integer:true:", "t:number:true:"]),
    row!("timeline.mark", ToolKind::Mutate, "Set in/out to a clip's [start,end) — defaults to the clip under the playhead. Returns {in,out}.", &["clip_id:integer:false:default: clip under playhead"]),
    row!("timeline.in_out", ToolKind::Read, "Current in_point/out_point (null if unset).", &[]),
    row!("timeline.ripple_trim", ToolKind::Mutate, "Trim one edge of a clip; ripple=true shifts downstream ripple-tracked clips (end edge) or just carries markers/cues (start edge).", &["clip_id:integer:true:", "start:boolean:true:trim the start edge?", "edge:number:true:new edge time", "ripple:boolean:false:"]),
    row!("timeline.roll", ToolKind::Mutate, "Move a shared cut; total length of the two clips is unchanged.", &["right_clip_id:integer:true:", "cut:number:true:"]),
    row!("timeline.slip", ToolKind::Mutate, "Change the source window in place (start/duration unchanged).", &["clip_ids:array:true:", "dsrc:number:true:source seconds"]),
    row!("timeline.slide", ToolKind::Mutate, "Move a clip; its immediate neighbours absorb the change.", &["clip_id:integer:true:", "dt:number:true:"]),
    row!("timeline.trim_edges", ToolKind::Mutate, "Asymmetric multi-roller trim: every listed edge moves by the same dt, all-or-nothing.", &["edges:array:true:[[clip_id,is_start], ...]", "dt:number:true:", "ripple:boolean:false:lifts the same-call collision guard"]),
    row!("timeline.extend", ToolKind::Mutate, "Extend an edit point to a time (rolls a shared cut, ripple-trims a single side).", &["track:integer:true:", "t:number:true:the edit point's boundary time", "side:string:true:Left|Right|Both", "to:number:true:"]),
    row!("timeline.splice", ToolKind::Mutate, "Insert edit: ripple-opens space then places the asset. Canonical registration — source-monitor reuses this, does not re-register.", &["asset_id:integer:true:", "at:number:true:", "track:integer:false:", "in:number:false:source seconds", "out:number:false:"]),
    row!("timeline.overwrite", ToolKind::Mutate, "Overwrite edit: no ripple. Canonical — source-monitor reuses this, does not re-register.", &["asset_id:integer:true:", "at:number:true:", "track:integer:false:", "in:number:false:", "out:number:false:"]),
    row!("timeline.lift", ToolKind::Mutate, "Remove a range, leaving a gap. Canonical — source-monitor reuses this, does not re-register.", &["a:number:true:", "b:number:true:", "tracks:array:false:default every track"]),
    row!("timeline.extract", ToolKind::Mutate, "Remove a range and close the gap (ripple tracks by default). Canonical — source-monitor reuses this, does not re-register.", &["a:number:true:", "b:number:true:", "tracks:array:false:default ripple_tracks()"]),
    row!("timeline.join", ToolKind::Mutate, "Merge a clip with its right neighbour if contiguous/same asset.", &["clip_id:integer:true:the left clip"]),
    row!("timeline.duplicate", ToolKind::Mutate, "Duplicate clips (default: selection) onto a free track each.", &["clip_ids:array:false:default: selection"]),
    row!("timeline.unnest", ToolKind::Mutate, "Flatten a Sequence clip back onto the timeline at its original positions.", &["clip_id:integer:true:"]),
    row!("timeline.replace", ToolKind::Mutate, "Swap a clip's asset, keeping duration/effects/transform. Canonical name (supersedes timeline.replace_clip) — source-monitor calls Project::replace_clip directly instead of a second tool.", &["clip_id:integer:true:", "asset_id:integer:true:"]),
    row!("timeline.magnetic_move", ToolKind::Mutate, "Move clips; a blocked move onto a magnetic track opens space first instead of refusing.", &["clip_ids:array:true:", "dt:number:true:", "dtrack:integer:false:"]),
    row!("timeline.select_forward", ToolKind::Read, "Clip ids from a time forward, or backward.", &["t:number:false:default playhead", "track:integer:false:", "backward:boolean:false:"]),
    row!("timeline.clips_at", ToolKind::Read, "Clip ids covering a time.", &["t:number:false:default playhead"]),
    row!("timeline.edit_point", ToolKind::Ui, "Get/set/clear the selected cut for keyboard trimming (U / Shift+U). Omit track+t to just read it.", &["track:integer:false:", "t:number:false:", "side:string:false:Left|Right|Both (default: nearest)", "clear:boolean:false:"]),
    row!("timeline.keyframe_nav", ToolKind::Ui, "Seek to the previous/next keyframe of the selected clip.", &["direction:string:true:prev|next"]),
];
