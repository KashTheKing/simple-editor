//! Export: render the timeline with the CPU Compositor + Mixer and pipe it into ffmpeg.exe
//! (`-f rawvideo -pix_fmt rgba|nv12 -s WxH -r fps -i pipe:0` + one headerless `f32le` audio input per
//! `AudioInput`, mixed on its own thread and streamed over a loopback socket while the video renders),
//! encoder chosen by the output extension / settings. Plus the lossless `-c copy` fast path for pure
//! cuts of one source. Runs on a background thread; the UI polls `Progress`.

use crate::engine::compose::Compositor;
use crate::engine::mixer::Mixer;
use crate::engine::text::TextRasterizer;
use crate::media::{ffpipe, Backend, DecoderPool, Frame, SAMPLE_RATE};
use crate::model::{ClipKind, Project, Track};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// One frame the export thread wants composited on the GPU. The UI thread owns the GL context, so it
/// picks these up in `App::update`, renders `layers` and sends the pixels back.
pub struct GpuFrameRequest {
    pub layers: crate::engine::gpu::LayerSet,
    pub t: f64,
    pub w: u32,
    pub h: u32,
    /// ws:export-pipeline: read back as NV12 bytes (`w*h*3/2` in `rgba`, BT.709 limited) instead of RGBA.
    pub nv12: bool,
    /// The buffer to read into and reply with (recycled by the sender; `Frame::default()` is fine).
    pub buf: Frame,
    /// The rendered frame, or None / a dropped sender when the renderer died (the export falls back
    /// to the CPU compositor).
    pub reply: std::sync::mpsc::SyncSender<Option<Frame>>,
}

/// Where an export gets its pictures.
#[derive(Clone)]
pub enum FrameSource {
    /// Composite on this thread with `engine::compose` (no GPU-only effects, no node graphs).
    Cpu,
    /// Decode here, composite on the UI thread's GL context - identical to what the preview shows.
    Gpu(std::sync::mpsc::Sender<GpuFrameRequest>),
}

impl std::fmt::Debug for FrameSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            FrameSource::Cpu => "Cpu",
            FrameSource::Gpu(_) => "Gpu",
        })
    }
}

#[derive(Clone, Debug)]
pub struct ExportOptions {
    pub out_path: PathBuf,
    /// "auto" or an ffmpeg encoder name (see Settings.encoder).
    pub encoder: String,
    pub crf: u32,
    pub preset: String,
    pub backend: Backend,
    /// Output size when it differs from the project (frames are rendered at project size and scaled by
    /// ffmpeg `-vf scale=W:H:flags=<scaler>`); None = project size.
    pub out_size: Option<(u32, u32)>,
    /// ffmpeg scale flags: "neighbor" | "bilinear" | "bicubic" | "lanczos" | "area" | "spline".
    pub scaler: String,
    /// CPU compositor, or the UI thread's GPU renderer (so exports match the preview exactly).
    pub frames: FrameSource,
    /// Container metadata to write, as `(key, value)`. Empty = strip everything (`-map_metadata -1`),
    /// which is the default: exports carry no title/encoder/creation-time unless asked for.
    pub metadata: Vec<(String, String)>,
    // ---- ws:export-deliver ----
    /// Timeline seconds `[a, b)` to render instead of the whole project (the Export window's
    /// "Export In/Out Range" box, `export.queue`'s range_in/range_out). None = everything.
    pub range: Option<(f64, f64)>,
    /// Append `-af loudnorm` (EBU R128, −14 LUFS / −1 dBTP - what YouTube/Spotify normalise to) when
    /// the export carries audio.
    pub loudnorm: bool,
    /// Fit-and-pad (pillar/letterbox with black bars) instead of stretching when `out_size`'s aspect
    /// differs from the project's. Only the platform-preset tiles set this; a hand-typed custom size
    /// keeps the plain stretch it always had.
    pub letterbox: bool,
    // ---- ws:pro-schema ----
    /// How the audio leaves the export: one mix, one stream per stem, or separate stem files.
    pub audio: AudioLayout,
    /// What a "stem" is when `audio` isn't `Mix`.
    pub stem_by: StemBy,
}

// ---- ws:pro-schema ----
/// `ExportOptions.audio` (ws:export-audio-tracks consumes it; `Settings.export_audio` is its string).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioLayout {
    /// One stereo mix - what every export did before.
    #[default]
    Mix,
    /// One audio stream per stem inside the same file.
    Streams,
    /// One separate audio file per stem next to the video.
    Stems,
}

/// `ExportOptions.stem_by`: how clips are grouped into stems.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StemBy {
    /// One stem per audio track.
    #[default]
    Track,
    /// One stem per mixer bus.
    Bus,
    /// One stem per clip role (Dialogue / Music / Sfx / Ambience).
    Role,
}

// ---- ws:export-audio-tracks ----
impl AudioLayout {
    /// From `Settings.export_audio` / the `audio` tool arg: "streams", "stems", anything else = Mix.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "streams" => AudioLayout::Streams,
            "stems" => AudioLayout::Stems,
            _ => AudioLayout::Mix,
        }
    }
}

impl StemBy {
    /// From `Settings.export_stem_by` / the `stem_by` tool arg: "bus", "role", anything else = Track.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "bus" => StemBy::Bus,
            "role" => StemBy::Role,
            _ => StemBy::Track,
        }
    }
}

/// Containers that can carry several audio streams (`AudioLayout::Streams`). ponytail: webm/ogg
/// (Matroska/Ogg can) left out until verified with the opus/vorbis encoders.
pub const MULTI_AUDIO_EXTS: &[&str] = &["mp4", "mov", "mkv", "m4v"];

/// The layout an export to `ext` really writes: Streams falls back to Mix where the container can't
/// hold several audio streams.
pub fn effective_layout(layout: AudioLayout, ext: &str) -> AudioLayout {
    if layout == AudioLayout::Streams && !MULTI_AUDIO_EXTS.contains(&ext) {
        AudioLayout::Mix
    } else {
        layout
    }
}

/// `<output stem> stems` beside `out`: where `AudioLayout::Stems` puts its WAVs.
pub fn stems_dir(out: &Path) -> PathBuf {
    let stem = out.file_stem().unwrap_or_default().to_string_lossy();
    out.with_file_name(format!("{stem} stems"))
}

/// Main-output audio options: loudnorm (only on the mix stream, `-filter:a:0`, when the file carries
/// stems too), and for Streams each stream's title with the mix as the only default track.
fn audio_out_args(af: &str, layout: AudioLayout, titles: &[&str]) -> Vec<String> {
    let streams = layout == AudioLayout::Streams;
    let mut a: Vec<String> = Vec::new();
    if !af.is_empty() {
        // loudnorm resamples to 192 kHz internally; without -ar a WAV/FLAC would keep that rate
        let flag = if streams { "-filter:a:0" } else { "-af" };
        a.extend([flag.to_string(), af.to_string(), "-ar".into(), "48000".into()]);
    }
    if streams {
        for (i, t) in titles.iter().enumerate() {
            a.extend([
                format!("-metadata:s:a:{i}"),
                format!("title={t}"),
                format!("-disposition:a:{i}"),
                (if i == 0 { "default" } else { "0" }).to_string(),
            ]);
        }
    }
    a
}

/// One extra ffmpeg output per stem (Stems layout): input `first_stem + i` → `dir/NN <title>.wav`,
/// 24-bit 48 kHz, no metadata.
fn stem_outputs(first_stem: usize, titles: &[&str], dir: &Path) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    for (i, t) in titles.iter().enumerate() {
        a.extend([
            "-map".to_string(),
            format!("{}:a", first_stem + i),
            "-c:a".into(),
            "pcm_s24le".into(),
            "-ar".into(),
            "48000".into(),
            "-map_metadata".into(),
            "-1".into(),
            dir.join(crate::engine::stems::file_name(i, t)).to_string_lossy().into_owned(),
        ]);
    }
    a
}

