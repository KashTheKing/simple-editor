//! "Convert To…": transcode a media file to another container/format with ffmpeg (no compositor):
//! video ↔ gif, mp4 ↔ mov/mkv/webm, audio extraction (mp3/wav/m4a/flac), optional rescale with a chosen
//! scaler. Runs on a background thread, reports through engine::export::Progress, never writes the
//! destination until done (temp + rename, same as exports), never touches the source.

use crate::engine::export::{self, codec_args, detect_encoders, Progress, AUDIO_EXTS};
use crate::media::ffpipe;
use std::io::BufRead;
use std::path::PathBuf;
use std::process::Stdio;
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
    /// Compression target in bytes. Some = bitrate mode: the video bitrate is derived from the source
    /// duration (minus a fixed audio allowance) and CRF is ignored. None = quality (CRF) mode.
    pub target_bytes: Option<u64>,
}

/// Audio allowance subtracted from a size target, bits per second. Matches `codec_args`' AAC default.
const AUDIO_BPS: f64 = 128_000.0;

/// Video bitrate (bits/s) that lands `target` bytes over `dur` seconds, leaving room for the audio.
/// None when the numbers make no sense (no duration, or the target is smaller than the audio track).
pub fn target_bitrate(target: u64, dur: f64) -> Option<u32> {
    if dur <= 0.0 || target == 0 {
        return None;
    }
    // 2 % container overhead: muxing is never free, and overshooting the target is the failure people notice
    let total = target as f64 * 8.0 * 0.98 / dur;
    let v = total - AUDIO_BPS;
    (v >= 32_000.0).then(|| v as u32)
}

/// Start a conversion; progress 0..1 by parsing ffmpeg `-progress pipe:1` (out_time_us) against the
/// input duration from ffprobe. `codec_args(ext, ...)` from export.rs picks the codecs; gif gets
/// `fps=<gif_fps>[,scale=...:flags=<scaler>],split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse`.
pub fn start_convert(opts: ConvertOptions) -> Arc<Progress> {
    export::spawn_job("convert", move |prog| run_convert(&opts, prog))
}

fn run_convert(opts: &ConvertOptions, prog: &Progress) -> Result<(), String> {
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let ext = export::ext_of(&opts.out);
    let tmp = export::temp_output(&opts.out);
    let dur = probe_duration(&opts.src).unwrap_or(0.0);
    let scaler = if opts.scaler.is_empty() { "lanczos" } else { &opts.scaler };
    let scale = opts.out_size.map(|(w, h)| format!("scale={w}:{h}:flags={scaler}"));

    let mut cmd = ffpipe::command(&ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-progress", "pipe:1"]);
    cmd.arg("-i").arg(&opts.src);
    if ext == "gif" {
        let fps = opts.gif_fps.max(1);
        let mut chain = format!("fps={fps}");
        if let Some(s) = &scale {
            chain.push(',');
            chain.push_str(s);
        }
        chain.push_str(",split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse");
        cmd.args(["-filter_complex", &chain, "-an"]);
    } else if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "bmp" | "webp") {
        if let Some(s) = &scale {
            cmd.args(["-vf", s]);
        }
        cmd.args(["-frames:v", "1", "-update", "1"]);
    } else if AUDIO_EXTS.contains(&ext.as_str()) {
        cmd.arg("-vn");
        cmd.args(codec_args(&ext, &opts.encoder, opts.crf, &opts.preset, &detect_encoders()));
    } else {
        let mut args = codec_args(&ext, &opts.encoder, opts.crf, &opts.preset, &detect_encoders());
        if let Some(bps) = opts.target_bytes.and_then(|t| target_bitrate(t, dur)) {
            // bitrate mode: drop the quality knob the codec args set and cap the rate instead
            strip_quality(&mut args);
            let (b, buf) = (format!("{bps}"), format!("{}", bps * 2));
            args.extend(["-b:v".into(), b.clone(), "-maxrate".into(), b, "-bufsize".into(), buf]);
        }
        let mut vf = scale.unwrap_or_default();
        if args.iter().any(|a| a == "yuv420p" || a == "nv12") {
            // even-size guard (no-op on even input): the source size is not probed here
            if !vf.is_empty() {
                vf.push(',');
            }
            vf.push_str("pad=ceil(iw/2)*2:ceil(ih/2)*2");
        }
        if !vf.is_empty() {
            cmd.args(["-vf", &vf]);
        }
        // keep every audio stream — ffmpeg's default stream selection would keep only one
        cmd.args(["-map", "0:v:0?", "-map", "0:a?"]);
        cmd.args(args);
    }
    cmd.arg(&tmp.0);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
    let tail = export::stderr_tail(&mut child);

    // read -progress key=value lines from stdout until EOF (cancel kills ffmpeg → EOF)
    if let Some(out) = child.stdout.take() {
        prog.set(0.0, "Converting…");
        for line in std::io::BufReader::new(out).lines() {
            let Ok(line) = line else { break };
            if prog.is_cancelled() {
                let _ = child.kill();
                break;
            }
            if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                let t = us / 1e6;
                if dur > 0.0 {
                    prog.set((t / dur).clamp(0.0, 1.0).min(0.99) as f32, format!("Converting {t:.1} / {dur:.1} s"));
                } else {
                    prog.set(0.0, format!("Converting {t:.1} s"));
                }
            }
        }
    }
    export::wait_ffmpeg(&mut child, tail, prog)?; // returns Err(CANCELLED) when cancelled
    tmp.commit(&opts.out)
}

