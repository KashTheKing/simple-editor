//! ---- parity-webcam ----
//! Webcam capture through ffmpeg's DirectShow input (`-f dshow -i video=<name>`), next to the screen
//! recorder: a webcam is one more `CaptureSource` (`CaptureSource::Webcam`), so a webcam recording goes
//! through the same crash-safe pipeline as a screen recording (MKV pieces, WASAPI audio taps lined up on
//! the wall clock, mux, probe, then the Library) and the same live preview.
//!
//! * Devices and formats come from `ffmpeg -list_devices` / `-list_options`, probed off the UI thread
//!   (`devices_nonblocking`, `formats_nonblocking`): the first can take seconds on some machines.
//! * A camera's frames are stamped with the wall clock (`-use_wallclock_as_timestamps 1`), like gdigrab's,
//!   so ffmpeg's `start:` line is the first frame's wall time and `capture::finalize` lines the microphone
//!   up with it exactly as it does for a screen.
//! * The bubble (`Bubble`): a round webcam picture in a corner of a screen recording, composited live by
//!   the recording's own ffmpeg (a second input and an `overlay`). `-copyts` keeps both inputs on the wall
//!   clock so the face matches the screen and the microphone; the result is shifted back to 0 after the
//!   overlay. Nothing extra runs in the app: the cost is ffmpeg's.

use crate::media::ffpipe;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Mutex;

/// One capture format a camera offers: a compressed stream (`codec`, e.g. mjpeg / h264) or raw frames
/// (`pix`, e.g. yuyv422 / nv12), at a size and its highest frame rate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CamFormat {
    pub codec: String,
    pub pix: String,
    pub w: u32,
    pub h: u32,
    pub fps: f64,
}

impl CamFormat {
    /// The settings value: "mjpeg 1280x720 30" / "yuyv422 640x480 30" (kind, size, rate).
    pub fn key(&self) -> String {
        let kind = if self.codec.is_empty() { &self.pix } else { &self.codec };
        format!("{kind} {}x{} {}", self.w, self.h, fps_text(self.fps))
    }
    pub fn label(&self) -> String {
        let kind = if self.codec.is_empty() { self.pix.to_ascii_uppercase() } else { self.codec.to_ascii_uppercase() };
        format!("{}×{} · {} fps · {kind}", self.w, self.h, fps_text(self.fps))
    }
    /// Back from `key` (the kind is a codec when it is one ffmpeg's dshow input decodes, else a pixel format).
    pub fn from_key(k: &str) -> Option<CamFormat> {
        let mut it = k.split_whitespace();
        let kind = it.next()?.to_string();
        let (w, h) = it.next()?.split_once('x')?;
        let fps: f64 = it.next()?.parse().ok()?;
        let (w, h) = (w.parse().ok()?, h.parse().ok()?);
        let compressed = matches!(kind.as_str(), "mjpeg" | "h264" | "hevc" | "h265");
        let (codec, pix) = if compressed { (kind, String::new()) } else { (String::new(), kind) };
        (w > 0 && h > 0 && fps > 0.0).then_some(CamFormat { codec, pix, w, h, fps })
    }
}

