//! ---- ws:layout-modes-onboarding ----
//! MCP tools for layout modes, workspaces, pins, surfacing, maximise and the welcome wizard - each a
//! thin call into `layout_ctl` / `Layout` / `Settings` (`ToolKind::Ui`: UI state, never the project,
//! never undo). `ui.action` covers the hotkey-shaped verbs (Workspace1..6 / MaximizePane / TogglePin /
//! ToggleSource plus the consumed ToggleLayoutMode / ShowWelcome) by id, as for any other Action.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::layout::WORKSPACES;

/// A pane by its title, case-insensitively ("inspector", "Auto-cut", …).
fn pane_by_name(s: &str) -> Result<Pane, String> {
    Pane::ALL
        .iter()
        .copied()
        .find(|p| p.title().eq_ignore_ascii_case(s.trim()))
        .ok_or_else(|| format!("unknown pane '{s}' (one of {})", Pane::ALL.iter().map(|p| p.title()).collect::<Vec<_>>().join(", ")))
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "layout.mode",
        desc: "Set the layout mode: 'dynamic' (a selection surfaces the pane that edits it) or 'granular' \
               (panes stay put, the helpful tab only glows). Re-evaluates the current selection right away.",
        args: &["mode:string:true:'dynamic' | 'granular'"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let mode = req(arg_str(args, "mode"), "mode")?;
            let dynamic = match mode {
                "dynamic" => true,
                "granular" => false,
                other => return Err(format!("mode must be 'dynamic' or 'granular', got '{other}'")),
            };
            layout_ctl::set_mode(app, dynamic);
            Ok(ToolOutcome::Done(json!({"ok": true, "mode": app.settings.layout_mode})))
        },
    },
    ToolDef {
        name: "layout.workspace",
        desc: "Switch to a named workspace (see layout.list's 'workspaces') - the same undo-preserving \
               swap as Alt+1..6 / the menu-bar strip.",
        args: &["name:string:true:one of Simple, Edit, Color, Audio, Text, Deliver"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?;
            let canonical = WORKSPACES
                .iter()
                .copied()
                .find(|w| w.eq_ignore_ascii_case(name.trim()))
                .ok_or_else(|| format!("unknown workspace '{name}' (one of {})", WORKSPACES.join(", ")))?;
            layout_ctl::switch_workspace(app, canonical);
            Ok(ToolOutcome::Done(json!({"ok": true, "workspace": canonical})))
        },
    },
    ToolDef {
        name: "layout.pin",
        desc: "Pin (on=true) or unpin a pane against selection-driven auto-surfacing: a pinned active tab keeps \
               its group from switching, and a pinned pane is never switched to (it glows instead).",
        args: &["pane:string:true:pane title, e.g. Inspector", "on:boolean:true:pin state"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let pane = pane_by_name(req(arg_str(args, "pane"), "pane")?)?;
            let on = req(arg_bool(args, "on"), "on")?;
            app.layout.set_pinned(pane, on);
            app.layout_dirty = true;
            Ok(ToolOutcome::Done(json!({"ok": true, "pane": pane.title(), "pinned": on})))
        },
    },
    ToolDef {
        name: "layout.surface",
        desc: "Reveal a pane now, the pin-aware way (result: Shown | Pinned | Hidden | Absent - Pinned/Hidden \
               mean the tab was NOT switched); force=true ignores pins and re-opens a hidden pane like the View menu.",
        args: &["pane:string:true:pane title", "force:boolean:false:ignore pins / hidden state (default false)"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let pane = pane_by_name(req(arg_str(args, "pane"), "pane")?)?;
            let result = if arg_bool(args, "force").unwrap_or(false) {
                app.surface(pane);
                "Shown".to_string()
            } else {
                let r = app.layout.reveal_auto(pane);
                if r == crate::ui::layout::Surfaced::Shown {
                    app.layout_dirty = true;
                }
                format!("{r:?}")
            };
            Ok(ToolOutcome::Done(json!({"ok": true, "pane": pane.title(), "result": result})))
        },
    },
    ToolDef {
        name: "layout.maximize",
        desc: "Maximise a pane to the full tile (the arrangement is stashed), or omit 'pane' to restore it.",
        args: &["pane:string:false:pane title; omit to unmaximize"],
        kind: ToolKind::Ui,
        run: |app, args| {
            match arg_str(args, "pane") {
                Some(name) => {
                    let pane = pane_by_name(name)?;
                    app.layout.maximize(pane);
                }
                None => app.layout.unmaximize(),
            }
            app.layout_dirty = true;
            let maximized = app.layout.maximized.as_ref().map(|(p, _)| p.title());
            Ok(ToolOutcome::Done(json!({"ok": true, "maximized": maximized})))
        },
    },
    ToolDef {
        name: "layout.list",
        desc: "Current layout mode, workspace (and every workspace name), pinned panes, the maximised pane, \
               and each pane's visibility / popped-out state.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _args| {
            let panes: Vec<Value> = Pane::ALL
                .iter()
                .map(|&p| {
                    json!({
                        "name": p.title(),
                        "visible": app.layout.is_visible(p),
                        "popped": app.layout.popped.contains(&p),
                        "pinned": app.layout.pinned.contains(&p),
                    })
                })
                .collect();
            Ok(ToolOutcome::Done(json!({
                "mode": if layout_ctl::is_dynamic(&app.settings) { "dynamic" } else { "granular" },
                "workspace": app.settings.workspace,
                "workspaces": WORKSPACES,
                "pinned": app.layout.pinned.iter().map(|p| p.title()).collect::<Vec<_>>(),
                "maximized": app.layout.maximized.as_ref().map(|(p, _)| p.title()),
                "home_screen": app.settings.home_screen,
                "onboarded": app.settings.onboarded,
                "panes": panes,
            })))
        },
    },
    ToolDef {
        name: "onboarding.reset",
        desc: "Re-arm the first-run welcome wizard (settings.onboarded=false) and open it now.",
        args: &[],
        kind: ToolKind::Ui,
        run: |app, _args| {
            app.settings.onboarded = false;
            app.settings.save();
            app.onboarding = Some(crate::ui::onboarding::Onboarding::new(&app.settings));
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Structural parity: every row here is registered exactly once crate-wide with a unique,
    /// namespaced name, every arg spec parses into a valid schema, and layout.list's output fields
    /// name the inputs layout.mode / layout.workspace / layout.pin accept (so a client can round-trip
    /// them without a live App, which no test in this crate can build).
    #[test]
    fn layout_tools_names_unique_and_args_parse() {
        let mine: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
        assert_eq!(
            mine,
            ["layout.mode", "layout.workspace", "layout.pin", "layout.surface", "layout.maximize", "layout.list", "onboarding.reset"]
        );
        for t in TOOLS {
            assert_eq!(mcp::tools::all().filter(|d| d.name == t.name).count(), 1, "{} registered once", t.name);
            assert!(t.name.contains('.'));
            let schema = mcp::tools::input_schema(t.args);
            assert_eq!(schema["type"], "object", "{}", t.name);
            for (_, p) in schema["properties"].as_object().unwrap() {
                let ty = p["type"].as_str().unwrap();
                assert!(matches!(ty, "string" | "boolean" | "number" | "integer" | "array" | "object"), "{}: {ty}", t.name);
            }
        }
        let schema = |name: &str| mcp::tools::input_schema(mcp::tools::find(name).unwrap().args);
        assert_eq!(schema("layout.mode")["required"], json!(["mode"]));
        assert_eq!(schema("layout.workspace")["required"], json!(["name"]));
        assert_eq!(schema("layout.pin")["required"], json!(["pane", "on"]));
        assert_eq!(schema("layout.pin")["properties"]["on"]["type"], "boolean");
        assert_eq!(schema("layout.maximize")["required"], json!([]));
        assert!(mcp::tools::find("layout.list").unwrap().args.is_empty());
        assert_eq!(mcp::tools::find("layout.list").unwrap().kind, ToolKind::Read);
        assert!(TOOLS.iter().filter(|t| t.name != "layout.list").all(|t| t.kind == ToolKind::Ui), "UI state only, never undo");
        // the hotkey-shaped verbs go through ui.action by id
        for id in ["workspace_1", "workspace_6", "maximize_pane", "toggle_pin", "toggle_source", "toggle_layout_mode", "show_welcome"] {
            assert!(Action::from_id(id).is_some(), "ui.action must resolve {id}");
        }
        // pane names resolve case-insensitively, and a bad one is a clear error
        assert_eq!(pane_by_name("inspector"), Ok(Pane::Inspector));
        assert_eq!(pane_by_name(" Auto-cut "), Ok(Pane::AutoCut));
        assert!(pane_by_name("Scopes").unwrap_err().contains("unknown pane"));
    }
}
