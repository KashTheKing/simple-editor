//! Object masks: SAM 2.1 tiny (Meta's Segment Anything 2, Apache-2.0) as an optional add-on, never part
//! of the exe. Click an object in the Preview and get a mask that follows its outline through the clip.
//!
//! The add-on (`%APPDATA%\SimpleEditor\bin\sam2`) is the `se-sam2.exe` helper (`tools/sam2-helper`, ours,
//! MIT), ONNX Runtime (Microsoft, MIT) with DirectML (Microsoft's redistributable DirectML.dll) for the
//! GPU and its CPU fallback, and the five SAM 2.1 tiny ONNX graphs (square-zero-labs' export of
//! facebook/sam2.1-hiera-tiny, Apache-2.0). The library's `bin/sam2.json` lists every part with its size
//! and SHA-256 (a part can be a file, or a zip / nupkg to take members out of); `install` fetches,
//! checks and unpacks them, then swaps the folder in.
//!
//! The helper is a child process (JSON lines on stdin / stdout, see its main.rs) that reads frames
//! through ffmpeg itself. One helper serves the whole app (`HELPER`): its model load takes seconds on
//! the CPU and up to a minute on DirectML the first time, so it stays up once started.
//!
//! **Mask cache format** (`Settings::cache_dir()/masks/<key>/`, what `Mask.matte` points at):
//!   `mask.json`  {"format":"se-matte","version":1,"width":W,"height":H,"fps":R,"start":S,"count":N,
//!                 "done":bool,"model":"sam2.1-hiera-tiny","source":path,"points":[[x,y,label]..],"seed":K}
//!   `NNNNNN.png` one 8-bit greyscale PNG per frame (0 = outside, 255 = inside, soft edge between),
//!                W×H = the source frame's aspect, long side ≤ 1024. Frame i shows source second
//!                S + i/R. A frame not written yet (a track still running) is simply absent.
//! Masks render from that cache. On the playback path nothing decodes: `frame` returns what is in
//! memory (the exact frame, else the nearest one already decoded) and queues the decode on a
//! background thread; off playback (scrub, export, a still) it decodes the exact frame in place.

use crate::engine::blitz;
use crate::engine::export::{self, Progress};
use crate::media::ffpipe;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub const EXE: &str = "se-sam2.exe";
/// The library file that lists the download.
pub const MANIFEST: &str = "bin/sam2.json";
/// Longest side of a cached mask frame (the model's own output is 1024²).
pub const MAX_SIDE: u32 = 1024;
const FORMAT: &str = "se-matte";
/// The graphs the helper loads (models/), and the constants file beside them.
pub const MODEL_FILES: [&str; 6] = [
    "vision_encoder.onnx",
    "mask_decoder.onnx",
    "memory_encoder.onnx",
    "memory_attention.onnx",
    "pointer_tpos.onnx",
    "constants.json",
];

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// `%APPDATA%\SimpleEditor\bin\sam2` - the unpacked add-on.
pub fn dir() -> PathBuf {
    blitz::bin_dir().join("sam2")
}

/// The helper: `SE_SAM2` (a path to se-sam2.exe; tests and development), else the installed copy. It
/// counts only with its runtime and models beside it. None = not installed.
pub fn exe() -> Option<PathBuf> {
    let env = std::env::var_os("SE_SAM2").map(PathBuf::from).filter(|p| p.is_file());
    let p = env.unwrap_or_else(|| dir().join(EXE));
    complete(p.parent()?).then_some(p)
}

/// The exe, the runtime and every model file are in `root`.
fn complete(root: &Path) -> bool {
    root.join(EXE).is_file()
        && root.join("onnxruntime.dll").is_file()
        && MODEL_FILES.iter().all(|f| root.join("models").join(f).is_file())
}

pub fn available() -> bool {
    exe().is_some()
}

/// Everything an add-on list needs in one call: the install folder, whether it is installed (exe,
/// runtime and models all present), the installed manifest version, and whether an install is running.
pub struct AddonState {
    pub dir: PathBuf,
    pub installed: bool,
    pub version: Option<String>,
    pub installing: bool,
}

/// The object-mask add-on's install folder and detection (see [`AddonState`]).
pub fn addon_state() -> AddonState {
    AddonState { dir: dir(), installed: available(), version: version(), installing: installing().is_some() }
}

/// The installed manifest's version, when installed through the app.
pub fn version() -> Option<String> {
    let text = std::fs::read_to_string(dir().join("sam2.json")).ok()?;
    parse_manifest(&text).ok().map(|m| m.version)
}

// ---------------------------------------------------------------------------------------------------
// install / uninstall

/// One download. `take` empty = the file itself lands at `path`; otherwise the download is a zip (a
/// .nupkg is one) and each (member, dest) pair is copied out of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub url: String,
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub take: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub version: String,
    pub parts: Vec<Part>,
}