/// "30", "29.97", "7.5": no trailing zeros.
fn fps_text(f: f64) -> String {
    let s = format!("{:.3}", f);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A camera and how to open it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CamSpec {
    /// The DirectShow name ("" = no camera picked / none found).
    pub device: String,
    /// None = whatever the camera starts with.
    pub format: Option<CamFormat>,
    /// Flip left-right, like a mirror (in the recording and the preview).
    pub mirror: bool,
}

/// ffmpeg input args that open `spec`. `fps` is the rate asked for when no format is picked.
pub fn input_args(spec: &CamSpec, fps: u32) -> Vec<String> {
    let mut a: Vec<String> = ["-f", "dshow", "-rtbufsize", "256M", "-thread_queue_size", "1024", "-use_wallclock_as_timestamps", "1"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    match &spec.format {
        Some(f) => {
            if !f.codec.is_empty() {
                a.extend(["-vcodec".into(), f.codec.clone()]);
            } else if !f.pix.is_empty() {
                a.extend(["-pixel_format".into(), f.pix.clone()]);
            }
            a.extend(["-video_size".into(), format!("{}x{}", f.w, f.h), "-framerate".into(), fps_text(f.fps)]);
        }
        None => a.extend(["-framerate".into(), fps.clamp(1, 120).to_string()]),
    }
    a.extend(["-i".into(), format!("video={}", spec.device.trim())]);
    a
}

/// Video device names out of `ffmpeg -list_devices true -f dshow -i dummy` (both the modern
/// `"name" (video)` layout and the older section headers).
pub fn parse_video_devices(stderr: &str) -> Vec<String> {
    crate::engine::capture::parse_dshow(stderr, true).into_iter().map(|(n, _)| n).collect()
}

/// A camera's formats out of `ffmpeg -list_options true -f dshow -i video=<name>`:
/// `vcodec=mjpeg  min s=1280x720 fps=5 max s=1280x720 fps=30` / `pixel_format=yuyv422 ...`. Each size
/// once per kind, at its highest rate; largest first, compressed before raw at the same size.
pub fn parse_formats(stderr: &str) -> Vec<CamFormat> {
    let mut out: Vec<CamFormat> = Vec::new();
    for line in stderr.lines() {
        let l = line.trim();
        let l = match (l.starts_with('['), l.find("] ")) {
            (true, Some(i)) => l[i + 2..].trim(),
            _ => l,
        };
        let (codec, pix) = if let Some(r) = l.strip_prefix("vcodec=") {
            (r.split_whitespace().next().unwrap_or("").to_string(), String::new())
        } else if let Some(r) = l.strip_prefix("pixel_format=") {
            (String::new(), r.split_whitespace().next().unwrap_or("").to_string())
        } else {
            continue;
        };
        // the "max" half: the largest size and fastest rate of this entry
        let Some(max) = l.split(" max ").nth(1) else { continue };
        let mut size = None;
        let mut fps = None;
        for tok in max.split_whitespace() {
            if let Some(s) = tok.strip_prefix("s=") {
                size = s.split_once('x').and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)));
            } else if let Some(f) = tok.strip_prefix("fps=") {
                fps = f.trim_end_matches(',').parse::<f64>().ok();
            }
        }
        let (Some((w, h)), Some(fps)) = (size, fps) else { continue };
        if w == 0 || h == 0 || fps <= 0.0 || (codec.is_empty() && pix.is_empty()) {
            continue;
        }
        let fps = (fps * 1000.0).round() / 1000.0;
        let f = CamFormat { codec, pix, w, h, fps };
        match out.iter_mut().find(|o| o.codec == f.codec && o.pix == f.pix && o.w == f.w && o.h == f.h) {
            Some(o) => o.fps = o.fps.max(f.fps),
            None => out.push(f),
        }
    }
    out.sort_by(|a, b| {
        (b.w * b.h).cmp(&(a.w * a.h)).then(b.fps.total_cmp(&a.fps)).then(a.codec.is_empty().cmp(&b.codec.is_empty()))
    });
    out
}

/// The cameras on this machine (blocking: it runs ffmpeg).
pub fn devices() -> Vec<String> {
    let Some(exe) = ffpipe::ffmpeg_exe() else { return Vec::new() };
    match ffpipe::command(&exe)
        .args(["-hide_banner", "-list_devices", "true", "-f", "dshow", "-i", "dummy"])
        .stdin(Stdio::null())
        .output()
    {
        Ok(o) => parse_video_devices(&String::from_utf8_lossy(&o.stderr)),
        Err(_) => Vec::new(),
    }
}

static DEVICES: crate::engine::bg_probe::BgProbe<Vec<String>> = crate::engine::bg_probe::BgProbe::new();

/// `devices` for the UI thread: `None` while the background probe runs (`wake` runs when it lands).
pub fn devices_nonblocking(wake: fn()) -> Option<Vec<String>> {
    DEVICES.get(devices, wake)
}

/// List the cameras again (a camera plugged in after the first look), and their formats.
pub fn refresh() {
    DEVICES.reset();
    *FORMATS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// A camera's formats (blocking: it runs ffmpeg).
pub fn formats(device: &str) -> Vec<CamFormat> {
    let Some(exe) = ffpipe::ffmpeg_exe() else { return Vec::new() };
    match ffpipe::command(&exe)
        .args(["-hide_banner", "-list_options", "true", "-f", "dshow", "-i"])
        .arg(format!("video={}", device.trim()))
        .stdin(Stdio::null())
        .output()
    {
        Ok(o) => parse_formats(&String::from_utf8_lossy(&o.stderr)),
        Err(_) => Vec::new(),
    }
}

/// device -> None while probing, Some(list) once known.
static FORMATS: Mutex<Option<HashMap<String, Option<Vec<CamFormat>>>>> = Mutex::new(None);

/// `formats` for the UI thread: `None` while its probe runs (started on the first ask; `wake` when done).
pub fn formats_nonblocking(device: &str, wake: fn()) -> Option<Vec<CamFormat>> {
    if device.trim().is_empty() {
        return Some(Vec::new());
    }
    let mut g = FORMATS.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(HashMap::new);
    if let Some(v) = map.get(device) {
        return v.clone();
    }
    map.insert(device.to_string(), None);
    drop(g);
    let d = device.to_string();
    let _ = std::thread::Builder::new().name("cam-formats".into()).spawn(move || {
        let v = formats(&d);
        let mut g = FORMATS.lock().unwrap_or_else(|e| e.into_inner());
        g.get_or_insert_with(HashMap::new).insert(d, Some(v));
        drop(g);
        wake();
    });
    None
}

/// Which corner the bubble sits in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
}

