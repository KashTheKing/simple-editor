//! Export: render the timeline with the Compositor + Mixer and pipe it into ffmpeg.exe
//! (`-f rawvideo -pix_fmt rgba -s WxH -r fps -i pipe:0` + pre-mixed temp WAV), encoder chosen by the
//! output extension / settings. Plus the lossless `-c copy` fast path for pure cuts of one source.
//! Runs on a background thread; the UI polls `Progress`.

use crate::engine::compose::Compositor;
use crate::engine::mixer::Mixer;
use crate::engine::text::TextRasterizer;
use crate::media::{ffpipe, Backend, DecoderPool, Frame, SAMPLE_RATE};
use crate::model::{ClipKind, Project, Track};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct ExportOptions {
    pub out_path: PathBuf,
    /// "auto" or an ffmpeg encoder name (see Settings.encoder).
    pub encoder: String,
    pub crf: u32,
    pub preset: String,
    pub backend: Backend,
}

pub struct Progress {
    fraction: Mutex<f32>,
    status: Mutex<String>,
    error: Mutex<Option<String>>,
    pub cancel: AtomicBool,
    done: AtomicBool,
}

impl Progress {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            fraction: Mutex::new(0.0),
            status: Mutex::new(String::new()),
            error: Mutex::new(None),
            cancel: AtomicBool::new(false),
            done: AtomicBool::new(false),
        })
    }
    pub fn set(&self, fraction: f32, status: impl Into<String>) {
        *self.fraction.lock().unwrap() = fraction;
        *self.status.lock().unwrap() = status.into();
    }
    pub fn fraction(&self) -> f32 {
        *self.fraction.lock().unwrap()
    }
    pub fn status(&self) -> String {
        self.status.lock().unwrap().clone()
    }
    pub fn finish(&self, error: Option<String>) {
        *self.error.lock().unwrap() = error;
        self.done.store(true, Ordering::SeqCst);
    }
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }
}

const CANCELLED: &str = "cancelled";
/// Audio mix block (frames) — 100 ms.
const MIX_BLOCK: usize = 4800;

/// Start a full (re-encoding) export on a background thread. Frames are rendered at project size
/// with `Compositor` and audio mixed with `Mixer` into a temp WAV first (fast), then video frames are
/// piped to ffmpeg's stdin. Progress 0..1; cancel kills ffmpeg and removes the partial file.
/// Audio-only extensions (mp3, wav, m4a, flac, ogg, aac, opus) skip video (`-vn`); gif skips audio.
pub fn start_export(project: Project, opts: ExportOptions, text: Arc<Mutex<TextRasterizer>>) -> Arc<Progress> {
    spawn_job("export", move |prog| run_export(&project, &opts, &text, prog))
}

/// Run `job` on a named thread; any Err (or panic) lands in `Progress::finish`.
fn spawn_job(name: &str, job: impl FnOnce(&Progress) -> Result<(), String> + Send + 'static) -> Arc<Progress> {
    let prog = Progress::new();
    let p = prog.clone();
    let spawned = std::thread::Builder::new().name(name.into()).spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&p)));
        p.finish(match r {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e),
            Err(_) => Some(format!("{} thread panicked", std::thread::current().name().unwrap_or("job"))),
        });
    });
    if let Err(e) = spawned {
        prog.finish(Some(format!("could not start thread: {e}")));
    }
    prog
}

