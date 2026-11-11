//! ---- ws:assistant-pane ----
//! One Assistant-pane request on a worker thread, streamed back to the UI as `Event`s over a channel.
//! Backends that run a child process live here (the "external" one just queues for `assistant.wait`, in
//! `ui/app/assistant_ctl.rs`):
//! - "claude": `claude -p --output-format stream-json --verbose --mcp-config <tmp> --strict-mcp-config
//!   --allowedTools mcp__simple-editor --tools "" [--model M] [--resume S]`, the prompt on stdin (so quotes and
//!   newlines need no escaping, and `claude.cmd` works through std's batch-file handling), one JSON
//!   object per stdout line (`parse_line`). A failed `--resume` is retried once without it.
//! - "codex": `codex exec --skip-git-repo-check -c mcp_servers.simple-editor.url="<url>" [-m M] <prompt>`
//!   (`codex_args`); stdout lines are shown as they come.
//! - "custom": a user template with `{prompt}` / `{mcp_url}` / `{model}`, split into argv without a
//!   shell (`custom_argv`); stdout lines are shown as they come.
//!
//! The API key (optional) lives in Windows Credential Manager (generic credential
//! "SimpleEditor/assistant", `key_load` / `key_save` / `key_delete`) and only ever reaches the child's
//! environment - never settings.json, never the log.

use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// What a run reports, in order; `Done` is always last.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The agent's session id (claude: from the init line) - remembered so follow-ups resume it.
    Session(String),
    /// Something the agent said.
    Text(String),
    /// A tool call: "name {short args}" (the `mcp__simple-editor__` prefix stripped).
    Tool(String),
    /// A plain stdout line (custom backend).
    Line(String),
    /// A note from the runner itself (e.g. the resume retry).
    Info(String),
    Error(String),
    Done { ok: bool, session: Option<String> },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Backend {
    Claude,
    Codex,
    Custom,
}

/// Everything one run needs (built on the UI thread).
pub struct Spec {
    pub backend: Backend,
    pub prompt: String,
    pub mcp_url: String,
    /// claude: `--model`, codex: `-m`, custom: `{model}`; empty = the CLI's default.
    pub model: String,
    /// claude: the session to `--resume`.
    pub resume: Option<String>,
    /// custom: the command template.
    pub command: String,
    /// (env var name, value) for the API key, when one is stored.
    pub key: Option<(String, String)>,
}

/// A running request. Dropping it (or `stop`) kills the child.
pub struct Run {
    pub rx: Receiver<Event>,
    child: Arc<Mutex<Option<Child>>>,
    /// Set by `stop`: the worker must not start another child (the `--resume` retry).
    stopped: Arc<AtomicBool>,
}

