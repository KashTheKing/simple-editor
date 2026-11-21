//! Smart render: save a plain cut (`export::lossless_segments`) frame-accurately without re-encoding
//! the whole video. Whole GOPs between the cuts are stream-copied; only the partial GOP at each cut is
//! re-encoded, with the source's codec, profile, level, pixel format, size, frame rate and colour tags.
//!
//! Steps (`run`):
//! 1. Probe the source once per file version (size + mtime): stream parameters and the packet list
//!    (`ffprobe -show_entries packet=pts_time,dts_time,flags`, no decoding). Keyframes come from the
//!    packet flags; a keyframe whose GOP has leading frames (open GOP) is not a clean entry point and
//!    is dropped from the list, so copies only ever start where decoding can start.
//! 2. `export::smart_spans` plans copy / encode spans; each span becomes an MPEG-TS part (Annex B, so
//!    every part carries its own SPS/PPS in-band and the joins decode with either encoder's headers).
//!    Spans are cut by frame count, never by timestamps, so no frame is lost or doubled at a join.
//! 3. The concat demuxer joins the parts (with each part's exact duration) into the output container,
//!    adding the audio: stream-copied for a single segment, re-encoded with the source's codec when
//!    there are several segments (sample-accurate, so A/V sync holds at every join).
//! 4. The result is probed (frame count, duration within one frame, codec / size / pixel format) and
//!    every join is decoded; any mismatch or decode error is an Err, and the caller re-encodes.
//!
//! Anything that can't be matched (codec other than H.264 / HEVC, interlaced, unknown timestamps, no
//! suitable encoder, a container other than MP4 / MOV / M4V / MKV) is an Err before any work is done.

use super::export::{Progress, SmartSpan, TempFile};
use crate::media::ffpipe;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Containers the smart render writes (they all take H.264 / HEVC from an MPEG-TS concat).
pub const CONTAINERS: &[&str] = &["mp4", "mov", "m4v", "mkv"];

/// The source's first video stream, as ffprobe reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoParams {
    pub codec: String,
    pub profile: String,
    pub level: i64,
    pub pix_fmt: String,
    pub width: u32,
    pub height: u32,
    /// `r_frame_rate` as (num, den).
    pub rate: (u64, u64),
    /// Denominator of the stream time base (the MP4 track timescale to keep).
    pub timescale: u64,
    pub color_range: String,
    pub color_space: String,
    pub color_transfer: String,
    pub color_primaries: String,
    pub field_order: String,
    /// Sample entry tag in the source container ("avc1", "hvc1", …; empty when not a fourcc).
    pub tag: String,
    pub bit_rate: u64,
}

impl VideoParams {
    pub fn fps(&self) -> f64 {
        if self.rate.1 == 0 {
            0.0
        } else {
            self.rate.0 as f64 / self.rate.1 as f64
        }
    }
}

/// One source audio stream.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioParams {
    pub codec: String,
    pub bit_rate: u64,
}

/// What `run` needs to know about the source.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourceInfo {
    pub video: VideoParams,
    pub audio: Vec<AudioParams>,
}

/// The source's video frames: every presentation time (sorted) and the clean keyframes (sorted), in
/// seconds from the file's start (what ffmpeg's `-ss` counts from).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frames {
    pub pts: Vec<f64>,
    pub keyframes: Vec<f64>,
}

impl Frames {
    /// Frames presented in `[a, b)`.
    pub fn count(&self, a: f64, b: f64) -> u64 {
        let lo = self.pts.partition_point(|&t| t < a);
        let hi = self.pts.partition_point(|&t| t < b);
        hi.saturating_sub(lo) as u64
    }
    /// Where a copy may start or end: the clean keyframes, plus the end of the last frame (a piece that
    /// runs to the end of the file copies its last GOP too).
    pub fn entry_points(&self, fps: f64) -> Vec<f64> {
        let mut k = self.keyframes.clone();
        if let (Some(&last), true) = (self.pts.last(), fps > 0.0) {
            k.push(last + 1.0 / fps);
        }
        k
    }
    /// First frame presented at or after `a`.
    pub fn first_at(&self, a: f64) -> Option<f64> {
        self.pts.get(self.pts.partition_point(|&t| t < a)).copied()
    }
}

/// What a finished smart render did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub copied_frames: u64,
    pub encoded_frames: u64,
    pub parts: usize,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}
fn n(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0)
}

/// ffprobe `-show_entries stream=… -of json` → the first video stream and the audio streams in order.
pub fn parse_streams(json: &str) -> Option<SourceInfo> {
    let v: Value = serde_json::from_str(json).ok()?;
    let streams = v.get("streams")?.as_array()?;
    let mut info = SourceInfo::default();
    let mut have_video = false;
    for st in streams {
        match st.get("codec_type").and_then(Value::as_str) {
            Some("video") if !have_video => {
                // cover art / thumbnails are "video" streams too
                if st.get("disposition").and_then(|d| d.get("attached_pic")).and_then(Value::as_i64) == Some(1) {
                    continue;
                }
                have_video = true;
                let rate = s(st, "r_frame_rate");
                let (num, den) = rate.split_once('/').unwrap_or((&rate, "1"));
                let tb = s(st, "time_base");
                let tag = s(st, "codec_tag_string");
                info.video = VideoParams {
                    codec: s(st, "codec_name"),
                    profile: s(st, "profile"),
                    level: n(st, "level"),
                    pix_fmt: s(st, "pix_fmt"),
                    width: n(st, "width").max(0) as u32,
                    height: n(st, "height").max(0) as u32,
                    rate: (num.trim().parse().unwrap_or(0), den.trim().parse().unwrap_or(0)),
                    timescale: tb.split_once('/').and_then(|(_, d)| d.trim().parse().ok()).unwrap_or(0),
                    color_range: s(st, "color_range"),
                    color_space: s(st, "color_space"),
                    color_transfer: s(st, "color_transfer"),
                    color_primaries: s(st, "color_primaries"),
                    field_order: s(st, "field_order"),
                    tag: if tag.len() == 4 && tag.chars().all(|c| c.is_ascii_alphanumeric()) {
                        tag
                    } else {
                        String::new()
                    },
                    bit_rate: n(st, "bit_rate").max(0) as u64,
                };
            }
            Some("audio") => {
                info.audio.push(AudioParams { codec: s(st, "codec_name"), bit_rate: n(st, "bit_rate").max(0) as u64 })
            }
            _ => {}
        }
    }
    have_video.then_some(info)
}

