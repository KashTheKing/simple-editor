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
    // ---- ws:media-library ----
    ToolDef {
        name: "media.status",
        desc: "AssetStatus per asset id (Ready | Decoding | Offline | ProxyBuilding, with percent) from the same tick that drives the library badge.",
        args: &["id:integer:false:omit for every asset"],
        kind: ToolKind::Read,
        run: |app, args| {
            let one = |app: &App, a: &crate::model::Asset| {
                let s = app.asset_status(a.id);
                let pct = match s {
                    media_sync::AssetStatus::ProxyBuilding(p) => Some(p),
                    _ => None,
                };
                json!({"id": a.id, "path": a.path, "status": s.name(), "progress": pct})
            };
            Ok(ToolOutcome::Done(match arg_u64(args, "id") {
                Some(id) => one(app, app.project.asset(id).ok_or("no such asset")?),
                None => Value::Array(app.project.assets.iter().map(|a| one(app, a)).collect()),
            }))
        },
    },
    ToolDef {
        name: "media.relink",
        desc: "Relink offline assets: by file name, then by duration within one frame, inside dir (+ one level of subfolders); returns {relinked, still_missing}.",
        args: &["ids:array:true:asset ids", "dir:string:true:folder to search"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let ids = req(arg_ids(args, "ids"), "ids")?;
            let dir = PathBuf::from(req(arg_str(args, "dir"), "dir")?);
            if !dir.is_dir() {
                return Err(format!("{} is not a folder", dir.display()));
            }
            let (ok, missing) = media_sync::relink_assets(&mut app.project, &ids, &dir);
            app.offline_scan_at = None; // badges re-scan on the next frame
            Ok(ToolOutcome::Done(json!({"ok": true, "relinked": ok, "still_missing": missing})))
        },
    },
    ToolDef {
        name: "media.consolidate",
        desc: "Copy every asset from outside dir (default: the project's own folder) into it and repoint the paths — both halves run here, synchronously, as one undo step.",
        args: &["dir:string:false:defaults to the project's folder (the project must be saved)"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let dir = match arg_str(args, "dir") {
                Some(d) => PathBuf::from(d),
                None => media_sync::project_dir(app)?,
            };
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let list = media_sync::consolidate_list(&app.project, &dir);
            let results = Project::consolidate_assets_copy(&dir, &list);
            let n = app.project.apply_consolidate(&results);
            let failed: Vec<Value> = results
                .iter()
                .filter_map(|(id, _, r)| r.as_ref().err().map(|e| json!({"id": id, "error": e})))
                .collect();
            app.offline_scan_at = None;
            Ok(ToolOutcome::Done(json!({"ok": true, "copied": n, "failed": failed, "dir": dir.to_string_lossy()})))
        },
    },
    ToolDef {
        name: "media.smart_bin",
        desc: "Manage Project.smart_bins: save the current library filter under a name, apply one (by index or name) to the library, list them, or delete one.",
        args: &["op:string:true:save|apply|list|delete", "name:string:false:for save/apply/delete", "id:integer:false:index, for apply/delete"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let op = req(arg_str(args, "op"), "op")?;
            let find = |app: &App| -> Result<usize, String> {
                if let Some(i) = arg_u64(args, "id") {
                    let i = i as usize;
                    return if i < app.project.smart_bins.len() { Ok(i) } else { Err(format!("no smart bin at index {i}")) };
                }
                let name = req(arg_str(args, "name"), "name or id")?;
                app.project.smart_bins.iter().position(|b| b.name == name).ok_or_else(|| format!("no smart bin '{name}'"))
            };
            match op {
                "save" => {
                    let name = req(arg_str(args, "name"), "name")?.to_string();
                    let bin = crate::model::SmartBin { name: name.clone(), query: library::bin_query(&app.library) };
                    app.project.smart_bins.retain(|b| b.name != name);
                    app.project.smart_bins.push(bin);
                    Ok(ToolOutcome::Done(json!({"ok": true, "index": app.project.smart_bins.len() - 1})))
                }
                "apply" => {
                    let i = find(app)?;
                    let q = app.project.smart_bins[i].query.clone();
                    library::apply_bin(&mut app.library, &q);
                    Ok(ToolOutcome::Done(json!({"ok": true, "query": q})))
                }
                "list" => Ok(ToolOutcome::Done(Value::Array(
                    app.project
                        .smart_bins
                        .iter()
                        .enumerate()
                        .map(|(i, b)| json!({"index": i, "name": b.name, "query": b.query}))
                        .collect(),
                ))),
                "delete" => {
                    let i = find(app)?;
                    app.project.smart_bins.remove(i);
                    Ok(ToolOutcome::Done(json!({"ok": true})))
                }
                _ => Err(format!("unknown op '{op}' (save | apply | list | delete)")),
            }
        },
    },
    ToolDef {
        name: "media.import_sequence",
        desc: "Detect the numbered still run `path` belongs to (3+ frames, same prefix/extension) and bake it to one video asset in the background; fires the `import` hook once when it lands.",
        args: &["path:string:true:one frame of the sequence", "fps:number:false:defaults to the project fps"],
        kind: ToolKind::Job,
        run: |app, args| {
            let path = PathBuf::from(req(arg_str(args, "path"), "path")?);
            let (prog, out) = media_sync::start_import_sequence(app, &path, arg_f64(args, "fps"))?;
            Ok(ToolOutcome::Job(prog, out))
        },
    },
    ToolDef {
        name: "library.select",
        desc: "Set the library selection (asset ids and/or file paths); omit both to clear it.",
        args: &["ids:array:false:asset ids", "paths:array:false:files on disk (Global tab)"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let ids: Vec<Id> = arg_ids(args, "ids")
                .unwrap_or_default()
                .into_iter()
                .filter(|id| app.project.asset(*id).is_some())
                .collect();
            let paths: Vec<String> = args
                .get("paths")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let lib = &mut app.library;
            lib.selected = ids.last().copied();
            lib.sel_path = if ids.is_empty() { paths.last().cloned() } else { None };
            lib.seen_selected = lib.selected; // our own write: don't let show() collapse it again
            lib.sel_ids = ids;
            lib.sel_paths = paths;
            Ok(ToolOutcome::Done(json!({"ok": true, "ids": app.library.sel_ids, "paths": app.library.sel_paths})))
        },
    },
    ToolDef {
        name: "library.columns",
        desc: "Get/set Settings.library_columns — the cells a list row shows after the name, in order.",
        args: &["columns:array:false:omit to just read; each one of kind|duration|fps|size|label|tags|proxy"],
        // Settings, not Project: Ui (a Mutate row would mark the project dirty for a settings change)
        kind: ToolKind::Ui,
        run: |app, args| {
            if let Some(cols) = args.get("columns").and_then(|v| v.as_array()) {
                let mut out = Vec::new();
                for c in cols {
                    let c = c.as_str().ok_or("columns: array of strings")?;
                    if !library::COLUMNS.contains(&c) {
                        return Err(format!("unknown column '{c}' ({})", library::COLUMNS.join(" | ")));
                    }
                    if !out.iter().any(|x| x == c) {
                        out.push(c.to_string());
                    }
                }
                app.settings.library_columns = out;
                app.settings.save();
            }
            Ok(ToolOutcome::Done(json!({"ok": true, "columns": app.settings.library_columns})))
        },
    },
    ToolDef {
        name: "media.batch_convert",
        desc: "Convert every id with the same options (the scripted batch path — the Convert… window stays single-target); starts one job per asset and returns their output paths at once.",
        args: &["ids:array:true:asset ids", "ext:string:true:target extension", "width:integer:false:", "height:integer:false:", "scaler:string:false:neighbor|bilinear|bicubic|lanczos"],
        // Read, not Job: one ToolOutcome::Job carries one handle, and this starts N — the app's own
        // convert-job poll imports each result; a script polls media.list for them.
        kind: ToolKind::Read,
        run: |app, args| {
            if media::ffpipe::ffmpeg_exe().is_none() {
                return Err("ffmpeg.exe not found".into());
            }
            let ids = req(arg_ids(args, "ids"), "ids")?;
            let ext = req(arg_str(args, "ext"), "ext")?.trim_start_matches('.').to_string();
            let out_size = match (arg_u64(args, "width"), arg_u64(args, "height")) {
                (Some(w), Some(h)) => Some((w as u32, h as u32)),
                _ => None,
            };
            let scaler = arg_str(args, "scaler").unwrap_or(&app.settings.export_scaler).to_string();
            let mut outputs = Vec::new();
            for id in ids {
                let src = PathBuf::from(&app.project.asset(id).ok_or_else(|| format!("no asset {id}"))?.path);
                let out = converted_path(&src, &ext);
                let opts = crate::engine::convert::ConvertOptions {
                    src,
                    out: out.clone(),
                    encoder: app.settings.encoder.clone(),
                    crf: app.settings.crf,
                    preset: app.settings.preset.clone(),
                    out_size,
                    scaler: scaler.clone(),
                    gif_fps: 15,
                    target_bytes: None,
                    vf_extra: None,
                    af_extra: None,
                };
                app.convert_jobs.push((crate::engine::convert::start_convert(opts), out.clone()));
                outputs.push(json!({"id": id, "path": out.to_string_lossy()}));
            }
            Ok(ToolOutcome::Done(json!({"ok": true, "started": outputs.len(), "outputs": outputs})))
        },
    },
];
