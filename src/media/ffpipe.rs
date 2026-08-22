//! ffmpeg.exe / ffprobe.exe child-process backend. Universal fallback decoder, image loader, prober,
//! and the process launcher used by export.
//!
//! Contract (see media/mod.rs):
//!  * `ffmpeg_exe()` / `ffprobe_exe()` locate the binaries: Settings.ffmpeg_dir (set via `set_dir`),
//!    then next to the app exe, then `ffmpeg/` beside the exe, then PATH (a few stats per call, not cached).
//!  * `command(exe)` returns a std Command with CREATE_NO_WINDOW (0x08000000) so no console flashes.
//!  * `probe(path)` -> Asset via `ffprobe -v error -print_format json -show_streams -show_format`.
//!  * `open_video(path)` -> VideoSource: `ffmpeg -ss T -i path -map 0:v:0 -f rawvideo -pix_fmt rgba -s WxH
//!    -fps_mode cfr -r FPS -an -sn pipe:1`, read W*H*4 bytes per frame (pts = T + n/fps); restart on
//!    non-sequential seeks (at the requested size) or when a request outgrows the pipe (then at native size,
//!    so later sizes never restart); smaller requests are shrunk in Rust. Images (1 frame) are decoded once
//!    per size and cached.
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
    find_exe(name, &dir)
}

/// Locate a helper executable: `dir` (when set), then next to our exe (and an `ffmpeg` subfolder),
/// then PATH. Shared with `media::ytdlp` — not cached, so callers must not run it every frame.
pub(crate) fn find_exe(name: &str, dir: &str) -> Option<PathBuf> {
    find_all_exe(name, dir).into_iter().next()
}

/// Every place `name` exists, in lookup order. `ytdlp` needs this because a PATH entry can hold a
/// broken shim (a pip launcher for an uninstalled Python) that must be skipped for the next one.
pub(crate) fn find_all_exe(name: &str, dir: &str) -> Vec<PathBuf> {
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
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|p| p.join(name)));
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for c in candidates {
        if c.is_file() && !out.contains(&c) {
            out.push(c);
        }
    }
    out
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
        folder: String::new(),
        tags: Vec::new(),
        label: 0,
        description: String::new(),
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
        // Display Matrix rotation (phone portrait): ffmpeg autorotates frames, so report display dims.
        let rot = s.get("side_data_list").and_then(Value::as_array).into_iter().flatten();
        let rot = rot.map(|d| f64_of(d, "rotation")).find(|r| *r != 0.0).unwrap_or(0.0);
        if (rot.abs().round() as i64) % 180 == 90 {
            std::mem::swap(&mut a.width, &mut a.height);
        }
        a.codec = str_of(s, "codec_name").to_string();
        a.fps = parse_rate(str_of(s, "avg_frame_rate"));
        if a.fps <= 0.0 {
            a.fps = parse_rate(str_of(s, "r_frame_rate"));
        }
        let single = matches!(a.codec.as_str(), "png" | "mjpeg" | "bmp" | "webp" | "tiff" | "gif")
            && f64_of(s, "nb_frames") <= 1.0
            && f64_of(&format, "duration") <= 0.0;
        // Animated GIFs are video (is_image_path still routes every GIF to ffmpeg, which is fine).
        let animated = a.codec == "gif" && f64_of(s, "nb_frames") > 1.0;
        let is_image =
            !animated && (is_image_path(path) || str_of(&format, "format_name").ends_with("_pipe") || single);
        a.kind = if is_image { ClipKind::Image } else { ClipKind::Video };
        if is_image {
            a.duration = 0.0;
        }
    } else if is_image_path(path) {
        a.kind = ClipKind::Image;
        a.duration = 0.0;
    }
    // Duration-less containers (stream-written WebM, raw .h264/.aac, some TS): measure the packets.
    if a.duration <= 0.0 && a.kind != ClipKind::Image && (video.is_some() || !a.audio_streams.is_empty()) {
        a.duration = packet_duration(&exe, path, if video.is_some() { "v:0" } else { "a:0" });
    }
    Ok(a)
}

/// Duration of stream `sel` from its packets: max(pts_time + duration_time) - min(pts_time); when no packet
/// carries a pts (raw .h264) the durations are summed instead. Full demux, so only for files whose container
/// has no duration. 0.0 on failure.
fn packet_duration(exe: &std::path::Path, path: &str, sel: &str) -> f64 {
    let entries = "packet=pts_time,duration_time";
    let Ok(out) = command(exe)
        .args(["-v", "error", "-select_streams", sel, "-show_entries", entries, "-of", "csv=p=0", path])
        .stdin(Stdio::null())
        .output()
    else {
        return 0.0;
    };
    let (mut start, mut end, mut sum) = (f64::INFINITY, 0.0f64, 0.0f64);
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut f = line.split(',').map(|x| x.trim().parse::<f64>().ok());
        let (pts, dur) = (f.next().flatten(), f.next().flatten().unwrap_or(0.0));
        sum += dur;
        if let Some(pts) = pts {
            start = start.min(pts);
            end = end.max(pts + dur);
        }
    }
    if start.is_finite() {
        (end - start).max(0.0)
    } else {
        sum
    }
}

