//! The AI enhancer: an optional add-on, never part of the exe. Real-ESRGAN (`realesrgan-ncnn-vulkan`,
//! runs on the GPU through Vulkan) plus its models are one zip the user installs on a click, the same
//! way the se-html helper is installed (`blitz::install`): the library's `bin/enhancer.json` manifest
//! names the zip, its size and SHA-256; the zip is unpacked into `%APPDATA%\SimpleEditor\bin\enhancer`.
//!
//! `run` upscales only the source range a clip shows (plus handles): ffmpeg pulls the frames out, the
//! upscaler works through them in chunks (a chunk's big PNGs are piped into the encoder and deleted, so
//! disk use stays at one chunk), ffmpeg encodes the result with the source's audio. Every child process
//! runs below normal priority so playback and the UI never wait on it.

use crate::engine::blitz;
use crate::engine::export::{self, Progress};
use crate::media::ffpipe;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const EXE: &str = "realesrgan-ncnn-vulkan.exe";
/// The library file that names the download.
pub const MANIFEST: &str = "bin/enhancer.json";
/// Frames per upscaler run: bounds the temporary PNGs on disk (a 4K PNG is ~20 MB).
const CHUNK: usize = 24;
/// A clip zoomed past this gets the upscale offered by name in its menu (the zoom is where it shows).
pub const OFFER_ZOOM: f64 = 1.2;

/// `%APPDATA%\SimpleEditor\bin\enhancer` - the unpacked add-on.
pub fn dir() -> PathBuf {
    blitz::bin_dir().join("enhancer")
}

/// The upscaler: `SE_ENHANCER` (a path; tests and development), else the installed copy. None = not installed.
pub fn exe() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SE_ENHANCER").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    Some(dir().join(EXE)).filter(|p| p.is_file())
}

pub fn available() -> bool {
    exe().is_some()
}

/// The installed manifest's version ("20220424"), when installed through the app.
pub fn version() -> Option<String> {
    let text = std::fs::read_to_string(dir().join("enhancer.json")).ok()?;
    parse_manifest(&text).ok().map(|m| m.version)
}

/// A child with no console window, at normal priority: the upscaler is GPU work that waits on the GPU
/// per tile, and below normal it starves whenever anything else keeps the CPU busy (97 frames took
/// ~45 min that way, 5 s per 24 frames at normal). `Pri` drops it below normal only while playing.
fn child(exe: &Path) -> Command {
    crate::media::ffpipe::command(exe)
}

/// Keeps a child's priority class in step with playback: below normal while the timeline or the
/// Source monitor plays (`media::bg::playing`), normal otherwise. Call `sync` from every wait loop.
#[derive(Default)]
struct Pri(Option<bool>);

impl Pri {
    fn sync(&mut self, child: &std::process::Child) {
        self.sync_to(child, crate::media::bg::playing());
    }
    /// Only calls into Windows when the wanted class changes.
    fn sync_to(&mut self, child: &std::process::Child, low: bool) {
        if self.0 != Some(low) && set_low(child, low) {
            self.0 = Some(low);
        }
    }
}

fn set_low(child: &std::process::Child, low: bool) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::System::Threading::{SetPriorityClass, BELOW_NORMAL_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS};
    let class = if low { BELOW_NORMAL_PRIORITY_CLASS } else { NORMAL_PRIORITY_CLASS };
    // SAFETY: the handle belongs to `child`, which outlives this call; a failure leaves the class as it was.
    unsafe { SetPriorityClass(windows::Win32::Foundation::HANDLE(child.as_raw_handle()), class).is_ok() }
}

/// `export::wait_ffmpeg` with the priority kept in step with playback while it waits.
fn wait_child(
    child: &mut std::process::Child,
    tail: Option<std::thread::JoinHandle<String>>,
    pri: &mut Pri,
    prog: &Progress,
) -> Result<(), String> {
    while !prog.is_cancelled() {
        pri.sync(child);
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(30)),
            Err(e) => return Err(e.to_string()),
        }
    }
    export::wait_ffmpeg(child, tail, prog) // exited (or cancelled: it kills): status + stderr tail
}

