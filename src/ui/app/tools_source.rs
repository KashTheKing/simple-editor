//! ---- ws:source-monitor ----
//! MCP tools for the Source monitor (`source.*`) plus the three `timeline.*` verbs this workstream
//! owns: `timeline.place` (the `DropMode` placement funnel), `timeline.match_frame`,
//! `timeline.smart_edit`. Splice/overwrite/lift/extract/replace are NOT re-registered here -
//! trim-model's `tools_trim.rs` owns `timeline.splice`/`overwrite`/`lift`/`extract`/`replace`; the
//! UI and the rows below call those `Project::` fns directly (`no_duplicate_tool_names_with_trim_model`
//! pins it). Every `Mutate` row relies on the generic snapshot/push-undo-iff-changed wrapper.

use super::edit_ops::DropMode;
use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};

fn done(v: Value) -> Result<ToolOutcome, String> {
    Ok(ToolOutcome::Done(v))
}

fn parse_mode(s: &str) -> Result<DropMode, String> {
    match s {
        "place" => Ok(DropMode::Place),
        "splice" => Ok(DropMode::Splice),
        "overwrite" => Ok(DropMode::Overwrite),
        "top" => Ok(DropMode::OnTop),
        _ => Err("mode: place|splice|overwrite|top".into()),
    }
}

