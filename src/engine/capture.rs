//! Screen recording and voiceover capture, both through ffmpeg child processes (no new deps).
//!
//! Screen: `ffmpeg -f gdigrab -framerate FPS [-offset_x X -offset_y Y -video_size WxH] -i desktop
//! [-f dshow -i audio="<mic>"] -c:v libx264 -preset veryfast <rate args> out.mp4`. Desktop audio needs a
//! loopback dshow device (see `audio_devices`). `auto_on_blur` (settings) starts the recorder when the
//! editor loses focus and stops when it regains focus, so another app can be recorded hands-free.
//!
//! Voiceover: `ffmpeg -f dshow -i audio="<device>" -ac N -ar 48000 out.wav`, started/stopped from the
//! voiceover panel while playback continues; the file is imported and placed at the record start time.
//!
//! Both report through `engine::export::Progress` and stop cleanly (ffmpeg gets `q` on stdin, so the
//! container is finalised).

use crate::engine::export::{self, Progress};
use crate::media::ffpipe;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct ScreenCaptureOptions {
    pub out: PathBuf,
    pub fps: u32,
    /// Video bitrate in kbit/s (0 = use `crf` instead).
    pub bitrate_kbps: u32,
    pub crf: u32,
    /// None = whole desktop; Some((x, y, w, h)) = a region in desktop pixels.
    pub region: Option<(i32, i32, u32, u32)>,
    /// dshow device name for the microphone (empty = none).
    pub mic: String,
    pub desktop_audio: bool,
    pub cursor: bool,
}