impl Run {
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Ok(mut c) = self.child.lock() {
            if let Some(c) = c.as_mut() {
                // a `.cmd` shim's real agent (node) is a grandchild: kill the whole tree, not just cmd.exe
                let _ = crate::media::ffpipe::command(Path::new("taskkill"))
                    .args(["/PID", c.id().to_string().as_str(), "/T", "/F"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                let _ = c.kill();
            }
        }
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn start(spec: Spec) -> Run {
    let (tx, rx) = mpsc::channel();
    let child = Arc::new(Mutex::new(None));
    let slot = child.clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop2 = stopped.clone();
    // a failed spawn drops the only sender: the UI sees a disconnected channel and ends the run
    let _ = std::thread::Builder::new().name("assistant-agent".into()).spawn(move || {
        let ok = match spec.backend {
            Backend::Claude => run_claude(&spec, &tx, &slot, &stop2),
            Backend::Codex => {
                let args = codex_args(&spec.prompt, &spec.mcp_url, &spec.model);
                run_lines("codex", &args, &spec, &tx, &slot)
            }
            Backend::Custom => {
                let argv = custom_argv(&spec.command.replace("{model}", spec.model.trim()), &spec.prompt, &spec.mcp_url);
                match argv.split_first().filter(|(first, _)| !first.is_empty()) {
                    Some((first, rest)) => run_lines(first, rest, &spec, &tx, &slot),
                    None => {
                        let _ = tx.send(Event::Error("No custom command set (Settings ▸ Assistant).".into()));
                        Some(false)
                    }
                }
            }
        };
        // run_* send their own Done when they reach one; this covers every early return
        if let Some(ok) = ok {
            let _ = tx.send(Event::Done { ok, session: None });
        }
    });
    Run { rx, child, stopped }
}

// ---------------------------------------------------------------------------------------------------
// claude

pub const NOT_FOUND: &str = "The agent's command line (`claude`) wasn't found. Install it (`npm install -g \
     @anthropic-ai/claude-code`, or the native installer), run `claude` once in a terminal to sign in, then try \
     again - or pick another agent in the Assistant's header.";

/// The flag that picks `model` for an agent CLI ("claude": `--model`, "codex": `-m`); nothing when the
/// model is empty (the CLI's own default) or the agent takes none.
pub fn model_flag(backend: &str, model: &str) -> Vec<String> {
    let m = model.trim();
    match backend {
        _ if m.is_empty() => Vec::new(),
        "claude" => vec!["--model".into(), m.into()],
        "codex" => vec!["-m".into(), m.into()],
        _ => Vec::new(),
    }
}

/// `codex exec`'s argv: this app's MCP server configured for the run, the model, the prompt.
pub fn codex_args(prompt: &str, mcp_url: &str, model: &str) -> Vec<String> {
    let mut a: Vec<String> = ["exec", "--skip-git-repo-check", "-c"].iter().map(|s| s.to_string()).collect();
    a.push(format!("mcp_servers.simple-editor.url=\"{mcp_url}\""));
    a.extend(model_flag("codex", model));
    a.push(prompt.to_string());
    a
}

/// The temp MCP config pointing the `claude` CLI at this app's server.
pub fn mcp_config_json(mcp_url: &str) -> String {
    serde_json::json!({"mcpServers": {"simple-editor": {"type": "http", "url": mcp_url}}}).to_string()
}

/// `claude`'s argv (the prompt goes on stdin).
pub fn claude_args(mcp_config: &str, model: &str, resume: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> = [
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--mcp-config",
        mcp_config,
        "--strict-mcp-config",
        "--allowedTools",
        "mcp__simple-editor",
        // no built-in tools at all (not even Read/Grep/Glob): only the editor's MCP tools. An empty
        // argument survives a `claude.cmd` shim too - std quotes it as "" for batch files.
        "--tools",
        "",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.extend(model_flag("claude", model));
    if let Some(s) = resume.filter(|s| !s.is_empty()) {
        a.extend(["--resume".to_string(), s.to_string()]);
    }
    a
}

/// `name` on PATH as given, `.exe` or `.cmd` (npm installs `claude.cmd` / `codex.cmd`); an absolute or
/// relative path that exists is returned as is.
pub fn which(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.components().count() > 1 {
        return p.is_file().then(|| p.to_path_buf());
    }
    let has_ext = p.extension().is_some();
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for ext in ["exe", "cmd", "bat"] {
            let c = if has_ext { dir.join(name) } else { dir.join(format!("{name}.{ext}")) };
            if c.is_file() {
                return Some(c);
            }
        }
    }
    None
}

pub fn claude_exe() -> Option<PathBuf> {
    which("claude").or_else(|| {
        let p = PathBuf::from(std::env::var_os("USERPROFILE")?).join(".local").join("bin").join("claude.exe");
        p.is_file().then_some(p)
    })
}

/// Events from one stream-json line. Unknown or malformed lines give nothing - never an error.
pub fn parse_line(line: &str) -> Vec<Event> {
    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return Vec::new() };
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    match v.get("type").and_then(Value::as_str) {
        Some("system") => s(&v, "session_id").map(Event::Session).into_iter().collect(),
        Some("assistant") => blocks(&v)
            .filter_map(|b| match b.get("type").and_then(Value::as_str) {
                Some("text") => s(b, "text").filter(|t| !t.trim().is_empty()).map(|t| Event::Text(t.trim().to_string())),
                Some("tool_use") => {
                    let name = s(b, "name").unwrap_or_default();
                    let name = name.strip_prefix("mcp__simple-editor__").unwrap_or(&name).to_string();
                    let args = b.get("input").map(Value::to_string).unwrap_or_default();
                    let args = if args == "{}" { String::new() } else { format!(" {}", clip(&args, 80)) };
                    Some(Event::Tool(format!("{name}{args}")))
                }
                _ => None,
            })
            .collect(),
        // a tool that failed: its result comes back as a user message
        Some("user") => blocks(&v)
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            .filter(|b| b.get("is_error").and_then(Value::as_bool) == Some(true))
            .map(|b| {
                let text = match b.get("content") {
                    Some(Value::String(t)) => t.clone(),
                    Some(Value::Array(a)) => a.iter().filter_map(|c| s(c, "text")).collect::<Vec<_>>().join(" "),
                    _ => "tool failed".into(),
                };
                Event::Error(clip(&text, 200))
            })
            .collect(),
        Some("result") => {
            let ok = v.get("is_error").and_then(Value::as_bool) != Some(true);
            let mut out = Vec::new();
            if !ok {
                out.push(Event::Error(s(&v, "result").unwrap_or_else(|| "the agent failed".into())));
            }
            out.push(Event::Done { ok, session: s(&v, "session_id") });
            out
        }
        _ => Vec::new(),
    }
}

fn blocks(v: &Value) -> impl Iterator<Item = &Value> {
    v.pointer("/message/content").and_then(Value::as_array).into_iter().flatten()
}

fn clip(s: &str, n: usize) -> String {
    let s = s.replace(['\n', '\r'], " ");
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    }
}

/// Some(ok) = the caller still has to send `Done`; None = Done was sent.
fn run_claude(spec: &Spec, tx: &Sender<Event>, slot: &Mutex<Option<Child>>, stopped: &AtomicBool) -> Option<bool> {
    let Some(exe) = claude_exe() else {
        let _ = tx.send(Event::Error(NOT_FOUND.into()));
        return Some(false);
    };
    let cfg = std::env::temp_dir().join(format!("se-assistant-mcp-{}.json", std::process::id()));
    if let Err(e) = std::fs::write(&cfg, mcp_config_json(&spec.mcp_url)) {
        let _ = tx.send(Event::Error(format!("couldn't write the MCP config: {e}")));
        return Some(false);
    }
    let cfg_s = cfg.to_string_lossy().into_owned();
    let mut resume = spec.resume.clone();
    loop {
        let args = claude_args(&cfg_s, &spec.model, resume.as_deref());
        let resuming = resume.is_some();
        let mut produced = false;
        let mut done: Option<Event> = None;
        // errors before any output are held while resuming: they may just be "no such session"
        let mut held: Vec<Event> = Vec::new();
        let r = run_child(&exe, &args, Some(&spec.prompt), spec.key.as_ref(), slot, &mut |line: &str| {
            for e in parse_line(line) {
                match e {
                    Event::Done { .. } => done = Some(e),
                    Event::Error(_) if resuming && !produced => held.push(e),
                    e => {
                        produced |= matches!(e, Event::Text(_) | Event::Tool(_));
                        let _ = tx.send(e);
                    }
                }
            }
        });
        let ok = matches!(done, Some(Event::Done { ok: true, .. }));
        // a killed run (Stop) also ends with no result line: never respawn it
        if !ok && resuming && !produced && !stopped.load(Ordering::Relaxed) {
            let _ = tx.send(Event::Info("Couldn't resume the last conversation - starting a new one.".into()));
            resume = None;
            continue;
        }
        let _ = std::fs::remove_file(&cfg);
        for e in held {
            let _ = tx.send(e);
        }
        if let Err(e) = r {
            let _ = tx.send(Event::Error(e));
        }
        return match done {
            Some(d) => {
                let _ = tx.send(d);
                None
            }
            None => Some(false), // no result line: crashed or killed
        };
    }
}

// ---------------------------------------------------------------------------------------------------
// custom

/// Split `template` into argv like a shell would for plain words and "double quotes" (no escapes, no
/// expansion), then substitute `{prompt}` / `{mcp_url}` inside each word - so a prompt with spaces or
/// quotes stays one argument.
pub fn custom_argv(template: &str, prompt: &str, mcp_url: &str) -> Vec<String> {
    let mut words = Vec::new();
    let (mut cur, mut quoted, mut any) = (String::new(), false, false);
    for ch in template.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        words.push(cur);
    }
    words.into_iter().map(|w| w.replace("{mcp_url}", mcp_url).replace("{prompt}", prompt)).collect()
}

