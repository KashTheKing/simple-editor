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
/// Default wall-clock budget for a `-- @on` hook fired by `App::fire_hook` - much tighter than a
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
/// to 250 ms - see `ScriptMeta::budget`/`DEFAULT_HOOK_BUDGET`). Building the `editor` table itself stays
/// separate in each caller - it borrows that call's own `call`/`logs` (and, for a hook, `event`), which
/// `Lua::scope`'s lifetime ties to the closure that builds it - so this covers exactly the part that
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
/// caller - it supplies the re-entrancy guard and per-session disable-on-overrun policy; this fn just
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

// ---- ws:assistant-luau ----
// The Assistant's script runner (`scripts.eval`): the same `editor` table plus a project / library API
// (`library.*`, `project.*`, `query.*`, the prelude below), under a sandbox an agent cannot leave: no `io`,
// no `require` / `package` (mlua's Luau `require` reads files), no `loadstring` / `getfenv` / `setfenv`,
// an `os` cut down to the clock; `editor.tool` goes through the host's allow list (`script_ctl`). It is
// bounded by wall-clock time, interpreter steps and memory, and its `print` / `return` come back capped.

/// What one `run_api` may use.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub wall: Duration,
    /// Interpreter steps (Luau's interrupt fires on calls and loop back-edges).
    pub steps: u64,
    /// Bytes the VM may allocate.
    pub memory: usize,
    /// Characters of `print` output + the returned value handed back.
    pub output: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { wall: Duration::from_secs(10), steps: 20_000_000, memory: 256 << 20, output: 16_000 }
    }
}

/// What a `run_api` produced. `error` = it failed (the host rolls its edits back).
#[derive(Debug, Default)]
pub struct ApiRun {
    pub output: String,
    pub result: Option<Value>,
    pub truncated: bool,
    pub error: Option<String>,
    pub steps: u64,
    pub calls: usize,
    pub ms: u64,
}

pub const TRUNCATED: &str = "\n[output truncated: refine your query (filter, fields, limit) or print less]";

/// The Luau half of the API, loaded before the script. Everything goes through `editor.tool`, so the
/// host's allow list and the tools' own validation cover it.
const PRELUDE: &str = r#"
local tool = editor.tool
local function merge(a, b)
  local t = {}
  for k, v in pairs(a or {}) do t[k] = v end
  for k, v in pairs(b or {}) do t[k] = v end
  return t
end

library = {}
-- every asset matching filter {kind, folder, nested, used, query, tags, label, ext, untagged, ids}, all fields
function library.assets(filter)
  local f = merge(filter, {})
  if f.fields == nil then f.fields = "all" end
  f.limit = 5000
  local out, offset = {}, 0
  while true do
    f.offset = offset
    local r = tool("media.list", f)
    for _, a in ipairs(r.assets) do table.insert(out, a) end
    if r.next_offset == nil then break end
    offset = r.next_offset
  end
  return out
end
function library.get(id) return library.assets({ids = {id}})[1] end
function library.summary() return tool("media.summary", {}) end
-- ids: one id or a list; fields: {folder, tags, add_tags, remove_tags, label, description}
function library.set(ids, fields)
  if type(ids) == "number" then ids = {ids} end
  if #ids == 0 then return {ok = true, matched = 0, changed = 0} end
  return tool("media.bulk_set", merge(fields, {ids = ids}))
end
function library.update(filter, fields) return tool("media.bulk_set", merge(fields, {filter = filter or {}})) end
function library.move(ids, bin) return library.set(ids, {folder = bin}) end
function library.tag(ids, tags) return library.set(ids, {add_tags = tags}) end
function library.bins() return tool("library.bins", {}).bins end
function library.create_bin(bin) return tool("library.bins", {action = "create", bin = bin}) end
function library.rename_bin(bin, name) return tool("library.bins", {action = "rename", bin = bin, to = name}) end
function library.move_bin(bin, parent) return tool("library.bins", {action = "move", bin = bin, to = parent or ""}) end
function library.delete_bin(bin) return tool("library.bins", {action = "delete", bin = bin}) end
function library.linked() return tool("media.linked", {}).linked end
function library.linked_files(bin, sub) return tool("media.linked", {bin = bin, sub = sub or ""}) end
function library.smart_bins() return tool("media.smart_bin", {op = "list"}) end

