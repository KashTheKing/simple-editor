//! Screen recording and voiceover capture, both through ffmpeg child processes.
//!
//! Screen recording is built so a recording can't be lost:
//! * ffmpeg (`gdigrab`) records **video only** into `<name>.rec.mkv`. Matroska is readable up to the last
//!   cluster written (1 s clusters, a keyframe every 2 s), so a crash, a kill or a power cut keeps
//!   everything up to about a second before it.
//! * Audio comes from `engine::audio_tap` (WASAPI): system audio is loopback of the default output,
//!   the microphone a normal capture stream. Each writes raw f32 to `<name>.system.f32` / `.mic.f32`.
//! * `<name>.rec.json` lists the pieces and their start times. gdigrab stamps frames with the wall
//!   clock (µs since the epoch, read from ffmpeg's `start:` line) and the taps stamp their first sample
//!   on the same clock, so `finalize` lines the audio up exactly.
//! * Stop sends ffmpeg `q` on stdin, waits for it to exit (killing it after `STOP_TIMEOUT`, which the
//!   MKV survives), then `finalize` muxes the pieces into `<name>.mp4` (video copied, audio AAC,
//!   `+faststart`), probes the result and only then deletes the pieces. The app imports the mp4 once the
//!   capture's progress is done without an error - never on "the file exists", which is true from the
//!   first byte (that race is what lost a recording: the import ran before ffmpeg wrote the moov atom).
//! * Pieces left by a crash are found by `unfinished` on the next start and finalised the same way. ffmpeg
//!   is in the app's kill-on-close job (`html::tie_to_us`): it ignores its stdin closing, so after a
//!   crash it would otherwise keep recording the screen until the disk is full.
//!
//! Voiceover: `ffmpeg -f dshow -audio_buffer_size 50 -i audio="<device>" -ac N -ar 48000 out.wav`,
//! started/stopped from the voiceover panel while playback continues; the file is imported and placed at
//! the record start time (plus `Capture::lead_in` when playback rolled under it).

use crate::engine::audio_tap::{AudioTap, TapSource, TapState};
use crate::engine::export::{self, Progress};
use crate::media::ffpipe;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long ffmpeg gets to finish after `q` before it is killed (the MKV is readable either way).
const STOP_TIMEOUT: Duration = Duration::from_secs(20);

/// Which sounds a screen recording keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioMode {
    /// What the computer plays (WASAPI loopback of the default output).
    #[default]
    System,
    Mic,
    Both,
    None,
}

impl AudioMode {
    pub const ALL: [AudioMode; 4] = [AudioMode::System, AudioMode::Mic, AudioMode::Both, AudioMode::None];
    /// The settings value.
    pub fn key(self) -> &'static str {
        match self {
            AudioMode::System => "system",
            AudioMode::Mic => "mic",
            AudioMode::Both => "both",
            AudioMode::None => "none",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            AudioMode::System => "System audio only",
            AudioMode::Mic => "Microphone only",
            AudioMode::Both => "Both",
            AudioMode::None => "None",
        }
    }
    pub fn from_key(k: &str) -> Option<AudioMode> {
        AudioMode::ALL.into_iter().find(|m| m.key() == k)
    }
    pub fn system(self) -> bool {
        matches!(self, AudioMode::System | AudioMode::Both)
    }
    pub fn mic(self) -> bool {
        matches!(self, AudioMode::Mic | AudioMode::Both)
    }
}

/// What gets recorded.
#[derive(Clone, Debug, PartialEq)]
pub enum CaptureSource {
    /// A rectangle of the virtual desktop in physical pixels (a whole monitor, or a typed region).
    Region(i32, i32, u32, u32),
    /// The whole virtual desktop (every monitor).
    Desktop,
    /// One window, by its title (gdigrab `title=`).
    Window(String),
}

#[derive(Clone, Debug)]
pub struct ScreenCaptureOptions {
    /// The finished mp4.
    pub out: PathBuf,
    pub fps: u32,
    /// Video bitrate in kbit/s (0 = use `crf` instead).
    pub bitrate_kbps: u32,
    pub crf: u32,
    pub source: CaptureSource,
    pub audio: AudioMode,
    /// Microphone name ("" = the default input).
    pub mic: String,
    pub cursor: bool,
}

