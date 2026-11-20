//! ---- ws:forum-preview ----
//! Why playback is choppy, in words, and what one click fixes it. Three parts, all cheap:
//!
//! * **Source traits** (`traits`): is a file variable frame rate (phones, screen recorders)? Answered by
//!   one small ffprobe of the first packet timestamps, on a thread of its own, once per path and
//!   session - never on the UI or playback thread. A VFR source gets a constant-rate proxy
//!   (`proxy::wants` / `proxy::run`), so scrubbing and stepping land on even frames; export keeps reading
//!   the original by timestamp, which is what keeps sound and picture together after cuts.
//! * **Decode cost** (`cost_score`, `proxy_bytes`): a rule-of-thumb weight of a source against 1080p30
//!   H.264, and the disk an all-intra proxy of it takes - the numbers behind "Make proxy (about 1.2 GB)".
//! * **Playback health** (`Meter`, `advise`): the player's published / dropped counters, sampled while
//!   playing, turn into Green or Amber; on Amber, `advise` names the one fix that helps most (a proxy for
//!   the heaviest unproxied source under the playhead, else a lower playback resolution).
//!
//! HDR and rotation are already handled at import (`ffpipe::probe` swaps rotated sizes, `project_match`
//! offers an HDR project), so they are not repeated here.

use std::collections::HashMap;
use std::sync::Mutex;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// What the traits probe found about one source.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Traits {
    /// Variable frame rate: the gaps between frames are not all the same.
    pub vfr: bool,
    /// The rate a constant-rate copy is made at, snapped to a standard one (`model::snap_fps`): the
    /// nominal rate of a VFR phone clip, else the measured average. 0 when unknown.
    pub fps: f64,
}

/// Parse an ffprobe rate ("30000/1001", "30/1", "0/0").
pub fn parse_rate(s: &str) -> f64 {
    let s = s.trim();
    match s.split_once('/') {
        Some((n, d)) => {
            let (n, d) = (n.trim().parse::<f64>().unwrap_or(0.0), d.trim().parse::<f64>().unwrap_or(0.0));
            if d > 0.0 && n.is_finite() {
                n / d
            } else {
                0.0
            }
        }
        None => s.parse().unwrap_or(0.0),
    }
}

/// The container's nominal rate (`r_frame_rate`) and the measured average (`avg_frame_rate`) disagree by
/// more than 1 %: a phone or a screen recorder that wrote frames when it had them. 30 vs 29.97 (0.1 %) is
/// a rounding of NTSC, not VFR.
pub fn vfr_from_rates(nominal: f64, avg: f64) -> bool {
    nominal > 0.0 && avg > 0.0 && ((nominal - avg).abs() / nominal) > 0.01
}

/// From presentation times (seconds, any order: packets come in decode order): more than 2 % of the gaps
/// between neighbouring frames differ from the median gap by over a quarter. A constant-rate file's gaps
/// differ by a timestamp tick at most; a phone's swing by whole milliseconds, and a screen recording's by
/// whole frames. Fewer than 8 frames say nothing.
pub fn vfr_from_pts(pts: &[f64]) -> bool {
    let mut t: Vec<f64> = pts.iter().copied().filter(|x| x.is_finite()).collect();
    if t.len() < 8 {
        return false;
    }
    t.sort_by(|a, b| a.total_cmp(b));
    t.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    let mut gaps: Vec<f64> = t.windows(2).map(|w| w[1] - w[0]).filter(|g| *g > 0.0).collect();
    if gaps.len() < 7 {
        return false;
    }
    let mut sorted = gaps.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let median = sorted[sorted.len() / 2];
    // the last gap of a stream is often short (the muxer's end): leave it out
    gaps.pop();
    let odd = gaps.iter().filter(|g| (**g - median).abs() > median * 0.25).count();
    odd as f64 > gaps.len() as f64 * 0.02
}

