//! ws:export-encoders: which hardware encoders actually run on this machine.
//!
//! `ffmpeg -encoders` lists NVENC/QSV/AMF whenever ffmpeg was built with them, not when the GPU and
//! driver can use them. `start()` test-encodes one black frame per listed candidate on a background
//! thread, once per (ffmpeg path, modified time), and caches the working set in memory and in
//! `Settings::cache_dir()/encoders.json`. Everything here is non-blocking for callers: until the probe
//! finishes, `working()` holds what is known so far and "auto" stays on software.
//!
//! forum-crash: each test runs in its own ffmpeg child process, so a driver fault kills ffmpeg, not the
//! editor. The cache is keyed by the GPU driver versions too (a driver update re-tests), and progress is
//! written before each test: if the machine or the app went down during one (`testing` still set on the
//! next start), that encoder counts as broken and is never tried again for this driver.

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

/// `<ffmpeg path>|<mtime secs>|<drivers>`: a new or replaced ffmpeg, or a driver update, re-probes.
fn key_for(exe: &Path) -> String {
    // read once per run (callers are background jobs; DXGI takes a few ms)
    static DRIVERS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    format!("{}|{}", ffmpeg_key(exe), DRIVERS.get_or_init(driver_key))
}

fn ffmpeg_key(exe: &Path) -> String {
    let mtime = std::fs::metadata(exe)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}|{mtime}", exe.display())
}

/// Every hardware adapter as `vendor:device:driver version` (DXGI's user-mode driver version), sorted.
/// "" when DXGI is unavailable. Called on the probe thread or once at start; a few milliseconds.
#[cfg(windows)]
pub fn driver_key() -> String {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIDevice, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
    use windows::core::Interface;
    let mut parts = Vec::new();
    unsafe {
        let Ok(f) = CreateDXGIFactory1::<IDXGIFactory1>() else { return String::new() };
        let mut i = 0;
        while let Ok(a) = f.EnumAdapters1(i) {
            i += 1;
            let Ok(d) = a.GetDesc1() else { continue };
            if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let ver = a.CheckInterfaceSupport(&IDXGIDevice::IID).unwrap_or(0);
            parts.push(format!("{:04x}:{:04x}:{ver:x}", d.VendorId, d.DeviceId));
        }
    }
    parts.sort();
    parts.dedup();
    parts.join(",")
}

#[cfg(not(windows))]
pub fn driver_key() -> String {
    String::new()
}

/// What the cache file knows for `key`: the working encoders, the ones already tested, and whether
/// every candidate was tested.
#[derive(Debug, PartialEq, Default)]
struct Known {
    working: Vec<String>,
    tested: Vec<String>,
    done: bool,
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|e| e.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

/// The cache when the file's key matches. A file from before progress was recorded has no `done`:
/// it was only ever written finished. An encoder still marked `testing` took the app or the machine
/// down last time: it counts as tested and broken.
fn parse_cache(text: &str, key: &str) -> Option<Known> {
    let v: Value = serde_json::from_str(text).ok()?;
    if v.get("key")?.as_str()? != key {
        return None;
    }
    let working = strings(v.get("working"));
    v.get("working")?.as_array()?;
    let done = v.get("done").and_then(Value::as_bool).unwrap_or(true);
    let mut tested = strings(v.get("tested"));
    if let Some(t) = v.get("testing").and_then(Value::as_str) {
        if !tested.iter().any(|x| x == t) {
            tested.push(t.to_string());
        }
    }
    Some(Known { working, tested, done })
}

fn write_cache(key: &str, k: &Known, testing: Option<&str>) {
    let _ = std::fs::create_dir_all(Settings::cache_dir());
    let json = serde_json::json!({ "key": key, "working": k.working, "tested": k.tested, "done": k.done, "testing": testing });
    let _ = std::fs::write(cache_file(), json.to_string());
}

/// Start the probe (no-op when it already ran or is running for this ffmpeg). Call at app start and
/// when the Export pane opens; it never blocks.
pub fn start() {
    let Some(exe) = ffpipe::ffmpeg_exe() else {
        return;
    };
    let key = key_for(&exe);
    let resume: Known;
    {
        let mut st = lock();
        if st.as_ref().is_some_and(|s| s.key == key) {
            return;
        }
        let known = std::fs::read_to_string(cache_file()).ok().and_then(|t| parse_cache(&t, &key)).unwrap_or_default();
        if known.done {
            *st = Some(State { key, working: known.working, done: true });
            return;
        }
        *st = Some(State { key: key.clone(), working: known.working.clone(), done: false });
        resume = known;
    }
    std::thread::spawn(move || {
        let mut k = resume;
        let listed = detect_encoders();
        for enc in CANDIDATES.iter().filter(|c| listed.iter().any(|l| l == *c)) {
            if k.tested.iter().any(|t| t == enc) {
                continue; // answered by an earlier run (or it went down testing it)
            }
            write_cache(&key, &k, Some(enc)); // on disk before the risky part
            let ok = {
                let _inflight = crate::crashguard::begin(crate::crashguard::Kind::EncoderTest, "", enc);
                test_encode(&exe, enc)
            };
            k.tested.push(enc.to_string());
            if ok {
                k.working.push(enc.to_string());
            }
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
        k.done = true;
        let mut st = lock();
        if let Some(s) = st.as_mut().filter(|s| s.key == key) {
            s.done = true;
            k.working = s.working.clone();
            write_cache(&key, &k, None);
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

/// Drop `enc` from the working set (an export found it dead after a GPU/driver change) and rewrite
/// the cache file, so "auto" stops picking it until ffmpeg changes or the cache is cleared.
/// ponytail: no machine key in the cache (the GPU name lives with the UI's GL context); this is the
/// self-heal instead.
pub fn mark_broken(enc: &str) {
    let mut st = lock();
    let Some(s) = st.as_mut() else { return };
    s.working.retain(|w| w != enc);
    if s.done {
        let tested = CANDIDATES.iter().map(|c| c.to_string()).collect();
        write_cache(&s.key, &Known { working: s.working.clone(), tested, done: true }, None);
    }
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
        // a file from before progress was recorded: written only when finished
        let text = serde_json::json!({ "key": "C:/ff.exe|12|10de:2484:1f0", "working": ["h264_nvenc"] }).to_string();
        let k = parse_cache(&text, "C:/ff.exe|12|10de:2484:1f0").unwrap();
        assert_eq!((k.working, k.done), (v(&["h264_nvenc"]), true));
        assert_eq!(parse_cache(&text, "C:/ff.exe|13|10de:2484:1f0"), None, "a replaced ffmpeg re-probes");
        assert_eq!(parse_cache(&text, "C:/ff.exe|12|10de:2484:200"), None, "a driver update re-probes");
        assert_eq!(parse_cache("not json", "k"), None);
    }

    /// forum-crash: the machine went down while testing NVENC: the next start skips it (broken) and
    /// carries on with the encoders not yet tested.
    #[test]
    fn an_encoder_that_took_the_machine_down_is_never_retried() {
        let text = serde_json::json!({
            "key": "k", "working": ["h264_qsv"], "tested": ["h264_qsv"], "done": false, "testing": "h264_nvenc"
        })
        .to_string();
        let k = parse_cache(&text, "k").unwrap();
        assert!(!k.done);
        assert_eq!(k.working, v(&["h264_qsv"]));
        assert_eq!(k.tested, v(&["h264_qsv", "h264_nvenc"]), "the one in flight counts as tested");
        assert!(!k.working.iter().any(|w| w == "h264_nvenc"), "and not working");
    }
}