project = {}
function project.get() return tool("project.get", {}) end
function project.summary() return tool("project.summary", {}) end
function project.timeline() return tool("timeline.list", {}) end
function project.tracks() return tool("track.list", {}) end
-- every clip of the timeline being edited, with its track's index and name
function project.clips()
  local out = {}
  for i, t in ipairs(tool("timeline.list", {}).tracks) do
    for _, c in ipairs(t.clips) do
      c.track = i - 1
      c.track_name = t.name
      table.insert(out, c)
    end
  end
  return out
end
function project.markers() return tool("markers.list", {}) end
function project.subtitles() return tool("subtitles.get", {}) end
function project.sequences() return tool("sequence.list", {}) end
function project.spans() return tool("animate.spans", {}) end
function project.blocks(clip_id) return tool("keyblocks.list", {clip_id = clip_id}) end

query = {}
local function getter(key)
  if type(key) == "function" then return key end
  return function(x) return x[key] end
end
local function pred(p)
  if type(p) == "function" then return p end
  return function(x)
    for k, v in pairs(p) do
      if type(v) == "function" then
        if not v(x[k]) then return false end
      elseif x[k] ~= v then
        return false
      end
    end
    return true
  end
end
function query.filter(list, p)
  local f, out = pred(p), {}
  for _, x in ipairs(list) do if f(x) then table.insert(out, x) end end
  return out
end
function query.find(list, p)
  local f = pred(p)
  for _, x in ipairs(list) do if f(x) then return x end end
  return nil
end
function query.map(list, fn)
  local out = {}
  for i, x in ipairs(list) do out[i] = fn(x) end
  return out
end
function query.pluck(list, key) return query.map(list, getter(key)) end
function query.ids(list) return query.pluck(list, "id") end
function query.sort(list, key, desc)
  local g, out = getter(key or "id"), table.clone(list)
  table.sort(out, function(a, b)
    local x, y = g(a), g(b)
    if x == nil then return false end
    if y == nil then return true end
    if type(x) == "string" then x, y = x:lower(), tostring(y):lower() end
    if desc then return x > y end
    return x < y
  end)
  return out
end
function query.group_by(list, key)
  local g, out = getter(key), {}
  for _, x in ipairs(list) do
    local k = g(x)
    if k == nil then k = "" end
    out[k] = out[k] or {}
    table.insert(out[k], x)
  end
  return out
end
function query.count_by(list, key)
  local g, out = getter(key), {}
  for _, x in ipairs(list) do
    local k = g(x)
    if k == nil then k = "" end
    out[k] = (out[k] or 0) + 1
  end
  return out
end
function query.sum(list, key)
  local g, s = getter(key), 0
  for _, x in ipairs(list) do s = s + (g(x) or 0) end
  return s
end
function query.keys(t)
  local out = {}
  for k in pairs(t) do table.insert(out, k) end
  table.sort(out, function(a, b) return tostring(a) < tostring(b) end)
  return out
end
-- s contains any of words (case-insensitive, plain text)
function query.has(s, words)
  s = string.lower(tostring(s or ""))
  if type(words) == "string" then words = {words} end
  for _, w in ipairs(words) do
    if string.find(s, string.lower(w), 1, true) then return true end
  end
  return false
end
"#;

/// `print` / `editor.log` / the returned value, kept under `cap` characters.
struct Out {
    text: String,
    cap: usize,
    truncated: bool,
}

impl Out {
    fn push(&mut self, s: &str) {
        if self.truncated {
            return;
        }
        let room = self.cap.saturating_sub(self.text.len());
        if s.len() <= room {
            self.text.push_str(s);
        } else {
            let mut cut = room;
            while !s.is_char_boundary(cut) {
                cut -= 1;
            }
            self.text.push_str(&s[..cut]);
            self.truncated = true;
        }
    }
}