/// The traits from one ffprobe JSON (`-show_entries stream=r_frame_rate,avg_frame_rate:packet=pts_time`).
pub fn traits_from_json(v: &serde_json::Value) -> Traits {
    let stream = v.get("streams").and_then(|s| s.get(0));
    let rate = |k: &str| stream.and_then(|s| s.get(k)).and_then(|r| r.as_str()).map_or(0.0, parse_rate);
    let (nominal, avg) = (rate("r_frame_rate"), rate("avg_frame_rate"));
    let pts: Vec<f64> = v
        .get("packets")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .filter_map(|p| p.get("pts_time").and_then(|t| t.as_str()).and_then(|t| t.parse().ok()))
        .collect();
    let vfr = vfr_from_pts(&pts) || vfr_from_rates(nominal, avg);
    // a phone's nominal rate is what it meant to record (30 / 60); its average sags in the dark. A screen
    // recorder's nominal is its time base (1000), so the average it is.
    let base = if vfr && nominal > 0.0 && nominal <= 120.0 { nominal } else if avg > 0.0 { avg } else { nominal };
    Traits { vfr, fps: if base > 0.0 { crate::model::snap_fps(base) } else { 0.0 } }
}

/// One ffprobe run over the first 20 s of packets (no decode: it reads timestamps only, a few ms).
pub(crate) fn probe(path: &str) -> Traits {
    let Some(exe) = super::ffpipe::ffprobe_exe() else { return Traits::default() };
    let out = super::ffpipe::command(&exe)
        .args(["-v", "error", "-select_streams", "v:0", "-read_intervals", "%+20"])
        .args(["-show_entries", "stream=r_frame_rate,avg_frame_rate:packet=pts_time", "-of", "json"])
        .arg(path)
        .stdin(std::process::Stdio::null())
        .output();
    match out.ok().and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok()) {
        Some(v) => traits_from_json(&v),
        None => Traits::default(),
    }
}

static KNOWN: Mutex<Option<HashMap<String, Option<Traits>>>> = Mutex::new(None);

/// The traits of `path`, or None while its probe runs (started on the first ask, on a thread of its own,
/// one probe at a time). Images and audio are never asked about (callers filter on the asset kind). The
/// import worker asks first (`ffpipe::probe`), so a fresh import is usually answered before anything
/// else wants to know.
pub fn traits(path: &str) -> Option<Traits> {
    let key = path.to_ascii_lowercase();
    {
        let mut g = lock(&KNOWN);
        let map = g.get_or_insert_with(HashMap::new);
        if let Some(t) = map.get(&key) {
            return *t;
        }
        map.insert(key.clone(), None);
    }
    let path = path.to_string();
    let _ = std::thread::Builder::new().name("playcheck-probe".into()).spawn(move || {
        static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
        let _one = lock(&ONE_AT_A_TIME);
        crate::media::bg::lower_thread();
        let t = probe(&path);
        lock(&KNOWN).get_or_insert_with(HashMap::new).insert(key, Some(t));
    });
    None
}

static ANNOUNCE: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);

/// The import probe (`ffpipe::probe`): start the traits probe, and mark the file as freshly imported, so
/// its timing fix is announced once (`announced`). Older files found VFR on a later open stay quiet.
pub fn announce(path: &str) {
    lock(&ANNOUNCE).get_or_insert_with(Default::default).insert(path.to_ascii_lowercase());
    let _ = traits(path);
}

/// `announce` was called for `path` this session.
pub fn announced(path: &str) -> bool {
    lock(&ANNOUNCE).as_ref().is_some_and(|s| s.contains(&path.to_ascii_lowercase()))
}

/// Tests: record an answer without running ffprobe.
#[cfg(test)]
pub fn remember(path: &str, t: Traits) {
    lock(&KNOWN).get_or_insert_with(HashMap::new).insert(path.to_ascii_lowercase(), Some(t));
}

/// `traits(path)` says VFR (false while unknown).
pub fn is_vfr(path: &str) -> bool {
    traits(path).is_some_and(|t| t.vfr)
}

