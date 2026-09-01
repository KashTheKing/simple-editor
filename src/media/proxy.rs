//! Proxy media: background all-intra low-res transcodes of imported video, played instead of the
//! originals in the preview. Every proxy frame is a keyframe (`-g 1`), so seeks, reverse scrubs and
//! clip-boundary cold opens decode without reference chains — the reason big NLEs feel instant.
//! Proxies live in the cache dir, named by a hash of (source path, mtime, height): a re-exported
//! source gets a fresh proxy automatically and stale files are just never referenced again.
//! Export and full-quality one-shot renders never see proxies (their DecoderPools carry no map).

use crate::engine::export::{self, Progress};
use crate::media::ffpipe;
use std::io::BufRead;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The proxy build currently running: (source path, fraction 0..1). Owned entirely by the build job
/// (set on entry, fraction updated from ffmpeg progress, cleared by a drop guard so an error or
/// panic can never leave a stuck "building" badge). Read from UI leaf code — library rows, the
/// inspector's Asset block, the preview badge — which has no channel to `App` state; same shape as
/// `engine::import::is_probing`.
static BUILDING: Mutex<Option<(String, f32)>> = Mutex::new(None);
/// Source paths (lowercased) whose proxy is built and in use, as last pushed by `App::sync_proxies`
/// — pushed in the same breath as the player's proxy map, so badges and playback agree.
static READY: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// What the proxy pipeline is doing for one asset — drives the per-asset badges.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ProxyStatus {
    /// Not video, already at/below proxy height, zero-length, or proxies are off.
    NotNeeded,
    /// Eligible, waiting its turn (builds run one at a time).
    Queued,
    /// The transcode running right now (fraction 0..1).
    Building(f32),
    Ready,
}

/// The build in flight, if any: (source path, fraction 0..1).
pub fn building() -> Option<(String, f32)> {
    lock(&BUILDING).clone()
}

/// `App::sync_proxies` publishes which sources currently play from a proxy (call sites: the normal
/// scan push and the regenerate path — both, or badges lie for up to one 2 s scan).
pub fn set_ready(sources: Vec<String>) {
    *lock(&READY) = sources.into_iter().map(|s| s.to_ascii_lowercase()).collect();
}

/// Per-asset proxy state. The eligibility gate mirrors `App::sync_proxies` verbatim so the badge
/// and the builder can't drift.
pub fn status(a: &crate::model::Asset, use_proxies: bool, proxy_height: u32) -> ProxyStatus {
    let h = proxy_height.max(120);
    if !use_proxies || a.kind != crate::model::ClipKind::Video || a.height <= h || a.duration <= 0.0 {
        return ProxyStatus::NotNeeded;
    }
    if let Some((p, f)) = lock(&BUILDING).as_ref() {
        if p.eq_ignore_ascii_case(&a.path) {
            return ProxyStatus::Building(*f);
        }
    }
    if lock(&READY).iter().any(|r| r == &a.path.to_ascii_lowercase()) {
        return ProxyStatus::Ready;
    }
    ProxyStatus::Queued
}

/// Clears the BUILDING badge when the job ends, however it ends (ok / ffmpeg error / panic).
struct BuildingGuard(String);
impl BuildingGuard {
    fn set(src: &str) -> Self {
        *lock(&BUILDING) = Some((src.to_string(), 0.0));
        BuildingGuard(src.to_string())
    }
}
impl Drop for BuildingGuard {
    fn drop(&mut self) {
        let mut b = lock(&BUILDING);
        if b.as_ref().is_some_and(|(p, _)| *p == self.0) {
            *b = None;
        }
    }
}

fn set_building_fraction(src: &str, f: f32) {
    if let Some((p, frac)) = lock(&BUILDING).as_mut() {
        if p == src {
            *frac = f;
        }
    }
}

pub fn dir() -> PathBuf {
    crate::settings::Settings::cache_dir().join("proxies")
}

/// Where the proxy for `src` at `height` lives (whether or not it has been built yet).
pub fn proxy_path(src: &str, height: u32) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mtime = std::fs::metadata(src)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (src.to_ascii_lowercase(), mtime, height).hash(&mut h);
    dir().join(format!("{:016x}.mp4", h.finish()))
}