/// Run `program args` and show its stdout lines as they come (codex, custom).
fn run_lines(program: &str, args: &[String], spec: &Spec, tx: &Sender<Event>, slot: &Mutex<Option<Child>>) -> Option<bool> {
    let Some(exe) = which(program) else {
        let _ = tx.send(Event::Error(format!("'{program}' not found on PATH")));
        return Some(false);
    };
    let r = run_child(&exe, args, None, spec.key.as_ref(), slot, &mut |line: &str| {
        if !line.trim().is_empty() {
            let _ = tx.send(Event::Line(line.trim_end().to_string()));
        }
    });
    match r {
        Ok(()) => Some(true),
        Err(e) => {
            let _ = tx.send(Event::Error(e));
            Some(false)
        }
    }
}

// ---------------------------------------------------------------------------------------------------

/// Spawn `exe args` (no console window), write `stdin` then close it, feed stdout lines to `on_line`,
/// keep the child in `slot` so `Run::stop` can kill it. Err = spawn failure or a non-zero exit (with the
/// tail of stderr).
fn run_child(
    exe: &Path,
    args: &[String],
    stdin: Option<&str>,
    key: Option<&(String, String)>,
    slot: &Mutex<Option<Child>>,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let batch = exe.extension().is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    let mut cmd = crate::media::ffpipe::command(exe);
    for a in args {
        // ponytail: cmd.exe can't carry a newline inside an argument; flatten them for .cmd shims only
        if batch {
            cmd.arg(a.replace(['\r', '\n'], " "));
        } else {
            cmd.arg(a);
        }
    }
    if let Some((name, value)) = key.filter(|(n, v)| !n.trim().is_empty() && !v.is_empty()) {
        cmd.env(name.trim(), value);
    }
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("couldn't start {}: {e}", exe.display()))?;
    if let (Some(text), Some(mut w)) = (stdin, child.stdin.take()) {
        let _ = w.write_all(text.as_bytes()); // dropped right after: stdin closed
    }
    let out = child.stdout.take();
    let err = child.stderr.take();
    if let Ok(mut s) = slot.lock() {
        *s = Some(child);
    }
    // stderr on its own thread so a chatty child can't block on a full pipe
    let err_tail = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut e) = err {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    if let Some(out) = out {
        for line in BufReader::new(out).lines() {
            let Ok(line) = line else { break };
            on_line(&line);
        }
    }
    let status = slot.lock().ok().and_then(|mut s| s.take()).and_then(|mut c| c.wait().ok());
    let tail = err_tail.join().unwrap_or_default();
    match status {
        Some(s) if !s.success() => {
            let tail = tail.trim();
            let skip = tail.chars().count().saturating_sub(400);
            let tail: String = tail.chars().skip(skip).collect();
            Err(if tail.is_empty() { format!("exited with {s}") } else { format!("exited with {s}: {tail}") })
        }
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------------------------------
// API key in Windows Credential Manager

const CRED_TARGET: &str = "SimpleEditor/assistant";

pub fn key_load() -> Option<String> {
    use windows::core::HSTRING;
    use windows::Win32::Security::Credentials::{CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC};
    let mut p: *mut CREDENTIALW = std::ptr::null_mut();
    unsafe { CredReadW(&HSTRING::from(CRED_TARGET), CRED_TYPE_GENERIC, None, &mut p) }.ok()?;
    if p.is_null() {
        return None;
    }
    let bytes = unsafe {
        let c = &*p;
        let b = if c.CredentialBlob.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec()
        };
        CredFree(p as *const std::ffi::c_void);
        b
    };
    String::from_utf8(bytes).ok().filter(|s| !s.is_empty())
}

pub fn key_save(key: &str) -> Result<(), String> {
    use windows::core::PWSTR;
    use windows::Win32::Security::Credentials::{CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC};
    let mut target: Vec<u16> = CRED_TARGET.encode_utf16().chain(Some(0)).collect();
    let mut user: Vec<u16> = "SimpleEditor".encode_utf16().chain(Some(0)).collect();
    let mut blob = key.trim().as_bytes().to_vec();
    let cred = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_mut_ptr()),
        UserName: PWSTR(user.as_mut_ptr()),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    unsafe { CredWriteW(&cred, 0) }.map_err(|e| e.to_string())
}

