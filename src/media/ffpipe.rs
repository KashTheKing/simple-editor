//! ffmpeg.exe / ffprobe.exe child-process backend. Universal fallback decoder, image loader, prober,
//! and the process launcher used by export.
//!
//! Contract (see media/mod.rs):
//!  * `ffmpeg_exe()` / `ffprobe_exe()` locate the binaries: Settings.ffmpeg_dir (set via `set_dir`),
//!    then next to the app exe, then `ffmpeg/` beside the exe, then PATH. Cached.
//!  * `command(exe)` returns a std Command with CREATE_NO_WINDOW (0x08000000) so no console flashes.
//!  * `probe(path)` -> Asset via `ffprobe -v error -print_format json -show_streams -show_format`.
//!  * `open_video(path)` -> VideoSource: `ffmpeg -ss T -i path -map 0:v:0 -f rawvideo -pix_fmt rgba -s WxH
//!    -fps_mode cfr -r FPS -an -sn pipe:1`, read W*H*4 bytes per frame (pts = T + n/fps); restart on
//!    non-sequential seeks or size change. Images (1 frame) are decoded once per size and cached.
//!  * `open_audio(path, stream)` -> AudioSource: `ffmpeg -ss T -i path -map 0:a:N -f f32le -ac 2 -ar 48000 pipe:1`.
//!  Kill children on Drop.

use super::{is_image_path, AudioSource, Frame, VideoSource, SAMPLE_RATE};
use crate::model::{Asset, AudioStreamInfo, ClipKind};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::Mutex;

static DIR: Mutex<String> = Mutex::new(String::new());

/// Set the user-configured ffmpeg directory ("" = auto).
pub fn set_dir(dir: &str) {
    *DIR.lock().unwrap() = dir.to_string();
}

pub fn ffmpeg_exe() -> Option<PathBuf> {
    find("ffmpeg.exe")
}
pub fn ffprobe_exe() -> Option<PathBuf> {
    find("ffprobe.exe")
}

fn find(name: &str) -> Option<PathBuf> {
    let dir = DIR.lock().unwrap().clone();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if !dir.is_empty() {
        candidates.push(PathBuf::from(&dir).join(name));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            candidates.push(d.join(name));
            candidates.push(d.join("ffmpeg").join(name));
        }
    }
    for c in candidates {
        if c.is_file() {
            return Some(c);
        }
    }
    // PATH
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|p| p.join(name)).find(|p| p.is_file())
}

/// A Command for `exe` that never opens a console window.
pub fn command(exe: &std::path::Path) -> Command {
    use std::os::windows::process::CommandExt;
    let mut c = Command::new(exe);
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    c
}

// ---------------------------------------------------------------- probe

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
fn f64_of(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0.0)
}
/// "30000/1001" -> 29.97; "0/0" / garbage -> 0.
fn parse_rate(s: &str) -> f64 {
    let (n, d) = s.split_once('/').unwrap_or((s, "1"));
    match (n.trim().parse::<f64>(), d.trim().parse::<f64>()) {
        (Ok(n), Ok(d)) if d > 0.0 && n.is_finite() => n / d,
        _ => 0.0,
    }
}