fn ext_of(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn run_export(
    project: &Project,
    opts: &ExportOptions,
    text: &Arc<Mutex<TextRasterizer>>,
    prog: &Progress,
) -> Result<(), String> {
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let ext = ext_of(&opts.out_path);
    let tmp = temp_output(&opts.out_path);
    let audio_only = AUDIO_EXTS.contains(&ext.as_str());
    let is_gif = ext == "gif";
    let dur = project.duration();
    if dur <= 0.0 {
        return Err("Project is empty".into());
    }
    let fps = project.fps.max(1.0);
    let (w, h) = (project.width.max(1), project.height.max(1));
    let mut pool = DecoderPool::new(opts.backend);

    // 1. audio → temp WAV
    let has_audio = !is_gif
        && project
            .audio_tracks()
            .into_iter()
            .any(|i| project.active(i) && project.tracks[i].clips.iter().any(|c| c.enabled));
    let wav = if has_audio { Some(mix_to_wav(project, &mut pool, prog, dur)?) } else { None };
    if audio_only && wav.is_none() {
        return Err("Nothing to export: no audio clips".into());
    }

    // 2. ffmpeg
    let mut cmd = ffpipe::command(&ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error"]);
    if !audio_only {
        cmd.args(["-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "-r", &format!("{fps}")]);
        cmd.args(["-i", "pipe:0"]);
    }
    if let Some(wav) = &wav {
        cmd.arg("-i").arg(&wav.0);
    }
    if !audio_only {
        cmd.args(["-map", "0:v"]);
        if wav.is_some() {
            cmd.args(["-map", "1:a"]);
        }
    }
    let args = codec_args(&ext, &opts.encoder, opts.crf, &opts.preset, &detect_encoders());
    if !audio_only && (w % 2 == 1 || h % 2 == 1) && args.iter().any(|a| a == "yuv420p" || a == "nv12") {
        cmd.args(["-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2"]);
    }
    cmd.args(&args);
    if is_gif {
        cmd.arg("-an");
    }
    // (no `-shortest`: it drops the last video frame when dur isn't a whole number of frames; video and
    // WAV both end at ≥ dur anyway)
    cmd.arg(&tmp.0);
    cmd.stdin(if audio_only { Stdio::null() } else { Stdio::piped() }).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
    let tail = stderr_tail(&mut child);

    // 3. frames
    let mut stdin = child.stdin.take();
    if let Some(pipe) = stdin.as_mut() {
        let n = (dur * fps).ceil().max(1.0) as u64;
        let mut frame = Frame::new(w, h);
        let mut comp = Compositor::new();
        for i in 0..n {
            if prog.is_cancelled() {
                break;
            }
            let t = i as f64 / fps;
            {
                let mut tr = text.lock().unwrap_or_else(|e| e.into_inner());
                comp.render(project, t, w, h, &mut pool, &mut tr, &mut frame);
            }
            if pipe.write_all(&frame.rgba).is_err() {
                break; // ffmpeg died — its stderr tells why
            }
            prog.set(0.1 + 0.9 * (i + 1) as f32 / n as f32, format!("Encoding {t:.1} / {dur:.1} s"));
        }
    } else {
        prog.set(0.5, "Encoding audio…");
    }
    drop(stdin);

    // 4. wait, then move the finished file into place (decoders released first so an in-place
    // export over the source can replace it)
    wait_ffmpeg(&mut child, tail, prog)?;
    drop(pool);
    tmp.commit(&opts.out_path)
}

/// Mix the whole timeline into a temp WAV (f32 stereo 48 kHz). Progress 0..0.1.
fn mix_to_wav(project: &Project, pool: &mut DecoderPool, prog: &Progress, dur: f64) -> Result<TempFile, String> {
    static N: AtomicU32 = AtomicU32::new(0);
    let tmp = TempFile(std::env::temp_dir().join(format!(
        "simple-editor-mix-{}-{}.wav",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )));
    let total = (dur * SAMPLE_RATE as f64).ceil() as u64;
    let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp.0).map_err(|e| format!("temp wav: {e}"))?);
    f.write_all(&wav_header(total)).map_err(|e| format!("temp wav: {e}"))?;
    let mut mixer = Mixer::new();
    let mut buf = vec![0f32; MIX_BLOCK * 2];
    let mut bytes = vec![0u8; MIX_BLOCK * 8];
    let mut done = 0u64;
    while done < total {
        if prog.is_cancelled() {
            return Err(CANCELLED.into());
        }
        let n = (total - done).min(MIX_BLOCK as u64) as usize;
        mixer.mix(project, done as f64 / SAMPLE_RATE as f64, pool, &mut buf[..n * 2]);
        for (b, s) in bytes.chunks_exact_mut(4).zip(&buf[..n * 2]) {
            b.copy_from_slice(&s.to_le_bytes());
        }
        f.write_all(&bytes[..n * 8]).map_err(|e| format!("temp wav: {e}"))?;
        done += n as u64;
        prog.set(0.1 * done as f32 / total.max(1) as f32, "Mixing audio…");
    }
    f.flush().map_err(|e| format!("temp wav: {e}"))?;
    drop(f);
    Ok(tmp)
}

/// 44-byte RIFF/WAVE header: IEEE float 32-bit, stereo, 48 kHz, `frames` sample frames.
fn wav_header(frames: u64) -> [u8; 44] {
    // ponytail: sizes saturate at 4 GB (~3 h); ffmpeg reads to EOF anyway — RF64 if that matters
    let data = (frames * 8).min(u32::MAX as u64 - 36) as u32;
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data).to_le_bytes());
    h[8..16].copy_from_slice(b"WAVEfmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&3u16.to_le_bytes()); // WAVE_FORMAT_IEEE_FLOAT
    h[22..24].copy_from_slice(&2u16.to_le_bytes());
    h[24..28].copy_from_slice(&SAMPLE_RATE.to_le_bytes());
    h[28..32].copy_from_slice(&(SAMPLE_RATE * 8).to_le_bytes());
    h[32..34].copy_from_slice(&8u16.to_le_bytes());
    h[34..36].copy_from_slice(&32u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data.to_le_bytes());
    h
}

/// Deleted on drop (a no-op once `commit`ted into place).
struct TempFile(PathBuf);
impl TempFile {
    /// Move the finished file over `dst` (replaces an existing file).
    fn commit(self, dst: &Path) -> Result<(), String> {
        std::fs::rename(&self.0, dst).map_err(|e| format!("rename to {}: {e}", dst.display()))
    }
}
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Hidden sibling of `out` (same folder, same extension so ffmpeg still picks the muxer by name) that
/// ffmpeg writes into; it is renamed over `out` only on success, so a cancel or failure never truncates
/// or deletes an existing destination — or the source, when cutting in place.
fn temp_output(out: &Path) -> TempFile {
    let stem = out.file_stem().unwrap_or_default().to_string_lossy();
    let ext = out.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    TempFile(out.with_file_name(format!(".{stem}.simple-editor-tmp{ext}")))
}

/// Drain ffmpeg's stderr on a helper thread; returns the last ~2000 chars.
fn stderr_tail(child: &mut Child) -> Option<JoinHandle<String>> {
    let mut err = child.stderr.take()?;
    Some(std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        let s = String::from_utf8_lossy(&buf);
        let s = s.trim();
        let cut = s.char_indices().rev().nth(1999).map(|(i, _)| i).unwrap_or(0);
        s[cut..].to_string()
    }))
}