/// How `print` shows a value: tables as compact JSON, everything else as `tostring` would.
fn show(v: &mlua::Value) -> String {
    match v {
        mlua::Value::Nil => "nil".into(),
        mlua::Value::Boolean(b) => b.to_string(),
        mlua::Value::Integer(i) => i.to_string(),
        mlua::Value::Number(n) => n.to_string(),
        mlua::Value::String(s) => s.to_string_lossy().to_string(),
        mlua::Value::Table(_) => match lua_to_json(v.clone()) {
            Ok(j) => j.to_string(),
            Err(e) => format!("<table: {e}>"),
        },
        other => format!("<{}>", other.type_name()),
    }
}

/// A VM for an agent's script: the sandbox plus `limits`' step / time / memory budget. `steps` counts
/// the interrupts (it is read back for the run's stats).
fn setup_api_vm(limits: &Limits, steps: std::sync::Arc<std::sync::atomic::AtomicU64>) -> Result<mlua::Lua, String> {
    use std::sync::atomic::Ordering;
    let lua = mlua::Lua::new();
    let g = lua.globals();
    let e = |e: mlua::Error| e.to_string();
    // mlua's Luau `require` loads files from disk; `loadstring` / fenv would only reach this same sandbox,
    // but nothing an agent needs either
    for name in ["require", "package", "loadstring", "load", "dofile", "loadfile", "getfenv", "setfenv", "io", "debug"] {
        g.raw_set(name, mlua::Value::Nil).map_err(e)?;
    }
    // `os` keeps the clock only (Luau's has no file / process calls, but say it in code)
    if let Ok(os) = g.get::<mlua::Table>("os") {
        let safe = lua.create_table().map_err(e)?;
        for k in ["clock", "time", "date", "difftime"] {
            safe.raw_set(k, os.get::<mlua::Value>(k).map_err(e)?).map_err(e)?;
        }
        g.raw_set("os", safe).map_err(e)?;
    }
    lua.sandbox(true).map_err(e)?;
    lua.set_memory_limit(limits.memory).map_err(e)?;
    let (start, wall, max) = (Instant::now(), limits.wall, limits.steps);
    lua.set_interrupt(move |_| {
        let n = steps.fetch_add(1, Ordering::Relaxed) + 1;
        if n > max {
            return Err(mlua::Error::runtime(format!(
                "script ran out of its instruction budget ({max} steps) - a runaway loop, or do less per run"
            )));
        }
        if start.elapsed() > wall {
            return Err(mlua::Error::runtime(format!("script took too long ({} ms budget)", wall.as_millis())));
        }
        Ok(mlua::VmState::Continue)
    });
    Ok(lua)
}

