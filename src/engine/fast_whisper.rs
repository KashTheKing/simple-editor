//! faster-whisper: the optional transcription add-on, never part of the exe. Transcription runs on
//! faster-whisper (MIT, github.com/SYSTRAN/faster-whisper: Whisper on CTranslate2, int8 on the CPU,
//! with Silero VAD) and its word times are then pinned to the audio the way WhisperX does it (BSD-2,
//! github.com/m-bain/whisperX): a wav2vec2 CTC model scores every 20 ms of audio per character and a
//! Viterbi pass finds where each word is spoken. whisper.cpp's `-ml 1` word times drift by up to a
//! second over pauses (why islands exist); these land on the spoken word.
//!
//! No Python is asked of the user. Install downloads, into `%APPDATA%\SimpleEditor\bin\faster-whisper`:
//! the embeddable CPython from python.org and the eleven wheels faster-whisper needs, straight from
//! PyPI (unzipped, no pip), plus the aligner and the picked model into the models cache. Every file is
//! pinned by URL, size and SHA-256 in `library/bin/faster-whisper.json`, which is compiled in (the
//! versions are the ones tested here). Licences, all permissive: CPython PSF-2.0; faster-whisper,
//! ctranslate2, onnxruntime, pyyaml MIT; numpy BSD-3 (+0BSD/MIT/Zlib/CC0 parts); tokenizers, flatbuffers
//! Apache-2.0; protobuf BSD-3; packaging Apache-2.0/BSD-2; colorama BSD-3; tqdm MPL-2.0 + MIT
//! (unmodified); the Systran/faster-whisper-* models MIT; wav2vec2-base-960h Apache-2.0. Two
//! packages faster-whisper imports are NOT installed, each replaced by a one-line stub: PyAV (it
//! bundles FFmpeg; the app decodes with its own ffmpeg and hands over a 16 kHz wav) and
//! huggingface_hub (downloads; the app fetches the models itself). Purfview's standalone
//! faster-whisper builds were not used: they carry no licence.
//!
//! The runner (`fw_runner.py`, compiled in and written next to python.exe) prints one JSON object
//! per line (`parse_line`); its result is one word per `Segment`, exactly what whisper.cpp's `-ml 1`
//! gives, so everything downstream (captions, cut words, fillers, subtitles) only sees better times.

use crate::engine::blitz;
use crate::engine::export::{self, Progress};
use crate::engine::transcribe::{self, Options, Segment};
use crate::media::ffpipe;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

/// The pinned download list (also in the online library, for reference).
pub const MANIFEST: &str = include_str!("../../library/bin/faster-whisper.json");
/// The runner script, written into the add-on on install (and refreshed before each run).
const RUNNER: &str = include_str!("fw_runner.py");
const RUNNER_NAME: &str = "se_fw.py";
/// The aligner's folder inside the models cache.
const ALIGN_DIR: &str = "wav2vec2-base-960h";

// --------------------------------------------------------------------------------- engine choice

/// Which program transcribes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Engine {
    /// whisper.cpp (`whisper-cli.exe`), built in.
    #[default]
    WhisperCpp,
    /// faster-whisper, the add-on.
    Faster,
}

impl Engine {
    pub const ALL: [Engine; 2] = [Engine::WhisperCpp, Engine::Faster];
    pub fn parse(s: &str) -> Option<Engine> {
        match s.trim().to_ascii_lowercase().as_str() {
            "whisper.cpp" | "whisper-cpp" | "whispercpp" | "cpp" | "whisper" => Some(Engine::WhisperCpp),
            "faster-whisper" | "faster_whisper" | "fasterwhisper" | "faster" | "fw" => Some(Engine::Faster),
            _ => None,
        }
    }
    /// The settings / MCP id.
    pub fn id(self) -> &'static str {
        match self {
            Engine::WhisperCpp => "whisper.cpp",
            Engine::Faster => "faster-whisper",
        }
    }
    /// The engine pickers' tooltip.
    pub fn tip(self) -> &'static str {
        match self {
            Engine::WhisperCpp => "Built in: whisper.cpp on the CPU",
            Engine::Faster => "The faster-whisper add-on: faster, with word times pinned to the audio",
        }
    }
}

/// The engine a run uses: `asked` (a tool argument), else `setting` (`Settings.transcribe_engine`; ""
/// = faster-whisper once it is installed). faster-whisper asked for but not installed falls back to
/// whisper.cpp, and the note says so.
pub fn resolve(asked: Option<Engine>, setting: &str, installed: bool) -> (Engine, Option<&'static str>) {
    let want = asked.or_else(|| Engine::parse(setting)).unwrap_or(if installed { Engine::Faster } else { Engine::WhisperCpp });
    match want {
        Engine::Faster if !installed => {
            (Engine::WhisperCpp, Some("faster-whisper is not installed (Settings ▸ Add-ons ▸ Transcription), so whisper.cpp ran"))
        }
        e => (e, None),
    }
}

// --------------------------------------------------------------------------------- manifest

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct Download {
    #[serde(default)]
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    #[serde(default)]
    pub license: String,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct RepoFile {
    pub path: String,
    pub save_as: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct Repo {
    pub repo: String,
    pub rev: String,
    pub files: Vec<RepoFile>,
    #[serde(default)]
    pub license: String,
}

impl Repo {
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
    fn url(&self, f: &RepoFile) -> String {
        format!("https://huggingface.co/{}/resolve/{}/{}", self.repo, self.rev, f.path)
    }
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct Manifest {
    pub version: String,
    pub python: Download,
    pub wheels: Vec<Download>,
    /// "tiny.en", "base.en", … (the whisper.cpp model names without `ggml-` / `.bin`).
    pub models: std::collections::BTreeMap<String, Repo>,
    pub align: Repo,
}

impl Manifest {
    /// Python + wheels, the download Install starts with.
    pub fn runtime_size(&self) -> u64 {
        self.python.size + self.wheels.iter().map(|w| w.size).sum::<u64>()
    }
}

pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let m: Manifest =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("faster-whisper.json: {e}"))?;
    let bad = |sha: &str, size: u64| sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) || size == 0;
    let bad_download =
        std::iter::once(&m.python).chain(&m.wheels).any(|d| bad(&d.sha256, d.size) || !d.url.starts_with("https://"));
    let bad_file = m
        .models
        .values()
        .chain([&m.align])
        .flat_map(|r| r.files.iter())
        .any(|f| bad(&f.sha256, f.size) || f.save_as.contains(['/', '\\']) || f.save_as.contains(".."));
    if bad_download || bad_file || m.models.is_empty() {
        return Err("faster-whisper.json is incomplete".into());
    }
    Ok(m)
}

