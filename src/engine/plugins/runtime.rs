//! ---- feat/plugins ----
//! Running plugin code. A plugin's scripts and panels run in a Luau VM with the same sandbox as the
//! Assistant's scripts (no io / require / load / debug, an `os` cut down to the clock, a memory cap and an
//! interrupt that enforces a wall-clock deadline), the same project API (`tool`, `tools`, `print`, `warn`,
//! `library`, `project`, `query`) and, on top, what its permissions allow:
//!
//! - `net.get(url)`: needs `network` with the URL's host granted. Non-blocking: nil while the download
//!   runs (the panel redraws when it lands), then `{ok, status, body}`.
//! - `fs.read / write / list / exists(path)`: needs `files`; the open project's folder (`project`) or a
//!   folder the user picked (`pick`). Relative paths are inside the project folder.
//! - `programs.run(exe, args)`: needs `run_programs`.
//! - `tool(name, args)`: the project / library tools every script may call, plus the app tools
//!   granted in `mcp_tools`.
//!
//! Every refusal names the plugin and the permission it is missing. The host table the API calls into is
//! rebuilt for each run (`Vm::run`), so a VM kept between panel frames never holds a stale borrow.

use super::{Permissions, Plugin};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One plugin as its code sees it.
#[derive(Clone)]
pub struct Ctx {
    pub id: String,
    pub name: String,
    pub version: String,
    pub dir: PathBuf,
    pub granted: Permissions,
    pub declared: Permissions,
    pub folders: Vec<PathBuf>,
    pub net: Arc<Net>,
}

impl Ctx {
    pub fn of(p: &Plugin) -> Ctx {
        Ctx {
            id: p.manifest.id.clone(),
            name: p.manifest.name.clone(),
            version: p.manifest.version.clone(),
            dir: p.dir.clone(),
            granted: p.granted(),
            declared: p.manifest.permissions.clone(),
            folders: p.state.folders.iter().map(PathBuf::from).collect(),
            net: Arc::new(Net::default()),
        }
    }

    fn refuse(&self, what: &str, key: &str) -> String {
        let asked = match key {
            "network" => !self.declared.network.is_empty(),
            "files" => !self.declared.files.is_empty(),
            "run_programs" => self.declared.run_programs,
            _ => !self.declared.mcp_tools.is_empty(),
        };
        if asked {
            format!("{} can't {what}: you didn't allow it when installing (Plugins pane ▸ Permissions)", self.name)
        } else {
            format!("{} can't {what}: it didn't ask for the \"{key}\" permission in plugin.json", self.name)
        }
    }

    /// May this plugin call app tool `name`? `base` = the project / library tools every script may call.
    pub fn check_tool(&self, name: &str, base: &dyn Fn(&str) -> bool) -> Result<(), String> {
        if base(name) || self.granted.mcp_tools.iter().any(|t| t == name) {
            Ok(())
        } else if crate::mcp::tools::find(name).is_none() {
            Err(format!("unknown tool '{name}'"))
        } else {
            Err(self.refuse(&format!("use the app tool '{name}'"), "mcp_tools"))
        }
    }

