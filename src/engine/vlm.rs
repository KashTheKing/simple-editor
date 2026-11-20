//! ---- feat/styles-context ----
//! The video describer: an optional add-on, never part of the exe, installed on a click like the AI enhancer
//! (`enhance.rs`). llama.cpp's `llama-mtmd-cli` (MIT, CPU build, no Python) runs SmolVLM2 GGUF weights
//! (Apache-2.0) with their vision projector (`mmproj`). The 256M Video Instruct model is the default; the
//! 2.2B is an optional upgrade. `library/bin/vlm.json` (compiled in) names each download, its size and
//! SHA-256; everything lands in `%APPDATA%\SimpleEditor\bin\vlm`:
//! `llama\` (the unpacked runtime), `<model id>\model.gguf` + `mmproj.gguf`.
//!
//! It only describes pictures (framing, subject, camera, on-screen text). Every number a learned style
//! carries comes from our own measurements (`style_learn`).

use crate::engine::blitz;
use crate::engine::export::{self, Progress};
use crate::media::ffpipe;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

pub const EXE: &str = "llama-mtmd-cli.exe";
pub const MANIFEST: &str = include_str!("../../library/bin/vlm.json");
/// The default model id; `MODELS[1]` is the upgrade.
pub const DEFAULT: &str = "256m";

#[derive(Clone, Debug, PartialEq)]
pub struct Download {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub model: Download,
    pub mmproj: Download,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub version: String,
    pub runtime: Download,
    pub models: Vec<ModelInfo>,
}

fn download(v: &Value) -> Result<Download, String> {
    let d = Download {
        url: v["url"].as_str().unwrap_or("").to_string(),
        sha256: v["sha256"].as_str().unwrap_or("").to_ascii_lowercase(),
        size: v["size"].as_u64().unwrap_or(0),
    };
    if d.url.is_empty() || d.sha256.len() != 64 || d.size == 0 {
        return Err("vlm.json is incomplete".into());
    }
    Ok(d)
}

pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("vlm.json: {e}"))?;
    let models = v["models"]
        .as_array()
        .ok_or("vlm.json has no models")?
        .iter()
        .map(|m| {
            Ok(ModelInfo {
                id: m["id"].as_str().unwrap_or("").to_string(),
                name: m["name"].as_str().unwrap_or("").to_string(),
                model: download(&m["model"])?,
                mmproj: download(&m["mmproj"])?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Manifest { version: v["runtime"]["version"].as_str().unwrap_or("").to_string(), runtime: download(&v["runtime"])?, models })
}

pub fn manifest() -> Manifest {
    parse_manifest(MANIFEST).expect("the compiled-in vlm.json parses")
}

pub fn model_info(id: &str) -> Option<ModelInfo> {
    manifest().models.into_iter().find(|m| m.id == id)
}

/// `%APPDATA%\SimpleEditor\bin\vlm`, or `SE_VLM_DIR` (tests, development).
pub fn dir() -> PathBuf {
    std::env::var_os("SE_VLM_DIR").map(PathBuf::from).unwrap_or_else(|| blitz::bin_dir().join("vlm"))
}

pub fn exe() -> Option<PathBuf> {
    Some(dir().join("llama").join(EXE)).filter(|p| p.is_file())
}

fn model_files(id: &str) -> (PathBuf, PathBuf) {
    let d = dir().join(id);
    (d.join("model.gguf"), d.join("mmproj.gguf"))
}

pub fn has_model(id: &str) -> bool {
    let (m, p) = model_files(id);
    m.is_file() && p.is_file()
}

/// Installed models, best first (the 2.2B when both are there).
pub fn installed_models() -> Vec<String> {
    let mut v: Vec<String> = manifest().models.iter().map(|m| m.id.clone()).filter(|id| has_model(id)).collect();
    v.reverse();
    v
}

/// The describer is ready: the runtime plus at least one model.
pub fn available() -> bool {
    exe().is_some() && !installed_models().is_empty()
}

/// The download size of installing `id` (the runtime too when it is not there yet), bytes.
pub fn install_size(id: &str) -> u64 {
    let m = manifest();
    let model = m.models.iter().find(|x| x.id == id).map_or(0, |x| x.model.size + x.mmproj.size);
    model + if exe().is_some() { 0 } else { m.runtime.size }
}

fn fetch(d: &Download, dest: &Path, prog: &Progress, at: f32, what: &str) -> Result<(), String> {
    if dest.is_file() && std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0) == d.size {
        return Ok(()); // already here (a retry after a later step failed)
    }
    let part = dest.with_extension("part");
    let _ = std::fs::remove_file(&part);
    prog.set(at, format!("Downloading {what} ({:.0} MB)…", d.size as f64 / 1048576.0));
    blitz::curl(&d.url, &part, 3600)?;
    if prog.is_cancelled() {
        let _ = std::fs::remove_file(&part);
        return Err(export::CANCELLED.into());
    }
    let size = std::fs::metadata(&part).map(|x| x.len()).unwrap_or(0);
    let r = if size != d.size {
        Err(format!("{what}: the download is {size} bytes, expected {}", d.size))
    } else if blitz::sha256_of(&part)? != d.sha256 {
        Err(format!("{what}: the download does not match its checksum"))
    } else {
        std::fs::rename(&part, dest).map_err(|e| format!("{}: {e}", dest.display()))
    };
    if r.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    r
}

/// Download and check the runtime (when missing) and model `id`, unpack, check it runs. Blocking: a
/// background job only, started by a click or `styles.install_describer`.
pub fn install(id: &str, prog: &Progress) -> Result<(), String> {
    let m = manifest();
    let info = m.models.iter().find(|x| x.id == id).ok_or(format!("no describer model '{id}'"))?;
    let root = dir();
    std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    if exe().is_none() {
        let zip = root.join("llama.zip");
        fetch(&m.runtime, &zip, prog, 0.02, "llama.cpp")?;
        prog.set(0.12, "Unpacking…");
        let fresh = root.join("llama.new");
        let _ = std::fs::remove_dir_all(&fresh);
        std::fs::create_dir_all(&fresh).map_err(|e| e.to_string())?;
        let tar = std::env::var_os("SystemRoot")
            .map(|r| PathBuf::from(r).join("System32").join("tar.exe"))
            .filter(|p| p.is_file())
            .ok_or("tar.exe not found")?;
        let out = ffpipe::command(&tar).arg("-xf").arg(&zip).arg("-C").arg(&fresh).stdin(Stdio::null()).output();
        let out = out.map_err(|e| format!("tar: {e}"))?;
        if !out.status.success() {
            return Err(format!("could not unpack: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        let inner = if fresh.join(EXE).is_file() {
            fresh.clone()
        } else {
            std::fs::read_dir(&fresh).ok().into_iter().flatten().flatten().map(|e| e.path()).find(|p| p.join(EXE).is_file()).ok_or("the download has no llama-mtmd-cli.exe")?
        };
        check_runs(&inner.join(EXE))?;
        let dest = root.join("llama");
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&inner, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        let _ = std::fs::remove_dir_all(&fresh);
        let _ = std::fs::remove_file(&zip);
    }
    let d = root.join(id);
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    fetch(&info.mmproj, &d.join("mmproj.gguf"), prog, 0.2, "the vision projector")?;
    fetch(&info.model, &d.join("model.gguf"), prog, 0.45, &info.name)?;
    let _ = std::fs::write(root.join("vlm.json"), MANIFEST);
    prog.set(1.0, "Video describer installed");
    Ok(())
}

/// It starts and is llama.cpp (`--version` prints "version: <build>").
fn check_runs(exe: &Path) -> Result<(), String> {
    let out = ffpipe::command(exe).arg("--version").stdin(Stdio::null()).output().map_err(|e| format!("llama-mtmd-cli does not start: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if text.contains("version") {
        Ok(())
    } else {
        Err("the installed file is not llama.cpp".into())
    }
}

/// Remove the add-on (runtime and every model).
pub fn uninstall() -> Result<(), String> {
    let d = dir();
    if d.exists() {
        std::fs::remove_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    Ok(())
}

static INSTALLING: Mutex<Option<Arc<Progress>>> = Mutex::new(None);

pub fn installing() -> Option<Arc<Progress>> {
    INSTALLING.lock().unwrap_or_else(|e| e.into_inner()).clone().filter(|p| !p.is_done())
}

/// Why the last install failed (until the next try).
pub fn install_error() -> Option<String> {
    let slot = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    slot.as_ref().filter(|p| p.is_done() && !p.is_cancelled()).and_then(|p| p.error())
}

/// The click on Install: a background job (the Jobs pane shows it). A second call while one runs returns it.
pub fn start_install(id: &str) -> Arc<Progress> {
    let mut slot = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = slot.as_ref().filter(|p| !p.is_done()) {
        return p.clone();
    }
    let id = id.to_string();
    let p = export::spawn_job("vlm-install", move |prog| install(&id, prog));
    *slot = Some(p.clone());
    p
}

// ---------------------------------------------------------------------------------------------------
// describing

/// What the describer said about one shot. Fields it did not give stay empty.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Description {
    pub framing: String,
    pub subject: String,
    pub camera: String,
    pub text: String,
    /// The whole answer, for anything the four fields missed.
    pub raw: String,
}

/// The question asked about each shot's picture (three frames side by side).
/// The small model copies a list of choices back instead of choosing, so it is asked for one plain sentence
/// and the fields are read out of it (`parse_answer`).
pub const PROMPT: &str = "These are three frames from one shot of a video, left to right: start, middle, end. \
In one or two sentences, say what the shot shows, whether it is a close-up, a medium shot or a wide shot, \
whether the camera moves, and quote any words written on screen.";

/// The four fields out of an answer: `Key: value` lines in any order and case; an answer without them
/// becomes the subject (the small model sometimes just describes).
pub fn parse_answer(raw: &str) -> Description {
    let mut d = Description { raw: raw.trim().to_string(), ..Default::default() };
    for line in raw.lines() {
        let line = line.trim().trim_start_matches(['-', '*', ' ']).replace("**", "");
        let Some((k, v)) = line.split_once(':') else { continue };
        let v = v.trim().trim_end_matches('.').to_string();
        match k.trim().to_ascii_lowercase().as_str() {
            "framing" | "shot" | "shot size" => d.framing = v,
            "subject" => d.subject = v,
            "camera" | "camera move" | "camera movement" => d.camera = v,
            "text" | "on-screen text" | "text on screen" => d.text = v,
            _ => {}
        }
    }
    // a plain answer: the subject is the first sentence, the rest is found in the words
    if d.subject.is_empty() {
        let first = d.raw.split(['.', '\n']).find(|x| !x.trim().is_empty()).unwrap_or("").trim();
        d.subject = first.chars().take(160).collect();
    }
    if d.framing.is_empty() {
        d.framing = framing_class(&d.raw).to_string();
    }
    if d.camera.is_empty() {
        d.camera = camera_class(&d.raw).to_string();
    }
    if d.text.is_empty() {
        d.text = quoted(&d.raw).unwrap_or_default();
    }
    d
}

/// The camera move a description names ("static" when it says the camera does not move; empty when it says nothing).
pub fn camera_class(t: &str) -> &'static str {
    let t = t.to_ascii_lowercase();
    let not = t.contains("does not move") || t.contains("doesn't move") || t.contains("not moving") || t.contains("still camera");
    if not || t.contains("static") || t.contains("stationary") {
        return "static";
    }
    for (k, v) in [("zoom in", "zoom in"), ("zooms in", "zoom in"), ("zoom out", "zoom out"), ("zooms out", "zoom out"), (" pan", "pan"), (" tilt", "tilt"), ("track", "tracking"), ("follow", "tracking"), ("handheld", "handheld"), ("shak", "handheld"), ("moves", "moving"), ("moving", "moving")] {
        if t.contains(k) {
            return v;
        }
    }
    ""
}

/// The first "quoted" words in a description (straight or curly quotes).
fn quoted(t: &str) -> Option<String> {
    for (a, b) in [('"', '"'), ('\u{201c}', '\u{201d}')] {
        if let Some(i) = t.find(a) {
            let rest = &t[i + a.len_utf8()..];
            if let Some(j) = rest.find(b) {
                let q = rest[..j].trim();
                if !q.is_empty() {
                    return Some(q.to_string());
                }
            }
        }
    }
    None
}

/// The shot size a framing answer names, as one of five words (empty when it names none).
pub fn framing_class(f: &str) -> &'static str {
    let f = f.to_ascii_lowercase();
    if f.contains("extreme close") {
        "extreme close-up"
    } else if f.contains("close") {
        "close-up"
    } else if f.contains("extreme wide") || f.contains("establishing") {
        "extreme wide"
    } else if f.contains("wide") || f.contains("long") {
        "wide"
    } else if f.contains("medium") || f.contains("mid") {
        "medium"
    } else {
        ""
    }
}

/// Describe one picture with model `id` (blocks: seconds per picture on the CPU). The child runs below
/// normal priority so playback never waits on it.
pub fn describe(png: &Path, id: &str) -> Result<Description, String> {
    let exe = exe().ok_or("the video describer is not installed")?;
    let (model, mmproj) = model_files(id);
    if !model.is_file() {
        return Err(format!("describer model '{id}' is not installed"));
    }
    let threads = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(1, 8));
    let out = crate::media::bg::command(&exe)
        .arg("-m")
        .arg(&model)
        .arg("--mmproj")
        .arg(&mmproj)
        .arg("--image")
        .arg(png)
        .args(["-p", PROMPT, "-n", "120", "--temp", "0", "-t", &threads.to_string(), "-c", "4096"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{}: {e}", exe.display()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
        return Err(if last.is_empty() { format!("the describer failed ({})", out.status) } else { last });
    }
    Ok(parse_answer(&String::from_utf8_lossy(&out.stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_complete_and_permissive() {
        let m = manifest();
        assert_eq!(m.models.len(), 2);
        assert_eq!(m.models[0].id, DEFAULT);
        assert!(m.runtime.url.contains("ggml-org/llama.cpp") && m.runtime.url.ends_with("win-cpu-x64.zip"));
        for x in &m.models {
            assert!(x.model.url.contains("ggml-org/SmolVLM2") && x.mmproj.url.contains("mmproj"), "{}", x.id);
        }
        let v: Value = serde_json::from_str(MANIFEST).unwrap();
        assert!(v["runtime"]["license"].as_str().unwrap().starts_with("MIT"));
        assert!(v["models"].as_array().unwrap().iter().all(|m| m["license"].as_str().unwrap().starts_with("Apache-2.0")));
        assert!(parse_manifest("{\"runtime\": {}, \"models\": []}").is_err());
    }

    #[test]
    fn answers_parse_leniently() {
        let d = parse_answer("Framing: close-up\nSubject: a man talking to camera.\nCamera: static\nText: none\n");
        assert_eq!((d.framing.as_str(), d.subject.as_str(), d.camera.as_str(), d.text.as_str()), ("close-up", "a man talking to camera", "static", "none"));
        let d = parse_answer(" - **Camera**: pan\n**framing**: Wide shot\n");
        assert_eq!((d.framing.as_str(), d.camera.as_str()), ("Wide shot", "pan"));
        let d = parse_answer("A wide shot of a red car on a road. The camera pans left and the sign says \"LISBON\".");
        assert_eq!(d.subject, "A wide shot of a red car on a road");
        assert_eq!((d.framing.as_str(), d.camera.as_str(), d.text.as_str()), ("wide", "pan", "LISBON"));
        let d = parse_answer("A close-up of a man talking. The camera does not move.");
        assert_eq!((d.framing.as_str(), d.camera.as_str(), d.text.as_str()), ("close-up", "static", ""));
        assert_eq!(framing_class("Extreme close-up of eyes"), "extreme close-up");
        assert_eq!(framing_class("medium shot"), "medium");
        assert_eq!(framing_class("Wide shot"), "wide");
        assert_eq!(framing_class("establishing shot"), "extreme wide");
        assert_eq!(framing_class("?"), "");
    }
}