impl Default for ScreenCaptureOptions {
    fn default() -> Self {
        Self {
            out: PathBuf::new(),
            fps: 30,
            bitrate_kbps: 8000,
            crf: 20,
            source: CaptureSource::Desktop,
            audio: AudioMode::System,
            mic: String::new(),
            cursor: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct VoiceoverOptions {
    pub out: PathBuf,
    pub device: String,
    pub sample_rate: u32,
    pub channels: u32,
}

impl Default for VoiceoverOptions {
    fn default() -> Self {
        Self { out: PathBuf::new(), device: String::new(), sample_rate: 48000, channels: 1 }
    }
}

/// One raw audio piece of a recording.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioPart {
    pub path: PathBuf,
    pub rate: u32,
    pub channels: u32,
    /// µs since the Unix epoch of its first sample (0 = nothing was captured).
    pub t0_us: u64,
}

/// `<name>.rec.json`: everything `finalize` needs, also after a crash.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RecParts {
    pub out: PathBuf,
    pub video: PathBuf,
    /// µs since the Unix epoch of the first video frame (0 = unknown, audio then starts with it).
    pub video_t0_us: u64,
    pub audio: AudioMode,
    pub system: Option<AudioPart>,
    pub mic: Option<AudioPart>,
}

/// The pieces' paths for an output `dir/name.mp4`.
pub fn part_paths(out: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let stem = out.with_extension("");
    let with = |ext: &str| PathBuf::from(format!("{}.{ext}", stem.display()));
    (with("rec.json"), with("rec.mkv"), with("system.f32"), with("mic.f32"))
}

/// A running capture. `stop()` finalises the file; dropping it stops too.
pub struct Capture {
    progress: Arc<Progress>,
    /// Shared with the watcher thread so `stop`/`drop` can reach the process from the UI thread.
    child: Arc<Mutex<Option<Child>>>,
    /// Kept open for the whole recording: ffmpeg only accepts `q` while its stdin is a live pipe.
    stdin: Option<ChildStdin>,
    start: Instant,
    /// µs from `start` to the first captured sample, from ffmpeg's progress (`u64::MAX` until it reports).
    lead_us: Arc<AtomicU64>,
    /// Set by `stop`: the watcher then treats any exit status as success.
    graceful: Arc<AtomicBool>,
    /// The screen recording's audio taps (system, mic), for the level meters.
    taps: Vec<(TapSource, Arc<TapState>)>,
}

impl Capture {
    pub fn progress(&self) -> Arc<Progress> {
        self.progress.clone()
    }
    /// Ask ffmpeg to finish; the file is complete (and, for a screen recording, verified) once
    /// `progress.is_done()` without an error.
    pub fn stop(mut self) {
        self.graceful.store(true, Ordering::SeqCst);
        self.quit();
    }
    /// `q` on stdin, then EOF; a process that hasn't exited after `STOP_TIMEOUT` is killed.
    fn quit(&mut self) {
        if let Some(mut si) = self.stdin.take() {
            let _ = si.write_all(b"q\n");
            let _ = si.flush();
        }
        let child = self.child.clone();
        let _ = std::thread::Builder::new().name("capture-stop".into()).spawn(move || {
            let until = Instant::now() + STOP_TIMEOUT;
            while Instant::now() < until {
                std::thread::sleep(Duration::from_millis(100));
                match child.lock().ok().and_then(|mut g| g.as_mut().map(|c| c.try_wait())) {
                    Some(Ok(None)) => continue,
                    _ => return, // exited (or already reaped by the watcher)
                }
            }
            // the whole tree: a package manager's ffmpeg.exe is a shim whose child does the recording
            if let Some(c) = child.lock().ok().as_mut().and_then(|g| g.as_mut()) {
                crate::engine::html::kill_tree(c);
            }
        });
    }
    /// Seconds recorded so far.
    pub fn elapsed(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
    /// Seconds from the process start to the first captured sample: opening a dshow device takes a
    /// while (~0.8 s measured for a USB mic), so a take recorded under rolling playback lands this
    /// much later on the timeline. 0 until ffmpeg's first progress report (~0.5 s in).
    pub fn lead_in(&self) -> f64 {
        match self.lead_us.load(Ordering::Relaxed) {
            u64::MAX => 0.0,
            us => us as f64 / 1e6,
        }
    }
    /// The first audio device error of a running screen recording (it then records silence for it).
    pub fn tap_error(&self) -> Option<String> {
        self.taps.iter().find_map(|(_, t)| t.error())
    }
    /// Peak levels of the screen recording's audio: (system, mic), None for a source not recorded.
    pub fn levels(&self) -> (Option<f32>, Option<f32>) {
        let get = |want: fn(&TapSource) -> bool| self.taps.iter().find(|(s, _)| want(s)).map(|(_, t)| t.peak());
        (get(|s| *s == TapSource::System), get(|s| matches!(s, TapSource::Mic(_))))
    }
}

impl Drop for Capture {
    /// Never a kill: closing the app mid-recording still finalises the MKV (ffmpeg gets `q`), and the
    /// next start's `unfinished` scan turns the pieces into the mp4.
    fn drop(&mut self) {
        if self.stdin.is_some() {
            self.progress.cancel.store(true, Ordering::SeqCst);
            self.quit();
        }
    }
}

pub fn start_voiceover(opts: VoiceoverOptions) -> Result<Capture, String> {
    if opts.device.trim().is_empty() {
        return Err("No input device selected.".into());
    }
    spawn(&opts.out, voice_args(&opts), "Recording")
}

/// Launch ffmpeg with `args` and watch it on a background thread.
fn spawn(out: &std::path::Path, args: Vec<String>, label: &'static str) -> Result<Capture, String> {
    if out.as_os_str().is_empty() {
        return Err("No output file.".into());
    }
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let exe = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let mut cmd = ffpipe::command(&exe);
    cmd.args(&args);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
    // dies with the app: ffmpeg ignores stdin closing, so an orphan would record forever
    crate::engine::html::tie_to_us(&child);

    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let tail = export::stderr_tail(&mut child);
    let progress = Progress::new();
    let graceful = Arc::new(AtomicBool::new(false));
    let shared = Arc::new(Mutex::new(Some(child)));
    let (start, lead_us) = (Instant::now(), Arc::new(AtomicU64::new(u64::MAX)));

    let (p, g, c, l) = (progress.clone(), graceful.clone(), shared.clone(), lead_us.clone());
    let watcher = std::thread::Builder::new().name("capture".into()).spawn(move || {
        p.set(0.0, format!("{label}…"));
        if let Some(out) = stdout {
            // blocks until ffmpeg exits (or is killed) - no polling loop
            for line in std::io::BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                    p.set(0.0, format!("{label} {:.1} s", (us / 1e6).max(0.0)));
                    if let Some(lead) = lead_estimate(start.elapsed(), us) {
                        l.fetch_min(lead, Ordering::Relaxed);
                    }
                }
            }
        }
        let status = c.lock().ok().and_then(|mut g| g.take()).map(|mut ch| ch.wait());
        let msg = tail.and_then(|t| t.join().ok()).unwrap_or_default();
        let graceful = g.load(Ordering::SeqCst);
        p.finish(match status {
            // `q` makes ffmpeg exit non-zero (255) even though the file is finalised
            _ if graceful => None,
            Some(Ok(st)) if st.success() => None,
            _ if p.is_cancelled() => Some(export::CANCELLED.to_string()),
            Some(Ok(st)) if msg.is_empty() => Some(format!("ffmpeg exited with {st}")),
            Some(Ok(_)) => Some(msg),
            Some(Err(e)) => Some(format!("ffmpeg: {e}")),
            None => Some("capture process lost".into()),
        });
    });
    if let Err(e) = watcher {
        if let Ok(mut g) = shared.lock() {
            if let Some(ch) = g.as_mut() {
                let _ = ch.kill();
            }
        }
        return Err(format!("could not start the capture thread: {e}"));
    }
    Ok(Capture { progress, child: shared, stdin, start, lead_us, graceful, taps: Vec::new() })
}

