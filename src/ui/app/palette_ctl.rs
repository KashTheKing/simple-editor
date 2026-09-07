//! ---- ws:command-palette ----
//! ACT_HANDLERS/WINDOW_DRAWERS/FRAME_HOOKS entries for the palette + cheat sheet, `App::fire_hook`'s
//! real implementation (re-entrancy guard, per-script budget, disable-on-overrun) and the 1 Hz
//! script-metadata cache a script's own `@hotkey` is polled from. `App::run_tool_undoable`
//! (`mcp_exec.rs`) is the shared runner the palette's Enter/arg-form-Run path and `tools_commands.rs`'s
//! `scripts.run` both call.

use super::*;
use crate::scripting::ScriptMeta;
use crate::ui::cheatsheet;
use crate::ui::palette::{self, Command};

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::CommandPalette => {
            app.cmd_palette.open = true;
            true
        }
        Action::CheatSheet => {
            app.cheat_sheet_open = true;
            true
        }
        // ToggleLayoutMode / ShowWelcome: declared exclusively in hotkeys.rs by this workstream (see its
        // doc comment there) but deliberately NOT handled here — both fall through App::act's `_ => {}`
        // catch-all, inert until ws:layout-modes-onboarding (wave 2) adds its own ACT_HANDLERS arm.
        _ => false,
    }
}

pub(super) fn windows(app: &mut App, ctx: &egui::Context) {
    cheatsheet::show(ctx, &app.hotkeys, &mut app.cheat_sheet_open);

    if !app.cmd_palette.open {
        return;
    }
    let query = app.cmd_palette.query.clone();
    let rows = match query.strip_prefix(':') {
        Some(rest) => palette::tool_rows(rest.trim_start()),
        None => {
            let recent = app.settings.palette_recent.clone();
            palette::rows(&app.hotkeys, |a| app.enabled(a), &app.script_meta_cache.1, &recent, &query)
        }
    };
    if let Some(cmd) = palette::show(ctx, &mut app.cmd_palette, &rows) {
        // read BEFORE anything else touches cmd_palette — `palette::show`'s doc comment guarantees this
        // is still the just-submitted arg form for exactly this Command::Tool result, nothing else's.
        let arg_form = app.cmd_palette.arg_form.take();
        remember_recent(app, &cmd);
        dispatch(app, cmd, arg_form);
    }
}

/// 1 Hz FRAME_HOOK: refresh the script-metadata cache, poll every script's own `@hotkey` (pressing it
/// runs the script directly, without opening the Scripts menu — an acceptance criterion of this ws),
/// and fire the one `@on` event this ws can reach from a file it owns (`selection_changed` — see
/// `fire_hook`'s doc comment for why the other five events aren't wired here).
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    refresh_if_stale(&mut app.script_meta_cache);
    let extra: Vec<(String, egui::KeyboardShortcut)> = app
        .script_meta_cache
        .1
        .iter()
        .filter_map(|m| m.hotkey.as_deref().and_then(Hotkeys::parse).map(|ks| (m.name.clone(), ks)))
        .collect();
    app.hotkeys.set_extra(extra.clone());
    if !ctx.wants_keyboard_input() {
        let hit = ctx.input_mut(|i| extra.iter().find(|pair| i.consume_shortcut(&pair.1)).cloned());
        if let Some((name, _)) = hit {
            if let Some(m) = app.script_meta_cache.1.iter().find(|m| m.name == name) {
                app.run_script_path = Some(m.path.clone());
            }
        }
    }
    if app.selection != app.last_fired_selection {
        app.last_fired_selection = app.selection.clone();
        let ids = app.selection.clone();
        app.fire_hook("selection_changed", json!({"clip_ids": ids}));
    }
}

fn refresh_if_stale(cache: &mut (Instant, Vec<ScriptMeta>)) {
    if cache.0.elapsed() < Duration::from_secs(1) {
        return;
    }
    cache.0 = Instant::now();
    cache.1 = scripting::list().iter().map(|p| scripting::meta(p)).collect();
}

/// Best-effort string -> JSON coercion for the palette's `:` arg form (every field is a plain text
/// box): `"true"`/`"false"` become booleans, a value that parses as a number becomes one, an empty,
/// non-required-looking field is omitted rather than sent as `""`, everything else stays a string.
fn arg_form_to_json(fields: &[(String, String)]) -> Value {
    let mut obj = serde_json::Map::new();
    for (k, v) in fields {
        if v.is_empty() {
            continue;
        }
        let val = match v.as_str() {
            "true" => json!(true),
            "false" => json!(false),
            _ => match v.parse::<f64>() {
                Ok(n) => json!(n),
                Err(_) => json!(v),
            },
        };
        obj.insert(k.clone(), val);
    }
    Value::Object(obj)
}

