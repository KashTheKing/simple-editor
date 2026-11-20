//! Stem separation: an optional add-on, never part of the exe. Demucs v4 (`htdemucs`, 4 sources) through
//! demucs.cpp, a Python-free C++ port (MIT, Eigen only), built for Windows by us; the weights are the
//! upstream htdemucs model converted by demucs.cpp's script (MIT, as Demucs itself). Installed on a click
//! the way the AI enhancer is (`enhance::install`): the library's `bin/demucs.json` names the program zip and
//! the weights file with their sizes and SHA-256; both land in `%APPDATA%\SimpleEditor\bin\demucs`.
//!
//! `run` separates a whole audio stream of a file once: ffmpeg writes it as 44.1 kHz stereo WAV (all the
//! model takes), demucs.cpp splits it into drums / bass / other / vocals, and ffmpeg turns each back into
//! 16-bit WAV (half the float size; plain PCM decodes for free on the playback path). The four files are
//! cached under `Settings::cache_dir()/stems/<key>` - never beside the user's media - where `key` is the
//! source's path, size, modified time and stream, so a second Separate is instant and an edited file is
//! separated again. The child runs below normal priority with half the cores, so playback never waits.

use crate::engine::blitz;
use crate::engine::export::{self, Progress};
use crate::media::ffpipe;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The AVX2 + FMA build (x86-64-v3): the one the install checks and most PCs run.
pub const EXE: &str = "demucs_mt.cpp.main.exe";
/// The SSE4.2 build (x86-64-v2) beside it, for processors without AVX2 (`tools/demucs-helper/build.sh`).
pub const EXE_V2: &str = "demucs_mt.cpp.main.v2.exe";
pub const MODEL: &str = "ggml-model-htdemucs-4s-f16.bin";
/// The library file that names the download.
pub const MANIFEST: &str = "bin/demucs.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stem {
    Drums,
    Bass,
    Other,
    Vocals,
}

impl Stem {
    /// In demucs.cpp's output order (`target_<i>_<name>.wav`).
    pub const ALL: [Stem; 4] = [Stem::Drums, Stem::Bass, Stem::Other, Stem::Vocals];
    pub fn id(self) -> &'static str {
        match self {
            Stem::Drums => "drums",
            Stem::Bass => "bass",
            Stem::Other => "other",
            Stem::Vocals => "vocals",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Stem::Drums => "Drums",
            Stem::Bass => "Bass",
            Stem::Other => "Other",
            Stem::Vocals => "Vocals",
        }
    }
    pub fn parse(s: &str) -> Option<Stem> {
        Stem::ALL.into_iter().find(|x| x.id().eq_ignore_ascii_case(s.trim()))
    }
}

/// `%APPDATA%\SimpleEditor\bin\demucs` - the unpacked add-on. THE install folder (installers and
/// add-on listings: `dir()` for where, `available()` for whether, `version()` for which).
pub fn dir() -> PathBuf {
    blitz::bin_dir().join("demucs")
}

/// The separator: `SE_DEMUCS` (a path; tests and development), else the installed copy. None = not installed.
pub fn exe() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SE_DEMUCS").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let d = dir();
    pick_exe(&d, has_avx2()).filter(|p| p.is_file())
}

/// The processor runs the AVX2 + FMA build.
fn has_avx2() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::is_x86_feature_detected!("avx2") && std::is_x86_feature_detected!("fma")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// Which build in `d`: the AVX2 one when the processor has it, else the SSE4.2 one when the zip carried it
/// (an older zip has only the one exe: use it as before).
fn pick_exe(d: &Path, avx2: bool) -> Option<PathBuf> {
    let (fast, compat) = (d.join(EXE), d.join(EXE_V2));
    if !fast.is_file() {
        return None; // the install puts EXE there; without it nothing is installed
    }
    Some(if !avx2 && compat.is_file() { compat } else { fast })
}

/// The weights: `SE_DEMUCS_MODEL`, else beside the installed exe.
pub fn model() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SE_DEMUCS_MODEL").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    Some(dir().join(MODEL)).filter(|p| p.is_file())
}

/// THE detection: the separator and its weights are both present.
pub fn available() -> bool {
    exe().is_some() && model().is_some()
}

