//! Speech denoise: an optional add-on, never part of the exe. DeepFilterNet (MIT / Apache-2.0,
//! github.com/Rikorose/DeepFilterNet) ships a prebuilt `deep-filter` CLI with its model built in; the
//! user installs it on a click, the same way the AI enhancer is installed (`engine::enhance`): the
//! library's `bin/deepfilter.json` manifest names the exe, its size and SHA-256; it lands in
//! `%APPDATA%\SimpleEditor\bin\deepfilter`. Linking the `deep_filter` crate instead would cost the exe
//! tens of MB (tract + the model; the prebuilt CLI alone is 26.9 MB) and does not build on rustc 1.89.
//!
//! `run` cleans only the source range the clips use (plus handles) into a WAV in the cache
//! (`<cache>\denoise`), keyed by source file, range, strength and engine, so a second Clean of the same
//! thing is instant. The original file is never touched. Without the add-on, ffmpeg's `afftdn` is the
//! fallback. Every child runs below normal priority while something plays, so playback never waits on it.

use crate::engine::blitz;
use crate::engine::export::{self, Progress};
use crate::media::ffpipe;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const EXE: &str = "deep-filter.exe";
/// The library file that names the download.
pub const MANIFEST: &str = "bin/deepfilter.json";
pub const NOT_INSTALLED: &str =
    "Speech denoise (DeepFilterNet) is not installed (Settings ▸ General ▸ Speech denoise, or denoise.install)";
/// The strength a first Clean uses: strong, with a little of the room left so voices stay natural.
pub const DEFAULT_STRENGTH: f64 = 0.8;
/// Source seconds kept each side of what the clips use (room for trims and fades).
pub const HANDLE: f64 = 1.0;
/// Silence added after the range before DeepFilterNet runs: its look-ahead eats the last ~30 ms.
const TAIL_PAD: f64 = 0.25;

/// `%APPDATA%\SimpleEditor\bin\deepfilter` - the installed add-on.
pub fn dir() -> PathBuf {
    blitz::bin_dir().join("deepfilter")
}

/// The CLI: `SE_DEEPFILTER` (a path; tests and development), else the installed copy. None = not installed.
pub fn exe() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SE_DEEPFILTER").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    Some(dir().join(EXE)).filter(|p| p.is_file())
}

/// Whether the add-on is installed. The Inspector and Settings ask every frame, so the answer is kept for
/// a few seconds (a stat on the UI thread can take tens of ms on a busy disk); install and uninstall
/// refresh it at once.
pub fn available() -> bool {
    const TTL: std::time::Duration = std::time::Duration::from_secs(5);
    let mut g = AVAILABLE.lock().unwrap_or_else(|e| e.into_inner());
    match *g {
        Some((at, yes)) if at.elapsed() < TTL => yes,
        _ => {
            let yes = exe().is_some();
            *g = Some((std::time::Instant::now(), yes));
            yes
        }
    }
}

static AVAILABLE: std::sync::Mutex<Option<(std::time::Instant, bool)>> = std::sync::Mutex::new(None);

/// Forget the kept `available()` answer (after an install or uninstall).
fn recheck() {
    *AVAILABLE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The installed manifest's version ("0.5.6"), when installed through the app.
pub fn version() -> Option<String> {
    let text = std::fs::read_to_string(dir().join("deepfilter.json")).ok()?;
    parse_manifest(&text).ok().map(|m| m.version)
}

/// Installer hook - everything about this add-on's install in one call:
/// (install folder, installed exe if detected, installed version). The folder is all it owns.
#[allow(dead_code)] // for the installer / uninstaller (claude/feat-installer); nothing in the app calls it
pub fn install_state() -> (PathBuf, Option<PathBuf>, Option<String>) {
    (dir(), exe(), version())
}

pub fn parse_manifest(text: &str) -> Result<crate::engine::enhance::Manifest, String> {
    crate::engine::enhance::parse_manifest(text).map_err(|e| e.replace("enhancer.json", "deepfilter.json"))
}

// ---------------------------------------------------------------------------------------------------
// engine + strength

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// DeepFilterNet: a speech model; removes fans, hum, traffic and room noise, keeps the voice.
    DeepFilter,
    /// ffmpeg `afftdn`: spectral subtraction; fine on steady hiss, smears on anything else.
    Ffmpeg,
}