/// Transcode `src` into its proxy file on a background thread (temp + rename; the destination never
/// exists half-written). Video only — audio always plays from the original.
pub fn generate(src: String, dst: PathBuf, height: u32) -> Arc<Progress> {
    export::spawn_job("proxy", move |prog| {
        let _badge = BuildingGuard::set(&src); // set + cleared inside the job: no startup race
        run(&src, &dst, height, prog)
    })
}

fn run(src: &str, dst: &PathBuf, height: u32, prog: &Progress) -> Result<(), String> {
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    std::fs::create_dir_all(dir()).map_err(|e| format!("proxies dir: {e}"))?;
    let dur = crate::engine::convert::probe_seconds(std::path::Path::new(src)).unwrap_or(0.0);
    let tmp = export::temp_output(dst);
    let mut cmd = ffpipe::command(&ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-progress", "pipe:1"]);
    cmd.arg("-i").arg(src);
    // -2 keeps aspect at an even width; -g 1 = all-intra; no audio (the original supplies it)
    let vf = format!("scale=-2:{}", height.max(120));
    cmd.args(["-vf", &vf, "-c:v", "libx264", "-preset", "veryfast", "-g", "1", "-crf", "20"]);
    cmd.args(["-pix_fmt", "yuv420p", "-an", "-movflags", "+faststart"]);
    cmd.arg(&tmp.0);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
    let tail = export::stderr_tail(&mut child);
    if let Some(out) = child.stdout.take() {
        prog.set(0.0, "Building proxy…");
        for line in std::io::BufReader::new(out).lines() {
            let Ok(line) = line else { break };
            if prog.is_cancelled() {
                let _ = child.kill();
                break;
            }
            if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                let f = if dur > 0.0 { (us / 1e6 / dur).clamp(0.0, 1.0).min(0.99) as f32 } else { 0.0 };
                prog.set(f, "Building proxy…");
                set_building_fraction(src, f);
            }
        }
    }
    export::wait_ffmpeg(&mut child, tail, prog)?;
    tmp.commit(dst.as_path())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test for the whole ladder (BUILDING/READY are process-global statics — keeping every
    /// assertion in one test avoids parallel-test interference on them).
    #[test]
    fn status_classifies_the_pipeline() {
        use crate::model::{Asset, ClipKind};
        let a = Asset {
            id: 1,
            path: "Z:\\proxy-status-test.mp4".into(),
            kind: ClipKind::Video,
            duration: 10.0,
            width: 3840,
            height: 2160,
            fps: 30.0,
            audio_streams: Vec::new(),
            codec: "hevc".into(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
        };
        assert_eq!(status(&a, false, 720), ProxyStatus::NotNeeded, "proxies off");
        let small = Asset { height: 720, ..a.clone() };
        assert_eq!(status(&small, true, 720), ProxyStatus::NotNeeded, "at/below proxy height");
        let audio = Asset { kind: ClipKind::Audio, ..a.clone() };
        assert_eq!(status(&audio, true, 720), ProxyStatus::NotNeeded, "audio never proxies");
        assert_eq!(status(&a, true, 720), ProxyStatus::Queued, "eligible, not building, not ready");

        {
            let _g = BuildingGuard::set(&a.path);
            set_building_fraction(&a.path, 0.25);
            assert_eq!(status(&a, true, 720), ProxyStatus::Building(0.25));
        }
        assert_eq!(status(&a, true, 720), ProxyStatus::Queued, "the drop guard cleared the badge");

        set_ready(vec![a.path.to_uppercase()]);
        assert_eq!(status(&a, true, 720), ProxyStatus::Ready, "ready is case-insensitive");
        set_ready(Vec::new());
        assert_eq!(status(&a, true, 720), ProxyStatus::Queued);
    }

    /// The proxy name is stable for the same (path, mtime, height) and changes with any of them.
    #[test]
    fn proxy_path_is_deterministic() {
        let a = proxy_path("C:\\missing\\clip.mp4", 720);
        assert_eq!(a, proxy_path("C:\\missing\\clip.mp4", 720));
        assert_eq!(a, proxy_path("C:\\MISSING\\CLIP.mp4", 720), "case-insensitive paths hash alike");
        assert_ne!(a, proxy_path("C:\\missing\\clip.mp4", 540));
        assert_ne!(a, proxy_path("C:\\missing\\other.mp4", 720));
        assert!(a.extension().is_some_and(|e| e == "mp4"));
    }
}
