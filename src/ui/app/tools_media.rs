use super::tools_helpers::*;
use super::*;

pub(super) fn dispatch(app: &mut App, name: &str, args: &Value) -> Option<Result<Value, String>> {
    let prefix = name.split('.').next().unwrap_or("");
    if !matches!(prefix, "media") {
        return None;
    }
    pub(super) fn run(app: &mut App, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "media.import" => {
                let paths: Vec<PathBuf> = req(args.get("paths").and_then(|v| v.as_array()), "paths")?
                    .iter()
                    .filter_map(|v| v.as_str().map(PathBuf::from))
                    .collect();
                let ids = app.import_files(&paths);
                Ok(json!({"ok": true, "asset_ids": ids}))
            }
            "media.list" => {
                let used = app.project.used_assets();
                let list: Vec<Value> = app
                    .project
                    .assets
                    .iter()
                    .map(|a| {
                        json!({
                            "id": a.id, "path": a.path, "kind": format!("{:?}", a.kind),
                            "duration": a.duration, "width": a.width, "height": a.height,
                            "tags": a.tags, "label": a.label, "folder": a.folder,
                            "description": a.description, "used": used.contains(&a.id),
                        })
                    })
                    .collect();
                Ok(json!(list))
            }
            "media.set" => {
                let id = req(arg_u64(args, "id"), "id")?;
                let a = app.project.asset_mut(id).ok_or("no such asset")?;
                if let Some(d) = arg_str(args, "description") {
                    a.description = d.to_string();
                }
                if let Some(tags) = args.get("tags").and_then(|v| v.as_array()) {
                    a.tags = tags.iter().filter_map(|t| t.as_str().map(String::from)).collect();
                }
                if let Some(l) = arg_u64(args, "label") {
                    a.label = l.min(8) as u8;
                }
                if let Some(f) = arg_str(args, "folder") {
                    a.folder = f.to_string();
                }
                Ok(json!({"ok": true}))
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
    // media.import self-manages its own undo push (`App::import_files`) — wrapping it in the generic
    // Mutate snapshot/rollback would double-push, so it stays Read here just as it was absent from the
    // old hand-kept mutating-tool name list.
    row!("media.import", ToolKind::Read, "Import media files into the library; returns asset ids.", &["paths:array:true:absolute paths"]),
    row!("media.list", ToolKind::Read, "Library assets (id, path, kind, duration, size, tags, label, folder, description, used).", &[]),
    row!("media.set", ToolKind::Mutate, "Edit asset metadata.", &["id:integer:true:", "description:string:false:", "tags:array:false:strings", "label:integer:false:0..8", "folder:string:false:"]),
    // Job-kind: never routed through `dispatch` (they don't mutate the project directly, they write a
    // file) — `App::handle_tool` starts the job via `start_tool_job` and replies when it finishes.
    ToolDef {
        name: "media.convert",
        desc: "Convert a file with ffmpeg (gif/mp4/mov/mkv/webm/mp3/wav…); returns the output path when done (blocks up to 10 min).",
        args: &["path:string:true:source", "ext:string:true:target extension", "width:integer:false:", "height:integer:false:", "scaler:string:false:neighbor|bilinear|bicubic|lanczos"],
        kind: ToolKind::Job,
        run: |app, args| app.start_tool_job("media.convert", args).map(|(p, o)| ToolOutcome::Job(p, o)),
    },
    ToolDef {
        name: "export.video",
        desc: "Export the timeline to a file (blocks until done, up to 30 min); encoder/crf default to settings.",
        args: &["path:string:true:", "encoder:string:false:", "crf:integer:false:", "width:integer:false:", "height:integer:false:", "scaler:string:false:"],
        kind: ToolKind::Job,
        run: |app, args| app.start_tool_job("export.video", args).map(|(p, o)| ToolOutcome::Job(p, o)),
    },
    // ---- ws:registries-schema-hooks: new this wave ----
    ToolDef {
        name: "media.subclip",
        desc: "Create a subclip asset (a named in/out range into an existing library asset).",
        args: &["asset_id:integer:true:", "in:number:true:seconds", "out:number:true:seconds", "name:string:false:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let asset_id = req(arg_u64(args, "asset_id"), "asset_id")?;
            let in_t = req(arg_f64(args, "in"), "in")?;
            let out_t = req(arg_f64(args, "out"), "out")?;
            let name = arg_str(args, "name").map(str::to_string);
            let id = app.project.add_subclip(asset_id, in_t, out_t, name).ok_or("no such asset, or an invalid range")?;
            Ok(ToolOutcome::Done(json!({"ok": true, "id": id})))
        },
    },
];
