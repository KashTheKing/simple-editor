//! ---- ws:registries-schema-hooks ----
//! New this wave: `ui.action`/`ui.actions`/`selection.get`/`selection.set`/`playhead.get` — the bridge
//! an MCP client or a future palette needs to drive the UI itself, not just the project. None of these
//! touch `Project`, so none push undo (`ui.action` dispatches through the normal `act()` path next
//! frame, which pushes its own undo exactly as a keypress would).

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "ui.action",
        desc: "Dispatch a UI Action by id (see ui.actions). Toasts App::enabled's reason and no-ops when disabled.",
        args: &["id:string:true:Action id (Action::id())", "args:object:false:reserved, unused today"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let id = req(arg_str(args, "id"), "id")?;
            let a = Action::from_id(id).ok_or_else(|| format!("unknown action id '{id}'"))?;
            match app.enabled(a) {
                Ok(()) => {
                    app.pending_actions.push(a);
                    Ok(ToolOutcome::Done(json!({"ok": true})))
                }
                Err(reason) => {
                    app.toast(reason);
                    Ok(ToolOutcome::Done(json!({"ok": false, "reason": reason})))
                }
            }
        },
    },
    ToolDef {
        name: "ui.actions",
        desc: "List every Action: id, label, default chord text, whether it's currently enabled.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _args| {
            let list: Vec<Value> = Action::ALL
                .iter()
                .map(|&a| {
                    json!({
                        "id": a.id(), "label": a.label(),
                        "chord": app.hotkeys.text(a),
                        "enabled": app.enabled(a).is_ok(),
                    })
                })
                .collect();
            Ok(ToolOutcome::Done(json!(list)))
        },
    },
    ToolDef {
        name: "selection.get",
        desc: "Current clip/transition selection ids and the dominant SelectionKind.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _args| {
            Ok(ToolOutcome::Done(json!({
                "clip_ids": app.selection,
                "transition_ids": app.sel_transitions,
                "kind": format!("{:?}", app.selection_kind()),
            })))
        },
    },
    ToolDef {
        name: "selection.set",
        desc: "Replace the current selection (UI state, not a project edit — no undo).",
        args: &["clip_ids:array:false:", "transition_ids:array:false:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            if let Some(ids) = arg_ids(args, "clip_ids") {
                app.selection = ids;
            }
            if let Some(ids) = arg_ids(args, "transition_ids") {
                app.sel_transitions = ids;
            }
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
    ToolDef {
        name: "playhead.get",
        desc: "Current playhead time in seconds.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _args| Ok(ToolOutcome::Done(json!({"t": app.playhead}))),
    },
];