// ---------------------------------------------------------------- helpers

#[cfg(test)]
thread_local!(static SPAWNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) });

fn spawn(args: &[String]) -> Result<(Child, ChildStdout), String> {
    #[cfg(test)]
    SPAWNS.with(|c| c.set(c.get() + 1));
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
    /// `cur` shrunk to `skey` = (frame index, w, h): repeated calls at the same t and size are free.
    scaled: Vec<u8>,
    skey: (i64, u32, u32),
}

impl VideoPipe {
    fn respawn(&mut self, idx: i64, w: u32, h: u32) -> bool {
        kill(&mut self.child);
        self.have = false;
        self.skey = (-1, 0, 0);
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
    fn frame_at(&mut self, t: f64, w: u32, h: u32, out: &mut Frame) -> bool {
        if w == 0 || h == 0 || t >= self.end || (self.duration > 0.0 && t >= self.duration + 0.5 / self.fps) {
            return false;
        }
        let t = t.max(0.0);
        let idx = (t * self.fps + 1e-6).floor() as i64;
        // The pipe's output is only ever shrunk in Rust, so any request that fits keeps it running.
        let fits = w <= self.out_size.0 && h <= self.out_size.1;
        if !(fits && self.have && idx == self.next - 1) {
            let sequential = self.child.is_some() && idx >= self.next - 1 && t <= self.cur.pts + 1.5;
            if !sequential || !fits {
                // Outgrown on the same t progression (keyframed scale): respawn once at native so later sizes
                // never respawn. Real seek: the requested size.
                let (sw, sh) = if sequential { (w.max(self.size.0), h.max(self.size.1)) } else { (w, h) };
                if !self.respawn(idx, sw, sh) {
                    return false;
                }
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
        out.pts = self.cur.pts;
        if (w, h) == self.out_size {
            out.rgba.copy_from_slice(&self.cur.rgba);
        } else {
            let key = (self.next - 1, w, h);
            if self.skey != key {
                self.scaled.resize((w * h * 4) as usize, 0);
                box_down(&self.cur.rgba, self.out_size.0, self.out_size.1, w, h, &mut self.scaled);
                self.skey = key;
            }
            out.rgba.copy_from_slice(&self.scaled);
        }
        true
    }
}

/// Area-average downscale of top-down RGBA (`w <= sw`, `h <= sh`).
fn box_down(src: &[u8], sw: u32, sh: u32, w: u32, h: u32, dst: &mut [u8]) {
    debug_assert!(w <= sw && h <= sh);
    let (sw, sh, w, h) = (sw as usize, sh as usize, w as usize, h as usize);
    for (j, drow) in dst.chunks_exact_mut(w * 4).enumerate() {
        let (y0, y1) = (j * sh / h, (j + 1) * sh / h);
        for (i, d) in drow.chunks_exact_mut(4).enumerate() {
            let (x0, x1) = (i * sw / w, (i + 1) * sw / w);
            let mut acc = [0u32; 4];
            for y in y0..y1 {
                for px in src[(y * sw + x0) * 4..(y * sw + x1) * 4].chunks_exact(4) {
                    for k in 0..4 {
                        acc[k] += px[k] as u32;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)).max(1) as u32;
            for k in 0..4 {
                d[k] = (acc[k] / n) as u8;
            }
        }
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
        scaled: Vec::new(),
        skey: (-1, 0, 0),
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
        // A small step back (the mixer reads one guard frame beyond what it consumes, and resampling
        // re-reads the block edge) is served from the frames still sitting behind `pos` — respawning
        // ffmpeg for one frame would cost ~50-100 ms per block.
        if self.child.is_some() && want < self.next {
            let back = (self.next - want) as usize;
            if back * 2 <= self.pos {
                self.pos -= back * 2;
                self.next = want;
            }
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
    fn audio_reads_with_guard_frame_do_not_respawn() {
        let path = test_mp4();
        let Ok(mut a) = open_audio(&path, 0) else { return };
        let mut buf = vec![0f32; 4800 * 2];
        let before = SPAWNS.with(|s| s.get());
        // the mixer's pattern: read [t, t+0.1) but only consume 0.1 s minus one frame each time
        let mut t = 0.5;
        for _ in 0..20 {
            a.read_at(t, &mut buf);
            t += (4800 - 1) as f64 / SAMPLE_RATE as f64;
        }
        let spawns = SPAWNS.with(|s| s.get()) - before;
        assert!(spawns <= 1, "{spawns} ffmpeg respawns for 20 near-sequential audio blocks");
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

    /// Animated GIFs are video (every frame reachable); single-frame GIFs stay images.
    #[test]
    fn animated_gif_is_video() {
        let p = Path::new(&test_mp4()).with_file_name(format!("{}-anim.gif", std::process::id()));
        let st = Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "color=red:s=64x48:d=1,format=rgb8"])
            .args(["-f", "lavfi", "-i", "color=blue:s=64x48:d=1,format=rgb8"])
            .args(["-filter_complex", "[0:v][1:v]concat=n=2:v=1[v]", "-map", "[v]", "-r", "10"])
            .arg(&p)
            .status()
            .expect("ffmpeg on PATH");
        assert!(st.success());
        let p = p.to_string_lossy().into_owned();
        let a = probe(&p).unwrap();
        assert_eq!(a.kind, ClipKind::Video);
        assert!((a.duration - 2.0).abs() < 0.5, "{}", a.duration);
        let mut v = crate::media::open_video(&p, crate::media::Backend::Auto).unwrap();
        let mut f = Frame::default();
        assert!(v.frame_at(0.5, 64, 48, &mut f));
        assert!(is_red(centre(&f)), "{:?}", centre(&f));
        assert!(v.frame_at(1.5, 64, 48, &mut f));
        let c = centre(&f);
        assert!(c[2] > 150 && c[0] < 60, "{c:?}"); // rgb8 palette: blue = 170
        let still = Path::new(&test_mp4()).with_file_name(format!("{}-still.gif", std::process::id()));
        let st = Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "color=blue:s=64x48", "-frames:v", "1"])
            .arg(&still)
            .status()
            .expect("ffmpeg on PATH");
        assert!(st.success());
        assert_eq!(probe(&still.to_string_lossy()).unwrap().kind, ClipKind::Image);
    }

    /// A WebM streamed to stdout carries no duration anywhere; probe must measure the packets.
    #[test]
    fn probe_duration_from_packets() {
        let p = Path::new(&test_mp4()).with_file_name(format!("{}-stdout.webm", std::process::id()));
        let out = std::fs::File::create(&p).unwrap();
        let st = Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "color=red:s=160x120:d=3", "-f", "lavfi", "-i"])
            .args(["sine=duration=3", "-c:v", "libvpx", "-c:a", "libopus", "-f", "webm", "-"])
            .stdout(out)
            .status()
            .expect("ffmpeg on PATH");
        assert!(st.success());
        let a = probe(&p.to_string_lossy()).unwrap();
        assert_eq!(a.kind, ClipKind::Video);
        assert!((a.duration - 3.0).abs() < 0.2, "{}", a.duration);
        assert_eq!((a.width, a.height), (160, 120));
        assert_eq!(a.audio_streams.len(), 1);
    }

    /// Size changes on a sequential t progression (keyframed scale) must not respawn ffmpeg: the pipe is
    /// shrunk in Rust, and one growth respawns at native size after which every size fits.
    #[test]
    fn scale_keeps_pipe() {
        let mut v = open_video(&test_mp4()).unwrap();
        let mut f = Frame::default();
        let sizes = [(80, 60), (120, 90), (160, 120)];
        let before = SPAWNS.with(|c| c.get());
        let t0 = std::time::Instant::now();
        for n in 0..=60 {
            let t = 0.5 + n as f64 / 30.0;
            let (w, h) = sizes[n % 3];
            assert!(v.frame_at(t, w, h, &mut f), "n={n}");
            assert_eq!((f.width, f.height), (w, h));
            let c = centre(&f);
            assert!(if t < 2.0 { is_red(c) } else { is_green(c) }, "t={t} {c:?}");
        }
        let spawns = SPAWNS.with(|c| c.get()) - before;
        eprintln!("scale_keeps_pipe: 61 frames, {spawns} spawns, {:?}", t0.elapsed());
        // first (80x60) + one growth (120x90 -> native 320x240); 160x120 then fits
        assert_eq!(spawns, 2);
        // repeated call at the same t/size is free
        assert!(v.frame_at(2.5, 80, 60, &mut f));
        assert_eq!(SPAWNS.with(|c| c.get()) - before, 2);
        // a real seek still respawns (at the requested size)
        assert!(v.frame_at(0.5, 80, 60, &mut f));
        assert!(is_red(centre(&f)));
        assert_eq!(SPAWNS.with(|c| c.get()) - before, 3);
        assert!(v.frame_at(0.5 + 1.0 / 30.0, 40, 30, &mut f));
        assert_eq!((f.width, f.height), (40, 30));
        assert!(is_red(centre(&f)));
        assert_eq!(SPAWNS.with(|c| c.get()) - before, 3);
    }

    #[test]
    fn box_down_averages() {
        // 4x2 -> 2x1: each output pixel averages a 2x2 box.
        #[rustfmt::skip]
        let src = [
            0, 0, 0, 255,   100, 100, 100, 255,   200, 0, 0, 255,   200, 0, 0, 255,
            0, 0, 0, 255,   100, 100, 100, 255,   0, 0, 0, 255,     0, 0, 0, 255,
        ];
        let mut dst = [0u8; 8];
        box_down(&src, 4, 2, 2, 1, &mut dst);
        assert_eq!(dst, [50, 50, 50, 255, 100, 0, 0, 255]);
    }
}