impl Manifest {
    pub fn total(&self) -> u64 {
        self.parts.iter().map(|p| p.size).sum()
    }
}

/// A dest path stays inside the add-on folder: relative, no `..`, no drive.
fn safe_rel(p: &str) -> bool {
    let p = p.replace('\\', "/");
    !p.is_empty() && !p.starts_with('/') && !p.contains(':') && !p.split('/').any(|s| s == ".." || s.is_empty())
}

pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("sam2.json: {e}"))?;
    let s = |v: &Value, k: &str| v[k].as_str().unwrap_or("").trim().to_string();
    let mut parts = Vec::new();
    for p in v["files"].as_array().ok_or("sam2.json has no files")? {
        let take: Vec<(String, String)> = p["take"]
            .as_object()
            .map(|o| o.iter().map(|(k, d)| (k.clone(), d.as_str().unwrap_or("").to_string())).collect())
            .unwrap_or_default();
        let part = Part {
            url: s(p, "url"),
            path: s(p, "path"),
            sha256: s(p, "sha256").to_ascii_lowercase(),
            size: p["size"].as_u64().unwrap_or(0),
            take,
        };
        let dests_ok = if part.take.is_empty() { safe_rel(&part.path) } else { part.take.iter().all(|(_, d)| safe_rel(d)) };
        if part.url.is_empty() || part.sha256.len() != 64 || part.size == 0 || !dests_ok {
            return Err("sam2.json has an incomplete file entry".into());
        }
        parts.push(part);
    }
    let m = Manifest { version: s(&v, "version"), parts };
    if m.parts.is_empty() {
        return Err("sam2.json lists no files".into());
    }
    Ok(m)
}