/// Wait for ffmpeg, killing it if the job is cancelled. Non-zero exit → Err(stderr tail).
fn wait_ffmpeg(child: &mut Child, tail: Option<JoinHandle<String>>, prog: &Progress) -> Result<(), String> {
    let status = loop {
        if prog.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            if let Some(t) = tail {
                let _ = t.join();
            }
            return Err(CANCELLED.into());
        }
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => std::thread::sleep(Duration::from_millis(30)),
            Err(e) => return Err(format!("ffmpeg: {e}")),
        }
    };
    let msg = tail.and_then(|t| t.join().ok()).unwrap_or_default();
    if status.success() {
        Ok(())
    } else if msg.is_empty() {
        Err(format!("ffmpeg exited with {status}"))
    } else {
        Err(msg)
    }
}

/// If the project is a pure cut of exactly one video source (every clip from the same asset, audio clips
/// exactly mirroring their linked video clip's timing on each audio stream (muted tracks may be dropped),
/// no gaps between clips, no text/image clips, no effects, volume 1, all clips enabled), return the
/// segments as (src_in, duration) in timeline order. Such a project can be written with `-c copy`.
pub fn lossless_segments(project: &Project) -> Option<Vec<(f64, f64)>> {
    const EPS: f64 = 1e-4;
    let mut asset = None;
    for (_, c) in project.all_clips() {
        if c.kind != ClipKind::Video && c.kind != ClipKind::Audio {
            return None;
        }
        if !c.enabled || c.has_effects() || !c.volume.is_default(1.0) {
            return None;
        }
        if asset.is_some_and(|a| a != c.asset) {
            return None;
        }
        asset = Some(c.asset);
    }
    if project.asset(asset?)?.kind != ClipKind::Video {
        return None;
    }
    let mut video: Option<&Track> = None;
    for ti in project.video_tracks() {
        let t = &project.tracks[ti];
        if t.clips.is_empty() {
            continue;
        }
        if !project.active(ti) || video.is_some() {
            return None;
        }
        video = Some(t);
    }
    let video = video?;
    let mut segs = Vec::with_capacity(video.clips.len());
    let mut pos = 0.0;
    for c in &video.clips {
        if (c.start - pos).abs() > EPS {
            return None;
        }
        segs.push((c.src_in, c.duration));
        pos = c.end();
    }
    for ti in project.audio_tracks() {
        let t = &project.tracks[ti];
        if t.clips.is_empty() || !project.active(ti) {
            continue;
        }
        if t.clips.len() != video.clips.len() {
            return None;
        }
        for (a, v) in t.clips.iter().zip(&video.clips) {
            if (a.start - v.start).abs() > EPS
                || (a.duration - v.duration).abs() > EPS
                || (a.src_in - v.src_in).abs() > EPS
            {
                return None;
            }
        }
    }
    Some(segs)
}