// ---------------------------------------------------------------------------------------------------
// install / uninstall

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub version: String,
    /// The zip: a path relative to the library (a folder source) or a full address.
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let v: serde_json::Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("enhancer.json: {e}"))?;
    let s = |k: &str| v[k].as_str().unwrap_or("").trim().to_string();
    let m = Manifest {
        version: s("version"),
        url: s("url"),
        sha256: s("sha256").to_ascii_lowercase(),
        size: v["size"].as_u64().unwrap_or(0),
    };
    if m.url.is_empty() || m.sha256.len() != 64 || m.size == 0 {
        return Err("enhancer.json is incomplete".into());
    }
    Ok(m)
}

/// Where the exe sits in an unpacked zip: its root, or one folder down (zips often wrap one folder).
fn find_exe(root: &Path) -> Option<PathBuf> {
    let top = root.join(EXE);
    if top.is_file() {
        return Some(root.to_path_buf());
    }
    std::fs::read_dir(root).ok()?.flatten().map(|e| e.path()).find(|p| p.join(EXE).is_file())
}

/// Download (or copy, from a folder library) the zip, check size and SHA-256, unpack it, check the
/// upscaler runs, then swap it in as `dir()`. Blocking: a background job only, started by a click.
pub fn install(prog: &Progress) -> Result<PathBuf, String> {
    use crate::engine::library::Source;
    let src = Source::of(&crate::settings::Settings::load());
    prog.set(0.02, "Looking up the AI enhancer…");
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
    let part = bin.join("enhancer.zip.part");
    let fresh = bin.join("enhancer.new");
    let clean = || {
        let _ = std::fs::remove_file(&part);
        let _ = std::fs::remove_dir_all(&fresh);
    };
    clean();
    prog.set(0.1, format!("Downloading the AI enhancer ({:.0} MB)…", m.size as f64 / 1048576.0));
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
        prog.set(0.8, "Checking it…");
        let size = std::fs::metadata(&part).map(|x| x.len()).unwrap_or(0);
        if size != m.size {
            return Err(format!("the download is {size} bytes, expected {}", m.size));
        }
        if blitz::sha256_of(&part)? != m.sha256 {
            return Err("the download does not match its checksum".into());
        }
        prog.set(0.88, "Unpacking…");
        std::fs::create_dir_all(&fresh).map_err(|e| e.to_string())?;
        // Windows' own tar (bsdtar) reads zip; git's GNU tar on PATH does not, so name it exactly
        let tar = std::env::var_os("SystemRoot")
            .map(|r| PathBuf::from(r).join("System32").join("tar.exe"))
            .filter(|p| p.is_file())
            .ok_or("tar.exe not found")?;
        let out = ffpipe::command(&tar)
            .arg("-xf")
            .arg(&part)
            .arg("-C")
            .arg(&fresh)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("tar: {e}"))?;
        if !out.status.success() {
            return Err(format!("could not unpack: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        let root = find_exe(&fresh).ok_or("the download has no realesrgan-ncnn-vulkan.exe")?;
        check_runs(&root.join(EXE))?;
        // the demo video and sample pictures in the upstream zip are not needed
        for junk in ["onepiece_demo.mp4", "input.jpg", "input2.jpg"] {
            let _ = std::fs::remove_file(root.join(junk));
        }
        let dest = dir();
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&root, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        std::fs::write(dest.join("enhancer.json"), &text).map_err(|e| e.to_string())?;
        Ok(dest.join(EXE))
    });
    clean();
    let exe = r?;
    prog.set(1.0, "AI enhancer installed");
    Ok(exe)
}

/// It starts and is the upscaler (`-h` prints its usage; it exits non-zero after that).
fn check_runs(exe: &Path) -> Result<(), String> {
    let out = ffpipe::command(exe)
        .arg("-h")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("the upscaler does not start: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if text.contains("realesrgan-ncnn-vulkan") {
        Ok(())
    } else {
        Err("the installed file is not the Real-ESRGAN upscaler".into())
    }
}

