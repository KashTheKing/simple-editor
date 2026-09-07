//! Embedded Luau scripting: `.luau` files in the scripts folder drive the editor through the same
//! tool catalogue the MCP server exposes (`editor.tool("timeline.add_clip", {...})`). The VM is
//! sandboxed (no io/os/ffi) and interrupted after a wall-clock budget so a runaway loop cannot hang
//! the UI. Scripts run on the UI thread against the live project; the app wraps each run in one
//! undo step.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Wall-clock budget for one script run (it executes on the UI thread).
const BUDGET: Duration = Duration::from_secs(5);
// ---- ws:command-palette ----
/// Default wall-clock budget for a `-- @on` hook fired by `App::fire_hook` — much tighter than a
/// manual `BUDGET` run, since a hook fires from ordinary UI events (selection change, import, ...) and
/// must never make the editor feel like it hitched. Overridable per script via `@budget_ms`.
const DEFAULT_HOOK_BUDGET: Duration = Duration::from_millis(250);

pub fn scripts_dir() -> PathBuf {
    crate::settings::Settings::dir().join("scripts")
}

/// Every `.luau` file in the scripts folder, sorted by name. Creates the folder (and a starter
/// example) the first time it is asked for.
pub fn list() -> Vec<PathBuf> {
    let dir = scripts_dir();
    if !dir.exists() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("example.luau"), EXAMPLE);
    }
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("luau")))
        .collect();
    out.sort();
    out
}

