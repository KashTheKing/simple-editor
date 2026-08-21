//! ffmpeg.exe / ffprobe.exe child-process backend. Universal fallback decoder, image loader, prober,
//! and the process launcher used by export.
//!
//! Contract (see media/mod.rs):
//!  * `ffmpeg_exe()` / `ffprobe_exe()` locate the binaries: Settings.ffmpeg_dir (set via `set_dir`),
//!    then next to the app exe, then `ffmpeg/` beside the exe, then PATH. Cached.
//!  * `command(exe)` returns a std Command with CREATE_NO_WINDOW (0x08000000) so no console flashes.
//!  * `probe(path)` -> Asset via `ffprobe -v quiet -print_format json -show_streams -show_format`.
//!  * `open_video(path)` -> VideoSource: `ffmpeg -ss T -i path -map 0:v:0 -f rawvideo -pix_fmt rgba -s WxH
//!    -fps_mode cfr -r FPS -an -sn pipe:1`, read W*H*4 bytes per frame (pts = T + n/fps); restart on
//!    non-sequential seeks or size change. Images (1 frame) are decoded once per size and cached.
//!  * `open_audio(path, stream)` -> AudioSource: `ffmpeg -ss T -i path -map 0:a:N -f f32le -ac 2 -ar 48000 pipe:1`.
//!  Kill children on Drop.

use super::{AudioSource, Frame, VideoSource};
use crate::model::Asset;
use std::path::PathBuf;
use std::process::Command;
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

pub fn probe(_path: &str) -> Result<Asset, String> {
    Err("ffpipe::probe not implemented".into())
}

pub fn open_video(_path: &str) -> Result<Box<dyn VideoSource>, String> {
    Err("ffpipe::open_video not implemented".into())
}

pub fn open_audio(_path: &str, _stream: usize) -> Result<Box<dyn AudioSource>, String> {
    Err("ffpipe::open_audio not implemented".into())
}

#[allow(dead_code)]
fn _unused(_: &mut Frame) {}