impl Engine {
    pub fn parse(s: &str) -> Option<Engine> {
        match s.trim().to_ascii_lowercase().as_str() {
            "deepfilter" | "deep-filter" | "deepfilternet" | "dfn" | "ai" => Some(Engine::DeepFilter),
            "ffmpeg" | "afftdn" => Some(Engine::Ffmpeg),
            _ => None,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Engine::DeepFilter => "deepfilter",
            Engine::Ffmpeg => "ffmpeg",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Engine::DeepFilter => "DeepFilterNet",
            Engine::Ffmpeg => "ffmpeg afftdn",
        }
    }
}

/// The engine a request runs on: the one asked for, else DeepFilterNet when installed, else ffmpeg.
/// Asking for DeepFilterNet without the add-on is an error (never a silent downgrade).
pub fn resolve(asked: Option<Engine>, installed: bool) -> Result<Engine, String> {
    match asked {
        Some(Engine::DeepFilter) if !installed => Err(NOT_INSTALLED.into()),
        Some(e) => Ok(e),
        None if installed => Ok(Engine::DeepFilter),
        None => Ok(Engine::Ffmpeg),
    }
}

/// DeepFilterNet's attenuation limit for a strength 0..1: 0 dB = untouched, 100 = no limit (full
/// removal). Below full, the noise is turned down by at most this much, which keeps a room's air.
pub fn atten_db(strength: f64) -> f64 {
    let s = strength.clamp(0.0, 1.0);
    if s >= 0.995 {
        100.0
    } else {
        (s * 60.0 * 10.0).round() / 10.0
    }
}

/// The fallback's filter for a strength 0..1 (`nr` = dB of reduction; ffmpeg's own default is 12).
pub fn afftdn_filter(strength: f64) -> String {
    format!("afftdn=nr={:.1}", 3.0 + 27.0 * strength.clamp(0.0, 1.0))
}

// ---------------------------------------------------------------------------------------------------
// install / uninstall