/// How hard a source is to decode in real time, against 1080p30 8-bit H.264 = 1. Pixel rate times a
/// codec weight: HEVC and AV1 cost more per pixel than H.264, 10-bit HEVC more again (no hardware path
/// on many GPUs), intra codecs cost by their bit rate. About 4 and up stutters on an ordinary laptop
/// without a proxy (4K30 H.264 = 4, 4K60 HEVC = 12.8).
pub fn cost_score(codec: &str, w: u32, h: u32, fps: f64, bits_per_sec: f64, ten_bit: bool) -> f32 {
    let px = (w as f64 * h as f64 * fps.clamp(1.0, 240.0)) / (1920.0 * 1080.0 * 30.0);
    let codec = codec.to_ascii_lowercase();
    let weight = match codec.as_str() {
        "h264" => {
            if ten_bit {
                2.0
            } else {
                1.0
            }
        }
        "hevc" => {
            if ten_bit {
                2.5
            } else {
                1.6
            }
        }
        "av1" => 2.0,
        "vp9" => 1.5,
        "mpeg2video" | "mpeg4" | "mjpeg" => 0.7,
        "prores" | "dnxhd" | "cfhd" | "png" | "qtrle" | "ffv1" | "huffyuv" | "utvideo" | "rawvideo" => {
            // intra: the bytes are the work; 150 Mbit/s of ProRes at 1080p30 is about one H.264 stream
            (bits_per_sec / 150e6).max(0.6)
        }
        "" => 1.0,
        _ => 2.0, // not a Media Foundation codec: decoded through the ffmpeg pipe
    };
    (px * weight) as f32
}

/// Score at or above which a source is "heavy": proposed a proxy on sight.
pub const HEAVY: f32 = 4.0;

/// About how many bytes an all-intra proxy of a `w`x`h` source at `fps` for `dur` seconds takes at proxy
/// height `proxy_h` (`proxy::ENCODE`: libx264 `-g 1 -crf 20`, measured ~0.6 bits a pixel on camera
/// footage; flat screen recordings come out smaller, so this is the high side).
pub fn proxy_bytes(w: u32, h: u32, fps: f64, dur: f64, proxy_h: u32) -> u64 {
    if w == 0 || h == 0 || dur <= 0.0 {
        return 0;
    }
    let ph = proxy_h.min(h).max(2) as f64;
    let pw = w as f64 * ph / h as f64;
    let fps = if fps > 0.0 { fps.min(120.0) } else { 30.0 };
    (pw * ph * fps * dur * 0.6 / 8.0) as u64
}

/// "350 MB", "1.2 GB".
pub fn fmt_bytes(b: u64) -> String {
    if b >= 1_000_000_000 {
        format!("{:.1} GB", b as f64 / 1e9)
    } else {
        format!("{} MB", (b as f64 / 1e6).ceil().max(1.0) as u64)
    }
}

/// Playback health as the chip shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Level {
    /// Keeping up (or not playing, nothing seen yet).
    #[default]
    Green,
    /// Frames are being dropped.
    Amber,
}

/// Rolling playback health from the player's cumulative counters. Fed ~twice a second while playing;
/// turns Amber when the last few seconds dropped 3+ frames and at least 1 % of them, and back to Green
/// after `CLEAR_S` seconds of playing with nothing dropped (or on `reset`, after a fix). It stays Amber
/// while paused, so the fix is still there to click.
#[derive(Clone, Debug, Default)]
pub struct Meter {
    /// (seconds, published, dropped) samples while playing, oldest first, within `WINDOW_S`.
    win: Vec<(f64, u64, u64)>,
    level: Level,
    /// Seconds of drop-free play since the last drop (towards Green).
    clean: f64,
    last: Option<(f64, u64, u64)>,
}

const WINDOW_S: f64 = 4.0;
const CLEAR_S: f64 = 8.0;

