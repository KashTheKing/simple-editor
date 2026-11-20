//! The player's file tools: rename the file by clicking its name, and Convert… (⋯ menu) through the
//! editor's converter (`engine::convert`, ffmpeg), next to the source under a name that never clobbers.

use crate::engine::convert::{self, ConvertOptions};
use crate::engine::export::Progress;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Why `name` cannot be a Windows file name, or None when it can.
pub fn bad_name(name: &str) -> Option<&'static str> {
    let n = name.trim();
    if n.is_empty() {
        return Some("The name is empty");
    }
    if n.chars().any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control()) {
        return Some("A name can't contain < > : \" / \\ | ? *");
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Some("A name can't end with a dot or a space");
    }
    let base = n.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&base.as_str())
        || (base.len() == 4 && (base.starts_with("COM") || base.starts_with("LPT")) && base.as_bytes()[3].is_ascii_digit());
    reserved.then_some("That name is reserved by Windows")
}

/// The path `path` gets renamed to `stem` (the extension kept), or why not.
pub fn rename_target(path: &Path, stem: &str) -> Result<PathBuf, String> {
    if let Some(why) = bad_name(stem) {
        return Err(why.into());
    }
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let to = path.with_file_name(format!("{}{ext}", stem.trim()));
    let same = to.to_string_lossy().eq_ignore_ascii_case(&path.to_string_lossy());
    if to.exists() && !same {
        return Err(format!("{} already exists", to.file_name().unwrap_or_default().to_string_lossy()));
    }
    Ok(to)
}

/// Rename `from` to `to`, retrying for up to a second while a handle that was just closed lets go.
pub fn rename_retrying(from: &Path, to: &Path) -> Result<(), String> {
    let mut last = String::new();
    for _ in 0..20 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Err(last)
}

/// Convert… formats: (label, extension, encoder).
pub const FORMATS: &[(&str, &str, &str)] = &[
    ("MP4 (H.264)", "mp4", "libx264"),
    ("MP4 (H.265)", "mp4", "libx265"),
    ("WebM", "webm", "auto"),
    ("MOV", "mov", "libx264"),
    ("GIF", "gif", "auto"),
    ("MP3", "mp3", "auto"),
    ("WAV", "wav", "auto"),
    ("M4A", "m4a", "auto"),
    ("PNG frames", "png", "auto"),
];

/// The Convert panel's choices.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    /// Index into `FORMATS`.
    pub format: usize,
    /// Quality 0 (smallest) ..= 4 (best); ignored when `size_mb` is set.
    pub quality: u8,
    /// Aim at this size (MB) instead of a quality.
    pub size_mb: Option<f64>,
    /// Output height (width follows the picture's aspect); None = as it is.
    pub height: Option<u32>,
    pub fps: Option<f64>,
    /// Keep only the ticked audio tracks (else all).
    pub ticked_only: bool,
}

impl Default for Spec {
    fn default() -> Self {
        Spec { format: 0, quality: 2, size_mb: None, height: None, fps: None, ticked_only: false }
    }
}

/// CRF for quality 0..=4.
fn crf(q: u8) -> u32 {
    [32, 28, 23, 20, 17][q.min(4) as usize]
}

/// Where the output goes: beside the source, never over a file (a PNG sequence: a new folder).
pub fn out_path(src: &Path, spec: &Spec) -> PathBuf {
    let (_, ext, _) = FORMATS[spec.format.min(FORMATS.len() - 1)];
    if ext == "png" {
        let dir = crate::ui::app::converted_path(src, "frames");
        let dir = dir.with_extension(""); // "<name>_converted" (a folder)
        return dir.join("frame_%05d.png");
    }
    crate::ui::app::converted_path(src, ext)
}

/// The converter's options for `spec`. `size` = the source picture (w, h); `ticked` = the player's
/// ticked audio tracks (None = all).
pub fn options(src: &Path, spec: &Spec, size: (u32, u32), ticked: Option<Vec<usize>>) -> ConvertOptions {
    let (_, _, enc) = FORMATS[spec.format.min(FORMATS.len() - 1)];
    let out_size = spec.height.filter(|_| size.1 > 0).map(|h| {
        let w = ((size.0 as f64 * h as f64 / size.1 as f64 / 2.0).round() as u32 * 2).max(2);
        (w, h & !1)
    });
    let vf_extra = spec.fps.map(|f| format!("fps={f}"));
    ConvertOptions {
        src: src.to_path_buf(),
        out: out_path(src, spec),
        encoder: enc.into(),
        crf: crf(spec.quality),
        preset: "balanced".into(),
        out_size,
        scaler: "lanczos".into(),
        gif_fps: spec.fps.map_or(15, |f| f.round().max(1.0) as u32),
        target_bytes: spec.size_mb.map(|mb| (mb * 1e6) as u64),
        vf_extra: if FORMATS[spec.format].1 == "gif" { None } else { vf_extra },
        af_extra: None,
        range: None,
        audio_streams: if spec.ticked_only { ticked } else { None },
    }
}

/// A conversion running in the background.
pub struct Job {
    pub progress: Arc<Progress>,
    pub out: PathBuf,
}