/// The installed manifest's version, when installed through the app.
pub fn version() -> Option<String> {
    let text = std::fs::read_to_string(dir().join("demucs.json")).ok()?;
    parse_manifest(&text).ok().map(|m| m.version)
}

// ---------------------------------------------------------------------------------------------------
// install / uninstall

#[derive(Clone, Debug, PartialEq)]
pub struct Download {
    /// A path relative to the library (a folder source) or a full address.
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub version: String,
    /// The program zip (demucs_mt.cpp.main.exe + licences).
    pub program: Download,
    /// The weights file.
    pub model: Download,
}

pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let v: serde_json::Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("demucs.json: {e}"))?;
    let dl = |o: &serde_json::Value| -> Result<Download, String> {
        let s = |k: &str| o[k].as_str().unwrap_or("").trim().to_string();
        let d = Download { url: s("url"), sha256: s("sha256").to_ascii_lowercase(), size: o["size"].as_u64().unwrap_or(0) };
        if d.url.is_empty() || d.sha256.len() != 64 || d.size == 0 {
            return Err("demucs.json is incomplete".into());
        }
        Ok(d)
    };
    Ok(Manifest {
        version: v["version"].as_str().unwrap_or("").trim().to_string(),
        program: dl(&v["program"])?,
        model: dl(&v["model"])?,
    })
}

/// Fetch one download to `dest` (copied from a folder library, else curl) and check its size and SHA-256.
fn fetch(src: &crate::engine::library::Source, d: &Download, dest: &Path, prog: &Progress) -> Result<(), String> {
    use crate::engine::library::Source;
    let _ = std::fs::remove_file(dest);
    match src {
        Source::Dir(dir) if !d.url.contains("://") => std::fs::copy(dir.join(&d.url), dest)
            .map(|_| ())
            .map_err(|e| format!("{}: {e}", dir.join(&d.url).display()))?,
        _ => {
            let url = if d.url.contains("://") { d.url.clone() } else { src.url(&d.url).ok_or("no library address")? };
            blitz::curl(&url, dest, 3600)?
        }
    }
    if prog.is_cancelled() {
        return Err(export::CANCELLED.into());
    }
    let size = std::fs::metadata(dest).map(|x| x.len()).unwrap_or(0);
    if size != d.size {
        return Err(format!("the download is {size} bytes, expected {}", d.size));
    }
    if blitz::sha256_of(dest)? != d.sha256 {
        return Err("the download does not match its checksum".into());
    }
    Ok(())
}

/// Where the exe sits in an unpacked zip: its root, or one folder down.
fn find_exe(root: &Path) -> Option<PathBuf> {
    if root.join(EXE).is_file() {
        return Some(root.to_path_buf());
    }
    std::fs::read_dir(root).ok()?.flatten().map(|e| e.path()).find(|p| p.join(EXE).is_file())
}