pub fn probe(path: &str) -> Result<Asset, String> {
    let exe = ffprobe_exe().ok_or("ffprobe.exe not found")?;
    let out = command(&exe)
        .args(["-v", "error", "-print_format", "json", "-show_streams", "-show_format", path])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ffprobe: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { format!("ffprobe failed ({})", out.status) } else { err });
    }
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("ffprobe json: {e}"))?;
    let format = v.get("format").cloned().unwrap_or(Value::Null);
    let empty = Vec::new();
    let streams = v.get("streams").and_then(Value::as_array).unwrap_or(&empty);

    let mut a = Asset {
        id: 0,
        path: path.to_string(),
        kind: ClipKind::Audio,
        duration: f64_of(&format, "duration"),
        width: 0,
        height: 0,
        fps: 0.0,
        audio_streams: Vec::new(),
        codec: String::new(),
    };
    let mut video: Option<&Value> = None;
    let mut stream_dur: f64 = 0.0;
    for s in streams {
        stream_dur = stream_dur.max(f64_of(s, "duration"));
        match str_of(s, "codec_type") {
            "video" => {
                let attached = s.pointer("/disposition/attached_pic").and_then(Value::as_i64).unwrap_or(0) != 0;
                if video.is_none() && !attached {
                    video = Some(s);
                }
            }
            "audio" => {
                let tags = s.get("tags").cloned().unwrap_or(Value::Null);
                a.audio_streams.push(AudioStreamInfo {
                    index: a.audio_streams.len(),
                    channels: f64_of(s, "channels") as u32,
                    sample_rate: f64_of(s, "sample_rate") as u32,
                    language: str_of(&tags, "language").to_string(),
                    title: str_of(&tags, "title").to_string(),
                    codec: str_of(s, "codec_name").to_string(),
                });
            }
            _ => {}
        }
    }
    if a.duration <= 0.0 {
        a.duration = stream_dur;
    }
    if let Some(s) = video {
        a.width = f64_of(s, "width") as u32;
        a.height = f64_of(s, "height") as u32;
        a.codec = str_of(s, "codec_name").to_string();
        a.fps = parse_rate(str_of(s, "avg_frame_rate"));
        if a.fps <= 0.0 {
            a.fps = parse_rate(str_of(s, "r_frame_rate"));
        }
        let single = matches!(a.codec.as_str(), "png" | "mjpeg" | "bmp" | "webp" | "tiff" | "gif")
            && f64_of(s, "nb_frames") <= 1.0
            && f64_of(&format, "duration") <= 0.0;
        let is_image = is_image_path(path) || str_of(&format, "format_name").ends_with("_pipe") || single;
        a.kind = if is_image { ClipKind::Image } else { ClipKind::Video };
        if is_image {
            a.duration = 0.0;
        }
    } else if is_image_path(path) {
        a.kind = ClipKind::Image;
        a.duration = 0.0;
    }
    Ok(a)
}

// ---------------------------------------------------------------- helpers