/// Download (or copy, from a folder library) the CLI, check size and SHA-256, check it runs, then
/// swap it in as `dir()`. Blocking: a background job only, started by a click.
pub fn install(prog: &Progress) -> Result<PathBuf, String> {
    use crate::engine::library::Source;
    let src = Source::of(&crate::settings::Settings::load());
    prog.set(0.02, "Looking up speech denoise…");
    let text = match &src {
        Source::Dir(d) => {
            std::fs::read_to_string(d.join(MANIFEST)).map_err(|e| format!("{}: {e}", d.join(MANIFEST).display()))?
        }
        Source::Url(_) => {
            let url = src.url(MANIFEST).ok_or("no library address")?;
            let dest = crate::engine::library::cache_root().join(MANIFEST);
            blitz::curl(&url, &dest, 30)?;
            std::fs::read_to_string(&dest).map_err(|e| e.to_string())?
        }
    };
    let m = parse_manifest(&text)?;
    let bin = blitz::bin_dir();
    std::fs::create_dir_all(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let fresh = bin.join("deepfilter.new");
    let part = fresh.join(EXE);
    let _ = std::fs::remove_dir_all(&fresh);
    std::fs::create_dir_all(&fresh).map_err(|e| format!("{}: {e}", fresh.display()))?;
    prog.set(0.1, format!("Downloading speech denoise ({:.0} MB)…", m.size as f64 / 1048576.0));
    let got = match &src {
        Source::Dir(d) if !m.url.contains("://") => {
            std::fs::copy(d.join(&m.url), &part).map(|_| ()).map_err(|e| format!("{}: {e}", d.join(&m.url).display()))
        }
        _ => {
            let url =
                if m.url.contains("://") { m.url.clone() } else { src.url(&m.url).ok_or("no library address")? };
            blitz::curl(&url, &part, 1800)
        }
    };
    let r = got.and_then(|_| {
        if prog.is_cancelled() {
            return Err(export::CANCELLED.into());
        }
        prog.set(0.85, "Checking it…");
        verify(&part, m.size, &m.sha256)?;
        check_runs(&part)?;
        std::fs::write(fresh.join("deepfilter.json"), &text).map_err(|e| e.to_string())?;
        let dest = dir();
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&fresh, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        Ok(dest.join(EXE))
    });
    let _ = std::fs::remove_dir_all(&fresh);
    recheck();
    let exe = r?;
    prog.set(1.0, "Speech denoise installed");
    Ok(exe)
}

/// The download is the file the manifest names: its size, then its SHA-256.
fn verify(file: &Path, size: u64, sha256: &str) -> Result<(), String> {
    let got = std::fs::metadata(file).map(|x| x.len()).unwrap_or(0);
    if got != size {
        return Err(format!("the download is {got} bytes, expected {size}"));
    }
    if blitz::sha256_of(file)? != sha256 {
        return Err("the download does not match its checksum".into());
    }
    Ok(())
}

/// It starts and is DeepFilterNet (`--version` prints "deep_filter 0.5.6").
fn check_runs(exe: &Path) -> Result<(), String> {
    let out = ffpipe::command(exe)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("deep-filter does not start: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if text.contains("deep_filter") {
        Ok(())
    } else {
        Err("the installed file is not DeepFilterNet".into())
    }
}

/// Remove the add-on (cleaned audio it made stays in the cache).
pub fn uninstall() -> Result<(), String> {
    let d = dir();
    let r = if d.exists() { std::fs::remove_dir_all(&d).map_err(|e| format!("{}: {e}", d.display())) } else { Ok(()) };
    recheck();
    r
}

static INSTALLING: std::sync::Mutex<Option<std::sync::Arc<Progress>>> = std::sync::Mutex::new(None);

/// The running install, if any.
pub fn installing() -> Option<std::sync::Arc<Progress>> {
    INSTALLING.lock().unwrap_or_else(|e| e.into_inner()).clone().filter(|p| !p.is_done())
}

/// Why the last install failed (shown beside the Install button until the next try).
pub fn install_error() -> Option<String> {
    let slot = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    slot.as_ref().filter(|p| p.is_done() && !p.is_cancelled()).and_then(|p| p.error())
}

/// The click on "Install": a background job (the Jobs pane shows it). Never called by anything but a
/// click or the `denoise.install` tool. A second click while one runs returns that one.
pub fn start_install() -> std::sync::Arc<Progress> {
    let mut slot = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = slot.as_ref().filter(|p| !p.is_done()) {
        return p.clone();
    }
    let p = export::spawn_job("deepfilter-install", |prog| install(prog).map(|_| ()));
    *slot = Some(p.clone());
    p
}

// ---------------------------------------------------------------------------------------------------
// the cache

/// `<cache>\denoise`: cleaned audio. Not part of the cache budget's evictable folders (clips play these).
pub fn cache_dir() -> PathBuf {
    crate::settings::Settings::cache_dir().join("denoise")
}

/// What a cleaned file is: written beside it as `<file>.json`, read back for A/B.
#[derive(Clone, Debug, PartialEq)]
pub struct Sidecar {
    /// The original source file.
    pub src: String,
    /// Source second the cleaned file's t=0 is.
    pub t0: f64,
    pub len: f64,
    pub strength: f64,
    pub engine: Engine,
}

impl Sidecar {
    fn to_json(&self) -> String {
        serde_json::json!({
            "src": self.src, "t0": self.t0, "len": self.len, "strength": self.strength, "engine": self.engine.id(),
        })
        .to_string()
    }
    fn parse(text: &str) -> Option<Sidecar> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        Some(Sidecar {
            src: v["src"].as_str()?.to_string(),
            t0: v["t0"].as_f64()?,
            len: v["len"].as_f64()?,
            strength: v["strength"].as_f64().unwrap_or(DEFAULT_STRENGTH),
            engine: Engine::parse(v["engine"].as_str().unwrap_or(""))?,
        })
    }
    /// The source window `(a, b)` fits inside this file.
    pub fn covers(&self, a: f64, b: f64) -> bool {
        a + 1e-3 >= self.t0 && b <= self.t0 + self.len + 0.05
    }
}

fn sidecar_path(cleaned: &Path) -> PathBuf {
    let mut s = cleaned.as_os_str().to_owned();
    s.push(".json");
    PathBuf::from(s)
}

static SIDECARS: std::sync::Mutex<Option<std::collections::HashMap<PathBuf, Option<Sidecar>>>> =
    std::sync::Mutex::new(None);