/// ffprobe `-select_streams v:0 -show_entries packet=pts_time,dts_time,flags -of csv=p=0` (packets in
/// decode order) → frames relative to `start` (the container's start time). None when a packet has no
/// timestamp. A keyframe is kept only when no later packet of its GOP is presented before it: with
/// leading frames (an open GOP) a copy starting there would show frames that reference the previous GOP.
pub fn parse_packets(csv: &str, start: f64) -> Option<Frames> {
    let mut pkts: Vec<(f64, bool)> = Vec::new();
    for line in csv.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let mut it = line.split(',');
        let pts: f64 = it.next()?.trim().parse().ok()?;
        let _dts = it.next();
        let key = it.next().unwrap_or("").starts_with('K');
        pkts.push((pts - start, key));
    }
    if pkts.is_empty() {
        return None;
    }
    let mut keyframes = Vec::new();
    for (i, &(t, key)) in pkts.iter().enumerate() {
        if !key {
            continue;
        }
        let leading = pkts[i + 1..].iter().take_while(|p| !p.1).any(|p| p.0 < t - 1e-6);
        if !leading {
            keyframes.push(t);
        }
    }
    let mut pts: Vec<f64> = pkts.iter().map(|p| p.0).collect();
    pts.sort_by(f64::total_cmp);
    keyframes.sort_by(f64::total_cmp);
    Some(Frames { pts, keyframes })
}

/// One part of the render: `frames` frames starting with the one presented at `first` (source seconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Part {
    pub copy: bool,
    pub first: f64,
    pub frames: u64,
}

/// Turn the span plan into parts with exact frame counts. Each span covers the frames presented in
/// `[a - h, b - h)` (`h` = half a frame), so consecutive spans share no frame and miss none, and a
/// copy span starts on its keyframe. Empty spans are dropped. Err when a copy would not start on a
/// keyframe (the timestamps don't line up with the probe).
pub fn plan_parts(spans: &[SmartSpan], frames: &Frames, fps: f64) -> Result<Vec<Part>, String> {
    if fps <= 0.0 {
        return Err("unknown frame rate".into());
    }
    let h = 0.5 / fps;
    let mut out = Vec::new();
    for sp in spans {
        let (copy, a, b) = match *sp {
            SmartSpan::Copy(a, b) => (true, a, b),
            SmartSpan::Encode(a, b) => (false, a, b),
        };
        let n = frames.count(a - h, b - h);
        if n == 0 {
            continue;
        }
        let first = frames.first_at(a - h).ok_or("no frame at the cut")?;
        if copy && !frames.keyframes.iter().any(|&k| (k - first).abs() < 1e-6) {
            return Err(format!("copy at {first:.3}s doesn't start on a keyframe"));
        }
        out.push(Part { copy, first, frames: n });
    }
    if out.is_empty() {
        return Err("nothing to render".into());
    }
    Ok(out)
}

/// ffmpeg video encoder arguments that reproduce the source's stream (codec, profile, level, pixel
/// format, colour tags) for an encoded part. `available` = ffmpeg's encoder list (empty = unknown,
/// assume the software encoders exist), `hardware` = the hardware encoders that passed the probe
/// (used only when the software encoder is missing). None when the source can't be matched.
pub fn encoder_args(v: &VideoParams, available: &[String], hardware: &[String]) -> Option<Vec<String>> {
    if !matches!(v.field_order.as_str(), "" | "progressive" | "unknown") {
        return None; // interlaced: field order and MBAFF/PAFF can't be matched reliably
    }
    if v.width == 0 || v.height == 0 || v.fps() <= 0.0 {
        return None;
    }
    let has = |e: &str| available.is_empty() || available.iter().any(|a| a == e);
    let ten_bit = v.pix_fmt.contains("10");
    let mut a: Vec<String> = Vec::new();
    let push = |a: &mut Vec<String>, x: &[&str]| a.extend(x.iter().map(|s| s.to_string()));
    match v.codec.as_str() {
        "h264" if has("libx264") => {
            if !matches!(
                v.pix_fmt.as_str(),
                "yuv420p" | "yuvj420p" | "yuv422p" | "yuv444p" | "yuv420p10le" | "yuv422p10le"
            ) {
                return None;
            }
            push(&mut a, &["-c:v", "libx264", "-preset", "fast", "-crf", "16", "-pix_fmt", &v.pix_fmt]);
            if let Some(p) = x264_profile(&v.profile) {
                push(&mut a, &["-profile:v", p]);
            }
            if v.level > 9 {
                push(&mut a, &["-level:v", &format!("{}.{}", v.level / 10, v.level % 10)]);
            }
            // distinct parameter-set ids from the source's (0): both sets can live in one stream
            push(&mut a, &["-x264-params", "sps-id=1"]);
        }
        "hevc" if has("libx265") => {
            if !matches!(v.pix_fmt.as_str(), "yuv420p" | "yuv420p10le" | "yuv422p" | "yuv422p10le" | "yuv444p") {
                return None;
            }
            push(&mut a, &["-c:v", "libx265", "-preset", "fast", "-crf", "18", "-pix_fmt", &v.pix_fmt]);
            push(&mut a, &["-profile:v", if ten_bit { "main10" } else { "main" }]);
            push(&mut a, &["-x265-params", "open-gop=0:repeat-headers=1:log-level=error"]);
        }
        "h264" | "hevc" => {
            // no software encoder: a hardware one of the same family, 8-bit 4:2:0 only
            if ten_bit || !matches!(v.pix_fmt.as_str(), "yuv420p" | "yuvj420p" | "nv12") {
                return None;
            }
            let fam = if v.codec == "h264" { "h264" } else { "hevc" };
            let hw = ["nvenc", "qsv", "amf"].iter().map(|k| format!("{fam}_{k}")).find(|e| hardware.contains(e))?;
            let kbps = (v.bit_rate / 1000).max(4000) * 3 / 2;
            push(&mut a, &["-c:v", &hw, "-pix_fmt", "yuv420p", "-b:v", &format!("{kbps}k")]);
        }
        _ => return None,
    }
    let tag = |a: &mut Vec<String>, flag: &str, val: &str| {
        if !val.is_empty() && val != "unknown" && val != "reserved" {
            a.extend([flag.to_string(), val.to_string()]);
        }
    };
    tag(&mut a, "-color_range", &v.color_range);
    tag(&mut a, "-colorspace", &v.color_space);
    tag(&mut a, "-color_trc", &v.color_transfer);
    tag(&mut a, "-color_primaries", &v.color_primaries);
    Some(a)
}

