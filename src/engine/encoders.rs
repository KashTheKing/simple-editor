//! ws:export-encoders: which hardware encoders actually run on this machine.
//!
//! `ffmpeg -encoders` lists NVENC/QSV/AMF whenever ffmpeg was built with them, not when the GPU and
//! driver can use them. `start()` test-encodes one black frame per listed candidate on a background
//! thread, once per (ffmpeg path, modified time), and caches the working set in memory and in
//! `Settings::cache_dir()/encoders.json`. Everything here is non-blocking for callers: until the probe
//! finishes, `working()` holds what is known so far and "auto" stays on software.

use crate::engine::export::detect_encoders;
use crate::media::ffpipe;
use crate::settings::Settings;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant, UNIX_EPOCH};

/// Probe order = auto preference: NVENC, then QSV, then AMF.
pub const CANDIDATES: [&str; 9] = [
    "h264_nvenc",
    "hevc_nvenc",
    "av1_nvenc",
    "h264_qsv",
    "hevc_qsv",
    "av1_qsv",
    "h264_amf",
    "hevc_amf",
    "av1_amf",
];

struct State {
    key: String,
    working: Vec<String>,
    done: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn lock() -> std::sync::MutexGuard<'static, Option<State>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn cache_file() -> PathBuf {
    Settings::cache_dir().join("encoders.json")
}

/// `<ffmpeg path>|<mtime secs>`: a new or replaced ffmpeg re-probes.
fn key_for(exe: &Path) -> String {
    let mtime = std::fs::metadata(exe)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}|{mtime}", exe.display())
}

/// The cached working list when the file's key matches.
fn parse_cache(text: &str, key: &str) -> Option<Vec<String>> {
    let v: Value = serde_json::from_str(text).ok()?;
    if v.get("key")?.as_str()? != key {
        return None;
    }
    Some(v.get("working")?.as_array()?.iter().filter_map(|e| e.as_str().map(String::from)).collect())
}

/// Start the probe (no-op when it already ran or is running for this ffmpeg). Call at app start and
/// when the Export pane opens; it never blocks.
pub fn start() {
    let Some(exe) = ffpipe::ffmpeg_exe() else {
        return;
    };
    let key = key_for(&exe);
    {
        let mut st = lock();
        if st.as_ref().is_some_and(|s| s.key == key) {
            return;
        }
        if let Some(working) = std::fs::read_to_string(cache_file()).ok().and_then(|t| parse_cache(&t, &key)) {
            *st = Some(State { key, working, done: true });
            return;
        }
        *st = Some(State { key: key.clone(), working: Vec::new(), done: false });
    }
    std::thread::spawn(move || {
        let listed = detect_encoders();
        for enc in CANDIDATES.iter().filter(|c| listed.iter().any(|l| l == *c)) {
            let ok = test_encode(&exe, enc);
            let mut st = lock();
            match st.as_mut() {
                Some(s) if s.key == key => {
                    if ok {
                        s.working.push(enc.to_string());
                    }
                }
                _ => return, // ffmpeg changed under us; a newer probe owns the state
            }
        }
        let mut st = lock();
        if let Some(s) = st.as_mut().filter(|s| s.key == key) {
            s.done = true;
            let _ = std::fs::create_dir_all(Settings::cache_dir());
            let json = serde_json::json!({ "key": s.key, "working": s.working });
            let _ = std::fs::write(cache_file(), json.to_string());
        }
    });
}

/// One-frame null encode; a hung driver is killed after 15 s and counts as broken.
fn test_encode(exe: &Path, enc: &str) -> bool {
    let child = ffpipe::command(exe)
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=black:s=256x144:r=30"])
        .args(["-frames:v", "1", "-c:v", enc, "-f", "null", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return false;
    };
    let t0 = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if t0.elapsed() < Duration::from_secs(15) => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Hardware encoders known to work so far (empty until the probe has tested something).
pub fn working() -> Vec<String> {
    lock().as_ref().map(|s| s.working.clone()).unwrap_or_default()
}

/// True once every candidate has been tested (or the cache answered).
pub fn done() -> bool {
    lock().as_ref().is_some_and(|s| s.done)
}

/// True for a hardware encoder the finished probe found unable to run (hide it, fall back to auto).
pub fn is_broken(enc: &str) -> bool {
    let st = lock();
    st.as_ref().is_some_and(|s| s.done && CANDIDATES.contains(&enc) && !s.working.iter().any(|w| w == enc))
}

/// Fastest working encoder for "auto" on this container, None = the software default.
pub fn best_for(ext: &str) -> Option<String> {
    pick(ext, &working()).map(String::from)
}

/// Pure part of `best_for`: H.264 only (plays everywhere), only for mp4/mov/m4v/mkv, in probe order.
pub fn pick<'a>(ext: &str, working: &'a [String]) -> Option<&'a str> {
    if !matches!(ext, "mp4" | "mov" | "m4v" | "mkv") {
        return None;
    }
    ["h264_nvenc", "h264_qsv", "h264_amf"]
        .into_iter()
        .find_map(|c| working.iter().find(|w| w.as_str() == c).map(String::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn pick_prefers_nvenc_then_qsv_then_amf_h264_only() {
        assert_eq!(pick("mp4", &v(&["h264_amf", "h264_qsv", "h264_nvenc"])), Some("h264_nvenc"));
        assert_eq!(pick("mkv", &v(&["h264_amf", "h264_qsv"])), Some("h264_qsv"));
        assert_eq!(pick("mov", &v(&["hevc_nvenc", "h264_amf"])), Some("h264_amf"));
        assert_eq!(pick("mp4", &v(&["hevc_nvenc", "av1_nvenc"])), None, "auto never picks hevc/av1");
        assert_eq!(pick("webm", &v(&["h264_nvenc"])), None);
        assert_eq!(pick("gif", &v(&["h264_nvenc"])), None);
        assert_eq!(pick("avi", &v(&["h264_nvenc"])), None);
        assert_eq!(pick("mp4", &[]), None, "probe not finished = software");
    }

    #[test]
    fn cache_file_matches_key_only() {
        let text = serde_json::json!({ "key": "C:/ff.exe|12", "working": ["h264_nvenc"] }).to_string();
        assert_eq!(parse_cache(&text, "C:/ff.exe|12"), Some(v(&["h264_nvenc"])));
        assert_eq!(parse_cache(&text, "C:/ff.exe|13"), None, "a replaced ffmpeg re-probes");
        assert_eq!(parse_cache("not json", "k"), None);
    }
}