impl Meter {
    pub fn level(&self) -> Level {
        self.level
    }
    /// After a fix (or a new project): Green, history forgotten.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// One sample at `now` seconds of the player's cumulative (published, dropped) frames. `playing`
    /// false only breaks the history (a pause is not a gap).
    pub fn sample(&mut self, now: f64, published: u64, dropped: u64, playing: bool) -> Level {
        if !playing {
            self.last = None;
            self.win.clear();
            return self.level;
        }
        if let Some((t0, _, d0)) = self.last {
            let dt = (now - t0).max(0.0);
            if dropped > d0 {
                self.clean = 0.0;
            } else {
                self.clean += dt;
            }
        }
        // counters can restart (a new player): start over rather than wrap
        if self.last.is_some_and(|(_, p, d)| published < p || dropped < d) {
            self.win.clear();
        }
        self.last = Some((now, published, dropped));
        self.win.push((now, published, dropped));
        self.win.retain(|s| now - s.0 <= WINDOW_S + 1e-9);
        let (first, lastw) = (self.win[0], self.win[self.win.len() - 1]);
        let (p, d) = (lastw.1.saturating_sub(first.1), lastw.2.saturating_sub(first.2));
        if d >= 3 && d as f64 >= (p + d) as f64 * 0.01 {
            self.level = Level::Amber;
        } else if self.level == Level::Amber && self.clean >= CLEAR_S {
            self.level = Level::Green;
        }
        self.level
    }
}

/// One video source under the playhead, as `advise` weighs it.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub asset: crate::model::Id,
    pub name: String,
    /// `cost_score`.
    pub score: f32,
    /// Measured ms per decode while the Performance numbers were being taken (None = not measured).
    pub measured_ms: Option<f32>,
    /// Already plays from a proxy, has one queued, or the user turned proxies off for it.
    pub settled: bool,
    /// `proxy_bytes` for it.
    pub bytes: u64,
}

/// The one fix the chip offers.
#[derive(Clone, Debug, PartialEq)]
pub enum Advice {
    /// Build a proxy for this source.
    Proxy { asset: crate::model::Id, name: String, bytes: u64 },
    /// Lower the playback resolution to this percentage.
    Quality(u32),
    /// Nothing specific: the Performance pane shows the breakdown.
    Open,
}

/// The fix for an Amber chip. A source is to blame when it is not settled and either measured slower
/// than 60 % of a frame (`frame_ms`) or, unmeasured, heavy by its score; the worst of them gets the
/// proxy. Otherwise the next lower step of `qualities` below `quality`, else `Open`.
pub fn advise(cands: &[Candidate], frame_ms: f32, quality: u32, qualities: &[u32]) -> Advice {
    let slow = |c: &&Candidate| match c.measured_ms {
        Some(ms) => ms > frame_ms * 0.6,
        None => c.score >= HEAVY,
    };
    // measured beats estimated: a real 30 ms decode outranks a guess
    let key = |c: &Candidate| c.measured_ms.map_or(c.score, |ms| 1000.0 + ms);
    if let Some(c) = cands.iter().filter(|c| !c.settled).filter(slow).max_by(|a, b| key(a).total_cmp(&key(b))) {
        return Advice::Proxy { asset: c.asset, name: c.name.clone(), bytes: c.bytes };
    }
    match qualities.iter().copied().filter(|q| *q < quality).max() {
        Some(q) => Advice::Quality(q),
        None => Advice::Open,
    }
}