/// Fetch `url` into `dest` with curl, keeping `prog` between `lo` and `hi` by the bytes on disk;
/// killed on cancel.
fn fetch(url: &str, dest: &Path, size: u64, prog: &Progress, (lo, hi): (f32, f32), what: &str) -> Result<(), String> {
    let curl = ffpipe::find_exe("curl.exe", "").ok_or("curl.exe not found")?;
    let mut child = ffpipe::command(&curl)
        .args(["--fail", "--location", "--silent", "--show-error", "--max-time", "3600", "-o"])
        .arg(dest)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    let tail = export::stderr_tail(&mut child);
    loop {
        if prog.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(export::CANCELLED.into());
        }
        match child.try_wait() {
            Ok(Some(st)) => {
                let msg = tail.and_then(|t| t.join().ok()).unwrap_or_default();
                return if st.success() {
                    Ok(())
                } else {
                    Err(if msg.trim().is_empty() { format!("download failed ({st})") } else { msg.trim().to_string() })
                };
            }
            Ok(None) => {
                let got = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
                let f = lo + (hi - lo) * (got as f32 / size.max(1) as f32).min(1.0);
                prog.set(f, format!("Downloading {what} ({:.0} / {:.0} MB)…", got as f64 / 1048576.0, size as f64 / 1048576.0));
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Windows' own tar (bsdtar reads zip; git's GNU tar on PATH does not).
fn unzip(zip: &Path, into: &Path) -> Result<(), String> {
    let tar = std::env::var_os("SystemRoot")
        .map(|r| PathBuf::from(r).join("System32").join("tar.exe"))
        .filter(|p| p.is_file())
        .ok_or("tar.exe not found")?;
    std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
    let out = ffpipe::command(&tar)
        .arg("-xf")
        .arg(zip)
        .arg("-C")
        .arg(into)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("tar: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("could not unpack: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// `install` from the settings' library.
pub fn install(prog: &Progress) -> Result<PathBuf, String> {
    use crate::engine::library::Source;
    install_from(&Source::of(&crate::settings::Settings::load()), &blitz::bin_dir(), prog)
}

/// Read the manifest from `src`, fetch (a folder library: copy) every part into `bin/sam2.dl` (kept by
/// checksum, so a failed install resumes without downloading again), check sizes and SHA-256, lay the
/// add-on out in `bin/sam2.new`, check the helper starts, then swap it in as `bin/sam2`.
pub fn install_from(src: &crate::engine::library::Source, bin: &Path, prog: &Progress) -> Result<PathBuf, String> {
    use crate::engine::library::Source;
    prog.set(0.01, "Looking up the object-mask add-on…");
    let text = crate::engine::library::manifest(src, MANIFEST)?;
    let m = parse_manifest(&text)?;
    let (dl, fresh, dest) = (bin.join("sam2.dl"), bin.join("sam2.new"), bin.join("sam2"));
    std::fs::create_dir_all(&dl).map_err(|e| format!("{}: {e}", dl.display()))?;
    let _ = std::fs::remove_dir_all(&fresh);
    let total = m.total().max(1) as f32;
    let mut at = 0u64;
    let r = (|| {
        for part in &m.parts {
            let (lo, hi) = (0.02 + 0.86 * at as f32 / total, 0.02 + 0.86 * (at + part.size) as f32 / total);
            at += part.size;
            let name = part.url.rsplit('/').next().unwrap_or("a file").to_string();
            let file = dl.join(&part.sha256);
            let have = std::fs::metadata(&file).map(|x| x.len() == part.size).unwrap_or(false)
                && blitz::sha256_of(&file).ok().as_deref() == Some(part.sha256.as_str());
            if !have {
                let tmp = dl.join(format!("{}.part", part.sha256));
                let _ = std::fs::remove_file(&tmp);
                match src {
                    Source::Dir(d) if !part.url.contains("://") => std::fs::copy(d.join(&part.url), &tmp)
                        .map(|_| ())
                        .map_err(|e| format!("{}: {e}", d.join(&part.url).display()))?,
                    _ => {
                        let url = if part.url.contains("://") {
                            part.url.clone()
                        } else {
                            src.url(&part.url).ok_or("no library address")?
                        };
                        fetch(&url, &tmp, part.size, prog, (lo, hi), &name)?;
                    }
                }
                prog.set(hi, format!("Checking {name}…"));
                let size = std::fs::metadata(&tmp).map(|x| x.len()).unwrap_or(0);
                if size != part.size {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(format!("{name} is {size} bytes, expected {}", part.size));
                }
                if blitz::sha256_of(&tmp)? != part.sha256 {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(format!("{name} does not match its checksum"));
                }
                std::fs::rename(&tmp, &file).map_err(|e| e.to_string())?;
            }
            if prog.is_cancelled() {
                return Err(export::CANCELLED.to_string());
            }
            if part.take.is_empty() {
                let to = fresh.join(&part.path);
                std::fs::create_dir_all(to.parent().unwrap_or(&fresh)).map_err(|e| e.to_string())?;
                std::fs::copy(&file, &to).map_err(|e| format!("{}: {e}", to.display()))?;
            } else {
                let x = bin.join("sam2.unzip");
                let _ = std::fs::remove_dir_all(&x);
                unzip(&file, &x)?;
                for (member, d) in &part.take {
                    let (from, to) = (x.join(member), fresh.join(d));
                    std::fs::create_dir_all(to.parent().unwrap_or(&fresh)).map_err(|e| e.to_string())?;
                    std::fs::copy(&from, &to).map_err(|e| format!("{name} has no {member}: {e}"))?;
                }
                let _ = std::fs::remove_dir_all(&x);
            }
        }
        prog.set(0.9, "Checking the helper starts…");
        if !complete(&fresh) {
            return Err("the download is missing se-sam2.exe, onnxruntime.dll or a model file".into());
        }
        check_runs(&fresh.join(EXE))?;
        std::fs::write(fresh.join("sam2.json"), &text).map_err(|e| e.to_string())?;
        drop_helper(); // the old one holds its files open
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&fresh, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        Ok(dest.join(EXE))
    })();
    let _ = std::fs::remove_dir_all(&fresh);
    if r.is_ok() {
        let _ = std::fs::remove_dir_all(&dl);
        prog.set(1.0, "Object masks installed");
    }
    r
}

/// It starts and is the helper (`--version` prints "se-sam2 x.y.z").
fn check_runs(exe: &Path) -> Result<(), String> {
    let out = ffpipe::command(exe)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("the helper does not start: {e}"))?;
    if String::from_utf8_lossy(&out.stdout).starts_with("se-sam2 ") {
        Ok(())
    } else {
        Err("the installed file is not the object-mask helper".into())
    }
}

/// Remove the add-on (masks it already made stay in the cache).
pub fn uninstall() -> Result<(), String> {
    drop_helper();
    let d = dir();
    if d.exists() {
        std::fs::remove_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    Ok(())
}

static INSTALLING: Mutex<Option<Arc<Progress>>> = Mutex::new(None);

/// The running install, if any.
pub fn installing() -> Option<Arc<Progress>> {
    lock(&INSTALLING).clone().filter(|p| !p.is_done())
}

/// Why the last install failed (shown beside the Install button until the next try).
pub fn install_error() -> Option<String> {
    let slot = lock(&INSTALLING);
    slot.as_ref().filter(|p| p.is_done() && !p.is_cancelled()).and_then(|p| p.error())
}

/// The click on "Install" (or `mask.install`): a background job. A second click returns the running one.
pub fn start_install() -> Arc<Progress> {
    let mut slot = lock(&INSTALLING);
    if let Some(p) = slot.as_ref().filter(|p| !p.is_done()) {
        return p.clone();
    }
    let p = export::spawn_job("sam2-install", |prog| install(prog).map(|_| ()));
    *slot = Some(p.clone());
    p
}

// ---------------------------------------------------------------------------------------------------
// the helper process

/// A line from the helper: a progress tick or a reply.
enum Line {
    Progress(usize, usize),
    Reply(Value),
}

pub struct Helper {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Line>,
    /// "gpu" or "cpu".
    pub device: String,
    low: Option<bool>,
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "{{\"cmd\":\"quit\"}}");
        let _ = self.stdin.flush();
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(500) {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Below normal priority while the timeline or the Source monitor plays (`media::bg::playing`).
fn set_low(child: &Child, low: bool) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::System::Threading::{SetPriorityClass, BELOW_NORMAL_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS};
    let class = if low { BELOW_NORMAL_PRIORITY_CLASS } else { NORMAL_PRIORITY_CLASS };
    // SAFETY: the handle belongs to `child`, which outlives this call; a failure leaves the class as it was.
    unsafe { SetPriorityClass(windows::Win32::Foundation::HANDLE(child.as_raw_handle()), class).is_ok() }
}

impl Helper {
    /// Start `exe serve` and wait for its "ready" line (the model load: seconds on the CPU, up to a
    /// minute on DirectML the first time). `cpu` = never try the GPU.
    pub fn start(exe: &Path, cpu: bool, cancel: &dyn Fn() -> bool) -> Result<Helper, String> {
        let mut cmd = ffpipe::command(exe);
        cmd.arg("serve");
        if cpu {
            cmd.arg("--cpu");
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("se-sam2: {e}"))?;
        let stdin = child.stdin.take().ok_or("se-sam2: no stdin")?;
        let out = child.stdout.take().ok_or("se-sam2: no stdout")?;
        let (tx, rx) = channel();
        let _ = std::thread::Builder::new().name("sam2-read".into()).spawn(move || {
            for line in std::io::BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                let msg = match (v["progress"].as_u64(), v["of"].as_u64()) {
                    (Some(i), Some(n)) => Line::Progress(i as usize, n as usize),
                    _ => Line::Reply(v),
                };
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
        let mut h = Helper { child, stdin, rx, device: String::new(), low: None };
        let ready = h.wait(&mut |_, _| {}, cancel, Duration::from_secs(600))?;
        h.device = ready["device"].as_str().unwrap_or("cpu").to_string();
        Ok(h)
    }

    /// The next reply, forwarding progress ticks. Err when the helper died, timed out or `cancel` said so.
    fn wait(&mut self, tick: &mut dyn FnMut(usize, usize), cancel: &dyn Fn() -> bool, limit: Duration) -> Result<Value, String> {
        let t = Instant::now();
        loop {
            let low = crate::media::bg::playing();
            if self.low != Some(low) && set_low(&self.child, low) {
                self.low = Some(low);
            }
            if cancel() {
                return Err(export::CANCELLED.into());
            }
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Line::Progress(i, n)) => tick(i, n),
                Ok(Line::Reply(v)) => {
                    return if v["ok"].as_bool() == Some(true) {
                        Ok(v)
                    } else {
                        Err(v["error"].as_str().unwrap_or("the object-mask helper failed").to_string())
                    };
                }
                Err(RecvTimeoutError::Timeout) if t.elapsed() < limit => {}
                Err(RecvTimeoutError::Timeout) => return Err("the object-mask helper stopped answering".into()),
                Err(RecvTimeoutError::Disconnected) => return Err("the object-mask helper stopped".into()),
            }
        }
    }

    pub fn call(&mut self, cmd: &Value, tick: &mut dyn FnMut(usize, usize), cancel: &dyn Fn() -> bool) -> Result<Value, String> {
        writeln!(self.stdin, "{cmd}").and_then(|_| self.stdin.flush()).map_err(|_| "the object-mask helper stopped".to_string())?;
        self.wait(tick, cancel, Duration::from_secs(600))
    }
}

/// The one helper the app keeps, and what it last encoded (`frame` is per helper).
struct Shared {
    helper: Option<Helper>,
    /// (video, source second) of the frame the helper holds as its seed.
    seed: Option<(String, f64)>,
}

static HELPER: Mutex<Shared> = Mutex::new(Shared { helper: None, seed: None });
/// What the helper is doing ("Loading the model…" while it starts), for the UI.
static HELPER_NOTE: Mutex<String> = Mutex::new(String::new());

pub fn helper_note() -> String {
    lock(&HELPER_NOTE).clone()
}

/// The running helper's device, kept apart from `HELPER` (which a track holds for minutes).
static DEVICE: Mutex<Option<String>> = Mutex::new(None);

/// "gpu" / "cpu" of the running helper (None = not started).
pub fn device() -> Option<String> {
    lock(&DEVICE).clone()
}

/// Stop the helper (uninstall, a cancel that killed it mid-command).
pub fn drop_helper() {
    let mut s = lock(&HELPER);
    s.helper = None;
    s.seed = None;
    *lock(&DEVICE) = None;
}

/// Run `f` with the helper, starting it first if needed. An error that came from the process (it died,
/// a cancel) drops it, so the next call starts a fresh one.
fn with_helper<T>(cancel: &dyn Fn() -> bool, f: impl FnOnce(&mut Helper, &mut Option<(String, f64)>) -> Result<T, String>) -> Result<T, String> {
    let exe = exe().ok_or("The object-mask add-on is not installed")?;
    let mut s = lock(&HELPER);
    if s.helper.is_none() {
        *lock(&HELPER_NOTE) = "Loading the model…".into();
        let cpu = std::env::var_os("SE_SAM2_CPU").is_some();
        let h = Helper::start(&exe, cpu, cancel);
        lock(&HELPER_NOTE).clear();
        let h = h?;
        *lock(&DEVICE) = Some(h.device.clone());
        s.helper = Some(h);
        s.seed = None;
    }
    let Shared { helper, seed } = &mut *s;
    let h = helper.as_mut().ok_or("no helper")?;
    let r = f(h, seed);
    if let Err(e) = &r {
        if e == export::CANCELLED || e.contains("stopped") {
            *helper = None;
            *seed = None;
            *lock(&DEVICE) = None;
        }
    }
    r
}

/// Start the helper now (the model load), so the first click does not wait for it.
pub fn warm(cancel: &dyn Fn() -> bool) -> Result<(), String> {
    with_helper(cancel, |_, _| Ok(()))
}

/// Write `m` as an 8-bit greyscale PNG (the cache format's frame file).
pub fn write_png(path: &Path, m: &Matte) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let f = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), m.w, m.h);
    e.set_color(png::ColorType::Grayscale);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().and_then(|mut w| w.write_image_data(&m.px)).map_err(|x| x.to_string())
}

// ---------------------------------------------------------------------------------------------------
// prompts: click points on one frame

/// A click: x, y in 0..1 of the source frame, `add` = on the object (left click), else off it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    pub add: bool,
}