/// The sidecar of a cleaned file (None for any other file). Memoised: the Inspector asks every frame,
/// and only files inside `cache_dir()` are ever read.
pub fn sidecar_of(path: &Path) -> Option<Sidecar> {
    if !path.starts_with(cache_dir()) {
        return None;
    }
    let mut g = SIDECARS.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(Default::default);
    map.entry(path.to_path_buf())
        .or_insert_with(|| std::fs::read_to_string(sidecar_path(path)).ok().and_then(|t| Sidecar::parse(&t)))
        .clone()
}

fn remember(path: &Path, s: Option<Sidecar>) {
    let mut g = SIDECARS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(Default::default).insert(path.to_path_buf(), s);
}

/// FNV-1a: a stable short key (std's hasher may change between toolchains, which would orphan the cache).
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}

/// `<cache>\denoise\<stem>.<key>.wav`: the key covers the source file (path, size, modified time),
/// the range to the millisecond, the strength to the percent and the engine.
pub fn cached_path(src: &Path, range: (f64, f64), strength: f64, engine: Engine) -> PathBuf {
    let meta = std::fs::metadata(src).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let ms = |t: f64| (t * 1000.0).round() as i64;
    let key = format!(
        "{}|{size}|{mtime}|{}|{}|{}|{}",
        src.to_string_lossy().to_lowercase(),
        ms(range.0),
        ms(range.1),
        (strength.clamp(0.0, 1.0) * 100.0).round() as i64,
        engine.id()
    );
    let stem = src.file_stem().unwrap_or_default().to_string_lossy();
    cache_dir().join(format!("{stem}.{:016x}.wav", fnv(key.as_bytes())))
}

/// A finished cleaned file for exactly this request is already in the cache.
pub fn cached(out: &Path) -> bool {
    out.is_file() && sidecar_of(out).is_some()
}

// ---------------------------------------------------------------------------------------------------
// cleaning

/// One clean: `range` of `src` (source seconds) through `engine` at `strength`, written to `out` (WAV).
#[derive(Clone, Debug)]
pub struct Request {
    pub src: PathBuf,
    pub range: (f64, f64),
    pub strength: f64,
    pub engine: Engine,
    pub out: PathBuf,
}

/// ffmpeg: the source's first audio stream over `range`, 48 kHz 16-bit (DeepFilterNet's rate), with
/// `TAIL_PAD` of silence after it.
pub fn extract_args(src: &Path, range: (f64, f64), wav: &Path) -> Vec<String> {
    let len = (range.1 - range.0).max(0.001);
    let s = |v: &str| v.to_string();
    vec![
        s("-v"),
        s("error"),
        s("-y"),
        s("-ss"),
        format!("{:.6}", range.0),
        s("-t"),
        format!("{len:.6}"),
        s("-i"),
        src.to_string_lossy().into_owned(),
        s("-map"),
        s("0:a:0"),
        s("-vn"),
        s("-af"),
        format!("apad=pad_dur={TAIL_PAD}"),
        s("-ar"),
        s("48000"),
        s("-c:a"),
        s("pcm_s16le"),
        wav.to_string_lossy().into_owned(),
    ]
}

/// deep-filter: `-D` keeps it in sync (compensates the STFT + look-ahead delay), `-a` is the strength.
pub fn df_args(input: &Path, out_dir: &Path, strength: f64) -> Vec<String> {
    vec![
        "-D".into(),
        "-a".into(),
        format!("{}", atten_db(strength)),
        "-o".into(),
        out_dir.to_string_lossy().into_owned(),
        input.to_string_lossy().into_owned(),
    ]
}

/// ffmpeg: the cleaned audio cut (or padded) to exactly `len` seconds - the file lines up sample for
/// sample with the source range.
pub fn trim_args(input: &Path, len: f64, out: &Path) -> Vec<String> {
    let s = |v: &str| v.to_string();
    vec![
        s("-v"),
        s("error"),
        s("-y"),
        s("-i"),
        input.to_string_lossy().into_owned(),
        s("-af"),
        format!("apad=whole_dur={len:.6},atrim=duration={len:.6}"),
        s("-c:a"),
        s("pcm_s16le"),
        s("-f"),
        s("wav"),
        out.to_string_lossy().into_owned(),
    ]
}

