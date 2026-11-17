//! ---- ws:html-layer ----
//! The fast HTML renderer: a client of the `se-html` helper (`tools/se-html`, Blitz = Stylo + Taffy +
//! Parley painted on the CPU: no browser, no JavaScript, no network, ~20 MB of RAM instead of Edge's
//! ~700 MB). The helper is a separate program that is NOT linked into this exe (it is ~11 MB): it lives in
//! `%APPDATA%\SimpleEditor\bin\se-html.exe`, installed on request (`install`, one click, nothing downloads
//! before it), and is talked to over stdin / stdout (`proto.rs`, shared source).
//!
//! What this file owns: finding / installing the helper, a `Helper` process handle with a timeout on every
//! call (a hung or crashed helper is killed and reported, never waited for), the choice of renderer for a
//! layer (`choose`: the Renderer setting + `html_lint` + failures seen so far), and the thin calls
//! `engine::html` makes at its frame-render entries. Every failure is an `Err(String)`: the caller draws the
//! layer with Edge instead and `remember_failure`s it so the Inspector can say why.

use crate::engine::html_lint;
use crate::media::ffpipe;
use crate::model::HtmlRenderer;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::BufReader;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{mpsc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[path = "../../tools/se-html/src/proto.rs"]
pub mod proto;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------------------------------
// where the helper is

/// `%APPDATA%\SimpleEditor\bin`
pub fn bin_dir() -> PathBuf {
    crate::settings::Settings::dir().join("bin")
}

/// The helper program: `SE_HTML_HELPER` (a path; tests and development), else the installed copy, else
/// one beside this exe (a portable / developer layout). None = not installed.
pub fn helper_exe() -> Option<PathBuf> {
    // a stale SE_HTML_HELPER (a path that is gone) must not hide an installed helper
    if let Some(p) = std::env::var_os("SE_HTML_HELPER").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let own = bin_dir().join("se-html.exe");
    if own.is_file() {
        return Some(own);
    }
    let beside = std::env::current_exe().ok()?.with_file_name("se-html.exe");
    beside.is_file().then_some(beside)
}

/// Is the helper installed? Remembered for 2 s: `choose` asks it per HTML layer per scan, and while
/// renders run the scan follows every still they write - two stats of a missing exe each time were most
/// of the UI thread on a project with nine layers. An install clears it (`start_install`).
pub fn available() -> bool {
    installed_stamp().is_some()
}

/// Which helper is installed (its size and time, hashed; None = none), remembered for 2 s like `available`.
/// A failure remembered for a page holds for that helper only: an update tries the page again.
fn installed_stamp() -> Option<u64> {
    let mut g = lock(&AVAILABLE);
    if let Some((at, stamp)) = *g {
        if at.elapsed() < Duration::from_secs(2) {
            return stamp;
        }
    }
    let stamp = helper_exe().map(|p| {
        use std::hash::{Hash, Hasher};
        let m = std::fs::metadata(&p).ok();
        let mut h = std::hash::DefaultHasher::new();
        (p, m.as_ref().map(|m| m.len()), m.and_then(|m| m.modified().ok())).hash(&mut h);
        h.finish()
    });
    *g = Some((Instant::now(), stamp));
    stamp
}

static AVAILABLE: Mutex<Option<(Instant, Option<u64>)>> = Mutex::new(None);

// ---------------------------------------------------------------------------------------------------
// the process

type Reply = std::io::Result<Option<(Value, Vec<u8>)>>;

/// A running `se-html`. Dropping it ends the process.
pub struct Helper {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<Reply>,
    pub last_used: Instant,
}

/// How long one call may take before the helper is declared hung and killed.
const LOAD_TIMEOUT: Duration = Duration::from_secs(120);
const FRAME_TIMEOUT: Duration = Duration::from_secs(45);

impl Helper {
    pub fn spawn() -> Result<Helper, String> {
        let exe = helper_exe().ok_or("the fast HTML renderer is not installed")?;
        let mut child = ffpipe::command(&exe)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("se-html: {e}"))?;
        crate::engine::html::tie_to_us(&child);
        let stdin = child.stdin.take().ok_or("se-html: no stdin")?;
        let stdout = child.stdout.take().ok_or("se-html: no stdout")?;
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("se-html-read".into())
            .spawn(move || {
                let mut r = BufReader::with_capacity(1 << 16, stdout);
                loop {
                    let m = proto::read_msg(&mut r);
                    let end = !matches!(m, Ok(Some(_)));
                    if tx.send(m).is_err() || end {
                        return;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let mut h = Helper { child, stdin, rx, last_used: Instant::now() };
        let (head, _) = h.call(json!({"cmd": "ping"}), Duration::from_secs(20))?;
        if head["protocol"].as_u64() != Some(proto::PROTOCOL as u64) {
            return Err(format!("se-html speaks protocol {} but this app needs {}: reinstall the fast renderer", head["protocol"], proto::PROTOCOL));
        }
        Ok(h)
    }

    /// One request and its answer. A timeout, a crash or a protocol error kills the process and is an Err;
    /// `{"ok": false}` is an Err carrying the helper's message (`panic: ` first when it was a panic).
    pub fn call(&mut self, head: Value, timeout: Duration) -> Result<(Value, Vec<u8>), String> {
        self.call_body(head, &[], timeout)
    }

    pub fn call_body(&mut self, head: Value, body: &[u8], timeout: Duration) -> Result<(Value, Vec<u8>), String> {
        self.last_used = Instant::now();
        if proto::write_msg(&mut self.stdin, &head, body).is_err() {
            self.kill();
            return Err("se-html stopped (it did not take the request)".into());
        }
        match self.rx.recv_timeout(timeout) {
            Ok(Ok(Some((h, b)))) => {
                if h["ok"].as_bool() == Some(true) {
                    Ok((h, b))
                } else {
                    let msg = h["error"].as_str().unwrap_or("unknown error");
                    Err(if h["panic"].as_bool() == Some(true) { format!("panic: {msg}") } else { msg.to_string() })
                }
            }
            Ok(_) => {
                self.kill();
                Err("se-html stopped unexpectedly".into())
            }
            Err(_) => {
                self.kill();
                Err(format!("se-html did not answer within {} s: stopped", timeout.as_secs()))
            }
        }
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Ask it to quit, then make sure.
    pub fn close(mut self) {
        let _ = proto::write_msg(&mut self.stdin, &json!({"cmd": "quit"}), &[]);
        let until = Instant::now() + Duration::from_millis(400);
        while Instant::now() < until && matches!(self.child.try_wait(), Ok(None)) {
            std::thread::sleep(Duration::from_millis(10));
        }
        self.kill();
    }

    pub fn load(&mut self, page: &str, base: &str, size: (u32, u32), scale: f64) -> Result<Loaded, String> {
        // the page's CSS for its inline svgs' parts, written onto them (the helper's SVG engine never sees
        // the stylesheet); what cannot be resolved Auto sends to Edge, an explicit Blitz gets it as it is
        let page = html_lint::inline_svg_styles(page).unwrap_or_else(|_| page.to_string());
        let (h, _) = self.call(json!({"cmd": "load", "html": page, "base": base, "w": size.0, "h": size.1, "scale": scale}), LOAD_TIMEOUT)?;
        Ok(Loaded {
            id: h["id"].as_u64().unwrap_or(0),
            height: h["height"].as_f64().unwrap_or(0.0),
            animated: h["animated"].as_bool().unwrap_or(false),
            ends: h["ends"].as_f64().filter(|e| *e >= 0.0),
        })
    }

    /// One picture (`png`: PNG bytes; else raw straight-alpha RGBA) of page `id` at second `t`.
    pub fn frame(&mut self, id: u64, t: f64, vars: &[(String, f64)], ptr: Option<(f64, f64, bool)>, png: bool) -> Result<Picture, String> {
        let mut head = json!({"cmd": "frame", "id": id, "t": t, "fmt": if png { "png" } else { "rgba" }});
        if !vars.is_empty() {
            head["vars"] = Value::Object(vars.iter().map(|(k, v)| (k.clone(), json!(v))).collect());
        }
        if let Some((x, y, down)) = ptr {
            head["ptr"] = json!([x, y, if down { 1.0 } else { 0.0 }]);
        }
        let (h, body) = self.call(head, FRAME_TIMEOUT)?;
        Ok(Picture { w: h["w"].as_u64().unwrap_or(0) as u32, h: h["h"].as_u64().unwrap_or(0) as u32, bytes: body })
    }

    /// Several pictures in one round trip (export): `items` = (t, params, pointer).
    #[allow(clippy::type_complexity)]
    pub fn frames(&mut self, id: u64, items: &[(f64, Vec<(String, f64)>, Option<(f64, f64, bool)>)], png: bool) -> Result<Vec<Picture>, String> {
        let list: Vec<Value> = items
            .iter()
            .map(|(t, vars, ptr)| {
                let mut o = json!({"t": t});
                if !vars.is_empty() {
                    o["vars"] = Value::Object(vars.iter().map(|(k, v)| (k.clone(), json!(v))).collect());
                }
                if let Some((x, y, d)) = ptr {
                    o["ptr"] = json!([x, y, if *d { 1.0 } else { 0.0 }]);
                }
                o
            })
            .collect();
        let (h, body) = self.call(json!({"cmd": "frames", "id": id, "items": list, "fmt": if png { "png" } else { "rgba" }}), FRAME_TIMEOUT * 4)?;
        let (w, hh) = (h["w"].as_u64().unwrap_or(0) as u32, h["h"].as_u64().unwrap_or(0) as u32);
        let (mut out, mut at) = (Vec::new(), 0usize);
        while at + 4 <= body.len() {
            let n = u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]) as usize;
            at += 4;
            if at + n > body.len() {
                return Err("se-html: a frame was cut short".into());
            }
            out.push(Picture { w, h: hh, bytes: body[at..at + n].to_vec() });
            at += n;
        }
        if out.len() != items.len() {
            return Err("se-html: wrong number of frames".into());
        }
        Ok(out)
    }

    /// Links, buttons, fields and headings of the page with their boxes (CSS px).
    pub fn elements(&mut self, id: u64) -> Result<Vec<Value>, String> {
        let (h, _) = self.call(json!({"cmd": "elements", "id": id}), FRAME_TIMEOUT)?;
        Ok(h["elements"].as_array().cloned().unwrap_or_default())
    }

    /// PNG of the part `rect` (x, y, w, h CSS px) of the page at `scale` picture px per CSS px.
    pub fn region(&mut self, id: u64, rect: [f64; 4], scale: f64, t: f64) -> Result<Vec<u8>, String> {
        let (_, body) = self.call(json!({"cmd": "region", "id": id, "rect": rect, "scale": scale, "t": t, "fmt": "png"}), FRAME_TIMEOUT * 2)?;
        Ok(body)
    }

    pub fn free(&mut self, id: u64) {
        let _ = self.call(json!({"cmd": "free", "id": id}), Duration::from_secs(10));
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = proto::write_msg(&mut self.stdin, &json!({"cmd": "quit"}), &[]);
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[derive(Clone, Debug)]
pub struct Loaded {
    pub id: u64,
    /// The laid-out page height in CSS px.
    pub height: f64,
    pub animated: bool,
    /// Seconds after which no finite animation runs; None = one repeats forever (or none: see `animated`).
    pub ends: Option<f64>,
}

pub struct Picture {
    pub w: u32,
    pub h: u32,
    pub bytes: Vec<u8>,
}

/// A helper with one page loaded: the frame source of a render worker.
pub struct Shooter {
    pub helper: Helper,
    pub info: Loaded,
}

impl Shooter {
    pub fn open(page: &str, base: &str, size: (u32, u32), scale: f64) -> Result<Shooter, String> {
        let mut helper = Helper::spawn()?;
        let info = helper.load(page, base, size, scale)?;
        Ok(Shooter { helper, info })
    }
    /// PNG of the page at `t` (seconds) with params and (optionally) the pointer.
    pub fn png(&mut self, t: f64, vars: &[(String, f64)], ptr: Option<(f64, f64, bool)>) -> Result<Vec<u8>, String> {
        self.helper.frame(self.info.id, t, vars, ptr, true).map(|p| p.bytes)
    }
    pub fn close(self) {
        self.helper.close();
    }
}

// ---------------------------------------------------------------------------------------------------
// the shared helper (stills, html.check, the browser layer's map): one process, started on first use,
// closed after a minute without a call

static SHARED: Mutex<Option<Helper>> = Mutex::new(None);
static REAPER: std::sync::Once = std::sync::Once::new();
const IDLE: Duration = Duration::from_secs(60);

pub fn with_shared<T>(mut f: impl FnMut(&mut Helper) -> Result<T, String>) -> Result<T, String> {
    REAPER.call_once(|| {
        let _ = std::thread::Builder::new().name("se-html-idle".into()).spawn(|| loop {
            std::thread::sleep(Duration::from_secs(10));
            let mut g = lock(&SHARED);
            if g.as_ref().is_some_and(|h| h.last_used.elapsed() > IDLE) {
                if let Some(h) = g.take() {
                    h.close();
                }
            }
        });
    });
    let mut g = lock(&SHARED);
    if g.is_none() {
        *g = Some(Helper::spawn()?);
    }
    let r = f(g.as_mut().unwrap());
    if r.is_err() && g.as_mut().is_some_and(|h| matches!(h.child.try_wait(), Ok(Some(_)))) {
        *g = None; // it died: the next call starts a new one
    }
    r
}

/// Stop the shared helper now (app exit, reinstall).
pub fn shutdown() {
    if let Some(h) = lock(&SHARED).take() {
        h.close();
    }
}

/// A still PNG of `page` at `t`: load, draw, free. `fullpage`: the whole page height (a web page snapshot).
#[allow(clippy::too_many_arguments)]
pub fn png_still(
    page: &str,
    base: &str,
    size: (u32, u32),
    scale: f64,
    t: f64,
    vars: &[(String, f64)],
    ptr: Option<(f64, f64, bool)>,
    fullpage: bool,
) -> Result<Vec<u8>, String> {
    with_shared(|h| {
        let mut info = h.load(page, base, size, scale)?;
        if fullpage && !info.animated && info.height > size.1 as f64 + 1.0 {
            let tall = info.height.ceil().min(16000.0) as u32;
            h.free(info.id);
            info = h.load(page, base, (size.0, tall), scale)?;
        }
        let r = h.frame(info.id, t, vars, ptr, true).map(|p| p.bytes);
        h.free(info.id);
        r
    })
}

/// The element map of `page` laid out at `width` CSS px (the browser layer's cursor targets) and the page
/// height. Replaces the Edge DOM query.
pub fn page_map(page: &str, base: &str, width: u32) -> Result<(Vec<Value>, f64), String> {
    with_shared(|h| {
        let info = h.load(page, base, (width, 1080), 1.0)?;
        let r = h.elements(info.id).map(|e| (e, info.height));
        h.free(info.id);
        r
    })
}

/// PNG of the part `rect` of `page` drawn at `scale` (lossless zoom).
pub fn region_png(page: &str, base: &str, width: u32, rect: [f64; 4], scale: f64, t: f64) -> Result<Vec<u8>, String> {
    with_shared(|h| {
        let info = h.load(page, base, (width, 1080), 1.0)?;
        let r = h.region(info.id, rect, scale, t);
        h.free(info.id);
        r
    })
}

// ---------------------------------------------------------------------------------------------------
// which renderer draws a layer

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Blitz,
    Edge,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub engine: Engine,
    /// Why Edge was chosen although the fast renderer was wanted ("uses backdrop filter: rendering with
    /// Edge"), or what the fast path drops ("text-shadow is drawn without the shadow").
    pub reason: Option<String>,
}

/// A failure of the helper on a source: for which helper (`installed_stamp`), how often, the last message.
/// Kept in memory only: a restart tries every page with the fast renderer again.
struct Failure {
    stamp: Option<u64>,
    count: u32,
    why: String,
}

static FAILED: Mutex<Option<HashMap<u64, Failure>>> = Mutex::new(None);

pub fn source_key(src: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    src.hash(&mut h);
    h.finish()
}

/// The helper failed (panic, timeout, crash, a page it could not load) on this source. Auto draws it with
/// Edge from then on when the failure is certain to come again (a panic: the helper cannot draw something
/// in it), else after the second one (a timeout on a busy PC is tried once more) - for this helper, this run.
pub fn remember_failure(src: &str, why: &str) {
    remember_failure_key(source_key(src), why);
}

pub fn remember_failure_key(key: u64, why: &str) {
    let stamp = installed_stamp();
    let mut g = lock(&FAILED);
    let f = g.get_or_insert_with(HashMap::new).entry(key).or_insert(Failure { stamp, count: 0, why: String::new() });
    if f.stamp != stamp {
        f.count = 0; // another helper since: its own count
        f.stamp = stamp;
    }
    f.count += 1;
    f.why = why.to_string();
}

/// Why Auto does not use the fast renderer for this source any more (None = it does).
fn failed_key(key: u64) -> Option<String> {
    let stamp = installed_stamp();
    let g = lock(&FAILED);
    let f = g.as_ref()?.get(&key)?;
    (f.stamp == stamp && (f.count >= 2 || f.why.starts_with("panic"))).then(|| f.why.clone())
}

pub fn failure_of(src: &str) -> Option<String> {
    failed_key(source_key(src))
}

/// Forget one source's failures (tests: `forget_failures` would race tests running beside them).
#[cfg(test)]
pub fn forget_failure(src: &str) {
    if let Some(m) = lock(&FAILED).as_mut() {
        m.remove(&source_key(src));
    }
}

pub fn forget_failures() {
    *lock(&FAILED) = None;
}

/// The renderer for a layer with this setting and source. Pure decision (no process is started, nothing
/// is downloaded): `Auto` = Blitz when the helper is installed, the lint finds nothing it cannot draw and
/// it has not failed on this source; `Blitz` = the user insists (Edge only when the helper is missing);
/// `Edge` = Edge.
pub fn choose(setting: HtmlRenderer, src: &str) -> Choice {
    choose_with(setting, src, available())
}

static SCANS: Mutex<Option<HashMap<u64, html_lint::Findings>>> = Mutex::new(None);

/// `html_lint::scan`, remembered per source (a web page is ~1 MB and the decision is asked for every frame).
fn scan_cached(src: &str, key: u64) -> html_lint::Findings {
    if let Some(f) = lock(&SCANS).as_ref().and_then(|m| m.get(&key)) {
        return f.clone();
    }
    let f = html_lint::scan(src);
    let mut g = lock(&SCANS);
    let m = g.get_or_insert_with(HashMap::new);
    if m.len() > 64 {
        m.clear();
    }
    m.insert(key, f.clone());
    f
}

pub fn choose_with(setting: HtmlRenderer, src: &str, helper_installed: bool) -> Choice {
    match setting {
        HtmlRenderer::Edge => Choice { engine: Engine::Edge, reason: None },
        HtmlRenderer::Blitz => {
            if helper_installed {
                let f = scan_cached(src, source_key(src));
                let reason = (!f.blocks.is_empty()).then(|| format!("uses {}: may not draw as in a browser", f.blocks.join(", ")));
                Choice { engine: Engine::Blitz, reason }
            } else {
                Choice { engine: Engine::Edge, reason: Some("the fast renderer is not installed: rendering with Edge".into()) }
            }
        }
        HtmlRenderer::Auto => {
            if !helper_installed {
                return Choice { engine: Engine::Edge, reason: Some("the fast renderer is not installed: rendering with Edge".into()) };
            }
            // ponytail: one hash of the source for both lookups (a web page is ~1 MB, asked per layer per scan)
            let key = source_key(src);
            let f = scan_cached(src, key);
            if let Some(b) = f.blocks.first() {
                return Choice { engine: Engine::Edge, reason: Some(format!("uses {b}: rendering with Edge")) };
            }
            if let Some(why) = failed_key(key) {
                return Choice { engine: Engine::Edge, reason: Some(format!("the fast renderer failed ({why}): rendering with Edge")) };
            }
            Choice { engine: Engine::Blitz, reason: f.notes.first().cloned() }
        }
    }
}

// ---------------------------------------------------------------------------------------------------
// install (opt-in): the card in the Inspector calls this once, on a click

/// What `bin/se-html.json` of the library says about the helper.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub version: String,
    /// Where the exe is: a path relative to the library (a folder source) or a full address.
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("se-html.json: {e}"))?;
    let s = |k: &str| v[k].as_str().unwrap_or("").trim().to_string();
    if v["protocol"].as_u64() != Some(proto::PROTOCOL as u64) {
        return Err(format!("this app needs renderer protocol {}, the library offers {}", proto::PROTOCOL, v["protocol"]));
    }
    let m = Manifest { version: s("version"), url: s("url"), sha256: s("sha256").to_ascii_lowercase(), size: v["size"].as_u64().unwrap_or(0) };
    if m.url.is_empty() || m.sha256.len() != 64 || m.size == 0 {
        return Err("se-html.json is incomplete".into());
    }
    Ok(m)
}

pub(crate) fn sha256_of(path: &std::path::Path) -> Result<String, String> {
    let out = ffpipe::command(std::path::Path::new("certutil"))
        .args(["-hashfile"])
        .arg(path)
        .arg("SHA256")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("certutil: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().map(|l| l.trim().replace(' ', "")).find(|l| l.len() == 64 && l.chars().all(|c| c.is_ascii_hexdigit())).map(|l| l.to_ascii_lowercase()).ok_or_else(|| "could not hash the download".to_string())
}

/// Download (or copy, from a folder library) the helper, check size, SHA-256 and that it runs and speaks
/// this protocol, then put it in `bin_dir()`. Blocking: a background job only, started by a click.
pub fn install(prog: &crate::engine::export::Progress) -> Result<PathBuf, String> {
    use crate::engine::library::Source;
    let src = Source::of(&crate::settings::Settings::load());
    prog.set(0.02, "Looking up the fast HTML renderer…");
    let manifest_text = match &src {
        Source::Dir(d) => std::fs::read_to_string(d.join("bin").join("se-html.json")).map_err(|e| format!("{}: {e}", d.join("bin").join("se-html.json").display()))?,
        Source::Url(_) => {
            let url = src.url("bin/se-html.json").ok_or("no library address")?;
            let dest = crate::engine::library::cache_root().join("bin").join("se-html.json");
            curl(&url, &dest, 30)?;
            std::fs::read_to_string(&dest).map_err(|e| e.to_string())?
        }
    };
    let m = parse_manifest(&manifest_text)?;
    let dir = bin_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = dir.join("se-html.exe.part");
    prog.set(0.1, format!("Downloading the fast HTML renderer ({:.0} MB)…", m.size as f64 / 1048576.0));
    match &src {
        Source::Dir(d) if !m.url.contains("://") => {
            std::fs::copy(d.join(&m.url), &part).map_err(|e| format!("{}: {e}", d.join(&m.url).display()))?;
        }
        _ => {
            let url = if m.url.contains("://") { m.url.clone() } else { src.url(&m.url).ok_or("no library address")? };
            curl(&url, &part, 900)?;
        }
    }
    prog.set(0.85, "Checking it…");
    let fail = |e: String| {
        let _ = std::fs::remove_file(&part);
        Err(e)
    };
    let got = std::fs::metadata(&part).map(|x| x.len()).unwrap_or(0);
    if got != m.size {
        return fail(format!("the download is {got} bytes, expected {}", m.size));
    }
    match sha256_of(&part) {
        Ok(h) if h == m.sha256 => {}
        Ok(_) => return fail("the download does not match its checksum".into()),
        Err(e) => return fail(e),
    }
    let exe = dir.join("se-html.exe");
    shutdown(); // a running copy blocks the replace
    let _ = std::fs::remove_file(&exe);
    std::fs::rename(&part, &exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    // it must run and speak this protocol
    match Helper::spawn() {
        Ok(h) => h.close(),
        Err(e) => {
            let _ = std::fs::remove_file(&exe);
            return Err(format!("installed file does not work: {e}"));
        }
    }
    std::fs::write(dir.join("se-html.json"), manifest_text).ok();
    forget_failures();
    prog.set(1.0, "Fast HTML renderer installed");
    Ok(exe)
}

static INSTALLING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// An install job is running (the card shows a progress note instead of the button).
pub fn installing() -> bool {
    INSTALLING.load(std::sync::atomic::Ordering::Relaxed)
}

/// The click on "Install": a background job (Jobs pane shows it). Never called by anything but a click or
/// the `html.helper` tool with install=true.
pub fn start_install() {
    use std::sync::atomic::Ordering::SeqCst;
    if INSTALLING.swap(true, SeqCst) || available() {
        return;
    }
    crate::engine::export::spawn_job("html-helper", |prog| {
        let r = install(prog);
        *lock(&AVAILABLE) = None;
        INSTALLING.store(false, SeqCst);
        r.map(|_| ())
    });
}

pub(crate) fn curl(url: &str, dest: &std::path::Path, max_secs: u32) -> Result<(), String> {
    let curl = ffpipe::find_exe("curl.exe", "").ok_or("curl.exe not found")?;
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let out = ffpipe::command(&curl)
        .args(["--fail", "--location", "--silent", "--show-error", "--max-time", &max_secs.to_string(), "-o"])
        .arg(dest)
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let _ = std::fs::remove_file(dest);
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if msg.is_empty() { format!("download failed ({})", out.status) } else { msg })
    }
}

/// The installed helper's version line, for the status tool and the Inspector (starts it if needed).
pub fn version_line() -> Option<String> {
    let exe = helper_exe()?;
    let out = ffpipe::command(&exe).arg("--version").stdin(Stdio::null()).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_chooses_by_install_lint_and_failures() {
        let plain = "<style>p{color:red}</style><p>x</p>";
        let filt = "<style>p{mix-blend-mode:screen}</style><p>x</p>";
        // not installed: Edge, whatever the setting says except an explicit Edge (no reason)
        assert_eq!(choose_with(HtmlRenderer::Auto, plain, false).engine, Engine::Edge);
        assert!(choose_with(HtmlRenderer::Auto, plain, false).reason.unwrap().contains("not installed"));
        assert_eq!(choose_with(HtmlRenderer::Edge, plain, true), Choice { engine: Engine::Edge, reason: None });
        // installed
        assert_eq!(choose_with(HtmlRenderer::Auto, plain, true), Choice { engine: Engine::Blitz, reason: None });
        let c = choose_with(HtmlRenderer::Auto, filt, true);
        assert_eq!(c.engine, Engine::Edge);
        assert_eq!(c.reason.as_deref(), Some("uses a blend mode (mix-blend-mode): rendering with Edge"));
        // the user insists: Blitz, with a warning
        let c = choose_with(HtmlRenderer::Blitz, filt, true);
        assert_eq!(c.engine, Engine::Blitz);
        assert!(c.reason.is_some());
        // a panic on this source sticks (for this helper, this run)
        let bad = "<p>fails once</p>";
        assert_eq!(choose_with(HtmlRenderer::Auto, bad, true).engine, Engine::Blitz);
        remember_failure(bad, "panic: x");
        let c = choose_with(HtmlRenderer::Auto, bad, true);
        assert_eq!(c.engine, Engine::Edge);
        assert!(c.reason.unwrap().contains("panic: x"));
        forget_failure(bad);
    }

    /// Regression: one failure of the helper (a timeout on a busy PC) sent a page to Edge for good. A
    /// timeout is tried once more; any failure counts for the helper it happened with only, and a restart
    /// (an empty memory) tries again.
    #[test]
    fn a_blitz_failure_is_retried_and_belongs_to_one_helper() {
        let page = "<p>timed out once</p>";
        let key = source_key(page);
        remember_failure(page, "se-html did not answer within 45 s: stopped");
        assert_eq!(choose_with(HtmlRenderer::Auto, page, true).engine, Engine::Blitz, "one timeout: tried again");
        remember_failure(page, "se-html did not answer within 45 s: stopped");
        assert_eq!(choose_with(HtmlRenderer::Auto, page, true).engine, Engine::Edge, "twice: Edge");
        // another helper version installed since: its own chance
        if let Some(f) = lock(&FAILED).as_mut().and_then(|m| m.get_mut(&key)) {
            f.stamp = Some(f.stamp.unwrap_or(0) ^ 1);
        }
        assert_eq!(choose_with(HtmlRenderer::Auto, page, true).engine, Engine::Blitz, "a new helper tries again");
        // a restart: nothing remembered
        forget_failure(page);
        assert_eq!(failure_of(page), None);
    }

    #[test]
    fn manifest_must_be_complete_and_match_the_protocol() {
        let ok = format!(r#"{{"version":"0.1.0","protocol":{},"url":"bin/se-html.exe","sha256":"{}","size":123}}"#, proto::PROTOCOL, "a".repeat(64));
        assert_eq!(parse_manifest(&ok).unwrap().size, 123);
        assert!(parse_manifest(&ok.replace("123", "0")).is_err());
        assert!(parse_manifest(&ok.replace(&"a".repeat(64), "abc")).is_err());
        assert!(parse_manifest(&ok.replace(&format!("\"protocol\":{}", proto::PROTOCOL), "\"protocol\":99")).is_err());
        assert!(parse_manifest("not json").is_err());
    }

    /// The decision itself never starts a process or reaches the network (nothing downloads before the
    /// click): `choose` only reads the file system.
    #[test]
    fn choosing_touches_neither_process_nor_network() {
        let before = lock(&SHARED).is_some();
        let _ = choose(HtmlRenderer::Auto, "<p>x</p>");
        assert_eq!(lock(&SHARED).is_some(), before);
    }
}