/// Start a screen recording (see the module doc). The returned capture's progress finishes once the
/// mp4 is written and verified, or with a plain-language error.
pub fn start_screen(opts: ScreenCaptureOptions) -> Result<Capture, String> {
    if opts.out.as_os_str().is_empty() {
        return Err("No output file.".into());
    }
    if let Some(dir) = opts.out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let exe = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let (json, video, sys_path, mic_path) = part_paths(&opts.out);
    let mut parts = RecParts { out: opts.out.clone(), video: video.clone(), audio: opts.audio, ..Default::default() };

    // audio first: a tap is running (and stamping its first sample) before the first frame
    let mut taps: Vec<AudioTap> = Vec::new();
    let mut tap_states = Vec::new();
    let mut tap_sources = Vec::new();
    if opts.audio.system() {
        tap_sources.push((TapSource::System, sys_path));
    }
    if opts.audio.mic() {
        tap_sources.push((TapSource::Mic(opts.mic.clone()), mic_path));
    }
    for (src, path) in tap_sources {
        let tap = AudioTap::start(src.clone(), Some(path.clone()));
        tap_states.push((src.clone(), tap.state.clone()));
        let part = AudioPart { path, ..Default::default() };
        match src {
            TapSource::System => parts.system = Some(part),
            TapSource::Mic(_) => parts.mic = Some(part),
        }
        taps.push(tap);
    }
    write_parts(&json, &parts)?;

    let mut cmd = ffpipe::command(&exe);
    cmd.args(video_args(&opts, &video));
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        // tied to the app: a crash must not leave ffmpeg recording forever (it ignores stdin closing);
        // the MKV it leaves is finished by `unfinished` + `finalize` on the next start
        Ok(c) => {
            crate::engine::html::tie_to_us(&c);
            c
        }
        Err(e) => {
            drop(taps);
            cleanup(&parts, &json);
            return Err(format!("ffmpeg: {e}"));
        }
    };
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let video_t0 = Arc::new(AtomicU64::new(0));
    let stderr = child.stderr.take().map(|e| {
        let t0 = video_t0.clone();
        std::thread::spawn(move || {
            let mut tail: Vec<String> = Vec::new();
            for line in std::io::BufReader::new(e).lines().map_while(Result::ok) {
                if t0.load(Ordering::Relaxed) == 0 {
                    if let Some(us) = start_line_us(&line) {
                        t0.store(us, Ordering::SeqCst);
                    }
                }
                tail.push(line);
                if tail.len() > 12 {
                    tail.remove(0);
                }
            }
            tail.join("\n")
        })
    });
    let progress = Progress::new();
    let graceful = Arc::new(AtomicBool::new(false));
    let shared = Arc::new(Mutex::new(Some(child)));
    let (start, lead_us) = (Instant::now(), Arc::new(AtomicU64::new(u64::MAX)));
    let tap_states: Vec<(TapSource, Arc<TapState>)> = tap_states;
    let (p, g, c, l) = (progress.clone(), graceful.clone(), shared.clone(), lead_us.clone());
    let states: Vec<Arc<TapState>> = taps.iter().map(|t| t.state.clone()).collect();
    let watcher = std::thread::Builder::new().name("screen-capture".into()).spawn(move || {
        p.set(0.0, "Recording…");
        let mut noted = false;
        if let Some(out) = stdout {
            for line in std::io::BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                    p.set(0.0, format!("Recording {:.1} s", (us / 1e6).max(0.0)));
                    if let Some(lead) = lead_estimate(start.elapsed(), us) {
                        l.fetch_min(lead, Ordering::Relaxed);
                    }
                    // once every start time is known, the sidecar is complete for crash recovery
                    if !noted && fill_times(&mut parts, video_t0.load(Ordering::SeqCst), &states) {
                        noted = write_parts(&json, &parts).is_ok();
                    }
                }
            }
        }
        let status = c.lock().ok().and_then(|mut g| g.take()).map(|mut ch| ch.wait());
        let log = stderr.and_then(|t| t.join().ok()).unwrap_or_default();
        for t in taps {
            t.stop();
        }
        fill_times(&mut parts, video_t0.load(Ordering::SeqCst), &states);
        let _ = write_parts(&json, &parts);
        let stopped = g.load(Ordering::SeqCst) || p.is_cancelled();
        if !stopped {
            // ffmpeg ended on its own (bad window title, display change...): say why, keep what it wrote
            let why = last_line(&log);
            let ok = matches!(&status, Some(Ok(s)) if s.success());
            if !ok && std::fs::metadata(&parts.video).map(|m| m.len()).unwrap_or(0) == 0 {
                cleanup(&parts, &json);
                p.finish(Some(format!("Screen recording stopped: {why}")));
                return;
            }
        }
        p.set(0.0, "Finishing the recording…");
        p.finish(finalize(&json).err());
    });
    if let Err(e) = watcher {
        if let Ok(mut g) = shared.lock() {
            if let Some(ch) = g.as_mut() {
                let _ = ch.kill();
            }
        }
        return Err(format!("could not start the capture thread: {e}"));
    }
    Ok(Capture { progress, child: shared, stdin, start, lead_us, graceful, taps: tap_states })
}

/// Copy the taps' formats and start times into `parts`; true once every start time is known.
fn fill_times(parts: &mut RecParts, video_t0: u64, states: &[Arc<TapState>]) -> bool {
    parts.video_t0_us = video_t0;
    let mut all = video_t0 > 0;
    let mut it = states.iter();
    for part in [parts.system.as_mut(), parts.mic.as_mut()].into_iter().flatten() {
        let Some(st) = it.next() else { break };
        part.rate = st.rate.load(Ordering::SeqCst);
        part.channels = st.channels.load(Ordering::SeqCst);
        part.t0_us = st.t0_us.load(Ordering::SeqCst);
        all &= part.t0_us > 0;
    }
    all
}

fn write_parts(json: &Path, parts: &RecParts) -> Result<(), String> {
    let s = serde_json::to_string_pretty(parts).map_err(|e| e.to_string())?;
    // write-then-rename, so a crash mid-write never leaves half a sidecar
    let tmp = json.with_extension("json.tmp");
    std::fs::write(&tmp, s).and_then(|_| std::fs::rename(&tmp, json)).map_err(|e| format!("{}: {e}", json.display()))
}

fn cleanup(parts: &RecParts, json: &Path) {
    let _ = std::fs::remove_file(&parts.video);
    for a in [&parts.system, &parts.mic].into_iter().flatten() {
        let _ = std::fs::remove_file(&a.path);
    }
    let _ = std::fs::remove_file(json);
}