/// Run an agent's script: `editor.tool` / `editor.tools` (only the tools `allowed` admits) /
/// `editor.log` / `editor.purpose`, `print`, the prelude's `library` / `project` / `query`, and the
/// chunk's return value. Never panics; every failure is `ApiRun::error` (with the output so far).
pub fn run_api(
    src: &str,
    chunk_name: &str,
    purpose: &str,
    limits: &Limits,
    allowed: &dyn Fn(&str) -> bool,
    call: &mut dyn FnMut(&str, &Value) -> Result<Value, String>,
) -> ApiRun {
    use std::sync::atomic::{AtomicU64, Ordering};
    let t0 = Instant::now();
    let steps = std::sync::Arc::new(AtomicU64::new(0));
    let mut run = ApiRun::default();
    let out = std::cell::RefCell::new(Out { text: String::new(), cap: limits.output, truncated: false });
    let calls = std::cell::Cell::new(0usize);
    let lua = match setup_api_vm(limits, steps.clone()) {
        Ok(l) => l,
        Err(e) => {
            run.error = Some(e);
            return run;
        }
    };
    let call = std::cell::RefCell::new(call);
    let result = lua.scope(|scope| {
        let editor = lua.create_table()?;
        editor.set(
            "tool",
            scope.create_function(|lua, (name, args): (String, Option<mlua::Table>)| {
                if !allowed(&name) {
                    return Err(mlua::Error::runtime(format!(
                        "'{name}' is not available to scripts (they reach the project and library only: no \
                         files, exports, settings or network)"
                    )));
                }
                let args = match args {
                    Some(t) => lua_to_json(mlua::Value::Table(t))?,
                    None => Value::Object(Default::default()),
                };
                calls.set(calls.get() + 1);
                let r = (call.borrow_mut())(&name, &args).map_err(|e| mlua::Error::runtime(format!("{name}: {e}")))?;
                json_to_lua(lua, &r)
            })?,
        )?;
        editor.set(
            "tools",
            scope.create_function(|lua, ()| {
                let t = lua.create_table()?;
                for def in crate::mcp::tools::all().filter(|d| allowed(d.name)) {
                    let row = lua.create_table()?;
                    row.set("name", def.name)?;
                    row.set("description", def.desc)?;
                    row.set("args", def.args.join("; "))?;
                    t.push(row)?;
                }
                Ok(t)
            })?,
        )?;
        let print = scope.create_function(|_, args: mlua::Variadic<mlua::Value>| {
            let line: Vec<String> = args.iter().map(show).collect();
            let mut o = out.borrow_mut();
            o.push(&line.join("\t"));
            o.push("\n");
            Ok(())
        })?;
        editor.set("log", print.clone())?;
        editor.set("json", scope.create_function(|_, v: mlua::Value| Ok(show(&v)))?)?;
        editor.set("purpose", purpose)?;
        lua.globals().set("editor", editor)?;
        lua.globals().set("print", print)?;
        lua.load(PRELUDE).set_name("prelude").exec()?;
        let ret: mlua::MultiValue = lua.load(src).set_name(chunk_name).eval()?;
        match ret.into_iter().next() {
            None | Some(mlua::Value::Nil) => Ok(None),
            Some(v) => lua_to_json(v).map(Some),
        }
    });
    let mut out = out.into_inner();
    match result {
        Ok(v) => {
            if let Some(v) = v {
                // the result shares the output budget; a huge one comes back as the start of its JSON
                let text = v.to_string();
                if out.text.len() + text.len() <= out.cap {
                    run.result = Some(v);
                } else {
                    let mut o = Out { text: String::new(), cap: out.cap.saturating_sub(out.text.len()), truncated: false };
                    o.push(&text);
                    run.result = Some(Value::String(o.text));
                    out.truncated = true;
                }
            }
        }
        Err(e) => {
            let msg = match &e {
                mlua::Error::MemoryError(_) => format!("script used more than its {} MB of memory", limits.memory >> 20),
                e => e.to_string(),
            };
            // the Rust side of a callback error is noise to a script author: keep the message and the
            // script's own frames
            let msg: Vec<&str> = msg.lines().filter(|l| !l.contains("[C]:") && !l.trim().is_empty()).take(12).collect();
            run.error = Some(msg.join("\n"));
        }
    }
    run.truncated = out.truncated;
    run.output = out.text;
    run.steps = steps.load(Ordering::Relaxed);
    run.calls = calls.get();
    run.ms = t0.elapsed().as_millis() as u64;
    run
}

/// A small, fixed set of icon keywords a script header's `@icon` may name - matches a subset of
/// `ui::tools::Glyph::name()` strings (chosen without depending on `ui::tools` from this low-level
/// module: menus.rs resolves the name back into a `Glyph` at draw time). An unknown name comes back
/// `None` rather than an error - a stale `@icon` in a script file must never break metadata parsing.
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
/// parsed without starting the VM - cheap enough for the Scripts menu / palette / `scripts.list` tool to
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
/// A table nested deeper than 100 (a cycle, usually) is an error, not a stack overflow.
fn lua_to_json(v: mlua::Value) -> mlua::Result<Value> {
    lua_to_json_at(v, 0)
}

