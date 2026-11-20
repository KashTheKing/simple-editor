//! ---- feat/styles-context ----
//! "Learn this style": measure a reference video and write a draft Editing Style (`engine::styles`).
//!
//! Every number comes from our own analysis of the file, opened read-only:
//! - shots: `analysis::scene_cuts` (ffmpeg scene score) for hard cuts, plus dissolves found in a 10 fps,
//!   64x36 picture stream (a frame that is the average of the frames either side) and dips to black;
//! - pacing: shot lengths, their average / median, and the average per quarter;
//! - cut on beat: onsets of the sound (`analysis::onsets` over 100-per-second peaks) within two frames
//!   of a cut, counted only when the sound has a steady beat (`analysis::bpm`);
//! - loudness: ffmpeg's EBU R128 integrated loudness;
//! - colour: the scopes' readings over the same small frames (luma level and range, saturation, warm/cool);
//! - words per minute: a whisper transcript when speech-to-text is installed and the file has speech.
//! Optionally SmolVLM2 (`engine::vlm`) describes each shot - framing, subject, camera, on-screen text - and
//! those words go into the prose and the shot table; it never sets a number.

use crate::engine::analysis;
use crate::engine::export::{self, Progress};
use crate::engine::styles::{self, Rules, Source, Style};
use crate::engine::vlm::{self, Description};
use crate::media::ffpipe;
use crate::media::waveform::{Peaks, PEAKS_PER_SEC};
use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

/// The picture stream the colour / transition analysis reads.
const FPS: f64 = 10.0;
const W: usize = 64;
const H: usize = 36;
/// ffmpeg scene score above which a frame starts a new shot.
pub const SCENE_THR: f32 = 0.3;
/// Mean luma under which a frame counts as black (0..1).
const BLACK: f32 = 0.06;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    Cut,
    Dissolve,
    /// Through black (a fade out then in, or a dip).
    DipToBlack,
}

impl Join {
    pub fn name(self) -> &'static str {
        match self {
            Join::Cut => "cut",
            Join::Dissolve => "dissolve",
            Join::DipToBlack => "dip to black",
        }
    }
}