/// The wall-clock start of ffmpeg's input, from its `Duration: N/A, start: 1795024956.626457, ...` line.
pub(crate) fn start_line_us(line: &str) -> Option<u64> {
    let rest = line.split("start: ").nth(1)?;
    let v: f64 = rest.split(',').next()?.trim().parse().ok()?;
    (v > 1e9).then(|| (v * 1e6) as u64) // gdigrab's wall clock; anything else is not a usable stamp
}

fn last_line(log: &str) -> String {
    log.lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("Press [q]"))
        .unwrap_or("ffmpeg ended unexpectedly")
        .to_string()
}

/// A recording's sidecar, if it reads.
pub fn read_parts(json: &Path) -> Option<RecParts> {
    std::fs::read_to_string(json).ok().and_then(|s| serde_json::from_str(&s).ok())
}

/// Turn a recording's pieces (`<name>.rec.json`) into the finished mp4: mux, probe, then delete the
/// pieces. On failure the pieces stay and the error says so in plain words.
pub fn finalize(json: &Path) -> Result<PathBuf, String> {
    let parts: RecParts = std::fs::read_to_string(json)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .ok_or_else(|| format!("The recording's notes ({}) can't be read.", json.display()))?;
    let name = parts.out.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    if size(&parts.video) == 0 {
        cleanup(&parts, json);
        return Err(format!("Nothing was recorded for {name}."));
    }
    let has = |a: &Option<AudioPart>| a.as_ref().filter(|a| a.t0_us > 0 && a.rate > 0 && size(&a.path) > 0).cloned();
    let exe = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let Some(recorded) = video_length(&parts.video) else {
        return Err(format!("{name}'s video can't be read. Its pieces are kept next to it."));
    };
    let args = mux_args(&parts, has(&parts.system).as_ref(), has(&parts.mic).as_ref(), recorded);
    let kept = format!(
        "Its pieces are kept next to it ({}); Simple Editor tries again the next time it starts.",
        parts.video.file_name().unwrap_or_default().to_string_lossy()
    );
    let out = ffpipe::command(&exe).args(&args).stdin(Stdio::null()).output().map_err(|e| format!("ffmpeg: {e}"))?;
    if !out.status.success() {
        let why = last_line(&String::from_utf8_lossy(&out.stderr));
        return Err(format!("{name} couldn't be finished ({why}). {kept}"));
    }
    let dur = verify(&parts.out).map_err(|e| format!("{name} was written but doesn't play ({e}). {kept}"))?;
    // the mux copies every video packet, so the lengths match; a big gap means packets were lost
    if recorded > 1.0 && (recorded - dur).abs() > 1.5 {
        return Err(format!("{name} is {dur:.1} s but {recorded:.1} s were recorded. {kept}"));
    }
    cleanup(&parts, json);
    Ok(parts.out)
}

/// The file's duration if ffprobe reads it as a video with a length (an mp4 without its moov atom fails).
pub fn verify(path: &Path) -> Result<f64, String> {
    let a = ffpipe::probe(&path.to_string_lossy())?;
    if a.width == 0 {
        return Err("no video stream".into());
    }
    if a.duration <= 0.0 {
        return Err("no length".into());
    }
    Ok(a.duration)
}

/// How long the recorded video runs: the container's duration, or - for an MKV cut off before its
/// header was rewritten - the end of its last packet.
pub(crate) fn video_length(video: &Path) -> Option<f64> {
    if let Ok(d) = verify(video) {
        return Some(d);
    }
    let exe = ffpipe::ffprobe_exe()?;
    let out = ffpipe::command(&exe)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "packet=pts_time,duration_time", "-of", "csv=p=0"])
        .arg(video)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let end = text
        .lines()
        .filter_map(|l| {
            let mut f = l.split(',').map(|v| v.trim().parse::<f64>().ok());
            Some(f.next()?? + f.next().flatten().unwrap_or(0.0))
        })
        .fold(0.0f64, f64::max);
    (end > 0.0).then_some(end)
}

/// Sidecars of recordings that never finished (a crash, a power cut): every piece untouched for 30 s,
/// so a recording still running in another Simple Editor is never picked up.
pub fn unfinished(dir: &Path) -> Vec<PathBuf> {
    let old = |p: &Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().unwrap_or_default() > Duration::from_secs(30))
            .unwrap_or(true)
    };
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".rec.json") && old(p))
        .filter(|p| {
            let parts: Option<RecParts> = std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok());
            parts.is_some_and(|r| {
                old(&r.video) && [&r.system, &r.mic].into_iter().flatten().all(|a| old(&a.path))
            })
        })
        .collect();
    v.sort();
    v
}

/// When sample 0 was captured, in µs after spawn: wall time elapsed minus media time written. A report
/// that lands between two device buffers overshoots by the wait for the next one, so callers keep the
/// smallest estimate. None before anything was written.
pub(crate) fn lead_estimate(elapsed: Duration, out_time_us: f64) -> Option<u64> {
    (out_time_us > 0.0).then(|| (elapsed.as_micros() as f64 - out_time_us).max(0.0) as u64)
}

fn s(v: impl Into<String>) -> String {
    v.into()
}