impl Corner {
    pub const ALL: [Corner; 4] = [Corner::BottomRight, Corner::BottomLeft, Corner::TopRight, Corner::TopLeft];
    pub fn key(self) -> &'static str {
        match self {
            Corner::TopLeft => "tl",
            Corner::TopRight => "tr",
            Corner::BottomLeft => "bl",
            Corner::BottomRight => "br",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Corner::TopLeft => "Top left",
            Corner::TopRight => "Top right",
            Corner::BottomLeft => "Bottom left",
            Corner::BottomRight => "Bottom right",
        }
    }
    pub fn from_key(k: &str) -> Corner {
        Corner::ALL.into_iter().find(|c| c.key() == k).unwrap_or_default()
    }
}

/// A round webcam picture in a corner of a screen recording.
#[derive(Clone, Debug, PartialEq)]
pub struct Bubble {
    pub cam: CamSpec,
    /// Diameter in pixels of the captured screen (scaled with the recording's Scale).
    pub size: u32,
    pub corner: Corner,
}

/// The bubble's diameter and margin in output pixels for a recording at `scale` percent: even, at least 32.
pub fn bubble_px(size: u32, scale: u32) -> (u32, u32) {
    let d = ((size.clamp(32, 2048) as u64 * scale.clamp(10, 100) as u64 / 100) as u32).max(32) & !1;
    (d, ((d / 8).max(8)) & !1)
}

