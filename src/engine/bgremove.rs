//! ---- ws:addon-bg-removal ----
//! Background removal without a green screen: an optional add-on, never part of the exe.
//!
//! **The add-on** (`%APPDATA%\SimpleEditor\bin\bgremove`), installed on a click from the library's
//! `bin/bgremove.json` manifest. Each part is downloaded on its own, checked against its size and
//! SHA-256, and only then moved in:
//! * `se-matte.exe` - our helper (`tools/se-matte`, MIT): ONNX Runtime with DirectML on the GPU and a
//!   CPU fallback. Raw RGB frames in on stdin, 8-bit mattes out on stdout.
//! * `onnxruntime.dll` (+ `onnxruntime_providers_shared.dll`) - Microsoft's ONNX Runtime 1.22 DirectML
//!   build (MIT), taken out of its NuGet package. DirectML itself is the `DirectML.dll` Windows ships.
//! * `modnet.onnx` (Fast, MODNet, Apache-2.0) and `birefnet_lite.onnx` (Best, BiRefNet lite, MIT), each
//!   downloaded the first time that quality is used.
//!
//! **The matte cache** (documented in notes.md, "bg-removal"): one folder per (source file, quality)
//! under `<cache>\mattes\<16 hex>-<fast|best>\` (`cache_dir`; the hash is FNV-1a 64 of the lower-cased
//! path, its size and mtime). In it `matte.json` (`{"format":1,"source":..,"quality":..,"fps":..,
//! "width":..,"height":..}`) and one 8-bit greyscale PNG per source frame, `<n>.png`, `n` =
//! `floor(t·fps + 0.001)` at the source's frame rate (a still has only `0.png`). Frames are written to a
//! temp name and renamed, so a reader never sees half a file. Missing frame = not computed yet.
//!
//! **Playback reads only the cache** (`matte_frame`): a PNG of at most 1024 px, decoded and shaped by
//! the Edge control on the render thread's read-ahead; a frame that is not there draws the clip unkeyed.
//! The model never runs on the playback path - only in the job (`run`), below normal priority while
//! anything plays.

use crate::engine::blitz;
use crate::engine::export::{self, Progress};
use crate::media::{ffpipe, Frame};
use crate::model::BgQuality;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

pub const HELPER: &str = "se-matte.exe";
pub const RUNTIME: &str = "onnxruntime.dll";
/// The library file that names the downloads.
pub const MANIFEST: &str = "bin/bgremove.json";
/// The cache folder's format number (`matte.json`).
pub const FORMAT: u32 = 1;

/// `%APPDATA%\SimpleEditor\bin\bgremove` - the installed add-on. `SE_BGREMOVE_DIR` overrides it (tests,
/// development).
pub fn dir() -> PathBuf {
    match std::env::var_os("SE_BGREMOVE_DIR") {
        Some(d) => PathBuf::from(d),
        None => blitz::bin_dir().join("bgremove"),
    }
}

pub fn model_file(q: BgQuality) -> &'static str {
    match q {
        BgQuality::Fast => "modnet.onnx",
        BgQuality::Best => "birefnet_lite.onnx",
    }
}

/// The helper's `--kind`.
fn kind(q: BgQuality) -> &'static str {
    match q {
        BgQuality::Fast => "modnet",
        BgQuality::Best => "birefnet",
    }
}

/// Longest side of a stored matte. MODNet works at 512, BiRefNet at 1024.
pub fn matte_long_side(q: BgQuality) -> u32 {
    match q {
        BgQuality::Fast => 512,
        BgQuality::Best => 1024,
    }
}

/// What is installed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Installed {
    pub helper: bool,
    pub runtime: bool,
    pub fast: bool,
    pub best: bool,
}

pub fn installed() -> Installed {
    let d = dir();
    Installed {
        helper: d.join(HELPER).is_file(),
        runtime: d.join(RUNTIME).is_file(),
        fast: d.join(model_file(BgQuality::Fast)).is_file(),
        best: d.join(model_file(BgQuality::Best)).is_file(),
    }
}

/// Everything quality `q` needs is installed.
pub fn available(q: BgQuality) -> bool {
    let i = installed();
    i.helper && i.runtime && if q == BgQuality::Fast { i.fast } else { i.best }
}

/// The installed manifest's version, when installed through the app.
pub fn version() -> Option<String> {
    let text = std::fs::read_to_string(dir().join("bgremove.json")).ok()?;
    parse_manifest(&text).ok().map(|m| m.version)
}

/// The one call an add-on list needs: install folder, what is in it, and the installed version.
/// "Installed" means `available(Fast) || available(Best)`.
pub fn addon_status() -> (PathBuf, Installed, Option<String>) {
    (dir(), installed(), version())
}

// ---------------------------------------------------------------------------------------------------
// install / uninstall

/// One download of the add-on.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    /// "helper", "runtime", "fast" or "best".
    pub id: String,
    /// A path relative to the library (a folder source) or a full address.
    pub url: String,
    pub sha256: String,
    pub size: u64,
    /// Saved as this file name (a plain download).
    pub save: String,
    /// Or: members to take out of the download (a zip / .nupkg), each saved under its file name.
    pub extract: Vec<String>,
    pub license: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub version: String,
    pub parts: Vec<Part>,
}

pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let v: serde_json::Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("bgremove.json: {e}"))?;
    let s = |v: &serde_json::Value, k: &str| v[k].as_str().unwrap_or("").trim().to_string();
    let mut parts = Vec::new();
    for p in v["parts"].as_array().ok_or("bgremove.json has no parts")? {
        let part = Part {
            id: s(p, "id"),
            url: s(p, "url"),
            sha256: s(p, "sha256").to_ascii_lowercase(),
            size: p["size"].as_u64().unwrap_or(0),
            save: s(p, "save"),
            extract: p["extract"]
                .as_array()
                .map(|a| a.iter().filter_map(|m| m.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            license: s(p, "license"),
        };
        if part.id.is_empty()
            || part.url.is_empty()
            || part.sha256.len() != 64
            || part.size == 0
            || (part.save.is_empty() && part.extract.is_empty())
        {
            return Err(format!("bgremove.json: part \"{}\" is incomplete", part.id));
        }
        parts.push(part);
    }
    for id in ["helper", "runtime", "fast", "best"] {
        if !parts.iter().any(|p| p.id == id) {
            return Err(format!("bgremove.json has no \"{id}\" part"));
        }
    }
    Ok(Manifest { version: s(&v, "version"), parts })
}

/// The file names a part puts in `dir()`.
fn part_files(p: &Part) -> Vec<String> {
    if p.extract.is_empty() {
        vec![p.save.clone()]
    } else {
        p.extract.iter().map(|m| m.rsplit(['/', '\\']).next().unwrap_or(m).to_string()).collect()
    }
}

/// The parts quality `q` needs that `d` does not have yet.
fn missing<'m>(m: &'m Manifest, q: BgQuality, d: &Path) -> Vec<&'m Part> {
    m.parts
        .iter()
        .filter(|p| p.id == "helper" || p.id == "runtime" || p.id == q.id())
        .filter(|p| part_files(p).iter().any(|f| !d.join(f).is_file()))
        .collect()
}

/// Download (or copy, from a folder library) what quality `q` still needs, check each part's size and
/// SHA-256, unpack, check the helper starts, and move the files into `dir()`. Blocking: a background
/// job only, started by a click or the `clip.remove_background` tool.
pub fn install(q: BgQuality, prog: &Progress) -> Result<(), String> {
    use crate::engine::library::Source;
    let src = Source::of(&crate::settings::Settings::load());
    prog.set(0.01, "Looking up background removal…");
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
    let dest = dir();
    std::fs::create_dir_all(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let todo = missing(&m, q, &dest);
    let total: u64 = todo.iter().map(|p| p.size).sum::<u64>().max(1);
    let mut done = 0u64;
    for p in todo {
        if prog.is_cancelled() {
            return Err(export::CANCELLED.into());
        }
        let frac = |d: u64| 0.02 + 0.93 * (d as f32 / total as f32);
        prog.set(frac(done), format!("Downloading {} ({:.0} MB)…", part_name(&p.id), p.size as f64 / 1048576.0));
        let part = dest.join(format!("{}.part", p.id));
        let fresh = dest.join(format!("{}.new", p.id));
        let clean = || {
            let _ = std::fs::remove_file(&part);
            let _ = std::fs::remove_dir_all(&fresh);
        };
        clean();
        let r = fetch(&src, p, &part).and_then(|_| {
            if prog.is_cancelled() {
                return Err(export::CANCELLED.into());
            }
            let size = std::fs::metadata(&part).map(|x| x.len()).unwrap_or(0);
            if size != p.size {
                return Err(format!("{}: the download is {size} bytes, expected {}", p.id, p.size));
            }
            if blitz::sha256_of(&part)? != p.sha256 {
                return Err(format!("{}: the download does not match its checksum", p.id));
            }
            if p.extract.is_empty() {
                let to = dest.join(&p.save);
                let _ = std::fs::remove_file(&to);
                return std::fs::rename(&part, &to).map_err(|e| format!("{}: {e}", to.display()));
            }
            unpack(&part, &fresh, &p.extract)?;
            for (member, name) in p.extract.iter().zip(part_files(p)) {
                let to = dest.join(&name);
                let _ = std::fs::remove_file(&to);
                std::fs::rename(fresh.join(member), &to).map_err(|e| format!("{}: {e}", to.display()))?;
            }
            Ok(())
        });
        clean();
        r?;
        done += p.size;
    }
    prog.set(0.97, "Checking it…");
    check_runs(&dest.join(HELPER))?;
    std::fs::write(dest.join("bgremove.json"), &text).map_err(|e| e.to_string())?;
    prog.set(1.0, "Background removal installed");
    Ok(())
}

fn part_name(id: &str) -> &'static str {
    match id {
        "helper" => "the helper",
        "runtime" => "ONNX Runtime",
        "fast" => "the Fast model (MODNet)",
        "best" => "the Best model (BiRefNet)",
        _ => "a part",
    }
}

fn fetch(src: &crate::engine::library::Source, p: &Part, to: &Path) -> Result<(), String> {
    use crate::engine::library::Source;
    match src {
        Source::Dir(d) if !p.url.contains("://") => {
            std::fs::copy(d.join(&p.url), to).map(|_| ()).map_err(|e| format!("{}: {e}", d.join(&p.url).display()))
        }
        _ => {
            let url = if p.url.contains("://") { p.url.clone() } else { src.url(&p.url).ok_or("no library address")? };
            blitz::curl(&url, to, 3600)
        }
    }
}