/// A folder deleted (with its contents) on drop, unless its files were `commit`ted elsewhere first.
struct TempDir(PathBuf);
impl TempDir {
    /// Move every file into `dst` (created if missing; same-named files replaced, others kept).
    fn commit(&self, dst: &Path) -> Result<(), String> {
        let err = |e: std::io::Error| format!("stems to {}: {e}", dst.display());
        std::fs::create_dir_all(dst).map_err(err)?;
        for e in std::fs::read_dir(&self.0).map_err(err)? {
            let e = e.map_err(err)?;
            std::fs::rename(e.path(), dst.join(e.file_name())).map_err(err)?;
        }
        Ok(())
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---- ws:export-deliver ----
/// One platform export tile ("YouTube 1080p", "Shorts 9:16", …): a target size + container +
/// quality. Stored in `Settings.export_presets` (a plain Vec, so users can edit/remove tiles).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExportPreset {
    pub name: String,
    pub ext: String,
    pub width: u32,
    pub height: u32,
    pub crf: u32,
    pub loudnorm: bool,
}

/// The 4 shipped tiles - also the serde field default for `Settings.export_presets`, so a
/// settings.json from before this field existed backfills the same tiles a fresh install gets.
pub fn default_export_presets() -> Vec<ExportPreset> {
    let p = |name: &str, width, height, crf| ExportPreset {
        name: name.into(),
        ext: "mp4".into(),
        width,
        height,
        crf,
        loudnorm: true,
    };
    vec![
        p("YouTube 1080p", 1920, 1080, 18),
        p("YouTube 4K", 3840, 2160, 18),
        p("Shorts / Reels / TikTok", 1080, 1920, 20),
        p("Instagram 1:1", 1080, 1080, 20),
    ]
}

/// ffmpeg's EBU R128 normaliser at the streaming-platform target. Fixed on purpose (see the issue's
/// deliberate simplifications): a user-tunable target is a Settings field away if ever asked for.
pub const LOUDNORM: &str = "loudnorm=I=-14:TP=-1:LRA=11";

/// The `-vf` scale chain for an export of `src` px rendered to `out` px: the plain
/// `scale=W:H:flags=<scaler>` every export used before, or - when `letterbox` and the aspects differ -
/// fit inside then pad with black bars so a 16:9 project lands in a 9:16 Shorts frame un-stretched.
/// Empty when `out == src` (nothing to scale).
pub fn scale_vf(src: (u32, u32), out: (u32, u32), scaler: &str, letterbox: bool) -> String {
    if out == src {
        return String::new();
    }
    let (ow, oh) = out;
    let flags = if scaler.is_empty() { "bicubic" } else { scaler };
    let same_aspect = (src.0 as u64 * oh as u64) == (src.1 as u64 * ow as u64);
    if letterbox && !same_aspect {
        format!(
            "scale={ow}:{oh}:force_original_aspect_ratio=decrease:flags={flags},pad={ow}:{oh}:(ow-iw)/2:(oh-ih)/2:color=black"
        )
    } else {
        format!("scale={ow}:{oh}:flags={flags}")
    }
}

/// The `(-vf, -af)` filter strings for an export's ffmpeg command (empty = omit the flag). Pure so
/// the loudnorm / letterbox / even-size rules are testable without spawning ffmpeg. `audio` = the
/// export carries audio; `args` = the codec args (`pix_fmt` decides the even-size pad).
fn output_filters(opts: &ExportOptions, project: (u32, u32), audio: bool, args: &[String]) -> (String, String) {
    let scale = opts.out_size.filter(|&s| s != project);
    let mut vf = match scale {
        Some(out) => scale_vf(project, out, &opts.scaler, opts.letterbox),
        None => String::new(),
    };
    let (fw, fh) = scale.unwrap_or(project);
    if (fw % 2 == 1 || fh % 2 == 1) && args.iter().any(|a| a == "yuv420p" || a == "nv12") {
        if !vf.is_empty() {
            vf.push(',');
        }
        vf.push_str("pad=ceil(iw/2)*2:ceil(ih/2)*2");
    }
    let af = if opts.loudnorm && audio { LOUDNORM.to_string() } else { String::new() };
    (vf, af)
}

/// `(start, duration, frame count)` an export renders: the whole project, or `range` clamped into it.
/// Err when nothing is left to render.
fn frame_plan(project_dur: f64, fps: f64, range: Option<(f64, f64)>) -> Result<(f64, f64, u64), String> {
    if project_dur <= 0.0 {
        return Err("Project is empty".into());
    }
    let (start, end) = match range {
        Some((a, b)) => (a.max(0.0).min(project_dur), b.min(project_dur)),
        None => (0.0, project_dur),
    };
    let dur = end - start;
    if dur <= 0.0 {
        return Err("Export range is empty".into());
    }
    Ok((start, dur, (dur * fps).ceil().max(1.0) as u64))
}

pub struct Progress {
    fraction: Mutex<f32>,
    status: Mutex<String>,
    error: Mutex<Option<String>>,
    pub cancel: AtomicBool,
    done: AtomicBool,
    // ---- ws:export-deliver ----
    started: std::time::Instant,
}

impl Progress {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            fraction: Mutex::new(0.0),
            status: Mutex::new(String::new()),
            error: Mutex::new(None),
            cancel: AtomicBool::new(false),
            done: AtomicBool::new(false),
            started: std::time::Instant::now(),
        })
    }
    // ---- ws:export-deliver ----
    /// Time since the job started.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
    /// Remaining time, extrapolated linearly from the fraction done; None below 1 % (nothing to
    /// extrapolate from yet) or once finished.
    pub fn eta(&self) -> Option<Duration> {
        if self.is_done() {
            return None;
        }
        eta_from(self.elapsed(), self.fraction())
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

// ---- ws:export-deliver ----
/// `elapsed * (1/fraction - 1)`: the pure half of `Progress::eta`, so the curve is testable on a fixed
/// clock. None below 1 %.
pub fn eta_from(elapsed: Duration, fraction: f32) -> Option<Duration> {
    if !(fraction >= 0.01) || fraction >= 1.0 {
        return None;
    }
    Some(elapsed.mul_f64((1.0 / fraction as f64 - 1.0).max(0.0)))
}

pub(crate) const CANCELLED: &str = "cancelled";
/// Audio mix block (frames). Same as `playback::BLOCK` on purpose: `mixer::resample_add` samples clip
/// gains at the block ends and lerps between them, so a longer block here would smear fades, volume
/// keyframes and audio crossfades shorter than the block - export must ramp like playback does.
const MIX_BLOCK: usize = 1024;

/// Start a full (re-encoding) export on a background thread. Frames are rendered at project size
/// (`Compositor`, or the UI thread's GPU) and piped to ffmpeg's stdin by a writer thread, while audio
/// is mixed with `Mixer` on its own thread and streamed alongside. Progress 0..1 follows the video;
/// cancel kills ffmpeg and removes the partial file.
/// Audio-only extensions (mp3, wav, m4a, flac, ogg, aac, opus) skip video (`-vn`); gif skips audio.
pub fn start_export(project: Project, opts: ExportOptions, text: Arc<Mutex<TextRasterizer>>) -> Arc<Progress> {
    spawn_job("export", move |prog| run_export(&project, &opts, &text, prog))
}

/// Run `job` on a named thread; any Err (or panic) lands in `Progress::finish`.
pub(crate) fn spawn_job(
    name: &str,
    job: impl FnOnce(&Progress) -> Result<(), String> + Send + 'static,
) -> Arc<Progress> {
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

pub(crate) fn ext_of(path: &Path) -> String {
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
    let fps = project.fps.max(1.0);
    // ws:export-deliver: a range export renders [start, start+dur) instead of the whole timeline
    let (start, dur, n) = frame_plan(project.duration(), fps, opts.range)?;
    let (w, h) = (project.width.max(1), project.height.max(1));
    let mut pool = DecoderPool::new(opts.backend);
    let backend = opts.backend;

    // 1. audio inputs: each is mixed on its own thread while the video renders (step 4)
    // ws:export-audio-tracks: Streams needs a container with room for several audio streams
    let layout = effective_layout(opts.audio, &ext);
    let audio_note = if layout != opts.audio {
        " · separate audio tracks need mp4/mov/mkv/m4v - wrote one mix"
    } else {
        ""
    };
    let inputs = audio_inputs(project, is_gif, layout, opts.stem_by, (start, start + dur));
    // Stems: the file gets the mix (input 0 of the audio), the rest go to their own WAV outputs
    let main_audio = if layout == AudioLayout::Stems { inputs.len().min(1) } else { inputs.len() };
    let stem_dir = if inputs.len() > main_audio {
        let d = TempDir(stems_dir(&tmp.0));
        let _ = std::fs::remove_dir_all(&d.0); // a leftover from a crashed run
        std::fs::create_dir_all(&d.0).map_err(|e| format!("stems folder: {e}"))?;
        Some(d)
    } else {
        None
    };
    if audio_only && inputs.is_empty() {
        return Err("Nothing to export: no audio clips".into());
    }
    let listeners = inputs
        .iter()
        .map(|_| TcpListener::bind(("127.0.0.1", 0)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("audio socket: {e}"))?;
    let ports = listeners
        .iter()
        .map(|l| l.local_addr().map(|a| a.port()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("audio socket: {e}"))?;

    // 2. how frames leave: NV12 straight off the GPU when the encode is 4:2:0 anyway (even sizes
    // only - NV12 has no odd-size chroma here), RGBA otherwise and always on the CPU path
    // ws:export-encoders: "auto" → probed hardware encoder; a stale explicit choice falls back to auto
    let (encoder, note) = resolve_encoder(&ext, &opts.encoder);
    if let Some(n) = note {
        prog.set(0.0, n);
    }
    let args = codec_args(&ext, &encoder, opts.crf, &opts.preset, &detect_encoders());
    let gpu_tx = match &opts.frames {
        FrameSource::Gpu(tx) => Some(tx.clone()),
        FrameSource::Cpu => None,
    };
    let nv12 = NV12_READBACK
        && gpu_tx.is_some()
        && w % 2 == 0
        && h % 2 == 0
        && args.iter().any(|a| a == "yuv420p" || a == "nv12");
    let video = (!audio_only).then_some(if nv12 { VideoIn::Nv12 } else { VideoIn::Rgba });

    // 3. ffmpeg
    let mut cmd = ffpipe::command(&ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error"]);
    cmd.args(input_args(video, (w, h), fps, &ports, main_audio));
    let (vf, af) = output_filters(opts, (w, h), !inputs.is_empty(), &args);
    if !audio_only && !vf.is_empty() {
        cmd.args(["-vf", &vf]);
    }
    let titles: Vec<&str> = inputs.iter().map(|a| a.title.as_str()).collect();
    cmd.args(audio_out_args(&af, layout, &titles[..main_audio]));
    cmd.args(&args);
    // nothing is inherited from the inputs; only what the user typed is written
    cmd.args(["-map_metadata", "-1"]);
    for (k, v) in opts.metadata.iter().filter(|(k, _)| !k.trim().is_empty()) {
        cmd.args(["-metadata", &format!("{}={}", k.trim(), v)]);
    }
    if is_gif {
        cmd.arg("-an");
    }
    // (no `-shortest`: it drops the last video frame when dur isn't a whole number of frames; video and
    // audio both end at ≥ dur anyway)
    cmd.arg(&tmp.0);
    if let Some(d) = &stem_dir {
        cmd.args(stem_outputs(usize::from(video.is_some()) + main_audio, &titles[main_audio..], &d.0));
    }
    cmd.stdin(if audio_only { Stdio::null() } else { Stdio::piped() }).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
    let tail = stderr_tail(&mut child);

    // 4. audio threads + writer thread + frames on this thread, all at once. Whatever ends first
    // (done, cancel, ffmpeg dying) the others follow: a dead ffmpeg breaks the pipe/sockets, cancel is
    // polled everywhere, and `stop` releases audio threads still waiting for ffmpeg to connect.
    let stop = AtomicBool::new(false);
    let audio_done: Vec<AtomicU32> = inputs.iter().map(|_| AtomicU32::new(0)).collect();
    std::thread::scope(|s| {
        for ((input, listener), done) in inputs.iter().zip(listeners).zip(&audio_done) {
            let stop = &stop;
            s.spawn(move || feed_audio(input, listener, backend, start, dur, prog, stop, done, audio_only));
        }
        if let Some(mut pipe) = child.stdin.take() {
            // writer: a small bounded queue so the pipe write overlaps the next decode/render; written
            // buffers come back for reuse
            let (wtx, wrx) = std::sync::mpsc::sync_channel::<Frame>(WRITE_QUEUE);
            let (back_tx, back_rx) = std::sync::mpsc::channel::<Frame>();
            let writer = s.spawn(move || {
                for f in wrx {
                    if pipe.write_all(&f.rgba).is_err() {
                        return; // ffmpeg died - its stderr tells why
                    }
                    let _ = back_tx.send(f);
                }
            });
            let mut comp = Compositor::new();
            let mut cpu_rgba = Frame::new(w, h);
            // GPU path: decode here (off the UI thread), composite there (the GL context lives on the UI
            // thread), so an export runs the very same shaders the preview does.
            let mut gpu = gpu_tx.map(|tx| GpuScratch::new(backend, tx, nv12));
            let mut gaps = if gpu.is_some() { String::new() } else { cpu_gaps(project) };
            let end = start + dur;
            let mut next_req = 0u64;
            for i in 0..n {
                if prog.is_cancelled() {
                    break;
                }
                let t = start + i as f64 / fps;
                let mut frame = None;
                if let Some(g) = gpu.as_mut() {
                    next_req = next_req.max(i);
                    while next_req < n && g.window.has_room() {
                        let buf = back_rx.try_recv().unwrap_or_default();
                        if !g.request(project, start + next_req as f64 / fps, w, h, buf, text) {
                            break;
                        }
                        next_req += 1;
                    }
                    frame = g.window.pop(GPU_TIMEOUT);
                    if frame.is_none() {
                        // the renderer died or the UI stopped answering: finish on the CPU from this very
                        // frame (requests still in flight are dropped with the window) and say so
                        gpu = None;
                        gaps = cpu_gaps(project);
                    }
                }
                let frame = match frame {
                    Some(f) => f,
                    None => {
                        let mut out = back_rx.try_recv().unwrap_or_default();
                        let mut tr = text.lock().unwrap_or_else(|e| e.into_inner());
                        if nv12 {
                            // ffmpeg was promised NV12: convert the CPU frame the same way the GPU does
                            comp.render(project, t, w, h, &mut pool, &mut tr, &mut cpu_rgba);
                            out.resize(w, h);
                            rgba_to_nv12(&cpu_rgba.rgba, w, h, &mut out.rgba);
                        } else {
                            out.resize(w, h);
                            out.rgba.resize(w as usize * h as usize * 4, 0);
                            comp.render(project, t, w, h, &mut pool, &mut tr, &mut out);
                        }
                        out
                    }
                };
                if wtx.send(frame).is_err() {
                    break; // the writer stopped: ffmpeg died
                }
                prog.set(
                    (i + 1) as f32 / n as f32,
                    format!("Encoding {t:.1} / {end:.1} s{gaps}{audio_note}{}", audio_status(&audio_done)),
                );
            }
            if prog.is_cancelled() {
                let _ = child.kill(); // unblocks the writer and the audio threads at once
            }
            drop(wtx);
            let _ = writer.join();
        }
        // 5. wait (audio threads are joined when the scope ends - after `stop` frees any still waiting)
        let r = wait_ffmpeg(&mut child, tail, prog);
        stop.store(true, Ordering::SeqCst);
        r
    })?;
    // move the finished file into place (decoders released first so an in-place export over the source
    // can replace it)
    drop(pool);
    tmp.commit(&opts.out_path)?;
    // the stems folder only appears once the whole export succeeded (a failure drops `stem_dir`,
    // which deletes the partial WAVs)
    match &stem_dir {
        Some(d) => d.commit(&stems_dir(&opts.out_path)),
        None => Ok(()),
    }
}

// ---- ws:export-pipeline ----
/// Item 4's switch: GPU exports read frames back as NV12 when the encode is 4:2:0.
const NV12_READBACK: bool = true;
/// GPU frame requests kept in flight by one export.
const GPU_WINDOW: usize = 4;
/// "The GPU stopped answering": how long the oldest in-flight request may take before the export
/// finishes on the CPU.
const GPU_TIMEOUT: Duration = Duration::from_secs(5);
/// Frames queued between the render loop and the pipe writer.
const WRITE_QUEUE: usize = 3;

/// How the picture reaches ffmpeg's stdin.
#[derive(Clone, Copy, Debug, PartialEq)]
enum VideoIn {
    Rgba,
    /// BT.709 limited range, declared on the input so ffmpeg never guesses.
    Nv12,
}

/// One audio stream of the export: headerless `f32le` stereo at `SAMPLE_RATE`, mixed from `project`
/// on its own thread and streamed to ffmpeg over a loopback socket (`tcp://127.0.0.1:<port>` - std
/// has no second pipe to hand a child on Windows, and a temp file can't be read while it is written).
/// The order of `audio_inputs` is the order of the output's audio streams.
pub(crate) struct AudioInput<'a> {
    /// Stream title (`-metadata:s:a:N title=`); empty = none.
    pub title: String,
    /// What this stream mixes: the project itself, or (stems) an owned copy with other tracks muted.
    pub project: std::borrow::Cow<'a, Project>,
}

/// The export's audio streams: one mix of everything (titled "Mix" for Streams), then - unless the
/// layout is Mix - one per stem over `range` (`engine::stems`); none for a gif / a project with no
/// enabled audio.
fn audio_inputs(
    project: &Project,
    is_gif: bool,
    layout: AudioLayout,
    stem_by: StemBy,
    range: (f64, f64),
) -> Vec<AudioInput<'_>> {
    let has_audio = !is_gif
        && project
            .audio_tracks()
            .into_iter()
            .any(|i| project.active(i) && project.tracks[i].clips.iter().any(|c| c.enabled));
    if !has_audio {
        return Vec::new();
    }
    let title = if layout == AudioLayout::Streams { "Mix" } else { "" };
    let mut v = vec![AudioInput { title: title.into(), project: std::borrow::Cow::Borrowed(project) }];
    if layout != AudioLayout::Mix {
        v.extend(
            crate::engine::stems::groups(project, stem_by, range)
                .into_iter()
                .map(|s| AudioInput { title: s.title, project: std::borrow::Cow::Owned(s.project) }),
        );
    }
    v
}

/// ffmpeg's inputs and stream maps: raw video on stdin as input 0 (none for audio-only), then one
/// `f32le` input per audio port; the first `mapped` of them are mapped (in order) into the main output
/// (the rest feed the stem outputs).
fn input_args(video: Option<VideoIn>, size: (u32, u32), fps: f64, audio_ports: &[u16], mapped: usize) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    if let Some(v) = video {
        a.extend(["-f", "rawvideo"].map(String::from));
        match v {
            VideoIn::Rgba => a.extend(["-pix_fmt", "rgba"].map(String::from)),
            VideoIn::Nv12 => a.extend(
                [
                    "-pix_fmt",
                    "nv12",
                    "-colorspace",
                    "bt709",
                    "-color_primaries",
                    "bt709",
                    "-color_trc",
                    "bt709",
                    "-color_range",
                    "tv",
                ]
                .map(String::from),
            ),
        }
        a.extend([
            String::from("-s"),
            format!("{}x{}", size.0, size.1),
            String::from("-r"),
            format!("{fps}"),
            String::from("-i"),
            String::from("pipe:0"),
        ]);
    }
    for port in audio_ports {
        a.extend([
            String::from("-f"),
            String::from("f32le"),
            String::from("-ar"),
            SAMPLE_RATE.to_string(),
            String::from("-ac"),
            String::from("2"),
            String::from("-i"),
            format!("tcp://127.0.0.1:{port}"),
        ]);
    }
    let first_audio = usize::from(video.is_some());
    if video.is_some() {
        a.extend(["-map", "0:v"].map(String::from));
    }
    for i in 0..mapped.min(audio_ports.len()) {
        a.extend([String::from("-map"), format!("{}:a", first_audio + i)]);
    }
    a
}

/// " · mixing audio N %" while any audio stream is still behind, for the progress line.
fn audio_status(done: &[AtomicU32]) -> String {
    match done.iter().map(|d| d.load(Ordering::Relaxed)).min() {
        Some(p) if p < 1000 => format!(" · mixing audio {} %", p / 10),
        _ => String::new(),
    }
}

/// Mix `input` over `[start, start + dur)` and stream it to ffmpeg's first connection on `listener`.
/// `done` = permille mixed; `report` (audio-only exports) also drives `prog`. Returns when finished,
/// cancelled, `stop`ped, or when ffmpeg went away (its exit status says why, not this).
#[allow(clippy::too_many_arguments)]
fn feed_audio(
    input: &AudioInput,
    listener: TcpListener,
    backend: Backend,
    start: f64,
    dur: f64,
    prog: &Progress,
    stop: &AtomicBool,
    done: &AtomicU32,
    report: bool,
) {
    let quit = || prog.is_cancelled() || stop.load(Ordering::SeqCst);
    if listener.set_nonblocking(true).is_err() {
        return;
    }
    let mut sock = loop {
        match listener.accept() {
            Ok((sock, _)) => break sock,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if quit() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return,
        }
    };
    if sock.set_nonblocking(false).is_err() {
        return;
    }
    let mut pool = DecoderPool::new(backend);
    let mut mixer = Mixer::new();
    let total = (dur * SAMPLE_RATE as f64).ceil() as u64;
    let mut buf = vec![0f32; MIX_BLOCK * 2];
    let mut bytes: Vec<u8> = Vec::with_capacity(MIX_BLOCK * 8);
    let mut mixed = 0u64;
    while mixed < total {
        if quit() {
            return;
        }
        let n = (total - mixed).min(MIX_BLOCK as u64) as usize;
        mixer.mix(&input.project, start + mixed as f64 / SAMPLE_RATE as f64, &mut pool, &mut buf[..n * 2]);
        bytes.clear();
        for s in &buf[..n * 2] {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        if sock.write_all(&bytes).is_err() {
            return;
        }
        mixed += n as u64;
        let f = mixed as f32 / total.max(1) as f32;
        done.store((f * 1000.0) as u32, Ordering::Relaxed);
        if report {
            prog.set(f, "Encoding audio…");
        }
    }
    let _ = sock.shutdown(std::net::Shutdown::Write);
}

/// RGBA → NV12 with `NV12_MAIN`'s maths (BT.709 limited, 2x2 box chroma), for a GPU export that
/// finishes on the CPU after ffmpeg was already promised NV12. `w`, `h` even.
fn rgba_to_nv12(rgba: &[u8], w: u32, h: u32, out: &mut Vec<u8>) {
    let (w, h) = (w as usize, h as usize);
    out.clear();
    out.resize(w * h * 3 / 2, 0);
    let px = |x: usize, y: usize| {
        let i = (y * w + x) * 4;
        [rgba[i] as f32 / 255.0, rgba[i + 1] as f32 / 255.0, rgba[i + 2] as f32 / 255.0]
    };
    let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let q = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    for y in 0..h {
        for x in 0..w {
            out[y * w + x] = q(16.0 + 219.0 * luma(px(x, y)));
        }
    }
    let uv = &mut out[w * h..];
    for j in 0..h / 2 {
        for i in 0..w / 2 {
            let mut c = [0f32; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = px(2 * i + dx, 2 * j + dy);
                for k in 0..3 {
                    c[k] += p[k] * 0.25;
                }
            }
            let y = luma(c);
            uv[j * w + 2 * i] = q(128.0 + 224.0 * (c[2] - y) / 1.8556);
            uv[j * w + 2 * i + 1] = q(128.0 + 224.0 * (c[0] - y) / 1.5748);
        }
    }
}

/// GPU requests in flight, oldest first. Replies are taken strictly in send order, so frames reach
/// ffmpeg in timeline order however the UI thread schedules them.
struct ReplyWindow {
    cap: usize,
    rx: std::collections::VecDeque<std::sync::mpsc::Receiver<Option<Frame>>>,
}

impl ReplyWindow {
    fn new(cap: usize) -> Self {
        Self { cap, rx: std::collections::VecDeque::new() }
    }
    fn has_room(&self) -> bool {
        self.rx.len() < self.cap
    }
    fn push(&mut self, rx: std::sync::mpsc::Receiver<Option<Frame>>) {
        self.rx.push_back(rx);
    }
    /// The oldest request's frame. None = it failed (renderer gone / sender dropped), it took longer
    /// than `timeout`, or nothing is in flight.
    fn pop(&mut self, timeout: Duration) -> Option<Frame> {
        self.rx.pop_front()?.recv_timeout(timeout).ok().flatten()
    }
}

/// Decoder state for the GPU export path: the same layer decoding the player does, on this thread,
/// plus the requests in flight to the UI thread.
struct GpuScratch {
    pool: DecoderPool,
    spare: Vec<Frame>,
    shapes: crate::engine::shapes::ShapeRasterizer,
    comp: Compositor,
    tx: std::sync::mpsc::Sender<GpuFrameRequest>,
    window: ReplyWindow,
    nv12: bool,
}

impl GpuScratch {
    fn new(backend: Backend, tx: std::sync::mpsc::Sender<GpuFrameRequest>, nv12: bool) -> Self {
        Self {
            pool: DecoderPool::new(backend),
            spare: Vec::new(),
            shapes: crate::engine::shapes::ShapeRasterizer::new(),
            comp: Compositor::new(),
            tx,
            window: ReplyWindow::new(GPU_WINDOW),
            nv12,
        }
    }
    /// Decode the layers for `t` and queue them for the UI thread to composite into `buf`. False = the
    /// request channel is closed (give up on the GPU path once the window drains).
    #[allow(clippy::too_many_arguments)]
    fn request(
        &mut self,
        project: &Project,
        t: f64,
        w: u32,
        h: u32,
        buf: Frame,
        text: &Arc<Mutex<TextRasterizer>>,
    ) -> bool {
        let layers = {
            let mut tr = text.lock().unwrap_or_else(|e| e.into_inner());
            crate::playback::decode_layers(
                project,
                t,
                w,
                h,
                &mut self.pool,
                &mut self.spare,
                &mut tr,
                &mut self.shapes,
                &mut self.comp,
            )
        };
        let (reply, rx) = std::sync::mpsc::sync_channel(1);
        if self.tx.send(GpuFrameRequest { layers, t, w, h, nv12: self.nv12, buf, reply }).is_err() {
            return false;
        }
        self.window.push(rx);
        true
    }
}

/// What this export cannot render, as a suffix for the progress line (empty when nothing is dropped).
///
/// ponytail: export renders on a background thread with the CPU `Compositor`; the GL context lives on
/// the UI thread, so GPU-only effect kinds and node graphs are skipped. Naming them in the status beats
/// dropping them silently - replace this with a real GPU render path (`GpuRenderer::render_frame` fed
/// from the UI thread) and the note goes away.
fn cpu_gaps(project: &Project) -> String {
    let mut kinds: Vec<&str> = Vec::new();
    let mut graphs = 0usize;
    let tracks = project.tracks.iter().chain(project.sequences.iter().flat_map(|s| s.tracks.iter()));
    for clip in tracks.flat_map(|t| t.clips.iter()).filter(|c| c.enabled) {
        if clip.uses_graph() {
            graphs += 1;
        }
        for e in clip.effects.iter().filter(|e| e.enabled) {
            let name = e.kind.name();
            if crate::engine::effects::gpu_only(e.kind) && !kinds.contains(&name) {
                kinds.push(name);
            }
        }
    }
    let mut parts: Vec<String> = Vec::new();
    if !kinds.is_empty() {
        parts.push(format!("{} (GPU-only)", kinds.join(", ")));
    }
    if graphs > 0 {
        parts.push(format!("{graphs} node graph(s)"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" - not rendered: {}", parts.join(", "))
    }
}

/// Deleted on drop (a no-op once `commit`ted into place).
pub(crate) struct TempFile(pub(crate) PathBuf);
impl TempFile {
    /// Move the finished file over `dst` (replaces an existing file).
    pub(crate) fn commit(self, dst: &Path) -> Result<(), String> {
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
/// or deletes an existing destination - or the source, when cutting in place.
pub(crate) fn temp_output(out: &Path) -> TempFile {
    let stem = out.file_stem().unwrap_or_default().to_string_lossy();
    let ext = out.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    TempFile(out.with_file_name(format!(".{stem}.simple-editor-tmp{ext}")))
}

/// Drain ffmpeg's stderr on a helper thread; returns the last ~2000 chars.
pub(crate) fn stderr_tail(child: &mut Child) -> Option<JoinHandle<String>> {
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
pub(crate) fn wait_ffmpeg(child: &mut Child, tail: Option<JoinHandle<String>>, prog: &Progress) -> Result<(), String> {
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
/// no gaps between clips, no text/image/sequence clips, no effects/retime/pan/fades, volume 1, all clips
/// enabled, no transitions, no burnt-in subtitles), return the segments as (src_in, duration) in timeline
/// order. Such a project can be written with `-c copy`.
pub fn lossless_segments(project: &Project) -> Option<Vec<(f64, f64)>> {
    const EPS: f64 = 1e-4;
    if project.show_subtitles && !project.subtitles.is_empty() {
        return None;
    }
    if project.tracks.iter().any(|t| !t.transitions.is_empty()) {
        return None;
    }
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

// ---- ws:export-encoders ----
/// One stretch of a smart render, in SOURCE seconds `[a, b)`.
#[allow(dead_code)] // planning only: the copy/encode/concat/verify runner is not written yet
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SmartSpan {
    /// Whole GOPs (`a` is a keyframe, `b` is the next keyframe or the segment end on one) - stream copy.
    Copy(f64, f64),
    /// Partial GOPs at a cut - re-encode with parameters matched to the source.
    Encode(f64, f64),
}

/// Smart-render plan for `lossless_segments` output, given the source's sorted keyframe times (one
/// `ffprobe -skip_frame nokey -show_entries frame=pts_time` listing). Each segment copies from its
/// first keyframe at/after the in point to the last keyframe at/before the out point, and re-encodes
/// the head and tail around that. A segment with no whole GOP inside is re-encoded entirely.
#[allow(dead_code)]
pub fn smart_spans(segs: &[(f64, f64)], keyframes: &[f64]) -> Vec<SmartSpan> {
    const EPS: f64 = 1e-4;
    let mut out = Vec::new();
    for &(a, dur) in segs {
        let b = a + dur;
        let first = keyframes.iter().copied().find(|&k| k >= a - EPS);
        let last = keyframes.iter().copied().rev().find(|&k| k <= b + EPS);
        // a keyframe sitting on the out point ends the copy there exactly
        let copy_end = if keyframes.iter().any(|&k| (k - b).abs() <= EPS) { Some(b) } else { last };
        match (first, copy_end) {
            (Some(k0), Some(k1)) if k1 - k0 > EPS => {
                if k0 - a > EPS {
                    out.push(SmartSpan::Encode(a, k0));
                }
                out.push(SmartSpan::Copy(k0.max(a), k1));
                if b - k1 > EPS {
                    out.push(SmartSpan::Encode(k1, b));
                }
            }
            _ => out.push(SmartSpan::Encode(a, b)),
        }
    }
    out
}

/// Lossless cut: `ffmpeg -ss in -t dur -i src -c copy -avoid_negative_ts make_zero` per segment (with
/// `-map` dropping muted audio streams), then concat demuxer when there is more than one segment.
/// Cuts land on keyframes (not frame-accurate) - that's the trade for being instant.
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

// ---- ws:export-encoders ----
/// The preset handed to `codec_args`: the Advanced Preset when one is chosen, else the Export pane's
/// Speed choice (`Settings.export_speed`: "speed" | "balanced" | "small").
pub fn effective_preset(preset: &str, speed: &str) -> String {
    if preset.is_empty() || preset == "auto" { speed.to_string() } else { preset.to_string() }
}

/// Speed tier of a Speed token or an x264 preset name (hardware encoders map the tier to their own).
fn speed_tier(preset: &str) -> &'static str {
    match preset {
        "speed" | "ultrafast" | "superfast" | "veryfast" | "faster" => "speed",
        "small" | "slow" | "slower" | "veryslow" | "placebo" => "small",
        _ => "balanced",
    }
}

/// What "auto" / a stale choice becomes for this export: the probed fastest hardware H.264 encoder
/// for the container (software when the probe hasn't finished - never waits for it). The note is
/// for the export status when an explicit choice was dropped.
pub fn resolve_encoder(ext: &str, encoder: &str) -> (String, Option<String>) {
    use crate::engine::encoders;
    let note = encoders::is_broken(encoder)
        .then(|| format!("{encoder} can't run on this machine - using auto"));
    if note.is_some() || encoder.is_empty() || encoder == "auto" {
        return (encoders::best_for(ext).unwrap_or_else(|| "auto".into()), note);
    }
    (encoder.to_string(), None)
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
        // ws:export-encoders: a Speed token ("speed" | "balanced" | "small", see `effective_preset`)
        // maps per encoder; an explicit x264 name passes through to x264/x265 and picks the tier for
        // hardware encoders.
        let tier = speed_tier(preset);
        let preset = match preset {
            "" | "auto" | "speed" | "balanced" | "small" => match tier {
                "speed" => "veryfast",
                "small" => "slow",
                _ => "medium",
            },
            p => p,
        };
        let (nv, qsv, amf) = match tier {
            "speed" => ("p1", "veryfast", "speed"),
            "small" => ("p7", "veryslow", "quality"),
            _ => ("p4", "medium", "balanced"),
        };
        // `crf` is stored on the x264 0..=51 scale (what the quality-percent slider maps through).
        // Encoders on other scales get it converted HERE, at emission: vp9/av1 use 0..=63, and QSV's
        // global_quality treats 0 as "unset" so its best real value is 1.
        let crf63 = (crf as f64 * 63.0 / 51.0).round() as u32;
        let crf_qsv = crf.max(1);
        video = match enc {
            "libx264" | "libx265" => {
                format!("-c:v {enc} -preset {preset} -crf {crf} -pix_fmt yuv420p")
            }
            "h264_nvenc" | "hevc_nvenc" | "av1_nvenc" => {
                format!("-c:v {enc} -preset {nv} -cq {crf} -b:v 0 -pix_fmt yuv420p")
            }
            "h264_qsv" | "hevc_qsv" => {
                format!("-c:v {enc} -preset {qsv} -global_quality {crf_qsv} -pix_fmt nv12")
            }
            "h264_amf" | "hevc_amf" => format!("-c:v {enc} -quality {amf} -rc cqp -qp_i {crf} -qp_p {crf}"),
            // ponytail: av1_qsv / av1_amf get the speed preset only - their quality scales (0..255)
            // are not confirmed here, so they run at the encoder's default rate control.
            "av1_qsv" => format!("-c:v av1_qsv -preset {qsv} -pix_fmt nv12"),
            "av1_amf" => format!("-c:v av1_amf -quality {amf}"),
            "libvpx-vp9" => {
                format!("-c:v libvpx-vp9 -crf {crf63} -b:v 0 -row-mt 1 -cpu-used 2 -pix_fmt yuv420p")
            }
            "libaom-av1" | "libsvtav1" => format!("-c:v {enc} -crf {crf63} -pix_fmt yuv420p"),
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

/// User-facing quality percent (100 = best) -> the CRF `codec_args` actually uses (0 = best, 51 = worst).
/// `pct` is expected pre-clamped to 0..=100 (the slider's own range); clamps again here as a safety net.
pub fn crf_from_quality_percent(pct: u32) -> u32 {
    let pct = pct.min(100) as f64;
    (((100.0 - pct) / 100.0) * 51.0).round().clamp(0.0, 51.0) as u32
}

/// Inverse of `crf_from_quality_percent`, for displaying the slider from a stored `Settings.crf`.
pub fn quality_percent_from_crf(crf: u32) -> u32 {
    let crf = crf.min(51) as f64;
    (100.0 - (crf / 51.0) * 100.0).round().clamp(0.0, 100.0) as u32
}

/// Rough estimated output size in bytes for the export progress/quality UI - NOT a guarantee, since CRF
/// targets a quality level, not a bitrate. Video bitrate is extrapolated from a "reasonable quality" x264
/// ballpark at CRF 23 (0.1 bits/pixel/frame), scaled by the x264 rule of thumb that every 6 CRF steps
/// roughly halves/doubles bitrate; audio assumes ~128kbps AAC-ish stereo per stream in the file
/// (`audio_streams`: 1 for a mix, 1 + stems for "Separate tracks in file").
pub fn estimate_export_bytes(width: u32, height: u32, fps: f64, duration_secs: f64, crf: u32, audio_streams: u32) -> u64 {
    let pixels_per_sec = width as f64 * height as f64 * fps;
    let base_bitrate_bps = pixels_per_sec * 0.1;
    let scale = 2f64.powf((23.0 - crf as f64) / 6.0);
    let video_bitrate_bps = (base_bitrate_bps * scale).clamp(50_000.0, (pixels_per_sec * 2.0).max(50_000.0));
    let audio_bitrate_bps = 128_000.0 * audio_streams as f64;
    ((video_bitrate_bps + audio_bitrate_bps) / 8.0 * duration_secs.max(0.0)) as u64
}

pub const AUDIO_EXTS: &[&str] = &["mp3", "wav", "m4a", "flac", "ogg", "aac", "opus"];

#[cfg(test)]
pub(crate) mod tests {
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
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
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

    /// A constant-valued audio source (mirrors the mixer's own test source).
    struct Const(f32);
    impl crate::media::AudioSource for Const {
        fn duration(&self) -> f64 {
            10.0
        }
        fn read_at(&mut self, _t: f64, out: &mut [f32]) {
            out.fill(self.0);
        }
    }

    #[test]
    fn mix_block_keeps_short_fades_sharp() {
        // `resample_add` lerps the clip gain across one block, so the block length is the fade's
        // resolution: a 20 ms fade-in must be done at 20 ms, not still ramping (100 ms blocks put it
        // at ~0.2). Export has to ramp exactly like playback does.
        let mut p = Project::from_media(asset("Z:\\nope\\fake.wav", 1));
        let ai = p.audio_tracks()[0];
        p.tracks[ai].clips[0].fade_in = 0.02;
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_audio("Z:\\nope\\fake.wav", 0, Box::new(Const(1.0)));
        let mut buf = vec![0f32; MIX_BLOCK * 2];
        Mixer::new().mix(&p, 0.0, &mut pool, &mut buf);
        let at = |ms: f64| buf[(ms / 1000.0 * SAMPLE_RATE as f64) as usize * 2];
        assert!(at(0.0).abs() < 0.05, "fade should start at silence: {}", at(0.0));
        assert!((at(10.0) - 0.5).abs() < 0.1, "half way through the fade: {}", at(10.0));
        assert!(at(20.0) > 0.85, "20 ms fade still ramping at 20 ms: {}", at(20.0));
    }

    #[test]
    fn cpu_gaps_names_what_export_drops() {
        let mut p = Project::from_media(asset("Z:\\nope\\fake.mp4", 1));
        assert_eq!(cpu_gaps(&p), "");
        // effects the CPU compositor renders are not reported
        let id = p.tracks[0].clips[0].id;
        p.clip_mut(id).unwrap().effects.push(crate::model::Effect::new(crate::model::EffectKind::Blur));
        assert_eq!(cpu_gaps(&p), "");
        p.clip_mut(id).unwrap().effects.push(crate::model::Effect::new(crate::model::EffectKind::Vhs));
        assert!(cpu_gaps(&p).contains("VHS"), "{}", cpu_gaps(&p));
        // a disabled one is not a gap
        p.clip_mut(id).unwrap().effects.last_mut().unwrap().enabled = false;
        assert_eq!(cpu_gaps(&p), "");
    }

    pub(crate) fn wait_done(prog: &Progress) -> Option<String> {
        for _ in 0..1200 {
            if prog.is_done() {
                return prog.error();
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Some("timeout".into())
    }

    pub(crate) fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("simple-editor-export-test-{}-{name}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    /// red 0–2 s, green 2–4 s, two sine streams, keyframe every second. None if ffmpeg is missing.
    pub(crate) fn gen_media(dir: &Path) -> Option<PathBuf> {
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

    pub(crate) fn probe_duration(path: &Path) -> Option<f64> {
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
        // ws:export-encoders: an x264 preset name picks the hardware tier (slow → p7)
        assert!(s.contains("-c:v hevc_nvenc -preset p7 -cq 20 -b:v 0"), "{s}");
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

    // ---- ws:export-encoders ----
    #[test]
    fn speed_choice_maps_to_each_encoder_preset() {
        let j = |enc: &str, p: &str| codec_args("mp4", enc, 20, p, &[]).join(" ");
        for (speed, x264, nv, qsv, amf) in [
            ("speed", "veryfast", "p1", "veryfast", "speed"),
            ("balanced", "medium", "p4", "medium", "balanced"),
            ("small", "slow", "p7", "veryslow", "quality"),
        ] {
            assert!(j("libx264", speed).contains(&format!("-preset {x264} -crf 20")), "{speed}");
            assert!(j("h264_nvenc", speed).contains(&format!("-preset {nv} -cq 20")), "{speed}");
            assert!(j("h264_qsv", speed).contains(&format!("-preset {qsv} -global_quality 20")), "{speed}");
            assert!(j("h264_amf", speed).contains(&format!("-quality {amf} -rc cqp")), "{speed}");
        }
        // an explicit Advanced preset wins over the Speed choice
        assert_eq!(effective_preset("ultrafast", "small"), "ultrafast");
        assert_eq!(effective_preset("auto", "small"), "small");
        assert_eq!(effective_preset("", "speed"), "speed");
        assert!(j("libx264", "ultrafast").contains("-preset ultrafast"));
        // auto with no finished probe stays software
        assert!(j("auto", "balanced").contains("-c:v libx264 -preset medium"));
    }

    #[test]
    fn smart_spans_copy_whole_gops_only() {
        use SmartSpan::*;
        let kf = [0.0, 2.0, 4.0, 6.0, 8.0];
        // 1.0..7.0: head 1-2 encoded, 2-6 copied, tail 6-7 encoded
        assert_eq!(smart_spans(&[(1.0, 6.0)], &kf), vec![Encode(1.0, 2.0), Copy(2.0, 6.0), Encode(6.0, 7.0)]);
        // exactly on keyframes: all copy
        assert_eq!(smart_spans(&[(2.0, 4.0)], &kf), vec![Copy(2.0, 6.0)]);
        // no whole GOP inside: all encode
        assert_eq!(smart_spans(&[(2.5, 1.0)], &kf), vec![Encode(2.5, 3.5)]);
        assert_eq!(smart_spans(&[(1.0, 1.0)], &[]), vec![Encode(1.0, 2.0)]);
    }

    #[test]
    fn parse_encoder_list() {
        let s = " V..... = Video\n A..... = Audio\n ------\n V....D libx264  H.264\n A....D aac  AAC\n S..... srt  SubRip\n";
        assert_eq!(parse_encoders(s), vec!["libx264".to_string(), "aac".to_string()]);
    }

    /// The stored CRF is x264's 0..=51 scale; encoders on other scales get converted at emission.
    /// VP9/AV1 use 0..=63 (an un-rescaled 51 was never "worst", compressing the whole slider into
    /// 80% of the range) and QSV treats global_quality 0 as "unset" (best real value is 1).
    #[test]
    fn codec_args_convert_crf_scales_per_encoder() {
        let has = |v: &[String], flag: &str, val: &str| v.windows(2).any(|w| w[0] == flag && w[1] == val);
        let all = ["libx264", "libvpx-vp9", "libaom-av1", "h264_qsv"].map(String::from);
        let v = codec_args("webm", "libvpx-vp9", 51, "medium", &all);
        assert!(has(&v, "-crf", "63"), "vp9 worst = 63, got {v:?}");
        let v = codec_args("webm", "libvpx-vp9", 0, "medium", &all);
        assert!(has(&v, "-crf", "0"), "vp9 best stays 0: {v:?}");
        let v = codec_args("mkv", "libaom-av1", 26, "medium", &all);
        assert!(has(&v, "-crf", "32"), "av1 midpoint rescales 26 -> 32: {v:?}");
        let v = codec_args("mp4", "h264_qsv", 0, "medium", &all);
        assert!(has(&v, "-global_quality", "1"), "qsv 0 means unset - floor at 1: {v:?}");
        let v = codec_args("mp4", "libx264", 18, "medium", &all);
        assert!(has(&v, "-crf", "18"), "x264 passes through untouched: {v:?}");
    }

    #[test]
    fn quality_percent_crf_conversion() {
        assert_eq!(crf_from_quality_percent(100), 0);
        assert_eq!(crf_from_quality_percent(50), 26);
        assert_eq!(crf_from_quality_percent(0), 51);
        assert_eq!(quality_percent_from_crf(0), 100);
        assert_eq!(quality_percent_from_crf(51), 0);
    }

    #[test]
    fn quality_percent_round_trip_within_tolerance() {
        // Integer rounding both directions means this isn't exact; bound the drift instead.
        for pct in [0, 10, 25, 33, 50, 66, 75, 90, 100] {
            let crf = crf_from_quality_percent(pct);
            let back = quality_percent_from_crf(crf);
            let diff = (back as i32 - pct as i32).abs();
            assert!(diff <= 2, "pct={pct} crf={crf} back={back} diff={diff}");
        }
    }

    #[test]
    fn estimate_bytes_sane_ballpark() {
        // 1920x1080 @ 30fps, 60s, CRF 23, by hand: base = 1920*1080*30*0.1 = 6,220,800 bps;
        // scale(23) = 1; + 128kbps audio = 6,348,800 bps; /8 * 60s = 47,616,000 bytes (~47.6 MB).
        let bytes = estimate_export_bytes(1920, 1080, 30.0, 60.0, 23, 1);
        assert!((40_000_000..55_000_000).contains(&bytes), "{bytes}");
        // each extra audio stream adds 128 kbps * 60 s / 8 = 960,000 bytes
        assert_eq!(estimate_export_bytes(1920, 1080, 30.0, 60.0, 23, 3) - bytes, 1_920_000);
    }

    #[test]
    fn estimate_bytes_crf_scaling_halves_roughly() {
        let base = estimate_export_bytes(1920, 1080, 30.0, 60.0, 23, 1);
        let raised = estimate_export_bytes(1920, 1080, 30.0, 60.0, 29, 1); // +6 CRF ~= half the bitrate
        let ratio = raised as f64 / base as f64;
        assert!((0.4..0.6).contains(&ratio), "ratio={ratio}");
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
        // retimed clip → None
        let mut q = p.clone();
        let sp = q.tracks[0].clips[0].id;
        q.set_speed(&[sp], 2.0, false);
        assert!(lossless_segments(&q).is_none());
        // pan / fades → None
        let mut q = p.clone();
        q.tracks[1].clips[0].pan.value = 0.5;
        assert!(lossless_segments(&q).is_none());
        let mut q = p.clone();
        q.tracks[1].clips[0].fade_in = 0.5;
        assert!(lossless_segments(&q).is_none());
        // transition → None
        let mut q = p.clone();
        let right = q.tracks[0].clips[1].id;
        assert!(q.add_transition(right, crate::model::TransitionKind::CrossFade, 0.5).is_some());
        assert!(lossless_segments(&q).is_none());
        // burnt-in subtitles → None; hidden subtitles → Some
        let mut q = p.clone();
        q.add_cue(0.0, 1.0, "hi");
        assert!(lossless_segments(&q).is_none());
        q.show_subtitles = false;
        assert!(lossless_segments(&q).is_some());
        // sequence clip → None
        let mut q = p.clone();
        let seq = q.new_sequence("s", 320, 240, 30.0);
        q.insert_sequence_clip(seq, 4.0, None);
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

    // ---- ws:export-pipeline ----
    #[test]
    fn input_args_video_audio_and_maps() {
        let s = |v: Vec<String>| v.join(" ");
        assert_eq!(
            s(input_args(Some(VideoIn::Rgba), (320, 240), 30.0, &[5000], 1)),
            "-f rawvideo -pix_fmt rgba -s 320x240 -r 30 -i pipe:0 \
             -f f32le -ar 48000 -ac 2 -i tcp://127.0.0.1:5000 -map 0:v -map 1:a"
        );
        let nv12 = s(input_args(Some(VideoIn::Nv12), (1920, 1080), 25.0, &[], 0));
        assert!(nv12.starts_with("-f rawvideo -pix_fmt nv12 -colorspace bt709"), "{nv12}");
        assert!(nv12.contains("-color_range tv") && nv12.ends_with("-i pipe:0 -map 0:v"), "{nv12}");
        // audio-only: the audio inputs are 0..N; N stems map in order
        assert_eq!(
            s(input_args(None, (1, 1), 30.0, &[1, 2], 2)),
            "-f f32le -ar 48000 -ac 2 -i tcp://127.0.0.1:1 -f f32le -ar 48000 -ac 2 -i tcp://127.0.0.1:2 \
             -map 0:a -map 1:a"
        );
        // ws:export-audio-tracks: Stems maps only the mix into the main output
        assert!(s(input_args(Some(VideoIn::Rgba), (2, 2), 30.0, &[1, 2, 3], 1)).ends_with("-map 0:v -map 1:a"));
    }

    // ---- ws:export-audio-tracks ----
    #[test]
    fn streams_args_title_disposition_and_mix_only_loudnorm() {
        let s = |v: Vec<String>| v.join(" ");
        assert_eq!(
            s(audio_out_args(LOUDNORM, AudioLayout::Streams, &["Mix", "Dialogue", "Music"])),
            "-filter:a:0 loudnorm=I=-14:TP=-1:LRA=11 -ar 48000 \
             -metadata:s:a:0 title=Mix -disposition:a:0 default \
             -metadata:s:a:1 title=Dialogue -disposition:a:1 0 \
             -metadata:s:a:2 title=Music -disposition:a:2 0"
        );
        // Mix / Stems: plain -af over the single stream in the file, no titles
        assert_eq!(s(audio_out_args(LOUDNORM, AudioLayout::Mix, &[""])), "-af loudnorm=I=-14:TP=-1:LRA=11 -ar 48000");
        assert_eq!(s(audio_out_args("", AudioLayout::Stems, &[""])), "");
    }

    #[test]
    fn streams_fall_back_to_mix_outside_multi_audio_containers() {
        for ext in ["mp4", "mov", "mkv", "m4v"] {
            assert_eq!(effective_layout(AudioLayout::Streams, ext), AudioLayout::Streams);
        }
        for ext in ["webm", "avi", "wav", "mp3", "gif"] {
            assert_eq!(effective_layout(AudioLayout::Streams, ext), AudioLayout::Mix);
        }
        assert_eq!(effective_layout(AudioLayout::Stems, "wav"), AudioLayout::Stems);
        assert_eq!(AudioLayout::parse("Streams"), AudioLayout::Streams);
        assert_eq!(AudioLayout::parse("??"), AudioLayout::Mix);
        assert_eq!(StemBy::parse("role"), StemBy::Role);
    }

    #[test]
    fn stem_outputs_name_files_beside_the_video() {
        let out = Path::new("C:\\v\\My cut.mp4");
        let dir = stems_dir(out);
        assert_eq!(dir, Path::new("C:\\v\\My cut stems"));
        let a = stem_outputs(2, &["Dialogue", "SFX/Foley"], &dir);
        assert_eq!(&a[..8], ["-map", "2:a", "-c:a", "pcm_s24le", "-ar", "48000", "-map_metadata", "-1"]);
        assert!(a[8].ends_with("01 Dialogue.wav"), "{}", a[8]);
        assert_eq!(a[10], "3:a");
        assert!(a[17].ends_with("02 SFX_Foley.wav"), "{}", a[17]);
    }

    #[test]
    fn reply_window_answers_in_send_order() {
        let mut win = ReplyWindow::new(3);
        let mut tx = Vec::new();
        for _ in 0..3 {
            assert!(win.has_room());
            let (t, r) = std::sync::mpsc::sync_channel::<Option<Frame>>(1);
            win.push(r);
            tx.push(t);
        }
        assert!(!win.has_room());
        // answered out of order, taken in order
        for (i, t) in tx.iter().enumerate().rev() {
            t.send(Some(Frame { pts: i as f64, ..Frame::default() })).unwrap();
        }
        let short = Duration::from_millis(50);
        assert_eq!(win.pop(short).map(|f| f.pts), Some(0.0));
        assert!(win.has_room());
        assert_eq!(win.pop(short).map(|f| f.pts), Some(1.0));
        // a dropped sender / None reply / nothing in flight = give up on the GPU
        let (t, r) = std::sync::mpsc::sync_channel::<Option<Frame>>(1);
        win.push(r);
        drop(t);
        assert_eq!(win.pop(short).map(|f| f.pts), Some(2.0));
        assert!(win.pop(short).is_none());
        assert!(win.pop(short).is_none());
    }

    #[test]
    fn rgba_to_nv12_is_bt709_limited() {
        // 2x2: white, black / red, blue
        let px = [[255, 255, 255, 255], [0, 0, 0, 255], [255, 0, 0, 255], [0, 0, 255, 255]];
        let rgba: Vec<u8> = px.concat();
        let mut out = Vec::new();
        rgba_to_nv12(&rgba, 2, 2, &mut out);
        assert_eq!(out.len(), 6);
        assert_eq!(&out[..4], &[235, 16, 63, 32], "Y: 16..235, BT.709 weights");
        // one chroma sample for the block: the average colour (0.5, 0.25, 0.5)
        let (r, g, b) = (0.5f32, 0.25f32, 0.5f32);
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let cb = (128.0 + 224.0 * (b - y) / 1.8556).round() as u8;
        let cr = (128.0 + 224.0 * (r - y) / 1.5748).round() as u8;
        assert_eq!(&out[4..], &[cb, cr]);
        // grey has no chroma
        rgba_to_nv12(&[128, 128, 128, 255].repeat(4), 2, 2, &mut out);
        assert_eq!(&out[4..], &[128, 128]);
    }

    #[test]
    fn detect_encoders_has_x264() {
        if ffpipe::ffmpeg_exe().is_none() {
            eprintln!("ffmpeg missing - skipped");
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
            eprintln!("ffmpeg missing - skipped");
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
    fn scaled_export_real() {
        let dir = temp_dir("scaled");
        let Some(src) = gen_media(&dir) else {
            eprintln!("ffmpeg missing - skipped");
            return;
        };
        let p = Project::from_media(asset(&src.to_string_lossy(), 2));
        assert_eq!((p.width, p.height), (320, 240));
        let out = dir.join("half.mp4");
        let opts = ExportOptions {
            out_path: out.clone(),
            encoder: "auto".into(),
            crf: 23,
            preset: "ultrafast".into(),
            backend: Backend::Auto,
            out_size: Some((160, 120)),
            scaler: "bicubic".into(),
            frames: FrameSource::Cpu,
            metadata: Vec::new(),
            range: None,
            loudnorm: false,
            letterbox: false,
            // ---- ws:pro-schema ----
            audio: Default::default(),
            stem_by: Default::default(),
        };
        let text = Arc::new(Mutex::new(TextRasterizer::new()));
        assert_eq!(wait_done(&start_export(p, opts, text)), None);
        let o = ffpipe::command(&ffpipe::ffprobe_exe().unwrap())
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0"])
            .arg(&out)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "160,120");
        let d = probe_duration(&out).expect("probe");
        assert!((d - 4.0).abs() < 0.2, "duration {d}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn full_export_real() {
        let dir = temp_dir("export");
        let Some(src) = gen_media(&dir) else {
            eprintln!("ffmpeg missing - skipped");
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
            out_size: None,
            scaler: "bicubic".into(),
            frames: FrameSource::Cpu,
            metadata: Vec::new(),
            range: None,
            loudnorm: false,
            letterbox: false,
            // ---- ws:pro-schema ----
            audio: Default::default(),
            stem_by: Default::default(),
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

    // ---- ws:export-deliver ----
    fn opts_for(out: PathBuf) -> ExportOptions {
        ExportOptions {
            out_path: out,
            encoder: "auto".into(),
            crf: 23,
            preset: "ultrafast".into(),
            backend: Backend::Auto,
            out_size: None,
            scaler: "bicubic".into(),
            frames: FrameSource::Cpu,
            metadata: Vec::new(),
            range: None,
            loudnorm: false,
            letterbox: false,
            // ---- ws:pro-schema ----
            audio: Default::default(),
            stem_by: Default::default(),
        }
    }

    /// `n` comes from the range, not `project.duration()`; the real run (ffmpeg permitting) lands at
    /// the range's length.
    #[test]
    fn range_export_frame_count() {
        assert_eq!(frame_plan(4.0, 30.0, Some((1.0, 3.0))).unwrap(), (1.0, 2.0, 60));
        assert_eq!(frame_plan(4.0, 30.0, None).unwrap(), (0.0, 4.0, 120));
        // clamped into the project, and an empty/inverted range is an error, not a zero-frame file
        assert_eq!(frame_plan(4.0, 30.0, Some((3.0, 9.0))).unwrap(), (3.0, 1.0, 30));
        assert!(frame_plan(4.0, 30.0, Some((3.0, 1.0))).is_err());
        assert!(frame_plan(0.0, 30.0, None).is_err());

        let dir = temp_dir("range");
        let Some(src) = gen_media(&dir) else {
            eprintln!("ffmpeg missing - skipped");
            return;
        };
        let p = Project::from_media(asset(&src.to_string_lossy(), 2));
        let out = dir.join("range.mp4");
        let opts = ExportOptions { range: Some((1.0, 3.0)), ..opts_for(out.clone()) };
        let text = Arc::new(Mutex::new(TextRasterizer::new()));
        assert_eq!(wait_done(&start_export(p, opts, text)), None);
        let d = probe_duration(&out).expect("probe");
        assert!((d - 2.0).abs() < 0.1, "duration {d}, expected the 2 s range");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn letterbox_pads_mismatched_aspect_preset() {
        let s = scale_vf((1920, 1080), (1080, 1920), "lanczos", true);
        assert!(s.contains("force_original_aspect_ratio=decrease"), "{s}");
        assert!(s.contains("pad=1080:1920"), "{s}");
        // letterbox off, or a matching aspect: the plain scale every export used before
        assert_eq!(scale_vf((1920, 1080), (1080, 1920), "lanczos", false), "scale=1080:1920:flags=lanczos");
        assert_eq!(scale_vf((1920, 1080), (1280, 720), "lanczos", true), "scale=1280:720:flags=lanczos");
        assert_eq!(scale_vf((1920, 1080), (1920, 1080), "lanczos", true), "");
        assert_eq!(scale_vf((320, 240), (160, 120), "", false), "scale=160:120:flags=bicubic");
    }

    #[test]
    fn loudnorm_appends_af_filter_only_with_audio() {
        let args = codec_args("mp4", "auto", 23, "medium", &[]);
        let on = ExportOptions { loudnorm: true, ..opts_for("x.mp4".into()) };
        let (vf, af) = output_filters(&on, (320, 240), true, &args);
        assert_eq!(af, LOUDNORM);
        assert_eq!(vf, "", "no scale, even size: no -vf at all");
        // a silent / video-only project gets no -af
        assert_eq!(output_filters(&on, (320, 240), false, &args).1, "");
        // and off reproduces the pre-change filters byte for byte
        let off = ExportOptions { out_size: Some((160, 120)), ..opts_for("x.mp4".into()) };
        assert_eq!(
            output_filters(&off, (320, 240), true, &args),
            ("scale=160:120:flags=bicubic".into(), String::new())
        );
        // the odd-size guard still chains after the scale
        let odd = ExportOptions { out_size: Some((161, 121)), ..opts_for("x.mp4".into()) };
        assert_eq!(
            output_filters(&odd, (320, 240), false, &args).0,
            "scale=161:121:flags=bicubic,pad=ceil(iw/2)*2:ceil(ih/2)*2"
        );
    }

    #[test]
    fn eta_is_none_at_start_and_decreasing() {
        let elapsed = Duration::from_secs(10);
        assert_eq!(eta_from(elapsed, 0.0), None);
        assert_eq!(eta_from(elapsed, 0.005), None, "below 1 % there is nothing to extrapolate from");
        let e1 = eta_from(elapsed, 0.25).unwrap();
        let e2 = eta_from(elapsed, 0.5).unwrap();
        assert_eq!(e1, Duration::from_secs(30), "25 % done after 10 s → 30 s left");
        assert_eq!(e2, Duration::from_secs(10));
        assert!(e1 > e2, "more done, less left");
        assert_eq!(eta_from(elapsed, 1.0), None);
        let p = Progress::new();
        assert_eq!(p.eta(), None, "a fresh job has no ETA");
        p.set(0.5, "half");
        assert!(p.eta().is_some());
        p.finish(None);
        assert_eq!(p.eta(), None, "a finished job has no ETA");
    }

    // ---- ws:perf-baseline ----
    /// A fixed 10 s 1080p60 two-layer project (a full-frame clip with a half-size copy on top)
    /// exported through the CPU path with libx264 and, when ffmpeg lists one, a hardware H.264 encoder;
    /// prints fps for each. Timing-only, so not a regular test:
    /// `cargo test --release bench_export -- --ignored --nocapture`
    /// ponytail: GPU-path (`FrameSource::Gpu`) is not covered - it needs the UI thread's GL context
    /// serving frames; time it from the app (Export, with the HUD on) until a headless GL path exists.
    #[test]
    #[ignore]
    fn bench_export() {
        use crate::model::Keyframe;
        let dir = std::env::temp_dir().join("se-bench");
        let _ = std::fs::create_dir_all(&dir);
        let src = dir.join("1080p60.mp4");
        let ok = src.exists()
            || std::process::Command::new("ffmpeg")
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=60", "-t", "10"])
                .args(["-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p"])
                .arg(&src)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
        if !ok {
            eprintln!("ffmpeg missing; skipped");
            return;
        }
        let mut p = Project::new();
        (p.width, p.height, p.fps) = (1920, 1080, 60.0);
        let aid = p.add_asset(crate::media::probe(&src.to_string_lossy(), Backend::Auto).expect("probe 1080p60"));
        let _ = p.insert_asset_clips(aid, 0.0, None);
        let top = p.insert_asset_clips(aid, 0.0, None)[0];
        p.clip_mut(top).unwrap().scale.keys = vec![Keyframe { t: 0.0, v: 0.5, ease: Default::default() }];
        let mut encoders = vec!["libx264".to_string()];
        let available = detect_encoders();
        if let Some(hw) = ["h264_nvenc", "h264_qsv", "h264_amf"].into_iter().find(|e| available.iter().any(|a| a == e)) {
            encoders.push(hw.to_string());
        }
        let text = Arc::new(Mutex::new(TextRasterizer::new()));
        for enc in encoders {
            let out = dir.join(format!("bench-{enc}.mp4"));
            let opts = ExportOptions { encoder: enc.clone(), preset: "veryfast".into(), ..opts_for(out) };
            let t0 = std::time::Instant::now();
            let err = wait_done(&start_export(p.clone(), opts, text.clone()));
            let secs = t0.elapsed().as_secs_f64();
            eprintln!("export 10 s 1080p60, 2 layers, CPU path, {enc}: {:.1} fps ({secs:.1} s) {err:?}", 600.0 / secs);
        }
    }
}