pub fn points_json(points: &[Point]) -> Value {
    Value::Array(points.iter().map(|p| json!([p.x, p.y, if p.add { 1 } else { 0 }])).collect())
}

/// Points from `[[x, y, label], …]` (label 1 = add, 0 = remove; missing = add).
pub fn parse_points(v: &Value) -> Result<Vec<Point>, String> {
    let a = v.as_array().ok_or("points: a list of [x, y, label]")?;
    a.iter()
        .map(|p| {
            let q = p.as_array().ok_or("points: each one is [x, y, label]")?;
            let n = |i: usize| q.get(i).and_then(Value::as_f64);
            let (x, y) = (n(0).ok_or("points: x")?, n(1).ok_or("points: y")?);
            if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
                return Err("points: x and y are 0..1 of the frame".to_string());
            }
            Ok(Point { x: x as f32, y: y as f32, add: n(2).unwrap_or(1.0) > 0.5 })
        })
        .collect()
}

/// The mask frame size for a w×h source: same aspect, long side ≤ MAX_SIDE.
pub fn matte_size(w: u32, h: u32) -> (u32, u32) {
    let (w, h) = (w.max(1), h.max(1));
    let k = (MAX_SIDE as f64 / w.max(h) as f64).min(1.0);
    (((w as f64 * k).round() as u32).max(1), ((h as f64 * k).round() as u32).max(1))
}