    /// May it download `url`? Only http(s), only granted hosts.
    pub fn check_url(&self, url: &str) -> Result<(), String> {
        let host = url_host(url).ok_or_else(|| format!("'{url}' is not an http(s) link"))?;
        if self.granted.network.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
            return Ok(());
        }
        if self.declared.network.iter().any(|h| h.eq_ignore_ascii_case(&host)) || self.declared.network.is_empty() {
            return Err(self.refuse(&format!("reach {host}"), "network"));
        }
        Err(format!("{} can't reach {host}: plugin.json only lists {}", self.name, self.declared.network.join(", ")))
    }

    /// Where `path` may be read or written: inside the project's folder (`files: project`) or a picked
    /// folder (`files: pick`). Relative paths are taken from the project folder. `..` is refused outright.
    pub fn check_path(&self, project_dir: Option<&Path>, path: &str) -> Result<PathBuf, String> {
        let g = &self.granted.files;
        if g.is_empty() {
            return Err(self.refuse("read or write files", "files"));
        }
        let p = Path::new(path);
        if p.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(format!("{}: '..' is not allowed in plugin paths", self.name));
        }
        let mut roots: Vec<PathBuf> = Vec::new();
        if g.iter().any(|f| f == "project") {
            roots.extend(project_dir.map(Path::to_path_buf));
        }
        if g.iter().any(|f| f == "pick") {
            roots.extend(self.folders.iter().cloned());
        }
        let full = if p.is_absolute() {
            p.to_path_buf()
        } else {
            roots.first().ok_or_else(|| format!("{}: no project folder yet (save the project first) and no picked folder", self.name))?.join(p)
        };
        let inside = |r: &Path| {
            let same = |a: &Path, b: &Path| {
                let n = |x: &Path| x.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
                let (a, b) = (n(a), n(b));
                a == b || a.starts_with(&format!("{b}/"))
            };
            // compare both as written and, when it exists, as resolved (a junction can't lead out)
            same(&full, r)
                && std::fs::canonicalize(&full).ok().zip(std::fs::canonicalize(r).ok()).is_none_or(|(f, r)| same(&f, &r))
        };
        if roots.iter().any(|r| inside(r)) {
            Ok(full)
        } else {
            Err(format!("{} may only use files in {}", self.name, describe_roots(g)))
        }
    }

    pub fn check_run(&self) -> Result<(), String> {
        if self.granted.run_programs {
            Ok(())
        } else {
            Err(self.refuse("start other programs", "run_programs"))
        }
    }
}

fn describe_roots(g: &[String]) -> String {
    let mut v = Vec::new();
    if g.iter().any(|f| f == "project") {
        v.push("the project's folder");
    }
    if g.iter().any(|f| f == "pick") {
        v.push("the folders you picked for it");
    }
    v.join(" and ")
}

