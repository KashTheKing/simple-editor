//! ---- ws:forgiveness ----
//! MCP tools for every capability this workstream adds: autosave, recovery, history, caches, toasts
//! and the confirm queue. `history.restore`/`caches.clear`/`project.recover` live here in
//! `src/ui/app/*`, not `src/model/ops/*`, so they are exempt from `every_edit_op_has_a_tool`'s scan —
//! documented directly in `OP_INTERNAL` (tools_registry_tests.rs) rather than a test that scan can
//! never exercise.
//!
//! Kind choices deliberately deviate from the plan's own summary table for two rows: `project.autosave`
//! and `caches.clear` never touch `Project` (they write to disk / clear side caches), so they are
//! `ToolKind::Ui`, not `Mutate` — `handle_tool`'s Mutate path calls `App::after_edit` unconditionally
//! whenever `before.is_some()` (see mcp_exec.rs), which would mark a perfectly clean project dirty for
//! no project change at all. `project.recover` and `history.restore` DO replace the live project, so
//! they keep `Mutate` (and correctly end up dirty afterward — the recovered/restored state is not what
//! is on disk).

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "project.autosave",
        desc: "Force-writes the current project to the autosave folder now, regardless of the debounce; returns the path written.",
        args: &["force:boolean:false:write an autosave now regardless of the debounce"],
        kind: ToolKind::Ui,
        run: |app, _args| match autosave::force_write(app) {
            Some(p) => Ok(ToolOutcome::Done(json!({"ok": true, "path": p.to_string_lossy()}))),
            None => Err("autosave write failed".into()),
        },
    },
    ToolDef {
        name: "project.recover",
        desc: "Loads an autosave/backup as the live project. Omit path for the newest candidate.",
        args: &["path:string:false:autosave path to load; omitted = the newest candidate"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let path = match arg_str(args, "path") {
                Some(p) => PathBuf::from(p),
                None => recovery::recover_candidate(app.project_path.as_deref())
                    .ok_or_else(|| "no autosave/backup found".to_string())?,
            };
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            app.project = Project::from_json(&text)?;
            Ok(ToolOutcome::Done(json!({"ok": true, "path": path.to_string_lossy()})))
        },
    },
    ToolDef {
        name: "history.list",
        desc: "Undo-stack entries with their lazily-derived labels and categories, newest first.",
        args: &["limit:number:false:max rows, newest first"],
        kind: ToolKind::Read,
        run: |app, args| {
            let limit = arg_u64(args, "limit").map(|n| n as usize).unwrap_or(usize::MAX);
            let live = app.project.to_json();
            let mut rows: Vec<Value> = Vec::new();
            for (i, e) in app.undo.iter().enumerate().rev() {
                if rows.len() >= limit {
                    break;
                }
                let label = if !e.label.is_empty() {
                    e.label.clone()
                } else {
                    match app.undo.get(i + 1) {
                        Some(next) => describe_change(&e.json, &next.json),
                        None => describe_change(&e.json, &live),
                    }
                };
                rows.push(json!({
                    "index": i,
                    "label": label,
                    "category": format!("{:?}", e.category),
                    "restorable": e.category != HistoryCategory::Layout,
                }));
            }
            Ok(ToolOutcome::Done(json!(rows)))
        },
    },
    ToolDef {
        name: "history.restore",
        desc: "Restores the project to an undo-stack snapshot by index (0 = oldest); refuses Layout entries.",
        args: &["index:number:true:index into the undo stack (0 = oldest)"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let index = req(arg_u64(args, "index"), "index")? as usize;
            let entry =
                app.undo.get(index).cloned().ok_or_else(|| format!("no history entry at index {index}"))?;
            if entry.category == HistoryCategory::Layout {
                return Err("Layout history entries can't be restored".into());
            }
            app.project = Project::from_json(&entry.json)?;
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
    ToolDef {
        name: "caches.clear",
        desc: "Releases decoder file handles, clears in-memory waveform/thumb caches, deletes the on-disk cache dir; returns bytes freed.",
        args: &[],
        kind: ToolKind::Ui,
        run: |app, _args| {
            let freed = caches::cache_bytes();
            caches::clear(app);
            Ok(ToolOutcome::Done(json!({"ok": true, "bytes_freed": freed})))
        },
    },
    ToolDef {
        name: "caches.size",
        desc: "Returns the on-disk cache directory size in bytes.",
        args: &[],
        kind: ToolKind::Read,
        run: |_app, _args| Ok(ToolOutcome::Done(json!({"bytes": caches::cache_bytes()}))),
    },
    ToolDef {
        name: "ui.toast",
        desc: "Shows a toast identical to the app's own (dedupes against the last toast with the same text).",
        args: &["text:string:true:message", "kind:string:false:info|success|warn|error, default info"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let text = req(arg_str(args, "text"), "text")?;
            let kind = match arg_str(args, "kind") {
                Some("success") => feedback::ToastKind::Success,
                Some("warn") => feedback::ToastKind::Warn,
                Some("error") => feedback::ToastKind::Error,
                _ => feedback::ToastKind::Info,
            };
            app.push_toast(feedback::Toast::new(text).kind(kind));
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
    ToolDef {
        name: "ui.confirm_pending",
        desc: "Titles/bodies of currently-open non-blocking confirm windows, for a script that must wait on user input rather than racing it.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _args| {
            let list: Vec<Value> =
                app.confirm_active.iter().map(|p| json!({"title": p.title, "body": p.body})).collect();
            Ok(ToolOutcome::Done(json!(list)))
        },
    },
];