/// Take `members` out of a zip (a .nupkg is one) with Windows' own tar (bsdtar reads zip; git's GNU tar
/// on PATH does not, so it is named exactly).
fn unpack(zip: &Path, into: &Path, members: &[String]) -> Result<(), String> {
    std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
    let tar = std::env::var_os("SystemRoot")
        .map(|r| PathBuf::from(r).join("System32").join("tar.exe"))
        .filter(|p| p.is_file())
        .ok_or("tar.exe not found")?;
    let out = ffpipe::command(&tar)
        .arg("-xf")
        .arg(zip)
        .arg("-C")
        .arg(into)
        .args(members)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("tar: {e}"))?;
    if !out.status.success() {
        return Err(format!("could not unpack: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

/// It starts and is the helper.
fn check_runs(exe: &Path) -> Result<(), String> {
    let out = ffpipe::command(exe)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("the helper does not start: {e}"))?;
    if String::from_utf8_lossy(&out.stdout).contains("se-matte") {
        Ok(())
    } else {
        Err("the installed helper is not se-matte".into())
    }
}

/// Remove the add-on. Computed mattes stay in the cache (Settings ▸ General clears it).
pub fn uninstall() -> Result<(), String> {
    let d = dir();
    if d.exists() {
        std::fs::remove_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    Ok(())
}

static INSTALLING: Mutex<Option<(BgQuality, Arc<Progress>)>> = Mutex::new(None);

/// The running install, if any.
pub fn installing() -> Option<Arc<Progress>> {
    INSTALLING.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|s| s.1.clone()).filter(|p| !p.is_done())
}

/// Why the last install failed (shown beside the Install button until the next try).
pub fn install_error() -> Option<String> {
    let slot = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    slot.as_ref().map(|s| &s.1).filter(|p| p.is_done() && !p.is_cancelled()).and_then(|p| p.error())
}

/// The click on "Install": a background job (the Jobs pane shows it). A second click while one runs
/// returns that one.
pub fn start_install(q: BgQuality) -> Arc<Progress> {
    let mut slot = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, p)) = slot.as_ref().filter(|s| !s.1.is_done()) {
        return p.clone();
    }
    let p = export::spawn_job("bgremove-install", move |prog| install(q, prog));
    *slot = Some((q, p.clone()));
    p
}

// ---------------------------------------------------------------------------------------------------
// the matte cache

/// FNV-1a 64: stable across builds (std's hasher is not promised to be).
fn fnv(bytes: &[u8], mut h: u64) -> u64 {
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// `<cache>\mattes\<hash>-<quality>` for `path` as it is on disk now (size and mtime in the hash, so an
/// edited file gets fresh mattes).
pub fn cache_dir(path: &str, q: BgQuality) -> PathBuf {
    cache_dir_in(&crate::settings::Settings::cache_dir().join("mattes"), path, q)
}

/// `cache_dir` under another root (tests).
pub fn cache_dir_in(root: &Path, path: &str, q: BgQuality) -> PathBuf {
    let meta = std::fs::metadata(path).ok();
    let len = meta.as_ref().map_or(0, |m| m.len());
    let mtime = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let mut h = fnv(path.replace('/', "\\").to_lowercase().as_bytes(), 0xcbf29ce484222325);
    h = fnv(&len.to_le_bytes(), h);
    h = fnv(&mtime.to_le_bytes(), h);
    root.join(format!("{h:016x}-{}", q.id()))
}

/// `cache_dir` remembered per (path, quality): the playback path asks every frame, a stat each time is
/// not needed. `forget` drops a path when a job starts on it (the file may have changed).
static DIRS: Mutex<Option<HashMap<(String, BgQuality), PathBuf>>> = Mutex::new(None);

fn dir_for(path: &str, q: BgQuality) -> PathBuf {
    let mut g = DIRS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new).entry((path.to_string(), q)).or_insert_with(|| cache_dir(path, q)).clone()
}

pub fn forget(path: &str) {
    if let Some(m) = DIRS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        m.retain(|k, _| k.0 != path);
    }
}

/// The source frame shown at source time `t`.
pub fn frame_index(t: f64, fps: f64) -> i64 {
    let fps = if fps > 0.0 { fps } else { 30.0 };
    ((t.max(0.0) * fps) + 1e-3).floor() as i64
}

fn frame_file(dir: &Path, n: i64) -> PathBuf {
    dir.join(format!("{n}.png"))
}

/// The stored matte size for a `w`x`h` source: long side `matte_long_side` (never more than the
/// source), even sides.
pub fn matte_size(w: u32, h: u32, q: BgQuality) -> (u32, u32) {
    let (w, h) = (w.max(2), h.max(2));
    let long = matte_long_side(q).min(w.max(h));
    let s = long as f64 / w.max(h) as f64;
    let even = |v: u32| (((v as f64 * s) / 2.0).round() as u32).max(1) * 2;
    (even(w), even(h))
}

pub fn write_matte(dir: &Path, n: i64, w: u32, h: u32, m: &[u8]) -> Result<(), String> {
    let tmp = dir.join(format!("{n}.png.tmp"));
    {
        let f = std::fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut wr = enc.write_header().map_err(|e| e.to_string())?;
        wr.write_image_data(m).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, frame_file(dir, n)).map_err(|e| e.to_string())
}

/// A stored matte: (width, height, one byte per pixel).
pub fn read_matte(dir: &Path, n: i64) -> Option<(u32, u32, Vec<u8>)> {
    let f = std::fs::File::open(frame_file(dir, n)).ok()?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(f));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    let ch = info.color_type.samples();
    buf.truncate(info.buffer_size());
    let grey = if ch == 1 { buf } else { buf.chunks_exact(ch).map(|p| p[0]).collect() };
    (grey.len() == (w * h) as usize).then_some((w, h, grey))
}

/// How many of the frames `n0..n1` of `path` are in the cache.
pub fn coverage(path: &str, q: BgQuality, n0: i64, n1: i64) -> (usize, usize) {
    let d = dir_for(path, q);
    let total = (n1 - n0).max(0) as usize;
    let Ok(rd) = std::fs::read_dir(&d) else { return (0, total) };
    let have = rd
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".png")?.parse::<i64>().ok())
        .filter(|n| (n0..n1).contains(n))
        .count();
    (have, total)
}