/// The host of an http(s) URL, lower-cased (`https://API.example.com:8080/x` -> `api.example.com`).
pub fn url_host(url: &str) -> Option<String> {
    let rest = url.trim().strip_prefix("https://").or_else(|| url.trim().strip_prefix("http://"))?;
    let auth = rest.split(['/', '?', '#']).next()?;
    let host = auth.rsplit('@').next()?.split(':').next()?.to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

// ------------------------------------------------------------------ network

type Slot = Arc<Mutex<Option<Result<(u16, String), String>>>>;

/// A plugin's downloads: each URL is fetched once on a background thread, and `get` answers from here.
#[derive(Default)]
pub struct Net {
    slots: Mutex<HashMap<String, Slot>>,
    /// Called when a download finishes (the app repaints so the panel shows it).
    pub wake: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

/// The most a plugin keeps downloaded at once, and the largest body it gets.
const NET_SLOTS: usize = 64;
const NET_MAX_BODY: usize = 8 << 20;

impl Net {
    /// None while the download runs; started on the first call.
    pub fn get(&self, url: &str, fetch: fn(&str) -> Result<Vec<u8>, String>) -> Option<Result<(u16, String), String>> {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = slots.get(url) {
            return s.lock().unwrap_or_else(|e| e.into_inner()).clone();
        }
        if slots.len() >= NET_SLOTS {
            slots.retain(|_, s| s.lock().map(|g| g.is_none()).unwrap_or(false)); // keep the ones in flight
        }
        let slot: Slot = Arc::default();
        slots.insert(url.to_string(), slot.clone());
        let (url, wake) = (url.to_string(), self.wake.lock().unwrap_or_else(|e| e.into_inner()).clone());
        std::thread::spawn(move || {
            let r = fetch(&url).and_then(|b| {
                if b.len() > NET_MAX_BODY {
                    Err(format!("the reply is larger than {} MB", NET_MAX_BODY >> 20))
                } else {
                    Ok((200, String::from_utf8_lossy(&b).into_owned()))
                }
            });
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            if let Some(w) = wake {
                w();
            }
        });
        None
    }
    /// Forget every download (a panel's Reload).
    pub fn clear(&self) {
        self.slots.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

fn fetch_url(url: &str) -> Result<Vec<u8>, String> {
    super::github::get(url, false)
}

// ------------------------------------------------------------------ the VM

/// What a run reaches outside the VM.
pub struct Env<'a> {
    pub ctx: &'a Ctx,
    /// The open project's folder (None = unsaved).
    pub project_dir: Option<PathBuf>,
    /// The project / library tools every script may call (`script_ctl::script_may_call`).
    pub base_tool: &'a dyn Fn(&str) -> bool,
    pub call: RefCell<&'a mut dyn FnMut(&str, &Value) -> Result<Value, String>>,
    /// `print` / `warn` lines (capped; a `warn` line starts with `scripting::WARN`).
    pub log: RefCell<Vec<String>>,
    /// How downloads are made (tests swap it).
    pub fetch: fn(&str) -> Result<Vec<u8>, String>,
    /// Mutating tool calls made (the caller decides about undo).
    pub calls: std::cell::Cell<usize>,
}

impl<'a> Env<'a> {
    pub fn new(ctx: &'a Ctx, project_dir: Option<PathBuf>, base_tool: &'a dyn Fn(&str) -> bool, call: &'a mut dyn FnMut(&str, &Value) -> Result<Value, String>) -> Env<'a> {
        Env { ctx, project_dir, base_tool, call: RefCell::new(call), log: RefCell::new(Vec::new()), fetch: fetch_url, calls: Default::default() }
    }
}

const LOG_LINES: usize = 200;

/// The Lua side of the bridge: stable functions (`tool`, `net.get`, `ui.button`, ...) that call into a host
/// table swapped in for each run, so code holding `tool` from an earlier run still works. `editor` is the
/// hidden alias table of plugins written before `tool` / `tools` / `print` were globals (not documented,
/// not completed, not in `scripting_api::API`).
const BOOT: &str = r#"
local info = ...
local host = {}
local function ns(prefix, names)
  local t = {}
  for _, n in ipairs(names) do
    local key = prefix .. "." .. n
    t[n] = function(...)
      local f = host[key]
      if f == nil then error(key .. " is not available here (ui.* works inside a panel's draw() only)", 2) end
      return f(...)
    end
  end
  return t
end
local core = ns("core", {"tool", "tools", "print", "warn", "json"})
tool = core.tool
tools = core.tools
print = core.print
warn = core.warn
json = core.json
editor = {tool = core.tool, tools = core.tools, log = core.print, json = core.json}
net = ns("net", {"get"})
fs = ns("fs", {"read", "write", "list", "exists"})
programs = ns("programs", {"run"})
ui = ns("ui", {"label", "heading", "button", "slider", "text", "checkbox", "dropdown", "row", "column", "separator", "space", "image"})
plugin = info
return function(h) host = h end
"#;

/// A plugin's VM. Panels keep theirs between frames; a script run makes a fresh one.
pub struct Vm {
    lua: mlua::Lua,
    set_host: mlua::Function,
    deadline: Arc<Mutex<(Instant, Duration)>>,
}

impl Vm {
    pub fn new(ctx: &Ctx) -> Result<Vm, String> {
        let e = |e: mlua::Error| e.to_string();
        let lua = mlua::Lua::new();
        let g = lua.globals();
        for name in ["require", "package", "loadstring", "load", "dofile", "loadfile", "getfenv", "setfenv", "io", "debug", "collectgarbage"] {
            g.raw_set(name, mlua::Value::Nil).map_err(e)?;
        }
        if let Ok(os) = g.get::<mlua::Table>("os") {
            let safe = lua.create_table().map_err(e)?;
            for k in ["clock", "time", "date", "difftime"] {
                safe.raw_set(k, os.get::<mlua::Value>(k).map_err(e)?).map_err(e)?;
            }
            g.raw_set("os", safe).map_err(e)?;
        }
        lua.sandbox(true).map_err(e)?;
        lua.set_memory_limit(64 << 20).map_err(e)?;
        let deadline = Arc::new(Mutex::new((Instant::now() + Duration::from_secs(3600), Duration::from_secs(1))));
        let d = deadline.clone();
        lua.set_interrupt(move |_| {
            let (at, budget) = *d.lock().unwrap_or_else(|e| e.into_inner());
            if Instant::now() > at {
                Err(mlua::Error::runtime(format!("took longer than its {} ms budget", budget.as_millis())))
            } else {
                Ok(mlua::VmState::Continue)
            }
        });
        let info = lua.create_table().map_err(e)?;
        for (k, v) in [("id", &ctx.id), ("name", &ctx.name), ("version", &ctx.version)] {
            info.set(k, v.as_str()).map_err(e)?;
        }
        let set_host: mlua::Function = lua.load(BOOT).set_name("plugin").call(info).map_err(e)?;
        lua.load(crate::scripting::PRELUDE).set_name("prelude").exec().map_err(e)?;
        Ok(Vm { lua, set_host, deadline })
    }

    pub fn lua(&self) -> &mlua::Lua {
        &self.lua
    }

    /// Run `body` with the plugin API bound to `env` and a `budget` deadline. `extra` adds host functions
    /// (a panel's `ui.*`). Errors come back as one readable message.
    pub fn run<'r, 'a: 'r, R>(
        &self,
        env: &'r Env<'a>,
        budget: Duration,
        extra: impl for<'s> FnOnce(&'s mlua::Scope<'s, 'r>, &mlua::Lua, &mlua::Table) -> mlua::Result<()>,
        body: impl FnOnce(&mlua::Lua) -> mlua::Result<R>,
    ) -> Result<R, String> {
        *self.deadline.lock().unwrap_or_else(|e| e.into_inner()) = (Instant::now() + budget, budget);
        let lua = &self.lua;
        let r = lua.scope(|scope| {
            let host = lua.create_table()?;
            bind(scope, lua, &host, env)?;
            extra(scope, lua, &host)?;
            self.set_host.call::<()>(host)?;
            let r = body(lua);
            self.set_host.call::<()>(lua.create_table()?)?;
            r
        });
        *self.deadline.lock().unwrap_or_else(|e| e.into_inner()) = (Instant::now() + Duration::from_secs(3600), budget);
        r.map_err(|e| tidy(&e))
    }
}

/// A Lua error as one or a few readable lines: the message and the script's own frames, not the Rust side.
pub fn tidy(e: &mlua::Error) -> String {
    let msg = match e {
        mlua::Error::MemoryError(_) => "used more than its 64 MB of memory".to_string(),
        e => e.to_string(),
    };
    let lines: Vec<&str> = msg.lines().filter(|l| !l.contains("[C]:") && !l.trim().is_empty() && !l.contains("stack traceback")).take(6).collect();
    lines.join("\n")
}

fn bind<'s, 'r, 'a: 'r>(scope: &'s mlua::Scope<'s, 'r>, lua: &mlua::Lua, host: &mlua::Table, env: &'r Env<'a>) -> mlua::Result<()> {
    use crate::scripting::{json_to_lua, lua_to_json, show};
    let ctx: &'r Ctx = env.ctx;
    host.set(
        "core.tool",
        scope.create_function(move |lua, (name, args): (String, Option<mlua::Table>)| {
            ctx.check_tool(&name, env.base_tool).map_err(mlua::Error::runtime)?;
            let args = match args {
                Some(t) => lua_to_json(mlua::Value::Table(t))?,
                None => Value::Object(Default::default()),
            };
            env.calls.set(env.calls.get() + 1);
            let r = (env.call.borrow_mut())(&name, &args).map_err(|e| mlua::Error::runtime(format!("{name}: {e}")))?;
            json_to_lua(lua, &r)
        })?,
    )?;
    host.set(
        "core.tools",
        scope.create_function(move |lua, ()| {
            let t = lua.create_table()?;
            for def in crate::mcp::tools::all().filter(|d| ctx.check_tool(d.name, env.base_tool).is_ok()) {
                let row = lua.create_table()?;
                row.set("name", def.name)?;
                row.set("description", def.desc)?;
                t.push(row)?;
            }
            Ok(t)
        })?,
    )?;
    for (key, prefix) in [("core.print", ""), ("core.warn", crate::scripting::WARN)] {
        let log = scope.create_function(move |_, args: mlua::Variadic<mlua::Value>| {
            let mut l = env.log.borrow_mut();
            if l.len() < LOG_LINES {
                l.push(format!("{prefix}{}", args.iter().map(show).collect::<Vec<_>>().join("\t")));
            }
            Ok(())
        })?;
        host.set(key, log)?;
    }
    host.set("core.json", scope.create_function(|_, v: mlua::Value| Ok(show(&v)))?)?;
    host.set(
        "net.get",
        scope.create_function(move |lua, url: String| {
            ctx.check_url(&url).map_err(mlua::Error::runtime)?;
            match ctx.net.get(&url, env.fetch) {
                None => Ok(mlua::Value::Nil),
                Some(r) => {
                    let t = lua.create_table()?;
                    match r {
                        Ok((status, body)) => {
                            t.set("ok", true)?;
                            t.set("status", status)?;
                            t.set("body", body)?;
                        }
                        Err(e) => {
                            t.set("ok", false)?;
                            t.set("error", e)?;
                        }
                    }
                    Ok(mlua::Value::Table(t))
                }
            }
        })?,
    )?;
    let pdir = env.project_dir.as_deref();
    host.set(
        "fs.read",
        scope.create_function(move |_, path: String| {
            let p = ctx.check_path(pdir, &path).map_err(mlua::Error::runtime)?;
            let meta = std::fs::metadata(&p).map_err(|e| mlua::Error::runtime(format!("{path}: {e}")))?;
            if meta.len() > 16 << 20 {
                return Err(mlua::Error::runtime(format!("{path}: larger than 16 MB")));
            }
            std::fs::read_to_string(&p).map_err(|e| mlua::Error::runtime(format!("{path}: {e}")))
        })?,
    )?;
    host.set(
        "fs.write",
        scope.create_function(move |_, (path, text): (String, String)| {
            let p = ctx.check_path(pdir, &path).map_err(mlua::Error::runtime)?;
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d).map_err(|e| mlua::Error::runtime(format!("{path}: {e}")))?;
            }
            std::fs::write(&p, text).map_err(|e| mlua::Error::runtime(format!("{path}: {e}")))
        })?,
    )?;
    host.set(
        "fs.list",
        scope.create_function(move |lua, path: Option<String>| {
            let path = path.unwrap_or_default();
            let p = if path.is_empty() {
                pdir.map(Path::to_path_buf).or_else(|| ctx.folders.first().cloned()).ok_or_else(|| mlua::Error::runtime("no folder to list"))?
            } else {
                ctx.check_path(pdir, &path).map_err(mlua::Error::runtime)?
            };
            if path.is_empty() {
                ctx.check_path(pdir, &p.to_string_lossy()).map_err(mlua::Error::runtime)?;
            }
            let t = lua.create_table()?;
            let mut names: Vec<String> = std::fs::read_dir(&p)
                .map_err(|e| mlua::Error::runtime(format!("{}: {e}", p.display())))?
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned() + if e.path().is_dir() { "/" } else { "" })
                .collect();
            names.sort();
            for n in names.into_iter().take(10_000) {
                t.push(n)?;
            }
            Ok(t)
        })?,
    )?;
    host.set(
        "fs.exists",
        scope.create_function(move |_, path: String| Ok(ctx.check_path(pdir, &path).map_err(mlua::Error::runtime)?.exists()))?,
    )?;
    host.set(
        "programs.run",
        scope.create_function(move |_, (exe, args): (String, Option<Vec<String>>)| {
            ctx.check_run().map_err(mlua::Error::runtime)?;
            let child = std::process::Command::new(&exe)
                .args(args.unwrap_or_default())
                .stdin(std::process::Stdio::null())
                .spawn()
                .map_err(|e| mlua::Error::runtime(format!("{exe}: {e}")))?;
            Ok(child.id())
        })?,
    )?;
    let _ = lua;
    Ok(())
}