/// Lossless cut: `ffmpeg -ss in -t dur -i src -c copy -avoid_negative_ts make_zero` per segment (with
/// `-map` dropping muted audio streams), then concat demuxer when there is more than one segment.
/// Cuts land on keyframes (not frame-accurate) — that's the trade for being instant.
pub fn start_lossless_cut(project: Project, out_path: PathBuf) -> Arc<Progress> {
    spawn_job("lossless-cut", move |prog| run_lossless(&project, &out_path, prog))
}

fn run_lossless(project: &Project, out: &Path, prog: &Progress) -> Result<(), String> {
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let segs = lossless_segments(project).ok_or("Project is not a plain cut of one video")?;
    let (_, first) = project.all_clips().next().ok_or("Project is empty")?;
    let src = project.asset(first.asset).ok_or("Missing asset")?.path.clone();
    let mut streams: Vec<usize> = project
        .audio_tracks()
        .into_iter()
        .filter(|&i| project.active(i))
        .flat_map(|i| project.tracks[i].clips.iter().map(|c| c.audio_stream))
        .collect();
    streams.sort_unstable();
    streams.dedup();

    let ext = ext_of(out);
    let stem = out.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let tmp = temp_output(out);
    let n = segs.len();
    let mut parts: Vec<TempFile> = Vec::new();
    let run = |cmd: &mut std::process::Command| -> Result<(), String> {
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
        let tail = stderr_tail(&mut child);
        wait_ffmpeg(&mut child, tail, prog)
    };
    for (i, &(src_in, dur)) in segs.iter().enumerate() {
        prog.set(i as f32 / n as f32, format!("Cutting segment {} / {n}", i + 1));
        let dst = if n == 1 { tmp.0.clone() } else { out.with_file_name(format!("{stem}.part{i}.{ext}")) };
        let mut cmd = ffpipe::command(&ffmpeg);
        cmd.args(["-y", "-hide_banner", "-loglevel", "error"]);
        cmd.args(["-ss", &format!("{src_in:.6}"), "-t", &format!("{dur:.6}"), "-i", &src]);
        cmd.args(["-map", "0:v:0"]);
        for s in &streams {
            cmd.args(["-map", &format!("0:a:{s}")]);
        }
        if streams.is_empty() {
            cmd.arg("-an");
        }
        cmd.args(["-c", "copy", "-avoid_negative_ts", "make_zero"]).arg(&dst);
        if n > 1 {
            parts.push(TempFile(dst));
        }
        run(&mut cmd)?;
    }
    if n > 1 {
        prog.set(0.95, "Joining segments…");
        let list = TempFile(out.with_file_name(format!("{stem}.concat.txt")));
        let mut txt = String::new();
        for p in &parts {
            let s = p.0.to_string_lossy().replace('\\', "/").replace('\'', "'\\''");
            txt.push_str(&format!("file '{s}'\n"));
        }
        std::fs::write(&list.0, txt).map_err(|e| format!("concat list: {e}"))?;
        let mut cmd = ffpipe::command(&ffmpeg);
        cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i"]);
        cmd.arg(&list.0).args(["-map", "0", "-c", "copy"]).arg(&tmp.0);
        run(&mut cmd)?;
    }
    tmp.commit(out)
}

static ENCODERS: Mutex<Option<(PathBuf, Vec<String>)>> = Mutex::new(None);