/// ffprobe's H.264 profile name → libx264's `-profile:v`.
fn x264_profile(p: &str) -> Option<&'static str> {
    Some(match p.to_ascii_lowercase().as_str() {
        "baseline" | "constrained baseline" => "baseline",
        "main" => "main",
        "high" => "high",
        "high 10" | "high 10 intra" => "high10",
        "high 4:2:2" | "high 4:2:2 intra" => "high422",
        "high 4:4:4 predictive" | "high 4:4:4 intra" => "high444",
        _ => return None,
    })
}

/// Audio encoder for a re-encoded (multi-segment) audio stream of codec `codec`. None = can't match.
pub fn audio_args(codec: &str, bit_rate: u64, o: usize) -> Option<Vec<String>> {
    let kbps = if bit_rate > 0 { (bit_rate / 1000).clamp(64, 512) } else { 192 };
    let enc = match codec {
        "aac" => "aac",
        "mp3" => "libmp3lame",
        "opus" => "libopus",
        "vorbis" => "libvorbis",
        "ac3" => "ac3",
        "eac3" => "eac3",
        "flac" | "alac" => codec,
        c if c.starts_with("pcm_") => c,
        _ => return None,
    };
    let mut a = vec![format!("-c:a:{o}"), enc.to_string()];
    if !matches!(enc, "flac" | "alac") && !enc.starts_with("pcm_") {
        a.extend([format!("-b:a:{o}"), format!("{kbps}k")]);
    }
    Some(a)
}

// ---------------------------------------------------------------- probe (cached per file version)

type ProbeKey = (PathBuf, u64, Option<SystemTime>);
static PROBES: Mutex<Vec<(ProbeKey, Arc<(SourceInfo, Frames)>)>> = Mutex::new(Vec::new());

fn probe_key(src: &Path) -> Result<ProbeKey, String> {
    let m = std::fs::metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
    Ok((src.to_path_buf(), m.len(), m.modified().ok()))
}