fn spawn(args: &[String]) -> Result<(Child, ChildStdout), String> {
    let exe = ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let mut child = command(&exe)
        .args(["-nostdin", "-loglevel", "error"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null()) // ponytail: errors dropped — read stderr on a thread if diagnostics matter
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let out = child.stdout.take().ok_or("ffmpeg: no stdout")?;
    Ok((child, out))
}

fn kill(child: &mut Option<(Child, ChildStdout)>) {
    if let Some((mut c, out)) = child.take() {
        let _ = c.kill();
        drop(out);
        let _ = c.wait();
    }
}

// ---------------------------------------------------------------- video

struct VideoPipe {
    path: String,
    size: (u32, u32),
    fps: f64,
    duration: f64,
    child: Option<(Child, ChildStdout)>,
    /// Output size of the running process.
    out_size: (u32, u32),
    /// Absolute frame index of the next frame the pipe will deliver.
    next: i64,
    /// Last delivered frame (index `next-1`), valid when `have` is true.
    cur: Frame,
    have: bool,
    /// Earliest known end (set when the pipe hits EOF) — avoids respawn storms past the end.
    end: f64,
}

impl VideoPipe {
    fn respawn(&mut self, idx: i64, w: u32, h: u32) -> bool {
        kill(&mut self.child);
        self.have = false;
        // Half a millisecond early so rounding never lands just past frame `idx` (ffmpeg drops frames < T).
        let t = (idx as f64 / self.fps - 0.0005).max(0.0);
        let args = [
            "-ss".into(),
            format!("{t:.6}"),
            "-i".into(),
            self.path.clone(),
            "-map".into(),
            "0:v:0".into(),
            "-f".into(),
            "rawvideo".into(),
            "-pix_fmt".into(),
            "rgba".into(),
            "-s".into(),
            format!("{w}x{h}"),
            "-fps_mode".into(),
            "cfr".into(),
            "-r".into(),
            format!("{}", self.fps),
            "-an".into(),
            "-sn".into(),
            "pipe:1".into(),
        ];
        match spawn(&args) {
            Ok(c) => {
                self.child = Some(c);
                self.out_size = (w, h);
                self.next = idx;
                self.cur.resize(w, h);
                true
            }
            Err(_) => false,
        }
    }
    /// Read the next frame into `cur`. False at EOF (child is reaped, `end` updated).
    fn read_next(&mut self) -> bool {
        let Some((_, out)) = self.child.as_mut() else {
            return false;
        };
        if out.read_exact(&mut self.cur.rgba).is_ok() {
            self.cur.pts = self.next as f64 / self.fps;
            self.next += 1;
            self.have = true;
            true
        } else {
            self.end = self.end.min(self.next as f64 / self.fps);
            kill(&mut self.child);
            false
        }
    }
}

impl VideoSource for VideoPipe {
    fn size(&self) -> (u32, u32) {
        self.size
    }
    fn duration(&self) -> f64 {
        self.duration
    }
    fn fps(&self) -> f64 {
        self.fps
    }
    fn frame_at(&mut self, t: f64, w: u32, h: u32, out: &mut Frame) -> bool {
        if w == 0 || h == 0 || t >= self.end || (self.duration > 0.0 && t >= self.duration + 0.5 / self.fps) {
            return false;
        }
        let t = t.max(0.0);
        let idx = (t * self.fps + 1e-6).floor() as i64;
        let cur_idx = self.next - 1;
        let same_size = self.out_size == (w, h);
        if same_size && self.have && idx == cur_idx {
            // cached
        } else {
            let sequential = self.child.is_some() && same_size && idx >= self.next - 1 && t <= self.cur.pts + 1.5;
            if !sequential && !self.respawn(idx, w, h) {
                return false;
            }
            while self.next <= idx {
                if !self.read_next() {
                    return false;
                }
            }
            if !self.have {
                return false;
            }
        }
        out.resize(w, h);
        out.rgba.copy_from_slice(&self.cur.rgba);
        out.pts = self.cur.pts;
        true
    }
}

impl Drop for VideoPipe {
    fn drop(&mut self) {
        kill(&mut self.child);
    }
}

/// Still image: decoded once per requested size (`-frames:v 1`), `t` ignored.
struct ImageSource {
    path: String,
    size: (u32, u32),
    cache: HashMap<(u32, u32), Vec<u8>>,
}

impl VideoSource for ImageSource {
    fn size(&self) -> (u32, u32) {
        self.size
    }
    fn duration(&self) -> f64 {
        0.0
    }
    fn fps(&self) -> f64 {
        0.0
    }
    fn frame_at(&mut self, _t: f64, w: u32, h: u32, out: &mut Frame) -> bool {
        if w == 0 || h == 0 {
            return false;
        }
        if !self.cache.contains_key(&(w, h)) {
            if self.cache.len() >= 8 {
                self.cache.clear(); // ponytail: bounded cache for animated scale — resize in Rust if it thrashes
            }
            let args: Vec<String> = [
                "-i",
                &self.path,
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "-s",
                &format!("{w}x{h}"),
                "-an",
                "-sn",
                "pipe:1",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
            let Ok((mut child, mut pipe)) = spawn(&args) else {
                return false;
            };
            let mut buf = Vec::with_capacity((w * h * 4) as usize);
            let ok = pipe.read_to_end(&mut buf).is_ok();
            let _ = child.wait();
            if !ok || buf.len() != (w * h * 4) as usize {
                return false;
            }
            self.cache.insert((w, h), buf);
        }
        let Some(rgba) = self.cache.get(&(w, h)) else {
            return false;
        };
        out.resize(w, h);
        out.rgba.copy_from_slice(rgba);
        out.pts = 0.0;
        true
    }
}

pub fn open_video(path: &str) -> Result<Box<dyn VideoSource>, String> {
    let a = probe(path)?;
    if a.kind == ClipKind::Image {
        return Ok(Box::new(ImageSource { path: path.to_string(), size: (a.width, a.height), cache: HashMap::new() }));
    }
    if a.kind != ClipKind::Video {
        return Err("no video stream".into());
    }
    let fps = if a.fps > 0.0 { a.fps } else { 30.0 };
    Ok(Box::new(VideoPipe {
        path: path.to_string(),
        size: (a.width, a.height),
        fps,
        duration: a.duration,
        child: None,
        out_size: (0, 0),
        next: 0,
        cur: Frame::default(),
        have: false,
        end: f64::INFINITY,
    }))
}

// ---------------------------------------------------------------- audio

const AUDIO_CHUNK_FRAMES: usize = 4800;

struct AudioPipe {
    path: String,
    stream: usize,
    channels: u32,
    duration: f64,
    child: Option<(Child, ChildStdout)>,
    /// Decoded interleaved stereo; `buf[pos..]` is unread.
    buf: Vec<f32>,
    pos: usize,
    bytes: Vec<u8>,
    /// Absolute sample-frame index of `buf[pos]`.
    next: i64,
    /// Frame index at which the pipe hit EOF (i64::MAX = unknown).
    end: i64,
}

impl AudioPipe {
    fn respawn(&mut self, frame: i64) -> bool {
        kill(&mut self.child);
        self.buf.clear();
        self.pos = 0;
        self.next = frame;
        let t = frame as f64 / SAMPLE_RATE as f64;
        let mut args: Vec<String> =
            ["-ss", &format!("{t:.6}"), "-i", &self.path, "-map", &format!("0:a:{}", self.stream)]
                .iter()
                .map(|s| s.to_string())
                .collect();
        if self.channels == 1 {
            // Duplicate mono into both channels (swresample's default upmix attenuates by 3 dB).
            args.extend(["-af", "pan=stereo|c0=c0|c1=c0"].map(String::from));
        }
        args.extend(["-f", "f32le", "-ac", "2", "-ar", "48000", "-vn", "pipe:1"].map(String::from));
        match spawn(&args) {
            Ok(c) => {
                self.child = Some(c);
                true
            }
            Err(_) => false,
        }
    }
    /// Refill `buf` from the pipe. False at EOF.
    fn refill(&mut self) -> bool {
        let Some((_, out)) = self.child.as_mut() else {
            return false;
        };
        self.bytes.resize(AUDIO_CHUNK_FRAMES * 8, 0);
        let mut got = 0;
        while got < self.bytes.len() {
            match out.read(&mut self.bytes[got..]) {
                Ok(0) | Err(_) => break,
                Ok(n) => got += n,
            }
        }
        let got = got / 4 * 4;
        self.buf.clear();
        self.pos = 0;
        self.buf.extend(self.bytes[..got].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
        if self.buf.len() % 2 == 1 {
            self.buf.pop();
        }
        if self.buf.is_empty() {
            self.end = self.end.min(self.next);
            kill(&mut self.child);
            return false;
        }
        true
    }
}

impl AudioSource for AudioPipe {
    fn duration(&self) -> f64 {
        self.duration
    }
    fn read_at(&mut self, t: f64, out: &mut [f32]) {
        let want = (t.max(0.0) * SAMPLE_RATE as f64).round() as i64;
        let frames = out.len() / 2;
        if want >= self.end {
            out.fill(0.0);
            return;
        }
        let sequential = self.child.is_some() && want >= self.next && want <= self.next + SAMPLE_RATE as i64 / 2;
        if !sequential && !self.respawn(want) {
            out.fill(0.0);
            return;
        }
        // Skip forward (small gaps), then copy.
        let mut done = 0usize; // frames written
        let mut skip = (want - self.next) as usize;
        while done < frames {
            if self.pos >= self.buf.len() && !self.refill() {
                break;
            }
            let avail = (self.buf.len() - self.pos) / 2;
            if skip > 0 {
                let n = skip.min(avail);
                self.pos += n * 2;
                self.next += n as i64;
                skip -= n;
                continue;
            }
            let n = avail.min(frames - done);
            out[done * 2..(done + n) * 2].copy_from_slice(&self.buf[self.pos..self.pos + n * 2]);
            self.pos += n * 2;
            self.next += n as i64;
            done += n;
        }
        out[done * 2..].fill(0.0);
    }
}

impl Drop for AudioPipe {
    fn drop(&mut self) {
        kill(&mut self.child);
    }
}

pub fn open_audio(path: &str, stream: usize) -> Result<Box<dyn AudioSource>, String> {
    let a = probe(path)?;
    if stream >= a.audio_streams.len() {
        return Err(format!("no audio stream {stream}"));
    }
    Ok(Box::new(AudioPipe {
        path: path.to_string(),
        stream,
        channels: a.audio_streams[stream].channels,
        duration: a.duration,
        child: None,
        buf: Vec::new(),
        pos: 0,
        bytes: Vec::new(),
        next: 0,
        end: i64::MAX,
    }))
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::OnceLock;

    /// Generates the shared test clip once: red 0–2 s, green 2–4 s, 440 Hz (eng) + 880 Hz (Music).
    pub(crate) fn test_mp4() -> String {
        static P: OnceLock<String> = OnceLock::new();
        P.get_or_init(|| {
            let dir = std::env::temp_dir().join("simple-editor-ffpipe-tests");
            let _ = std::fs::create_dir_all(&dir);
            let p = dir.join("test.mp4");
            let st = Command::new("ffmpeg")
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "color=red:s=320x240:d=2", "-f", "lavfi", "-i"])
                .args(["color=lime:s=320x240:d=2", "-f", "lavfi", "-i", "sine=frequency=440:duration=4", "-f", "lavfi"])
                .args(["-i", "sine=frequency=880:duration=4", "-filter_complex", "[0:v][1:v]concat=n=2:v=1[v]"])
                .args([
                    "-map", "[v]", "-map", "2:a", "-map", "3:a", "-r", "30", "-pix_fmt", "yuv420p", "-c:v", "libx264",
                ])
                .args(["-c:a", "aac", "-metadata:s:a:0", "language=eng", "-metadata:s:a:1", "title=Music"])
                .arg(&p)
                .status()
                .expect("ffmpeg on PATH");
            assert!(st.success(), "ffmpeg failed to generate test media");
            p.to_string_lossy().into_owned()
        })
        .clone()
    }

    /// mkv keeps per-stream titles (mp4 drops them).
    fn test_mkv() -> String {
        let p = Path::new(&test_mp4()).with_file_name("title.mkv");
        let st = Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=1", "-c:a", "aac"])
            .args(["-metadata:s:a:0", "title=Music"])
            .arg(&p)
            .status()
            .expect("ffmpeg on PATH");
        assert!(st.success());
        p.to_string_lossy().into_owned()
    }

    fn test_png() -> String {
        let p = Path::new(&test_mp4()).with_file_name("x.png");
        let st = Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "color=blue:s=64x48", "-frames:v", "1"])
            .arg(&p)
            .status()
            .expect("ffmpeg on PATH");
        assert!(st.success());
        p.to_string_lossy().into_owned()
    }

    fn centre(f: &Frame) -> [u8; 4] {
        let i = ((f.height / 2 * f.width + f.width / 2) * 4) as usize;
        [f.rgba[i], f.rgba[i + 1], f.rgba[i + 2], f.rgba[i + 3]]
    }
    fn is_red(p: [u8; 4]) -> bool {
        p[0] > 200 && p[1] < 60 && p[2] < 60 && p[3] == 255
    }
    fn is_green(p: [u8; 4]) -> bool {
        p[0] < 60 && p[1] > 200 && p[2] < 60 && p[3] == 255
    }
    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
    }
    /// Zero crossings of the left channel.
    fn crossings(s: &[f32]) -> usize {
        s.chunks_exact(2).map(|f| f[0]).collect::<Vec<_>>().windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count()
    }

    #[test]
    fn parse_rate_forms() {
        assert!((parse_rate("30000/1001") - 29.97).abs() < 0.01);
        assert_eq!(parse_rate("30/1"), 30.0);
        assert_eq!(parse_rate("0/0"), 0.0);
        assert_eq!(parse_rate("25"), 25.0);
        assert_eq!(parse_rate("x"), 0.0);
    }

    #[test]
    fn probe_mp4() {
        let a = probe(&test_mp4()).unwrap();
        assert_eq!(a.kind, ClipKind::Video);
        assert!((a.duration - 4.0).abs() < 0.1, "{}", a.duration);
        assert_eq!((a.width, a.height), (320, 240));
        assert!((a.fps - 30.0).abs() < 0.01);
        assert_eq!(a.codec, "h264");
        assert_eq!(a.audio_streams.len(), 2);
        assert_eq!(a.audio_streams[0].index, 0);
        assert_eq!(a.audio_streams[0].language, "eng");
        assert_eq!(a.audio_streams[1].index, 1);
        assert_eq!(a.audio_streams[1].codec, "aac");
        assert_eq!(a.audio_streams[1].channels, 1);
        assert_eq!(a.audio_streams[1].sample_rate, 44100);
        assert!(probe("Z:/definitely/missing.mp4").is_err());
        let m = probe(&test_mkv()).unwrap();
        assert_eq!(m.kind, ClipKind::Audio);
        assert_eq!(m.audio_streams[0].title, "Music");
        assert!((m.duration - 1.0).abs() < 0.1, "{}", m.duration);
    }

    #[test]
    fn video_decode_seek_scale() {
        let mut v = open_video(&test_mp4()).unwrap();
        assert_eq!(v.size(), (320, 240));
        assert!((v.fps() - 30.0).abs() < 0.01);
        let mut f = Frame::default();
        assert!(v.frame_at(0.5, 320, 240, &mut f));
        assert!(is_red(centre(&f)), "{:?}", centre(&f));
        assert!((f.pts - 0.5).abs() < 0.02);
        // sequential
        assert!(v.frame_at(0.5 + 1.0 / 30.0, 320, 240, &mut f));
        assert!(is_red(centre(&f)));
        // forward jump (respawn)
        assert!(v.frame_at(2.5, 320, 240, &mut f));
        assert!(is_green(centre(&f)), "{:?}", centre(&f));
        assert!((f.pts - 2.5).abs() < 0.02);
        // repeated call is cached
        assert!(v.frame_at(2.5, 320, 240, &mut f));
        assert!(is_green(centre(&f)));
        // seek back
        assert!(v.frame_at(0.5, 320, 240, &mut f));
        assert!(is_red(centre(&f)));
        // scaled
        assert!(v.frame_at(1.0, 160, 120, &mut f));
        assert_eq!((f.width, f.height), (160, 120));
        assert!(is_red(centre(&f)));
        // boundary: last frame exists, past end is false
        assert!(v.frame_at(3.97, 160, 120, &mut f));
        assert!(is_green(centre(&f)));
        assert!(!v.frame_at(10.0, 160, 120, &mut f));
        assert!(!v.frame_at(4.5, 160, 120, &mut f));
        // still usable after EOF
        assert!(v.frame_at(2.5, 160, 120, &mut f));
        assert!(is_green(centre(&f)));
    }

    #[test]
    fn audio_decode() {
        let mut a = open_audio(&test_mp4(), 0).unwrap();
        assert!((a.duration() - 4.0).abs() < 0.1);
        let mut buf = vec![0f32; 4800 * 2];
        a.read_at(0.5, &mut buf);
        // ffmpeg's sine source is ~-18 dBFS (peak 0.125, rms 0.088); mono is duplicated, not attenuated.
        let r = rms(&buf);
        assert!(r > 0.07 && r < 0.11, "rms {r}");
        assert!(buf.chunks_exact(2).all(|f| f[0] == f[1]));
        let c = crossings(&buf);
        assert!((80..=96).contains(&c), "440 Hz crossings {c}");
        // sequential continues seamlessly (no discontinuity spike)
        a.read_at(0.6, &mut buf);
        assert!(rms(&buf) > 0.07);
        // seek back
        a.read_at(0.1, &mut buf);
        assert!(rms(&buf) > 0.07);
        // past end is silent, then works again
        a.read_at(5.0, &mut buf);
        assert!(buf.iter().all(|&x| x == 0.0));
        a.read_at(1.0, &mut buf);
        assert!(rms(&buf) > 0.07);
        // second stream is 880 Hz
        let mut b = open_audio(&test_mp4(), 1).unwrap();
        b.read_at(0.5, &mut buf);
        let c = crossings(&buf);
        assert!((168..=184).contains(&c), "880 Hz crossings {c}");
        assert!(open_audio(&test_mp4(), 2).is_err());
    }

    #[test]
    fn image_decode() {
        let p = test_png();
        let a = probe(&p).unwrap();
        assert_eq!(a.kind, ClipKind::Image);
        assert_eq!((a.width, a.height), (64, 48));
        assert_eq!(a.duration, 0.0);
        let mut v = open_video(&p).unwrap();
        let mut f = Frame::default();
        assert!(v.frame_at(0.0, 32, 24, &mut f));
        assert_eq!((f.width, f.height), (32, 24));
        let px = centre(&f);
        assert!(px[0] < 10 && px[1] < 10 && px[2] > 240 && px[3] == 255, "{px:?}");
        assert!(v.frame_at(5.0, 32, 24, &mut f));
        assert!(v.frame_at(5.0, 64, 48, &mut f));
        assert_eq!((f.width, f.height), (64, 48));
    }
}