/// Where one shot ends and the next begins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Boundary {
    pub t: f64,
    pub join: Join,
    /// How long the transition takes (0 for a cut).
    pub dur: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Colour {
    /// Mean luma 0..1, its spread (contrast, the standard deviation), mean saturation 0..1, and warmth
    /// (mean red minus blue, -1..1).
    pub luma: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub warmth: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shot {
    pub start: f64,
    pub end: f64,
    pub colour: Colour,
    /// The join that starts this shot (None for the first).
    pub join_in: Option<Join>,
    pub described: Option<Description>,
}

impl Shot {
    pub fn len(&self) -> f64 {
        self.end - self.start
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measure {
    pub file: String,
    pub duration: f64,
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    pub shots: Vec<Shot>,
    pub boundaries: Vec<Boundary>,
    pub bpm: Option<f64>,
    /// Share of boundaries within two frames of an onset; None without a steady beat.
    pub cut_on_beat: Option<f64>,
    pub loudness: Option<f64>,
    pub colour: Colour,
    pub wpm: Option<f64>,
    /// What could not be measured and why ("no audio stream", "speech-to-text is not installed").
    pub notes: Vec<String>,
}

// ---------------------------------------------------------------------------------------------------
// pure parts (tested on their own)

/// Shots between 0, the boundaries and `duration` (boundaries closer than 0.2 s to another or to the
/// ends are dropped).
pub fn shots_from(boundaries: &[Boundary], duration: f64) -> Vec<Shot> {
    let mut b: Vec<Boundary> = boundaries.iter().copied().filter(|b| b.t > 0.2 && b.t < duration - 0.2).collect();
    b.sort_by(|x, y| x.t.total_cmp(&y.t));
    b.dedup_by(|x, y| x.t - y.t < 0.2);
    let mut out = Vec::new();
    let mut start = 0.0;
    let mut join = None;
    for x in b {
        out.push(Shot { start, end: x.t, join_in: join, ..Default::default() });
        start = x.t;
        join = Some(x.join);
    }
    out.push(Shot { start, end: duration.max(start), join_in: join, ..Default::default() });
    out
}

pub fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

pub fn median(v: &[f64]) -> f64 {
    let mut v = v.to_vec();
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let m = v.len() / 2;
    if v.len() % 2 == 0 {
        (v[m - 1] + v[m]) / 2.0
    } else {
        v[m]
    }
}

/// Average shot length in each quarter of the video (a shot counts toward the quarter holding its middle;
/// an empty quarter takes the overall average).
pub fn pacing(shots: &[Shot], duration: f64) -> Vec<f64> {
    let all = mean(&shots.iter().map(Shot::len).collect::<Vec<_>>());
    (0..4)
        .map(|q| {
            let (a, b) = (duration * q as f64 / 4.0, duration * (q + 1) as f64 / 4.0);
            let lens: Vec<f64> =
                shots.iter().filter(|s| (s.start + s.end) / 2.0 >= a && (s.start + s.end) / 2.0 < b).map(Shot::len).collect();
            if lens.is_empty() {
                all
            } else {
                mean(&lens)
            }
        })
        .collect()
}

/// Share of `cuts` with an onset within `tol` seconds.
pub fn on_beat_ratio(cuts: &[f64], onsets: &[f64], tol: f64) -> f64 {
    if cuts.is_empty() {
        return 0.0;
    }
    let hit = cuts.iter().filter(|&&c| onsets.iter().any(|&o| (o - c).abs() <= tol)).count();
    hit as f64 / cuts.len() as f64
}

/// 100-per-second peaks from mono samples at `rate` (the waveform module's format).
pub fn peaks_of(samples: &[f32], rate: u32) -> Peaks {
    let per = (rate / PEAKS_PER_SEC).max(1) as usize;
    let mut p = Peaks { min: Vec::new(), max: Vec::new() };
    for c in samples.chunks(per) {
        p.min.push(c.iter().copied().fold(0.0, f32::min));
        p.max.push(c.iter().copied().fold(0.0, f32::max));
    }
    p
}

/// One small RGB frame's luma, spread, saturation and warmth.
pub fn colour_of(rgb: &[u8]) -> Colour {
    let n = (rgb.len() / 3).max(1) as f32;
    let (mut sum, mut sq, mut sat, mut warm) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for px in rgb.chunks_exact(3) {
        let [r, g, b] = [px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0];
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        sum += y;
        sq += y * y;
        let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
        sat += if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
        warm += r - b;
    }
    let luma = sum / n;
    Colour { luma, contrast: (sq / n - luma * luma).max(0.0).sqrt(), saturation: sat / n, warmth: warm / n }
}

fn avg_colour(cs: &[Colour]) -> Colour {
    let n = cs.len().max(1) as f32;
    Colour {
        luma: cs.iter().map(|c| c.luma).sum::<f32>() / n,
        contrast: cs.iter().map(|c| c.contrast).sum::<f32>() / n,
        saturation: cs.iter().map(|c| c.saturation).sum::<f32>() / n,
        warmth: cs.iter().map(|c| c.warmth).sum::<f32>() / n,
    }
}

fn diff(a: &[u8], b: &[u8]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (*x as f32 - *y as f32).abs()).sum::<f32>() / a.len().max(1) as f32 / 255.0
}

/// Dissolves in a picture stream (`frames` at `fps`): a run of frames each close to the blend of the
/// frames on either side of the run, between two pictures that differ a lot. A pan or a zoom changes as
/// much but is no blend, so it is not taken. Skips runs that already hold a hard cut. -> (centre, length).
pub fn dissolves(frames: &[Vec<u8>], fps: f64, cuts: &[f64]) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let n = frames.len();
    // try runs of 2..=12 frames (0.2-1.2 s at 10 fps), longest-first per start so one dissolve is one find
    let mut i = 1;
    while i + 2 < n {
        let mut found = None;
        for len in (2..=12usize).rev() {
            let (a, b) = (i - 1, i + len);
            if b >= n {
                continue;
            }
            let span = diff(&frames[a], &frames[b]);
            if span < 0.12 {
                continue;
            }
            let ok = (i..b).all(|k| {
                let w = (k - a) as f32 / (b - a) as f32;
                let blend: Vec<u8> =
                    frames[a].iter().zip(&frames[b]).map(|(x, y)| (*x as f32 * (1.0 - w) + *y as f32 * w).round() as u8).collect();
                diff(&frames[k], &blend) < span * 0.18
            }) && (i..=b).all(|k| diff(&frames[k - 1], &frames[k]) < span * 0.7);
            if ok {
                let (t0, t1) = (a as f64 / fps, b as f64 / fps);
                if !cuts.iter().any(|&c| c > t0 && c < t1) {
                    found = Some(((t0 + t1) / 2.0, t1 - t0, b));
                }
                break;
            }
        }
        match found {
            Some((t, d, b)) => {
                out.push((t, d));
                i = b + 1;
            }
            None => i += 1,
        }
    }
    out
}

/// A hard cut whose neighbourhood (±1 s) goes through black is a dip to black; its length is the time spent dark.
pub fn classify_cut(t: f64, lumas: &[f32], fps: f64) -> Boundary {
    let at = |s: f64| ((s * fps).round().max(0.0) as usize).min(lumas.len().saturating_sub(1));
    if lumas.is_empty() {
        return Boundary { t, join: Join::Cut, dur: 0.0 };
    }
    let (a, b) = (at(t - 1.0), at(t + 1.0));
    let dark = (a..=b).filter(|&k| lumas[k] < BLACK).count();
    let before = lumas[a..at(t)].iter().copied().fold(0.0, f32::max);
    let after = lumas[at(t)..=b].iter().copied().fold(0.0, f32::max);
    if dark > 0 && before > BLACK * 2.0 && after > BLACK * 2.0 {
        Boundary { t, join: Join::DipToBlack, dur: dark as f64 / fps }
    } else {
        Boundary { t, join: Join::Cut, dur: 0.0 }
    }
}

/// `I: -14.2 LUFS` from ffmpeg's ebur128 summary (the last one printed).
pub fn parse_lufs(stderr: &str) -> Option<f64> {
    stderr.lines().rev().find_map(|l| {
        let l = l.trim();
        let v = l.strip_prefix("I:")?.trim().strip_suffix("LUFS")?.trim();
        v.parse::<f64>().ok().filter(|v| v.is_finite() && *v > -70.0)
    })
}

// ---------------------------------------------------------------------------------------------------
// reading the file

/// A number no other temp file of this process used.
fn counter() -> u64 {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn ffmpeg() -> Result<PathBuf, String> {
    ffpipe::ffmpeg_exe().ok_or("ffmpeg not found".into())
}

/// Every small RGB frame at `FPS` (read-only: ffmpeg only reads the file).
fn frames(path: &Path, prog: &Progress) -> Result<Vec<Vec<u8>>, String> {
    let mut child = crate::media::bg::command(&ffmpeg()?)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-an", "-vf", &format!("fps={FPS},scale={W}:{H}:flags=area"), "-pix_fmt", "rgb24", "-f", "rawvideo", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let mut out = child.stdout.take().ok_or("no ffmpeg output")?;
    let mut frames = Vec::new();
    let mut buf = vec![0u8; W * H * 3];
    while out.read_exact(&mut buf).is_ok() {
        frames.push(buf.clone());
        if prog.is_cancelled() {
            let _ = child.kill();
            return Err(export::CANCELLED.into());
        }
    }
    let _ = child.wait();
    Ok(frames)
}

/// Mono f32 samples at `rate` (empty when the file has no sound).
fn samples(path: &Path, rate: u32) -> Result<Vec<f32>, String> {
    let out = crate::media::bg::command(&ffmpeg()?)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", &rate.to_string(), "-f", "f32le", "pipe:1"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    Ok(out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
}

fn loudness(path: &Path) -> Option<f64> {
    let out = crate::media::bg::command(&ffmpeg().ok()?)
        .args(["-nostdin", "-hide_banner", "-i"])
        .arg(path)
        .args(["-vn", "-af", "ebur128", "-f", "null", "-"])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    parse_lufs(&String::from_utf8_lossy(&out.stderr))
}

/// Words per minute from a whisper transcript, when speech-to-text and its model are installed.
fn words_per_minute(path: &Path, duration: f64, notes: &mut Vec<String>) -> Option<f64> {
    if crate::engine::transcribe::exe().is_none() {
        notes.push("words per minute: speech-to-text is not installed".into());
        return None;
    }
    let wav = std::env::temp_dir().join(format!("se-learn-{}-{}.wav", std::process::id(), counter()));
    let ok = crate::media::bg::command(&ffmpeg().ok()?)
        .args(["-v", "error", "-nostdin", "-y", "-i"])
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", "16000"])
        .arg(&wav)
        .stdin(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    let text = if ok { crate::engine::transcribe::plain_text(&wav) } else { Err("no sound".into()) };
    let _ = std::fs::remove_file(&wav);
    match text {
        Ok(t) => {
            let words = t.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).count();
            if words < 20 || duration < 10.0 {
                notes.push("words per minute: little or no speech".into());
                None
            } else {
                Some(words as f64 / (duration / 60.0))
            }
        }
        Err(e) => {
            notes.push(format!("words per minute: {e}"));
            None
        }
    }
}

/// Measure `path` (read-only). `speech`: try a transcript for words per minute.
pub fn measure(path: &Path, speech: bool, prog: &Progress) -> Result<Measure, String> {
    let asset = ffpipe::probe(&path.to_string_lossy())?;
    let duration = asset.duration;
    if !(duration > 0.0) {
        return Err("the file has no length (is it a video?)".into());
    }
    let mut m = Measure {
        file: path.to_string_lossy().to_string(),
        duration,
        fps: asset.fps,
        width: asset.width,
        height: asset.height,
        ..Default::default()
    };
    prog.set(0.05, "Finding the cuts…");
    let cuts = analysis::scene_cuts(path, SCENE_THR)?;
    if prog.is_cancelled() {
        return Err(export::CANCELLED.into());
    }
    prog.set(0.25, "Reading the picture…");
    let frames = frames(path, prog)?;
    let colours: Vec<Colour> = frames.iter().map(|f| colour_of(f)).collect();
    let lumas: Vec<f32> = colours.iter().map(|c| c.luma).collect();
    let mut bounds: Vec<Boundary> = cuts.iter().map(|&t| classify_cut(t, &lumas, FPS)).collect();
    // a dip to black reads as two cuts (into and out of black): keep one, in the middle
    bounds.sort_by(|a, b| a.t.total_cmp(&b.t));
    let mut merged: Vec<Boundary> = Vec::new();
    for b in bounds {
        match merged.last_mut() {
            Some(l) if l.join == Join::DipToBlack && b.join == Join::DipToBlack && b.t - l.t < 1.5 => l.t = (l.t + b.t) / 2.0,
            _ => merged.push(b),
        }
    }
    for (t, d) in dissolves(&frames, FPS, &cuts) {
        merged.push(Boundary { t, join: Join::Dissolve, dur: d });
    }
    merged.sort_by(|a, b| a.t.total_cmp(&b.t));
    m.shots = shots_from(&merged, duration);
    m.boundaries = merged.into_iter().filter(|b| b.t > 0.2 && b.t < duration - 0.2).collect();
    for s in &mut m.shots {
        let (a, b) = ((s.start * FPS) as usize, ((s.end * FPS) as usize).min(colours.len()));
        if a < b {
            s.colour = avg_colour(&colours[a..b]);
        }
    }
    m.colour = avg_colour(&colours);
    prog.set(0.45, "Listening for the beat…");
    let rate = 8000;
    let audio = samples(path, rate)?;
    if audio.is_empty() {
        m.notes.push("no sound: no beat or loudness".into());
    } else {
        let peaks = peaks_of(&audio, rate);
        let onsets = analysis::onsets(&peaks, 0.0, duration, 0.25, 1.6);
        m.bpm = analysis::bpm(&onsets);
        let frame = if m.fps > 0.0 { 1.0 / m.fps } else { 1.0 / 30.0 };
        let cut_times: Vec<f64> = m.boundaries.iter().map(|b| b.t).collect();
        m.cut_on_beat = match m.bpm {
            Some(_) if !cut_times.is_empty() => Some(on_beat_ratio(&cut_times, &onsets, (2.0 * frame).max(0.07))),
            Some(_) => None,
            None => {
                m.notes.push("cut on beat: the sound has no steady beat".into());
                Some(0.0)
            }
        };
        prog.set(0.55, "Measuring loudness…");
        m.loudness = loudness(path);
        if speech {
            prog.set(0.6, "Transcribing for the speech rate…");
            m.wpm = words_per_minute(path, duration, &mut m.notes);
        }
    }
    Ok(m)
}

/// Three frames of a shot (start, middle, end) side by side, 240 px high, as a PNG for the describer.
fn shot_strip(path: &Path, s: &Shot, out: &Path) -> Result<(), String> {
    let pad = (s.len() * 0.1).min(0.3);
    let ts = [s.start + pad, (s.start + s.end) / 2.0, (s.end - pad).max(s.start)];
    let mut c = crate::media::bg::command(&ffmpeg()?);
    c.args(["-v", "error", "-nostdin", "-y"]);
    for t in ts {
        c.args(["-ss", &format!("{t:.3}"), "-i"]).arg(path);
    }
    let ok = c
        .args(["-filter_complex", "[0:v]scale=-2:240[a];[1:v]scale=-2:240[b];[2:v]scale=-2:240[c];[a][b][c]hstack=3", "-frames:v", "1"])
        .arg(out)
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if ok.success() && out.is_file() {
        Ok(())
    } else {
        Err("could not take the shot's frames".into())
    }
}

/// Describe each shot with the describer (`model` id). At most `max` shots, spread evenly over the video.
pub fn describe_shots(path: &Path, m: &mut Measure, model: &str, max: usize, prog: &Progress, at: (f32, f32)) -> Result<(), String> {
    let n = m.shots.len();
    let pick: Vec<usize> = if n <= max { (0..n).collect() } else { (0..max).map(|i| i * n / max).collect() };
    let dir = std::env::temp_dir().join(format!("se-learn-{}-{}", std::process::id(), counter()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let r = (|| {
        for (k, &i) in pick.iter().enumerate() {
            if prog.is_cancelled() {
                return Err(export::CANCELLED.to_string());
            }
            crate::media::bg::pause_point(|| prog.is_cancelled()); // playback first
            prog.set(at.0 + (at.1 - at.0) * k as f32 / pick.len() as f32, format!("Describing shot {} of {}…", k + 1, pick.len()));
            let png = dir.join(format!("shot{i}.png"));
            shot_strip(path, &m.shots[i], &png)?;
            m.shots[i].described = Some(vlm::describe(&png, model)?);
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    r
}

// ---------------------------------------------------------------------------------------------------
// the draft

fn mmss(t: f64) -> String {
    format!("{}:{:04.1}", (t / 60.0) as u64, t % 60.0)
}

fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Transition list for the front matter: kinds by how often they occur, with their typical length.
pub fn transitions_of(m: &Measure) -> Vec<(String, f64)> {
    let total = m.boundaries.len().max(1) as f64;
    let mut out: Vec<(String, f64)> = Vec::new();
    for j in [Join::Cut, Join::Dissolve, Join::DipToBlack] {
        let of: Vec<&Boundary> = m.boundaries.iter().filter(|b| b.join == j).collect();
        if of.is_empty() {
            continue;
        }
        let share = of.len() as f64 / total;
        let name = if j == Join::Cut {
            "cut".to_string()
        } else {
            format!("{} {}", j.name(), r2(median(&of.iter().map(|b| b.dur).collect::<Vec<_>>())))
        };
        out.push((name, share));
    }
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    out
}

fn colour_words(c: &Colour) -> String {
    let level = match c.luma {
        l if l < 0.3 => "dark",
        l if l > 0.6 => "bright",
        _ => "mid-key",
    };
    let contrast = if c.contrast > 0.25 { "high contrast" } else if c.contrast < 0.12 { "soft contrast" } else { "normal contrast" };
    let sat = match c.saturation {
        s if s < 0.15 => "muted colour",
        s if s > 0.45 => "saturated colour",
        _ => "natural colour",
    };
    let temp = match c.warmth {
        w if w > 0.06 => ", warm",
        w if w < -0.06 => ", cool",
        _ => "",
    };
    format!("{level}, {contrast}, {sat}{temp}")
}

/// The draft style: rules from the measurements, prose from them plus the describer's words, and a
/// shot table. `name` is slugged; the user reviews it before it is saved.
pub fn draft(m: &Measure, name: &str) -> Style {
    let lens: Vec<f64> = m.shots.iter().map(Shot::len).collect();
    let trans = transitions_of(m);
    let file = Path::new(&m.file).file_name().map_or_else(|| m.file.clone(), |f| f.to_string_lossy().to_string());
    let rules = Rules {
        avg_shot: Some(r2(mean(&lens))),
        cut_on_beat: m.cut_on_beat.map(r2),
        transitions: trans.iter().map(|t| t.0.clone()).collect(),
        captions: String::new(),
        loudness: m.loudness.map(|v| (v * 10.0).round() / 10.0),
        grade: colour_words(&m.colour),
        pacing: pacing(&m.shots, m.duration).into_iter().map(r2).collect(),
        wpm: m.wpm.map(f64::round),
    };
    let described: Vec<&Description> = m.shots.iter().filter_map(|s| s.described.as_ref()).collect();
    let mut body = String::new();
    let _ = writeln!(
        body,
        "Learned from {file} ({}, {}x{}, {} shots). The numbers are measured; review the words before saving.\n",
        mmss(m.duration),
        m.width,
        m.height,
        m.shots.len()
    );
    let _ = writeln!(
        body,
        "- Shots average {:.1} s (median {:.1} s, shortest {:.1} s, longest {:.1} s); pacing {}.",
        mean(&lens),
        median(&lens),
        lens.iter().copied().fold(f64::MAX, f64::min),
        lens.iter().copied().fold(0.0, f64::max),
        styles::pacing_words(&rules.pacing).trim()
    );
    if !trans.is_empty() {
        let t: Vec<String> = trans.iter().map(|(n, s)| format!("{n} {:.0}%", s * 100.0)).collect();
        let _ = writeln!(body, "- Transitions: {}.", t.join(", "));
    }
    match (m.bpm, m.cut_on_beat) {
        (Some(bpm), Some(r)) => {
            let _ = writeln!(body, "- The music runs at about {bpm:.0} BPM; {:.0}% of cuts land on a beat.", r * 100.0);
        }
        _ => {
            let _ = writeln!(body, "- Cuts do not follow a musical beat.");
        }
    }
    if let Some(l) = m.loudness {
        let _ = writeln!(body, "- Mixed to about {l:.1} LUFS.");
    }
    let _ = writeln!(body, "- Look: {}.", colour_words(&m.colour));
    if let Some(w) = m.wpm {
        let _ = writeln!(body, "- Speech at about {w:.0} words per minute.");
    }
    if !described.is_empty() {
        let mut sizes: Vec<(&str, usize)> = Vec::new();
        for d in &described {
            let c = vlm::framing_class(&d.framing);
            if c.is_empty() {
                continue;
            }
            match sizes.iter_mut().find(|(k, _)| *k == c) {
                Some(x) => x.1 += 1,
                None => sizes.push((c, 1)),
            }
        }
        sizes.sort_by(|a, b| b.1.cmp(&a.1));
        if !sizes.is_empty() {
            let n = described.len() as f64;
            let t: Vec<String> = sizes.iter().map(|(k, c)| format!("{k} {:.0}%", *c as f64 / n * 100.0)).collect();
            let _ = writeln!(body, "- Framing (described): {}.", t.join(", "));
        }
        let moving = described.iter().filter(|d| !d.camera.is_empty() && !d.camera.to_ascii_lowercase().contains("static")).count();
        let _ = writeln!(body, "- Camera (described): {} of {} shots move.", moving, described.len());
        let texts = described.iter().filter(|d| !d.text.is_empty() && !d.text.eq_ignore_ascii_case("none")).count();
        if texts > 0 {
            let _ = writeln!(body, "- On-screen text in {texts} of {} described shots.", described.len());
        }
    }
    if !m.notes.is_empty() {
        let _ = writeln!(body, "\nNot measured: {}.", m.notes.join("; "));
    }
    let _ = writeln!(body, "\n### Shots\n\n| # | start | length | join | look | framing | subject | camera | text |\n|---|---|---|---|---|---|---|---|---|");
    for (i, s) in m.shots.iter().enumerate() {
        let d = s.described.clone().unwrap_or_default();
        let cell = |v: &str| v.replace('|', "/").replace('\n', " ");
        let _ = writeln!(
            body,
            "| {} | {} | {:.2} s | {} | {} | {} | {} | {} | {} |",
            i + 1,
            mmss(s.start),
            s.len(),
            s.join_in.map_or("-", Join::name),
            colour_words(&s.colour),
            cell(&d.framing),
            cell(&d.subject),
            cell(&d.camera),
            cell(&d.text)
        );
    }
    let name = styles::slug(name);
    Style {
        title: name.clone(),
        name,
        description: format!("Learned from {file}."),
        rules,
        body: body.trim_end().to_string(),
        skill: String::new(),
        extra: vec![("learned_from".into(), file)],
        source: Source::User,
        path: None,
    }
}

// ---------------------------------------------------------------------------------------------------
// jobs (styles.learn + styles.learn_status, the Learn window)

#[derive(Clone, Debug)]
pub struct Options {
    pub path: PathBuf,
    pub name: String,
    /// Describe shots with this model id (None = measure only).
    pub describe: Option<String>,
    /// Most shots described.
    pub max_shots: usize,
    pub speech: bool,
}

pub struct LearnJob {
    pub id: u64,
    pub opts: Options,
    pub progress: Arc<Progress>,
    pub result: Arc<Mutex<Option<Style>>>,
}

static JOBS: Mutex<Vec<Arc<LearnJob>>> = Mutex::new(Vec::new());

/// Start learning (a background job; the Jobs pane shows it). Returns its id.
pub fn start(opts: Options) -> Arc<LearnJob> {
    let result: Arc<Mutex<Option<Style>>> = Arc::new(Mutex::new(None));
    let sink = result.clone();
    let o = opts.clone();
    let progress = export::spawn_job("style-learn", move |prog| {
        crate::media::bg::lower_thread();
        let mut m = measure(&o.path, o.speech, prog)?;
        if let Some(model) = &o.describe {
            describe_shots(&o.path, &mut m, model, o.max_shots, prog, (0.65, 0.98))?;
        }
        *sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(draft(&m, &o.name));
        prog.set(1.0, "Draft ready");
        Ok(())
    });
    let mut jobs = JOBS.lock().unwrap_or_else(|e| e.into_inner());
    let id = jobs.last().map_or(1, |j| j.id + 1);
    let job = Arc::new(LearnJob { id, opts, progress, result });
    jobs.push(job.clone());
    if jobs.len() > 16 {
        jobs.remove(0);
    }
    job
}

pub fn job(id: u64) -> Option<Arc<LearnJob>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|j| j.id == id).cloned()
}

pub fn latest() -> Option<Arc<LearnJob>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).last().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(t: f64) -> Boundary {
        Boundary { t, join: Join::Cut, dur: 0.0 }
    }

    #[test]
    fn shots_pacing_and_beats() {
        let s = shots_from(&[b(1.5), b(3.5), b(0.1), b(3.55), b(4.5), b(6.0), b(7.95)], 8.0);
        let lens: Vec<f64> = s.iter().map(|s| r2(s.len())).collect();
        assert_eq!(lens, [1.5, 2.0, 1.0, 1.5, 2.0]);
        assert_eq!(s[0].join_in, None);
        assert_eq!(s[1].join_in, Some(Join::Cut));
        assert_eq!(pacing(&s, 8.0).iter().map(|v| r2(*v)).collect::<Vec<_>>(), [1.5, 2.0, 1.25, 2.0]);
        assert_eq!(on_beat_ratio(&[1.5, 3.5, 4.5, 6.0], &[0.5, 1.0, 1.52, 3.49, 4.6, 6.0], 0.05), 0.75);
        assert_eq!(on_beat_ratio(&[], &[1.0], 0.05), 0.0);
        assert_eq!((mean(&[1.0, 2.0, 6.0]), median(&[1.0, 6.0, 2.0]), median(&[1.0, 2.0, 3.0, 4.0])), (3.0, 2.0, 2.5));
        let p = peaks_of(&[0.5, -0.25, 0.1, 0.0], 200);
        assert_eq!((p.min.len(), p.max[0], p.min[0], p.max[1]), (2, 0.5, -0.25, 0.1));
    }

    #[test]
    fn colour_cuts_and_dissolves() {
        let solid = |r: u8, g: u8, bl: u8| -> Vec<u8> { (0..W * H).flat_map(|_| [r, g, bl]).collect() };
        let c = colour_of(&solid(255, 0, 0));
        assert!((c.luma - 0.2126).abs() < 0.01 && c.saturation > 0.99 && c.warmth > 0.99 && c.contrast < 1e-3);
        assert!(colour_of(&solid(128, 128, 128)).saturation < 1e-3);
        // a dip: bright, two dark frames, bright
        let lumas = [0.5, 0.5, 0.5, 0.5, 0.02, 0.02, 0.5, 0.5, 0.5, 0.5, 0.5f32];
        let d = classify_cut(0.5, &lumas, 10.0);
        assert_eq!((d.join, r2(d.dur)), (Join::DipToBlack, 0.2));
        assert_eq!(classify_cut(0.5, &[0.5; 11], 10.0).join, Join::Cut);
        // a 6-frame dissolve from red to blue between holds; a hard cut elsewhere is not one
        let (red, blue) = (solid(220, 20, 20), solid(20, 20, 220));
        let mut fr: Vec<Vec<u8>> = (0..10).map(|_| red.clone()).collect();
        for k in 1..=6 {
            let w = k as f32 / 7.0;
            fr.push(red.iter().zip(&blue).map(|(a, b)| (*a as f32 * (1.0 - w) + *b as f32 * w).round() as u8).collect());
        }
        fr.extend((0..10).map(|_| blue.clone()));
        fr.extend((0..10).map(|_| red.clone())); // hard cut at frame 26
        let found = dissolves(&fr, 10.0, &[2.6]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!((found[0].0 - 1.25).abs() < 0.11 && (found[0].1 - 0.7).abs() < 0.11, "{found:?}");
        // a moving picture (a shifted pattern) is no dissolve
        let pat = |off: usize| -> Vec<u8> { (0..W * H).flat_map(|i| if (i % W + off) % 16 < 8 { [250, 250, 250] } else { [5, 5, 5] }).collect() };
        let pan: Vec<Vec<u8>> = (0..20).map(|k| pat(k * 2)).collect();
        assert!(dissolves(&pan, 10.0, &[]).is_empty());
    }

    #[test]
    fn lufs_and_draft() {
        assert_eq!(parse_lufs("  I:         -14.2 LUFS\n  Threshold: -24.4 LUFS\n"), Some(-14.2));
        assert_eq!(parse_lufs("Summary:\n  I:  -70.0 LUFS"), None);
        let mut m = Measure { file: "C:/x/ref clip.mp4".into(), duration: 8.0, fps: 30.0, width: 1280, height: 720, ..Default::default() };
        m.boundaries = vec![b(1.5), b(3.5), Boundary { t: 4.5, join: Join::Dissolve, dur: 0.5 }, b(6.0)];
        m.shots = shots_from(&m.boundaries, 8.0);
        m.shots[0].described = Some(vlm::parse_answer("Framing: close-up\nSubject: a face\nCamera: static\nText: none"));
        m.shots[1].described = Some(vlm::parse_answer("Framing: wide\nSubject: a street\nCamera: pan left\nText: LISBON"));
        m.bpm = Some(120.0);
        m.cut_on_beat = Some(0.75);
        m.loudness = Some(-14.04);
        let s = draft(&m, "Ref Clip");
        assert_eq!(s.name, "ref-clip");
        assert_eq!(s.rules.avg_shot, Some(1.6));
        assert_eq!(s.rules.cut_on_beat, Some(0.75));
        assert_eq!(s.rules.transitions, ["cut", "dissolve 0.5"]);
        assert_eq!(s.rules.loudness, Some(-14.0));
        assert!(s.body.contains("cut 75%, dissolve 0.5 25%"), "{}", s.body);
        assert!(s.body.contains("Framing (described): close-up 50%, wide 50%"), "{}", s.body);
        assert!(s.body.contains("Camera (described): 1 of 2 shots move"), "{}", s.body);
        assert!(s.body.contains("| 2 | 0:01.5 | 2.00 s | cut |"), "{}", s.body);
        // it is a valid style file
        let back = styles::parse("x", &styles::to_text(&s), Source::User, None);
        assert_eq!(back.rules, s.rules);
        assert!(back.extra.iter().any(|(k, v)| k == "learned_from" && v == "ref clip.mp4"));
    }

    /// The real pipeline on a generated clip: five solid-colour shots with cuts at 1.5, 3.5, 4.5 and 6.0 s,
    /// a click track at 120 BPM (every 0.5 s, so every cut is on a beat), at -20 dBFS-ish.
    #[test]
    fn measures_a_generated_clip_with_known_cuts() {
        let Some(ff) = ffpipe::ffmpeg_exe() else { return };
        let base = std::env::var_os("SE_TEST_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let dir = base.join(format!("se-learn-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("known.mp4");
        let shots = [("red", 1.5), ("blue", 2.0), ("green", 1.0), ("yellow", 1.5), ("white", 2.0)];
        let mut args: Vec<String> = Vec::new();
        for (c, d) in shots {
            args.extend(["-f".into(), "lavfi".into(), "-i".into(), format!("color=c={c}:s=320x180:r=30:d={d}")]);
        }
        // clicks: a 5 ms 1 kHz burst every 0.5 s
        args.extend(["-f".into(), "lavfi".into(), "-i".into(), "aevalsrc='if(lt(mod(t,0.5),0.005),0.8*sin(2*PI*1000*t),0)':s=48000:d=8".into()]);
        let concat: String = (0..5).map(|i| format!("[{i}:v]")).collect::<String>() + "concat=n=5:v=1:a=0[v]";
        args.extend(["-filter_complex".into(), concat, "-map".into(), "[v]".into(), "-map".into(), "5:a".into()]);
        args.extend(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", "-y"].map(String::from));
        let st = ffpipe::command(&ff).args(["-v", "error"]).args(&args).arg(&out).status().unwrap();
        assert!(st.success());
        let prog = Progress::new();
        let m = measure(&out, false, &prog).unwrap();
        let cuts: Vec<f64> = m.boundaries.iter().map(|b| r2(b.t)).collect();
        assert_eq!(cuts.len(), 4, "{cuts:?}");
        for (got, want) in cuts.iter().zip([1.5, 3.5, 4.5, 6.0]) {
            assert!((got - want).abs() <= 0.05, "{cuts:?}");
        }
        assert!(m.boundaries.iter().all(|b| b.join == Join::Cut), "{:?}", m.boundaries);
        assert_eq!(m.shots.len(), 5);
        assert!((mean(&m.shots.iter().map(Shot::len).collect::<Vec<_>>()) - 1.6).abs() < 0.05);
        let bpm = m.bpm.expect("a steady beat");
        assert!((bpm - 120.0).abs() < 3.0, "{bpm}");
        assert_eq!(m.cut_on_beat, Some(1.0));
        assert!(m.loudness.is_some_and(|l| l < -5.0 && l > -40.0), "{:?}", m.loudness);
        // the shots' colours are measured: the white shot is the brightest, red is warm, blue is cool
        assert!(m.shots[4].colour.luma > 0.9 && m.shots[0].colour.warmth > 0.5 && m.shots[1].colour.warmth < -0.5);
        let s = draft(&m, "known");
        assert_eq!(s.rules.avg_shot, Some(1.6));
        assert_eq!(s.rules.transitions, ["cut"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SmolVLM2 on one shot of a generated clip (needs the add-on: SE_VLM_DIR or an installed copy).
    #[test]
    #[ignore]
    fn describes_a_shot_with_smolvlm2() {
        assert!(vlm::available(), "install the video describer or set SE_VLM_DIR");
        let ff = ffpipe::ffmpeg_exe().unwrap();
        let base = std::env::var_os("SE_TEST_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let dir = base.join(format!("se-vlm-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("titled.mp4");
        let st = ffpipe::command(&ff)
            .args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=s=640x360:r=30:d=3", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-y"])
            .arg(&out)
            .status()
            .unwrap();
        assert!(st.success());
        let prog = Progress::new();
        let mut m = measure(&out, false, &prog).unwrap();
        let model = vlm::installed_models()[0].clone();
        describe_shots(&out, &mut m, &model, 1, &prog, (0.0, 1.0)).unwrap();
        let d = m.shots[0].described.clone().unwrap();
        eprintln!("described: {d:?}");
        assert!(!d.raw.is_empty());
        let s = draft(&m, "described");
        assert!(s.body.contains("Camera (described)"), "{}", s.body);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