fn range_arg(args: &Value) -> Option<(f64, f64)> {
    match (arg_f64(args, "in"), arg_f64(args, "out")) {
        (Some(i), Some(o)) => Some((i, o)),
        _ => None,
    }
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "source.open",
        desc: "Open a library asset (or any media file path) in the Source monitor; it takes transport focus.",
        args: &["asset_id:integer:false:", "path:string:false:one of asset_id/path required", "seek:number:false:source seconds"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let seek = arg_f64(args, "seek");
            match (arg_u64(args, "asset_id"), arg_str(args, "path")) {
                (Some(id), _) => {
                    if !app.open_asset_in_source(id, seek) {
                        return Err("no such asset".into());
                    }
                }
                (None, Some(p)) => app.open_in_source(PathBuf::from(p), seek),
                (None, None) => return Err("asset_id or path required".into()),
            }
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "source.mark",
        desc: "Set the source in/out marks (source seconds); omit a field to clear that mark. UI state only.",
        args: &["in:number:false:", "out:number:false:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let st = app.source.as_mut().ok_or("nothing is open in the Source monitor")?;
            st.src_in = arg_f64(args, "in").map(|t| t.clamp(0.0, st.duration));
            st.src_out = arg_f64(args, "out").map(|t| t.clamp(0.0, st.duration));
            done(json!({"ok": true, "in": st.src_in, "out": st.src_out}))
        },
    },
    ToolDef {
        name: "source.get",
        desc: "The Source monitor: asset id, path, duration, fps, marks, playhead, playing, tape, focused (null when empty).",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| {
            let focused = app.source_active();
            let Some(st) = app.source.as_ref() else { return done(Value::Null) };
            done(json!({
                "asset_id": st.asset, "path": st.path.to_string_lossy(), "duration": st.duration, "fps": st.fps,
                "in": st.src_in, "out": st.src_out, "playhead": st.player.time(), "playing": st.player.is_playing(),
                "tape": st.tape.as_ref().map(|t| t.assets.clone()), "focused": focused,
            }))
        },
    },
    ToolDef {
        name: "source.focus",
        desc: "Give the Source monitor transport focus (Space/JKL/I/O route there) and bring the pane forward.",
        args: &[],
        kind: ToolKind::Ui,
        run: |app, _| {
            if app.source.is_none() {
                return Err("nothing is open in the Source monitor".into());
            }
            app.source_focus = true;
            app.surface(Pane::Source);
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "source.tape",
        desc: "Build/refresh the Source Tape (the bin laid end to end) from asset_ids; omitted = the library selection, else every asset. off:true returns to the single clip.",
        args: &["asset_ids:array:false:", "off:boolean:false:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            if arg_bool(args, "off").unwrap_or(false) {
                let own = app.source_own_project();
                if let Some(s) = app.source.as_mut() {
                    s.set_tape(None, &own);
                }
                return done(json!({"ok": true}));
            }
            let ids: Vec<Id> = arg_ids(args, "asset_ids").unwrap_or_default();
            if let Some(&bad) = ids.iter().find(|&&id| app.project.asset(id).is_none()) {
                return Err(format!("no such asset {bad}"));
            }
            app.source_pending = Some(source_pane::Pending::Tape(ids));
            app.source_focus = true;
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "source.insert",
        desc: "Insert the open source clip's marked range into the timeline (three-point edit).",
        args: &["mode:string:true:place|splice|overwrite|top|append", "at:number:false:defaults to playhead", "track:integer:false:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let mode = req(arg_str(args, "mode"), "mode")?;
            let track = arg_u64(args, "track").map(|t| t as usize);
            let ids = app.source_insert(mode, arg_f64(args, "at"), track)?;
            done(json!({"ok": true, "clip_ids": ids}))
        },
    },
    ToolDef {
        name: "source.subclip",
        desc: "Create a library subclip asset from the current source in/out marks.",
        args: &["name:string:false:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let st = app.source.as_ref().ok_or("nothing is open in the Source monitor")?;
            let asset = st.asset.filter(|_| st.tape.is_none()).ok_or("subclips need an imported library clip")?;
            let (a, b) = st.marks().unwrap_or((0.0, st.duration));
            let name = arg_str(args, "name").map(str::to_string);
            let id = app.project.subclip_from_marks(asset, a, b, name).ok_or("invalid marks")?;
            done(json!({"ok": true, "asset_id": id}))
        },
    },
    ToolDef {
        name: "timeline.place",
        desc: "Place an asset with a DropMode: place (free track), splice (ripple-insert), overwrite (on a clip body = replace edit keeping duration/effects), top (new track above).",
        args: &["asset_id:integer:true:", "at:number:true:", "track:integer:false:", "mode:string:false:place|splice|overwrite|top (default place)", "in:number:false:source seconds", "out:number:false:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let asset = req(arg_u64(args, "asset_id"), "asset_id")?;
            if app.project.asset(asset).is_none() {
                return Err("no such asset".into());
            }
            let at = req(arg_f64(args, "at"), "at")?;
            let track = arg_u64(args, "track").map(|t| t as usize);
            let mode = parse_mode(arg_str(args, "mode").unwrap_or("place"))?;
            let ids = app.place_asset(asset, at, track, mode, range_arg(args));
            done(json!({"ok": true, "clip_ids": ids}))
        },
    },
    ToolDef {
        name: "timeline.match_frame",
        desc: "Open a clip's source asset in the Source monitor at the source time under the playhead (clip_id defaults to the clip under the playhead).",
        args: &["clip_id:integer:false:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            if app.match_frame(arg_u64(args, "clip_id")) {
                done(json!({"ok": true}))
            } else {
                Err("no media clip there".into())
            }
        },
    },
    ToolDef {
        name: "timeline.smart_edit",
        desc: "One of the four smart edits at the playhead using the open source clip's marked range.",
        args: &["kind:string:true:append|ripple_overwrite|close_up|place_on_top"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let kind = req(arg_str(args, "kind"), "kind")?;
            let ids = app.smart_edit(kind)?;
            done(json!({"ok": true, "clip_ids": ids}))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The audit fix this file exists for: trim-model's five canonical timeline verbs are registered
    /// exactly once crate-wide, and never by this table.
    #[test]
    fn no_duplicate_tool_names_with_trim_model() {
        for name in ["timeline.splice", "timeline.overwrite", "timeline.lift", "timeline.extract", "timeline.replace"] {
            assert!(TOOLS.iter().all(|t| t.name != name), "{name} must not be re-registered by tools_source");
            assert_eq!(mcp::tools::all().filter(|t| t.name == name).count(), 1, "{name} registered exactly once");
        }
    }

    /// `place_asset`/`place_assets` (not model ops, so outside `every_edit_op_has_a_tool`'s scan) and
    /// every source verb have a live ToolDef with the kind the plan specifies.
    #[test]
    fn every_edit_op_has_a_tool_covers_place_asset_and_source_verbs() {
        for (name, kind) in [
            ("timeline.place", ToolKind::Mutate),
            ("source.insert", ToolKind::Mutate),
            ("source.subclip", ToolKind::Mutate),
            ("timeline.smart_edit", ToolKind::Mutate),
            ("source.open", ToolKind::Ui),
            ("source.mark", ToolKind::Ui),
            ("source.focus", ToolKind::Ui),
            ("source.tape", ToolKind::Ui),
            ("timeline.match_frame", ToolKind::Ui),
            ("source.get", ToolKind::Read),
        ] {
            let def = mcp::tools::find(name).unwrap_or_else(|| panic!("{name} must be registered"));
            assert_eq!(def.kind, kind, "{name}");
        }
        assert_eq!(TOOLS.len(), 10);
        assert!(parse_mode("garbage").is_err());
        assert_eq!(parse_mode("top").unwrap(), DropMode::OnTop);
    }
}