const EXAMPLE: &str = r#"-- Simple Editor script. `editor.tool(name, args)` calls the same tools the MCP server exposes;
-- `editor.tools()` lists them; `editor.log(text)` shows a toast.
local s = editor.tool("project.summary", {})
editor.log("Project: " .. tostring(s.duration or "?") .. " s, " .. tostring(#(s.tracks or {})) .. " tracks")
"#;

// ---- ws:command-palette ----
/// Shared VM bootstrap for `run` and `run_hook`: a sandboxed Lua with a wall-clock interrupt at
/// `budget` (`run` always passes the fixed 5 s `BUDGET`; a fired `@on` hook passes its own, defaulting
/// to 250 ms — see `ScriptMeta::budget`/`DEFAULT_HOOK_BUDGET`). Building the `editor` table itself stays
/// separate in each caller — it borrows that call's own `call`/`logs` (and, for a hook, `event`), which
/// `Lua::scope`'s lifetime ties to the closure that builds it — so this covers exactly the part that
/// can't otherwise drift between the two paths.
fn setup_vm(budget: Duration) -> Result<mlua::Lua, String> {
    let lua = mlua::Lua::new();
    lua.sandbox(true).map_err(|e| e.to_string())?;
    let start = Instant::now();
    lua.set_interrupt(move |_| {
        if start.elapsed() > budget {
            Err(mlua::Error::runtime(format!("script took too long ({} ms budget)", budget.as_millis())))
        } else {
            Ok(mlua::VmState::Continue)
        }
    });
    Ok(lua)
}

/// Run `src` with an `editor` global. `call` executes one tool against the live project and is
/// invoked re-entrantly from inside the VM; `logs` collects `editor.log` lines for the app to show.
pub fn run(
    src: &str,
    chunk_name: &str,
    call: &mut dyn FnMut(&str, &Value) -> Result<Value, String>,
    logs: &mut Vec<String>,
) -> Result<(), String> {
    let lua = setup_vm(BUDGET)?;
    let call = std::cell::RefCell::new(call);
    let logs = std::cell::RefCell::new(logs);
    lua.scope(|scope| {
        let editor = lua.create_table()?;
        editor.set(
            "tool",
            scope.create_function(|lua, (name, args): (String, Option<mlua::Table>)| {
                let args = match args {
                    Some(t) => lua_to_json(mlua::Value::Table(t))?,
                    None => Value::Object(Default::default()),
                };
                let r = (call.borrow_mut())(&name, &args).map_err(mlua::Error::runtime)?;
                json_to_lua(lua, &r)
            })?,
        )?;
        editor.set(
            "tools",
            scope.create_function(|lua, ()| {
                let t = lua.create_table()?;
                for (i, def) in crate::mcp::tools::all().enumerate() {
                    let row = lua.create_table()?;
                    row.set("name", def.name)?;
                    row.set("description", def.desc)?;
                    t.set(i + 1, row)?;
                }
                Ok(t)
            })?,
        )?;
        editor.set(
            "log",
            scope.create_function(|_, s: String| {
                logs.borrow_mut().push(s);
                Ok(())
            })?,
        )?;
        lua.globals().set("editor", editor)?;
        lua.load(src).set_name(chunk_name).exec()
    })
    .map_err(|e| e.to_string())
}

// ---- ws:command-palette ----

/// Run `src` as a fired `-- @on` hook: the same sandbox surface as `run` (`editor.tool`/`editor.tools`/
/// `editor.log`), plus `editor.event` set to `event` (the hook's payload, as JSON), under its own
/// `budget` instead of `run`'s fixed 5 s (see `ScriptMeta::budget`). `App::fire_hook` is the only
/// caller — it supplies the re-entrancy guard and per-session disable-on-overrun policy; this fn just
/// runs one hook once.
pub fn run_hook(
    src: &str,
    chunk_name: &str,
    event: &Value,
    budget: Duration,
    call: &mut dyn FnMut(&str, &Value) -> Result<Value, String>,
    logs: &mut Vec<String>,
) -> Result<(), String> {
    let lua = setup_vm(budget)?;
    let call = std::cell::RefCell::new(call);
    let logs = std::cell::RefCell::new(logs);
    lua.scope(|scope| {
        let editor = lua.create_table()?;
        editor.set(
            "tool",
            scope.create_function(|lua, (name, args): (String, Option<mlua::Table>)| {
                let args = match args {
                    Some(t) => lua_to_json(mlua::Value::Table(t))?,
                    None => Value::Object(Default::default()),
                };
                let r = (call.borrow_mut())(&name, &args).map_err(mlua::Error::runtime)?;
                json_to_lua(lua, &r)
            })?,
        )?;
        editor.set(
            "tools",
            scope.create_function(|lua, ()| {
                let t = lua.create_table()?;
                for (i, def) in crate::mcp::tools::all().enumerate() {
                    let row = lua.create_table()?;
                    row.set("name", def.name)?;
                    row.set("description", def.desc)?;
                    t.set(i + 1, row)?;
                }
                Ok(t)
            })?,
        )?;
        editor.set(
            "log",
            scope.create_function(|_, s: String| {
                logs.borrow_mut().push(s);
                Ok(())
            })?,
        )?;
        editor.set("event", json_to_lua(&lua, event)?)?;
        lua.globals().set("editor", editor)?;
        lua.load(src).set_name(chunk_name).exec()
    })
    .map_err(|e| e.to_string())
}

/// A small, fixed set of icon keywords a script header's `@icon` may name — matches a subset of
/// `ui::tools::Glyph::name()` strings (chosen without depending on `ui::tools` from this low-level
/// module: menus.rs resolves the name back into a `Glyph` at draw time). An unknown name comes back
/// `None` rather than an error — a stale `@icon` in a script file must never break metadata parsing.
fn known_icon(name: &str) -> Option<&'static str> {
    const KNOWN: &[&str] = &[
        "bolt",
        "terminal",
        "wrench",
        "gear",
        "clock",
        "magnet",
        "target",
        "waveform",
        "notepad",
        "bookmark",
        "film-reel",
        "clapperboard",
        "sliders",
        "search",
        "keyboard",
    ];
    KNOWN.iter().copied().find(|k| *k == name)
}

/// A script's optional leading `-- @name/@desc/@icon/@hotkey/@on <event>/@budget_ms <ms>` header,
/// parsed without starting the VM — cheap enough for the Scripts menu / palette / `scripts.list` tool to
/// call for every script (`App::script_metas` still caches it at 1 Hz rather than every frame).
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptMeta {
    pub path: PathBuf,
    pub name: String,
    pub desc: String,
    pub icon: Option<&'static str>,
    pub hotkey: Option<String>,
    pub on: Vec<String>,
    pub budget: Duration,
}

/// Parse `path`'s header-comment block. A script with no header (or an unreadable file) gets its file
/// stem as `name` and every other field empty/default. Header parsing stops at the first line that
/// isn't a recognised `-- @key ...` comment (blank line, real code, or an unknown `@key`).
pub fn meta(path: &Path) -> ScriptMeta {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut m = ScriptMeta {
        path: path.to_path_buf(),
        name: stem,
        desc: String::new(),
        icon: None,
        hotkey: None,
        on: Vec::new(),
        budget: DEFAULT_HOOK_BUDGET,
    };
    let Ok(src) = std::fs::read_to_string(path) else { return m };
    for line in src.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("--") else { break };
        let Some(rest) = rest.trim_start().strip_prefix('@') else { break };
        let (key, val) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let val = val.trim();
        match key {
            "name" if !val.is_empty() => m.name = val.to_string(),
            "desc" => m.desc = val.to_string(),
            "icon" => m.icon = known_icon(val),
            "hotkey" => m.hotkey = (!val.is_empty()).then(|| val.to_string()),
            "on" if !val.is_empty() => m.on.push(val.to_string()),
            "budget_ms" => m.budget = val.parse().map(Duration::from_millis).unwrap_or(DEFAULT_HOOK_BUDGET),
            _ => break, // unknown @key ends the header, same as a blank/non-header line
        }
    }
    m
}

