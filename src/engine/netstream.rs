//! ws:parity-io: network streams (HTTP / HLS / DASH / RTMP / RTSP / SRT / UDP / RTP) through ffmpeg.
//!
//! Three things, each on its own thread so the UI never waits on the network:
//!  * `probe` - what is there (codecs, size, live or not), via `engine::media_info::properties`.
//!  * `Preview` - a small live picture (about 12 fps, 640 px wide) decoded by an ffmpeg child; the UI only
//!    uploads the newest frame when one arrives.
//!  * `Recording` - a stream copy into a local file (no re-encode, so it keeps up with any bitrate), stopped
//!    on request (ffmpeg's own `q`, so the file is finalised) or after a set length; the app then imports it
//!    into the Library like a screen recording.

use crate::media::{ffpipe, Frame};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The URL schemes ffmpeg can open as a stream.
pub const SCHEMES: &[&str] = &["http", "https", "rtmp", "rtmps", "rtsp", "rtsps", "srt", "udp", "rtp", "tcp", "mms", "mmsh"];

/// A trimmed URL ffmpeg can read, or why not. Local paths and `file:` are refused (that's Import Media).
pub fn validate(url: &str) -> Result<String, String> {
    let u = url.trim();
    if u.is_empty() {
        return Err("Enter a stream address".into());
    }
    let Some((scheme, rest)) = u.split_once("://") else {
        return Err("The address needs a protocol, like https:// or rtsp://".into());
    };
    let scheme = scheme.to_ascii_lowercase();
    if !SCHEMES.contains(&scheme.as_str()) {
        return Err(format!("{scheme}:// streams aren't supported (try {})", SCHEMES[..6].join(", ")));
    }
    if rest.is_empty() || (rest.starts_with('/') && scheme != "udp" && scheme != "rtp") {
        return Err("The address has no host".into());
    }
    if u.chars().any(|c| c.is_control()) {
        return Err("The address has a line break or control character in it".into());
    }
    Ok(u.to_string())
}

/// Input options for a URL: TCP for RTSP (UDP drops packets through most routers), and reconnects for
/// HTTP so a live HLS playlist that hiccups keeps going.
pub fn input_args(url: &str) -> Vec<String> {
    let scheme = url.split_once("://").map(|(s, _)| s.to_ascii_lowercase()).unwrap_or_default();
    let mut a: Vec<&str> = Vec::new();
    match scheme.as_str() {
        "rtsp" | "rtsps" => a.extend(["-rtsp_transport", "tcp"]),
        "http" | "https" => a.extend(["-reconnect", "1", "-reconnect_streamed", "1", "-reconnect_delay_max", "5"]),
        _ => {}
    }
    a.extend(["-rw_timeout", "15000000"]);
    let mut out: Vec<String> = a.into_iter().map(String::from).collect();
    out.extend(["-i".into(), url.into()]);
    out
}

/// The record command's arguments: every video and audio stream copied as they come into `out` (Matroska
/// takes any codec a stream can carry), stopping after `secs` when given.
pub fn record_args(url: &str, out: &Path, secs: Option<f64>) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-loglevel", "error", "-y"].map(String::from).to_vec();
    a.extend(input_args(url));
    a.extend(["-map", "0:v?", "-map", "0:a?", "-c", "copy"].map(String::from));
    if let Some(s) = secs.filter(|s| *s > 0.0) {
        a.extend(["-t".to_string(), format!("{s:.3}")]);
    }
    a.push(out.to_string_lossy().into_owned());
    a
}

/// Where a recording of `url` goes: `dir/<host> <date time>.mkv`, never over an existing file.
pub fn record_path(dir: &Path, url: &str, stamp: &str) -> PathBuf {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default().split(':').next().filter(|h| !h.is_empty()).unwrap_or("stream");
    let host: String = host.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    let base = format!("{host} {stamp}");
    let mut p = dir.join(format!("{base}.mkv"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{base} ({n}).mkv"));
        n += 1;
    }
    p
}

// ------------------------------------------------------------------------------------------- preview

/// Preview picture width.
pub const PREVIEW_W: u32 = 640;

/// A live preview: an ffmpeg child scaled to `PREVIEW_W` at a low rate; the newest frame waits in `latest`.
pub struct Preview {
    child: Arc<Mutex<Option<Child>>>,
    latest: Arc<Mutex<Option<Frame>>>,
    error: Arc<Mutex<Option<String>>>,
}

impl Preview {
    /// `wake` runs on the reader thread after each frame (request a repaint).
    pub fn start(url: &str, wake: impl Fn() + Send + 'static) -> Result<Preview, String> {
        let exe = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
        let h = PREVIEW_W * 9 / 16;
        let vf = format!("fps=12,scale={PREVIEW_W}:{h}:force_original_aspect_ratio=decrease,pad={PREVIEW_W}:{h}:-1:-1");
        let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-fflags", "nobuffer"].map(String::from).to_vec();
        args.extend(input_args(url));
        args.extend(["-map", "0:v:0", "-an", "-sn", "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"].map(String::from));
        let mut child = ffpipe::command(&exe)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("ffmpeg: {e}"))?;
        let (mut out, mut err) = (child.stdout.take().ok_or("no stdout")?, child.stderr.take());
        let p = Preview { child: Arc::new(Mutex::new(Some(child))), latest: Default::default(), error: Default::default() };
        let (latest, error) = (p.latest.clone(), p.error.clone());
        std::thread::spawn(move || {
            crate::media::bg::lower_thread();
            let mut buf = vec![0u8; (PREVIEW_W * h * 4) as usize];
            while out.read_exact(&mut buf).is_ok() {
                let mut f = Frame::new(PREVIEW_W, h);
                f.rgba.copy_from_slice(&buf);
                *latest.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
                wake();
            }
            let mut msg = String::new();
            if let Some(e) = err.as_mut() {
                let _ = e.read_to_string(&mut msg);
            }
            let msg = msg.lines().last().unwrap_or("the stream ended").trim().to_string();
            *error.lock().unwrap_or_else(|e| e.into_inner()) = Some(msg);
            wake();
        });
        Ok(p)
    }
    /// The newest frame since the last call.
    pub fn take(&self) -> Option<Frame> {
        self.latest.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
    /// Why the preview stopped (None while it runs).
    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = c.kill();
            std::thread::spawn(move || {
                let _ = c.wait();
            });
        }
    }
}