/// Encoder names ffmpeg reports (`ffmpeg -hide_banner -encoders`), cached after the first call.
/// Empty if ffmpeg is missing.
pub fn detect_encoders() -> Vec<String> {
    let Some(exe) = ffpipe::ffmpeg_exe() else {
        return Vec::new();
    };
    let mut cache = ENCODERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((p, v)) = &*cache {
        if *p == exe {
            return v.clone();
        }
    }
    let out = ffpipe::command(&exe).args(["-hide_banner", "-encoders"]).stdin(Stdio::null()).output();
    let names = out.map(|o| parse_encoders(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default();
    *cache = Some((exe, names.clone()));
    names
}

fn parse_encoders(s: &str) -> Vec<String> {
    s.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let flags = it.next()?;
            let name = it.next()?;
            (flags.len() == 6 && (flags.starts_with('V') || flags.starts_with('A')) && name != "=")
                .then(|| name.to_string())
        })
        .collect()
}

/// ffmpeg output arguments (codec/quality) for an extension, given the user's encoder preference and the
/// available encoders. "auto": mp4/mov/mkv/m4v → libx264 (+aac), webm → libvpx-vp9 (+libopus),
/// gif → gif, avi → mpeg4 (+mp3), audio-only → sensible codec for the container. The preference is
/// overridden where the container forbids it (webm: vp9/av1 only; mov: no vp9/av1).
/// Hardware encoders (nvenc/qsv/amf) use `-cq`/`-global_quality`/`-qp` instead of `-crf`.
pub fn codec_args(ext: &str, encoder: &str, crf: u32, preset: &str, available: &[String]) -> Vec<String> {
    let ext = ext.to_ascii_lowercase();
    let has = |n: &str| available.is_empty() || available.iter().any(|a| a == n);
    let audio = match ext.as_str() {
        "webm" | "ogg" | "opus" => {
            if has("libopus") {
                "-c:a libopus -b:a 160k"
            } else {
                "-c:a libvorbis -q:a 5"
            }
        }
        "avi" | "mp3" => "-c:a libmp3lame -q:a 2",
        "mpg" | "mpeg" | "vob" => "-c:a mp2 -b:a 192k",
        "wav" => "-c:a pcm_s16le",
        "flac" => "-c:a flac",
        _ => "-c:a aac -b:a 192k",
    };
    let mut video = String::new();
    if ext == "gif" {
        video.push_str("-c:v gif");
    } else if !AUDIO_EXTS.contains(&ext.as_str()) {
        let mut enc = match encoder {
            "auto" | "" => match ext.as_str() {
                "webm" => "libvpx-vp9",
                "avi" if !has("libx264") => "mpeg4",
                _ => "libx264",
            },
            e => e,
        };
        let vpx_av1 = matches!(enc, "libvpx-vp9" | "libaom-av1" | "libsvtav1");
        if ext == "webm" && !vpx_av1 {
            enc = "libvpx-vp9"; // webm muxer: only vp8/vp9/av1
        } else if ext == "mov" && vpx_av1 {
            enc = "libx264"; // mov muxer rejects vp9/av1
        }
        if !has(enc) {
            enc = "libx264";
        }
        let preset = if preset.is_empty() { "medium" } else { preset };
        video = match enc {
            "libx264" | "libx265" => {
                format!("-c:v {enc} -preset {preset} -crf {crf} -pix_fmt yuv420p")
            }
            "h264_nvenc" | "hevc_nvenc" => {
                format!("-c:v {enc} -preset p4 -cq {crf} -b:v 0 -pix_fmt yuv420p")
            }
            "h264_qsv" | "hevc_qsv" => format!("-c:v {enc} -global_quality {crf} -pix_fmt nv12"),
            "h264_amf" | "hevc_amf" => format!("-c:v {enc} -rc cqp -qp_i {crf} -qp_p {crf}"),
            "libvpx-vp9" => {
                format!("-c:v libvpx-vp9 -crf {crf} -b:v 0 -row-mt 1 -cpu-used 2 -pix_fmt yuv420p")
            }
            "libaom-av1" | "libsvtav1" => format!("-c:v {enc} -crf {crf} -pix_fmt yuv420p"),
            "mpeg4" => "-c:v mpeg4 -q:v 4 -pix_fmt yuv420p".into(),
            other => format!("-c:v {other}"),
        };
        if matches!(ext.as_str(), "mp4" | "mov" | "m4v") {
            video.push_str(" -movflags +faststart");
        }
    }
    let mut v: Vec<String> = video.split(' ').filter(|s| !s.is_empty()).map(String::from).collect();
    if ext != "gif" {
        v.extend(audio.split(' ').map(String::from));
    }
    v
}