/// ffmpeg alone (the fallback): one pass through `afftdn`.
pub fn ffmpeg_args(src: &Path, range: (f64, f64), strength: f64, out: &Path) -> Vec<String> {
    let len = (range.1 - range.0).max(0.001);
    let s = |v: &str| v.to_string();
    vec![
        s("-v"),
        s("error"),
        s("-y"),
        s("-ss"),
        format!("{:.6}", range.0),
        s("-t"),
        format!("{len:.6}"),
        s("-i"),
        src.to_string_lossy().into_owned(),
        s("-map"),
        s("0:a:0"),
        s("-vn"),
        s("-af"),
        afftdn_filter(strength),
        s("-ar"),
        s("48000"),
        s("-c:a"),
        s("pcm_s16le"),
        s("-f"),
        s("wav"),
        out.to_string_lossy().into_owned(),
    ]
}

/// Clean `req` into `req.out` (+ its sidecar). A cache hit returns at once. Blocking; a background job
/// (`export::spawn_job`) only. On failure or cancel nothing is left behind.
pub fn run(req: &Request, prog: &Progress) -> Result<(), String> {
    if cached(&req.out) {
        prog.set(1.0, "Cleaned (cached)");
        return Ok(());
    }
    let df = match req.engine {
        Engine::DeepFilter => Some(exe().ok_or(NOT_INSTALLED)?),
        Engine::Ffmpeg => None,
    };
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let parent = req.out.parent().ok_or("no output folder")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let stem = req.out.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let tmp = parent.join(format!(".{stem}.denoise-tmp"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let part = tmp.join("out.wav");
    let len = (req.range.1 - req.range.0).max(0.001);
    let r = (|| -> Result<(), String> {
        match &df {
            Some(df) => {
                prog.set(0.02, "Reading the audio…");
                let input = tmp.join("in.wav");
                run_child(&ffmpeg, &extract_args(&req.src, req.range, &input), "ffmpeg", prog, None)?;
                let done = tmp.join("df");
                std::fs::create_dir_all(&done).map_err(|e| e.to_string())?;
                // the CLI prints no progress: ease towards 90 % over roughly how long it takes (~8× real time)
                let est = 1.0 + len / 8.0;
                prog.set(0.1, "Cleaning speech (DeepFilterNet)…");
                run_child(df, &df_args(&input, &done, req.strength), "deep-filter", prog, Some((0.1, 0.9, est)))?;
                let cleaned = done.join("in.wav");
                if !cleaned.is_file() {
                    return Err("deep-filter wrote no file".into());
                }
                prog.set(0.92, "Lining it up…");
                run_child(&ffmpeg, &trim_args(&cleaned, len, &part), "ffmpeg", prog, None)
            }
            None => {
                prog.set(0.05, "Cleaning (ffmpeg afftdn)…");
                let est = 1.0 + len / 40.0;
                run_child(&ffmpeg, &ffmpeg_args(&req.src, req.range, req.strength, &part), "ffmpeg", prog, Some((0.05, 0.95, est)))
            }
        }?;
        if prog.is_cancelled() {
            return Err(export::CANCELLED.into());
        }
        let _ = std::fs::remove_file(&req.out);
        std::fs::rename(&part, &req.out).map_err(|e| format!("{}: {e}", req.out.display()))?;
        let side = Sidecar {
            src: req.src.to_string_lossy().into_owned(),
            t0: req.range.0,
            len,
            strength: req.strength,
            engine: req.engine,
        };
        std::fs::write(sidecar_path(&req.out), side.to_json()).map_err(|e| e.to_string())?;
        remember(&req.out, Some(side));
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    if r.is_err() {
        let _ = std::fs::remove_file(&req.out);
        let _ = std::fs::remove_file(sidecar_path(&req.out));
    }
    r?;
    prog.set(1.0, "Cleaned");
    Ok(())
}

/// The Denoise bake's DeepFilterNet stage: `src` (a rendered clip) → `out` with its picture copied and
/// its sound cleaned. Blocking; a background job only.
pub fn remux(src: &Path, out: &Path, strength: f64, prog: &Progress) -> Result<(), String> {
    let df = exe().ok_or(NOT_INSTALLED)?;
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let dur = probe_duration(src).unwrap_or(0.0);
    let parent = out.parent().ok_or("no output folder")?;
    let tmp = parent.join(format!(".{}.denoise-tmp", out.file_stem().unwrap_or_default().to_string_lossy()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("df")).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let r = (|| -> Result<(), String> {
        prog.set(0.02, "Reading the audio…");
        let input = tmp.join("in.wav");
        let end = if dur > 0.0 { dur } else { 1e6 };
        run_child(&ffmpeg, &extract_args(src, (0.0, end), &input), "ffmpeg", prog, None)?;
        prog.set(0.1, "Cleaning speech (DeepFilterNet)…");
        let est = 1.0 + dur / 8.0;
        run_child(&df, &df_args(&input, &tmp.join("df"), strength), "deep-filter", prog, Some((0.1, 0.85, est)))?;
        let cleaned = tmp.join("df").join("in.wav");
        prog.set(0.88, "Muxing…");
        let len = if dur > 0.0 { dur } else { 1e6 };
        let mut a: Vec<String> = ["-v", "error", "-y", "-i"].iter().map(|s| s.to_string()).collect();
        a.push(src.to_string_lossy().into_owned());
        a.extend(["-i".to_string(), cleaned.to_string_lossy().into_owned()]);
        a.extend(
            ["-map", "0:v?", "-map", "1:a:0", "-c:v", "copy", "-c:a", "aac", "-b:a", "256k", "-af"]
                .iter()
                .map(|s| s.to_string()),
        );
        a.push(format!("apad=whole_dur={len:.6},atrim=duration={len:.6}"));
        a.extend(["-movflags".to_string(), "+faststart".to_string(), out.to_string_lossy().into_owned()]);
        run_child(&ffmpeg, &a, "ffmpeg", prog, None)
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    if r.is_err() {
        let _ = std::fs::remove_file(out);
    }
    r?;
    prog.set(1.0, "Denoised");
    Ok(())
}

/// Run one child to the end (killed on cancel), below normal priority while anything plays. `ease`:
/// (from, to, expected seconds) - the fraction eases from `from` towards `to` while it runs.
fn run_child(
    exe: &Path,
    args: &[String],
    what: &str,
    prog: &Progress,
    ease: Option<(f32, f32, f64)>,
) -> Result<(), String> {
    let mut cmd: Command = ffpipe::command(exe);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("{what}: {e}"))?;
    let tail = export::stderr_tail(&mut child);
    let t = std::time::Instant::now();
    let status = prog.status();
    let mut low: Option<bool> = None;
    while !prog.is_cancelled() {
        let playing = crate::media::bg::playing();
        if low != Some(playing) && set_low(&child, playing) {
            low = Some(playing);
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(30)),
            Err(e) => return Err(e.to_string()),
        }
        if let Some((a, b, est)) = ease {
            let k = 1.0 - (-t.elapsed().as_secs_f64() / est.max(0.1)).exp();
            prog.set(a + (b - a) * k as f32, status.clone());
        }
    }
    export::wait_ffmpeg(&mut child, tail, prog).map_err(|e| if e == export::CANCELLED { e } else { format!("{what}: {e}") })
}

/// Container duration in seconds via ffprobe.
fn probe_duration(path: &Path) -> Option<f64> {
    let o = ffpipe::command(&ffpipe::ffprobe_exe()?)
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    String::from_utf8_lossy(&o.stdout).trim().parse().ok()
}

#[cfg(not(windows))]
fn set_low(_child: &std::process::Child, _low: bool) -> bool {
    true
}

#[cfg(windows)]
fn set_low(child: &std::process::Child, low: bool) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::System::Threading::{SetPriorityClass, BELOW_NORMAL_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS};
    let class = if low { BELOW_NORMAL_PRIORITY_CLASS } else { NORMAL_PRIORITY_CLASS };
    // SAFETY: the handle belongs to `child`, which outlives this call; a failure leaves the class as it was.
    unsafe { SetPriorityClass(windows::Win32::Foundation::HANDLE(child.as_raw_handle()), class).is_ok() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_manifest_parses() {
        let m = parse_manifest(include_str!("../../library/bin/deepfilter.json")).unwrap();
        assert!(m.url.starts_with("https://github.com/Rikorose/DeepFilterNet/"), "{}", m.url);
        assert!(m.url.ends_with(".exe"));
        assert_eq!(m.sha256.len(), 64);
        assert!(parse_manifest("{}").unwrap_err().contains("deepfilter.json"));
    }

    #[test]
    fn engine_defaults_to_deepfilter_only_when_installed() {
        assert_eq!(resolve(None, true), Ok(Engine::DeepFilter));
        assert_eq!(resolve(None, false), Ok(Engine::Ffmpeg));
        assert_eq!(resolve(Some(Engine::Ffmpeg), true), Ok(Engine::Ffmpeg));
        assert_eq!(resolve(Some(Engine::DeepFilter), false), Err(NOT_INSTALLED.to_string()));
        assert_eq!(Engine::parse("DeepFilter"), Some(Engine::DeepFilter));
        assert_eq!(Engine::parse("afftdn"), Some(Engine::Ffmpeg));
        assert_eq!(Engine::parse("x"), None);
    }

    #[test]
    fn strength_maps_to_the_attenuation_limit_and_afftdn() {
        assert_eq!(atten_db(1.0), 100.0);
        assert_eq!(atten_db(0.0), 0.0);
        assert_eq!(atten_db(0.5), 30.0);
        assert_eq!(atten_db(7.0), 100.0);
        assert!(atten_db(0.8) < atten_db(0.9));
        assert_eq!(afftdn_filter(0.0), "afftdn=nr=3.0");
        assert_eq!(afftdn_filter(1.0), "afftdn=nr=30.0");
    }

    #[test]
    fn args_cut_the_range_and_keep_sync() {
        let src = Path::new(r"C:\v\talk.mp4");
        let a = extract_args(src, (4.5, 10.0), Path::new("in.wav")).join(" ");
        assert!(a.contains("-ss 4.500000 -t 5.500000 -i C:\\v\\talk.mp4"), "{a}");
        assert!(a.contains("-map 0:a:0 -vn") && a.contains("apad=pad_dur=0.25") && a.contains("-ar 48000"), "{a}");
        let d = df_args(Path::new("in.wav"), Path::new("df"), 0.5).join(" ");
        assert_eq!(d, "-D -a 30 -o df in.wav");
        let t = trim_args(Path::new("df.wav"), 5.5, Path::new("out.wav")).join(" ");
        assert!(t.contains("apad=whole_dur=5.500000,atrim=duration=5.500000"), "{t}");
        let f = ffmpeg_args(src, (1.0, 3.0), 1.0, Path::new("o.wav")).join(" ");
        assert!(f.contains("-ss 1.000000 -t 2.000000") && f.contains("afftdn=nr=30.0") && f.ends_with("-f wav o.wav"), "{f}");
    }

    #[test]
    fn the_cache_key_changes_with_range_strength_and_engine() {
        let src = Path::new(r"C:\nope\voice.wav");
        let a = cached_path(src, (0.0, 5.0), 0.8, Engine::DeepFilter);
        assert!(a.starts_with(cache_dir()));
        assert!(a.file_name().unwrap().to_string_lossy().starts_with("voice."));
        assert_eq!(a, cached_path(src, (0.0, 5.0), 0.8, Engine::DeepFilter), "stable");
        assert_ne!(a, cached_path(src, (0.0, 5.1), 0.8, Engine::DeepFilter));
        assert_ne!(a, cached_path(src, (0.0, 5.0), 0.7, Engine::DeepFilter));
        assert_ne!(a, cached_path(src, (0.0, 5.0), 0.8, Engine::Ffmpeg));
        assert_eq!(fnv(b"a"), 0xaf63dc4c8601ec8c);
    }

    #[test]
    fn sidecars_round_trip_and_cover_windows() {
        let s = Sidecar { src: "C:/a.wav".into(), t0: 2.0, len: 4.0, strength: 0.5, engine: Engine::Ffmpeg };
        assert_eq!(Sidecar::parse(&s.to_json()), Some(s.clone()));
        assert!(s.covers(2.0, 6.0));
        assert!(!s.covers(1.0, 3.0));
        assert!(!s.covers(3.0, 7.0));
        assert_eq!(Sidecar::parse("{}"), None);
        // only files in the cache folder have one
        assert_eq!(sidecar_of(Path::new(r"C:\Windows\win.ini")), None);
    }

    #[test]
    fn not_installed_fails_without_spawning_anything() {
        if available() {
            return; // installed here: the real run below covers it
        }
        let out = std::env::temp_dir().join(format!("se-dfn-none-{}.wav", std::process::id()));
        let req =
            Request { src: "nope.wav".into(), range: (0.0, 1.0), strength: 0.8, engine: Engine::DeepFilter, out: out.clone() };
        assert_eq!(run(&req, &Progress::new()).unwrap_err(), NOT_INSTALLED);
        assert!(!out.exists());
        assert_eq!(remux(Path::new("nope.mp4"), &out, 0.8, &Progress::new()).unwrap_err(), NOT_INSTALLED);
    }

    #[test]
    fn a_fake_cli_is_refused_and_a_bad_download_is_caught() {
        let d = std::env::temp_dir().join(format!("se-dfn-fake-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let fake = d.join(EXE);
        std::fs::write(&fake, b"not a program").unwrap();
        assert!(check_runs(&fake).is_err());
        assert!(verify(&fake, 3, &"0".repeat(64)).unwrap_err().contains("13 bytes"));
        assert!(verify(&fake, 13, &"0".repeat(64)).unwrap_err().contains("checksum"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The fallback really runs: a noisy tone through afftdn comes out the same length, quieter noise.
    #[test]
    fn the_ffmpeg_fallback_cleans_a_range() {
        let Some(ffmpeg) = ffpipe::ffmpeg_exe() else { return };
        let d = std::env::temp_dir().join(format!("se-dfn-ff-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let src = d.join("noisy.wav");
        let ok = ffpipe::command(&ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "anoisesrc=color=white:amplitude=0.1:duration=3", "-ar", "48000"])
            .arg(&src)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let out = d.join("clean.wav");
        let req = Request { src: src.clone(), range: (0.5, 2.5), strength: 1.0, engine: Engine::Ffmpeg, out: out.clone() };
        // not in the cache folder: write it, then the sidecar is read from disk directly
        run(&req, &Progress::new()).unwrap();
        assert!(out.is_file());
        let dur = probe_duration(&out).unwrap();
        assert!((dur - 2.0).abs() < 0.03, "{dur}");
        let side = Sidecar::parse(&std::fs::read_to_string(sidecar_path(&out)).unwrap()).unwrap();
        assert_eq!((side.t0, side.engine), (0.5, Engine::Ffmpeg));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A real DeepFilterNet run on a short noisy clip. Needs the add-on (`SE_DEEPFILTER` or installed):
    /// `cargo test real_deepfilter -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_deepfilter_run_lowers_the_noise_floor_and_keeps_the_length() {
        let ffmpeg = ffpipe::ffmpeg_exe().expect("ffmpeg");
        assert!(available(), "set SE_DEEPFILTER or install the add-on");
        let d = std::env::temp_dir().join(format!("se-dfn-real-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let src = d.join("noisy.wav");
        // a voice-band warble (a stand-in for speech) under steady pink noise
        let ok = ffpipe::command(&ffmpeg)
            .args([
                "-v", "error", "-y",
                "-f", "lavfi", "-i", "sine=frequency=220:duration=4,vibrato=f=5:d=0.5,volume=0.3",
                "-f", "lavfi", "-i", "anoisesrc=color=pink:amplitude=0.08:duration=4",
                "-filter_complex", "[0][1]amix=inputs=2:normalize=0", "-ar", "48000",
            ])
            .arg(&src)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let out = d.join("clean.wav");
        let req = Request { src: src.clone(), range: (0.5, 3.5), strength: 1.0, engine: Engine::DeepFilter, out: out.clone() };
        let t = std::time::Instant::now();
        run(&req, &Progress::new()).unwrap();
        let dur = probe_duration(&out).unwrap();
        assert!((dur - 3.0).abs() < 0.01, "length kept: {dur}");
        let floor = |f: &Path| -> f64 {
            let o = ffpipe::command(&ffmpeg).args(["-hide_banner", "-i"]).arg(f).args(["-af", "astats", "-f", "null", "-"]).output().unwrap();
            let text = String::from_utf8_lossy(&o.stderr).to_string();
            let line = text.lines().filter(|l| l.contains("Noise floor dB")).last().unwrap().to_string();
            line.rsplit(':').next().unwrap().trim().parse().unwrap()
        };
        let (before, after) = (floor(&src), floor(&out));
        println!("noise floor {before:.1} dB -> {after:.1} dB in {:.1} s", t.elapsed().as_secs_f64());
        assert!(after < before - 10.0, "{before} -> {after}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