/// Remove the add-on (the upscaled files it made stay where they are).
pub fn uninstall() -> Result<(), String> {
    let d = dir();
    if d.exists() {
        std::fs::remove_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    Ok(())
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
/// click or the `enhance.install` tool. A second click while one runs returns that one.
pub fn start_install() -> std::sync::Arc<Progress> {
    let mut slot = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = slot.as_ref().filter(|p| !p.is_done()) {
        return p.clone();
    }
    let p = export::spawn_job("enhancer-install", |prog| install(prog).map(|_| ()));
    *slot = Some(p.clone());
    p
}

// ---------------------------------------------------------------------------------------------------
// upscaling

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    /// `realesr-animevideov3`: game and animation footage, fast, native ×2/×3/×4.
    Game,
    /// `realesrgan-x4plus`: real-world video; ×4 only (×2 is ×4 scaled down).
    Real,
}

impl Model {
    pub fn parse(s: &str) -> Option<Model> {
        match s.trim().to_ascii_lowercase().as_str() {
            "game" | "anime" | "animation" | "realesr-animevideov3" => Some(Model::Game),
            "real" | "photo" | "realesrgan-x4plus" => Some(Model::Real),
            _ => None,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Model::Game => "game",
            Model::Real => "real",
        }
    }
    /// The upscaler's model name and the scale it runs at for an asked `scale`.
    fn run_as(self, scale: u32) -> (&'static str, u32) {
        match self {
            Model::Game => ("realesr-animevideov3", scale),
            Model::Real => ("realesrgan-x4plus", 4),
        }
    }
}

/// One upscale: `range` of `src` (source seconds) at `scale`, written to `out` (mp4; png for a still).
#[derive(Clone, Debug)]
pub struct Request {
    pub src: PathBuf,
    pub still: bool,
    pub range: (f64, f64),
    pub fps: f64,
    pub scale: u32,
    pub model: Model,
    pub out: PathBuf,
}

/// The source seconds a clip shows, widened by `handle` each side (room for transitions and small
/// trims) and snapped to frames: `(src_in, src_in + duration·speed)`, or the one held frame of a
/// freeze. ponytail: a keyframed speed ramp can read past the constant window, so it takes the rest of
/// the source; walk the ramp if that ever upscales too much.
pub fn source_range(clip: &crate::model::Clip, asset_duration: f64, fps: f64, handle: f64) -> (f64, f64) {
    let fps = if fps > 0.0 { fps } else { 30.0 };
    let (lo, hi) = match clip.freeze {
        Some(t) => (t, t + 1.0 / fps),
        None if !clip.speed_curve.keys.is_empty() => (clip.src_in, asset_duration),
        None => (clip.src_in, clip.src_in + clip.duration * clip.speed),
    };
    let a = (((lo - handle) * fps + 1e-6).floor() / fps).max(0.0); // 4.5·25 is 112.4999…
    let b = if asset_duration > 0.0 { (hi + handle).min(asset_duration) } else { hi + handle };
    (a, b.max(a + 1.0 / fps))
}

/// `name_x2_game.mp4` beside the source.
pub fn output_name(src: &Path, scale: u32, model: Model, still: bool) -> String {
    let stem = src.file_stem().unwrap_or_default().to_string_lossy();
    format!("{stem}_x{scale}_{}.{}", model.id(), if still { "png" } else { "mp4" })
}

/// Frame files in `dir`, by name (they are numbered).
fn frames_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Run one child to the end (killed on cancel); Err carries its stderr tail.
fn run_child(mut cmd: Command, what: &str, prog: &Progress) -> Result<(), String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("{what}: {e}"))?;
    let tail = export::stderr_tail(&mut child);
    wait_child(&mut child, tail, &mut Pri::default(), prog).map_err(|e| {
        if e == export::CANCELLED {
            e
        } else {
            format!("{what}: {e}")
        }
    })
}