/// One mask on one frame (blocking; a background thread only): encodes `video` at source second
/// `time` unless the helper already holds it, then decodes the points. Returns the mask and the
/// helper's reply (iou, score, area, ms).
pub fn segment(video: &str, time: f64, points: &[Point], size: (u32, u32), cancel: &dyn Fn() -> bool) -> Result<(Matte, Value), String> {
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let out = crate::settings::Settings::cache_dir().join("masks").join(format!("pick-{}.png", std::process::id()));
    std::fs::create_dir_all(out.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    let reply = with_helper(cancel, |h, seed| {
        let key = (video.to_string(), (time * 1000.0).round() / 1000.0);
        if seed.as_ref() != Some(&key) {
            *seed = None;
            h.call(&json!({"cmd": "frame", "ffmpeg": ffmpeg, "video": video, "time": time}), &mut |_, _| {}, cancel)?;
            *seed = Some(key);
        }
        h.call(
            &json!({"cmd": "points", "points": points_json(points), "out": out, "w": size.0, "h": size.1}),
            &mut |_, _| {},
            cancel,
        )
    })?;
    let m = decode_png(&out, 0)?;
    let _ = std::fs::remove_file(&out);
    Ok((m, reply))
}

// ---------------------------------------------------------------------------------------------------
// tracking

/// One propagation: `count` frames of `video` from source second `start` at `fps`, prompted with
/// `points` on frame `seed`, written into `dir` (the cache format above) at `size`.
#[derive(Clone, Debug)]
pub struct TrackReq {
    pub video: String,
    pub start: f64,
    pub fps: f64,
    pub count: usize,
    pub seed: usize,
    pub points: Vec<Point>,
    pub dir: PathBuf,
    pub size: (u32, u32),
}

/// The cache folder for a track: one per (source, range, rate, prompt, size), so the same request
/// lands on the same frames and a different one never mixes with them.
pub fn matte_dir(r: &TrackReq) -> PathBuf {
    let key = format!(
        "{}|{:.4}|{:.4}|{}|{}|{:?}|{:?}",
        r.video.to_ascii_lowercase(),
        r.start,
        r.fps,
        r.count,
        r.seed,
        r.points.iter().map(|p| ((p.x * 1e4) as i32, (p.y * 1e4) as i32, p.add)).collect::<Vec<_>>(),
        r.size
    );
    // FNV-1a: stable across runs (DefaultHasher is not)
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    crate::settings::Settings::cache_dir().join("masks").join(format!("{h:016x}"))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Meta {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub start: f64,
    pub count: usize,
    pub done: bool,
}

impl Meta {
    /// The frame that shows source second `t` (clamped into the sequence).
    pub fn index(&self, t: f64) -> usize {
        let i = ((t - self.start) * self.fps + 1e-6).floor();
        (i.max(0.0) as usize).min(self.count.saturating_sub(1))
    }
}

pub fn frame_path(dir: &Path, i: usize) -> PathBuf {
    dir.join(format!("{i:06}.png"))
}

pub fn write_meta(r: &TrackReq, done: bool) -> Result<(), String> {
    std::fs::create_dir_all(&r.dir).map_err(|e| format!("{}: {e}", r.dir.display()))?;
    let v = json!({
        "format": FORMAT, "version": 1, "width": r.size.0, "height": r.size.1, "fps": r.fps, "start": r.start,
        "count": r.count, "done": done, "model": "sam2.1-hiera-tiny", "source": r.video,
        "points": points_json(&r.points), "seed": r.seed,
    });
    let tmp = r.dir.join("mask.json.part");
    std::fs::write(&tmp, serde_json::to_string_pretty(&v).unwrap_or_default()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, r.dir.join("mask.json")).map_err(|e| e.to_string())
}

pub fn read_meta(dir: &Path) -> Result<Meta, String> {
    let text = std::fs::read_to_string(dir.join("mask.json")).map_err(|e| format!("{}: {e}", dir.display()))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("mask.json: {e}"))?;
    if v["format"].as_str() != Some(FORMAT) {
        return Err("mask.json is not a mask sequence".into());
    }
    let m = Meta {
        width: v["width"].as_u64().unwrap_or(0) as u32,
        height: v["height"].as_u64().unwrap_or(0) as u32,
        fps: v["fps"].as_f64().unwrap_or(0.0),
        start: v["start"].as_f64().unwrap_or(0.0),
        count: v["count"].as_u64().unwrap_or(0) as usize,
        done: v["done"].as_bool().unwrap_or(false),
    };
    if m.width == 0 || m.height == 0 || !(m.fps > 0.0) || m.count == 0 {
        return Err("mask.json is incomplete".into());
    }
    Ok(m)
}

/// Propagate `r` (blocking; a background job): the meta first (done = false), the frames as the helper
/// writes them, the meta again with done = true. Cancel kills the helper (the frames so far stay).
pub fn track(r: &TrackReq, prog: &Progress) -> Result<Value, String> {
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    if r.count == 0 || !(r.fps > 0.0) {
        return Err("nothing to track".into());
    }
    if !r.points.iter().any(|p| p.add) {
        return Err("add a point on the object first".into());
    }
    write_meta(r, false)?;
    forget(&r.dir);
    let cancel = || prog.is_cancelled();
    prog.set(0.0, if available() { "Starting the object tracker…" } else { "Not installed" });
    let cmd = json!({
        "cmd": "track", "ffmpeg": ffmpeg, "video": r.video, "start": r.start, "fps": r.fps, "count": r.count,
        "seed": r.seed, "points": points_json(&r.points), "dir": r.dir, "w": r.size.0, "h": r.size.1,
    });
    let t = Instant::now();
    let reply = with_helper(&cancel, |h, seed| {
        *seed = None; // tracking re-encodes frames, the held seed is gone
        let dev = h.device.clone();
        h.call(
            &cmd,
            &mut |i, n| {
                let fps = i as f64 / t.elapsed().as_secs_f64().max(1e-3);
                prog.set(i as f32 / n.max(1) as f32, format!("Tracking frame {i} of {n} ({fps:.1} fps, {dev})"));
            },
            &cancel,
        )
    })?;
    write_meta(r, true)?;
    forget(&r.dir);
    Ok(reply)
}

// ---------------------------------------------------------------------------------------------------
// reading the cache (the render path)

/// One decoded mask frame: w×h coverage bytes.
#[derive(Debug)]
pub struct Matte {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
    /// Its frame index in the sequence.
    pub idx: usize,
}

impl Matte {
    /// Coverage at u, v (0..1 of the frame), bilinear.
    #[inline]
    pub fn at(&self, u: f32, v: f32) -> f32 {
        let (w, h) = (self.w as usize, self.h as usize);
        let fx = (u * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
        let fy = (v * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let p = |x: usize, y: usize| self.px[y * w + x] as f32;
        let a = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * tx;
        let b = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * tx;
        (a + (b - a) * ty) / 255.0
    }
}

pub fn decode_png(path: &Path, idx: usize) -> Result<Matte, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(f));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; r.output_buffer_size().ok_or("png: too big")?];
    let info = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (info.width, info.height);
    let ch = info.color_type.samples();
    let px: Vec<u8> = if ch == 1 { buf[..(w * h) as usize].to_vec() } else { buf.chunks_exact(ch).take((w * h) as usize).map(|p| p[0]).collect() };
    if w == 0 || h == 0 || px.len() != (w * h) as usize {
        return Err("png: empty".into());
    }
    Ok(Matte { w, h, px, idx })
}