/// Stream parameters and frame list of `src`, from the cache while the file's size and mtime match.
pub fn probe(src: &Path) -> Result<Arc<(SourceInfo, Frames)>, String> {
    let key = probe_key(src)?;
    if let Some((_, p)) = PROBES.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(k, _)| *k == key) {
        return Ok(p.clone());
    }
    let ffprobe = ffpipe::ffprobe_exe().ok_or("ffprobe.exe not found")?;
    let run = |args: &[&str]| -> Result<String, String> {
        let o = ffpipe::command(&ffprobe)
            .args(["-v", "error"])
            .args(args)
            .arg(src)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("ffprobe: {e}"))?;
        if !o.status.success() {
            return Err(format!("ffprobe: {}", String::from_utf8_lossy(&o.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let streams = run(&[
        "-show_entries",
        "format=start_time:stream=codec_type,codec_name,profile,level,pix_fmt,width,height,r_frame_rate,time_base,\
         color_range,color_space,color_transfer,color_primaries,field_order,codec_tag_string,bit_rate:\
         stream_disposition=attached_pic",
        "-of",
        "json",
    ])?;
    let info = parse_streams(&streams).ok_or("no video stream")?;
    let start = serde_json::from_str::<Value>(&streams)
        .ok()
        .and_then(|v| v.get("format")?.get("start_time")?.as_str()?.parse::<f64>().ok())
        .unwrap_or(0.0);
    let csv = run(&["-select_streams", "v:0", "-show_entries", "packet=pts_time,dts_time,flags", "-of", "csv=p=0"])?;
    let frames = parse_packets(&csv, start).ok_or("the video has packets without timestamps")?;
    let p = Arc::new((info, frames));
    let mut cache = PROBES.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|(k, _)| k.0 != key.0);
    if cache.len() >= 8 {
        cache.remove(0);
    }
    cache.push((key, p.clone()));
    Ok(p)
}

// ---------------------------------------------------------------- the runner

fn ffmpeg_run(mut cmd: std::process::Command, prog: &Progress) -> Result<(), String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
    let tail = super::export::stderr_tail(&mut child);
    super::export::wait_ffmpeg(&mut child, tail, prog)
}

/// Smart-render `segs` (source `(in, duration)` pairs, as `lossless_segments` returns them) of `src`
/// into `out`, keeping the source audio streams `audio` (indexes among the audio streams). `metadata`
/// is written as container tags (the rest is stripped, like a normal export). Writes to a hidden
/// sibling and renames over `out` only after the result passed its checks.
pub fn run(
    src: &Path,
    segs: &[(f64, f64)],
    audio: &[usize],
    out: &Path,
    metadata: &[(String, String)],
    prog: &Progress,
) -> Result<Report, String> {
    let ext = out.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if !CONTAINERS.contains(&ext.as_str()) {
        return Err(format!("{ext} output"));
    }
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    prog.set(0.0, "Reading keyframes…");
    let probed = probe(src)?;
    let (info, frames) = (&probed.0, &probed.1);
    let v = &info.video;
    let fps = v.fps();
    let enc = encoder_args(v, &super::export::detect_encoders(), &crate::engine::encoders::working())
        .ok_or_else(|| format!("no matching encoder for {} {}", v.codec, v.pix_fmt))?;
    let mut streams = Vec::new();
    for &s in audio {
        streams.push(info.audio.get(s).ok_or_else(|| format!("audio stream {s} missing"))?.clone());
    }
    let multi = segs.len() > 1;
    let mut audio_enc = Vec::new();
    if multi {
        for (o, a) in streams.iter().enumerate() {
            audio_enc.extend(audio_args(&a.codec, a.bit_rate, o).ok_or_else(|| format!("{} audio", a.codec))?);
        }
    }
    let parts = plan_parts(&super::export::smart_spans(segs, &frames.entry_points(fps)), frames, fps)?;
    let total: u64 = parts.iter().map(|p| p.frames).sum();
    let encoded: u64 = parts.iter().filter(|p| !p.copy).map(|p| p.frames).sum();

    // progress weights: an encoded frame costs ~50 copied ones; joining and checking ~ a copy pass each
    let w_frame = |p: &Part| if p.copy { 0.02 } else { 1.0 } * p.frames as f64;
    let w_join = total as f64 * 0.02 + 1.0;
    let w_all = parts.iter().map(w_frame).sum::<f64>() + 2.0 * w_join;

    let stem = out.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let hidden = |name: String| out.with_file_name(format!(".{stem}.{name}"));
    let q = 0.25 / fps;
    let files: Vec<TempFile> = (0..parts.len()).map(|i| TempFile(hidden(format!("smart{i}.ts")))).collect();
    let part_cmd = |i: usize| {
        let p = &parts[i];
        let mut cmd = ffpipe::command(&ffmpeg);
        cmd.args(["-y", "-hide_banner", "-loglevel", "error"]);
        // a copy seeks just past its keyframe (ffmpeg starts a copy at the keyframe at or before -ss);
        // an encode seeks just before its first frame (decoded from the keyframe before, then dropped)
        let seek = if p.copy { p.first + q } else { (p.first - q).max(0.0) };
        cmd.args(["-ss", &format!("{seek:.6}"), "-i"]).arg(src);
        cmd.args(["-map", "0:v:0", "-an", "-sn", "-dn", "-frames:v", &p.frames.to_string()]);
        if p.copy {
            cmd.args(["-c:v", "copy"]);
        } else {
            cmd.args(&enc);
        }
        cmd.args(["-f", "mpegts"]).arg(&files[i].0);
        cmd
    };
    // the parts are independent: up to 4 at once, the encodes first (each one is short, so a single
    // x264 never fills the CPU on its own)
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by_key(|&i| parts[i].copy);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let state = Mutex::new((0.0f64, parts.iter().filter(|p| !p.copy).count(), None::<String>));
    prog.set(0.0, if encoded > 0 { "Encoding the cut…" } else { "Copying…" });
    std::thread::scope(|sc| {
        for _ in 0..order.len().min(4) {
            sc.spawn(|| loop {
                let k = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if k >= order.len() || state.lock().unwrap_or_else(|e| e.into_inner()).2.is_some() {
                    return;
                }
                let i = order[k];
                let r = ffmpeg_run(part_cmd(i), prog);
                let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
                match r {
                    Ok(()) => {
                        st.0 += w_frame(&parts[i]);
                        st.1 -= usize::from(!parts[i].copy);
                        prog.set((st.0 / w_all) as f32, if st.1 > 0 { "Encoding the cut…" } else { "Copying…" });
                    }
                    Err(e) => {
                        st.2.get_or_insert(e);
                    }
                }
            });
        }
    });
    let (mut done, _, failed) = state.into_inner().unwrap_or_else(|e| e.into_inner());
    if let Some(e) = failed {
        return Err(e);
    }

    prog.set((done / w_all) as f32, "Joining…");
    let list = TempFile(hidden("smart.txt".into()));
    let mut txt = String::from("ffconcat version 1.0\n");
    for (f, p) in files.iter().zip(&parts) {
        let path = f.0.to_string_lossy().replace('\\', "/").replace('\'', "'\\''");
        txt.push_str(&format!("file '{path}'\nduration {:.9}\n", p.frames as f64 / fps));
    }
    std::fs::write(&list.0, txt).map_err(|e| format!("concat list: {e}"))?;
    let tmp = super::export::temp_output(out);
    let mut cmd = ffpipe::command(&ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i"]).arg(&list.0);
    if !streams.is_empty() {
        if multi {
            cmd.arg("-i").arg(src);
        } else {
            let (a, d) = segs[0];
            cmd.args(["-ss", &format!("{a:.6}"), "-t", &format!("{d:.6}"), "-i"]).arg(src);
        }
    }
    cmd.args(["-map", "0:v:0"]);
    if multi && !streams.is_empty() {
        let mut graph = String::new();
        for (o, &s) in audio.iter().enumerate() {
            for (j, &(a, d)) in segs.iter().enumerate() {
                graph
                    .push_str(&format!("[1:a:{s}]atrim=start={a:.6}:end={:.6},asetpts=PTS-STARTPTS[s{o}_{j}];", a + d));
            }
            for j in 0..segs.len() {
                graph.push_str(&format!("[s{o}_{j}]"));
            }
            graph.push_str(&format!("concat=n={}:v=0:a=1[a{o}];", segs.len()));
        }
        graph.pop();
        cmd.args(["-filter_complex", &graph]);
        for o in 0..audio.len() {
            cmd.args(["-map", &format!("[a{o}]")]);
        }
        cmd.args(["-c:v", "copy"]).args(&audio_enc);
    } else {
        for &s in audio {
            cmd.args(["-map", &format!("1:a:{s}")]);
        }
        cmd.args(["-c", "copy"]);
    }
    cmd.args(["-map_metadata", "-1", "-map_chapters", "-1"]);
    for (o, &s) in audio.iter().enumerate() {
        cmd.args([format!("-map_metadata:s:a:{o}"), format!("1:s:a:{s}")]);
    }
    for (k, val) in metadata {
        cmd.args(["-metadata", &format!("{k}={val}")]);
    }
    if ext != "mkv" {
        if matches!(v.tag.as_str(), "avc1" | "avc3" | "hvc1" | "hev1") {
            cmd.args(["-tag:v", &v.tag]);
        }
        if (1..=1_000_000).contains(&v.timescale) {
            cmd.args(["-video_track_timescale", &v.timescale.to_string()]);
        }
        cmd.args(["-movflags", "+faststart"]);
    }
    cmd.arg(&tmp.0);
    ffmpeg_run(cmd, prog)?;
    drop(files);
    done += w_join;

    prog.set((done / w_all) as f32, "Checking the result…");
    verify(&tmp.0, v, total, audio.len(), &parts, fps)?;
    tmp.commit(out)?;
    prog.set(1.0, "Done");
    Ok(Report { copied_frames: total - encoded, encoded_frames: encoded, parts: parts.len() })
}

/// The written file has the expected frame count, a video duration within one frame of it, the
/// source's codec / size / pixel format, the audio streams asked for, and decodes cleanly around
/// every join.
fn verify(out: &Path, v: &VideoParams, frames: u64, audio: usize, parts: &[Part], fps: f64) -> Result<(), String> {
    let ffprobe = ffpipe::ffprobe_exe().ok_or("ffprobe.exe not found")?;
    let o = ffpipe::command(&ffprobe)
        .args(["-v", "error", "-count_packets", "-show_entries"])
        .args(["stream=codec_type,codec_name,width,height,pix_fmt,duration,nb_read_packets", "-of", "json"])
        .arg(out)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ffprobe: {e}"))?;
    let j: Value = serde_json::from_slice(&o.stdout).map_err(|_| "the result can't be read".to_string())?;
    let streams = j.get("streams").and_then(Value::as_array).cloned().unwrap_or_default();
    let video = streams.iter().find(|st| s(st, "codec_type") == "video").ok_or("the result has no video")?;
    let got = (s(video, "codec_name"), n(video, "width") as u32, n(video, "height") as u32, s(video, "pix_fmt"));
    if got != (v.codec.clone(), v.width, v.height, v.pix_fmt.clone()) {
        return Err(format!("the result is {} {}x{} {}", got.0, got.1, got.2, got.3));
    }
    let count = n(video, "nb_read_packets") as u64;
    if count != frames {
        return Err(format!("{count} frames written, expected {frames}"));
    }
    if let Ok(d) = s(video, "duration").parse::<f64>() {
        let want = frames as f64 / fps;
        if (d - want).abs() > 1.0 / fps + 1e-3 {
            return Err(format!("video lasts {d:.3}s, expected {want:.3}s"));
        }
    }
    let a = streams.iter().filter(|st| s(st, "codec_type") == "audio").count();
    if a != audio {
        return Err(format!("{a} audio streams written, expected {audio}"));
    }
    // decode a short window around each join: a reference into the wrong GOP or a parameter-set
    // mismatch shows up as decoder errors
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let mut t = 0u64;
    for w in parts.windows(2) {
        t += w[0].frames;
        let at = t as f64 / fps;
        let o = ffpipe::command(&ffmpeg)
            .args(["-v", "error", "-ss", &format!("{:.6}", (at - 0.25).max(0.0)), "-i"])
            .arg(out)
            .args(["-t", "0.5", "-map", "0:v:0", "-f", "null", "-"])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("ffmpeg: {e}"))?;
        let err = String::from_utf8_lossy(&o.stderr);
        if !o.status.success() || !err.trim().is_empty() {
            return Err(format!(
                "the join at {at:.2}s doesn't decode cleanly: {}",
                err.lines().next().unwrap_or("").trim()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kf(v: &[f64]) -> Vec<f64> {
        v.to_vec()
    }

    #[test]
    fn packets_closed_gop_keyframes_and_frame_list() {
        // decode order with B-frames (pts out of order inside the GOP), keyframes at 0 and 2
        let csv = "0.000,-0.033,K__\n0.050,-0.016,___\n0.016,0.000,___\n0.033,0.016,___\n\
                   2.000,1.966,K__\n2.050,1.983,___\n2.016,2.000,___\n";
        let f = parse_packets(csv, 0.0).unwrap();
        assert_eq!(f.keyframes, kf(&[0.0, 2.0]));
        assert_eq!(f.pts.len(), 7);
        assert!(f.pts.windows(2).all(|w| w[0] <= w[1]), "sorted");
        // a container start time shifts everything to what -ss counts from
        let g = parse_packets(csv, 1.4).unwrap();
        assert!((g.keyframes[1] - 0.6).abs() < 1e-9);
    }

    #[test]
    fn open_gop_keyframes_are_not_entry_points() {
        // the keyframe at 2.0 is followed (in decode order) by frames shown before it: leading pictures
        let csv =
            "0.0,0,K\n0.033,0.016,_\n2.0,1.95,K\n1.95,1.96,_\n1.983,1.97,_\n2.033,2.0,_\n4.0,3.98,K\n4.033,4.0,_\n";
        assert_eq!(parse_packets(csv, 0.0).unwrap().keyframes, kf(&[0.0, 4.0]));
        assert_eq!(parse_packets("N/A,N/A,K\n", 0.0), None);
        assert_eq!(parse_packets("", 0.0), None);
    }

    /// 60 fps, keyframe every 2 s, 20 s.
    fn frames60() -> Frames {
        let pts: Vec<f64> = (0..1200).map(|i| i as f64 / 60.0).collect();
        Frames { pts, keyframes: (0..10).map(|k| k as f64 * 2.0).collect() }
    }

    #[test]
    fn parts_count_frames_exactly_and_never_share_one() {
        let f = frames60();
        let spans = crate::engine::export::smart_spans(&[(3.5, 12.25)], &f.keyframes);
        let parts = plan_parts(&spans, &f, 60.0).unwrap();
        assert_eq!(
            parts,
            vec![
                Part { copy: false, first: 3.5, frames: 30 },
                Part { copy: true, first: 4.0, frames: 600 },
                Part { copy: false, first: 14.0, frames: 105 },
            ]
        );
        assert_eq!(parts.iter().map(|p| p.frames).sum::<u64>(), 735); // 12.25 s * 60
                                                                      // an in point a hair off a frame still picks that frame; two segments, one with no whole GOP
        let spans = crate::engine::export::smart_spans(&[(1.25 + 1e-5, 0.5), (8.0, 4.0)], &f.keyframes);
        let parts = plan_parts(&spans, &f, 60.0).unwrap();
        assert_eq!(parts[0], Part { copy: false, first: 1.25, frames: 30 });
        assert_eq!(parts[1], Part { copy: true, first: 8.0, frames: 240 });
        assert_eq!(parts.len(), 2);
        // a copy that the frame list says isn't on a keyframe is refused, not guessed
        // a piece running to the end of the file copies its last GOP (the file's end counts as a boundary)
        let spans = crate::engine::export::smart_spans(&[(5.0, 15.0)], &f.entry_points(60.0));
        let parts = plan_parts(&spans, &f, 60.0).unwrap();
        assert_eq!(
            parts,
            vec![Part { copy: false, first: 5.0, frames: 60 }, Part { copy: true, first: 6.0, frames: 840 }]
        );
        let bad = Frames { keyframes: vec![0.0, 2.0005], ..frames60() };
        assert!(plan_parts(&[SmartSpan::Copy(2.0005, 4.0)], &bad, 60.0).is_err());
        assert!(plan_parts(&[], &f, 60.0).is_err());
        assert!(plan_parts(&spans, &f, 0.0).is_err());
    }

    fn h264() -> VideoParams {
        VideoParams {
            codec: "h264".into(),
            profile: "High".into(),
            level: 42,
            pix_fmt: "yuv420p".into(),
            width: 1920,
            height: 1080,
            rate: (60, 1),
            timescale: 15360,
            color_range: "tv".into(),
            color_space: "bt709".into(),
            color_transfer: "bt709".into(),
            color_primaries: "bt709".into(),
            field_order: "progressive".into(),
            tag: "avc1".into(),
            bit_rate: 12_000_000,
        }
    }

    #[test]
    fn encoder_matches_the_source_or_refuses() {
        let none: Vec<String> = Vec::new();
        let a = encoder_args(&h264(), &none, &none).unwrap().join(" ");
        for want in [
            "-c:v libx264",
            "-profile:v high",
            "-level:v 4.2",
            "-pix_fmt yuv420p",
            "-colorspace bt709",
            "-color_range tv",
        ] {
            assert!(a.contains(want), "{want} in {a}");
        }
        let ten = VideoParams { profile: "High 10".into(), pix_fmt: "yuv420p10le".into(), ..h264() };
        assert!(encoder_args(&ten, &none, &none).unwrap().join(" ").contains("-profile:v high10"));
        let hevc =
            VideoParams { codec: "hevc".into(), profile: "Main 10".into(), pix_fmt: "yuv420p10le".into(), ..h264() };
        let a = encoder_args(&hevc, &none, &none).unwrap().join(" ");
        assert!(a.contains("libx265") && a.contains("main10") && a.contains("open-gop=0"), "{a}");
        // no libx264: the probed hardware encoder of the same family, else nothing
        let ff: Vec<String> = vec!["aac".into(), "h264_nvenc".into()];
        let hw: Vec<String> = vec!["h264_nvenc".into()];
        assert!(encoder_args(&h264(), &ff, &hw).unwrap().join(" ").contains("-c:v h264_nvenc"));
        assert_eq!(encoder_args(&h264(), &ff, &none), None);
        assert_eq!(encoder_args(&hevc, &ff, &hw), None, "no hevc encoder");
        // can't match: other codecs, interlaced, unknown rate
        for bad in [
            VideoParams { codec: "vp9".into(), ..h264() },
            VideoParams { codec: "mpeg4".into(), ..h264() },
            VideoParams { field_order: "tt".into(), ..h264() },
            VideoParams { rate: (0, 0), ..h264() },
            VideoParams { pix_fmt: "yuv420p12le".into(), ..h264() },
        ] {
            assert_eq!(encoder_args(&bad, &none, &none), None, "{bad:?}");
        }
    }

    #[test]
    fn audio_reencode_keeps_the_codec() {
        assert_eq!(audio_args("aac", 128_000, 1).unwrap().join(" "), "-c:a:1 aac -b:a:1 128k");
        assert_eq!(audio_args("aac", 0, 0).unwrap().join(" "), "-c:a:0 aac -b:a:0 192k");
        assert_eq!(audio_args("pcm_s16le", 1_536_000, 0).unwrap().join(" "), "-c:a:0 pcm_s16le");
        assert_eq!(audio_args("truehd", 0, 0), None);
    }

    #[test]
    fn stream_probe_json() {
        let j = r#"{"streams":[
            {"codec_type":"video","codec_name":"mjpeg","disposition":{"attached_pic":1}},
            {"codec_type":"video","codec_name":"h264","profile":"Main","level":32,"pix_fmt":"yuv420p","width":1280,
             "height":720,"r_frame_rate":"60000/1001","time_base":"1/60000","codec_tag_string":"avc1","field_order":"progressive",
             "bit_rate":"403346","disposition":{"attached_pic":0}},
            {"codec_type":"audio","codec_name":"aac","bit_rate":"128000"},
            {"codec_type":"audio","codec_name":"opus"}],"format":{"start_time":"0.000000"}}"#;
        let i = parse_streams(j).unwrap();
        assert_eq!(i.video.codec, "h264");
        assert_eq!((i.video.rate, i.video.timescale, i.video.level), ((60000, 1001), 60000, 32));
        assert!((i.video.fps() - 59.94).abs() < 0.01);
        assert_eq!(
            i.audio,
            vec![
                AudioParams { codec: "aac".into(), bit_rate: 128_000 },
                AudioParams { codec: "opus".into(), bit_rate: 0 }
            ]
        );
        assert_eq!(parse_streams(r#"{"streams":[{"codec_type":"audio","codec_name":"aac"}]}"#), None);
    }

    // ---------------------------------------------------------------- real ffmpeg

    fn tool_runs(name: &str) -> bool {
        let exe = if name == "ffmpeg" { ffpipe::ffmpeg_exe() } else { ffpipe::ffprobe_exe() };
        exe.is_some()
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("simple-editor-smart-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// testsrc 1280x720 @ 60 fps, closed GOP of exactly 2 s (120 frames, B-frames on), 40 s, two sine
    /// streams with a language and a title.
    fn gen_source(d: &Path, vcodec: &str, ext: &str) -> PathBuf {
        let out = d.join(format!("src.{ext}"));
        let mut cmd = ffpipe::command(&ffpipe::ffmpeg_exe().unwrap());
        cmd.args(["-y", "-hide_banner", "-loglevel", "error"])
            .args(["-f", "lavfi", "-i", "testsrc=s=1280x720:r=60:d=40"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=40"])
            .args(["-f", "lavfi", "-i", "sine=frequency=880:duration=40"])
            .args(["-map", "0:v", "-map", "1:a", "-map", "2:a", "-c:v", vcodec]);
        if vcodec == "libx264" {
            cmd.args(["-preset", "veryfast", "-bf", "2", "-g", "120", "-keyint_min", "120", "-sc_threshold", "0"]);
        } else {
            cmd.args(["-g", "120"]);
        }
        cmd.args(["-pix_fmt", "yuv420p", "-c:a", if ext == "webm" { "libopus" } else { "aac" }])
            .args(["-metadata:s:a:0", "language=eng", "-metadata:s:a:1", "title=Music"])
            .arg(&out);
        assert!(cmd.status().unwrap().success(), "could not generate the test source");
        out
    }

    /// Lowest per-frame PSNR of `out` against the same source ranges, frame by frame (both sides are
    /// re-timed by frame index, so MKV's millisecond timestamps can't pair the wrong frames).
    fn min_psnr(out: &Path, src: &Path, segs: &[(f64, f64)]) -> f64 {
        let mut g = String::new();
        for (j, (a, d)) in segs.iter().enumerate() {
            g.push_str(&format!("[1:v]trim=start={a}:duration={d},setpts=PTS-STARTPTS[r{j}];"));
        }
        for j in 0..segs.len() {
            g.push_str(&format!("[r{j}]"));
        }
        g.push_str(&format!(
            "concat=n={}:v=1:a=0,setpts=N/(60*TB)[r];[0:v]setpts=N/(60*TB)[o];[o][r]psnr=stats_file=-",
            segs.len()
        ));
        let o = ffpipe::command(&ffpipe::ffmpeg_exe().unwrap())
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(out)
            .arg("-i")
            .arg(src)
            .args(["-lavfi", &g, "-f", "null", "-"])
            .output()
            .unwrap();
        let txt = String::from_utf8_lossy(&o.stdout);
        let vals: Vec<f64> = txt
            .lines()
            .filter_map(|l| l.split_whitespace().find_map(|w| w.strip_prefix("psnr_avg:")))
            .map(|v| if v == "inf" { f64::INFINITY } else { v.parse().unwrap_or(0.0) })
            .collect();
        assert!(!vals.is_empty(), "no psnr output: {}", String::from_utf8_lossy(&o.stderr));
        vals.into_iter().fold(f64::INFINITY, f64::min)
    }

    /// Every video frame's pts in the output, sorted.
    fn out_pts(out: &Path) -> Vec<f64> {
        let o = ffpipe::command(&ffpipe::ffprobe_exe().unwrap())
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "packet=pts_time", "-of", "csv=p=0"])
            .arg(out)
            .output()
            .unwrap();
        let mut v: Vec<f64> =
            String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| l.trim().parse().ok()).collect();
        v.sort_by(f64::total_cmp);
        v
    }

    fn stream_tags(out: &Path) -> String {
        let o = ffpipe::command(&ffpipe::ffprobe_exe().unwrap())
            .args(["-v", "error", "-show_entries", "stream=codec_type:stream_tags=language,title", "-of", "compact"])
            .arg(out)
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    #[test]
    fn smart_render_real_is_frame_accurate_and_encodes_only_the_cut_gops() {
        if !tool_runs("ffmpeg") || !tool_runs("ffprobe") {
            eprintln!("ffmpeg missing - skipped");
            return;
        }
        let d = dir("accurate");
        let src = gen_source(&d, "libx264", "mp4");
        let ffmpeg = ffpipe::ffmpeg_exe().unwrap();

        // the full re-encode of the same range, for the time budget
        let t = std::time::Instant::now();
        let st = ffpipe::command(&ffmpeg)
            .args(["-y", "-hide_banner", "-loglevel", "error", "-ss", "3.5", "-t", "28.25", "-i"])
            .arg(&src)
            .args(["-map", "0", "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-c:a", "aac"])
            .arg(d.join("full.mp4"))
            .status()
            .unwrap();
        assert!(st.success());
        let full = t.elapsed();

        // trim mid-GOP at both ends: 3.5 s (1.75 GOPs in) to 31.75 s; then two pieces, one with no whole GOP
        for (name, segs) in [("one.mp4", vec![(3.5, 28.25)]), ("two.mkv", vec![(1.25, 3.0), (9.5, 22.25)])] {
            let out = d.join(name);
            let prog = Progress::new();
            let t = std::time::Instant::now();
            let r = run(&src, &segs, &[0, 1], &out, &[], &prog).unwrap_or_else(|e| panic!("{name}: {e}"));
            let took = t.elapsed();
            let want: u64 = segs.iter().map(|&(_, d)| (d * 60.0f64).round() as u64).sum();
            assert_eq!(r.copied_frames + r.encoded_frames, want, "{name}: {r:?}");
            assert!(r.encoded_frames <= 2 * 120 * segs.len() as u64, "{name}: about 2 GOPs per segment, {r:?}");
            assert!(r.copied_frames > r.encoded_frames, "{name}: {r:?}");
            // frame-accurate: first frame at 0, last at (n-1)/fps, evenly spaced, and the right pictures
            let pts = out_pts(&out);
            assert_eq!(pts.len() as u64, want, "{name}");
            assert!(pts[0].abs() < 1e-3, "{name}: first pts {}", pts[0]);
            assert!(
                (pts[pts.len() - 1] - (want - 1) as f64 / 60.0).abs() < 2e-3,
                "{name}: last pts {}",
                pts[pts.len() - 1]
            );
            assert!(pts.windows(2).all(|w| (w[1] - w[0] - 1.0 / 60.0).abs() < 2e-3), "{name}: uneven frame times");
            let psnr = min_psnr(&out, &src, &segs);
            assert!(psnr > 40.0, "{name}: a frame doesn't match the source (min PSNR {psnr:.1} dB)");
            // audio streams kept, with their tags (MP4 keeps the language; it never stored the title)
            let tags = stream_tags(&out);
            assert_eq!(tags.matches("codec_type=audio").count(), 2, "{name}: {tags}");
            assert!(tags.contains("language=eng"), "{name}: {tags}");
            // well under the full re-encode
            assert!(took * 3 < full, "{name}: smart {took:?} vs full {full:?}");
            eprintln!("{name}: {r:?} in {took:?} (full re-encode {full:?}), min PSNR {psnr:.1} dB");
        }
        // the frame list is cached per file version: a second probe is instant and identical
        let a = probe(&src).unwrap();
        assert!(Arc::ptr_eq(&a, &probe(&src).unwrap()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn smart_render_refuses_what_it_cant_match_and_leaves_the_output_alone() {
        if !tool_runs("ffmpeg") || !tool_runs("ffprobe") {
            eprintln!("ffmpeg missing - skipped");
            return;
        }
        let d = dir("refuse");
        // MPEG-4 Part 2: no matching encoder path
        let src = gen_source(&d, "mpeg4", "mp4");
        let out = d.join("out.mp4");
        std::fs::write(&out, b"keep").unwrap();
        let e = run(&src, &[(3.5, 4.0)], &[0], &out, &[], &Progress::new()).unwrap_err();
        assert!(e.contains("no matching encoder"), "{e}");
        assert_eq!(std::fs::read(&out).unwrap(), b"keep");
        // a container it doesn't write
        let src = gen_source(&d, "libx264", "mp4");
        assert!(run(&src, &[(3.5, 4.0)], &[0], &d.join("out.webm"), &[], &Progress::new()).is_err());
        // a missing audio stream
        assert!(run(&src, &[(3.5, 4.0)], &[5], &out, &[], &Progress::new()).unwrap_err().contains("audio stream"));
        assert_eq!(std::fs::read(&out).unwrap(), b"keep");
        let left: Vec<_> = std::fs::read_dir(&d).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name()).collect();
        assert_eq!(left.len(), 2, "no temp files left behind: {left:?}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