// ----------------------------------------------------------------------------------------- recording

/// A stream being copied to disk.
pub struct Recording {
    pub url: String,
    pub out: PathBuf,
    pub started: Instant,
    pub limit: Option<f64>,
    stdin: Option<ChildStdin>,
    /// None while running; Some(error text, "" = fine) once ffmpeg has exited.
    done: Arc<Mutex<Option<String>>>,
}

impl Recording {
    pub fn start(url: &str, out: PathBuf, limit: Option<f64>) -> Result<Recording, String> {
        let exe = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
        if let Some(d) = out.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        let mut child = ffpipe::command(&exe)
            .args(record_args(url, &out, limit))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("ffmpeg: {e}"))?;
        let stdin = child.stdin.take();
        let done: Arc<Mutex<Option<String>>> = Default::default();
        let d = done.clone();
        std::thread::spawn(move || {
            let mut msg = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = e.read_to_string(&mut msg);
            }
            let ok = child.wait().map(|s| s.success()).unwrap_or(false);
            let msg = if ok { String::new() } else { msg.lines().last().unwrap_or("ffmpeg failed").trim().to_string() };
            *d.lock().unwrap_or_else(|e| e.into_inner()) = Some(msg);
        });
        Ok(Recording { url: url.to_string(), out, started: Instant::now(), limit, stdin, done })
    }
    /// Ask ffmpeg to finish the file (its `q` key); `finished` reports when it has.
    pub fn stop(&mut self) {
        if let Some(mut s) = self.stdin.take() {
            let _ = s.write_all(b"q");
        }
    }
    pub fn stopping(&self) -> bool {
        self.stdin.is_none()
    }
    /// Some(Ok) once the file is complete, Some(Err) when ffmpeg failed (a file with data is still kept).
    pub fn finished(&self) -> Option<Result<(), String>> {
        let d = self.done.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
        let has_data = std::fs::metadata(&self.out).map(|m| m.len() > 0).unwrap_or(false);
        Some(if d.is_empty() || has_data { Ok(()) } else { Err(d) })
    }
    pub fn bytes(&self) -> u64 {
        std::fs::metadata(&self.out).map(|m| m.len()).unwrap_or(0)
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Whether a probed stream is live: no duration, or HLS/DASH reporting none.
pub fn is_live(info: &crate::engine::media_info::MediaInfo) -> bool {
    !(info.duration.is_finite() && info.duration > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_schemes_and_hosts() {
        assert_eq!(validate("  https://example.com/live.m3u8 ").unwrap(), "https://example.com/live.m3u8");
        assert!(validate("rtsp://192.168.1.20:554/stream1").is_ok());
        assert!(validate("udp://@:1234").is_ok());
        assert!(validate("C:\\videos\\a.mp4").is_err(), "a path is Import Media");
        assert!(validate("file:///c:/a.mp4").is_err());
        assert!(validate("ftp://x/y").unwrap_err().contains("ftp://"));
        assert!(validate("https://").is_err());
        assert!(validate("https://a.com/x\ny").is_err());
        assert!(validate("").is_err());
    }

    #[test]
    fn record_command_copies_and_limits() {
        let a = record_args("rtsp://cam/1", Path::new("C:/r/cam.mkv"), Some(30.0)).join(" ");
        assert_eq!(
            a,
            "-hide_banner -loglevel error -y -rtsp_transport tcp -rw_timeout 15000000 -i rtsp://cam/1 \
             -map 0:v? -map 0:a? -c copy -t 30.000 C:/r/cam.mkv"
        );
        let h = record_args("https://a.com/p.m3u8", Path::new("o.mkv"), None).join(" ");
        assert!(h.contains("-reconnect 1") && !h.contains(" -t "), "{h}");
    }

    #[test]
    fn record_paths_are_named_after_the_host_and_never_clash() {
        let dir = std::env::temp_dir().join(format!("se_netstream_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = record_path(&dir, "rtsp://user@cam.local:554/live", "2026-11-20 10.00.00");
        assert_eq!(p.file_name().unwrap().to_string_lossy(), "cam.local 2026-11-20 10.00.00.mkv");
        std::fs::write(&p, b"x").unwrap();
        let q = record_path(&dir, "rtsp://user@cam.local:554/live", "2026-11-20 10.00.00");
        assert_eq!(q.file_name().unwrap().to_string_lossy(), "cam.local 2026-11-20 10.00.00 (2).mkv");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