pub fn key_delete() {
    use windows::core::HSTRING;
    use windows::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
    let _ = unsafe { CredDeleteW(&HSTRING::from(CRED_TARGET), CRED_TYPE_GENERIC, None) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stream_json() {
        let init = r#"{"type":"system","subtype":"init","session_id":"abc-123","tools":["mcp__simple-editor__editor.context"]}"#;
        assert_eq!(parse_line(init), vec![Event::Session("abc-123".into())]);
        let asst = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Cutting the silences."},
            {"type":"tool_use","id":"t1","name":"mcp__simple-editor__timeline.cut","input":{"t":1.5}},
            {"type":"tool_use","id":"t2","name":"mcp__simple-editor__editor.context","input":{}}]}}"#;
        assert_eq!(
            parse_line(&asst.replace('\n', "")),
            vec![
                Event::Text("Cutting the silences.".into()),
                Event::Tool("timeline.cut {\"t\":1.5}".into()),
                Event::Tool("editor.context".into()),
            ]
        );
        let failed = r#"{"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":[{"type":"text","text":"no clip 9"}]}]}}"#;
        assert_eq!(parse_line(failed), vec![Event::Error("no clip 9".into())]);
        let ok = r#"{"type":"result","subtype":"success","is_error":false,"result":"Done","session_id":"abc-123"}"#;
        assert_eq!(parse_line(ok), vec![Event::Done { ok: true, session: Some("abc-123".into()) }]);
        let bad = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"No conversation found"}"#;
        assert_eq!(
            parse_line(bad),
            vec![Event::Error("No conversation found".into()), Event::Done { ok: false, session: None }]
        );
        // unknown shapes and junk are ignored, never an error
        for junk in ["", "not json", "[1,2]", r#"{"type":"stream_event"}"#, r#"{"type":"assistant"}"#] {
            assert!(parse_line(junk).is_empty(), "{junk}");
        }
    }

    #[test]
    fn claude_argv() {
        let a = claude_args("C:\\t\\m.json", "", None);
        assert_eq!(
            a,
            [
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--mcp-config",
                "C:\\t\\m.json",
                "--strict-mcp-config",
                "--allowedTools",
                "mcp__simple-editor",
                "--tools",
                ""
            ]
        );
        let a = claude_args("m.json", " sonnet ", Some("s1"));
        assert_eq!(&a[a.len() - 4..], ["--model", "sonnet", "--resume", "s1"]);
        // the header's model picker: "Default" is no flag, an alias goes to the agent's own flag
        assert!(model_flag("claude", "").is_empty() && model_flag("claude", "  ").is_empty());
        assert_eq!(model_flag("claude", "fable"), ["--model", "fable"]);
        assert_eq!(model_flag("codex", "gpt-5"), ["-m", "gpt-5"]);
        assert!(model_flag("external", "x").is_empty());
        let a = codex_args("cut it", "http://127.0.0.1:7340/mcp", "o3");
        assert_eq!(
            a,
            ["exec", "--skip-git-repo-check", "-c", "mcp_servers.simple-editor.url=\"http://127.0.0.1:7340/mcp\"", "-m", "o3", "cut it"]
        );
        assert_eq!(codex_args("p", "u", "").len(), 5, "no model, no -m");
        let cfg: Value = serde_json::from_str(&mcp_config_json("http://127.0.0.1:7340/mcp")).unwrap();
        assert_eq!(cfg["mcpServers"]["simple-editor"]["type"], "http");
        assert_eq!(cfg["mcpServers"]["simple-editor"]["url"], "http://127.0.0.1:7340/mcp");
    }

    #[test]
    fn custom_template_splits_then_substitutes() {
        let a = custom_argv(r#"codex exec --mcp "{mcp_url}" "{prompt}""#, "say \"hi\" to me", "http://x/mcp");
        assert_eq!(a, ["codex", "exec", "--mcp", "http://x/mcp", "say \"hi\" to me"]);
        let a = custom_argv("  my-agent   --ask={prompt} \"two words\" \"\" ", "a b", "u");
        assert_eq!(a, ["my-agent", "--ask=a b", "two words", ""]);
        assert!(custom_argv("   ", "p", "u").is_empty());
    }
}