/// The `-filter_complex` graph of a screen recording with a bubble: input 0 the screen (through `base`,
/// the recording's own filter, if any), input 1 the camera. The output is `[v]`, starting at 0 (the
/// screen's first frame); with `preview`, a letterboxed RGBA copy for the pane is `[pv]`.
pub fn bubble_graph(base: Option<&str>, b: &Bubble, scale: u32, preview: Option<&str>) -> String {
    let (d, m) = bubble_px(b.size, scale);
    let r = d as f64 / 2.0;
    let c = (d as f64 - 1.0) / 2.0;
    let flip = if b.cam.mirror { ",hflip" } else { "" };
    // a circle with a one-pixel soft edge; only the alpha plane changes
    let cam = format!(
        "[1:v]crop='min(iw,ih)':'min(iw,ih)',scale={d}:{d}{flip},format=yuva420p,\
         geq=lum='lum(X,Y)':cb='cb(X,Y)':cr='cr(X,Y)':a='255*clip({r}-hypot(X-{c},Y-{c})+0.5,0,1)'[cam]"
    );
    let x = match b.corner {
        Corner::TopLeft | Corner::BottomLeft => format!("{m}"),
        _ => format!("main_w-overlay_w-{m}"),
    };
    let y = match b.corner {
        Corner::TopLeft | Corner::TopRight => format!("{m}"),
        _ => format!("main_h-overlay_h-{m}"),
    };
    let base = format!("[0:v]{}[base]", base.unwrap_or("null"));
    // both inputs carry wall-clock times (-copyts): the overlay pairs frames taken at the same moment,
    // then the result starts at 0 like a recording without a bubble
    let mix = format!("[base][cam]overlay=x={x}:y={y}:eof_action=pass:format=auto,setpts=PTS-STARTPTS");
    match preview {
        Some(pv) => format!("{base};{cam};{mix},split=2[v][p];[p]{pv}[pv]"),
        None => format!("{base};{cam};{mix}[v]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_devices_only() {
        let modern = r#"[dshow @ 000001] "HD Pro Webcam C920" (video)
[dshow @ 000001]   Alternative name "@device_pnp_\\?\usb#vid_046d"
[dshow @ 000001] "OBS Virtual Camera" (none)
[dshow @ 000001] "Microphone (C920)" (audio)
[dshow @ 000001] "Integrated Camera" (video)"#;
        assert_eq!(parse_video_devices(modern), vec!["HD Pro Webcam C920", "Integrated Camera"]);
        let legacy = "[dshow @ 01] DirectShow video devices\n[dshow @ 01]  \"Cam A\"\n[dshow @ 01] DirectShow audio devices\n[dshow @ 01]  \"Mic\"\n";
        assert_eq!(parse_video_devices(legacy), vec!["Cam A"]);
    }

    #[test]
    fn formats_parse_dedupe_and_sort() {
        let s = r#"[dshow @ 01] DirectShow video device options (from video devices)
[dshow @ 01]  Pin "Capture" (alternative pin name "0")
[dshow @ 01]   pixel_format=yuyv422  min s=640x480 fps=5 max s=640x480 fps=30
[dshow @ 01]   pixel_format=yuyv422  min s=640x480 fps=5 max s=640x480 fps=15
[dshow @ 01]   vcodec=mjpeg  min s=1920x1080 fps=5 max s=1920x1080 fps=30 (tv, bt470bg/bt709/unknown, center)
[dshow @ 01]   vcodec=mjpeg  min s=1280x720 fps=5 max s=1280x720 fps=60.0002
[dshow @ 01]   pixel_format=yuyv422  min s=1920x1080 fps=5 max s=1920x1080 fps=5
video=Cam: Immediate exit requested"#;
        let f = parse_formats(s);
        let keys: Vec<String> = f.iter().map(CamFormat::key).collect();
        assert_eq!(keys, vec!["mjpeg 1920x1080 30", "yuyv422 1920x1080 5", "mjpeg 1280x720 60", "yuyv422 640x480 30"]);
        assert_eq!(f[0].label(), "1920×1080 · 30 fps · MJPEG");
        for x in &f {
            assert_eq!(CamFormat::from_key(&x.key()).as_ref(), Some(x), "key round trip");
        }
        assert_eq!(CamFormat::from_key("rubbish"), None);
        assert_eq!(CamFormat::from_key("nv12 1280x720 29.97").map(|f| (f.pix, f.fps)), Some(("nv12".into(), 29.97)));
    }

    #[test]
    fn input_args_pick_the_format() {
        let mut spec = CamSpec { device: "HD Cam".into(), format: None, mirror: false };
        let a = input_args(&spec, 30).join(" ");
        assert!(a.starts_with("-f dshow"), "{a}");
        assert!(a.contains("-use_wallclock_as_timestamps 1"), "wall clock, so audio lines up: {a}");
        assert!(a.contains("-framerate 30") && a.ends_with("-i video=HD Cam"), "{a}");
        assert!(!a.contains("-video_size"), "{a}");
        spec.format = CamFormat::from_key("mjpeg 1280x720 30");
        let a = input_args(&spec, 30).join(" ");
        assert!(a.contains("-vcodec mjpeg -video_size 1280x720 -framerate 30 -i"), "{a}");
        spec.format = CamFormat::from_key("yuyv422 640x480 29.97");
        let a = input_args(&spec, 30).join(" ");
        assert!(a.contains("-pixel_format yuyv422 -video_size 640x480 -framerate 29.97"), "{a}");
    }

    #[test]
    fn bubble_size_scales_and_stays_even() {
        assert_eq!(bubble_px(270, 100), (270, 32));
        assert_eq!(bubble_px(270, 50), (134, 16));
        assert_eq!(bubble_px(10, 100).0, 32);
        assert_eq!(bubble_px(301, 100).0 % 2, 0);
    }

    #[test]
    fn bubble_graph_overlays_a_round_camera() {
        let b = Bubble { cam: CamSpec { device: "C".into(), format: None, mirror: true }, size: 200, corner: Corner::BottomRight };
        let g = bubble_graph(Some("crop=trunc(iw/2)*2:trunc(ih/2)*2"), &b, 100, None);
        assert!(g.starts_with("[0:v]crop=trunc(iw/2)*2:trunc(ih/2)*2[base];[1:v]"), "{g}");
        assert!(g.contains("scale=200:200,hflip,format=yuva420p"), "{g}");
        assert!(g.contains("hypot(X-99.5,Y-99.5)"), "centred circle: {g}");
        assert!(g.contains("overlay=x=main_w-overlay_w-24:y=main_h-overlay_h-24"), "{g}");
        assert!(g.contains("setpts=PTS-STARTPTS[v]"), "starts at 0: {g}");
        let b = Bubble { corner: Corner::TopLeft, cam: CamSpec { mirror: false, ..b.cam }, ..b };
        let g = bubble_graph(None, &b, 100, Some("scale=480:270,format=rgba"));
        assert!(g.starts_with("[0:v]null[base]"), "{g}");
        assert!(!g.contains("hflip"), "{g}");
        assert!(g.contains("overlay=x=24:y=24"), "{g}");
        assert!(g.ends_with("split=2[v][p];[p]scale=480:270,format=rgba[pv]"), "{g}");
        // every quote is closed: the graph parser would otherwise swallow the rest
        assert_eq!(g.matches('\'').count() % 2, 0, "{g}");
    }

    #[test]
    fn corners_round_trip() {
        for c in Corner::ALL {
            assert_eq!(Corner::from_key(c.key()), c);
        }
        assert_eq!(Corner::from_key(""), Corner::BottomRight);
    }
}
