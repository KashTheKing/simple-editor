//! ---- ws:parity-formats ----
//! Made pictures: the generators (colour bars, noise, plasma, a counter / timer) and the encoder that
//! turns any frame source (these, `engine::lottie`) into a clip file.
//!
//! Generators are drawn on a job thread into a file in `Generated\` and imported like any media (the
//! same way a recording or an image-sequence bake lands), so playback, proxies and export treat them
//! as ordinary footage and nothing is computed while playing. A still (colour bars, a Lottie that
//! does not move) is one PNG; a moving opaque picture is H.264; a moving picture with transparency
//! (the counter over nothing, most Lottie animations) is colour stacked over alpha
//! (`.stacked.mp4`, `media::proxy::StackedAlpha` folds it back), as HTML layer renders are.

use std::io::Write;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenKind {
    Bars,
    Noise,
    Plasma,
    Counter,
}

impl GenKind {
    pub const ALL: [GenKind; 4] = [GenKind::Bars, GenKind::Noise, GenKind::Plasma, GenKind::Counter];

    pub fn parse(s: &str) -> Option<GenKind> {
        Some(match s.trim().to_ascii_lowercase().replace([' ', '_', '-'], "").as_str() {
            "bars" | "colorbars" | "colourbars" | "smpte" | "smptebars" => GenKind::Bars,
            "noise" | "static" => GenKind::Noise,
            "plasma" => GenKind::Plasma,
            "counter" | "timer" | "countdown" | "clock" => GenKind::Counter,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            GenKind::Bars => "Colour Bars",
            GenKind::Noise => "Noise",
            GenKind::Plasma => "Plasma",
            GenKind::Counter => "Counter",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            GenKind::Bars => "SMPTE-style colour bars, for checking colour and levels",
            GenKind::Noise => "Animated TV static",
            GenKind::Plasma => "A flowing colour plasma background",
            GenKind::Counter => "A timer counting up or down, over a transparent background",
        }
    }
}

/// How a counter shows its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterFormat {
    /// `M:SS` (`H:MM:SS` from an hour).
    Clock,
    /// `M:SS.cc` with hundredths.
    Precise,
    /// Whole seconds: `10`, `9`, ...
    Seconds,
    /// The frame number.
    Frames,
}

impl CounterFormat {
    pub fn parse(s: &str) -> Option<CounterFormat> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "clock" | "mm:ss" | "time" => CounterFormat::Clock,
            "precise" | "hundredths" | "mm:ss.cc" | "stopwatch" => CounterFormat::Precise,
            "seconds" | "secs" | "s" => CounterFormat::Seconds,
            "frames" | "frame" => CounterFormat::Frames,
            _ => return None,
        })
    }
}

/// What to generate. Sizes are pixels, times seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct GenSpec {
    pub kind: GenKind,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
    /// Noise: colour speckles instead of grey.
    pub colour: bool,
    /// Plasma: how fast it flows (1 = the default).
    pub speed: f64,
    /// Counter: count down (from `start` to 0) instead of up (from `start`).
    pub down: bool,
    /// Counter: the value at the first frame, seconds; None = 0 up, the clip's length down.
    pub start: Option<f64>,
    pub format: CounterFormat,
    /// Counter: text colour, and the background behind it (alpha 0 = transparent).
    pub color: [u8; 4],
    pub background: [u8; 4],
    pub seed: u64,
}

impl GenSpec {
    pub fn new(kind: GenKind, width: u32, height: u32, fps: f64) -> GenSpec {
        GenSpec {
            kind,
            width,
            height,
            fps,
            duration: if kind == GenKind::Counter { 10.0 } else { 5.0 },
            colour: false,
            speed: 1.0,
            down: false,
            start: None,
            format: CounterFormat::Clock,
            color: [255, 255, 255, 255],
            background: [0, 0, 0, 0],
            seed: 0x5EED,
        }
    }