/// The frame indices a clip shows (plus `handle` seconds each side) of a source at `fps`; a still is
/// frame 0.
pub fn clip_frames(clip: &crate::model::Clip, asset_duration: f64, fps: f64, still: bool, handle: f64) -> (i64, i64) {
    if still {
        return (0, 1);
    }
    let (a, b) = crate::engine::enhance::source_range(clip, asset_duration, fps, handle);
    let n0 = frame_index(a, fps);
    let n1 = ((b * if fps > 0.0 { fps } else { 30.0 }) - 1e-3).ceil() as i64;
    (n0, n1.max(n0 + 1))
}

/// Shape a matte by the Edge control: below 0 erode and harden it (shrink), above 0 blur it (feather).
pub fn shape_edge(m: &mut [u8], w: usize, h: usize, edge: f32) {
    let e = edge.clamp(-1.0, 1.0);
    if e.abs() < 1e-3 || w == 0 || h == 0 {
        return;
    }
    let long = w.max(h) as f32;
    if e < 0.0 {
        let s = -e;
        let r = (s * long / 128.0).round() as usize;
        if r > 0 {
            box_pass(m, w, h, r, true);
        }
        let lo = 0.6 * s * 255.0;
        for v in m.iter_mut() {
            *v = (((*v as f32 - lo) / (255.0 - lo)).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    } else {
        let r = (e * long / 64.0).round().max(1.0) as usize;
        box_pass(m, w, h, r, false);
        box_pass(m, w, h, r, false);
    }
}

/// One separable pass of radius `r`: a box mean, or (`min`) an erosion.
fn box_pass(m: &mut [u8], w: usize, h: usize, r: usize, min: bool) {
    let mut tmp = vec![0u8; m.len()];
    let line = |src: &[u8], dst: &mut [u8], n: usize, at: &dyn Fn(usize) -> usize| {
        if min {
            for i in 0..n {
                let (a, b) = (i.saturating_sub(r), (i + r).min(n - 1));
                dst[at(i)] = (a..=b).map(|j| src[at(j)]).min().unwrap_or(0);
            }
        } else {
            // running sum over the clamped window
            let mut sum: u32 = 0;
            for j in 0..=r.min(n - 1) {
                sum += src[at(j)] as u32;
            }
            let mut count = r.min(n - 1) + 1;
            for i in 0..n {
                dst[at(i)] = ((sum + count as u32 / 2) / count as u32) as u8;
                let add = i + r + 1;
                if add < n {
                    sum += src[at(add)] as u32;
                    count += 1;
                }
                if i >= r {
                    sum -= src[at(i - r)] as u32;
                    count -= 1;
                }
            }
        }
    };
    for y in 0..h {
        line(m, &mut tmp, w, &|x| y * w + x);
    }
    for x in 0..w {
        line(&tmp, m, h, &|y| y * w + x);
    }
}

/// The light temporal pass that keeps edges from flickering: the median of the frame and its two
/// neighbours (a one-frame spike goes, a real change stays), then a small pull towards the previous
/// output where the change is only a few levels (jitter). `next` = None at the end of a run.
pub fn smooth(prev: Option<&[u8]>, cur: &[u8], next: Option<&[u8]>, last: Option<&[u8]>, out: &mut [u8]) {
    for i in 0..cur.len() {
        let c = cur[i];
        let med = match (prev, next) {
            (Some(p), Some(n)) => {
                let (a, b) = (p[i], n[i]);
                c.max(a.min(b)).min(a.max(b))
            }
            _ => c,
        };
        out[i] = match last {
            Some(l) if (med as i16 - l[i] as i16).abs() < 16 => ((med as u16 * 3 + l[i] as u16 + 2) / 4) as u8,
            _ => med,
        };
    }
}

/// The matte for source time `t` of `path` as an RGBA frame (grey, opaque: white = keep), shaped by
/// `edge`. None = not computed (yet): draw the clip unkeyed. The playback path's only entry point -
/// it reads the cache and nothing else.
pub fn matte_frame(path: &str, q: BgQuality, fps: f64, t: f64, still: bool, edge: f32) -> Option<Arc<Frame>> {
    let n = if still { 0 } else { frame_index(t, fps) };
    let (w, h, mut m) = read_matte(&dir_for(path, q), n)?;
    shape_edge(&mut m, w as usize, h as usize, edge);
    let mut f = Frame::new(w, h);
    f.pts = t;
    for (px, v) in f.rgba.chunks_exact_mut(4).zip(&m) {
        px.copy_from_slice(&[*v, *v, *v, 255]);
    }
    Some(Arc::new(f))
}

/// The CPU compositor's side: multiply `img`'s alpha by `matte` (sampled bilinearly to its size), or
/// with `show` replace the picture by the matte.
pub fn apply_cpu(img: &mut Frame, matte: &Frame, show: bool) {
    img.make_rgba();
    let (w, h) = (img.width as usize, img.height as usize);
    let (mw, mh) = (matte.width as usize, matte.height as usize);
    if w == 0 || h == 0 || mw == 0 || mh == 0 {
        return;
    }
    let (sx, sy) = (mw as f32 / w as f32, mh as f32 / h as f32);
    let at = |x: usize, y: usize| matte.rgba[(y * mw + x) * 4] as f32;
    for y in 0..h {
        let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (mh - 1) as f32);
        let (y0, wy) = (fy as usize, fy.fract());
        let y1 = (y0 + 1).min(mh - 1);
        for x in 0..w {
            let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (mw - 1) as f32);
            let (x0, wx) = (fx as usize, fx.fract());
            let x1 = (x0 + 1).min(mw - 1);
            let top = at(x0, y0) * (1.0 - wx) + at(x1, y0) * wx;
            let bot = at(x0, y1) * (1.0 - wx) + at(x1, y1) * wx;
            let k = top * (1.0 - wy) + bot * wy;
            let px = &mut img.rgba[(y * w + x) * 4..][..4];
            if show {
                let v = (k + 0.5) as u8;
                px.copy_from_slice(&[v, v, v, 255]);
            } else {
                px[3] = ((px[3] as f32 * k) / 255.0 + 0.5) as u8;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------
// computing mattes

/// One matte job: the frames `frames.0..frames.1` of `src` with quality `quality`.
#[derive(Clone, Debug)]
pub struct Request {
    pub src: String,
    pub still: bool,
    pub fps: f64,
    /// The source's own size.
    pub size: (u32, u32),
    pub frames: (i64, i64),
    pub quality: BgQuality,
    /// "auto" (DirectML, else CPU), "dml" or "cpu".
    pub ep: String,
}

/// What a finished job measured.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub frames: usize,
    pub ms_per_frame: f64,
    /// "dml" or "cpu" (the helper's first stderr line).
    pub ep: String,
}

/// Runs of `n0..n1` with no cached frame.
fn missing_runs(dir: &Path, n0: i64, n1: i64) -> Vec<(i64, i64)> {
    let mut runs = Vec::new();
    let mut start = None;
    for n in n0..n1 {
        let have = frame_file(dir, n).is_file();
        match (have, start) {
            (false, None) => start = Some(n),
            (true, Some(s)) => {
                runs.push((s, n));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        runs.push((s, n1));
    }
    runs
}

fn set_low(child: &std::process::Child, low: bool) {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::System::Threading::{SetPriorityClass, BELOW_NORMAL_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS};
    let class = if low { BELOW_NORMAL_PRIORITY_CLASS } else { NORMAL_PRIORITY_CLASS };
    // SAFETY: the handle belongs to `child`, which outlives this call; a failure leaves the class as it was.
    unsafe {
        let _ = SetPriorityClass(windows::Win32::Foundation::HANDLE(child.as_raw_handle()), class);
    }
}

/// Compute the missing mattes of `req`: ffmpeg decodes each missing run at the matte size, the helper
/// turns frames into mattes, the temporal pass smooths them and each lands in the cache as it is done.
/// `landed` is called about once a second with new frames (so a paused preview can pick them up).
pub fn run(req: &Request, prog: &Progress, landed: &dyn Fn()) -> Result<Report, String> {
    forget(&req.src);
    run_in(&dir(), &cache_dir(&req.src, req.quality), req, prog, landed)
}

/// `run` with the add-on in `addon` and the mattes written to `d` (tests point both at scratch folders).
pub fn run_in(addon: &Path, d: &Path, req: &Request, prog: &Progress, landed: &dyn Fn()) -> Result<Report, String> {
    let helper = addon.join(HELPER);
    let model = addon.join(model_file(req.quality));
    if !helper.is_file() || !addon.join(RUNTIME).is_file() || !model.is_file() {
        return Err(format!("background removal ({}) is not installed", req.quality.label()));
    }
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let d = d.to_path_buf();
    std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    let (w, h) = matte_size(req.size.0, req.size.1, req.quality);
    let meta = serde_json::json!({
        "format": FORMAT, "source": req.src, "quality": req.quality.id(), "fps": req.fps, "width": w, "height": h,
    });
    // a folder made at another size (another format) starts over
    let old: Option<serde_json::Value> =
        std::fs::read_to_string(d.join("matte.json")).ok().and_then(|t| serde_json::from_str(&t).ok());
    if old.as_ref().is_some_and(|o| o["width"] != meta["width"] || o["height"] != meta["height"] || o["format"] != meta["format"]) {
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    }
    std::fs::write(d.join("matte.json"), meta.to_string()).map_err(|e| e.to_string())?;
    let runs = missing_runs(&d, req.frames.0, req.frames.1);
    let total: i64 = runs.iter().map(|r| r.1 - r.0).sum();
    let mut rep = Report::default();
    if total == 0 {
        prog.set(1.0, "Matte ready");
        return Ok(rep);
    }
    let fps = if req.fps > 0.0 { req.fps } else { 30.0 };
    let t0 = std::time::Instant::now();
    let mut done = 0i64;
    let mut last_note = std::time::Instant::now();
    let px = (w * h) as usize;
    for (a, b) in runs {
        let mut ff = ffpipe::command(&ffmpeg);
        ff.args(["-v", "error", "-nostdin"]);
        if !req.still {
            ff.arg("-ss").arg(format!("{:.6}", a as f64 / fps));
        }
        ff.arg("-i").arg(&req.src);
        let mut vf = format!("scale={w}:{h}:flags=bilinear");
        if !req.still {
            vf = format!("fps={fps},{vf}");
        }
        ff.args(["-frames:v", &(b - a).to_string(), "-vf", &vf, "-pix_fmt", "rgb24", "-f", "rawvideo", "pipe:1"]);
        let mut ff = ff.stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| format!("ffmpeg: {e}"))?;
        let frames_in = ff.stdout.take().ok_or("ffmpeg has no output")?;
        let spawned = ffpipe::command(&helper)
            .arg("--model")
            .arg(&model)
            .args(["--kind", kind(req.quality), "--width", &w.to_string(), "--height", &h.to_string(), "--ep", &req.ep])
            .stdin(Stdio::from(frames_in))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut hp = match spawned {
            Ok(c) => c,
            Err(e) => {
                let _ = ff.kill();
                let _ = ff.wait();
                return Err(format!("the helper does not start: {e}"));
            }
        };
        let mut out = hp.stdout.take().ok_or("the helper has no output")?;
        let err = hp.stderr.take();
        let tail = std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(mut e) = err {
                let _ = e.read_to_string(&mut s);
            }
            s
        });
        let mut low = None;
        let mut sync_pri = |hp: &std::process::Child, ff: &std::process::Child| {
            let want = crate::media::bg::playing();
            if low != Some(want) {
                set_low(hp, want);
                set_low(ff, want);
                low = Some(want);
            }
        };
        // a window of three raw mattes for the median, and the last written one
        let (mut prev, mut cur): (Option<Vec<u8>>, Option<Vec<u8>>) = (None, None);
        let mut last: Option<Vec<u8>> = None;
        let mut n = a;
        let mut buf = vec![0u8; px];
        let mut sm = vec![0u8; px];
        let mut fail = None;
        let mut write = |prev: Option<&[u8]>, cur: &[u8], next: Option<&[u8]>, last: &mut Option<Vec<u8>>, n: i64| {
            smooth(prev, cur, next, last.as_deref(), &mut sm);
            let r = write_matte(&d, n, w, h, &sm);
            *last = Some(sm.clone());
            r
        };
        loop {
            sync_pri(&hp, &ff);
            if prog.is_cancelled() {
                fail = Some(export::CANCELLED.to_string());
                break;
            }
            match out.read_exact(&mut buf) {
                Ok(()) => {}
                Err(_) => break, // EOF: the run is done (or the helper failed - its stderr says)
            }
            let next = std::mem::replace(&mut buf, vec![0u8; px]);
            if let Some(c) = cur.take() {
                if let Err(e) = write(prev.as_deref(), &c, Some(&next), &mut last, n) {
                    fail = Some(e);
                    break;
                }
                n += 1;
                done += 1;
                prev = Some(c);
            }
            cur = Some(next);
            prog.set(
                (done as f32 / total as f32).min(0.99),
                format!("Removing background · {done} / {total} frames"),
            );
            if last_note.elapsed().as_secs_f64() > 1.0 {
                landed();
                last_note = std::time::Instant::now();
            }
        }
        if fail.is_none() {
            if let Some(c) = cur.take() {
                if n < b {
                    match write(prev.as_deref(), &c, None, &mut last, n) {
                        Ok(()) => {
                            n += 1;
                            done += 1;
                        }
                        Err(e) => fail = Some(e),
                    }
                }
            }
        }
        let _ = hp.kill();
        let _ = ff.kill();
        let status = hp.wait().ok();
        let _ = ff.wait();
        let log = tail.join().unwrap_or_default();
        if let Some(ep) = log.lines().find_map(|l| l.strip_prefix("ep ")) {
            rep.ep = ep.trim().to_string();
        }
        if let Some(e) = fail {
            landed();
            return Err(e);
        }
        if n < b {
            let why = log.lines().find_map(|l| l.strip_prefix("error: ")).map(str::to_string);
            landed();
            return Err(why.unwrap_or_else(|| {
                format!("the helper stopped after {} of {} frames ({:?})", n - a, b - a, status.and_then(|s| s.code()))
            }));
        }
    }
    landed();
    rep.frames = done as usize;
    rep.ms_per_frame = t0.elapsed().as_secs_f64() * 1000.0 / done.max(1) as f64;
    prog.set(1.0, format!("Matte ready · {done} frames, {:.0} ms a frame ({})", rep.ms_per_frame, rep.ep));
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh scratch folder under the temp dir (point TMP at G:\se-tmp when running the tests).
    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("se-bgremove-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn shipped() -> Manifest {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/library/bin/bgremove.json")).unwrap();
        parse_manifest(&text).unwrap()
    }

    #[test]
    fn the_shipped_manifest_parses_and_names_every_part_with_a_licence() {
        let m = shipped();
        for id in ["helper", "runtime", "fast", "best"] {
            let p = m.parts.iter().find(|p| p.id == id).unwrap();
            assert!(!p.license.is_empty(), "{id} has no licence");
            assert!(!p.license.to_ascii_lowercase().contains("gpl"), "{id}: no GPL in the add-on");
        }
        let rt = m.parts.iter().find(|p| p.id == "runtime").unwrap();
        assert_eq!(part_files(rt), vec!["onnxruntime.dll", "onnxruntime_providers_shared.dll", "LICENSE", "ThirdPartyNotices.txt"]);
    }

    #[test]
    fn an_incomplete_manifest_is_refused() {
        assert!(parse_manifest("{}").is_err());
        let one = r#"{"parts":[{"id":"helper","url":"x","sha256":"00","size":1,"save":"a"}]}"#;
        assert!(parse_manifest(one).unwrap_err().contains("incomplete"));
    }

    #[test]
    fn only_the_missing_parts_of_a_quality_are_downloaded() {
        let m = shipped();
        let d = scratch("missing");
        let ids = |q| missing(&m, q, &d).iter().map(|p| p.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(BgQuality::Fast), ["helper", "runtime", "fast"]);
        for f in [HELPER, RUNTIME, "onnxruntime_providers_shared.dll", "LICENSE", "ThirdPartyNotices.txt", "modnet.onnx"] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        assert!(ids(BgQuality::Fast).is_empty());
        assert_eq!(ids(BgQuality::Best), ["best"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_matte_survives_the_cache_round_trip() {
        let d = scratch("io");
        let m: Vec<u8> = (0..64 * 36).map(|i| (i % 256) as u8).collect();
        write_matte(&d, 41, 64, 36, &m).unwrap();
        assert!(!d.join("41.png.tmp").exists(), "written under a temp name, then renamed");
        let (w, h, back) = read_matte(&d, 41).unwrap();
        assert_eq!((w, h), (64, 36));
        assert_eq!(back, m);
        assert!(read_matte(&d, 42).is_none(), "a frame not computed is simply missing");
        assert_eq!(missing_runs(&d, 38, 45), vec![(38, 41), (42, 45)]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn cache_folders_follow_the_file_and_the_quality() {
        let root = Path::new("Z:/cache/mattes");
        let a = cache_dir_in(root, "C:/v/a.mp4", BgQuality::Fast);
        assert_eq!(a, cache_dir_in(root, "c:\\V\\A.MP4", BgQuality::Fast), "same file however it is spelled");
        assert_ne!(a, cache_dir_in(root, "C:/v/a.mp4", BgQuality::Best));
        assert_ne!(a, cache_dir_in(root, "C:/v/b.mp4", BgQuality::Fast));
        assert!(a.file_name().unwrap().to_string_lossy().ends_with("-fast"));
    }

    #[test]
    fn frame_indices_and_sizes() {
        assert_eq!(frame_index(0.0, 25.0), 0);
        assert_eq!(frame_index(4.5, 25.0), 112);
        assert_eq!(frame_index(1.0 / 30.0, 30.0), 1);
        assert_eq!(matte_size(1920, 1080, BgQuality::Fast), (512, 288));
        assert_eq!(matte_size(1920, 1080, BgQuality::Best), (1024, 576));
        assert_eq!(matte_size(640, 360, BgQuality::Best), (640, 360), "never bigger than the source");
        assert_eq!(matte_size(1080, 1920, BgQuality::Fast), (288, 512));
    }

    #[test]
    fn the_temporal_pass_removes_a_one_frame_flicker_and_keeps_a_real_change() {
        let mut out = [0u8; 3];
        // pixel 0 flickers on for one frame, pixel 1 really turns on, pixel 2 jitters by a few levels
        smooth(Some(&[0, 0, 200]), &[255, 255, 206], Some(&[0, 255, 200]), None, &mut out);
        assert_eq!(out[0], 0, "a one-frame spike goes");
        assert_eq!(out[1], 255, "a change that stays, stays");
        let mut out2 = [0u8; 3];
        smooth(Some(&[0, 0, 200]), &[0, 255, 206], None, Some(&[0, 255, 200]), &mut out2);
        assert_eq!(out2[2], 205, "small jitter is pulled towards the last output");
        assert_eq!(out2[1], 255);
    }

    #[test]
    fn edge_shrinks_and_feathers() {
        let (w, h) = (64usize, 64usize);
        let disc: Vec<u8> = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32 - 32.0, (i / w) as f32 - 32.0);
                if x * x + y * y < 400.0 {
                    255
                } else {
                    0
                }
            })
            .collect();
        let sum = |m: &[u8]| m.iter().map(|&v| v as u64).sum::<u64>();
        let mut shrunk = disc.clone();
        shape_edge(&mut shrunk, w, h, -1.0);
        assert!(sum(&shrunk) < sum(&disc) * 9 / 10, "shrink takes the edge in");
        let mut soft = disc.clone();
        shape_edge(&mut soft, w, h, 1.0);
        let partial = |m: &[u8]| m.iter().filter(|&&v| v > 0 && v < 255).count();
        assert!(partial(&soft) > partial(&disc) + 50, "feather makes a soft edge");
        assert_eq!(soft[32 * w + 32], 255, "the middle stays kept");
        assert_eq!(soft[0], 0, "the far corner stays removed");
        let mut same = disc.clone();
        shape_edge(&mut same, w, h, 0.0);
        assert_eq!(same, disc);
    }

    #[test]
    fn the_cpu_path_multiplies_alpha_or_shows_the_matte() {
        let mut img = Frame::new(4, 2);
        img.fill([10, 20, 30, 255]);
        let mut m = Frame::new(2, 1);
        m.rgba.copy_from_slice(&[0, 0, 0, 255, 255, 255, 255, 255]); // left removed, right kept
        let mut keyed = img.clone();
        apply_cpu(&mut keyed, &m, false);
        assert_eq!(keyed.rgba[3], 0, "left edge removed");
        assert_eq!(keyed.rgba[3 * 4 + 3], 255, "right edge kept");
        assert_eq!(&keyed.rgba[12..15], &[10, 20, 30], "colour untouched");
        let mut shown = img.clone();
        apply_cpu(&mut shown, &m, true);
        assert_eq!(&shown.rgba[0..4], &[0, 0, 0, 255]);
        assert_eq!(&shown.rgba[12..16], &[255, 255, 255, 255]);
    }

    #[test]
    fn without_the_add_on_a_job_fails_at_once_and_the_clip_stays_unkeyed() {
        let empty = scratch("not-installed");
        let req = Request {
            src: "C:/nowhere/clip.mp4".into(),
            still: false,
            fps: 25.0,
            size: (640, 360),
            frames: (0, 25),
            quality: BgQuality::Fast,
            ep: "auto".into(),
        };
        let prog = Progress::new();
        let t0 = std::time::Instant::now();
        let e = run_in(&empty, &empty.join("cache"), &req, &prog, &|| {}).unwrap_err();
        assert!(e.contains("not installed"), "{e}");
        assert!(t0.elapsed() < std::time::Duration::from_secs(2), "nothing was started");
        assert!(!empty.join("cache").exists(), "no cache folder made");
        // the playback side: no matte = None = the clip draws unkeyed
        assert!(matte_frame("C:/nowhere/clip.mp4", BgQuality::Fast, 25.0, 0.5, false, 0.0).is_none());
        let _ = std::fs::remove_dir_all(&empty);
    }

    /// A 1 s 640x360 clip at 25 fps: a 140 px yellow square moving right over moving noise. Returns the
    /// file and the square's box (x0, y0, x1, y1) on frame `k`.
    fn shape_clip(d: &Path) -> (PathBuf, impl Fn(usize) -> (usize, usize, usize, usize)) {
        let ff = ffpipe::ffmpeg_exe().expect("ffmpeg.exe");
        let out = d.join("shape.mp4");
        let st = std::process::Command::new(ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=c=0x507090:s=640x360:r=25,noise=alls=90:allf=t+u,format=yuv420p"])
            .args(["-f", "lavfi", "-i", "color=c=yellow:s=140x140:r=25"])
            .args(["-filter_complex", "[0][1]overlay=x=100+100*t:y=110:shortest=1,format=yuv420p"])
            .args(["-t", "1", "-c:v", "libx264", "-crf", "12"])
            .arg(&out)
            .status()
            .unwrap();
        assert!(st.success());
        (out, |k: usize| {
            let x0 = 100 + (100.0 * k as f64 / 25.0) as usize;
            (x0, 110, x0 + 140, 250)
        })
    }

    /// Intersection over union of the matte (thresholded at half) and the square.
    fn iou(m: &[u8], w: usize, b: (usize, usize, usize, usize)) -> f64 {
        let (mut inter, mut uni) = (0usize, 0usize);
        for (i, &v) in m.iter().enumerate() {
            let (x, y) = (i % w, i / w);
            let inside = x >= b.0 && x < b.2 && y >= b.1 && y < b.3;
            let on = v >= 128;
            inter += (on && inside) as usize;
            uni += (on || inside) as usize;
        }
        inter as f64 / uni.max(1) as f64
    }

    /// `SE_BGREMOVE_DIR=<installed add-on> TMP=G:\se-tmp cargo test bgremove_real_run -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bgremove_real_run_finds_the_shape_over_a_busy_background() {
        let addon = dir();
        assert!(available(BgQuality::Best), "install the add-on (or set SE_BGREMOVE_DIR) first: {}", addon.display());
        let d = scratch("real");
        let (clip, square) = shape_clip(&d);
        for q in [BgQuality::Best, BgQuality::Fast] {
            if !available(q) {
                continue;
            }
            let cache = d.join(q.id());
            let req = Request {
                src: clip.to_string_lossy().into_owned(),
                still: false,
                fps: 25.0,
                size: (640, 360),
                frames: (0, 25),
                quality: q,
                ep: "auto".into(),
            };
            let prog = Progress::new();
            let landed = std::sync::atomic::AtomicUsize::new(0);
            let rep = run_in(&addon, &cache, &req, &prog, &|| {
                landed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            })
            .unwrap();
            assert_eq!(rep.frames, 25);
            assert!(landed.load(std::sync::atomic::Ordering::Relaxed) >= 1);
            let mut scores = Vec::new();
            for k in 0..25 {
                let (w, h, m) = read_matte(&cache, k as i64).expect("every frame cached");
                assert_eq!((w, h), matte_size(640, 360, q));
                scores.push(iou(&m, w as usize, square(k)));
            }
            let mean = scores.iter().sum::<f64>() / scores.len() as f64;
            let worst = scores.iter().cloned().fold(1.0, f64::min);
            println!("{}: mean IoU {mean:.3}, worst {worst:.3}, {:.0} ms a frame on {}", q.id(), rep.ms_per_frame, rep.ep);
            // a second run finds every frame cached and starts nothing
            let again = run_in(&addon, &cache, &req, &Progress::new(), &|| {}).unwrap();
            assert_eq!(again.frames, 0);
            if q == BgQuality::Best {
                assert!(mean > 0.9 && worst > 0.8, "BiRefNet should find the square: {scores:?}");
            }
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Speed per frame of both models on the GPU (DirectML) and the CPU, from a 1080p source:
    /// `SE_BGREMOVE_DIR=… TMP=G:\se-tmp cargo test bench_bgremove -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_bgremove_models() {
        let addon = dir();
        let d = scratch("bench");
        let ff = ffpipe::ffmpeg_exe().expect("ffmpeg.exe");
        let clip = d.join("bench.mp4");
        let st = std::process::Command::new(ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=1920x1080:r=30", "-t", "2", "-c:v", "libx264"])
            .arg(&clip)
            .status()
            .unwrap();
        assert!(st.success());
        let n: i64 = std::env::var("SE_BENCH_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(30);
        for q in [BgQuality::Fast, BgQuality::Best] {
            for ep in ["dml", "cpu"] {
                if !available(q) {
                    continue;
                }
                let req = Request {
                    src: clip.to_string_lossy().into_owned(),
                    still: false,
                    fps: 30.0,
                    size: (1920, 1080),
                    frames: (0, n.min(60)),
                    quality: q,
                    ep: ep.into(),
                };
                let cache = d.join(format!("{}-{ep}", q.id()));
                match run_in(&addon, &cache, &req, &Progress::new(), &|| {}) {
                    Ok(r) => println!(
                        "bench {} on {ep}: {} frames at {:?}, {:.1} ms a frame (got {})",
                        q.id(),
                        r.frames,
                        matte_size(1920, 1080, q),
                        r.ms_per_frame,
                        r.ep
                    ),
                    Err(e) => println!("bench {} on {ep}: failed: {e}", q.id()),
                }
            }
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