pub const AUDIO_EXTS: &[&str] = &["mp3", "wav", "m4a", "flac", "ogg", "aac", "opus"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, AudioStreamInfo, TrackKind};

    fn asset(path: &str, streams: usize) -> Asset {
        Asset {
            id: 0,
            path: path.into(),
            kind: ClipKind::Video,
            duration: 4.0,
            width: 320,
            height: 240,
            fps: 30.0,
            audio_streams: (0..streams)
                .map(|i| AudioStreamInfo { index: i, channels: 2, sample_rate: 48000, ..Default::default() })
                .collect(),
            codec: "h264".into(),
        }
    }

    /// from_media(4 s, 2 streams), split at 1 and 3, delete the middle → [0,1) + [3,4).
    fn cut_project(path: &str) -> Project {
        let mut p = Project::from_media(asset(path, 2));
        p.split_at(1.0, None);
        p.split_at(3.0, None);
        let mid = p.tracks[0].clips[1].id;
        let ids = p.linked(mid);
        p.delete_clips(&ids, true);
        p
    }

    fn wait_done(prog: &Progress) -> Option<String> {
        for _ in 0..1200 {
            if prog.is_done() {
                return prog.error();
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Some("timeout".into())
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("simple-editor-export-test-{}-{name}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    /// red 0–2 s, green 2–4 s, two sine streams, keyframe every second. None if ffmpeg is missing.
    fn gen_media(dir: &Path) -> Option<PathBuf> {
        let exe = ffpipe::ffmpeg_exe()?;
        let out = dir.join("test.mp4");
        let st = ffpipe::command(&exe)
            .args(["-y", "-hide_banner", "-loglevel", "error"])
            .args(["-f", "lavfi", "-i", "color=red:s=320x240:d=2", "-f", "lavfi", "-i", "color=lime:s=320x240:d=2"])
            .args([
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=4",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=880:duration=4",
            ])
            .args(["-filter_complex", "[0:v][1:v]concat=n=2:v=1[v]", "-map", "[v]", "-map", "2:a", "-map", "3:a"])
            .args(["-r", "30", "-g", "30", "-pix_fmt", "yuv420p", "-c:v", "libx264", "-c:a", "aac"])
            .args(["-metadata:s:a:0", "language=eng", "-metadata:s:a:1", "title=Music"])
            .arg(&out)
            .status()
            .expect("run ffmpeg");
        assert!(st.success(), "ffmpeg could not generate test media");
        Some(out)
    }

    fn probe_duration(path: &Path) -> Option<f64> {
        let o = ffpipe::command(&ffpipe::ffprobe_exe()?)
            .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
            .arg(path)
            .output()
            .ok()?;
        String::from_utf8_lossy(&o.stdout).trim().parse().ok()
    }

    #[test]
    fn codec_args_by_ext() {
        let j = |v: Vec<String>| v.join(" ");
        let s = j(codec_args("mp4", "auto", 18, "veryfast", &[]));
        assert!(s.contains("-c:v libx264 -preset veryfast -crf 18 -pix_fmt yuv420p"), "{s}");
        assert!(s.contains("-movflags +faststart") && s.contains("-c:a aac"), "{s}");
        let s = j(codec_args("webm", "auto", 30, "veryfast", &[]));
        assert!(s.contains("libvpx-vp9") && s.contains("libopus") && !s.contains("faststart"), "{s}");
        let s = j(codec_args("mp3", "auto", 18, "", &[]));
        assert_eq!(s, "-c:a libmp3lame -q:a 2");
        assert_eq!(j(codec_args("wav", "auto", 18, "", &[])), "-c:a pcm_s16le");
        assert_eq!(j(codec_args("gif", "auto", 18, "", &[])), "-c:v gif");
        let s = j(codec_args("mkv", "hevc_nvenc", 20, "slow", &["hevc_nvenc".into(), "libx264".into()]));
        assert!(s.contains("-c:v hevc_nvenc -preset p4 -cq 20 -b:v 0"), "{s}");
        // requested encoder unavailable → libx264
        let s = j(codec_args("mp4", "h264_nvenc", 20, "slow", &["libx264".into()]));
        assert!(s.contains("-c:v libx264 -preset slow -crf 20"), "{s}");
        let s = j(codec_args("mov", "h264_qsv", 22, "", &[]));
        assert!(s.contains("-global_quality 22 -pix_fmt nv12"), "{s}");
        let s = j(codec_args("avi", "auto", 22, "", &["mpeg4".into()]));
        assert!(s.contains("-c:v mpeg4") && s.contains("libmp3lame"), "{s}");
        let s = j(codec_args("mp4", "h264_amf", 22, "", &[]));
        assert!(s.contains("-rc cqp -qp_i 22 -qp_p 22"), "{s}");
        // container forbids the preferred encoder → clamped
        assert!(j(codec_args("webm", "libx264", 30, "", &[])).contains("-c:v libvpx-vp9"));
        assert!(j(codec_args("webm", "hevc_nvenc", 30, "", &[])).contains("-c:v libvpx-vp9"));
        assert!(j(codec_args("mov", "libvpx-vp9", 20, "", &[])).contains("-c:v libx264"));
        assert!(j(codec_args("mkv", "libvpx-vp9", 20, "", &[])).contains("-c:v libvpx-vp9"));
        assert!(j(codec_args("mpg", "auto", 20, "", &[])).ends_with("-c:a mp2 -b:a 192k"));
    }

    #[test]
    fn parse_encoder_list() {
        let s = " V..... = Video\n A..... = Audio\n ------\n V....D libx264  H.264\n A....D aac  AAC\n S..... srt  SubRip\n";
        assert_eq!(parse_encoders(s), vec!["libx264".to_string(), "aac".to_string()]);
    }

    #[test]
    fn lossless_segments_rules() {
        let p = cut_project("C:/fake/v.mp4");
        let segs = lossless_segments(&p).expect("pure cut");
        assert_eq!(segs.len(), 2);
        assert!((segs[0].0).abs() < 1e-9 && (segs[0].1 - 1.0).abs() < 1e-9);
        assert!((segs[1].0 - 3.0).abs() < 1e-9 && (segs[1].1 - 1.0).abs() < 1e-9);

        // text clip → None
        let mut q = p.clone();
        q.add_text_clip(0.5, 1.0);
        assert!(lossless_segments(&q).is_none());
        // effect → None
        let mut q = p.clone();
        q.tracks[0].clips[0].opacity.value = 0.5;
        assert!(lossless_segments(&q).is_none());
        // volume → None
        let mut q = p.clone();
        q.tracks[1].clips[0].volume.value = 0.5;
        assert!(lossless_segments(&q).is_none());
        // disabled clip → None
        let mut q = p.clone();
        q.tracks[0].clips[1].enabled = false;
        assert!(lossless_segments(&q).is_none());
        // gap → None
        let mut q = p.clone();
        q.tracks[0].clips[1].start += 0.5;
        assert!(lossless_segments(&q).is_none());
        // audio not mirroring → None; but muted track with odd clips → Some
        let mut q = p.clone();
        q.tracks[2].clips[1].start += 0.2;
        q.tracks[2].clips[1].duration -= 0.2;
        assert!(lossless_segments(&q).is_none());
        q.tracks[2].muted = true;
        assert_eq!(lossless_segments(&q).map(|s| s.len()), Some(2));
        // second asset → None
        let mut q = p.clone();
        let id = q.add_asset(asset("C:/fake/other.mp4", 0));
        q.insert_asset_clips(id, 4.0, Some(0));
        assert!(lossless_segments(&q).is_none());
        // empty project → None
        assert!(lossless_segments(&Project::new()).is_none());
        // clip on a second video track → None
        let mut q = p.clone();
        q.add_track(TrackKind::Video);
        let c = q.tracks[0].clips[0].clone();
        q.tracks[1].clips.push(c);
        assert!(lossless_segments(&q).is_none());
    }

    #[test]
    fn wav_header_is_readable() {
        if ffpipe::ffprobe_exe().is_none() {
            eprintln!("ffprobe missing — skipped");
            return;
        }
        let dir = temp_dir("wav");
        let path = dir.join("hdr.wav");
        let mut bytes = wav_header(SAMPLE_RATE as u64).to_vec();
        bytes.resize(44 + SAMPLE_RATE as usize * 8, 0);
        std::fs::write(&path, bytes).unwrap();
        let d = probe_duration(&path).expect("ffprobe reads header");
        assert!((d - 1.0).abs() < 0.01, "{d}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_encoders_has_x264() {
        if ffpipe::ffmpeg_exe().is_none() {
            eprintln!("ffmpeg missing — skipped");
            return;
        }
        let e = detect_encoders();
        assert!(e.iter().any(|x| x == "libx264"), "{e:?}");
        assert!(e.iter().any(|x| x == "aac"));
        assert_eq!(detect_encoders().len(), e.len()); // cached
    }

    #[test]
    fn lossless_cut_real() {
        let dir = temp_dir("cut");
        let Some(src) = gen_media(&dir) else {
            eprintln!("ffmpeg missing — skipped");
            return;
        };
        let p = cut_project(&src.to_string_lossy());
        let out = dir.join("cut.mp4");
        let prog = start_lossless_cut(p, out.clone());
        assert_eq!(wait_done(&prog), None);
        assert!(out.exists());
        let d = probe_duration(&out).expect("probe");
        assert!((d - 2.0).abs() < 0.6, "duration {d}");
        assert!(!dir.join("cut.part0.mp4").exists() && !dir.join("cut.concat.txt").exists());
        // muted A2 → only one audio stream in the output
        let mut p = cut_project(&src.to_string_lossy());
        p.tracks[2].muted = true;
        let out1 = dir.join("cut1.mp4");
        assert_eq!(wait_done(&start_lossless_cut(p, out1.clone())), None);
        let o = ffpipe::command(&ffpipe::ffprobe_exe().unwrap())
            .args(["-v", "error", "-show_entries", "stream=codec_type", "-of", "csv=p=0"])
            .arg(&out1)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&o.stdout).lines().count(), 2);
        // a failed cut never removes a pre-existing destination (wrong container → ffmpeg rejects)
        let gif = dir.join("keep.gif");
        std::fs::write(&gif, b"keep").unwrap();
        assert!(wait_done(&start_lossless_cut(cut_project(&src.to_string_lossy()), gif.clone())).is_some());
        assert_eq!(std::fs::read(&gif).unwrap(), b"keep");
        // cutting in place (destination == source) replaces the source with the cut, never loses it
        let p = cut_project(&src.to_string_lossy());
        assert_eq!(wait_done(&start_lossless_cut(p, src.clone())), None);
        let d = probe_duration(&src).expect("source still readable");
        assert!((d - 2.0).abs() < 0.6, "duration {d}");
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            4,
            "{:?}",
            std::fs::read_dir(&dir).unwrap().collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn full_export_real() {
        let dir = temp_dir("export");
        let Some(src) = gen_media(&dir) else {
            eprintln!("ffmpeg missing — skipped");
            return;
        };
        let p = Project::from_media(asset(&src.to_string_lossy(), 2));
        let out = dir.join("export.mp4");
        let opts = ExportOptions {
            out_path: out.clone(),
            encoder: "auto".into(),
            crf: 23,
            preset: "ultrafast".into(),
            backend: Backend::Auto,
        };
        let text = Arc::new(Mutex::new(TextRasterizer::new()));
        let prog = start_export(p.clone(), opts.clone(), text.clone());
        assert_eq!(wait_done(&prog), None);
        let d = probe_duration(&out).expect("probe");
        assert!((d - 4.0).abs() < 0.2, "duration {d}");
        // audio-only + cancel paths
        let mp3 = ExportOptions { out_path: dir.join("export.mp3"), ..opts.clone() };
        assert_eq!(wait_done(&start_export(p.clone(), mp3, text.clone())), None);
        // cancel: a pre-existing destination is left untouched, no temp file remains
        let cancel = dir.join("cancel.mp4");
        std::fs::write(&cancel, b"keep").unwrap();
        let prog = start_export(p, ExportOptions { out_path: cancel.clone(), ..opts }, text);
        prog.cancel.store(true, Ordering::SeqCst);
        assert_eq!(wait_done(&prog).as_deref(), Some(CANCELLED));
        assert_eq!(std::fs::read(&cancel).unwrap(), b"keep");
        assert!(!dir.join(".cancel.simple-editor-tmp.mp4").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