    /// Even sides between 16 and 4K, a sane rate and length.
    pub fn clamped(mut self) -> GenSpec {
        let even = |v: u32, max: u32| ((v.clamp(16, max) + 1) & !1).min(max);
        self.width = even(self.width, 3840);
        self.height = even(self.height, 2160);
        self.fps = if self.fps.is_finite() { self.fps.clamp(1.0, 120.0) } else { 30.0 };
        self.duration = if self.duration.is_finite() { self.duration.clamp(0.1, 3600.0) } else { 5.0 };
        self.speed = if self.speed.is_finite() { self.speed.clamp(0.0, 20.0) } else { 1.0 };
        self
    }

    pub fn frames(&self) -> usize {
        (self.duration * self.fps).round().max(1.0) as usize
    }

    /// A single picture (no time dimension).
    pub fn still(&self) -> bool {
        self.kind == GenKind::Bars
    }

    /// Has transparency.
    pub fn alpha(&self) -> bool {
        self.kind == GenKind::Counter && self.background[3] < 255
    }

    /// The file it is written to (in `Generated\`): `Plasma.mp4`, `Counter.stacked.mp4`, `Colour Bars.png`.
    pub fn file_ext(&self) -> &'static str {
        output_ext(self.still(), self.alpha())
    }
}

/// The output extension for a still / a clip with or without transparency.
pub fn output_ext(still: bool, alpha: bool) -> &'static str {
    match (still, alpha) {
        (true, _) => "png",
        (false, true) => "stacked.mp4",
        (false, false) => "mp4",
    }
}

// ---------------------------------------------------------------------------------------------------
// generators