/// ffmpeg arguments for the video half of a screen recording: gdigrab into a crash-safe MKV.
pub(crate) fn video_args(o: &ScreenCaptureOptions, video: &Path) -> Vec<String> {
    // info level: the input's `start:` line carries the wall clock of the first frame
    let mut a: Vec<String> =
        ["-y", "-hide_banner", "-nostats", "-loglevel", "info", "-progress", "pipe:1"].iter().map(|x| s(*x)).collect();
    a.extend([s("-f"), s("gdigrab"), s("-framerate"), o.fps.max(1).to_string()]);
    a.extend([s("-draw_mouse"), s(if o.cursor { "1" } else { "0" })]);
    match &o.source {
        CaptureSource::Region(x, y, w, h) => {
            // yuv420p needs even dimensions and gdigrab has no scaler
            let (w, h) = ((*w).max(2) & !1, (*h).max(2) & !1);
            a.extend([s("-offset_x"), x.to_string(), s("-offset_y"), y.to_string()]);
            a.extend([s("-video_size"), format!("{w}x{h}"), s("-i"), s("desktop")]);
        }
        CaptureSource::Desktop => a.extend([s("-i"), s("desktop")]),
        CaptureSource::Window(t) => a.extend([s("-i"), format!("title={t}")]),
    }
    // a window's client area can be any size: trim to even for yuv420p
    a.extend([s("-vf"), s("crop=trunc(iw/2)*2:trunc(ih/2)*2")]);
    // zerolatency: no lookahead and no frame threads, so a frame is on disk right after it is grabbed
    // (x264's default delay on a many-core CPU held every frame of a 3.5 s test recording in memory)
    a.extend([s("-c:v"), s("libx264"), s("-preset"), s("veryfast"), s("-tune"), s("zerolatency"), s("-pix_fmt"), s("yuv420p")]);
    if o.bitrate_kbps > 0 {
        a.extend([s("-b:v"), format!("{}k", o.bitrate_kbps)]);
    } else {
        a.extend([s("-crf"), o.crf.clamp(0, 51).to_string()]);
    }
    // a keyframe every 2 s and 1 s clusters: a cut-off file loses about a second, never the recording
    // and every packet goes to disk as it is muxed: a quiet screen makes so few bytes that ffmpeg's
    // write buffer would otherwise hold seconds of it (a killed test recording was 0 bytes)
    a.extend([s("-g"), (o.fps.max(1) * 2).to_string(), s("-cluster_time_limit"), s("1000"), s("-flush_packets"), s("1")]);
    a.extend([s("-f"), s("matroska"), video.to_string_lossy().into_owned()]);
    a
}

/// ffmpeg arguments that mux a recording's pieces into its mp4. `system` / `mic` are the pieces that
/// hold sound (None = not recorded, or silent the whole time); a mode that wants sound but got none
/// still gets a silent track, so every recording made with audio has an audio stream.
/// `length` is the video's length in seconds: the output is cut there (`-shortest` never ends a graph with
/// an endless `apad` / `anullsrc` in it).
pub(crate) fn mux_args(p: &RecParts, system: Option<&AudioPart>, mic: Option<&AudioPart>, length: f64) -> Vec<String> {
    let mut a: Vec<String> = ["-y", "-hide_banner", "-loglevel", "error"].iter().map(|x| s(*x)).collect();
    a.extend([s("-i"), p.video.to_string_lossy().into_owned()]);
    let mut chains = Vec::new();
    for part in [system, mic].into_iter().flatten() {
        let i = chains.len() + 1;
        a.extend([s("-f"), s("f32le"), s("-ar"), part.rate.to_string(), s("-ac"), part.channels.max(1).to_string()]);
        a.extend([s("-i"), part.path.to_string_lossy().into_owned()]);
        // audio that started after the first frame waits; audio from before it is trimmed
        let lead = if p.video_t0_us == 0 { 0 } else { part.t0_us as i64 - p.video_t0_us as i64 };
        let shift = match lead {
            d if d > 0 => format!("adelay={}:all=1,", d / 1000),
            d if d < 0 => format!("atrim=start={:.6},asetpts=PTS-STARTPTS,", -d as f64 / 1e6),
            _ => String::new(),
        };
        chains.push(format!("[{i}:a]{shift}aresample=48000,aformat=channel_layouts=stereo[a{i}]"));
    }
    let wants_audio = p.audio != AudioMode::None;
    if chains.is_empty() && wants_audio {
        a.extend([s("-f"), s("lavfi"), s("-i"), s("anullsrc=r=48000:cl=stereo")]);
        chains.push(s("[1:a]anull[a1]"));
    }
    let mut graph = chains.join(";");
    match chains.len() {
        0 => {}
        1 => graph.push_str(";[a1]apad[aout]"),
        _ => graph.push_str(";[a1][a2]amix=inputs=2:duration=longest:normalize=0,apad[aout]"),
    }
    a.extend([s("-map"), s("0:v")]);
    if !chains.is_empty() {
        a.extend([s("-filter_complex"), graph, s("-map"), s("[aout]"), s("-c:a"), s("aac"), s("-b:a"), s("192k")]);
        // apad makes the audio endless; the video's end is the recording's end
        a.extend([s("-t"), format!("{length:.3}")]);
    }
    a.extend([s("-c:v"), s("copy"), s("-movflags"), s("+faststart"), p.out.to_string_lossy().into_owned()]);
    a
}

/// ffmpeg arguments for a voiceover take (WAV, so a cut-off take is still readable).
pub(crate) fn voice_args(o: &VoiceoverOptions) -> Vec<String> {
    let mut a: Vec<String> =
        ["-y", "-hide_banner", "-loglevel", "error", "-progress", "pipe:1"].iter().map(|x| s(*x)).collect();
    // 50 ms device buffers instead of dshow's default 500 ms: progress then trails the capture by at most
    // one small buffer, which keeps `Capture::lead_in` within ~50 ms (measured: 0.76-0.80 s vs 0.80-0.86 s)
    a.extend([s("-f"), s("dshow"), s("-audio_buffer_size"), s("50"), s("-i"), format!("audio={}", o.device.trim())]);
    a.extend([s("-ac"), o.channels.clamp(1, 2).to_string()]);
    a.extend([s("-ar"), if o.sample_rate == 0 { s("48000") } else { o.sample_rate.to_string() }]);
    a.push(o.out.to_string_lossy().into_owned());
    a
}

/// Names that only ever belong to a loopback / "what you hear" input.
const LOOPBACK_HINTS: [&str; 10] = [
    "stereo mix",
    "what u hear",
    "wave out",
    "loopback",
    "cable output",
    "vb-audio",
    "voicemeeter",
    "virtual audio",
    "monitor of",
    "speakers",
];

/// dshow audio input devices (`ffmpeg -list_devices true -f dshow -i dummy`), cached.
/// The bool marks devices that look like desktop/loopback capture.
///
/// ponytail: probed once per process - the UI polls this every frame while the recorder/settings
/// windows are open and each probe spawns ffmpeg (~250 ms), so it must not re-run per frame. A device
/// plugged in (or Stereo Mix enabled) mid-session therefore needs a restart; swap the `OnceLock` for a
/// `Mutex<Option<_>>` when there is a "Refresh" button next to the device combos to clear it.
pub fn audio_devices() -> Vec<(String, bool)> {
    static CACHE: OnceLock<Vec<(String, bool)>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let Some(exe) = ffpipe::ffmpeg_exe() else { return Vec::new() };
            // listing devices always "fails" (dummy is not a real device); the list is on stderr
            match ffpipe::command(&exe)
                .args(["-hide_banner", "-list_devices", "true", "-f", "dshow", "-i", "dummy"])
                .stdin(Stdio::null())
                .output()
            {
                Ok(o) => parse_devices(&String::from_utf8_lossy(&o.stderr)),
                Err(_) => Vec::new(),
            }
        })
        .clone()
}

