//! "Convert To…": transcode a media file to another container/format with ffmpeg (no compositor):
//! video ↔ gif, mp4 ↔ mov/mkv/webm, audio extraction (mp3/wav/m4a/flac), optional rescale with a chosen
//! scaler. Runs on a background thread, reports through engine::export::Progress, never writes the
//! destination until done (temp + rename, same as exports), never touches the source.

use crate::engine::export::Progress;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct ConvertOptions {
    pub src: PathBuf,
    pub out: PathBuf,
    /// "auto" or an ffmpeg encoder name (see Settings.encoder / export::codec_args).
    pub encoder: String,
    pub crf: u32,
    pub preset: String,
    /// Rescale to this size (None = keep), with ffmpeg flags `scaler` ("neighbor" | "bilinear" | "bicubic"
    /// | "lanczos" | "area" | "spline").
    pub out_size: Option<(u32, u32)>,
    pub scaler: String,
    /// GIF outputs: frame rate (default 15) and palette generation for quality.
    pub gif_fps: u32,
}

/// Start a conversion; progress 0..1 by parsing ffmpeg `-progress pipe:2` (out_time_ms / duration) or
/// by the input duration from ffprobe. `codec_args(ext, ...)` from export.rs picks the codecs; gif gets
/// `fps=<gif_fps>,scale=...:flags=lanczos,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse`.
pub fn start_convert(_opts: ConvertOptions) -> Arc<Progress> {
    todo!("engine::convert::start_convert")
}

/// Output extensions offered by the converter UI.
pub const TARGETS: &[&str] = &["mp4", "mov", "mkv", "webm", "gif", "avi", "mp3", "wav", "m4a", "flac", "png", "jpg"];