pub fn manifest() -> &'static Manifest {
    static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
    M.get_or_init(|| parse_manifest(MANIFEST).expect("the compiled-in faster-whisper.json parses (tested)"))
}

// --------------------------------------------------------------------------------- the add-on at a glance

/// Everything an add-on list (Settings, an installer) needs about this add-on, in one call: where it
/// lives, whether it is installed and how big it is. Install with [`start_install`], remove with
/// [`uninstall`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct AddonInfo {
    pub id: &'static str,
    pub name: &'static str,
    /// The runtime folder (python, packages, runner); removing the add-on deletes it.
    pub dir: PathBuf,
    /// Model folders it also owns (the aligner and every `faster-whisper-<model>` folder).
    pub model_dirs: Vec<PathBuf>,
    pub installed: bool,
    pub version: Option<String>,
    /// Bytes on disk now (0 when not installed).
    pub bytes: u64,
    pub license: &'static str,
}

/// The one place that says where faster-whisper is installed and whether it is.
pub fn addon() -> AddonInfo {
    let installed = available();
    let mut model_dirs = vec![align_dir()];
    model_dirs.extend(manifest().models.keys().map(|k| model_dir(k)));
    AddonInfo {
        id: "faster-whisper",
        name: "faster-whisper",
        dir: dir(),
        model_dirs,
        installed,
        version: if installed { version() } else { None },
        bytes: if installed { disk_size() } else { 0 },
        license: "MIT (faster-whisper, CTranslate2, onnxruntime), PSF-2.0 (CPython), Apache-2.0 (wav2vec2)",
    }
}

// --------------------------------------------------------------------------------- where things are

/// `%APPDATA%\SimpleEditor\bin\faster-whisper`: python, its packages and the runner.
pub fn dir() -> PathBuf {
    blitz::bin_dir().join("faster-whisper")
}

fn python_exe() -> PathBuf {
    dir().join("python").join("python.exe")
}

/// The stamp written last by a finished install.
fn stamp() -> PathBuf {
    dir().join("installed.json")
}

/// Installed? Asked every frame (the Transcribe section, `transcript_ctl::tick`), so the two file
/// checks are cached for a couple of seconds; install and uninstall drop the cache.
pub fn available() -> bool {
    let mut slot = AVAILABLE.lock().unwrap_or_else(|e| e.into_inner());
    match *slot {
        Some((at, v)) if at.elapsed() < std::time::Duration::from_secs(2) => v,
        _ => {
            let v = stamp().is_file() && python_exe().is_file();
            *slot = Some((std::time::Instant::now(), v));
            v
        }
    }
}

static AVAILABLE: Mutex<Option<(std::time::Instant, bool)>> = Mutex::new(None);

struct Forget;
impl Drop for Forget {
    fn drop(&mut self) {
        forget_available();
    }
}

fn forget_available() {
    *AVAILABLE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The installed faster-whisper version ("1.2.1").
pub fn version() -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(stamp()).ok()?).ok()?;
    v["version"].as_str().map(str::to_string)
}

/// `ggml-base.en.bin` → "base.en" (the faster-whisper model with the same weights).
pub fn model_key(ggml_file: &str) -> &str {
    ggml_file.strip_prefix("ggml-").and_then(|f| f.strip_suffix(".bin")).unwrap_or(ggml_file)
}

pub fn model_dir(key: &str) -> PathBuf {
    transcribe::models_dir().join(format!("faster-whisper-{key}"))
}

pub fn align_dir() -> PathBuf {
    transcribe::models_dir().join(ALIGN_DIR)
}

/// Every file of `repo` is in `dir` at its published size.
fn have_repo(repo: &Repo, dir: &Path) -> bool {
    repo.files.iter().all(|f| std::fs::metadata(dir.join(&f.save_as)).map(|m| m.len() == f.size).unwrap_or(false))
}

/// The faster-whisper copy of a whisper.cpp model (by its ggml file name) is downloaded.
pub fn have_model(ggml_file: &str) -> bool {
    let key = model_key(ggml_file);
    manifest().models.get(key).is_some_and(|r| have_repo(r, &model_dir(key)))
}

/// MB to download for that model (0 = no faster-whisper copy of it).
pub fn model_mb(ggml_file: &str) -> u32 {
    manifest().models.get(model_key(ggml_file)).map_or(0, |r| (r.size() / 1_000_000) as u32)
}

pub fn have_align() -> bool {
    have_repo(&manifest().align, &align_dir())
}