/// Palette id for a `Command`, in the same shape `Settings.palette_recent` stores (Action ids reuse
/// `Action::id()`; the others get a small namespaced prefix so they never collide with an Action id).
fn recent_id(cmd: &Command) -> Option<String> {
    Some(match cmd {
        Command::Action(a) => a.id().to_string(),
        Command::Pane(p) => format!("pane.{}", p.title()),
        Command::Tool(name) => format!("tool.{name}"),
        Command::Script(path) => format!("script.{}", path.to_string_lossy()),
        Command::Workspace(_) => return None, // the wave-0b/self-authored stub isn't worth remembering
    })
}

fn remember_recent(app: &mut App, cmd: &Command) {
    let Some(id) = recent_id(cmd) else { return };
    app.settings.palette_recent.retain(|r| *r != id);
    app.settings.palette_recent.insert(0, id);
    app.settings.palette_recent.truncate(20);
    app.settings.save();
}

fn dispatch(app: &mut App, cmd: Command, arg_form: Option<(&'static str, Vec<(String, String)>)>) {
    match cmd {
        Command::Action(a) => match app.enabled(a) {
            Ok(()) => app.pending_actions.push(a),
            Err(reason) => app.toast(reason),
        },
        Command::Pane(p) => app.surface(p),
        Command::Tool(name) => {
            let args = arg_form.filter(|(n, _)| *n == name).map(|(_, f)| arg_form_to_json(&f)).unwrap_or(json!({}));
            if let Err(e) = app.run_tool_undoable(name, &args) {
                app.toast(format!("{name}: {e}"));
            }
        }
        Command::Script(path) => app.run_script_path = Some(path),
        Command::Workspace(name) => {
            // ---- ws:layout-modes-onboarding ----
            // the placeholder toast this arm carried until wave 2: the real switch
            layout_ctl::switch_workspace(app, name);
        }
    }
}

impl App {
    /// Cached `scripting::list()` + `scripting::meta()` for every script — refreshed here if stale
    /// (`tick`'s 1 Hz FRAME_HOOK already keeps it warm while the app is running, so this rarely does
    /// real filesystem work) instead of every call site re-parsing every script's header on its own.
    pub(crate) fn script_metas(&mut self) -> &[ScriptMeta] {
        refresh_if_stale(&mut self.script_meta_cache);
        &self.script_meta_cache.1
    }

    /// Run every script whose `-- @on` list contains `event`, passing `payload` as Luau's `editor.event`
    /// — re-entrancy guarded (`self.hook_running`; a hook that itself triggers another `fire_hook` call,
    /// directly or via a tool it runs, is a no-op instead of recursing) and per-script budget-limited
    /// (`ScriptMeta::budget`, default 250 ms — see `scripting::DEFAULT_HOOK_BUDGET`). A script whose run
    /// overruns its budget is disabled for the rest of the session (`self.disabled_hooks`) with exactly
    /// one toast; every other error is logged, not surfaced (a silently-erroring hook must not spam
    /// toasts on every ordinary UI event it's subscribed to).
    ///
    /// // ponytail: this ws implements `fire_hook` itself (and the `scripts.run` MCP tool calls it via
    /// `event`, so it IS exercised end-to-end) but does not wire the six real call sites the plan names
    /// (selection_changed/import/export_done/project_open/project_save/marker_added) — every one of
    /// them lives in a file this ws does not own this wave (files.rs is forgiveness's; selection/import/
    /// export/marker code lives in actions.rs/panes.rs/library.rs/markers_ui.rs, none in this ws's Files
    /// table). Wiring them is a small, safe follow-up once landed: one `app.fire_hook("name", payload)`
    /// call at each site, added by whoever owns that file next (or a tiny same-day follow-up commit,
    /// same hand-off shape as the menus.rs/forgiveness note in the issue plan's Risks table).
    pub(crate) fn fire_hook(&mut self, event: &'static str, payload: Value) {
        let metas = self.script_meta_cache.1.clone();
        let targets = hooks_for_event(&metas, event, &self.disabled_hooks, self.hook_running);
        if targets.is_empty() {
            return;
        }
        self.hook_running = true;
        for m in targets {
            let Ok(src) = std::fs::read_to_string(&m.path) else { continue };
            let mut logs = Vec::new();
            let result = {
                let app = std::cell::RefCell::new(&mut *self);
                let mut call = |tool: &str, args: &Value| -> Result<Value, String> {
                    app.borrow_mut().run_tool_undoable(tool, args)
                };
                scripting::run_hook(&src, &m.name, &payload, m.budget, &mut call, &mut logs)
            };
            for l in &logs {
                self.toast(format!("{}: {l}", m.name));
            }
            if let Err(e) = result {
                if is_budget_error(&e) {
                    self.disabled_hooks.push(m.path.clone());
                    self.toast(format!(
                        "{}: disabled for this session (over its {} ms budget)",
                        m.name,
                        m.budget.as_millis()
                    ));
                } else {
                    eprintln!("hook {} (@on {event}): {e}", m.name);
                }
            }
        }
        self.hook_running = false;
    }
}

/// Which of `metas` should fire for `event`: not currently disabled, and only when no hook is already
/// running (the re-entrancy guard). Pure and App-free on purpose — see `fire_hook_targets_matching_
/// scripts_guards_reentrancy_and_skips_disabled` below; there is no headless `App` harness in this crate
/// (`App::new` needs a real `eframe::CreationContext`), so `App::fire_hook` itself stays untested
/// directly, same as `App::enabled`/`App::run_rollback` elsewhere in `ui::app`.
fn hooks_for_event<'a>(
    metas: &'a [ScriptMeta],
    event: &str,
    disabled: &[std::path::PathBuf],
    already_running: bool,
) -> Vec<&'a ScriptMeta> {
    if already_running {
        return Vec::new();
    }
    metas.iter().filter(|m| m.on.iter().any(|e| e == event) && !disabled.contains(&m.path)).collect()
}