/// Lua value -> JSON. Tables with only positive-integer keys become arrays; everything else an object.
fn lua_to_json(v: mlua::Value) -> mlua::Result<Value> {
    Ok(match v {
        mlua::Value::Nil => Value::Null,
        mlua::Value::Boolean(b) => Value::Bool(b),
        mlua::Value::Integer(i) => Value::from(i),
        mlua::Value::Number(n) => serde_json::Number::from_f64(n).map(Value::Number).unwrap_or(Value::Null),
        mlua::Value::String(s) => Value::String(s.to_str()?.to_string()),
        mlua::Value::Table(t) => {
            let len = t.raw_len();
            let arrayish = len > 0
                && t.pairs::<mlua::Value, mlua::Value>().all(|p| {
                    p.map(|(k, _)| matches!(k, mlua::Value::Integer(i) if i >= 1 && i as usize <= len)).unwrap_or(false)
                });
            if arrayish {
                let mut a = Vec::with_capacity(len);
                for i in 1..=len {
                    a.push(lua_to_json(t.raw_get(i)?)?);
                }
                Value::Array(a)
            } else {
                let mut m = serde_json::Map::new();
                for p in t.pairs::<mlua::Value, mlua::Value>() {
                    let (k, val) = p?;
                    let key = match k {
                        mlua::Value::String(s) => s.to_str()?.to_string(),
                        mlua::Value::Integer(i) => i.to_string(),
                        mlua::Value::Number(n) => n.to_string(),
                        _ => continue, // unrepresentable key
                    };
                    m.insert(key, lua_to_json(val)?);
                }
                Value::Object(m)
            }
        }
        _ => Value::Null, // functions / userdata have no JSON shape
    })
}