impl Default for ScreenCaptureOptions {
    fn default() -> Self {
        Self {
            out: PathBuf::new(),
            fps: 30,
            bitrate_kbps: 8000,
            crf: 20,
            region: None,
            mic: String::new(),
            desktop_audio: false,
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

/// A running capture. `stop()` finalises the file; dropping it stops too.
pub struct Capture {
    progress: Arc<Progress>,
    /// Shared with the watcher thread so `stop`/`drop` can reach the process from the UI thread.
    child: Arc<Mutex<Option<Child>>>,
    /// Kept open for the whole recording: ffmpeg only accepts `q` while its stdin is a live pipe.
    stdin: Option<ChildStdin>,
    start: Instant,
    /// Set by `stop`: the watcher then treats any exit status as success and `drop` must not kill.
    graceful: Arc<AtomicBool>,
}

impl Capture {
    pub fn progress(&self) -> Arc<Progress> {
        self.progress.clone()
    }
    /// Ask ffmpeg to finish; the file is complete once `progress.is_done()`.
    pub fn stop(mut self) {
        self.graceful.store(true, Ordering::SeqCst);
        if let Some(mut si) = self.stdin.take() {
            let _ = si.write_all(b"q\n");
            let _ = si.flush();
        }
        // `self` drops here: stdin is closed (EOF is ffmpeg's second exit cue) and the watcher thread
        // waits for the process, finalises the container and finishes the progress.
    }
    /// Seconds recorded so far.
    pub fn elapsed(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if self.graceful.load(Ordering::SeqCst) {
            return;
        }
        self.progress.cancel.store(true, Ordering::SeqCst);
        if let Ok(mut g) = self.child.lock() {
            if let Some(c) = g.as_mut() {
                let _ = c.kill();
            }
        }
    }
}

pub fn start_screen(opts: ScreenCaptureOptions) -> Result<Capture, String> {
    let loopback = if opts.desktop_audio { loopback_device() } else { None };
    if opts.desktop_audio && loopback.is_none() {
        // ponytail: no WASAPI loopback of our own — a dshow loopback driver is the only route
        return Err("No desktop-audio device found. Enable \"Stereo Mix\" in Windows sound settings \
                    (or install a loopback driver) and reopen the recorder."
            .into());
    }
    spawn(&opts.out, screen_args(&opts, loopback.as_deref()), "Recording")
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

    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let tail = export::stderr_tail(&mut child);
    let progress = Progress::new();
    let graceful = Arc::new(AtomicBool::new(false));
    let shared = Arc::new(Mutex::new(Some(child)));

    let (p, g, c) = (progress.clone(), graceful.clone(), shared.clone());
    let watcher = std::thread::Builder::new().name("capture".into()).spawn(move || {
        p.set(0.0, format!("{label}…"));
        if let Some(out) = stdout {
            // blocks until ffmpeg exits (or is killed) — no polling loop
            for line in std::io::BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                    p.set(0.0, format!("{label} {:.1} s", (us / 1e6).max(0.0)));
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
    Ok(Capture { progress, child: shared, stdin, start: Instant::now(), graceful })
}

fn s(v: impl Into<String>) -> String {
    v.into()
}

/// ffmpeg arguments for a screen recording. `loopback` is the dshow device used for desktop audio
/// (already resolved by the caller); mic and loopback are mixed when both are present.
pub(crate) fn screen_args(o: &ScreenCaptureOptions, loopback: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> =
        ["-y", "-hide_banner", "-loglevel", "error", "-progress", "pipe:1"].iter().map(|x| s(*x)).collect();
    a.extend([s("-f"), s("gdigrab"), s("-framerate"), o.fps.max(1).to_string()]);
    a.extend([s("-draw_mouse"), s(if o.cursor { "1" } else { "0" })]);
    if let Some((x, y, w, h)) = o.region {
        // yuv420p needs even dimensions and gdigrab has no scaler
        let (w, h) = (w.max(2) & !1, h.max(2) & !1);
        a.extend([s("-offset_x"), x.to_string(), s("-offset_y"), y.to_string()]);
        a.extend([s("-video_size"), format!("{w}x{h}")]);
    }
    a.extend([s("-i"), s("desktop")]);

    let mic = o.mic.trim();
    let loop_dev = loopback.map(str::trim).filter(|d| !d.is_empty() && o.desktop_audio);
    let mut inputs = 0;
    for dev in [(!mic.is_empty()).then_some(mic), loop_dev].into_iter().flatten() {
        a.extend([s("-f"), s("dshow"), s("-i"), format!("audio={dev}")]);
        inputs += 1;
    }
    match inputs {
        0 => a.extend([s("-map"), s("0:v")]),
        1 => a.extend([s("-map"), s("0:v"), s("-map"), s("1:a")]),
        _ => a.extend([
            s("-filter_complex"),
            s("[1:a][2:a]amix=inputs=2:duration=longest[aout]"),
            s("-map"),
            s("0:v"),
            s("-map"),
            s("[aout]"),
        ]),
    }
    a.extend([s("-c:v"), s("libx264"), s("-preset"), s("veryfast"), s("-pix_fmt"), s("yuv420p")]);
    if o.bitrate_kbps > 0 {
        a.extend([s("-b:v"), format!("{}k", o.bitrate_kbps)]);
    } else {
        a.extend([s("-crf"), o.crf.clamp(0, 51).to_string()]);
    }
    if inputs > 0 {
        a.extend([s("-c:a"), s("aac"), s("-b:a"), s("192k")]);
    }
    a.push(o.out.to_string_lossy().into_owned());
    a
}

/// ffmpeg arguments for a voiceover take (WAV, so a cut-off take is still readable).
pub(crate) fn voice_args(o: &VoiceoverOptions) -> Vec<String> {
    let mut a: Vec<String> =
        ["-y", "-hide_banner", "-loglevel", "error", "-progress", "pipe:1"].iter().map(|x| s(*x)).collect();
    a.extend([s("-f"), s("dshow"), s("-i"), format!("audio={}", o.device.trim())]);
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

/// The first device that looks like desktop audio, if any.
pub fn loopback_device() -> Option<String> {
    audio_devices().into_iter().find(|(_, lb)| *lb).map(|(n, _)| n)
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
    fn screen_args_desktop_bitrate() {
        let a = screen_args(&opts(), None);
        let j = joined(&a);
        assert!(j.contains("-f gdigrab -framerate 30"), "{j}");
        assert!(j.contains("-draw_mouse 1") && j.ends_with(r"C:\rec\a.mp4"), "{j}");
        assert!(j.contains("-i desktop") && j.contains("-map 0:v"));
        assert!(j.contains("-b:v 8000k") && !j.contains("-crf"), "{j}");
        assert!(!j.contains("dshow") && !j.contains("-c:a"), "no audio inputs → no audio codec: {j}");
        assert!(j.contains("-c:v libx264 -preset veryfast -pix_fmt yuv420p"), "{j}");
        assert!(!j.contains("-offset_x"), "whole desktop takes no offset");
    }

    #[test]
    fn screen_args_crf_when_bitrate_zero() {
        let o = ScreenCaptureOptions { bitrate_kbps: 0, crf: 23, cursor: false, ..opts() };
        let j = joined(&screen_args(&o, None));
        assert!(j.contains("-crf 23") && !j.contains("-b:v"), "{j}");
        assert!(j.contains("-draw_mouse 0"), "{j}");
    }

    #[test]
    fn screen_args_region_is_even_sized() {
        let o = ScreenCaptureOptions { region: Some((100, 50, 641, 481)), ..opts() };
        let j = joined(&screen_args(&o, None));
        assert!(j.contains("-offset_x 100 -offset_y 50 -video_size 640x480"), "{j}");
        // negative offsets (a monitor left of the primary) survive
        let o = ScreenCaptureOptions { region: Some((-1920, 0, 1920, 1080)), ..opts() };
        assert!(joined(&screen_args(&o, None)).contains("-offset_x -1920"));
    }

    #[test]
    fn screen_args_mic_only_maps_one_audio_input() {
        let o = ScreenCaptureOptions { mic: "Microphone (Realtek(R) Audio)".into(), ..opts() };
        let j = joined(&screen_args(&o, None));
        assert!(j.contains("-f dshow -i audio=Microphone (Realtek(R) Audio)"), "{j}");
        assert!(j.contains("-map 0:v -map 1:a") && !j.contains("amix"), "{j}");
        assert!(j.contains("-c:a aac"), "{j}");
        // desktop audio requested but no loopback device → the mic stays the only input
        let want = screen_args(&ScreenCaptureOptions { desktop_audio: true, ..o.clone() }, None);
        assert_eq!(want, screen_args(&o, None));
    }

    #[test]
    fn screen_args_mic_plus_desktop_mixes() {
        let o = ScreenCaptureOptions { mic: "Mic".into(), desktop_audio: true, ..opts() };
        let j = joined(&screen_args(&o, Some("Stereo Mix")));
        assert_eq!(j.matches("-f dshow").count(), 2, "{j}");
        assert!(j.contains("-i audio=Mic ") && j.contains("-i audio=Stereo Mix"), "{j}");
        assert!(j.contains("[1:a][2:a]amix=inputs=2:duration=longest[aout]"), "{j}");
        assert!(j.contains("-map 0:v -map [aout]"), "{j}");
        // desktop audio alone is input 1, not 2
        let j = joined(&screen_args(&ScreenCaptureOptions { mic: String::new(), ..o }, Some("Stereo Mix")));
        assert!(j.contains("-map 0:v -map 1:a") && !j.contains("amix"), "{j}");
    }

    #[test]
    fn voice_args_wav() {
        let o = VoiceoverOptions { out: PathBuf::from(r"C:\rec\vo.wav"), device: " Mic ".into(), ..Default::default() };
        let j = joined(&voice_args(&o));
        assert!(j.contains("-f dshow -i audio=Mic"), "device name is trimmed: {j}");
        assert!(j.contains("-ac 1 -ar 48000") && j.ends_with(r"C:\rec\vo.wav"), "{j}");
        let clamped = voice_args(&VoiceoverOptions { channels: 7, sample_rate: 0, ..o });
        assert!(joined(&clamped).contains("-ac 2 -ar 48000"), "channels are clamped to stereo");
    }

    /// Hardware check (`cargo test -- --ignored capture_records`): records the desktop for ~2 s and
    /// stops it with `q`. Needs ffmpeg, a real desktop session and writes into the temp dir.
    #[test]
    #[ignore]
    fn capture_records_a_real_file() {
        let out = std::env::temp_dir().join(format!("se-capture-{}.mp4", std::process::id()));
        let cap = start_screen(ScreenCaptureOptions {
            out: out.clone(),
            fps: 10,
            region: Some((0, 0, 320, 240)),
            ..Default::default()
        })
        .expect("start");
        let prog = cap.progress();
        std::thread::sleep(std::time::Duration::from_millis(2000));
        assert!(cap.elapsed() >= 1.9, "elapsed tracks the recording");
        cap.stop();
        for _ in 0..100 {
            if prog.is_done() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(prog.is_done(), "the recorder finished: {}", prog.status());
        assert_eq!(prog.error(), None);
        let len = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        let _ = std::fs::remove_file(&out);
        assert!(len > 1000, "wrote a real mp4 ({len} bytes)");
    }

    /// No hardware, no ffmpeg needed: an empty device/output is refused before anything is spawned.
    #[test]
    fn empty_inputs_are_refused() {
        assert!(start_voiceover(VoiceoverOptions::default()).is_err());
        assert!(start_screen(ScreenCaptureOptions::default()).is_err());
    }
}