pub fn start(opts: ConvertOptions) -> Job {
    if convert::is_sequence(&opts.out) {
        if let Some(d) = opts.out.parent() {
            let _ = std::fs::create_dir_all(d);
        }
    }
    let out = opts.out.clone();
    Job { progress: convert::start_convert(opts), out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_keeps_the_extension_and_refuses_clashes_and_bad_names() {
        let dir = std::env::temp_dir().join(format!("se-player-rename-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("clip.mp4");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(dir.join("taken.mp4"), b"y").unwrap();
        assert_eq!(rename_target(&a, "new name").unwrap(), dir.join("new name.mp4"));
        assert_eq!(rename_target(&a, "CLIP").unwrap(), dir.join("CLIP.mp4"), "a case change is fine");
        assert!(rename_target(&a, "taken").unwrap_err().contains("already exists"));
        for bad in ["", "  ", "a/b", "a:b", "what?", "dot.", "CON", "com1", "x*"] {
            assert!(rename_target(&a, bad).is_err(), "{bad:?} accepted");
        }
        let to = rename_target(&a, "renamed").unwrap();
        rename_retrying(&a, &to).unwrap();
        assert!(to.is_file() && !a.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn args_of(o: &ConvertOptions) -> String {
        format!("{:?}", o)
    }

    #[test]
    fn convert_options_per_format() {
        let src = Path::new("C:/v/clip.mp4");
        let spec = |f: &str| Spec { format: FORMATS.iter().position(|x| x.0 == f).unwrap(), ..Default::default() };
        let o = options(src, &spec("MP4 (H.265)"), (1920, 1080), None);
        assert_eq!((o.encoder.as_str(), o.out.extension().unwrap().to_str().unwrap()), ("libx265", "mp4"));
        assert!(o.out.to_string_lossy().contains("clip_converted"));
        let o = options(src, &spec("WebM"), (1920, 1080), None);
        assert_eq!(o.out.extension().unwrap(), "webm");
        let o = options(src, &spec("PNG frames"), (1920, 1080), None);
        assert!(convert::is_sequence(&o.out), "{}", args_of(&o));
        let mut s = spec("MP3");
        s.ticked_only = true;
        assert_eq!(options(src, &s, (0, 0), Some(vec![1, 2])).audio_streams, Some(vec![1, 2]));
        s.ticked_only = false;
        assert_eq!(options(src, &s, (0, 0), Some(vec![1])).audio_streams, None, "all tracks");
        let s = Spec { height: Some(720), fps: Some(30.0), size_mb: Some(8.0), ..spec("MP4 (H.264)") };
        let o = options(src, &s, (1920, 1080), None);
        assert_eq!(o.out_size, Some((1280, 720)));
        assert_eq!(o.vf_extra.as_deref(), Some("fps=30"));
        assert_eq!(o.target_bytes, Some(8_000_000));
        let g = options(src, &Spec { fps: Some(12.0), ..spec("GIF") }, (640, 360), None);
        assert_eq!((g.gif_fps, g.vf_extra.as_deref()), (12, None));
        // audio maps: video files keep tracks apart, audio files mix them, none = no audio
        assert_eq!(convert::audio_map_args(None, false), ["-map", "0:a?"]);
        assert_eq!(convert::audio_map_args(Some(&[1, 2]), false), ["-map", "0:a:1", "-map", "0:a:2"]);
        assert_eq!(convert::audio_map_args(Some(&[0, 1]), true)[1], "[0:a:0][0:a:1]amix=inputs=2:normalize=0[a]");
        assert_eq!(convert::audio_map_args(Some(&[]), false), ["-an"]);
    }

    /// Real ffmpeg runs: the outputs probe as the format asked for.
    #[test]
    fn converted_files_probe_as_the_chosen_format() {
        let dir = std::env::temp_dir().join(format!("se-player-conv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.mp4");
        let ok = std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc=s=320x240:r=30:d=2", "-f", "lavfi", "-i", "sine=f=440:d=2"])
            .args(["-f", "lavfi", "-i", "sine=f=880:d=2", "-map", "0", "-map", "1", "-map", "2", "-pix_fmt", "yuv420p", "-shortest"])
            .arg(&src)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let probe = |p: &Path, what: &str| -> String {
            let o = std::process::Command::new("ffprobe")
                .args(["-v", "error", "-show_entries", what, "-of", "csv=p=0"])
                .arg(p)
                .output()
                .unwrap();
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        };
        for (fmt, want) in [("MP4 (H.265)", "hevc"), ("WebM", "vp9"), ("GIF", "gif"), ("MP3", "mp3"), ("WAV", "pcm_s16le"), ("M4A", "aac")] {
            let i = FORMATS.iter().position(|x| x.0 == fmt).unwrap();
            let spec = Spec { format: i, quality: 0, ticked_only: true, ..Default::default() };
            let job = start(options(&src, &spec, (320, 240), Some(vec![1])));
            let t0 = std::time::Instant::now();
            while !job.progress.is_done() {
                assert!(t0.elapsed().as_secs() < 120, "{fmt} timed out");
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            assert!(job.progress.error().is_none(), "{fmt}: {:?}", job.progress.error());
            let codecs = probe(&job.out, "stream=codec_name");
            assert!(codecs.lines().next() == Some(want), "{fmt}: {codecs}");
            if fmt.starts_with("MP4") || fmt == "WebM" {
                assert_eq!(codecs.lines().count(), 2, "{fmt}: the one ticked audio track kept: {codecs}");
            }
        }
        let job = start(options(&src, &Spec { format: FORMATS.len() - 1, fps: Some(5.0), ..Default::default() }, (320, 240), None));
        while !job.progress.is_done() {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let n = std::fs::read_dir(job.out.parent().unwrap()).unwrap().count();
        assert!((9..=11).contains(&n), "2 s at 5 fps: {n} frames");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