/// Upscale `req` into `req.out`. Blocking; a background job (`export::spawn_job`) only.
pub fn run(req: &Request, prog: &Progress) -> Result<(), String> {
    let up = exe().ok_or("The AI enhancer is not installed")?;
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let parent = req.out.parent().ok_or("no output folder")?;
    let tmp = parent.join(format!(".{}.enhance-tmp", req.out.file_stem().unwrap_or_default().to_string_lossy()));
    let _ = std::fs::remove_dir_all(&tmp);
    let (frames, chunk, done) = (tmp.join("in"), tmp.join("chunk"), tmp.join("up"));
    for d in [&frames, &chunk, &done] {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let r = upscale(req, &up, &ffmpeg, &frames, &chunk, &done, prog);
    let _ = std::fs::remove_dir_all(&tmp);
    if r.is_err() {
        let _ = std::fs::remove_file(&req.out);
    }
    r
}

fn upscale(
    req: &Request,
    up: &Path,
    ffmpeg: &Path,
    frames: &Path,
    chunk: &Path,
    done: &Path,
    prog: &Progress,
) -> Result<(), String> {
    let (model, native) = req.model.run_as(req.scale);
    let mut encoder_name = String::from("libx264");
    let shrink = native != req.scale; // x4plus asked for ×2
    let len = req.range.1 - req.range.0;
    prog.set(0.01, "Reading the frames…");
    crate::engine::encoders::start(); // the hardware-encoder probe (cached; never blocks) for the encode below
    let mut x = child(ffmpeg);
    x.args(["-v", "error", "-y"]);
    if req.still {
        x.arg("-i").arg(&req.src).args(["-frames:v", "1"]).arg(frames.join("00000001.png"));
    } else {
        // ponytail: high-quality JPEG frames keep a long clip's temporary files small on a full disk;
        // PNG here if the upscaler ever visibly sharpens JPEG blocking
        x.args(["-ss", &format!("{:.6}", req.range.0), "-t", &format!("{len:.6}")]).arg("-i").arg(&req.src);
        x.args(["-vf", &format!("fps={}", req.fps), "-q:v", "2"]).arg(frames.join("%08d.jpg"));
    }
    run_child(x, "ffmpeg", prog)?;
    let all = frames_in(frames);
    if all.is_empty() {
        return Err("no frames in that range of the source".into());
    }
    let n = all.len();

    // the encoder: PNGs on stdin, the source's audio for the same range alongside
    let mut enc = child(ffmpeg);
    enc.args(["-v", "error", "-y"]);
    if req.still {
        enc.args(["-f", "image2pipe", "-c:v", "png", "-i", "-"]);
        if shrink {
            enc.args(["-vf", "scale=iw/2:ih/2:flags=lanczos"]);
        }
        enc.args(["-frames:v", "1"]);
    } else {
        enc.args(["-f", "image2pipe", "-c:v", "png", "-framerate", &req.fps.to_string(), "-i", "-"]);
        enc.args(["-ss", &format!("{:.6}", req.range.0), "-t", &format!("{len:.6}")]).arg("-i").arg(&req.src);
        enc.args(["-map", "0:v:0", "-map", "1:a?"]);
        if shrink {
            enc.args(["-vf", "scale=iw/2:ih/2:flags=lanczos"]);
        }
        // NVENC / QSV / AMF when the probe found one (the frames already came off the GPU), else libx264;
        // CRF 16 = the export's "visually lossless". ponytail: a hardware encoder that dies mid-run fails
        // this job and is marked broken, so the next run uses libx264; retrying in place would mean
        // re-upscaling every frame.
        // the probe (started above, cached after the first run) usually finishes during the frame read;
        // on a first run give it up to 30 s rather than settle for libx264
        let wait = std::time::Instant::now();
        while !crate::engine::encoders::done() && wait.elapsed() < std::time::Duration::from_secs(30) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let hw = crate::engine::encoders::best_for("mp4");
        encoder_name = hw.clone().unwrap_or_else(|| "libx264".into());
        enc.args(export::codec_args("mp4", hw.as_deref().unwrap_or("libx264"), 16, "medium", &[]));
        enc.args(["-shortest"]);
    }
    enc.arg(&req.out).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut encoder = enc.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
    let mut enc_pri = Pri::default();
    let tail = export::stderr_tail(&mut encoder);
    let mut stdin = encoder.stdin.take();

    let fed = (|| -> Result<(), String> {
        for (i, group) in all.chunks(CHUNK).enumerate() {
            if prog.is_cancelled() {
                return Err(export::CANCELLED.into());
            }
            for f in group {
                std::fs::rename(f, chunk.join(f.file_name().unwrap_or_default())).map_err(|e| e.to_string())?;
            }
            let at = i * CHUNK;
            prog.set(0.05 + 0.9 * at as f32 / n as f32, format!("Upscaling frame {} of {n} ({encoder_name})…", at + 1));
            enc_pri.sync(&encoder);
            let mut cmd = child(up);
            cmd.arg("-i").arg(chunk).arg("-o").arg(done).args(["-n", model, "-s", &native.to_string(), "-f", "png"]);
            // the models sit beside the exe; -m makes that independent of the working directory
            if let Some(models) = up.parent().map(|p| p.join("models")).filter(|p| p.is_dir()) {
                cmd.arg("-m").arg(models);
            }
            run_child(cmd, "upscaler", prog)?;
            let outs = frames_in(done);
            if outs.len() != group.len() {
                return Err(format!(
                    "the upscaler wrote {} of {} frames (does this GPU support Vulkan? an up-to-date graphics driver usually fixes it)",
                    outs.len(),
                    group.len()
                ));
            }
            for o in outs {
                let bytes = std::fs::read(&o).map_err(|e| e.to_string())?;
                stdin.as_mut().ok_or("encoder closed")?.write_all(&bytes).map_err(|e| format!("encoder: {e}"))?;
                let _ = std::fs::remove_file(&o);
            }
            for f in frames_in(chunk) {
                let _ = std::fs::remove_file(f);
            }
        }
        Ok(())
    })();
    drop(stdin.take()); // EOF: the encoder finishes the file
    if let Err(e) = fed {
        let _ = encoder.kill();
        let _ = encoder.wait();
        return Err(e);
    }
    prog.set(0.97, "Encoding…");
    wait_child(&mut encoder, tail, &mut enc_pri, prog).map_err(|e| {
        if e == export::CANCELLED {
            e
        } else {
            if encoder_name != "libx264" {
                crate::engine::encoders::mark_broken(&encoder_name);
            }
            format!("encoder ({encoder_name}): {e}")
        }
    })?;
    prog.set(1.0, "Upscaled");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipKind, Keyframe};

    #[test]
    fn manifest_must_be_complete() {
        let ok = format!(
            r#"{{"version":"20220424","url":"https://x/y.zip","sha256":"{}","size":45474481}}"#,
            "A".repeat(64)
        );
        let m = parse_manifest(&ok).unwrap();
        assert_eq!((m.size, m.sha256, m.version.as_str()), (45474481, "a".repeat(64), "20220424"));
        assert!(parse_manifest(&ok.replace("45474481", "0")).is_err());
        assert!(parse_manifest(&ok.replace(&"A".repeat(64), "abc")).is_err());
        assert!(parse_manifest(&ok.replace("https://x/y.zip", "")).is_err());
        assert!(parse_manifest("not json").is_err());
        // the shipped manifest parses
        let shipped = include_str!("../../library/bin/enhancer.json");
        assert!(parse_manifest(shipped).unwrap().url.starts_with("https://"));
    }

    #[test]
    fn range_is_the_shown_window_plus_handles_snapped_to_frames() {
        let mut c = Clip::new(1, ClipKind::Video, "c", 10.0, 2.0);
        c.src_in = 5.0;
        // 4.5 s is not on a 25 fps frame: snapped down to 4.48
        assert_eq!(source_range(&c, 60.0, 25.0, 0.5), (4.48, 7.5));
        // twice the speed reads twice the source
        c.speed = 2.0;
        assert_eq!(source_range(&c, 60.0, 25.0, 0.0), (5.0, 9.0));
        // clamped to the source
        c.src_in = 0.1;
        assert_eq!(source_range(&c, 3.0, 25.0, 1.0), (0.0, 3.0));
        // start snapped down to a frame boundary
        c.src_in = 1.01;
        c.speed = 1.0;
        let (a, _) = source_range(&c, 60.0, 10.0, 0.0);
        assert!((a - 1.0).abs() < 1e-9, "{a}");
        // a freeze needs one frame
        c.freeze = Some(4.0);
        assert_eq!(source_range(&c, 60.0, 10.0, 0.0), (4.0, 4.1));
        // a speed ramp: the rest of the source
        c.freeze = None;
        c.src_in = 2.0;
        c.speed_curve.keys.push(Keyframe { t: 0.0, v: 2.0, ease: Default::default() });
        assert_eq!(source_range(&c, 30.0, 10.0, 0.0), (2.0, 30.0));
    }

    #[test]
    fn models_and_names() {
        assert_eq!(Model::parse("Anime"), Some(Model::Game));
        assert_eq!(Model::parse("real"), Some(Model::Real));
        assert_eq!(Model::parse("x"), None);
        assert_eq!(Model::Real.run_as(2), ("realesrgan-x4plus", 4));
        assert_eq!(Model::Game.run_as(2), ("realesr-animevideov3", 2));
        assert_eq!(output_name(Path::new(r"C:\v\shot.mov"), 2, Model::Game, false), "shot_x2_game.mp4");
        assert_eq!(output_name(Path::new("a.jpg"), 4, Model::Real, true), "a_x4_real.png");
    }

    #[test]
    fn not_installed_fails_without_spawning_anything() {
        if available() {
            return; // installed on this machine: the real path is covered by the end-to-end run
        }
        let req = Request {
            src: "nope.mp4".into(),
            still: false,
            range: (0.0, 1.0),
            fps: 30.0,
            scale: 2,
            model: Model::Game,
            out: std::env::temp_dir().join("se-enh-none.mp4"),
        };
        assert_eq!(run(&req, &Progress::new()).unwrap_err(), "The AI enhancer is not installed");
        assert!(!req.out.exists());
    }

    #[test]
    fn a_zip_wrapping_one_folder_is_found() {
        let root = std::env::temp_dir().join(format!("se-enh-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("wrapped")).unwrap();
        assert_eq!(find_exe(&root), None);
        std::fs::write(root.join("wrapped").join(EXE), b"x").unwrap();
        assert_eq!(find_exe(&root), Some(root.join("wrapped")));
        std::fs::write(root.join(EXE), b"x").unwrap();
        assert_eq!(find_exe(&root), Some(root.clone()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Children start at normal priority, drop below normal while playing and come back after.
    #[test]
    fn children_run_normal_and_drop_below_normal_only_while_playing() {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::System::Threading::{GetPriorityClass, BELOW_NORMAL_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS};
        let class = |c: &std::process::Child| unsafe {
            GetPriorityClass(windows::Win32::Foundation::HANDLE(c.as_raw_handle()))
        };
        let mut c = child(Path::new("ping.exe")).args(["-n", "30", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap();
        assert_eq!(class(&c), NORMAL_PRIORITY_CLASS.0, "starts at normal, not below");
        let mut pri = Pri::default();
        pri.sync_to(&c, false);
        assert_eq!(class(&c), NORMAL_PRIORITY_CLASS.0);
        pri.sync_to(&c, true); // the timeline / Source monitor started playing
        assert_eq!(class(&c), BELOW_NORMAL_PRIORITY_CLASS.0);
        assert_eq!(pri.0, Some(true));
        pri.sync_to(&c, false); // stopped
        assert_eq!(class(&c), NORMAL_PRIORITY_CLASS.0);
        let _ = c.kill();
        let _ = c.wait();
    }

    /// The encode picks a hardware H.264 encoder when the probe found one, else libx264; both get CRF-16-class
    /// quality (NVENC's CQ sits 6 above the CRF it matches) and AAC audio from the export's own argument builder.
    #[test]
    fn encode_args_use_the_hardware_encoder_with_libx264_fallback() {
        let pick = |w: &[&str]| {
            let w: Vec<String> = w.iter().map(|s| s.to_string()).collect();
            crate::engine::encoders::pick("mp4", &w).unwrap_or("libx264").to_string()
        };
        assert_eq!(pick(&["h264_nvenc", "h264_qsv"]), "h264_nvenc");
        assert_eq!(pick(&[]), "libx264");
        let nv = export::codec_args("mp4", "h264_nvenc", 16, "medium", &[]).join(" ");
        assert!(nv.contains("-c:v h264_nvenc") && nv.contains("-cq 22") && nv.contains("-c:a aac"), "{nv}");
        let sw = export::codec_args("mp4", "libx264", 16, "medium", &[]).join(" ");
        assert!(sw.contains("-c:v libx264") && sw.contains("-crf 16") && sw.contains("+faststart"), "{sw}");
    }

    #[test]
    fn a_fake_upscaler_is_refused() {
        let d = std::env::temp_dir().join(format!("se-enh-fake-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let fake = d.join(EXE);
        std::fs::write(&fake, b"not a program").unwrap();
        assert!(check_runs(&fake).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