fn walk(p: &Path) -> u64 {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::read_dir(p).map(|r| r.flatten().map(|e| walk(&e.path())).sum()).unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// Bytes the add-on takes on disk: its folder (measured once, at install: thousands of files), the
/// aligner and the faster-whisper models that are downloaded (a few stats each, cheap enough per frame).
pub fn disk_size() -> u64 {
    let stamp: Option<serde_json::Value> = std::fs::read_to_string(stamp()).ok().and_then(|t| serde_json::from_str(&t).ok());
    let runtime = stamp.and_then(|v| v["bytes"].as_u64()).unwrap_or(0);
    let m = manifest();
    let align = if have_align() { m.align.size() } else { 0 };
    let models: u64 = m.models.iter().filter(|(k, r)| have_repo(r, &model_dir(k))).map(|(_, r)| r.size()).sum();
    runtime + align + models
}

/// The Install button's size: runtime + aligner + the picked model, in MB.
pub fn install_mb(ggml_file: &str) -> u32 {
    let m = manifest();
    let model = if have_model(ggml_file) { 0 } else { m.models.get(model_key(ggml_file)).map_or(0, Repo::size) };
    ((m.runtime_size() + m.align.size() + model) / 1_000_000) as u32
}

// --------------------------------------------------------------------------------- downloads

/// curl `url` to `dest` (a `.part` beside it, renamed when its size and SHA-256 match). Progress is
/// the file's size against `size`, mapped into `range` (start, share). Cancellable.
fn fetch(url: &str, dest: &Path, size: u64, sha256: &str, range: (f32, f32), what: &str, prog: &Progress) -> Result<(), String> {
    if std::fs::metadata(dest).map(|m| m.len() == size).unwrap_or(false) && blitz::sha256_of(dest).ok().as_deref() == Some(sha256) {
        return Ok(()); // already here (a resumed install)
    }
    let curl = ffpipe::find_exe("curl.exe", "").ok_or("curl.exe not found (it ships with Windows 10 1803+)")?;
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let part = dest.with_extension(format!("{}.part", dest.extension().and_then(|e| e.to_str()).unwrap_or("")));
    let _ = std::fs::remove_file(&part);
    let mut child = ffpipe::command(&curl)
        .args(["-L", "--fail", "-sS", "--retry", "2", "-o"])
        .arg(&part)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    let tail = export::stderr_tail(&mut child);
    let status = loop {
        if prog.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&part);
            return Err(export::CANCELLED.into());
        }
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {}
            Err(e) => return Err(format!("curl: {e}")),
        }
        let got = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        let f = (got as f64 / size.max(1) as f64).min(1.0) as f32;
        prog.set(range.0 + range.1 * f, format!("Downloading {what}… {:.0} of {:.0} MB", got as f64 / 1e6, size as f64 / 1e6));
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    let msg = tail.and_then(|t| t.join().ok()).unwrap_or_default();
    let checked = if !status.success() {
        Err(if msg.trim().is_empty() { format!("downloading {what} failed ({status})") } else { msg.trim().to_string() })
    } else {
        let got = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if got != size {
            Err(format!("{what}: the download is {got} bytes, expected {size}"))
        } else if blitz::sha256_of(&part)? != sha256 {
            Err(format!("{what} does not match its checksum"))
        } else {
            std::fs::rename(&part, dest).map_err(|e| format!("{}: {e}", dest.display()))
        }
    };
    if checked.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    checked
}

/// Every file of `repo` into `dir`, progress spread over `range` by bytes.
fn fetch_repo(repo: &Repo, dir: &Path, range: (f32, f32), what: &str, prog: &Progress) -> Result<(), String> {
    let total = repo.size().max(1) as f32;
    let mut at = 0u64;
    for f in &repo.files {
        let r = (range.0 + range.1 * at as f32 / total, range.1 * f.size as f32 / total);
        fetch(&repo.url(f), &dir.join(&f.save_as), f.size, &f.sha256, r, what, prog)?;
        at += f.size;
    }
    Ok(())
}

