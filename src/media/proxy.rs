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
use std::sync::Arc;

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
    export::spawn_job("proxy", move |prog| run(&src, &dst, height, prog))
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
            }
        }
    }
    export::wait_ffmpeg(&mut child, tail, prog)?;
    tmp.commit(dst.as_path())
}

#[cfg(test)]
mod tests {
    use super::*;

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