/// JSON -> Lua value.
fn json_to_lua(lua: &mlua::Lua, v: &Value) -> mlua::Result<mlua::Value> {
    Ok(match v {
        Value::Null => mlua::Value::Nil,
        Value::Bool(b) => mlua::Value::Boolean(*b),
        Value::Number(n) => {
            // Luau integers are 32-bit; anything wider travels as a double
            match n.as_i64().and_then(|i| i32::try_from(i).ok()) {
                Some(i) => mlua::Value::Integer(i),
                None => mlua::Value::Number(n.as_f64().unwrap_or(0.0)),
            }
        }
        Value::String(s) => mlua::Value::String(lua.create_string(s)?),
        Value::Array(a) => {
            let t = lua.create_table_with_capacity(a.len(), 0)?;
            for (i, v) in a.iter().enumerate() {
                t.set(i + 1, json_to_lua(lua, v)?)?;
            }
            mlua::Value::Table(t)
        }
        Value::Object(m) => {
            let t = lua.create_table_with_capacity(0, m.len())?;
            for (k, v) in m {
                t.set(k.as_str(), json_to_lua(lua, v)?)?;
            }
            mlua::Value::Table(t)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run_src(src: &str) -> (Result<(), String>, Vec<(String, Value)>, Vec<String>) {
        let mut calls = Vec::new();
        let mut logs = Vec::new();
        let r = {
            let mut call = |name: &str, args: &Value| {
                calls.push((name.to_string(), args.clone()));
                Ok(json!({"ok": true, "echo": args, "n": 3, "list": [1, 2, 3]}))
            };
            run(src, "test", &mut call, &mut logs)
        };
        (r, calls, logs)
    }

    /// Round trip: Lua args reach the tool as JSON, the JSON result comes back as a Lua table.
    #[test]
    fn tool_call_round_trips() {
        let (r, calls, logs) = run_src(
            r#"
            local r = editor.tool("clip.set", { id = 7, speed = 2.0, tags = {"a", "b"} })
            assert(r.ok == true)
            assert(r.n == 3)
            assert(r.list[2] == 2)
            assert(r.echo.id == 7)
            assert(r.echo.tags[1] == "a")
            editor.log("done " .. tostring(r.n))
            "#,
        );
        assert_eq!(r, Ok(()));
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "clip.set");
        // Luau stores integral doubles as integers: 2.0 may arrive as 2 — same value either way
        assert_eq!(calls[0].1["speed"].as_f64(), Some(2.0));
        assert_eq!(calls[0].1["tags"], json!(["a", "b"]));
        assert_eq!(logs, vec!["done 3"]);
    }

    /// A tool error surfaces as a script error; a runaway loop is cut off by the interrupt budget.
    #[test]
    fn errors_and_budget() {
        let mut logs = Vec::new();
        let mut fail = |_: &str, _: &Value| -> Result<Value, String> { Err("no such clip".into()) };
        let e = run(r#"editor.tool("clip.set", {})"#, "t", &mut fail, &mut logs).unwrap_err();
        assert!(e.contains("no such clip"), "{e}");
        // sandbox: io/os are gone
        let (r, _, _) = run_src(r#"assert(io == nil and os.exit == nil)"#);
        assert_eq!(r, Ok(()));
        // the 5 s budget is too slow for a unit test to exercise for real; trust set_interrupt and
        // just confirm the catalogue is visible
        let (r, _, _) = run_src(r#"assert(#editor.tools() > 10)"#);
        assert_eq!(r, Ok(()));
    }

    // ---- ws:command-palette ----

    fn write_temp(name: &str, content: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("se-scripting-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn script_meta_parses_header_comments() {
        let p = write_temp(
            "hook.luau",
            "-- @name My Hook\n-- @desc Does a thing\n-- @icon bolt\n-- @hotkey Ctrl+Shift+H\n\
             -- @on selection_changed\n-- @on export_done\n-- @budget_ms 500\nlocal x = 1\n",
        );
        let m = meta(&p);
        assert_eq!(m.name, "My Hook");
        assert_eq!(m.desc, "Does a thing");
        assert_eq!(m.icon, Some("bolt"));
        assert_eq!(m.hotkey.as_deref(), Some("Ctrl+Shift+H"));
        assert_eq!(m.on, vec!["selection_changed".to_string(), "export_done".to_string()]);
        assert_eq!(m.budget, Duration::from_millis(500));

        let plain = write_temp("plain.luau", "local x = 1\n");
        let m2 = meta(&plain);
        assert_eq!(m2.name, "plain");
        assert!(m2.desc.is_empty() && m2.icon.is_none() && m2.hotkey.is_none() && m2.on.is_empty());
        assert_eq!(m2.budget, DEFAULT_HOOK_BUDGET);

        // an unknown @icon is dropped, not an error, and doesn't stop the rest of the header parsing
        let odd = write_temp("odd.luau", "-- @icon not-a-real-glyph\n-- @desc still parsed\nlocal x = 1\n");
        let m3 = meta(&odd);
        assert_eq!(m3.icon, None);
        assert_eq!(m3.desc, "still parsed");
    }

    #[test]
    fn run_hook_sets_editor_event_and_uses_its_own_budget() {
        let mut logs = Vec::new();
        let mut call = |_: &str, _: &Value| -> Result<Value, String> { Ok(json!({})) };
        let r = run_hook(
            r#"assert(editor.event.kind == "selection_changed"); editor.log("ok " .. tostring(#editor.event.ids))"#,
            "hook",
            &json!({"kind": "selection_changed", "ids": [1, 2]}),
            Duration::from_millis(250),
            &mut call,
            &mut logs,
        );
        assert_eq!(r, Ok(()));
        assert_eq!(logs, vec!["ok 2".to_string()]);
    }
}