/// SMPTE ECR 1-1978 style bars (75 %): seven bars over two thirds, the reverse-blue castellations,
/// then -I, white, +Q, black and the PLUGE (below black, black, above black). RGBA into `out`.
pub fn bars(w: u32, h: u32, out: &mut [u8]) {
    const G: u8 = 191;
    const TOP: [[u8; 3]; 7] = [[G, G, G], [G, G, 0], [0, G, G], [0, G, 0], [G, 0, G], [G, 0, 0], [0, 0, G]];
    const MID: [[u8; 3]; 7] = [[0, 0, G], [19, 19, 19], [G, 0, G], [19, 19, 19], [0, G, G], [19, 19, 19], [G, G, G]];
    let (wf, hf) = (w as f64, h as f64);
    let bar = wf / 7.0;
    for y in 0..h {
        let fy = (y as f64 + 0.5) / hf;
        for x in 0..w {
            let fx = x as f64 + 0.5;
            let i = ((fx / bar) as usize).min(6);
            let c = if fy < 0.67 {
                TOP[i]
            } else if fy < 0.75 {
                MID[i]
            } else {
                // four 5/4-bar blocks, then the PLUGE in thirds of the fifth bar's width, then black
                let q = bar * 1.25;
                match fx {
                    v if v < q => [0, 33, 76],
                    v if v < 2.0 * q => [255, 255, 255],
                    v if v < 3.0 * q => [50, 0, 106],
                    v if v < 5.0 * bar => [19, 19, 19],
                    v if v < 5.0 * bar + bar / 3.0 => [9, 9, 9],
                    v if v < 5.0 * bar + 2.0 * bar / 3.0 => [19, 19, 19],
                    v if v < 6.0 * bar => [29, 29, 29],
                    _ => [19, 19, 19],
                }
            };
            let o = ((y * w + x) * 4) as usize;
            out[o..o + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
}

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Frame `frame` of TV static: every pixel random, a new pattern each frame, the same for the same
/// seed (re-generating gives the same clip).
pub fn noise(w: u32, h: u32, frame: u64, seed: u64, colour: bool, out: &mut [u8]) {
    let base = splitmix(seed ^ frame.wrapping_mul(0xA24B_AED4_963E_E407));
    for (i, px) in out.chunks_exact_mut(4).take((w * h) as usize).enumerate() {
        let r = splitmix(base ^ i as u64);
        let b = r.to_le_bytes();
        if colour {
            px.copy_from_slice(&[b[0], b[1], b[2], 255]);
        } else {
            px.copy_from_slice(&[b[0], b[0], b[0], 255]);
        }
    }
}

/// Plasma at second `t`: four interfering sine fields mapped through a cycling palette. Coordinates
/// are in heights, so the pattern is the same shape at any resolution.
pub fn plasma(w: u32, h: u32, t: f64, speed: f64, out: &mut [u8]) {
    let s = (h.max(1)) as f64;
    let t = t * speed;
    let (cx, cy) = (w as f64 / s / 2.0 + 0.4 * (t * 0.37).sin(), 0.5 + 0.4 * (t * 0.23).cos());
    for y in 0..h {
        let v = y as f64 / s;
        for x in 0..w {
            let u = x as f64 / s;
            let d = ((u - cx).powi(2) + (v - cy).powi(2)).sqrt();
            let p = (u * 6.0 + t).sin() + (v * 5.0 - t * 1.3).sin() + ((u + v) * 4.0 + t * 0.7).sin() + (d * 9.0 - t * 1.7).sin();
            let a = p * std::f64::consts::PI / 2.0;
            let ch = |ph: f64| ((0.5 + 0.5 * (a + ph).sin()) * 255.0).round() as u8;
            let o = ((y * w + x) * 4) as usize;
            out[o..o + 4].copy_from_slice(&[ch(0.0), ch(2.094), ch(4.188), 255]);
        }
    }
}

/// The counter's text at second `t` of the clip.
pub fn counter_text(spec: &GenSpec, t: f64) -> String {
    let start = spec.start.unwrap_or(if spec.down { spec.duration } else { 0.0 });
    let v = if spec.down { (start - t).max(0.0) } else { start + t };
    match spec.format {
        CounterFormat::Frames => format!("{}", (v * spec.fps + 1e-6).floor() as u64),
        CounterFormat::Seconds => {
            // counting down shows the second it is in (10 for 9.5 s left), as a countdown does
            let s = if spec.down { (v - 1e-6).ceil().max(0.0) } else { (v + 1e-6).floor() };
            format!("{}", s as u64)
        }
        CounterFormat::Clock | CounterFormat::Precise => {
            let precise = spec.format == CounterFormat::Precise;
            let total = if precise {
                (v * 100.0 + 1e-6).floor() as u64
            } else if spec.down {
                ((v - 1e-6).ceil().max(0.0) as u64) * 100
            } else {
                ((v + 1e-6).floor() as u64) * 100
            };
            let (cs, secs) = (total % 100, total / 100);
            let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
            let hms = if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") };
            if precise {
                format!("{hms}.{cs:02}")
            } else {
                hms
            }
        }
    }
}

/// The characters a counter can show (what to rasterise up front).
pub const COUNTER_CHARS: &str = "0123456789:.";

/// One rasterised character: straight RGBA, `w` x `h`.
#[derive(Clone, Debug)]
pub struct Glyph {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// Paint `bg` everywhere, then `text` centred, digits in equal cells (the widest digit's width) so the
/// number does not jitter as it changes. Straight-alpha source-over.
pub fn compose_text(glyphs: &std::collections::HashMap<char, Glyph>, text: &str, bg: [u8; 4], w: u32, h: u32, out: &mut [u8]) {
    for px in out.chunks_exact_mut(4).take((w * h) as usize) {
        px.copy_from_slice(&bg);
    }
    let cell = ('0'..='9').filter_map(|c| glyphs.get(&c)).map(|g| g.w).max().unwrap_or(0);
    let width_of = |c: char| if c.is_ascii_digit() { cell } else { glyphs.get(&c).map_or(0, |g| g.w) };
    let total: u32 = text.chars().map(width_of).sum();
    // glyphs come cropped to their ink (`TextRasterizer`): each is centred on the line, except '.',
    // which sits on the digits' baseline
    let dh = ('0'..='9').filter_map(|c| glyphs.get(&c)).map(|g| g.h).max().unwrap_or(0) as i64;
    let digits_top = (h as i64 - dh) / 2;
    let mut x0 = (w as i64 - total as i64) / 2;
    for c in text.chars() {
        let cw = width_of(c);
        if let Some(g) = glyphs.get(&c) {
            let gx = x0 + (cw as i64 - g.w as i64) / 2;
            let y0 = if c == '.' && dh > 0 { digits_top + dh - g.h as i64 } else { (h as i64 - g.h as i64) / 2 };
            for yy in 0..g.h as i64 {
                let y = y0 + yy;
                if y < 0 || y >= h as i64 {
                    continue;
                }
                for xx in 0..g.w as i64 {
                    let x = gx + xx;
                    if x < 0 || x >= w as i64 {
                        continue;
                    }
                    let s = ((yy * g.w as i64 + xx) * 4) as usize;
                    let d = ((y * w as i64 + x) * 4) as usize;
                    let sa = g.rgba[s + 3] as f32 / 255.0;
                    if sa <= 0.0 {
                        continue;
                    }
                    let da = out[d + 3] as f32 / 255.0;
                    let oa = sa + da * (1.0 - sa);
                    for k in 0..3 {
                        let v = (g.rgba[s + k] as f32 * sa + out[d + k] as f32 * da * (1.0 - sa)) / oa.max(1e-6);
                        out[d + k] = v.round().clamp(0.0, 255.0) as u8;
                    }
                    out[d + 3] = (oa * 255.0).round() as u8;
                }
            }
        }
        x0 += cw as i64;
    }
}

/// Draw frame `i` of `spec` into `out` (RGBA, width x height). Counter needs its `glyphs`.
pub fn draw(spec: &GenSpec, i: usize, glyphs: &std::collections::HashMap<char, Glyph>, out: &mut [u8]) {
    let t = i as f64 / spec.fps;
    match spec.kind {
        GenKind::Bars => bars(spec.width, spec.height, out),
        GenKind::Noise => noise(spec.width, spec.height, i as u64, spec.seed, spec.colour, out),
        GenKind::Plasma => plasma(spec.width, spec.height, t, spec.speed, out),
        GenKind::Counter => compose_text(glyphs, &counter_text(spec, t), spec.background, spec.width, spec.height, out),
    }
}

// ---------------------------------------------------------------------------------------------------
// the encoder

/// ffmpeg arguments (after the executable) that read raw RGBA frames of `size` at `fps` from stdin and
/// write `out`: a PNG for a still, H.264 4:2:0 (CRF 18, a keyframe every half second, no B-frames:
/// cheap to scrub) for a clip, colour stacked over alpha when `alpha`.
pub fn encode_args(out: &Path, size: (u32, u32), fps: f64, still: bool, alpha: bool) -> Vec<String> {
    let wh = format!("{}x{}", size.0, size.1);
    let rate = format!("{}/1000", (fps * 1000.0).round().max(1.0));
    let mut a: Vec<String> = ["-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &wh, "-r", &rate, "-i", "pipe:0"]
        .map(String::from)
        .to_vec();
    if still {
        a.extend(["-frames:v", "1", "-c:v", "png", "-pix_fmt", "rgba"].map(String::from));
    } else {
        let vf = if alpha {
            crate::media::proxy::stacked_filter_to(&format!("{}:{}", size.0, size.1))
        } else {
            "format=yuv420p".to_string()
        };
        let gop = ((fps / 2.0).round() as u32).max(1).to_string();
        a.extend(["-vf".to_string(), vf]);
        a.extend(
            ["-c:v", "libx264", "-preset", "medium", "-crf", "18", "-g", &gop, "-bf", "0", "-pix_fmt", "yuv420p", "-an", "-movflags", "+faststart", "-f", "mp4"]
                .map(String::from),
        );
    }
    a.push(out.to_string_lossy().into_owned());
    a
}

/// Encode `frames` frames of `size` into `out` (see `encode_args`), drawing each with `draw(i, buf)`.
/// Progress and cancel through `prog`; written to a temp name and renamed, so `out` never exists
/// half-written.
pub fn encode(
    out: &Path,
    size: (u32, u32),
    fps: f64,
    frames: usize,
    still: bool,
    alpha: bool,
    prog: &crate::engine::export::Progress,
    mut draw: impl FnMut(usize, &mut [u8]) -> Result<(), String>,
) -> Result<(), String> {
    let exe = crate::media::ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let name = out.file_name().unwrap_or_default().to_string_lossy();
    let tmp = out.with_file_name(format!("partial-{name}"));
    let mut child = crate::media::ffpipe::command(&exe)
        .args(encode_args(&tmp, size, fps, still, alpha))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let mut buf = vec![0u8; (size.0 * size.1 * 4) as usize];
    let n = if still { 1 } else { frames.max(1) };
    let mut failed: Option<String> = None;
    {
        let mut stdin = child.stdin.take().ok_or("ffmpeg: no stdin")?;
        for i in 0..n {
            if prog.is_cancelled() {
                failed = Some("cancelled".into());
                break;
            }
            if let Err(e) = draw(i, &mut buf) {
                failed = Some(e);
                break;
            }
            if stdin.write_all(&buf).is_err() {
                break; // ffmpeg ended early: its own error says why
            }
            prog.set((i + 1) as f32 / n as f32, format!("Frame {} of {n}", i + 1));
        }
    }
    let o = child.wait_with_output().map_err(|e| format!("ffmpeg: {e}"))?;
    if let Some(e) = failed {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if !o.status.success() {
        let _ = std::fs::remove_file(&tmp);
        let err = String::from_utf8_lossy(&o.stderr);
        return Err(format!("ffmpeg: {}", err.lines().last().unwrap_or("failed").trim()));
    }
    std::fs::rename(&tmp, out).map_err(|e| format!("{}: {e}", out.display()))
}

/// Render a whole Lottie animation `comp` into `out` at its own size and rate. `alpha` decides the
/// layout (`lottie_alpha` asks). A still animation is a PNG.
pub fn encode_lottie(comp: &crate::engine::lottie::Comp, out: &Path, alpha: bool, prog: &crate::engine::export::Progress) -> Result<(), String> {
    let size = crate::engine::lottie::output_size(comp);
    let still = !comp.animated();
    let mut px = Vec::new();
    encode(out, size, comp.fps, comp.frames(), still, alpha, prog, |i, buf| {
        comp.render(comp.ip + i as f64, size.0, size.1, &mut px);
        buf.copy_from_slice(&px);
        Ok(())
    })
}

/// Does a Lottie animation have transparency? Asked of five moments, at a small size.
pub fn lottie_alpha(comp: &crate::engine::lottie::Comp) -> bool {
    let (w, h) = (64, ((64.0 * comp.height as f64 / comp.width as f64).round() as u32).clamp(2, 256));
    let mut px = Vec::new();
    let n = comp.frames();
    [0, n / 4, n / 2, 3 * n / 4, n.saturating_sub(1)].iter().any(|&i| {
        comp.render(comp.ip + i as f64, w, h, &mut px);
        px.chunks_exact(4).any(|p| p[3] < 255)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn kinds_parse_and_specs_clamp() {
        assert_eq!(GenKind::parse("Colour Bars"), Some(GenKind::Bars));
        assert_eq!(GenKind::parse("color_bars"), Some(GenKind::Bars));
        assert_eq!(GenKind::parse("timer"), Some(GenKind::Counter));
        assert_eq!(GenKind::parse("NOISE"), Some(GenKind::Noise));
        assert_eq!(GenKind::parse("fog"), None);
        for k in GenKind::ALL {
            assert_eq!(GenKind::parse(k.label()), Some(k), "{k:?} round-trips by its label");
        }
        let s = GenSpec { duration: -1.0, fps: f64::NAN, ..GenSpec::new(GenKind::Plasma, 1921, 5, 30.0) }.clamped();
        assert_eq!((s.width, s.height, s.fps, s.duration), (1922, 16, 30.0, 0.1));
        assert_eq!(GenSpec::new(GenKind::Noise, 9999, 9999, 500.0).clamped().width, 3840);
        assert_eq!(GenSpec::new(GenKind::Bars, 64, 64, 30.0).file_ext(), "png");
        assert_eq!(GenSpec::new(GenKind::Counter, 64, 64, 30.0).file_ext(), "stacked.mp4");
        let solid = GenSpec { background: [0, 0, 0, 255], ..GenSpec::new(GenKind::Counter, 64, 64, 30.0) };
        assert_eq!(solid.file_ext(), "mp4");
        assert_eq!(GenSpec::new(GenKind::Plasma, 64, 64, 30.0).frames(), 150);
    }

    #[test]
    fn counters_count() {
        let mut s = GenSpec::new(GenKind::Counter, 64, 64, 25.0);
        assert_eq!(counter_text(&s, 0.0), "0:00");
        assert_eq!(counter_text(&s, 61.9), "1:01");
        assert_eq!(counter_text(&s, 3725.0), "1:02:05");
        s.format = CounterFormat::Precise;
        assert_eq!(counter_text(&s, 1.234), "0:01.23");
        s.format = CounterFormat::Frames;
        assert_eq!(counter_text(&s, 2.0), "50");
        // a countdown from the clip's length, ending on 0
        s.down = true;
        s.duration = 10.0;
        s.format = CounterFormat::Seconds;
        assert_eq!(counter_text(&s, 0.0), "10");
        assert_eq!(counter_text(&s, 0.5), "10", "9.5 s left still reads 10");
        assert_eq!(counter_text(&s, 9.5), "1");
        assert_eq!(counter_text(&s, 10.0), "0");
        assert_eq!(counter_text(&s, 12.0), "0", "never below zero");
        s.format = CounterFormat::Clock;
        s.start = Some(90.0);
        assert_eq!(counter_text(&s, 0.0), "1:30");
        assert_eq!(counter_text(&s, 0.2), "1:30");
        assert_eq!(counter_text(&s, 1.0), "1:29");
        s.down = false;
        assert_eq!(counter_text(&s, 30.0), "2:00", "up from a start value");
    }

    fn px(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    #[test]
    fn bars_noise_and_plasma_draw() {
        let (w, h) = (140u32, 100u32);
        let mut buf = vec![0u8; (w * h * 4) as usize];
        bars(w, h, &mut buf);
        assert_eq!(px(&buf, w, 5, 10), [191, 191, 191, 255], "75% grey first");
        assert_eq!(px(&buf, w, 135, 10), [0, 0, 191, 255], "blue last");
        assert_eq!(px(&buf, w, 5, 70), [0, 0, 191, 255], "the reverse blue castellation");
        assert_eq!(px(&buf, w, 30, 90), [255, 255, 255, 255], "the white block below");
        let mut a = vec![0u8; (w * h * 4) as usize];
        let mut b = a.clone();
        noise(w, h, 3, 7, false, &mut a);
        noise(w, h, 3, 7, false, &mut b);
        assert_eq!(a, b, "the same frame and seed: the same static");
        noise(w, h, 4, 7, false, &mut b);
        assert_ne!(a, b, "a new pattern each frame");
        assert!(a.chunks_exact(4).all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 255), "grey and opaque");
        let mean = a.chunks_exact(4).map(|p| p[0] as f64).sum::<f64>() / (w * h) as f64;
        assert!((mean - 127.5).abs() < 8.0, "{mean}");
        noise(w, h, 3, 7, true, &mut b);
        assert!(b.chunks_exact(4).any(|p| p[0] != p[1]), "colour noise is coloured");
        plasma(w, h, 0.0, 1.0, &mut a);
        plasma(w, h, 1.0, 1.0, &mut b);
        assert_ne!(a, b, "it flows");
        plasma(w, h, 1.0, 0.0, &mut b);
        assert_eq!(a, b, "speed 0 holds still");
        assert!(a.chunks_exact(4).all(|p| p[3] == 255));
    }

    #[test]
    fn text_is_centred_in_steady_cells() {
        let glyph = |w: u32, v: u8| Glyph { w, h: 4, rgba: [v, v, v, 255].repeat((w * 4) as usize) };
        let mut g: HashMap<char, Glyph> = HashMap::new();
        g.insert('1', glyph(2, 200));
        g.insert('8', glyph(4, 100));
        g.insert(':', glyph(2, 50));
        g.insert('.', Glyph { w: 2, h: 1, rgba: [70, 70, 70, 255].repeat(2) });
        let (w, h) = (20u32, 8u32);
        let mut buf = vec![0u8; (w * h * 4) as usize];
        compose_text(&g, "1:8", [0, 0, 0, 0], w, h, &mut buf);
        // cells of 4 (the widest digit), 2 for ':' => 10 wide, starting at x 5, rows 2..6
        assert_eq!(px(&buf, w, 4, 3)[3], 0);
        assert_eq!(px(&buf, w, 6, 3), [200, 200, 200, 255], "'1' centred in its 4-wide cell: x 6..8");
        assert_eq!(px(&buf, w, 5, 3)[3], 0);
        assert_eq!(px(&buf, w, 9, 3), [50, 50, 50, 255], "':' at x 9..11");
        assert_eq!(px(&buf, w, 11, 3), [100, 100, 100, 255], "'8' at x 11..15");
        assert_eq!(px(&buf, w, 11, 1)[3], 0, "vertically centred");
        compose_text(&g, "8.", [0, 0, 0, 0], w, h, &mut buf);
        assert_eq!(px(&buf, w, 11, 5), [70, 70, 70, 255], "'.' on the digits' bottom row (5)");
        assert_eq!(px(&buf, w, 11, 3)[3], 0);
        compose_text(&g, "1", [9, 9, 9, 255], w, h, &mut buf);
        assert_eq!(px(&buf, w, 0, 0), [9, 9, 9, 255], "the background fills the rest");
    }

    #[test]
    fn encode_args_pick_the_container() {
        let still = encode_args(Path::new("C:/g/Bars.png"), (64, 48), 30.0, true, false).join(" ");
        assert!(still.contains("-f rawvideo -pix_fmt rgba -s 64x48 -r 30000/1000 -i pipe:0") && still.contains("-frames:v 1 -c:v png"), "{still}");
        let clip = encode_args(Path::new("C:/g/P.mp4"), (64, 48), 25.0, false, false).join(" ");
        assert!(clip.contains("-vf format=yuv420p -c:v libx264") && clip.contains("-g 13 -bf 0") && clip.ends_with("C:/g/P.mp4"), "{clip}");
        let alpha = encode_args(Path::new("C:/g/C.stacked.mp4"), (64, 48), 25.0, false, true).join(" ");
        assert!(alpha.contains("alphaextract") && alpha.contains("vstack") && alpha.contains("scale=64:48"), "{alpha}");
        assert_eq!(output_ext(false, true), "stacked.mp4");
    }

    /// With ffmpeg on the PATH: a short plasma clip, a bars PNG and a stacked counter clip really encode.
    #[test]
    fn encodes_with_real_ffmpeg() {
        if crate::media::ffpipe::ffmpeg_exe().is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("se-synth-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let prog = crate::engine::export::Progress::new();
        let spec = GenSpec { duration: 0.5, ..GenSpec::new(GenKind::Plasma, 64, 48, 10.0) }.clamped();
        let out = dir.join("Plasma.mp4");
        encode(&out, (spec.width, spec.height), spec.fps, spec.frames(), false, false, &prog, |i, b| {
            draw(&spec, i, &HashMap::new(), b);
            Ok(())
        })
        .unwrap();
        assert!(out.metadata().unwrap().len() > 100);
        assert!(!dir.join("partial-Plasma.mp4").exists());
        let png = dir.join("Bars.png");
        let bars_spec = GenSpec::new(GenKind::Bars, 64, 48, 10.0);
        encode(&png, (64, 48), 10.0, 1, true, false, &prog, |i, b| {
            draw(&bars_spec, i, &HashMap::new(), b);
            Ok(())
        })
        .unwrap();
        assert_eq!(&std::fs::read(&png).unwrap()[1..4], b"PNG");
        let st = dir.join("C.stacked.mp4");
        encode(&st, (64, 48), 10.0, 5, false, true, &prog, |_, b| {
            b.fill(0);
            Ok(())
        })
        .unwrap();
        assert!(st.exists());
        // a draw error stops it and leaves nothing behind
        let bad = dir.join("Bad.mp4");
        let e = encode(&bad, (64, 48), 10.0, 5, false, false, &prog, |i, _| if i == 2 { Err("boom".into()) } else { Ok(()) });
        assert_eq!(e.unwrap_err(), "boom");
        assert!(!bad.exists() && !dir.join("partial-Bad.mp4").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