/// Remove `-crf N` / `-qp N` / `-cq N` pairs so an explicit bitrate is not fighting a quality target.
fn strip_quality(args: &mut Vec<String>) {
    let mut i = 0;
    while i < args.len() {
        if matches!(args[i].as_str(), "-crf" | "-qp" | "-cq" | "-global_quality") && i + 1 < args.len() {
            args.drain(i..i + 2);
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod target_tests {
    use super::*;

    /// A size target becomes a video bitrate that leaves room for the audio, and refuses impossible asks.
    #[test]
    fn bitrate_from_a_size_target() {
        // 10 MB over 60 s = 1.307 Mbit/s total, minus 128 kbit/s of audio
        let bps = target_bitrate(10_000_000, 60.0).unwrap();
        assert!((1_170_000..=1_190_000).contains(&bps), "{bps}");
        // round trip: the encoded video plus the audio allowance lands within 2 % of the target
        let bytes = (bps as f64 + 128_000.0) * 60.0 / 8.0;
        assert!((bytes - 10_000_000.0).abs() < 10_000_000.0 * 0.03, "{bytes}");
        assert_eq!(target_bitrate(10_000_000, 0.0), None, "no duration, no bitrate");
        assert_eq!(target_bitrate(1_000, 60.0), None, "smaller than the audio track");
        // the bitrate override wins over the codec's quality knob
        let mut args: Vec<String> =
            ["-c:v", "libx264", "-crf", "23", "-pix_fmt", "yuv420p"].iter().map(|s| s.to_string()).collect();
        strip_quality(&mut args);
        assert!(!args.iter().any(|a| a == "-crf" || a == "23"), "{args:?}");
        assert!(args.iter().any(|a| a == "libx264"));
    }
}

/// Container duration in seconds via ffprobe (used by the Compress window to size a bitrate).
pub fn probe_seconds(path: &std::path::Path) -> Option<f64> {
    probe_duration(path)
}

/// Container duration in seconds via ffprobe.
fn probe_duration(path: &std::path::Path) -> Option<f64> {
    let o = ffpipe::command(&ffpipe::ffprobe_exe()?)
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    String::from_utf8_lossy(&o.stdout).trim().parse().ok()
}

/// Output extensions offered by the converter UI.
pub const TARGETS: &[&str] = &["mp4", "mov", "mkv", "webm", "gif", "avi", "mp3", "wav", "m4a", "flac", "png", "jpg"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::export::tests::{gen_media, temp_dir, wait_done};

    fn opts(src: &std::path::Path, out: PathBuf) -> ConvertOptions {
        ConvertOptions {
            src: src.to_path_buf(),
            out,
            encoder: "auto".into(),
            crf: 30,
            preset: "ultrafast".into(),
            out_size: None,
            scaler: "bicubic".into(),
            gif_fps: 12,
            target_bytes: None,
        }
    }

    fn probe(path: &std::path::Path, entries: &str) -> String {
        let o = ffpipe::command(&ffpipe::ffprobe_exe().unwrap())
            .args(["-v", "error", "-show_entries", entries, "-of", "csv=p=0"])
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    #[test]
    fn convert_real() {
        let dir = temp_dir("convert");
        let Some(src) = gen_media(&dir) else {
            eprintln!("ffmpeg missing — skipped");
            return;
        };
        // → gif (palette chain)
        let gif = dir.join("out.gif");
        assert_eq!(wait_done(&start_convert(opts(&src, gif.clone()))), None);
        assert!(gif.exists());
        assert_eq!(probe(&gif, "format=format_name"), "gif");
        // → mp3, duration ≈ 4 s
        let mp3 = dir.join("out.mp3");
        assert_eq!(wait_done(&start_convert(opts(&src, mp3.clone()))), None);
        let d: f64 = probe(&mp3, "format=duration").parse().expect("duration");
        assert!((d - 4.0).abs() < 0.3, "{d}");
        // → mp4 rescaled to 160x120 with bicubic
        let mp4 = dir.join("small.mp4");
        let o = ConvertOptions { out_size: Some((160, 120)), ..opts(&src, mp4.clone()) };
        let prog = start_convert(o);
        assert_eq!(wait_done(&prog), None);
        assert!(prog.fraction() > 0.5, "{}", prog.fraction());
        assert_eq!(probe(&mp4, "stream=width,height").lines().next(), Some("160,120"));
        // both source audio streams survive (default stream selection would keep one)
        let audio = probe(&mp4, "stream=codec_type").lines().filter(|l| *l == "audio").count();
        assert_eq!(audio, 2, "audio streams kept");
        // failure (bad source) reports an error and leaves no output
        let bad = dir.join("bad.mp4");
        assert!(wait_done(&start_convert(opts(&dir.join("missing.mp4"), bad.clone()))).is_some());
        assert!(!bad.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