/// `scripting::run_hook`'s interrupt error text (see `setup_vm`'s `mlua::Error::runtime(format!(...))`)
/// — matched by substring rather than a typed error so `scripting.rs` doesn't need an error enum just
/// for this one caller to distinguish "ran out of time" from every other script failure.
fn is_budget_error(e: &str) -> bool {
    e.contains("took too long")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_meta(name: &str, on: &[&str]) -> ScriptMeta {
        ScriptMeta {
            path: PathBuf::from(format!("{name}.luau")),
            name: name.to_string(),
            desc: String::new(),
            icon: None,
            hotkey: None,
            on: on.iter().map(|s| s.to_string()).collect(),
            budget: Duration::from_millis(250),
        }
    }

    #[test]
    fn fire_hook_targets_matching_scripts_guards_reentrancy_and_skips_disabled() {
        let a = dummy_meta("a", &["selection_changed"]);
        let b = dummy_meta("b", &["export_done"]);
        let c = dummy_meta("c", &["selection_changed"]);
        let metas = vec![a.clone(), b.clone(), c.clone()];

        let hit = hooks_for_event(&metas, "selection_changed", &[], false);
        assert_eq!(hit.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), vec!["a", "c"]);

        // re-entrancy guard: nothing fires while a hook is already running
        assert!(hooks_for_event(&metas, "selection_changed", &[], true).is_empty());

        // a disabled (over-budget) script is skipped, its sibling still fires
        let hit2 = hooks_for_event(&metas, "selection_changed", &[a.path.clone()], false);
        assert_eq!(hit2.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), vec!["c"]);

        // an event nothing subscribes to fires nothing
        assert!(hooks_for_event(&metas, "project_open", &[], false).is_empty());
    }

    #[test]
    fn arg_form_to_json_coerces_bool_and_number_and_omits_empty() {
        let v = arg_form_to_json(&[
            ("flag".into(), "true".into()),
            ("count".into(), "3".into()),
            ("name".into(), "hello".into()),
            ("skip_me".into(), String::new()),
        ]);
        assert_eq!(v["flag"], json!(true));
        assert_eq!(v["count"], json!(3.0));
        assert_eq!(v["name"], json!("hello"));
        assert!(v.get("skip_me").is_none());
    }

    #[test]
    fn recent_id_namespaces_non_action_commands() {
        assert_eq!(recent_id(&Command::Action(Action::Undo)), Some("undo".to_string()));
        assert_eq!(recent_id(&Command::Tool("ui.palette")), Some("tool.ui.palette".to_string()));
        assert_eq!(recent_id(&Command::Workspace("Default")), None);
    }
}