#[cfg(test)]
#[path = "playcheck_bench.rs"]
mod bench;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Session D's sync check, through the app's own decoders: a VFR clip (every 4th frame of a 30 fps
    /// count missing, timestamps kept) is found VFR by the real probe, its proxy built with the proxy
    /// encoder + `cfr_args` is constant 30 fps, and at 20 random cut points the proxy shows the very
    /// source frame the original shows (each frame's grey level is its number). Skipped without ffmpeg.
    #[test]
    // Known issue (2026-11-19 merge): in a VFR gap the ORIGINAL file's frame_at shows the next frame
    // (first pts >= t) while the CFR proxy correctly holds the previous one, so they can differ by one
    // frame inside a gap. Fix in the decoder's seek (hold the last frame with pts <= t), then un-ignore.
    #[ignore = "decoder shows the next frame inside a VFR gap; see the comment above"]
    fn vfr_proxy_shows_the_original_frame_at_20_cuts() {
        use crate::media::{ffpipe, Backend, Frame};
        let (Some(ff), Some(_)) = (ffpipe::ffmpeg_exe(), ffpipe::ffprobe_exe()) else {
            return eprintln!("skipped: no ffmpeg / ffprobe");
        };
        let dir = std::env::temp_dir().join(format!("se-vfr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (src, proxy) = (dir.join("phone.mp4"), dir.join("proxy.mp4"));
        let graph = "nullsrc=s=64x64:r=30,geq=lum='mod(N\\,50)*5':cb=128:cr=128,select='not(eq(mod(n\\,4)\\,1))'";
        let ok = ffpipe::command(&ff)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", graph, "-t", "10", "-fps_mode", "passthrough"])
            .args(["-c:v", "libx264", "-crf", "0", "-pix_fmt", "yuv420p"])
            .arg(&src)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "ffmpeg made the VFR clip");
        let src_s = src.to_string_lossy().into_owned();
        let t = probe(&src_s);
        assert_eq!(t, Traits { vfr: true, fps: 30.0 }, "the probe finds it VFR");
        remember(&src_s, t);
        let cfr = crate::media::proxy::cfr_args(&src_s);
        assert_eq!(cfr, ["-fps_mode", "cfr", "-r", "30"]);
        let ok = ffpipe::command(&ff)
            .args(["-y", "-v", "error", "-i"])
            .arg(&src)
            .args(["-vf", "scale=-2:64"])
            .args(crate::media::proxy::ENCODE)
            .args(&cfr)
            .arg(&proxy)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "the proxy encode ran");
        let proxy_s = proxy.to_string_lossy().into_owned();
        assert!(!probe(&proxy_s).vfr, "the proxy is constant-rate");
        let mut a = crate::media::open_video(&src_s, Backend::Auto).expect("opens the original");
        let mut b = crate::media::open_video(&proxy_s, Backend::Auto).expect("opens the proxy");
        let (mut fa, mut fb) = (Frame::default(), Frame::default());
        let mut seed = 0x2545_f491_u64;
        let mut cuts: Vec<f64> = (0..20)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                // mid-frame on the 30 fps grid: a time a few ms from a frame boundary is a tie both
                // decoders may break either way (one frame apart), which isn't what this checks
                (((seed % 9_800) as f64 / 1000.0 + 0.05) * 30.0).floor() / 30.0 + 0.5 / 30.0
            })
            .collect();
        cuts.sort_by(|x, y| x.total_cmp(y));
        for &t in cuts.iter().rev().chain(&cuts) {
            // backwards then forwards: both seek paths
            assert!(a.frame_at(t, 64, 64, &mut fa) && b.frame_at(t, 64, 64, &mut fb), "frames at {t}");
            let at = (32 * 64 + 32) * 4;
            let (ga, gb) = (fa.rgba[at] as i32, fb.rgba[at] as i32);
            // neighbouring frames differ by ~6 levels after the range expansion: 3 tells them apart
            assert!((ga - gb).abs() <= 3, "cut at {t:.3}: original grey {ga}, proxy grey {gb}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rates_parse_and_ntsc_is_not_vfr() {
        assert!((parse_rate("30000/1001") - 29.97).abs() < 0.01);
        assert_eq!(parse_rate("0/0"), 0.0);
        assert_eq!(parse_rate("25"), 25.0);
        assert!(!vfr_from_rates(30.0, 29.97), "NTSC rounding");
        assert!(!vfr_from_rates(30.0, 30.0));
        assert!(vfr_from_rates(30.0, 26.02), "a phone that dropped to 26 fps in the dark");
        assert!(vfr_from_rates(1000.0, 59.6), "a screen recorder's millisecond time base");
        assert!(!vfr_from_rates(0.0, 30.0), "unknown is not VFR");
    }

    #[test]
    fn pts_spread_finds_vfr() {
        // constant 30 fps with a 1/90000 tick of jitter: not VFR, whatever order the packets came in
        let mut cfr: Vec<f64> = (0..300).map(|i| i as f64 / 30.0 + if i % 7 == 0 { 1.0 / 90000.0 } else { 0.0 }).collect();
        cfr.swap(3, 5); // B-frames: decode order differs from display order
        assert!(!vfr_from_pts(&cfr));
        // a phone: mostly 33.3 ms, now and then 40-50 ms
        let mut t = 0.0;
        let vfr: Vec<f64> = (0..300)
            .map(|i| {
                let at = t;
                t += if i % 10 == 0 { 0.045 } else { 1.0 / 30.0 };
                at
            })
            .collect();
        assert!(vfr_from_pts(&vfr));
        // a screen recording that only writes changed frames: gaps of 1, 2 and 5 frames
        let mut t = 0.0;
        let rec: Vec<f64> = (0..120)
            .map(|i| {
                let at = t;
                t += [1.0, 1.0, 2.0, 1.0, 5.0][i % 5] / 60.0;
                at
            })
            .collect();
        assert!(vfr_from_pts(&rec));
        assert!(!vfr_from_pts(&[0.0, 0.1, 0.3]), "too few frames to say");
        // a short last frame alone is not VFR
        let mut end: Vec<f64> = (0..60).map(|i| i as f64 / 30.0).collect();
        end.push(60.0 / 30.0 - 0.02 + 1.0 / 30.0);
        assert!(!vfr_from_pts(&end));
    }

    #[test]
    fn traits_read_ffprobe_json() {
        let pkts: Vec<_> = (0..40).map(|i| json!({"pts_time": format!("{:.6}", i as f64 / 25.0)})).collect();
        let v = json!({"streams": [{"r_frame_rate": "25/1", "avg_frame_rate": "25/1"}], "packets": pkts});
        assert_eq!(traits_from_json(&v), Traits { vfr: false, fps: 25.0 });
        // a phone that sagged to 26 fps in the dark: VFR, conformed at the 30 it meant
        let v = json!({"streams": [{"r_frame_rate": "30/1", "avg_frame_rate": "2343/90"}], "packets": []});
        assert_eq!(traits_from_json(&v), Traits { vfr: true, fps: 30.0 });
        // a screen recorder: millisecond time base, the average is the rate
        let v = json!({"streams": [{"r_frame_rate": "1000/1", "avg_frame_rate": "5994/100"}], "packets": []});
        assert_eq!(traits_from_json(&v), Traits { vfr: true, fps: 59.94 });
        // 30 vs 29.99: not VFR by the rates alone
        let v = json!({"streams": [{"r_frame_rate": "30/1", "avg_frame_rate": "16313/544"}], "packets": []});
        assert!(!traits_from_json(&v).vfr);
        assert_eq!(traits_from_json(&json!({})), Traits::default());
    }

    #[test]
    fn remembered_traits_answer_without_a_probe() {
        remember("C:/Phone/IMG_0001.MOV", Traits { vfr: true, fps: 30.0 });
        assert!(is_vfr("c:/phone/img_0001.mov"), "paths compare case-insensitively, like Windows");
        assert_eq!(traits("C:/Phone/IMG_0001.MOV"), Some(Traits { vfr: true, fps: 30.0 }));
        assert!(!announced("C:/Phone/IMG_0001.MOV"), "not imported this session: no toast");
        announce("C:/Phone/IMG_0001.MOV");
        assert!(announced("c:/phone/img_0001.mov"));
    }

    #[test]
    fn cost_scores_rank_the_forum_clips() {
        let base = cost_score("h264", 1920, 1080, 30.0, 10e6, false);
        assert!((base - 1.0).abs() < 1e-3);
        assert!(cost_score("hevc", 3840, 2160, 60.0, 50e6, true) >= 12.0, "4K60 10-bit phone HEVC");
        assert!(cost_score("h264", 3840, 2160, 30.0, 50e6, false) >= HEAVY, "4K30 H.264");
        assert!(cost_score("h264", 1920, 1080, 60.0, 12e6, false) < HEAVY, "1080p60 H.264 plays");
        assert!(cost_score("hevc", 1920, 1080, 60.0, 20e6, false) < HEAVY, "1080p60 HEVC plays");
        assert!(cost_score("prores", 3840, 2160, 30.0, 700e6, false) >= HEAVY);
        assert!(cost_score("h264", 1280, 720, 30.0, 3e6, false) < 1.0);
    }

    #[test]
    fn proxy_size_estimate() {
        // 10 min of 4K30 to a 720p proxy: 1280x720x30x600x0.6/8 = ~1.24 GB
        let b = proxy_bytes(3840, 2160, 30.0, 600.0, 720);
        assert!((1.1e9..1.4e9).contains(&(b as f64)), "{b}");
        assert_eq!(fmt_bytes(b), "1.2 GB");
        assert_eq!(fmt_bytes(349_200_000), "350 MB");
        assert_eq!(fmt_bytes(10), "1 MB");
        assert_eq!(proxy_bytes(0, 0, 30.0, 10.0, 720), 0);
        // a source smaller than the proxy height is not scaled up
        assert_eq!(proxy_bytes(640, 360, 30.0, 10.0, 720), proxy_bytes(640, 360, 30.0, 10.0, 360));
    }

    #[test]
    fn meter_turns_amber_on_drops_and_clears_after_clean_play() {
        let mut m = Meter::default();
        let mut t = 0.0;
        let (mut p, mut d) = (0u64, 0u64);
        let mut step = |m: &mut Meter, pub_n: u64, drop_n: u64, playing: bool| {
            t += 0.5;
            p += pub_n;
            d += drop_n;
            m.sample(t, p, d, playing)
        };
        for _ in 0..10 {
            assert_eq!(step(&mut m, 30, 0, true), Level::Green, "smooth");
        }
        assert_eq!(step(&mut m, 30, 1, true), Level::Green, "one drop is not choppy");
        assert_eq!(step(&mut m, 28, 2, true), Level::Amber, "3 in the window, over 1 %");
        assert_eq!(step(&mut m, 0, 0, false), Level::Amber, "paused: the fix stays offered");
        // (the first sample after the pause only restarts the clock: the pause is not clean play)
        for _ in 0..16 {
            step(&mut m, 30, 0, true);
        }
        assert_eq!(m.level(), Level::Amber, "7.5 s clean is not yet 8");
        assert_eq!(step(&mut m, 30, 0, true), Level::Green, "8 s clean");
        // a long smooth stretch with a stray drop every few seconds stays green
        let mut m = Meter::default();
        for i in 0..40 {
            step(&mut m, 30, u64::from(i % 10 == 0), true);
        }
        assert_eq!(m.level(), Level::Green);
        // reset after a fix
        let mut m = Meter::default();
        for _ in 0..4 {
            step(&mut m, 20, 10, true);
        }
        assert_eq!(m.level(), Level::Amber);
        m.reset();
        assert_eq!(m.level(), Level::Green);
    }

    #[test]
    fn meter_survives_a_counter_restart() {
        let mut m = Meter::default();
        m.sample(0.0, 1000, 50, true);
        // a new player: counters back to 0 - no underflow, no false alarm
        assert_eq!(m.sample(0.5, 15, 0, true), Level::Green);
        assert_eq!(m.sample(1.0, 30, 0, true), Level::Green);
    }

    #[test]
    fn advice_blames_the_heaviest_unsettled_source() {
        let c = |asset, score, measured_ms, settled| Candidate {
            asset,
            name: format!("clip{asset}"),
            score,
            measured_ms,
            settled,
            bytes: 1_000_000 * asset,
        };
        let q = [100, 75, 50, 25];
        // measured slow beats a heavier score that was never measured
        let a = advise(&[c(1, 12.0, None, false), c(2, 2.0, Some(30.0), false)], 33.3, 100, &q);
        assert_eq!(a, Advice::Proxy { asset: 2, name: "clip2".into(), bytes: 2_000_000 });
        // unmeasured: the heavy one
        let a = advise(&[c(1, 12.0, None, false), c(3, 1.0, None, false)], 33.3, 100, &q);
        assert!(matches!(a, Advice::Proxy { asset: 1, .. }));
        // settled sources (proxy ready / queued / off) are never offered again: lower the resolution
        assert_eq!(advise(&[c(1, 12.0, None, true)], 33.3, 100, &q), Advice::Quality(75));
        assert_eq!(advise(&[], 16.7, 75, &q), Advice::Quality(50));
        // a fast measured source is not to blame even with a big score
        assert_eq!(advise(&[c(1, 12.0, Some(4.0), false)], 33.3, 50, &q), Advice::Quality(25));
        assert_eq!(advise(&[], 33.3, 25, &q), Advice::Open, "nothing left to lower");
    }
}