/// Download the program zip and the weights, check both, unpack, check the program starts, then swap it
/// in as `dir()`. Blocking: a background job only, started by a click or `audio.stems install`.
pub fn install(prog: &Progress) -> Result<PathBuf, String> {
    use crate::engine::library::Source;
    let src = Source::of(&crate::settings::Settings::load());
    prog.set(0.02, "Looking up Stem Separation…");
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
    let (zip, weights, fresh) = (bin.join("demucs.zip.part"), bin.join("demucs-model.part"), bin.join("demucs.new"));
    let clean = || {
        let _ = std::fs::remove_file(&zip);
        let _ = std::fs::remove_file(&weights);
        let _ = std::fs::remove_dir_all(&fresh);
    };
    clean();
    let r = (|| {
        prog.set(0.05, format!("Downloading Stem Separation ({:.0} MB)…", m.program.size as f64 / 1048576.0));
        fetch(&src, &m.program, &zip, prog)?;
        prog.set(0.2, format!("Downloading the htdemucs model ({:.0} MB)…", m.model.size as f64 / 1048576.0));
        fetch(&src, &m.model, &weights, prog)?;
        prog.set(0.9, "Unpacking…");
        std::fs::create_dir_all(&fresh).map_err(|e| e.to_string())?;
        // Windows' own tar (bsdtar) reads zip; git's GNU tar on PATH does not, so name it exactly
        let tar = std::env::var_os("SystemRoot")
            .map(|r| PathBuf::from(r).join("System32").join("tar.exe"))
            .filter(|p| p.is_file())
            .ok_or("tar.exe not found")?;
        let out = ffpipe::command(&tar)
            .arg("-xf")
            .arg(&zip)
            .arg("-C")
            .arg(&fresh)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("tar: {e}"))?;
        if !out.status.success() {
            return Err(format!("could not unpack: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        let root = find_exe(&fresh).ok_or(format!("the download has no {EXE}"))?;
        check_runs(&pick_exe(&root, has_avx2()).unwrap_or_else(|| root.join(EXE)))?;
        std::fs::rename(&weights, root.join(MODEL)).map_err(|e| e.to_string())?;
        let dest = dir();
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&root, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        std::fs::write(dest.join("demucs.json"), &text).map_err(|e| e.to_string())?;
        Ok(dest.join(EXE))
    })();
    clean();
    let exe = r?;
    prog.set(1.0, "Stem Separation installed");
    Ok(exe)
}

/// It starts and is demucs.cpp (no arguments prints its usage and exits 1).
fn check_runs(exe: &Path) -> Result<(), String> {
    let out = ffpipe::command(exe)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("the separator does not start: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if text.contains("<model file> <wav file> <out dir> <num threads>") {
        Ok(())
    } else {
        Err("the installed file is not the demucs.cpp separator".into())
    }
}

/// Remove the add-on (separated stems stay in the cache).
pub fn uninstall() -> Result<(), String> {
    let d = dir();
    if d.exists() {
        std::fs::remove_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    Ok(())
}

static INSTALLING: std::sync::Mutex<Option<std::sync::Arc<Progress>>> = std::sync::Mutex::new(None);

pub fn installing() -> Option<std::sync::Arc<Progress>> {
    INSTALLING.lock().unwrap_or_else(|e| e.into_inner()).clone().filter(|p| !p.is_done())
}

/// Why the last install failed (shown beside the Install button until the next try).
pub fn install_error() -> Option<String> {
    let slot = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    slot.as_ref().filter(|p| p.is_done() && !p.is_cancelled()).and_then(|p| p.error())
}

/// The click on "Install": a background job (the Jobs pane shows it). A second click returns the running one.
pub fn start_install() -> std::sync::Arc<Progress> {
    let mut slot = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = slot.as_ref().filter(|p| !p.is_done()) {
        return p.clone();
    }
    let p = export::spawn_job("demucs-install", |prog| install(prog).map(|_| ()));
    *slot = Some(p.clone());
    p
}

// ---------------------------------------------------------------------------------------------------
// the cache

/// `cache_dir()/stems`.
pub fn cache_root() -> PathBuf {
    crate::settings::Settings::cache_dir().join("stems")
}

/// The source's identity: path (case-folded), size, modified time and audio stream, hashed (FNV-1a).
pub fn key(src: &Path, stream: usize) -> String {
    let meta = std::fs::metadata(src).ok();
    let size = meta.as_ref().map_or(0, |m| m.len());
    let mtime = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let text = format!("{}|{size}|{mtime}|{stream}", src.to_string_lossy().to_lowercase());
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// The folder a source's stems go in.
pub fn stems_dir(src: &Path, stream: usize) -> PathBuf {
    cache_root().join(key(src, stream))
}

/// `<dir>/vocals.wav`.
pub fn stem_file(dir: &Path, s: Stem) -> PathBuf {
    dir.join(format!("{}.wav", s.id()))
}

/// The four stems of `src`'s audio stream, when they are already separated.
pub fn cached(src: &Path, stream: usize) -> Option<PathBuf> {
    let d = stems_dir(src, stream);
    Stem::ALL.iter().all(|&s| stem_file(&d, s).is_file()).then_some(d)
}

// ---------------------------------------------------------------------------------------------------
// separating

/// Half the cores (at least 1, at most 8): each thread holds its own segment's activations (~1 GB), and
/// the other half stays free for playback and the UI.
pub fn threads() -> usize {
    let n = std::thread::available_parallelism().map_or(4, |n| n.get());
    (n / 2).clamp(1, 8)
}

/// `[THREAD 2] ( 41.250%) …` → (2, 0.4125).
fn parse_progress(line: &str) -> Option<(usize, f32)> {
    let rest = line.split("[THREAD ").nth(1)?;
    let (n, rest) = rest.split_once(']')?;
    let pct = rest.trim_start().strip_prefix('(')?.split_once("%)")?.0.trim();
    Some((n.trim().parse().ok()?, pct.parse::<f32>().ok()? / 100.0))
}

/// The arguments of the ffmpeg run that writes the model's input: one audio stream, 44.1 kHz stereo.
fn input_args(src: &Path, stream: usize, out: &Path) -> Vec<String> {
    let mut v: Vec<String> = ["-v", "error", "-y", "-i"].map(String::from).to_vec();
    v.push(src.to_string_lossy().into_owned());
    v.extend(["-map".into(), format!("0:a:{stream}"), "-vn".into()]);
    v.extend(["-ac", "2", "-ar", "44100", "-c:a", "pcm_s16le"].map(String::from));
    v.push(out.to_string_lossy().into_owned());
    v
}

/// The separator's arguments, run in the work folder: `<model> in.wav raw <threads>`.
fn separator_args(weights: &Path, threads: usize) -> Vec<std::ffi::OsString> {
    vec![weights.as_os_str().to_owned(), "in.wav".into(), "raw".into(), threads.to_string().into()]
}

/// One child to the end, killed on cancel; Err carries its stderr tail.
fn run_child(mut cmd: Command, what: &str, prog: &Progress) -> Result<(), String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("{what}: {e}"))?;
    let tail = export::stderr_tail(&mut child);
    export::wait_ffmpeg(&mut child, tail, prog).map_err(|e| if e == export::CANCELLED { e } else { format!("{what}: {e}") })
}

/// A child at below-normal priority with no console window.
fn low(exe: &Path) -> Command {
    use std::os::windows::process::CommandExt;
    let mut c = Command::new(exe);
    c.creation_flags(0x0800_0000 | 0x0000_4000); // CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS
    c
}

/// Separate `src`'s audio stream `stream` into the cache (or return the cached stems at once). Returns the
/// stems' folder. Blocking: a background job (`export::spawn_job`) only.
pub fn run(src: &Path, stream: usize, prog: &Progress) -> Result<PathBuf, String> {
    if let Some(d) = cached(src, stream) {
        prog.set(1.0, "Stems ready");
        return Ok(d);
    }
    let (Some(sep), Some(weights)) = (exe(), model()) else {
        return Err("Stem Separation is not installed".into());
    };
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    if !src.is_file() {
        return Err(format!("{} is offline", src.display()));
    }
    let dest = stems_dir(src, stream);
    let work = dest.with_extension("part");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let r = separate(src, stream, &sep, &weights, &ffmpeg, &work, prog).and_then(|_| {
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&work, &dest).map_err(|e| format!("{}: {e}", dest.display()))
    });
    let _ = std::fs::remove_dir_all(&work);
    r?;
    prog.set(1.0, "Stems ready");
    Ok(dest)
}

fn separate(
    src: &Path,
    stream: usize,
    sep: &Path,
    weights: &Path,
    ffmpeg: &Path,
    work: &Path,
    prog: &Progress,
) -> Result<(), String> {
    prog.set(0.01, "Reading the audio…");
    let input = work.join("in.wav");
    let mut x = low(ffmpeg);
    x.args(input_args(src, stream, &input));
    run_child(x, "ffmpeg", prog)?;

    let n = threads();
    let raw = work.join("raw");
    let mut cmd = low(sep);
    // the WAVs by name inside the work folder: demucs.cpp opens files through narrow (ANSI) paths, so a
    // cache folder under a non-ASCII user name stays out of its arguments (its UTF-8 manifest covers the model)
    cmd.current_dir(work).args(separator_args(weights, n));
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("separator: {e}"))?;
    let tail = export::stderr_tail(&mut child);
    let out = child.stdout.take();
    let shared = std::sync::Arc::new(std::sync::Mutex::new(vec![0f32; n]));
    let reader = out.map(|o| {
        let shared = shared.clone();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(o).lines().map_while(Result::ok) {
                if let Some((i, f)) = parse_progress(&line) {
                    let mut v = shared.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(x) = v.get_mut(i) {
                        *x = x.max(f);
                    }
                }
            }
        })
    });
    let status = loop {
        if prog.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(export::CANCELLED.into());
        }
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {
                let v = shared.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let f = v.iter().sum::<f32>() / n as f32;
                prog.set(0.03 + 0.9 * f, format!("Separating stems {:.0}%…", f * 100.0));
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Err(e) => return Err(format!("separator: {e}")),
        }
    };
    if let Some(r) = reader {
        let _ = r.join();
    }
    let msg = tail.and_then(|t| t.join().ok()).unwrap_or_default();
    if !status.success() {
        return Err(format!("separator: {}", if msg.is_empty() { status.to_string() } else { msg }));
    }
    let _ = std::fs::remove_file(&input);
    prog.set(0.94, "Writing the stems…");
    for (i, s) in Stem::ALL.into_iter().enumerate() {
        let f = raw.join(format!("target_{i}_{}.wav", s.id()));
        if !f.is_file() {
            return Err(format!("the separator wrote no {} stem", s.id()));
        }
        let mut c = low(ffmpeg);
        c.args(["-v", "error", "-y", "-i"]).arg(&f).args(["-c:a", "pcm_s16le"]).arg(stem_file(work, s));
        run_child(c, "ffmpeg", prog)?;
        let _ = std::fs::remove_file(&f);
    }
    let _ = std::fs::remove_dir_all(&raw);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_must_be_complete() {
        let ok = format!(
            r#"{{"version":"1","program":{{"url":"https://x/a.zip","sha256":"{a}","size":10}},"model":{{"url":"m.bin","sha256":"{a}","size":20}}}}"#,
            a = "A".repeat(64)
        );
        let m = parse_manifest(&ok).unwrap();
        assert_eq!((m.program.size, m.model.size, m.model.sha256), (10, 20, "a".repeat(64)));
        assert!(parse_manifest(&ok.replace("\"size\":20", "\"size\":0")).is_err());
        assert!(parse_manifest(&ok.replace("m.bin", "")).is_err());
        assert!(parse_manifest("{}").is_err());
        let shipped = include_str!("../../library/bin/demucs.json");
        let m = parse_manifest(shipped).unwrap();
        assert!(m.program.url.starts_with("https://") && m.model.url.starts_with("https://"));
    }

    #[test]
    fn progress_lines_parse() {
        assert_eq!(parse_progress("[THREAD 2] ( 41.250%) Encoder 3"), Some((2, 0.4125)));
        assert_eq!(parse_progress("[THREAD 0] (100.000%) done"), Some((0, 1.0)));
        assert_eq!(parse_progress("Writing wav file"), None);
        assert_eq!(parse_progress("[THREAD x] (1%)"), None);
    }

    #[test]
    fn input_is_one_stream_at_44k1_stereo() {
        let a = input_args(Path::new(r"C:\m\song.mp4"), 1, Path::new(r"G:\t\in.wav")).join(" ");
        assert!(a.contains("-map 0:a:1") && a.contains("-ar 44100") && a.contains("-ac 2"), "{a}");
        assert!(a.contains("pcm_s16le") && a.ends_with(r"G:\t\in.wav") && a.contains("-vn"), "{a}");
    }

    #[test]
    fn cache_key_follows_the_file_and_stream_and_lives_in_the_cache() {
        let f = std::env::temp_dir().join(format!("se-demucs-key-{}.wav", std::process::id()));
        std::fs::write(&f, b"one").unwrap();
        let k = key(&f, 0);
        assert_eq!(k, key(&f, 0));
        assert_ne!(k, key(&f, 1));
        std::fs::write(&f, b"longer").unwrap(); // a different size is a different source
        assert_ne!(k, key(&f, 0));
        assert!(stems_dir(&f, 0).starts_with(crate::settings::Settings::cache_dir()));
        assert_eq!(stem_file(Path::new("d"), Stem::Vocals), Path::new("d").join("vocals.wav"));
        assert_eq!(cached(&f, 0), None);
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn the_separator_runs_on_names_inside_the_work_folder() {
        let a = separator_args(Path::new(r"C:\Users\Zoë\AppData\Roaming\SimpleEditor\bin\demucs\m.bin"), 3);
        assert_eq!(a[1..], ["in.wav", "raw", "3"].map(std::ffi::OsString::from));
        assert!(a[0].to_string_lossy().ends_with("m.bin"));
    }

    #[test]
    fn the_sse_build_is_picked_only_without_avx2() {
        let d = std::env::temp_dir().join(format!("se-demucs-pick-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(pick_exe(&d, true), None);
        std::fs::write(d.join(EXE), b"x").unwrap();
        // an older zip with one exe: that one either way
        assert_eq!(pick_exe(&d, false), Some(d.join(EXE)));
        std::fs::write(d.join(EXE_V2), b"x").unwrap();
        assert_eq!(pick_exe(&d, true), Some(d.join(EXE)));
        assert_eq!(pick_exe(&d, false), Some(d.join(EXE_V2)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn stems_parse_and_keep_the_separator_order() {
        assert_eq!(Stem::parse("Vocals"), Some(Stem::Vocals));
        assert_eq!(Stem::parse("piano"), None);
        let ids: Vec<_> = Stem::ALL.iter().map(|s| s.id()).collect();
        assert_eq!(ids, ["drums", "bass", "other", "vocals"]);
    }

    #[test]
    fn not_installed_fails_without_spawning_anything() {
        if available() {
            return; // installed here: the real path is covered by the ignored end-to-end run
        }
        let f = std::env::temp_dir().join(format!("se-demucs-none-{}.wav", std::process::id()));
        std::fs::write(&f, b"x").unwrap();
        assert_eq!(run(&f, 0, &Progress::new()).unwrap_err(), "Stem Separation is not installed");
        assert!(!stems_dir(&f, 0).with_extension("part").exists());
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn a_fake_separator_is_refused() {
        let d = std::env::temp_dir().join(format!("se-demucs-fake-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let fake = d.join(EXE);
        std::fs::write(&fake, b"not a program").unwrap();
        assert!(check_runs(&fake).is_err());
        assert_eq!(find_exe(&d), Some(d.clone()));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The real thing on a generated 10 s mix (a tone, a kick and a noise "voice"): set SE_DEMUCS and
    /// SE_DEMUCS_MODEL (or install the add-on). `cargo test demucs_real_run -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn demucs_real_run_on_a_generated_mix() {
        assert!(available(), "set SE_DEMUCS + SE_DEMUCS_MODEL or install the add-on");
        let ffmpeg = ffpipe::ffmpeg_exe().expect("ffmpeg");
        let tmp = std::env::var_os("SE_TMP").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let mix = tmp.join(format!("se-demucs-mix-{}.wav", std::process::id()));
        let st = ffpipe::command(&ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=110:duration=10"])
            .args(["-f", "lavfi", "-i", "aevalsrc=sin(2*PI*60*t)*exp(-30*mod(t\\,0.5)):d=10"])
            .args(["-f", "lavfi", "-i", "anoisesrc=d=10:c=pink:a=0.1"])
            .args(["-filter_complex", "amix=inputs=3:normalize=0", "-ar", "48000", "-ac", "2"])
            .arg(&mix)
            .status()
            .unwrap();
        assert!(st.success());
        let t = std::time::Instant::now();
        let d = run(&mix, 0, &Progress::new()).unwrap();
        eprintln!("separated 10 s in {:.1} s with {} threads", t.elapsed().as_secs_f64(), threads());
        for s in Stem::ALL {
            let f = stem_file(&d, s);
            let a = ffpipe::probe(&f.to_string_lossy()).unwrap();
            assert!((a.duration - 10.0).abs() < 0.1, "{s:?}: {}", a.duration);
        }
        assert_eq!(cached(&mix, 0), Some(d.clone()));
        // a second run is the cache
        let t = std::time::Instant::now();
        run(&mix, 0, &Progress::new()).unwrap();
        assert!(t.elapsed().as_secs_f64() < 1.0);
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_file(&mix);
    }
}