fn lua_to_json_at(v: mlua::Value, depth: usize) -> mlua::Result<Value> {
    if depth > 100 {
        return Err(mlua::Error::runtime("table nested too deeply (does it contain itself?)"));
    }
    let lua_to_json = |v| lua_to_json_at(v, depth + 1);
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
        // Luau stores integral doubles as integers: 2.0 may arrive as 2 - same value either way
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

    // ---- ws:assistant-luau ----

    fn api(src: &str, limits: Limits) -> (ApiRun, Vec<String>) {
        let mut called = Vec::new();
        let run = {
            let mut call = |name: &str, args: &Value| -> Result<Value, String> {
                called.push(name.to_string());
                Ok(match name {
                    "media.list" => json!({"total": 2, "next_offset": null, "assets": [
                        {"id": 1, "name": "a.wav", "kind": "audio"}, {"id": 2, "name": "b.mp4", "kind": "video"}]}),
                    _ => json!({"ok": true, "echo": args}),
                })
            };
            let allowed = |n: &str| !n.starts_with("export.");
            run_api(src, "t", "test", &limits, &allowed, &mut call)
        };
        (run, called)
    }

    #[test]
    fn api_sandbox_has_no_files_processes_or_require() {
        let (r, _) = api(
            r#"
            assert(io == nil, "io")
            assert(require == nil and package == nil, "require")
            assert(loadstring == nil and getfenv == nil and setfenv == nil and dofile == nil, "loaders")
            assert(os.execute == nil and os.remove == nil and os.exit == nil and os.getenv == nil, "os")
            assert(type(os.clock()) == "number")
            local ok, err = pcall(function() return editor.tool("export.video", {path = "C:/x.mp4"}) end)
            assert(not ok and string.find(tostring(err), "not available to scripts"), tostring(err))
            for _, t in ipairs(editor.tools()) do assert(string.sub(t.name, 1, 7) ~= "export.") end
            "#,
            Limits::default(),
        );
        assert_eq!(r.error, None);
        // a global the sandbox froze can't be replaced to sneak something back in
        let (r, _) = api("string.rep = nil", Limits::default());
        assert!(r.error.is_some());
    }

    #[test]
    fn api_budgets_stop_runaway_scripts() {
        let t = Instant::now();
        let (r, _) = api("while true do end", Limits { steps: 200_000, ..Limits::default() });
        assert!(r.error.as_deref().unwrap_or("").contains("instruction budget"), "{:?}", r.error);
        assert!(t.elapsed() < Duration::from_secs(5));
        let (r, _) = api("local n = 0 while true do n = n + 1 end", Limits { wall: Duration::from_millis(200), steps: u64::MAX, ..Limits::default() });
        assert!(r.error.as_deref().unwrap_or("").contains("too long"), "{:?}", r.error);
        let (r, _) = api(
            r#"local t = {} for i = 1, 1e7 do t[i] = string.rep("x", 1000) .. i end"#,
            Limits { memory: 16 << 20, ..Limits::default() },
        );
        assert!(r.error.as_deref().unwrap_or("").contains("memory"), "{:?}", r.error);
        // a table that contains itself is an error, not a crash
        let (r, _) = api("local t = {} t.me = t print(t)", Limits::default());
        assert!(r.output.contains("nested too deeply"), "{}", r.output);
    }

    #[test]
    fn api_output_is_capped_and_results_come_back() {
        let (r, _) = api("for i = 1, 10000 do print('line', i) end", Limits { output: 1_000, ..Limits::default() });
        assert_eq!(r.error, None);
        assert!(r.truncated && r.output.len() <= 1_000);
        let (r, called) = api(
            r#"
            local a = library.assets({kind = "audio"})
            print(#a, a[1])
            local g = query.group_by(a, "kind")
            assert(#g.audio == 1 and #g.video == 1)
            assert(query.has("Big Whoosh.wav", {"whoosh", "hit"}))
            assert(query.sort(a, "name", true)[1].name == "b.mp4")
            library.move({1, 2}, "Audio/SFX")
            return {count = #a, ids = query.ids(a)}
            "#,
            Limits::default(),
        );
        assert_eq!(r.error, None, "{}", r.output);
        assert!(r.output.starts_with("2\t{"), "{}", r.output);
        assert_eq!(r.result, Some(json!({"count": 2, "ids": [1, 2]})));
        assert_eq!(called, vec!["media.list", "media.bulk_set"]);
        assert_eq!(r.calls, 2);
        // a big result shares the cap and is cut, flagged
        let (r, _) = api("return string.rep('y', 5000)", Limits { output: 1_000, ..Limits::default() });
        assert!(r.truncated);
        assert!(r.result.unwrap().as_str().unwrap().len() <= 1_000);
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