/// Windows' own tar (bsdtar) reads zip (wheels are zips); git's GNU tar on PATH does not.
fn unzip(zip: &Path, into: &Path) -> Result<(), String> {
    let tar = std::env::var_os("SystemRoot")
        .map(|r| PathBuf::from(r).join("System32").join("tar.exe"))
        .filter(|p| p.is_file())
        .ok_or("tar.exe not found")?;
    std::fs::create_dir_all(into).map_err(|e| format!("{}: {e}", into.display()))?;
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
        Err(format!("could not unpack {}: {}", zip.display(), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// `python312._pth`: the embeddable Python's whole import path (it ignores PYTHONPATH and the user's
/// site-packages, so a Python the user installed can never leak in).
fn pth_text(zip_name: &str) -> String {
    format!("{zip_name}\n.\nsite-packages\nstubs\n")
}

/// The stand-ins for the two packages faster-whisper imports but the add-on does not install.
const STUBS: [(&str, &str); 2] = [
    ("av", "# PyAV is not installed: Simple Editor decodes with its own ffmpeg and passes 16 kHz samples.\ndef open(*a, **k):\n    raise RuntimeError(\"audio is decoded by the app\")\n"),
    ("huggingface_hub", "# huggingface_hub is not installed: Simple Editor downloads the models itself.\ndef snapshot_download(*a, **k):\n    raise RuntimeError(\"models are downloaded by the app\")\n"),
];

/// Write the runner next to python.exe (every run, so an app update brings its runner along).
fn write_runner(root: &Path) -> Result<PathBuf, String> {
    let p = root.join("python").join(RUNNER_NAME);
    if std::fs::read_to_string(&p).ok().as_deref() != Some(RUNNER) {
        std::fs::write(&p, RUNNER).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok(p)
}

/// Download, check and unpack Python and the wheels into a fresh folder, check faster-whisper imports,
/// swap it in as `dir()`, then fetch the aligner and the picked model. Blocking: a background job only,
/// started by a click (or `transcribe.install_addon`).
pub fn install(model_file: &str, prog: &Progress) -> Result<(), String> {
    let m = manifest();
    let bin = blitz::bin_dir();
    let dl = bin.join("faster-whisper.downloads");
    let fresh = bin.join("faster-whisper.new");
    std::fs::create_dir_all(&dl).map_err(|e| format!("{}: {e}", dl.display()))?;
    let _ = std::fs::remove_dir_all(&fresh);
    let model = m.models.get(model_key(model_file));
    let total = (m.runtime_size() + m.align.size() + model.map_or(0, Repo::size)).max(1) as f32;
    let share = |bytes: u64| 0.9 * bytes as f32 / total;
    let mut at = 0u64;
    let r = (|| -> Result<(), String> {
        if !available() {
            for d in std::iter::once(&m.python).chain(&m.wheels) {
                let file = dl.join(d.url.rsplit('/').next().unwrap_or("download.zip"));
                let what = if d.name.is_empty() { "Python" } else { d.name.as_str() };
                fetch(&d.url, &file, d.size, &d.sha256, (0.02 + share(at), share(d.size)), what, prog)?;
                at += d.size;
            }
            prog.set(0.02 + share(at), "Unpacking…");
            let py = fresh.join("python");
            unzip(&dl.join(m.python.url.rsplit('/').next().unwrap_or("")), &py)?;
            let site = py.join("site-packages");
            for w in &m.wheels {
                if prog.is_cancelled() {
                    return Err(export::CANCELLED.into());
                }
                unzip(&dl.join(w.url.rsplit('/').next().unwrap_or("")), &site)?;
            }
            for (name, text) in STUBS {
                let d = py.join("stubs").join(name);
                std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
                std::fs::write(d.join("__init__.py"), text).map_err(|e| e.to_string())?;
            }
            // the one ._pth the zip ships (python312._pth): replace its contents
            let pth = std::fs::read_dir(&py)
                .map_err(|e| e.to_string())?
                .flatten()
                .map(|e| e.path())
                .find(|p| p.extension().is_some_and(|e| e == "_pth"))
                .ok_or("the Python download has no ._pth file")?;
            let zip_name = pth.file_stem().unwrap_or_default().to_string_lossy().into_owned() + ".zip";
            std::fs::write(&pth, pth_text(&zip_name)).map_err(|e| e.to_string())?;
            prog.set(0.02 + share(at), "Checking it runs…");
            check_runs(&py.join("python.exe"))?;
            let dest = dir();
            let _ = std::fs::remove_dir_all(&dest);
            std::fs::rename(&fresh, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
            write_runner(&dest)?;
            let stamp_text = serde_json::json!({"version": m.version, "python": m.python.name, "bytes": walk(&dest)}).to_string();
            std::fs::write(stamp(), stamp_text).map_err(|e| e.to_string())?;
            forget_available();
        } else {
            at += m.runtime_size();
        }
        let _ = std::fs::remove_dir_all(&dl); // the zips are unpacked; resumes re-download only what is missing
        fetch_repo(&m.align, &align_dir(), (0.02 + share(at), share(m.align.size())), "the aligner", prog)?;
        at += m.align.size();
        if let Some(r) = model {
            let key = model_key(model_file);
            fetch_repo(r, &model_dir(key), (0.02 + share(at), share(r.size())), &format!("model {key}"), prog)?;
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&fresh);
    r?;
    prog.set(1.0, "faster-whisper installed");
    Ok(())
}

/// It starts and imports faster-whisper and onnxruntime (also compiles their .pyc once, so the first
/// transcription does not pay for it).
fn check_runs(python: &Path) -> Result<(), String> {
    let out = ffpipe::command(python)
        .args(["-c", "import faster_whisper, onnxruntime; print('se-fw', faster_whisper.__version__)"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Python does not start: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    if out.status.success() && text.contains("se-fw") {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("faster-whisper does not load: {}", err.lines().last().unwrap_or("").trim()))
    }
}

/// Download one more faster-whisper model (the add-on is installed, the user picked another model).
pub fn download_model(ggml_file: &str) -> Arc<Progress> {
    let file = ggml_file.to_string();
    export::spawn_job("faster-whisper-model", move |prog| {
        let key = model_key(&file);
        let r = manifest().models.get(key).ok_or_else(|| format!("faster-whisper has no {key} model"))?;
        fetch_repo(r, &model_dir(key), (0.0, 0.99), &format!("model {key}"), prog)?;
        prog.set(1.0, "Done");
        Ok(())
    })
}

/// Remove the add-on, the aligner and the faster-whisper models (transcripts already made stay).
pub fn uninstall() -> Result<(), String> {
    let _forget = Forget; // even when a folder will not go (a file in use), the next look is fresh
    for d in std::iter::once(dir()).chain([align_dir()]).chain(manifest().models.keys().map(|k| model_dir(k))) {
        if d.exists() {
            std::fs::remove_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
    }
    Ok(())
}

static INSTALLING: Mutex<Option<Arc<Progress>>> = Mutex::new(None);

/// The running install, if any.
pub fn installing() -> Option<Arc<Progress>> {
    INSTALLING.lock().unwrap_or_else(|e| e.into_inner()).clone().filter(|p| !p.is_done())
}

/// Why the last install failed (shown beside the Install button until the next try).
pub fn install_error() -> Option<String> {
    let slot = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    slot.as_ref().filter(|p| p.is_done() && !p.is_cancelled()).and_then(|p| p.error())
}

/// The click on "Install": a background job (the Jobs pane shows it), with the picked model. A second
/// click while one runs returns that one.
pub fn start_install(model_file: &str) -> Arc<Progress> {
    let mut slot = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = slot.as_ref().filter(|p| !p.is_done()) {
        return p.clone();
    }
    let file = model_file.to_string();
    let p = export::spawn_job("faster-whisper-install", move |prog| install(&file, prog));
    *slot = Some(p.clone());
    p
}

/// `transcribe.status`'s `addon` block.
pub fn status_json() -> serde_json::Value {
    let a = addon();
    serde_json::json!({
        "name": a.name,
        "installed": a.installed,
        "version": a.version,
        "aligner": have_align(),
        "models": manifest().models.keys().filter(|k| have_repo(&manifest().models[*k], &model_dir(k))).collect::<Vec<_>>(),
        "installing": installing().map(|p| serde_json::json!({"fraction": p.fraction(), "status": p.status()})),
        "install_error": install_error(),
        "disk_mb": a.bytes / 1_000_000,
        "license": a.license,
    })
}

// --------------------------------------------------------------------------------- the runner's output

/// One line of the runner's stdout.
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    Progress(f32, Option<String>),
    /// A transcribed segment, while it runs (seconds relative to `src_start`), with its progress.
    Seg(f64, f64, String, f32),
    /// The result: one word each, `aligned` of them pinned by the aligner.
    Words { words: Vec<(f64, f64, String)>, aligned: usize, language: String, device: String },
    Error(String),
}

/// A runner line → `Msg`; anything else (a library's stray print) is None.
pub fn parse_line(line: &str) -> Option<Msg> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let o = v.as_object()?;
    if let Some(e) = o.get("error").and_then(|e| e.as_str()) {
        return Some(Msg::Error(e.to_string()));
    }
    let p = o.get("p").and_then(|p| p.as_f64()).map(|p| p.clamp(0.0, 1.0) as f32);
    let triple = |w: &serde_json::Value| -> Option<(f64, f64, String)> {
        let a = w.get(0)?.as_f64()?;
        let b = w.get(1)?.as_f64()?;
        let t = w.get(2)?.as_str()?.trim();
        (!t.is_empty() && a.is_finite() && b.is_finite()).then(|| (a, b.max(a), t.to_string()))
    };
    if let Some(words) = o.get("words").and_then(|w| w.as_array()) {
        return Some(Msg::Words {
            words: words.iter().filter_map(triple).collect(),
            aligned: o.get("aligned").and_then(|a| a.as_u64()).unwrap_or(0) as usize,
            language: o.get("language").and_then(|l| l.as_str()).unwrap_or("").to_string(),
            device: o.get("device").and_then(|l| l.as_str()).unwrap_or("").to_string(),
        });
    }
    if let Some(s) = o.get("seg") {
        let (a, b, t) = triple(s)?;
        return Some(Msg::Seg(a, b, t, p.unwrap_or(0.0)));
    }
    p.map(|p| Msg::Progress(p, o.get("s").and_then(|s| s.as_str()).map(str::to_string)))
}

/// What the last run said about itself (for the toast / MCP reply).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunInfo {
    pub words: usize,
    pub aligned: usize,
    pub language: String,
    pub device: String,
    pub secs: f64,
}

// --------------------------------------------------------------------------------- running

/// A temp job file no other run is using.
fn temp_job() -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("se-fw-{}-{n}.json", std::process::id()))
}

/// Transcribe `opts` with faster-whisper (pinning the words when `opts.align`), pushing one word per
/// segment into `sink` (seconds relative to `opts.src_start`, as whisper.cpp's path does). Blocking:
/// the transcribe job's thread.
pub fn run(opts: &Options, prog: &Progress, sink: &Mutex<Vec<Segment>>, info: &Mutex<RunInfo>) -> Result<(), String> {
    let started = std::time::Instant::now();
    if !available() {
        return Err("faster-whisper is not installed".into());
    }
    let key = model_key(&opts.model).to_string();
    if !have_model(&opts.model) {
        return Err(format!("faster-whisper model {key} is not downloaded"));
    }
    let align = opts.align && have_align();
    let runner = write_runner(&dir())?;
    // the audio: one wav per span (the whole clip, or each speech island), all through ONE model load
    let spans: Vec<(f64, f64)> = if opts.islands.is_empty() {
        vec![(opts.src_start, opts.src_duration)]
    } else {
        opts.islands.iter().map(|&(a, b)| (a, b - a)).collect()
    };
    let mut wavs: Vec<PathBuf> = Vec::new();
    let cleanup = |w: &[PathBuf], job: Option<&Path>| {
        for p in w {
            let _ = std::fs::remove_file(p);
        }
        if let Some(j) = job {
            let _ = std::fs::remove_file(j);
        }
    };
    let mut list = Vec::new();
    for (i, &(start, dur)) in spans.iter().enumerate() {
        if prog.is_cancelled() {
            cleanup(&wavs, None);
            return Err(export::CANCELLED.into());
        }
        prog.set(0.01 * i as f32 / spans.len() as f32, "Extracting audio…");
        match transcribe::extract_wav(&opts.path, start, dur, prog) {
            Ok(w) => {
                list.push(serde_json::json!({"wav": w, "shift": start - opts.src_start}));
                wavs.push(w);
            }
            Err(_) if spans.len() > 1 && !prog.is_cancelled() => continue, // an island with no audio
            Err(e) => {
                cleanup(&wavs, None);
                return Err(e);
            }
        }
    }
    if list.is_empty() {
        return Err("that clip has no audio to transcribe".into());
    }
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
    let job = serde_json::json!({
        "wavs": list,
        "model_dir": model_dir(&key),
        "language": opts.language,
        "prompt": opts.prompt,
        "align": align,
        "align_dir": align_dir(),
        "threads": threads,
    });
    let job_path = temp_job();
    if let Err(e) = std::fs::write(&job_path, job.to_string()) {
        cleanup(&wavs, None);
        return Err(format!("{}: {e}", job_path.display()));
    }
    let r = run_python(&python_exe(), &runner, &job_path, prog, sink, info);
    cleanup(&wavs, Some(&job_path));
    let mut i = info.lock().unwrap_or_else(|e| e.into_inner());
    i.secs = started.elapsed().as_secs_f64();
    let n = r?;
    if n == 0 {
        return Err("nothing was transcribed (no speech found)".into());
    }
    let pinned = if i.aligned > 0 { format!(", {} pinned to the audio", i.aligned) } else { String::new() };
    prog.set(1.0, format!("{n} words{pinned}"));
    Ok(())
}

/// The runner over `job`: progress forwarded, the words pushed to `sink`. Returns how many.
fn run_python(
    python: &Path,
    runner: &Path,
    job: &Path,
    prog: &Progress,
    sink: &Mutex<Vec<Segment>>,
    info: &Mutex<RunInfo>,
) -> Result<usize, String> {
    let mut child = crate::media::bg::command(python)
        .arg("-I") // isolated: no user site-packages, no PYTHON* variables, no current-directory imports
        .arg(runner)
        .arg(job)
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", python.display()))?;
    let tail = export::stderr_tail(&mut child);
    let mut fatal: Option<String> = None;
    let mut n = 0usize;
    let mut status = String::from("Transcribing…");
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if prog.is_cancelled() {
                break; // wait_ffmpeg kills the child and reports CANCELLED
            }
            match parse_line(&line) {
                Some(Msg::Progress(p, s)) => {
                    if let Some(s) = s {
                        status = s + "…";
                    }
                    prog.set(p.clamp(0.01, 0.99), status.as_str());
                }
                Some(Msg::Seg(_, _, text, p)) => {
                    status = "Transcribing…".into();
                    prog.set(p.clamp(0.01, 0.99), format!("Transcribing… {}", transcribe::short_label(&text, 40)));
                }
                Some(Msg::Words { words, aligned, language, device }) => {
                    n = words.len();
                    let mut out = sink.lock().unwrap_or_else(|e| e.into_inner());
                    out.extend(words.into_iter().map(|(a, b, t)| Segment { start: a, end: b, text: t, words: Vec::new() }));
                    let mut i = info.lock().unwrap_or_else(|e| e.into_inner());
                    *i = RunInfo { words: n, aligned, language, device, secs: 0.0 };
                }
                Some(Msg::Error(e)) => fatal = Some(e),
                None => {}
            }
        }
    }
    let waited = export::wait_ffmpeg(&mut child, tail, prog);
    if let Some(e) = fatal {
        return Err(format!("faster-whisper: {e}"));
    }
    waited.map_err(|e| if e == export::CANCELLED { e } else { format!("faster-whisper: {e}") })?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_manifest_is_pinned_and_permissive() {
        let m = parse_manifest(MANIFEST).unwrap();
        assert!(m.python.url.starts_with("https://www.python.org/ftp/python/"));
        let names: Vec<&str> = m.wheels.iter().map(|w| w.name.as_str()).collect();
        for need in ["faster_whisper", "ctranslate2", "numpy", "tokenizers", "onnxruntime", "tqdm"] {
            assert!(names.contains(&need), "{need} missing from {names:?}");
        }
        // what the stubs stand in for is never downloaded
        assert!(!names.contains(&"av") && !names.contains(&"huggingface_hub"), "{names:?}");
        for w in &m.wheels {
            assert!(w.url.starts_with("https://files.pythonhosted.org/") && w.url.ends_with(".whl"), "{}", w.url);
            assert!(!w.license.to_ascii_uppercase().contains("GPL"), "{}: {}", w.name, w.license);
        }
        for key in ["tiny.en", "base.en", "small.en", "base"] {
            let r = &m.models[key];
            assert_eq!(r.repo, format!("Systran/faster-whisper-{key}"));
            assert_eq!(r.rev.len(), 40, "pinned to a commit");
            assert!(r.files.iter().any(|f| f.save_as == "model.bin"));
        }
        assert!(m.align.files.iter().any(|f| f.save_as == "model.onnx") && m.align.files.iter().any(|f| f.save_as == "vocab.json"));
        // every whisper.cpp model in the picker has a faster-whisper copy
        for (_, file, _) in transcribe::MODELS {
            assert!(m.models.contains_key(model_key(file)), "{file}");
        }
        // the runtime is about 66 MB
        assert!((50_000_000..90_000_000).contains(&m.runtime_size()), "{}", m.runtime_size());
    }

    #[test]
    fn a_tampered_manifest_is_refused() {
        let ok = MANIFEST.to_string();
        assert!(parse_manifest(&ok).is_ok());
        let sha = &manifest().python.sha256;
        assert!(parse_manifest(&ok.replacen(sha.as_str(), "abc", 1)).is_err());
        assert!(parse_manifest(&ok.replacen("\"save_as\": \"model.bin\"", "\"save_as\": \"..\\\\model.bin\"", 1)).is_err());
        assert!(parse_manifest(&ok.replacen("https://www.python.org", "http://www.python.org", 1)).is_err());
        assert!(parse_manifest("{}").is_err());
    }

    #[test]
    fn model_keys_follow_the_whisper_cpp_names() {
        assert_eq!(model_key("ggml-base.en.bin"), "base.en");
        assert_eq!(model_key("ggml-base.bin"), "base");
        assert_eq!(model_key("small.en"), "small.en");
        assert!(model_dir("tiny.en").ends_with("faster-whisper-tiny.en"));
        assert!(model_mb("ggml-small.en.bin") > model_mb("ggml-tiny.en.bin"));
    }

    #[test]
    fn engines_parse_and_fall_back_when_not_installed() {
        assert_eq!(Engine::parse("faster-whisper"), Some(Engine::Faster));
        assert_eq!(Engine::parse(" Whisper.CPP "), Some(Engine::WhisperCpp));
        assert_eq!(Engine::parse("nope"), None);
        for e in Engine::ALL {
            assert_eq!(Engine::parse(e.id()), Some(e));
        }
        // nothing set: faster-whisper once installed, whisper.cpp before
        assert_eq!(resolve(None, "", true), (Engine::Faster, None));
        assert_eq!(resolve(None, "", false), (Engine::WhisperCpp, None));
        // the setting picks
        assert_eq!(resolve(None, "whisper.cpp", true), (Engine::WhisperCpp, None));
        // an asked engine wins over the setting
        assert_eq!(resolve(Some(Engine::WhisperCpp), "faster-whisper", true).0, Engine::WhisperCpp);
        // asked for but missing: whisper.cpp, with a note
        let (e, note) = resolve(Some(Engine::Faster), "", false);
        assert_eq!(e, Engine::WhisperCpp);
        assert!(note.unwrap().contains("not installed"));
        assert_eq!(resolve(None, "faster-whisper", false).0, Engine::WhisperCpp);
    }

    #[test]
    fn parses_runner_lines() {
        assert_eq!(parse_line(r#"{"p": 0.01, "s": "Loading the model"}"#), Some(Msg::Progress(0.01, Some("Loading the model".into()))));
        assert_eq!(parse_line(r#"{"p": 1.7}"#), Some(Msg::Progress(1.0, None)));
        assert_eq!(parse_line(r#"{"seg": [1.5, 2.25, " Hello there. "], "p": 0.5}"#), Some(Msg::Seg(1.5, 2.25, "Hello there.".into(), 0.5)));
        let words = r#"{"words": [[0.8, 1.2, "Welcome"], [1.25, 1.1, "back"], [2.0, 2.1, " "], [null, 3, "x"], [3.0, 3.5]], "aligned": 2, "language": "en", "device": "cpu", "secs": 3.2}"#;
        assert_eq!(
            parse_line(words),
            Some(Msg::Words {
                // an end before its start is held at the start; blank and broken words are dropped
                words: vec![(0.8, 1.2, "Welcome".into()), (1.25, 1.25, "back".into())],
                aligned: 2,
                language: "en".into(),
                device: "cpu".into(),
            })
        );
        assert_eq!(parse_line(r#"{"error": "RuntimeError: boom"}"#), Some(Msg::Error("RuntimeError: boom".into())));
        // stray prints from a library are ignored
        assert_eq!(parse_line("Warning: something"), None);
        assert_eq!(parse_line("[1, 2]"), None);
        assert_eq!(parse_line(r#"{"other": 1}"#), None);
        assert_eq!(parse_line(r#"{"seg": [1, 2]}"#), None);
    }

    /// The runner the app writes out is the one compiled in, and it speaks the protocol `parse_line` reads.
    #[test]
    fn runner_script_speaks_the_protocol() {
        for key in ["\"p\"", "\"seg\"", "\"words\"", "\"aligned\"", "\"error\"", "\"wavs\"", "\"shift\""] {
            assert!(RUNNER.contains(key), "{key}");
        }
        assert!(RUNNER.contains("vad_filter=True") && RUNNER.contains("word_timestamps=True"));
        assert_eq!(pth_text("python312.zip"), "python312.zip\n.\nsite-packages\nstubs\n");
    }

    /// Word start error (seconds) of `got` against `truth`, over the words both have in the same order
    /// (an LCS over normalised words, so a misheard word is skipped, not mis-paired).
    fn start_errors(truth: &[(f64, String)], got: &[(f64, f64, String)]) -> Vec<f64> {
        let norm = |w: &str| transcribe::normalize(w).join("");
        let a: Vec<String> = truth.iter().map(|t| norm(&t.1)).collect();
        let b: Vec<String> = got.iter().map(|g| norm(&g.2)).collect();
        let (n, m) = (a.len(), b.len());
        let mut l = vec![vec![0u32; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                l[i][j] = if a[i] == b[j] && !a[i].is_empty() { l[i + 1][j + 1] + 1 } else { l[i + 1][j].max(l[i][j + 1]) };
            }
        }
        let (mut i, mut j, mut out) = (0, 0, Vec::new());
        while i < n && j < m {
            if a[i] == b[j] && !a[i].is_empty() {
                out.push((got[j].0 - truth[i].0).abs());
                i += 1;
                j += 1;
            } else if l[i + 1][j] >= l[i][j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
        out
    }

    #[test]
    fn start_errors_pair_words_in_order() {
        let truth = vec![(1.0, "Hello".to_string()), (2.0, "big".into()), (3.0, "world".into())];
        let got = vec![(1.1, 1.5, "hello,".to_string()), (2.5, 2.9, "pig".into()), (2.9, 3.3, "World.".into())];
        let e = start_errors(&truth, &got);
        assert_eq!(e.len(), 2);
        assert!((e[0] - 0.1).abs() < 1e-9 && (e[1] - 0.1).abs() < 1e-9, "{e:?}");
    }

    /// The real thing, on a speech sample made here (Windows' own voice, which reports where every word
    /// starts): installs the add-on through `install` when it is missing, then transcribes the sample
    /// with whisper.cpp (when it and ggml-base.en are there), faster-whisper, and faster-whisper with
    /// Precise word timing, and prints each one's word-start error and time. Run it in a scratch
    /// profile (APPDATA / LOCALAPPDATA redirected): it downloads ~590 MB into that profile.
    #[test]
    #[ignore = "real run: downloads the add-on (~590 MB) and runs whisper.cpp + faster-whisper"]
    fn real_run_word_times_against_whisper_cpp() {
        let dir = std::env::temp_dir().join(format!("se-fw-real-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (wav, truth_json) = (dir.join("speech.wav"), dir.join("truth.json"));
        let script = dir.join("say.ps1");
        std::fs::write(&script, SAY_PS1).unwrap();
        let out = ffpipe::command(Path::new("powershell.exe"))
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script)
            .arg(&wav)
            .arg(&truth_json)
            .output()
            .unwrap();
        assert!(out.status.success() && wav.is_file(), "{}", String::from_utf8_lossy(&out.stderr));
        let tj: serde_json::Value =
            serde_json::from_str(std::fs::read_to_string(&truth_json).unwrap().trim_start_matches('\u{feff}')).unwrap();
        let truth: Vec<(f64, String)> = tj
            .as_array()
            .unwrap()
            .iter()
            .map(|w| (w["t"].as_f64().unwrap(), w["text"].as_str().unwrap().to_string()))
            .collect();
        let dur = truth.last().unwrap().0 + 2.0;
        if !available() {
            install("ggml-base.en.bin", &Progress::new()).expect("install");
        } else if !have_model("ggml-base.en.bin") || !have_align() {
            install("ggml-base.en.bin", &Progress::new()).expect("models");
        }
        let run_one = |engine: Engine, align: bool| -> Option<(Vec<(f64, f64, String)>, f64, usize)> {
            if engine == Engine::WhisperCpp && (transcribe::exe().is_none() || !transcribe::have_model("ggml-base.en.bin")) {
                return None;
            }
            let job = transcribe::start(Options {
                path: wav.to_string_lossy().into_owned(),
                src_start: 0.0,
                src_duration: dur,
                model: "ggml-base.en.bin".into(),
                language: "en".into(),
                words: true,
                prompt: String::new(),
                islands: Vec::new(),
                engine,
                align,
            });
            let t = std::time::Instant::now();
            while !job.progress.is_done() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            assert_eq!(job.progress.error(), None, "{engine:?}");
            let words = job.segments().into_iter().map(|s| (s.start, s.end, s.text)).collect();
            Some((words, t.elapsed().as_secs_f64(), job.info().aligned))
        };
        let mut medians = Vec::new();
        for (engine, align, name) in [
            (Engine::WhisperCpp, false, "whisper.cpp base.en"),
            (Engine::Faster, false, "faster-whisper base.en"),
            (Engine::Faster, true, "faster-whisper base.en + precise timing"),
        ] {
            let Some((words, secs, aligned)) = run_one(engine, align) else {
                println!("{name}: not available here, skipped");
                continue;
            };
            let mut e = start_errors(&truth, &words);
            e.sort_by(f64::total_cmp);
            let pct = |x: f64| 100.0 * e.iter().filter(|&&v| v <= x).count() as f64 / e.len().max(1) as f64;
            let med = e.get(e.len() / 2).copied().unwrap_or(f64::NAN);
            println!(
                "{name}: {secs:.1} s for {dur:.1} s of audio ({:.1}x realtime), {} of {} words matched, {aligned} pinned; start error median {:.0} ms, mean {:.0} ms, p90 {:.0} ms, max {:.0} ms; within 50 ms {:.0}%, 100 ms {:.0}%, 250 ms {:.0}%",
                dur / secs,
                e.len(),
                truth.len(),
                med * 1000.0,
                1000.0 * e.iter().sum::<f64>() / e.len().max(1) as f64,
                e.get(e.len() * 9 / 10).copied().unwrap_or(f64::NAN) * 1000.0,
                e.last().copied().unwrap_or(f64::NAN) * 1000.0,
                pct(0.05),
                pct(0.1),
                pct(0.25),
            );
            assert!(e.len() * 10 >= truth.len() * 8, "{name} matched too few words");
            medians.push((engine, align, med));
        }
        let pinned = medians.iter().find(|m| m.0 == Engine::Faster && m.1).map(|m| m.2).unwrap();
        assert!(pinned < 0.15, "precise word timing should land within 150 ms (median), got {pinned}");
        if let Some(cpp) = medians.iter().find(|m| m.0 == Engine::WhisperCpp) {
            assert!(pinned < cpp.2, "precise timing ({pinned}) should beat whisper.cpp ({})", cpp.2);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Windows' voice reads a script with long pauses (where whisper.cpp's times drift) into a 16 kHz wav
    /// and writes where each word starts (SpeakProgress' audio position) as JSON.
    const SAY_PS1: &str = r#"param([string]$Out, [string]$Truth)
Add-Type -AssemblyName System.Speech
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
$fmt = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
$s.SetOutputToWaveFile($Out, $fmt)
$words = New-Object System.Collections.ArrayList
$s.add_SpeakProgress({ param($o, $e) [void]$words.Add(@{ t = $e.AudioPosition.TotalSeconds; text = $e.Text }) })
$lines = @(
  "Welcome back to the channel. Today we are testing how well captions line up with speech.",
  "This sentence comes after a long pause, which is where timings usually drift.",
  "Short words like a, to, and of are hard to place exactly.",
  "Now another long silence before the next part of the video.",
  "Numbers such as twenty four and one hundred should still land on the right frame.",
  "Finally, thank you for watching, and remember to subscribe for more editing tips."
)
$pauses = @("800ms", "3500ms", "1200ms", "4000ms", "1500ms", "500ms")
$ssml = '<speak version="1.0" xmlns="http://www.w3.org/2001/10/synthesis" xml:lang="en-US"><break time="700ms"/>'
for ($i = 0; $i -lt $lines.Count; $i++) { $ssml += $lines[$i] + '<break time="' + $pauses[$i] + '"/>' }
$ssml += '</speak>'
$s.SpeakSsml($ssml)
$s.Dispose()
$words | ConvertTo-Json -Compress | Set-Content -Encoding utf8 $Truth
"#;

    #[test]
    fn addon_info_names_its_folders() {
        let a = addon();
        assert_eq!(a.id, "faster-whisper");
        assert_eq!(a.dir, dir());
        assert!(a.model_dirs.contains(&align_dir()) && a.model_dirs.contains(&model_dir("base.en")));
        assert_eq!(a.installed, available());
        if !a.installed {
            assert_eq!((a.bytes, a.version), (0, None));
        }
    }

    #[test]
    fn not_installed_fails_without_spawning_anything() {
        if available() {
            return; // installed on this machine: the real run covers it
        }
        let opts = Options {
            path: "nope.mp4".into(),
            src_start: 0.0,
            src_duration: 1.0,
            model: "ggml-base.en.bin".into(),
            language: "en".into(),
            words: true,
            prompt: String::new(),
            islands: Vec::new(),
            engine: Engine::Faster,
            align: true,
        };
        let sink = Mutex::new(Vec::new());
        let info = Mutex::new(RunInfo::default());
        assert_eq!(run(&opts, &Progress::new(), &sink, &info).unwrap_err(), "faster-whisper is not installed");
        assert!(sink.lock().unwrap().is_empty());
    }
}
