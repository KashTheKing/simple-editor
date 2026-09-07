//! ---- ws:command-palette ----
//! The 9 MCP tools this workstream owns: `ui.palette`, `hotkeys.get`/`set`/`preset`, `ui.zoom_factor`,
//! `scripts.list`/`run`, `settings.get`/`set`. Registered once in `TOOL_TABLES` — see mod.rs.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::palette;

/// Scalar `Settings` fields the palette/scripts/MCP may read or write via `settings.get`/`set` —
/// deliberately small (a stringly-typed round-trip through JSON scalars, no nested structures): this is
/// a scripting convenience, not a settings-migration tool. Extend when a genuinely useful field shows up.
const SCALAR_WHITELIST: &[&str] = &[
    "ui_scale",
    "keymap_preset",
    "theme",
    "ui_look",
    "snap",
    "mcp_enabled",
    "mcp_port",
    "gpu",
    "preview_quality",
    "autosave_secs",
    "cache_mb",
    "layout_mode",
];

fn get_scalar(s: &Settings, key: &str) -> Option<Value> {
    Some(match key {
        "ui_scale" => json!(s.ui_scale),
        "keymap_preset" => json!(s.keymap_preset),
        "theme" => json!(s.theme),
        "ui_look" => json!(s.ui_look),
        "snap" => json!(s.snap),
        "mcp_enabled" => json!(s.mcp_enabled),
        "mcp_port" => json!(s.mcp_port),
        "gpu" => json!(s.gpu),
        "preview_quality" => json!(s.preview_quality),
        "autosave_secs" => json!(s.autosave_secs),
        "cache_mb" => json!(s.cache_mb),
        "layout_mode" => json!(s.layout_mode),
        _ => return None,
    })
}

fn parse_bool(v: &str) -> Result<bool, String> {
    match v {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(format!("expected true/false, got '{v}'")),
    }
}

fn set_scalar(s: &mut Settings, key: &str, value: &str) -> Result<(), String> {
    match key {
        "ui_scale" => s.ui_scale = value.parse().map_err(|_| "ui_scale: expected a number".to_string())?,
        "keymap_preset" => s.keymap_preset = value.to_string(),
        "theme" => s.theme = value.to_string(),
        "ui_look" => s.ui_look = value.to_string(),
        "snap" => s.snap = parse_bool(value)?,
        "mcp_enabled" => s.mcp_enabled = parse_bool(value)?,
        "mcp_port" => s.mcp_port = value.parse().map_err(|_| "mcp_port: expected an integer".to_string())?,
        "gpu" => s.gpu = parse_bool(value)?,
        "preview_quality" => {
            s.preview_quality = value.parse().map_err(|_| "preview_quality: expected an integer".to_string())?
        }
        "autosave_secs" => {
            s.autosave_secs = value.parse().map_err(|_| "autosave_secs: expected an integer".to_string())?
        }
        "cache_mb" => s.cache_mb = value.parse().map_err(|_| "cache_mb: expected an integer".to_string())?,
        "layout_mode" => s.layout_mode = value.to_string(),
        _ => return Err(format!("'{key}' is not a whitelisted setting")),
    }
    Ok(())
}

