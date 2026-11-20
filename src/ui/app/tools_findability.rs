//! ---- ws:forum-ui ----
//! MCP tools for the forum-findability edits (`model::ops::findability`): duplicate a track, arrange a
//! split screen, censor a range. Each is `Mutate`, so the generic snapshot/undo wrapper in mcp_exec.rs
//! makes it one undo step; the timeline menus reach the same `Project` fns through their own `Act`s.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::ops::findability::{Censor, Layout};

/// The clips a tool acts on: `clip_ids` when given, else the selection (links included).
fn target_ids(app: &App, args: &Value) -> Vec<Id> {
    let ids = arg_ids(args, "clip_ids").unwrap_or_else(|| app.selection.clone());
    app.project.expand_links(&ids)
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "track.duplicate",
        desc: "Duplicate a track (its clips, transitions and settings) right next to it. Returns the new track's index.",
        args: &["track:integer:true:track index"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let ti = req(arg_u64(args, "track"), "track")? as usize;
            let n = app.project.duplicate_track(ti).ok_or_else(|| format!("no track {ti}"))?;
            Ok(ToolOutcome::Done(json!({"track": n})))
        },
    },
    ToolDef {
        name: "clip.layout",
        desc: "Arrange visual clips into a split screen: each is moved, scaled to cover its cell and cropped to it. \
               layout: 2up, 3up, 2x2, 3x2 or pip (the first clip fills the frame, the second sits in the corner).",
        args: &[
            "layout:string:true:2up | 3up | 2x2 | 3x2 | pip",
            "clip_ids:array:false:clips in fill order (default: the selection)",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = req(arg_str(args, "layout"), "layout")?;
            let l = Layout::from_id(id).ok_or_else(|| format!("unknown layout '{id}'"))?;
            let ids = target_ids(app, args);
            if !app.project.apply_layout(&ids, l) {
                return Err("no video, image, text or shape clip to arrange".into());
            }
            Ok(ToolOutcome::Done(json!({"layout": l.id(), "cells": l.cells(1.0, 1.0).len()})))
        },
    },
    ToolDef {
        name: "clip.censor",
        desc: "Hide something over a time range. what=blur adds a blur layer with a movable box over the range; \
               what=mute silences the clips' sound over it. Range: start/end, else In to Out, else the clips' span.",
        args: &[
            "what:string:true:blur | mute",
            "clip_ids:array:false:clips to mute (default: the selection; with none, every audio clip on the range)",
            "start:number:false:timeline seconds",
            "end:number:false:timeline seconds",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let what = match req(arg_str(args, "what"), "what")? {
                "blur" => Censor::Blur,
                "mute" => Censor::Mute,
                w => return Err(format!("unknown censor '{w}' (blur or mute)")),
            };
            let ids = target_ids(app, args);
            let range = match (arg_f64(args, "start"), arg_f64(args, "end")) {
                (Some(a), Some(b)) => Some((a, b)),
                _ => app.project.censor_range(&ids),
            };
            let (t0, t1) = range.ok_or("no range: give start and end, mark In and Out, or select clips")?;
            let id = app.project.censor(what, &ids, t0, t1).ok_or("nothing to censor on that range")?;
            Ok(ToolOutcome::Done(json!({"clip_id": id, "start": t0.min(t1), "end": t0.max(t1)})))
        },
    },
    ToolDef {
        name: "clip.rotate",
        desc: "Turn the selected (or given) visual clips by `degrees` (default 90, clockwise), keyframes included.",
        args: &["degrees:number:false:default 90", "clip_ids:array:false:default: the selection"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let d = arg_f64(args, "degrees").unwrap_or(90.0);
            let mut n = 0;
            for id in target_ids(app, args) {
                let Some(c) = app.project.clip_mut(id) else { continue };
                if !c.draws() {
                    continue;
                }
                c.rotation.value += d;
                c.rotation.keys.iter_mut().for_each(|k| k.v += d);
                n += 1;
            }
            if n == 0 {
                return Err("select a video, image, text or shape clip first".into());
            }
            Ok(ToolOutcome::Done(json!({"rotated": n})))
        },
    },
];