/// Audio device names out of ffmpeg's `-list_devices` output (both the modern `"name" (audio)` layout
/// and the older `DirectShow audio devices` section headers).
pub(crate) fn parse_devices(stderr: &str) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut in_audio = false;
    for raw in stderr.lines() {
        // strip the "[dshow @ 0000...] " log prefix
        let line = match (raw.trim_start().starts_with('['), raw.find("] ")) {
            (true, Some(i)) => &raw[i + 2..],
            _ => raw,
        }
        .trim();
        let lower = line.to_ascii_lowercase();
        if lower.contains("directshow audio devices") {
            in_audio = true;
            continue;
        }
        if lower.contains("directshow video devices") {
            in_audio = false;
            continue;
        }
        if lower.starts_with("alternative name") {
            continue;
        }
        let Some(rest) = line.strip_prefix('"') else { continue };
        let Some(end) = rest.find('"') else { continue };
        let name = &rest[..end];
        let tail = rest[end + 1..].trim().to_ascii_lowercase();
        let is_audio = if tail.contains("(audio)") {
            true
        } else if tail.contains("(video)") {
            false
        } else {
            in_audio
        };
        if !is_audio || name.is_empty() || out.iter().any(|(n, _)| n == name) {
            continue;
        }
        let l = name.to_ascii_lowercase();
        out.push((name.to_string(), LOOPBACK_HINTS.iter().any(|h| l.contains(h))));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `ffmpeg -list_devices true -f dshow -i dummy` output (ffmpeg 7.1, Windows 11), including the
    /// chatter some virtual-camera drivers print before the listing.
    const LISTING: &str = r#"I2026-08-21 19:11:27.057487 (26856) [INFO] [VCAMDS] ffmpeg.exe
E2026-08-21 19:11:27.074296 (26856)  [ERR] [VCAMDS] Failed to open NBX hive
[dshow @ 000001e0c1b0f900] "Integrated Camera" (video)
[dshow @ 000001e0c1b0f900]   Alternative name "@device_pnp_\\?\usb#vid_04f2&pid_b6d9&mi_00#6&1e1b7d3f&0&0000#{65e8773d-8f56-11d0-a3b9-00a0c9223196}\global"
[dshow @ 000001e0c1b0f900] "OBS Virtual Camera" (video)
[dshow @ 000001e0c1b0f900]   Alternative name "@device_sw_{860BB310-5D01-11D0-BD3B-00A0C911CE86}\{A3FCE0F5-3493-419F-958A-ABA1250EC20B}"
[dshow @ 000001e0c1b0f900] "Microphone (Realtek(R) Audio)" (audio)
[dshow @ 000001e0c1b0f900]   Alternative name "@device_cm_{33D9A762-90C8-11D0-BD43-00A0C911CE86}\wave_{B1F4F9E6-1A0C-4E0D-9C6F-2A9C3D5E7A11}"
[dshow @ 000001e0c1b0f900] "Stereo Mix (Realtek(R) Audio)" (audio)
[dshow @ 000001e0c1b0f900]   Alternative name "@device_cm_{33D9A762-90C8-11D0-BD43-00A0C911CE86}\wave_{2F0E1A22-7C3B-4A55-8D9E-0C1B2D3E4F50}"
[dshow @ 000001e0c1b0f900] "CABLE Output (VB-Audio Virtual Cable)" (audio)
[in#0 @ 000001e0c1b0e2c0] Error opening input: Immediate exit requested
Error opening input file dummy.
"#;

    /// The pre-4.4 layout: section headers instead of a "(audio)" suffix.
    const OLD_LISTING: &str = r#"[dshow @ 0000021d] DirectShow video devices (some may be both video and audio devices)
[dshow @ 0000021d]  "Integrated Camera"
[dshow @ 0000021d]     Alternative name "@device_pnp_\\?\usb#vid_04f2"
[dshow @ 0000021d] DirectShow audio devices
[dshow @ 0000021d]  "Microphone (2- USB Audio Device)"
[dshow @ 0000021d]     Alternative name "@device_cm_{33D9A762}\wave_{7C0EF}"
"#;

    #[test]
    fn devices_parse_audio_only_and_mark_loopback() {
        let d = parse_devices(LISTING);
        let names: Vec<&str> = d.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Microphone (Realtek(R) Audio)",
                "Stereo Mix (Realtek(R) Audio)",
                "CABLE Output (VB-Audio Virtual Cable)"
            ]
        );
        assert_eq!(d.iter().map(|(_, lb)| *lb).collect::<Vec<_>>(), vec![false, true, true]);
        // the "Alternative name" lines and both cameras are never offered as inputs
        assert!(!names.iter().any(|n| n.contains("device_") || n.contains("Camera")));

        let old = parse_devices(OLD_LISTING);
        assert_eq!(old, vec![("Microphone (2- USB Audio Device)".to_string(), false)]);
        assert!(parse_devices("").is_empty());
    }

    fn opts() -> ScreenCaptureOptions {
        ScreenCaptureOptions { out: PathBuf::from(r"C:\rec\a.mp4"), ..Default::default() }
    }

    /// Args as one string, so tests can look for "-flag value" pairs.
    fn joined(a: &[String]) -> String {
        a.join(" ")
    }

    #[test]
    fn voice_args_wav() {
        let o = VoiceoverOptions { out: PathBuf::from(r"C:\rec\vo.wav"), device: " Mic ".into(), ..Default::default() };
        let j = joined(&voice_args(&o));
        assert!(j.contains("-f dshow -audio_buffer_size 50 -i audio=Mic"), "device name is trimmed: {j}");
        assert!(j.contains("-ac 1 -ar 48000") && j.ends_with(r"C:\rec\vo.wav"), "{j}");
        let clamped = voice_args(&VoiceoverOptions { channels: 7, sample_rate: 0, ..o });
        assert!(joined(&clamped).contains("-ac 2 -ar 48000"), "channels are clamped to stereo");
    }

    #[test]
    fn lead_in_is_wall_time_minus_media_time() {
        let ms = Duration::from_millis;
        assert_eq!(lead_estimate(ms(1300), 500_000.0), Some(800_000), "0.5 s written 1.3 s after spawn");
        assert_eq!(lead_estimate(ms(200), 500_000.0), Some(0), "never negative");
        assert_eq!(lead_estimate(ms(900), 0.0), None, "nothing written yet says nothing");
    }


    fn parts(audio: AudioMode) -> RecParts {
        RecParts {
            out: PathBuf::from(r"C:\rec\a.mp4"),
            video: PathBuf::from(r"C:\rec\a.rec.mkv"),
            video_t0_us: 1_000_000_000_000,
            audio,
            ..Default::default()
        }
    }

    fn part(name: &str, t0_us: u64) -> AudioPart {
        AudioPart { path: PathBuf::from(format!(r"C:\rec\a.{name}.f32")), rate: 44_100, channels: 2, t0_us }
    }

    #[test]
    fn video_args_crash_safe_mkv() {
        let o = ScreenCaptureOptions { out: PathBuf::from(r"C:\rec\a.mp4"), ..Default::default() };
        let j = joined(&video_args(&o, Path::new(r"C:\rec\a.rec.mkv")));
        assert!(j.contains("-f gdigrab -framerate 30 -draw_mouse 1 -i desktop"), "{j}");
        assert!(j.ends_with(r"-f matroska C:\rec\a.rec.mkv"), "video goes to the MKV, never straight to mp4: {j}");
        assert!(j.contains("-g 60 -cluster_time_limit 1000 -flush_packets 1"), "{j}");
        assert!(j.contains("-loglevel info"), "the start: line is needed for A/V sync: {j}");
        assert!(!j.contains("dshow") && !j.contains("-c:a"), "ffmpeg records no audio itself: {j}");
        assert!(j.contains("-b:v 8000k") && !j.contains("-crf"), "{j}");
        assert!(j.contains("-tune zerolatency"), "frames reach the disk as they are grabbed: {j}");
        let j = joined(&video_args(&ScreenCaptureOptions { bitrate_kbps: 0, crf: 23, cursor: false, ..o.clone() }, Path::new("v.mkv")));
        assert!(j.contains("-crf 23") && j.contains("-draw_mouse 0"), "{j}");
        let j = joined(&video_args(&ScreenCaptureOptions { source: CaptureSource::Region(-1920, 50, 641, 481), ..o.clone() }, Path::new("v.mkv")));
        assert!(j.contains("-offset_x -1920 -offset_y 50 -video_size 640x480 -i desktop"), "{j}");
        let j = joined(&video_args(&ScreenCaptureOptions { source: CaptureSource::Window("Notepad".into()), ..o }, Path::new("v.mkv")));
        assert!(j.contains("-i title=Notepad") && !j.contains("-offset_x"), "{j}");
    }

    #[test]
    fn mux_args_per_audio_mode() {
        // None: video only, no audio codec
        let j = joined(&mux_args(&parts(AudioMode::None), None, None, 12.5));
        assert!(j.contains("-map 0:v -c:v copy -movflags +faststart C:\\rec\\a.mp4"), "{j}");
        assert!(!j.contains("-c:a") && !j.contains("f32le") && !j.contains("-t "), "{j}");

        // System only, started 250 ms after the first frame: delayed, padded, ends with the video
        let sys = part("system", 1_000_000_250_000);
        let j = joined(&mux_args(&parts(AudioMode::System), Some(&sys), None, 12.5));
        assert!(j.contains(r"-f f32le -ar 44100 -ac 2 -i C:\rec\a.system.f32"), "{j}");
        assert!(j.contains("[1:a]adelay=250:all=1,aresample=48000"), "{j}");
        assert!(j.contains("[a1]apad[aout]") && !j.contains("amix"), "{j}");
        assert!(j.contains("-map 0:v -filter_complex") && j.contains("-map [aout] -c:a aac"), "{j}");
        assert!(j.contains("-c:a aac -b:a 192k -t 12.500") && !j.contains("-shortest"), "cut at the video's end: {j}");

        // Microphone only, started 0.5 s BEFORE the first frame: the lead is trimmed off
        let mic = AudioPart { channels: 1, ..part("mic", 999_999_500_000) };
        let j = joined(&mux_args(&parts(AudioMode::Mic), None, Some(&mic), 12.5));
        assert!(j.contains(r"-ac 1 -i C:\rec\a.mic.f32"), "{j}");
        assert!(j.contains("[1:a]atrim=start=0.500000,asetpts=PTS-STARTPTS,aresample"), "{j}");
        assert!(j.contains("channel_layouts=stereo[a1]"), "mono mic becomes stereo: {j}");

        // Both: two inputs, mixed without halving either
        let j = joined(&mux_args(&parts(AudioMode::Both), Some(&sys), Some(&mic), 12.5));
        assert_eq!(j.matches("-f f32le").count(), 2, "{j}");
        assert!(j.contains("[a1][a2]amix=inputs=2:duration=longest:normalize=0,apad[aout]"), "{j}");
        assert!(j.contains("[2:a]atrim"), "the mic is input 2 when both are recorded: {j}");

        // System audio wanted but nothing played the whole time: a silent track, not a missing one
        let j = joined(&mux_args(&parts(AudioMode::System), None, None, 12.5));
        assert!(j.contains("-f lavfi -i anullsrc=r=48000:cl=stereo") && j.contains("-c:a aac"), "{j}");

        // unknown video start (ffmpeg never printed it): audio starts with the video
        let j = joined(&mux_args(&RecParts { video_t0_us: 0, ..parts(AudioMode::System) }, Some(&sys), None, 12.5));
        assert!(j.contains("[1:a]aresample"), "{j}");
    }

    #[test]
    fn audio_modes_round_trip() {
        for m in AudioMode::ALL {
            assert_eq!(AudioMode::from_key(m.key()), Some(m));
        }
        assert_eq!(AudioMode::from_key("?"), None);
        assert!(AudioMode::System.system() && !AudioMode::System.mic());
        assert!(AudioMode::Mic.mic() && !AudioMode::Mic.system());
        assert!(AudioMode::Both.mic() && AudioMode::Both.system());
        assert!(!AudioMode::None.mic() && !AudioMode::None.system());
    }

    #[test]
    fn start_line_is_the_wall_clock() {
        let l = "  Duration: N/A, start: 1795024956.626457, bitrate: 24580 kb/s";
        assert_eq!(start_line_us(l), Some(1_795_024_956_626_457));
        assert_eq!(start_line_us("  Duration: 00:00:01.00, start: 0.000000, bitrate: 1 kb/s"), None);
        assert_eq!(start_line_us("frame=  10"), None);
    }

    #[test]
    fn part_paths_sit_beside_the_mp4() {
        let (j, v, s, m) = part_paths(Path::new(r"C:\rec\screen-1.mp4"));
        assert_eq!(j, PathBuf::from(r"C:\rec\screen-1.rec.json"));
        assert_eq!(v, PathBuf::from(r"C:\rec\screen-1.rec.mkv"));
        assert_eq!(s, PathBuf::from(r"C:\rec\screen-1.system.f32"));
        assert_eq!(m, PathBuf::from(r"C:\rec\screen-1.mic.f32"));
    }

    /// Where the real-recording tests write: `SE_TEST_DIR`, else the temp dir.
    fn test_dir(tag: &str) -> Option<PathBuf> {
        ffpipe::ffmpeg_exe()?;
        let d = std::env::var_os("SE_TEST_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let d = d.join(format!("se-capture-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).ok()?;
        Some(d)
    }

    fn wait_done(p: &Progress) {
        for _ in 0..300 {
            if p.is_done() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn record(dir: &Path, audio: AudioMode) -> (Capture, PathBuf) {
        let out = dir.join("rec.mp4");
        let cap = start_screen(ScreenCaptureOptions {
            out: out.clone(),
            fps: 15,
            source: CaptureSource::Region(0, 0, 320, 240),
            audio,
            ..Default::default()
        })
        .expect("start");
        (cap, out)
    }

    /// Real gdigrab (needs ffmpeg and a desktop session; skips without ffmpeg): stop with `q`, and the
    /// mp4 has its moov atom, probes, and lasts as long as the recording ran. The pieces are gone.
    #[test]
    fn stop_then_probe_has_moov_and_duration() {
        let Some(dir) = test_dir("stop") else { return };
        let (cap, out) = record(&dir, AudioMode::System);
        let prog = cap.progress();
        // what ffmpeg says it has recorded ("Recording 3.4 s"); a loaded test run can take seconds to open
        // gdigrab, so the wall clock is no measure
        let recorded = |p: &Progress| -> f64 {
            p.status().strip_prefix("Recording ").and_then(|r| r.trim_end_matches(" s").parse().ok()).unwrap_or(0.0)
        };
        for _ in 0..200 {
            if recorded(&prog) > 0.0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        std::thread::sleep(Duration::from_millis(3000));
        let at_stop = recorded(&prog);
        assert!(at_stop > 2.0, "recording: {}", prog.status());
        cap.stop();
        wait_done(&prog);
        assert!(prog.is_done(), "finished: {}", prog.status());
        assert_eq!(prog.error(), None);
        let bytes = std::fs::read(&out).expect("the mp4 exists once the progress is done");
        assert!(bytes.windows(4).any(|w| w == b"moov"), "moov atom written");
        let dur = verify(&out).expect("probes");
        // the stop lands up to one progress report (0.5 s) plus the frames in flight after `at_stop`
        assert!(dur >= at_stop - 0.2 && dur <= at_stop + 1.5, "{dur} s, {at_stop} s recorded when stopped");
        let a = ffpipe::probe(&out.to_string_lossy()).unwrap();
        assert_eq!(a.audio_streams.len(), 1, "system-audio mode always has an audio track");
        let (j, v, s, _) = part_paths(&out);
        assert!(!j.exists() && !v.exists() && !s.exists(), "pieces removed after a verified finish");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ffmpeg killed mid-recording (no `q`, no trailer): the MKV still remuxes into a playable mp4.
    #[test]
    fn killed_mid_recording_still_plays() {
        let Some(dir) = test_dir("kill") else { return };
        let (cap, out) = record(&dir, AudioMode::None);
        let prog = cap.progress();
        std::thread::sleep(Duration::from_millis(3500));
        // the real ffmpeg, not just a package manager's shim in front of it
        if let Some(c) = cap.child.lock().unwrap().as_mut() {
            crate::engine::html::kill_tree(c);
        }
        wait_done(&prog);
        assert_eq!(prog.error(), None, "the killed recording is finished from its MKV");
        let dur = verify(&out).expect("plays");
        assert!((1.5..4.5).contains(&dur), "kept most of the ~3.5 s recording: {dur} s");
        drop(cap);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The crash path: pieces left behind (the app died before finalising) are found by `unfinished`
    /// once they are old enough and finalised into a playable mp4.
    #[test]
    fn leftover_pieces_are_recovered() {
        let Some(dir) = test_dir("crash") else { return };
        let out = dir.join("crash.mp4");
        let (json, video, _, _) = part_paths(&out);
        let ff = ffpipe::ffmpeg_exe().unwrap();
        // a 2 s MKV cut off mid-cluster, the way a power cut leaves it
        let ok = ffpipe::command(&ff)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=15", "-t", "2"])
            .args(["-c:v", "libx264", "-preset", "ultrafast", "-g", "30", "-f", "matroska"])
            .arg(&video)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let len = std::fs::metadata(&video).unwrap().len();
        std::fs::OpenOptions::new().write(true).open(&video).unwrap().set_len(len * 9 / 10).unwrap();
        write_parts(&json, &RecParts { out: out.clone(), video: video.clone(), audio: AudioMode::None, ..Default::default() })
            .unwrap();
        assert!(unfinished(&dir).is_empty(), "fresh pieces may belong to a running recording");
        let old = std::time::SystemTime::now() - Duration::from_secs(120);
        for p in [&json, &video] {
            std::fs::File::options().write(true).open(p).unwrap().set_modified(old).unwrap();
        }
        assert_eq!(unfinished(&dir), vec![json.clone()]);
        assert_eq!(finalize(&json), Ok(out.clone()));
        assert!(verify(&out).unwrap() > 1.0);
        assert!(!json.exists() && !video.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_inputs_are_refused() {
        assert!(start_voiceover(VoiceoverOptions::default()).is_err());
        assert!(start_screen(ScreenCaptureOptions::default()).is_err());
    }
}