/// Decoded frames in memory, newest last; a small per-sequence window around what is shown.
const KEEP: usize = 48;
/// Frames decoded ahead of the asked one while playing.
const AHEAD: usize = 12;

#[derive(Default)]
struct Store {
    metas: HashMap<PathBuf, (Option<Meta>, Instant)>,
    frames: VecDeque<(PathBuf, usize, Arc<Matte>)>,
    queue: VecDeque<(PathBuf, usize)>,
    worker: bool,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();

fn with_store<T>(f: impl FnOnce(&mut Store) -> T) -> T {
    let mut g = lock(&STORE);
    f(g.get_or_insert_with(Store::default))
}

/// Drop what is cached for `dir` (it is being rewritten).
pub fn forget(dir: &Path) {
    with_store(|s| {
        s.metas.remove(dir);
        s.frames.retain(|f| f.0 != dir);
        s.queue.retain(|q| q.0 != dir);
    });
}

/// The meta of `dir`, re-read every second while its track is still running.
fn meta_of(s: &mut Store, dir: &Path) -> Option<Meta> {
    let stale = match s.metas.get(dir) {
        Some((Some(m), _)) if m.done => false,
        Some((_, at)) => at.elapsed() > Duration::from_secs(1),
        None => true,
    };
    if stale {
        s.metas.insert(dir.to_path_buf(), (read_meta(dir).ok(), Instant::now()));
    }
    s.metas.get(dir).and_then(|m| m.0.clone())
}

fn insert(s: &mut Store, dir: &Path, m: Arc<Matte>) {
    if s.frames.iter().any(|f| f.0 == dir && f.1 == m.idx) {
        return;
    }
    s.frames.push_back((dir.to_path_buf(), m.idx, m));
    while s.frames.len() > KEEP {
        s.frames.pop_front();
    }
}

fn start_worker(s: &mut Store) {
    if s.worker {
        return;
    }
    s.worker = std::thread::Builder::new()
        .name("matte-decode".into())
        .spawn(|| {
            crate::media::bg::lower_thread();
            loop {
                let job = {
                    let mut g = lock(&STORE);
                    loop {
                        if let Some(j) = g.as_mut().and_then(|s| s.queue.pop_front()) {
                            break j;
                        }
                        g = WAKE.wait(g).unwrap_or_else(|e| e.into_inner());
                    }
                };
                if let Ok(m) = decode_png(&frame_path(&job.0, job.1), job.1) {
                    with_store(|s| insert(s, &job.0, Arc::new(m)));
                }
            }
        })
        .is_ok();
}

/// The mask frame of sequence `dir` that shows source second `t`, without ever blocking playback:
/// while playing, the exact frame if decoded, else the nearest decoded one at or before it (else any),
/// with the decode of it and the next few queued. Off playback (`blocking`), the exact frame is
/// decoded here when it exists. None = nothing usable yet (the mask then lets the layer through).
pub fn frame(dir: &Path, t: f64, blocking: bool) -> Option<Arc<Matte>> {
    let (idx, hit, near, want) = with_store(|s| {
        let meta = meta_of(s, dir)?;
        let idx = meta.index(t);
        let hit = s.frames.iter().find(|f| f.0 == dir && f.1 == idx).map(|f| f.2.clone());
        let mut near: Option<&Arc<Matte>> = None;
        for f in s.frames.iter().filter(|f| f.0 == dir) {
            let better = match near {
                None => true,
                Some(n) => {
                    let (a, b) = (f.1 <= idx, n.idx <= idx);
                    (a && !b) || (a == b && f.1.abs_diff(idx) < n.idx.abs_diff(idx))
                }
            };
            if better {
                near = Some(&f.2);
            }
        }
        let near = near.cloned();
        let mut want = Vec::new();
        if hit.is_none() || !blocking {
            for i in idx..(idx + AHEAD).min(meta.count) {
                let queued = s.queue.iter().any(|q| q.0 == dir && q.1 == i);
                let have = s.frames.iter().any(|f| f.0 == dir && f.1 == i);
                if !queued && !have && (i != idx || !blocking) {
                    want.push(i);
                }
            }
        }
        Some((idx, hit, near, want))
    })?;
    if let Some(h) = hit {
        queue(dir, &want);
        return Some(h);
    }
    if blocking {
        if let Ok(m) = decode_png(&frame_path(dir, idx), idx) {
            let m = Arc::new(m);
            with_store(|s| insert(s, dir, m.clone()));
            queue(dir, &want);
            return Some(m);
        }
    }
    queue(dir, &want);
    near
}

fn queue(dir: &Path, idx: &[usize]) {
    if idx.is_empty() {
        return;
    }
    with_store(|s| {
        // only frames that exist: a running track has not written the later ones yet
        for &i in idx {
            if frame_path(dir, i).is_file() {
                s.queue.push_back((dir.to_path_buf(), i));
            }
        }
        while s.queue.len() > 4 * AHEAD {
            s.queue.pop_front();
        }
        start_worker(s);
    });
    WAKE.notify_one();
}

/// The matte frame for `mask` on `clip` at timeline time `t` (None: not an object mask, or no frame
/// yet). Blocks to decode only off playback.
pub fn frame_for(mask: &crate::model::Mask, clip: &crate::model::Clip, t: f64) -> Option<Arc<Matte>> {
    if mask.shape != crate::model::MaskShape::Object || mask.matte.is_empty() {
        return None;
    }
    frame(Path::new(&mask.matte), clip.src_time(t), !crate::media::bg::playing())
}

/// The coverage byte for matte value `k` (0..1) with the mask's invert and opacity at clip time `lt`.
#[inline]
fn cov(k: f32, invert: bool, opacity: f32) -> u8 {
    let k = if invert { 1.0 - k } else { k };
    ((k * opacity).clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Object-mask coverage over a w×h layer image (the layer is the source frame, so the matte maps
/// straight onto it): the CPU twin of the GPU's layer-space pass. False = nothing to apply.
pub fn coverage_layer(mask: &crate::model::Mask, clip: &crate::model::Clip, t: f64, w: u32, h: u32, out: &mut Vec<u8>) -> bool {
    let Some(m) = frame_for(mask, clip, t) else { return false };
    let lt = clip.local(t);
    let op = mask.opacity.at(lt).clamp(0.0, 1.0) as f32;
    out.clear();
    out.resize(w as usize * h as usize, 0);
    for y in 0..h as usize {
        let v = (y as f32 + 0.5) / h as f32;
        for x in 0..w as usize {
            out[y * w as usize + x] = cov(m.at((x as f32 + 0.5) / w as f32, v), mask.invert, op);
        }
    }
    true
}

/// Object-mask coverage over a w×h canvas for a layer placed at (cx, cy) with size pw×ph rotated `rot`
/// degrees clockwise (`compose::Placement`, perspective ignored). Outside the layer the coverage is the
/// mask's "outside" value; the layer is transparent there anyway.
#[allow(clippy::too_many_arguments)]
pub fn coverage_canvas(
    mask: &crate::model::Mask,
    clip: &crate::model::Clip,
    t: f64,
    w: u32,
    h: u32,
    (cx, cy, pw, ph, rot): (f32, f32, f32, f32, f32),
    out: &mut Vec<u8>,
) -> bool {
    let Some(m) = frame_for(mask, clip, t) else { return false };
    let lt = clip.local(t);
    let op = mask.opacity.at(lt).clamp(0.0, 1.0) as f32;
    let outside = cov(0.0, mask.invert, op);
    out.clear();
    out.resize(w as usize * h as usize, outside);
    if !(pw > 0.0 && ph > 0.0) {
        return true;
    }
    let (sin, cos) = rot.to_radians().sin_cos();
    for y in 0..h as usize {
        let dy = y as f32 + 0.5 - cy;
        for x in 0..w as usize {
            let dx = x as f32 + 0.5 - cx;
            // canvas → layer space (undo the clockwise rotation, as compose::draw_layer does)
            let u = (dx * cos + dy * sin) / pw + 0.5;
            let v = (-dx * sin + dy * cos) / ph + 0.5;
            if (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v) {
                out[y * w as usize + x] = cov(m.at(u, v), mask.invert, op);
            }
        }
    }
    true
}

/// Intersection over union of two coverage buffers, each thresholded at half.
pub fn iou(a: &[u8], b: &[u8]) -> f64 {
    let (mut i, mut u) = (0usize, 0usize);
    for (&x, &y) in a.iter().zip(b) {
        let (x, y) = (x >= 128, y >= 128);
        i += (x && y) as usize;
        u += (x || y) as usize;
    }
    if u == 0 {
        1.0
    } else {
        i as f64 / u as f64
    }
}

#[cfg(test)]
#[path = "sam2_tests.rs"]
mod tests;