/// How long a plugin script may run (it is on the UI thread, like the user's own scripts).
pub const SCRIPT_BUDGET: Duration = Duration::from_secs(5);

/// Run a plugin script once: its whole file, with the plugin API. Ok = what it printed.
pub fn run_script(env: &Env, src: &str, name: &str) -> Result<Vec<String>, String> {
    let vm = Vm::new(env.ctx)?;
    vm.run(env, SCRIPT_BUDGET, |_, _, _| Ok(()), |lua| lua.load(src).set_name(name).exec())?;
    Ok(env.log.borrow().clone())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn ctx(granted: Permissions, declared: Permissions) -> Ctx {
        Ctx {
            id: "t".into(),
            name: "Test Plugin".into(),
            version: "1".into(),
            dir: std::env::temp_dir(),
            granted,
            declared,
            folders: Vec::new(),
            net: Arc::new(Net::default()),
        }
    }

    fn fake_fetch(url: &str) -> Result<Vec<u8>, String> {
        Ok(format!("body of {url}").into_bytes())
    }

    /// Run `src` as a plugin script against a call log; returns (result, calls made).
    fn run(c: &Ctx, src: &str) -> (Result<Vec<String>, String>, Vec<String>) {
        let mut made = Vec::new();
        let mut call = |n: &str, _: &Value| -> Result<Value, String> {
            made.push(n.to_string());
            Ok(serde_json::json!({"ok": true}))
        };
        let base = |n: &str| n == "timeline.list";
        let mut env = Env::new(c, Some(std::env::temp_dir()), &base, &mut call);
        env.fetch = fake_fetch;
        let r = run_script(&env, src, "t.luau");
        drop(env);
        (r, made)
    }

    #[test]
    fn a_script_without_network_cannot_fetch() {
        let c = ctx(Permissions::default(), Permissions::default());
        let (r, _) = run(&c, r#"net.get("https://api.example.com/x")"#);
        let e = r.unwrap_err();
        assert!(e.contains("Test Plugin can't reach api.example.com") && e.contains("\"network\""), "{e}");
        // declared but not granted: says so
        let declared = Permissions { network: vec!["api.example.com".into()], ..Default::default() };
        let c = ctx(Permissions::default(), declared.clone());
        assert!(run(&c, r#"net.get("https://api.example.com/x")"#).0.unwrap_err().contains("didn't allow"));
        // granted: allowed (for that host only)
        let c = ctx(declared.clone(), declared);
        let (r, _) = run(&c, r#"
            local r = net.get("https://api.example.com/x")
            print(r == nil and "pending" or r.body)
        "#);
        assert_eq!(r.unwrap(), vec!["pending".to_string()]);
        assert!(run(&c, r#"net.get("https://other.com/")"#).0.unwrap_err().contains("only lists api.example.com"));
    }

    #[test]
    fn tools_files_and_programs_need_their_permissions() {
        let c = ctx(Permissions::default(), Permissions::default());
        let (r, made) = run(&c, r#"tool("timeline.list", {})"#);
        assert!(r.is_ok() && made == ["timeline.list"], "project tools need no permission");
        let (r, made) = run(&c, r#"tool("export.video", {})"#);
        assert!(r.unwrap_err().contains("mcp_tools") && made.is_empty());
        assert!(run(&c, r#"fs.read("notes.txt")"#).0.unwrap_err().contains("\"files\""));
        assert!(run(&c, r#"programs.run("notepad.exe", {})"#).0.unwrap_err().contains("run_programs"));
        assert!(run(&c, r#"ui.button("x")"#).0.unwrap_err().contains("panel's draw()"));
        // the sandbox: no io / require / load
        for code in ["io.open('x')", "require('x')", "load('return 1')", "os.execute('x')", "debug.getinfo(1)"] {
            assert!(run(&c, code).0.is_err(), "{code} must fail");
        }
        let tools = Permissions { mcp_tools: vec!["export.video".into()], ..Default::default() };
        let c = ctx(tools.clone(), tools);
        let (r, made) = run(&c, r#"tool("export.video", {})"#);
        assert!(r.is_ok() && made == ["export.video"]);
    }

    #[test]
    fn file_access_stays_in_the_granted_folders() {
        let base = std::env::temp_dir().join(format!("se-plugin-fs-{}", std::process::id()));
        let (proj, picked, other) = (base.join("proj"), base.join("picked"), base.join("other"));
        for d in [&proj, &picked, &other] {
            std::fs::create_dir_all(d).unwrap();
        }
        let both = Permissions { files: vec!["project".into(), "pick".into()], ..Default::default() };
        let mut c = ctx(both.clone(), both);
        c.folders = vec![picked.clone()];
        assert_eq!(c.check_path(Some(&proj), "a/b.txt").unwrap(), proj.join("a/b.txt"));
        assert!(c.check_path(Some(&proj), &picked.join("x.txt").to_string_lossy()).is_ok());
        assert!(c.check_path(Some(&proj), &other.join("x.txt").to_string_lossy()).is_err());
        assert!(c.check_path(Some(&proj), "../other/x.txt").is_err());
        let only_project = Permissions { files: vec!["project".into()], ..Default::default() };
        let c2 = Ctx { granted: only_project.clone(), declared: only_project, ..c.clone() };
        assert!(c2.check_path(Some(&proj), &picked.join("x.txt").to_string_lossy()).is_err(), "picked folders need 'pick'");
        // a real round trip through the API
        let mut call = |_: &str, _: &Value| -> Result<Value, String> { Ok(Value::Null) };
        let base_tool = |_: &str| false;
        let env = Env::new(&c, Some(proj.clone()), &base_tool, &mut call);
        let out = run_script(&env, r#"fs.write("out/n.txt", "hi") print(fs.read("out/n.txt"), fs.exists("nope.txt"))"#, "t").unwrap();
        assert_eq!(out, vec!["hi\tfalse".to_string()]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_runaway_script_is_stopped_by_its_budget() {
        let c = ctx(Permissions::default(), Permissions::default());
        let vm = Vm::new(&c).unwrap();
        let mut call = |_: &str, _: &Value| -> Result<Value, String> { Ok(Value::Null) };
        let base = |_: &str| false;
        let env = Env::new(&c, None, &base, &mut call);
        let t = Instant::now();
        let e = vm.run(&env, Duration::from_millis(20), |_, _, _| Ok(()), |lua| lua.load("while true do end").exec()).unwrap_err();
        assert!(e.contains("budget"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(2));
        // the VM is still usable afterwards, with a fresh deadline
        assert!(vm.run(&env, Duration::from_millis(200), |_, _, _| Ok(()), |lua| lua.load("local x = 1").exec()).is_ok());
    }

    #[test]
    fn url_hosts_are_read_safely() {
        assert_eq!(url_host("https://API.example.com:8080/x?y").as_deref(), Some("api.example.com"));
        assert_eq!(url_host("https://user@evil.com@good.com/").as_deref(), Some("good.com"));
        assert_eq!(url_host("ftp://x.com"), None);
        assert_eq!(url_host("file:///C:/x"), None);
    }
}