fn command_kind(cmd: &palette::Command) -> &'static str {
    match cmd {
        palette::Command::Action(_) => "action",
        palette::Command::Pane(_) => "pane",
        palette::Command::Tool(_) => "tool",
        palette::Command::Script(_) => "script",
        palette::Command::Workspace(_) => "workspace",
    }
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "ui.palette",
        desc: "List palette rows (Actions, Panes, arg-free Tools, Scripts, Workspaces), optionally fuzzy-filtered by 'query'.",
        args: &["query:string:false:filter text"],
        kind: ToolKind::Read,
        run: |app, args| {
            let query = arg_str(args, "query").unwrap_or("");
            let recent = app.settings.palette_recent.clone();
            let scripts = app.script_metas().to_vec();
            let rows = palette::rows(&app.hotkeys, |a| app.enabled(a), &scripts, &recent, query);
            let list: Vec<Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "label": r.label, "shortcut": r.shortcut, "enabled": r.enabled,
                        "reason": r.reason, "kind": command_kind(&r.cmd),
                    })
                })
                .collect();
            Ok(ToolOutcome::Done(json!(list)))
        },
    },
    ToolDef {
        name: "hotkeys.get",
        desc: "Every action id, label, chord text and section.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _args| {
            let list: Vec<Value> = Action::ALL
                .iter()
                .map(|&a| json!({"id": a.id(), "label": a.label(), "chord": app.hotkeys.text(a), "group": crate::hotkeys::group(a)}))
                .collect();
            Ok(ToolOutcome::Done(json!(list)))
        },
    },
    ToolDef {
        name: "hotkeys.set",
        desc: "Rebind one action. Empty chord unbinds.",
        args: &["action:string:true:Action id", "chord:string:true:e.g. Ctrl+Shift+B, empty unbinds"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let id = req(arg_str(args, "action"), "action")?;
            let a = Action::from_id(id).ok_or_else(|| format!("unknown action id '{id}'"))?;
            let chord = req(arg_str(args, "chord"), "chord")?;
            app.hotkeys.set(a, Hotkeys::parse(chord));
            app.hotkeys.to_settings(&mut app.settings);
            app.settings.save();
            Ok(ToolOutcome::Done(json!({"ok": true, "chord": app.hotkeys.text(a)})))
        },
    },
    ToolDef {
        name: "hotkeys.preset",
        desc: "Apply a keymap preset.",
        args: &["name:string:true:Simple Editor|Premiere|Resolve|Avid"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?;
            crate::keymaps::apply(name, &mut app.hotkeys)?;
            app.hotkeys.to_settings(&mut app.settings);
            app.settings.keymap_preset = name.to_string();
            app.settings.save();
            app.toast(format!("Keymap preset applied: {name}"));
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
    ToolDef {
        name: "ui.zoom_factor",
        desc: "Get/set the UI zoom factor (0.5-2.5); omit 'value' to read.",
        args: &["value:number:false:0.5-2.5, omit to read"],
        kind: ToolKind::Ui,
        run: |app, args| match arg_f64(args, "value") {
            Some(v) => {
                app.settings.ui_scale = v.clamp(0.5, 2.5) as f32;
                app.settings.save();
                Ok(ToolOutcome::Done(json!({"ok": true, "ui_scale": app.settings.ui_scale})))
            }
            None => Ok(ToolOutcome::Done(json!({"ui_scale": app.settings.ui_scale}))),
        },
    },
    ToolDef {
        name: "scripts.list",
        desc: "Luau scripts with @name/@desc/@hotkey/@on metadata.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _args| {
            let list: Vec<Value> = app
                .script_metas()
                .iter()
                .map(|m| {
                    json!({
                        "name": m.name, "desc": m.desc, "icon": m.icon, "hotkey": m.hotkey, "on": m.on,
                        "budget_ms": m.budget.as_millis() as u64, "path": m.path.to_string_lossy(),
                    })
                })
                .collect();
            Ok(ToolOutcome::Done(json!(list)))
        },
    },
    ToolDef {
        name: "scripts.run",
        desc: "Run a script by name (file stem). Pass 'event' to test-run it as if an @on hook fired with \
               that JSON payload (its own @budget_ms applies); omit 'event' to queue a plain run, same as \
               picking it from the Scripts menu.",
        args: &["name:string:true:file stem", "event:object:false:test-run an @on hook's payload"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?;
            let m = app
                .script_metas()
                .iter()
                .find(|m| m.name == name)
                .cloned()
                .ok_or_else(|| format!("no script named '{name}'"))?;
            match args.get("event").filter(|e| !e.is_null()) {
                Some(event) => {
                    let src = std::fs::read_to_string(&m.path).map_err(|e| e.to_string())?;
                    let mut logs = Vec::new();
                    let result = {
                        let app_cell = std::cell::RefCell::new(&mut *app);
                        let mut call = |tool: &str, a: &Value| -> Result<Value, String> {
                            app_cell.borrow_mut().run_tool_undoable(tool, a)
                        };
                        scripting::run_hook(&src, &m.name, event, m.budget, &mut call, &mut logs)
                    };
                    result?;
                    Ok(ToolOutcome::Done(json!({"ok": true, "logs": logs})))
                }
                None => {
                    app.run_script_path = Some(m.path);
                    Ok(ToolOutcome::Done(json!({"ok": true, "queued": true})))
                }
            }
        },
    },
    ToolDef {
        name: "settings.get",
        desc: "Read whitelisted scalar settings (omit 'key' for all of them).",
        args: &["key:string:false:omit for all whitelisted keys"],
        kind: ToolKind::Read,
        run: |app, args| match arg_str(args, "key") {
            Some(k) => get_scalar(&app.settings, k)
                .map(ToolOutcome::Done)
                .ok_or_else(|| format!("'{k}' is not a whitelisted setting")),
            None => {
                let mut obj = serde_json::Map::new();
                for &k in SCALAR_WHITELIST {
                    if let Some(v) = get_scalar(&app.settings, k) {
                        obj.insert(k.to_string(), v);
                    }
                }
                Ok(ToolOutcome::Done(Value::Object(obj)))
            }
        },
    },
    ToolDef {
        name: "settings.set",
        desc: "Set one whitelisted scalar setting ('value' is a stringified scalar).",
        args: &["key:string:true:", "value:string:true:stringified scalar"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let key = req(arg_str(args, "key"), "key")?;
            let value = req(arg_str(args, "value"), "value")?;
            set_scalar(&mut app.settings, key, value)?;
            app.settings.save();
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_scalar_whitelist_round_trips_and_rejects_unknown_key() {
        let mut s = Settings::default();
        set_scalar(&mut s, "ui_scale", "1.5").unwrap();
        assert_eq!(get_scalar(&s, "ui_scale"), Some(json!(1.5)));
        set_scalar(&mut s, "snap", "false").unwrap();
        assert_eq!(get_scalar(&s, "snap"), Some(json!(false)));
        let before = serde_json::to_string(&s).unwrap();
        assert!(set_scalar(&mut s, "not_a_real_key", "x").is_err());
        assert_eq!(get_scalar(&s, "not_a_real_key"), None);
        // a rejected key must not have mutated settings.json's shape
        assert_eq!(serde_json::to_string(&s).unwrap(), before);
    }

    #[test]
    fn every_whitelisted_key_has_a_getter_and_rejects_bad_values() {
        let mut s = Settings::default();
        for &k in SCALAR_WHITELIST {
            assert!(get_scalar(&s, k).is_some(), "{k} has no getter arm");
        }
        assert!(set_scalar(&mut s, "mcp_port", "not-a-number").is_err());
        assert!(set_scalar(&mut s, "snap", "maybe").is_err());
    }
}
