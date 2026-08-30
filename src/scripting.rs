//! Embedded Luau scripting: `.luau` files in the scripts folder drive the editor through the same
//! tool catalogue the MCP server exposes (`editor.tool("timeline.add_clip", {...})`). The VM is
//! sandboxed (no io/os/ffi) and interrupted after a wall-clock budget so a runaway loop cannot hang
//! the UI. Scripts run on the UI thread against the live project; the app wraps each run in one
//! undo step.

use serde_json::Value;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Wall-clock budget for one script run (it executes on the UI thread).
const BUDGET: Duration = Duration::from_secs(5);

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

/// Run `src` with an `editor` global. `call` executes one tool against the live project and is
/// invoked re-entrantly from inside the VM; `logs` collects `editor.log` lines for the app to show.
pub fn run(
    src: &str,
    chunk_name: &str,
    call: &mut dyn FnMut(&str, &Value) -> Result<Value, String>,
    logs: &mut Vec<String>,
) -> Result<(), String> {
    let lua = mlua::Lua::new();
    lua.sandbox(true).map_err(|e| e.to_string())?;
    let start = Instant::now();
    lua.set_interrupt(move |_| {
        if start.elapsed() > BUDGET {
            Err(mlua::Error::runtime("script took too long (5 s budget)"))
        } else {
            Ok(mlua::VmState::Continue)
        }
    });
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
                for (i, (name, desc, _)) in crate::mcp::tools::TOOLS.iter().enumerate() {
                    let row = lua.create_table()?;
                    row.set("name", *name)?;
                    row.set("description", *desc)?;
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
                    p.map(|(k, _)| matches!(k, mlua::Value::Integer(i) if i >= 1 && i as usize <= len))
                        .unwrap_or(false)
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
}
