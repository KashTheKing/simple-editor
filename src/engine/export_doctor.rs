//! ---- ws:forum-export ----
//! Export reliability (plans/shotcut-forum-insights.md, Session B). Everything here is pure or runs on
//! a job thread; `ui::app::export_fix` is the App side.
//!
//! - `classify`: ffmpeg's stderr tail -> one plain sentence plus the one button that fixes it.
//! - `should_retry_software`: the rule `export::run_export` uses to finish a failed hardware encode on
//!   software x264 (any hardware failure, at any frame, unless the error is the disk, the file or the
//!   media - a software retry can't fix those).
//! - `STALL` / `stalled`: the stuck-job watchdog the Jobs row reads (`Progress::idle_for`).
//! - `lint`: clips that break or balloon an export (a 0.0025x speed clip, a broken length, a bad frame
//!   rate), checked before the export starts.
//! - `find_problem`: "Find the problem" - export the timeline in halves at low quality and name the clip
//!   whose section fails (or hangs). `bisect` is the pure search, tested with a fake exporter.

use crate::engine::export::{self, ExportOptions, FrameSource, Progress, CANCELLED};
use crate::engine::text::TextRasterizer;
use crate::model::{Id, Project};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What went wrong, in the user's terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The (usually hardware) video encoder refused or crashed.
    Encoder,
    DiskFull,
    /// Another program holds the output, or the folder is read-only.
    FileLocked,
    /// A source file used on the timeline is gone.
    MissingMedia,
    /// A source file is there but can't be read.
    DamagedMedia,
    FrameRate,
    /// Width / height the encoder can't take.
    PictureSize,
    AudioLayout,
    OutOfMemory,
    /// No pattern matched.
    Unknown,
}

/// The one button a failure offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fix {
    RetrySoftware,
    /// Opens the Export pane (frame rate, size, audio layout, quality).
    ChangeSettings,
    Relink,
    /// Opens the Export pane to pick another file name / folder.
    ChooseAnotherFile,
    FindProblem,
}

impl Fix {
    pub fn label(self) -> &'static str {
        match self {
            Fix::RetrySoftware => "Retry in Software",
            Fix::ChangeSettings => "Change Settings",
            Fix::Relink => "Relink…",
            Fix::ChooseAnotherFile => "Choose Another File",
            Fix::FindProblem => "Find the Problem",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnosis {
    pub problem: Problem,
    /// One short sentence, no jargon.
    pub sentence: String,
    pub fix: Fix,
}

/// ffmpeg's stderr (or our own error text) -> a sentence and a fix. Order matters: a disk-full error is
/// often reported while "writing the trailer" of an encoder, so the disk and the file go first.
pub fn classify(err: &str) -> Diagnosis {
    let e = err.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| e.contains(n));
    let d = |problem, sentence: &str, fix| Diagnosis { problem, sentence: sentence.to_string(), fix };
    if has(&["no space left", "disk full", "not enough space", "enospc", "disk quota"]) {
        return d(
            Problem::DiskFull,
            "The disk is full. Free some space or save to another drive.",
            Fix::ChooseAnotherFile,
        );
    }
    if has(&[
        "permission denied",
        "access is denied",
        "being used by another process",
        "sharing violation",
        "read-only file system",
        "eacces",
    ]) {
        return d(
            Problem::FileLocked,
            "The file can't be written: another program has it open, or the folder is read-only.",
            Fix::ChooseAnotherFile,
        );
    }
    if has(&["out of memory", "cannot allocate memory", "enomem", "memory allocation failed", "failed to allocate"]) {
        return d(
            Problem::OutOfMemory,
            "The computer ran out of memory. Try a smaller size or close other programs.",
            Fix::ChangeSettings,
        );
    }
    if has(&[
        "nvenc",
        "openencodesessionex",
        "no capable devices",
        "nvcuda",
        "_amf",
        "amf ",
        "_qsv",
        "mfx",
        "error while opening encoder",
        "unknown encoder",
        "encoder not found",
        "failed to start - retried",
        "device creation failed",
        "initializeencoder",
        "driver does not support",
        "hardware",
    ]) {
        return d(Problem::Encoder, "The video encoder stopped working.", Fix::RetrySoftware);
    }
    if has(&["not divisible by 2", "invalid dimensions", "frame size", "width or height", "picture size"]) {
        return d(Problem::PictureSize, "The encoder can't use this picture size.", Fix::ChangeSettings);
    }
    if has(&["frame rate", "framerate", "invalid fps", "time base", "timebase"]) {
        return d(Problem::FrameRate, "The encoder can't use this frame rate.", Fix::ChangeSettings);
    }
    if has(&["channel layout", "channel_layout", "sample rate", "sample_rate", "sample format"]) {
        return d(Problem::AudioLayout, "The audio layout doesn't fit this file type.", Fix::ChangeSettings);
    }
    if has(&["no such file or directory", "cannot find the file", "cannot find the path", "does not exist"]) {
        return d(Problem::MissingMedia, "A file used on the timeline is missing.", Fix::Relink);
    }
    if has(&["invalid data found", "moov atom not found", "error while decoding", "corrupt", "end of file"]) {
        return d(Problem::DamagedMedia, "A file used on the timeline can't be read.", Fix::FindProblem);
    }
    d(Problem::Unknown, "The export stopped.", Fix::FindProblem)
}

/// A hardware video encoder (the probe's candidates, plus the families it does not list).
pub fn is_hw(encoder: &str) -> bool {
    crate::engine::encoders::CANDIDATES.contains(&encoder)
        || ["_nvenc", "_qsv", "_amf", "_mf", "_vaapi", "_videotoolbox"].iter().any(|s| encoder.ends_with(s))
}

/// Finish a failed export in software? Only a hardware encode, only an error a different encoder can
/// change (never a cancel, the disk, the output file or the media).
pub fn should_retry_software(err: &str, encoder: &str) -> bool {
    if err == CANCELLED || !is_hw(encoder) {
        return false;
    }
    !matches!(
        classify(err).problem,
        Problem::DiskFull | Problem::FileLocked | Problem::MissingMedia | Problem::DamagedMedia
    )
}

/// `export::run_export`'s second rule (the first, a hardware encoder "auto" picked that failed in its
/// first frames, is its own): the encoder this export resolves to is hardware and the error is one a
/// software encode can change. HDR exports pick their own 10-bit encoder and are left alone.
pub fn retry_in_software(project: &Project, opts: &ExportOptions, err: &str) -> bool {
    let ext = export::ext_of(&opts.out_path);
    if project.color.is_hdr() || export::AUDIO_EXTS.contains(&ext.as_str()) {
        return false;
    }
    should_retry_software(err, &export::resolve_encoder(&ext, &opts.encoder).0)
}

/// The status note a software retry carries in the Jobs row.
pub fn retry_note(encoder: &str, early: bool) -> String {
    if early {
        format!("{encoder} failed to start - retried in software")
    } else {
        format!("{encoder} stopped partway - finished in software")
    }
}

// ---------- what the last export was ----------

type Remembered = (Arc<Progress>, Project, ExportOptions);
static LAST: Mutex<Option<Remembered>> = Mutex::new(None);

/// `export::start_export` keeps what it was asked for, so Retry in Software and Find the Problem can
/// re-run the same export (one at a time: the newest replaces the last).
pub fn remember(prog: &Arc<Progress>, project: Project, opts: ExportOptions) {
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some((prog.clone(), project, opts));
}

/// The project and options of the export `prog` belongs to, while it is the newest one started.
pub fn recall(prog: &Arc<Progress>) -> Option<(Project, ExportOptions)> {
    let last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    last.as_ref().filter(|(p, ..)| Arc::ptr_eq(p, prog)).map(|(_, project, opts)| (project.clone(), opts.clone()))
}

// ---------- stuck-job watchdog ----------

/// No progress for this long: the Jobs row says so and offers "Retry in Software".
pub const STALL: Duration = Duration::from_secs(60);

pub fn stalled(idle: Duration, done: bool) -> bool {
    !done && idle >= STALL
}

// ---------- estimated time ----------

/// Megapixel-frames per second before any export has been timed on this machine (a mid-range CPU on
/// software x264 at Balanced: about 1080p50).
pub const DEFAULT_MPX_RATE: f64 = 100.0;

/// Seconds an export of `w`x`h` at `fps` for `dur` seconds should take at `mpx_rate`.
pub fn estimate_secs(w: u32, h: u32, fps: f64, dur: f64, mpx_rate: f64) -> f64 {
    let mpx = w as f64 * h as f64 / 1e6 * fps.max(1.0) * dur.max(0.0);
    mpx / if mpx_rate > 0.1 { mpx_rate } else { DEFAULT_MPX_RATE }
}

/// The rate a finished export measured, blended into the remembered one (new exports count for half,
/// so one odd export does not swing the next estimate).
pub fn blend_rate(old: f64, w: u32, h: u32, fps: f64, dur: f64, took: Duration) -> f64 {
    let secs = took.as_secs_f64();
    if secs < 1.0 || dur <= 0.0 {
        return old;
    }
    let measured = w as f64 * h as f64 / 1e6 * fps.max(1.0) * dur / secs;
    if old > 0.1 {
        (old + measured) / 2.0
    } else {
        measured
    }
}

// ---------- pre-export lint ----------

/// A clip (or, with `clip == 0`, the project) that will break or balloon the export.
#[derive(Clone, Debug, PartialEq)]
pub struct Suspect {
    pub clip: Id,
    /// Timeline seconds to jump to.
    pub at: f64,
    /// One plain sentence naming the clip.
    pub why: String,
}

const MIN_SPEED: f64 = 0.01;
const MAX_SPEED: f64 = 100.0;
const MAX_CLIP_SECS: f64 = 24.0 * 3600.0;

/// The project's main timeline, enabled clips only.
pub fn lint(project: &Project) -> Vec<Suspect> {
    let mut out = Vec::new();
    if !(project.fps.is_finite() && (1.0..=240.0).contains(&project.fps)) {
        out.push(Suspect {
            clip: 0,
            at: 0.0,
            why: format!("The project frame rate ({}) can't be exported. Set it in Project Settings.", project.fps),
        });
    }
    for (_, c) in project.all_clips().filter(|(_, c)| c.enabled) {
        let name = if c.name.is_empty() { "A clip".to_string() } else { format!("\"{}\"", c.name) };
        let at = if c.start.is_finite() { c.start.max(0.0) } else { 0.0 };
        if !c.start.is_finite() || !c.duration.is_finite() || c.duration <= 0.0 || c.duration > MAX_CLIP_SECS {
            out.push(Suspect { clip: c.id, at, why: format!("{name} has a broken length.") });
        } else if c.freeze.is_none() && (c.speed < MIN_SPEED || c.speed > MAX_SPEED || !c.speed.is_finite()) {
            out.push(Suspect { clip: c.id, at, why: format!("{name} plays at {}x speed.", fmt_speed(c.speed)) });
        }
    }
    out
}

fn fmt_speed(s: f64) -> String {
    let t = format!("{s:.4}");
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

// ---------- Find the problem ----------

/// What "Find the problem" found: the smallest section that still fails and the clips in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Culprit {
    pub range: (f64, f64),
    /// Enabled clips overlapping `range`, topmost track first.
    pub clips: Vec<Id>,
    /// The failure the last test hit, classified.
    pub why: Diagnosis,
}

/// Sections short enough to name one clip: every clip edge inside `span` is a cut, and pieces shorter
/// than half a second are merged into their neighbour.
pub fn segments(project: &Project, span: (f64, f64)) -> Vec<(f64, f64)> {
    let (a, b) = span;
    let mut cuts: Vec<f64> = vec![a, b];
    for (_, c) in project.all_clips().filter(|(_, c)| c.enabled) {
        for t in [c.start, c.start + c.duration] {
            if t.is_finite() && t > a && t < b {
                cuts.push(t);
            }
        }
    }
    cuts.sort_by(|x, y| x.total_cmp(y));
    cuts.dedup_by(|x, y| (*x - *y).abs() < 0.5);
    if let Some(last) = cuts.last_mut() {
        *last = b; // a merged last cut still ends the span
    }
    cuts.windows(2).map(|w| (w[0], w[1])).filter(|(x, y)| y > x).collect()
}

/// Narrow a failure to the fewest segments: `fails(range)` exports that range and says whether it
/// failed. None = the whole span exports fine at test quality. When neither half fails alone the
/// problem needs both (a transition across the cut): the pair is the answer.
pub fn bisect(
    segs: &[(f64, f64)],
    mut fails: impl FnMut((f64, f64)) -> Result<bool, String>,
) -> Result<Option<(f64, f64)>, String> {
    let Some((first, last)) = segs.first().zip(segs.last()) else { return Ok(None) };
    if !fails((first.0, last.1))? {
        return Ok(None);
    }
    let (mut lo, mut hi) = (0usize, segs.len());
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if fails((segs[lo].0, segs[mid - 1].1))? {
            hi = mid;
        } else if fails((segs[mid].0, segs[hi - 1].1))? {
            lo = mid;
        } else {
            // the failure needs both halves (e.g. a transition across the cut): trim each end while it still fails
            while lo + 1 < hi && fails((segs[lo + 1].0, segs[hi - 1].1))? {
                lo += 1;
            }
            while hi > lo + 1 && fails((segs[lo].0, segs[hi - 2].1))? {
                hi -= 1;
            }
            break;
        }
    }
    Ok(Some((segs[lo].0, segs[hi - 1].1)))
}

/// Enabled clips overlapping `range`, the topmost (last) track first.
pub fn clips_in(project: &Project, range: (f64, f64)) -> Vec<Id> {
    let mut v: Vec<(usize, Id)> = project
        .all_clips()
        .filter(|(_, c)| c.enabled && c.start < range.1 && c.start + c.duration > range.0)
        .map(|(t, c)| (t, c.id))
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().map(|(_, id)| id).collect()
}

/// One test attempt that makes no progress for this long counts as a failure (the stuck-at-99% case).
const TEST_STALL: Duration = Duration::from_secs(30);

/// The low-quality, small, software version of `opts` for one test range.
pub fn test_options(opts: &ExportOptions, out: PathBuf, range: (f64, f64), project: &Project) -> ExportOptions {
    let (w, h) = (project.width.max(2), project.height.max(2));
    let tw = 320u32.min(w);
    let th = ((h as f64 * tw as f64 / w as f64).round() as u32).max(2);
    ExportOptions {
        out_path: out,
        encoder: "libx264".into(),
        crf: 40,
        preset: "ultrafast".into(),
        out_size: Some((tw & !1, th & !1)),
        scaler: "bilinear".into(),
        frames: FrameSource::Cpu,
        metadata: Vec::new(),
        range: Some(range),
        loudnorm: false,
        letterbox: false,
        audio: export::AudioLayout::Mix,
        target_bytes: None,
        ..opts.clone()
    }
}

/// Export `range` at test quality on its own thread; Ok(true) = it failed (or hung), with the error.
fn test_range(
    project: &Project,
    opts: &ExportOptions,
    text: &Arc<Mutex<TextRasterizer>>,
    prog: &Progress,
    dir: &Path,
    range: (f64, f64),
    last_err: &mut Option<String>,
) -> Result<bool, String> {
    let ext = export::ext_of(&opts.out_path);
    let ext = if ext.is_empty() { "mp4".to_string() } else { ext };
    let o = test_options(opts, dir.join(format!("t{:.3}.{ext}", range.0)), range, project);
    let sub = Progress::new();
    let r = std::thread::scope(|s| {
        let h = s.spawn(|| export::run_export(project, &o, text, &sub));
        while !h.is_finished() {
            if prog.is_cancelled() || sub.idle_for() >= TEST_STALL {
                sub.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        h.join().unwrap_or_else(|_| Err("the export thread panicked".into()))
    });
    let _ = std::fs::remove_file(&o.out_path);
    if prog.is_cancelled() {
        return Err(CANCELLED.into());
    }
    match r {
        Ok(()) => Ok(false),
        Err(e) if e == CANCELLED => {
            *last_err = Some("The export stopped making progress.".into());
            Ok(true)
        }
        Err(e) => {
            *last_err = Some(e);
            Ok(true)
        }
    }
}

/// "Find the problem": bisect the export at test quality. Ok(None) = it exports fine at test quality
/// (so the full settings - encoder, size, quality - are what fails).
pub fn find_problem(
    project: &Project,
    opts: &ExportOptions,
    text: &Arc<Mutex<TextRasterizer>>,
    prog: &Progress,
) -> Result<Option<Culprit>, String> {
    // the remembered project is the undrawn one: draw zones + browser zooms as start_export does
    let drawn = crate::engine::zones::drawn(project);
    let project = &drawn;
    let span = opts.range.unwrap_or((0.0, project.duration()));
    let segs = segments(project, span);
    let dir = std::env::temp_dir().join(format!("simple-editor-find-problem-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // log2(n) + 2 tests at most, two per step
    let steps = 2.0 * ((segs.len().max(1) as f32).log2().ceil() + 1.0);
    let mut done = 0f32;
    let mut last_err = None;
    let found = bisect(&segs, |r| {
        prog.set((done / steps).min(0.99), format!("Testing {} to {}", clock(r.0), clock(r.1)));
        done += 1.0;
        test_range(project, opts, text, prog, &dir, r, &mut last_err)
    });
    let _ = std::fs::remove_dir_all(&dir);
    Ok(found?.map(|range| Culprit {
        range,
        clips: clips_in(project, range),
        why: classify(last_err.as_deref().unwrap_or("")),
    }))
}

/// m:ss for a status line.
pub fn clock(t: f64) -> String {
    let s = t.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipKind, TrackKind};

    #[test]
    fn injected_failures_read_as_one_sentence_with_a_fix() {
        let cases = [
            // the hardware encoder dies (driver change, no NVENC session)
            ("[h264_nvenc @ 0x1] OpenEncodeSessionEx failed: out of memory (10): (no details)", Problem::OutOfMemory),
            ("[h264_nvenc @ 0x1] No capable devices found", Problem::Encoder),
            ("Error while opening encoder for output stream #0:0 - maybe incorrect parameters", Problem::Encoder),
            ("h264_amf failed to start - retried in software", Problem::Encoder),
            // the disk / the file
            ("av_interleaved_write_frame(): No space left on device", Problem::DiskFull),
            ("C:\\out.mp4: Permission denied", Problem::FileLocked),
            ("The process cannot access the file because it is being used by another process.", Problem::FileLocked),
            // odd settings
            ("[libx264 @ 0x2] width not divisible by 2 (1279x720)", Problem::PictureSize),
            ("[mp4 @ 0x3] Invalid frame rate 1/0", Problem::FrameRate),
            ("[aac @ 0x4] Unsupported channel layout \"7.1(wide)\"", Problem::AudioLayout),
            // the media
            ("D:\\clips\\a.mp4: No such file or directory", Problem::MissingMedia),
            ("[mov,mp4 @ 0x5] moov atom not found", Problem::DamagedMedia),
            ("ffmpeg exited with exit code: 3221225477", Problem::Unknown),
        ];
        for (err, want) in cases {
            let d = classify(err);
            assert_eq!(d.problem, want, "{err}");
            assert!(d.sentence.ends_with('.') && d.sentence.len() < 110, "{err}: {}", d.sentence);
            assert!(!d.sentence.contains('@') && !d.sentence.contains("0x"), "no jargon: {}", d.sentence);
            assert!(!d.fix.label().is_empty());
        }
        assert_eq!(classify("No capable devices found").fix, Fix::RetrySoftware);
        assert_eq!(classify("No such file or directory").fix, Fix::Relink);
        assert_eq!(classify("No space left on device").fix, Fix::ChooseAnotherFile);
    }

    #[test]
    fn software_retry_follows_any_hardware_failure_but_not_the_disk_or_media() {
        assert!(should_retry_software("No capable devices found", "h264_nvenc"));
        assert!(should_retry_software("ffmpeg exited with exit code: 1", "hevc_qsv")); // mid-export crash
        assert!(should_retry_software("Broken pipe", "h264_amf"));
        assert!(!should_retry_software("No space left on device", "h264_nvenc"));
        assert!(!should_retry_software("Permission denied", "h264_nvenc"));
        assert!(!should_retry_software("x.mp4: No such file or directory", "h264_nvenc"));
        assert!(!should_retry_software(CANCELLED, "h264_nvenc"));
        assert!(!should_retry_software("No capable devices found", "libx264"), "software has nothing to fall to");
        assert!(is_hw("h264_mf") && is_hw("av1_nvenc") && !is_hw("libx264") && !is_hw("auto"));
        assert!(retry_note("h264_nvenc", false).contains("software"));
    }

    #[test]
    fn the_watchdog_waits_a_minute_and_never_fires_on_a_finished_job() {
        assert!(!stalled(Duration::from_secs(59), false));
        assert!(stalled(STALL, false));
        assert!(!stalled(Duration::from_secs(600), true));
    }

    #[test]
    fn progress_idle_time_resets_only_when_the_fraction_moves() {
        let p = Progress::new();
        p.set(0.5, "a");
        std::thread::sleep(Duration::from_millis(30));
        p.set(0.5, "b"); // a status change alone is not progress
        assert!(p.idle_for() >= Duration::from_millis(25));
        p.set(0.6, "c");
        assert!(p.idle_for() < Duration::from_millis(25));
    }

    #[test]
    fn estimated_time_scales_with_pixels_and_learns_the_rate() {
        // 1080p30 for 60 s at 100 Mpx/s is about 37 s
        let t = estimate_secs(1920, 1080, 30.0, 60.0, 100.0);
        assert!((t - 37.3).abs() < 0.5, "{t}");
        assert_eq!(estimate_secs(1920, 1080, 30.0, 60.0, 0.0), t, "no rate yet = the default");
        // a 60 s 1080p30 export that took 30 s measured 124 Mpx/s; blended half and half
        let r = blend_rate(100.0, 1920, 1080, 30.0, 60.0, Duration::from_secs(30));
        assert!((r - 112.2).abs() < 0.5, "{r}");
        assert_eq!(blend_rate(100.0, 1920, 1080, 30.0, 60.0, Duration::from_millis(10)), 100.0);
    }

    fn project(clips: &[(f64, f64, f64)]) -> Project {
        let mut p = Project::new();
        let t = p.add_track(TrackKind::Video);
        for (i, &(start, dur, speed)) in clips.iter().enumerate() {
            let mut c = Clip::new(i as Id + 1, ClipKind::Video, format!("c{i}"), start, dur);
            c.speed = speed;
            p.tracks[t].clips.push(c);
        }
        p
    }

    #[test]
    fn lint_names_the_slow_clip_and_a_broken_length() {
        // forum 51460: a clip at 0.0025x speed made the export hang
        let p = project(&[(0.0, 5.0, 1.0), (5.0, 800.0, 0.0025), (805.0, f64::NAN, 1.0)]);
        let s = lint(&p);
        assert_eq!(s.len(), 2, "{s:?}");
        assert_eq!((s[0].clip, s[0].at), (2, 5.0));
        assert_eq!(s[0].why, "\"c1\" plays at 0.0025x speed.");
        assert!(s[1].why.contains("broken length"));
        let mut q = project(&[(0.0, 5.0, 1.0)]);
        assert!(lint(&q).is_empty());
        q.fps = 0.0;
        assert_eq!(lint(&q)[0].clip, 0);
    }

    #[test]
    fn segments_cut_at_clip_edges_and_merge_slivers() {
        let p = project(&[(0.0, 4.0, 1.0), (4.0, 4.2, 1.0), (8.2, 3.0, 1.0)]);
        assert_eq!(segments(&p, (0.0, 11.2)), vec![(0.0, 4.0), (4.0, 8.2), (8.2, 11.2)]);
        // a 0.2 s sliver at the end merges into the last section, which still ends the span
        let p = project(&[(0.0, 4.0, 1.0), (4.0, 3.8, 1.0)]);
        assert_eq!(segments(&p, (0.0, 8.0)), vec![(0.0, 4.0), (4.0, 8.0)]);
    }

    #[test]
    fn bisect_names_the_one_bad_section() {
        let segs: Vec<(f64, f64)> = (0..10).map(|i| (i as f64, i as f64 + 1.0)).collect();
        for bad in 0..10 {
            let mut tests = 0;
            let got = bisect(&segs, |(a, b)| {
                tests += 1;
                Ok(a <= bad as f64 && b > bad as f64)
            })
            .unwrap();
            assert_eq!(got, Some((bad as f64, bad as f64 + 1.0)), "bad = {bad}");
            assert!(tests <= 1 + 2 * 4, "{tests} tests for 10 sections");
        }
        // nothing fails at test quality
        assert_eq!(bisect(&segs, |_| Ok(false)).unwrap(), None);
        // a failure that needs both sides of a cut (a transition across 4|5) returns the pair's span
        let pair = bisect(&segs, |(a, b)| Ok(a <= 4.0 && b >= 6.0)).unwrap();
        let (a, b) = pair.unwrap();
        assert!(a <= 4.0 && b >= 6.0 && b - a < 10.0, "{pair:?}");
        // a cancel stops the search
        assert!(bisect(&segs, |_| Err(CANCELLED.to_string())).is_err());
    }

    #[test]
    fn culprit_clips_are_the_ones_in_the_section_topmost_first() {
        let mut p = project(&[(0.0, 5.0, 1.0), (5.0, 5.0, 1.0)]);
        let t2 = p.add_track(TrackKind::Video);
        p.tracks[t2].clips.push(Clip::new(9, ClipKind::Text, "title", 4.0, 3.0));
        assert_eq!(clips_in(&p, (5.0, 10.0)), vec![9, 2]);
    }

    #[test]
    fn test_exports_are_small_software_and_even() {
        let mut p = Project::new();
        (p.width, p.height) = (1918, 1078);
        // (a literal on purpose: a new ExportOptions field must be thought about here too)
        let o = ExportOptions {
            out_path: PathBuf::from("x.mp4"),
            encoder: "h264_nvenc".into(),
            crf: 18,
            preset: "slow".into(),
            backend: crate::media::Backend::Auto,
            out_size: None,
            scaler: "lanczos".into(),
            frames: FrameSource::Cpu,
            metadata: vec![("title".into(), "x".into())],
            range: None,
            loudnorm: true,
            letterbox: true,
            audio: export::AudioLayout::Stems,
            stem_by: Default::default(),
            overlay: None,
            fps: Some(60.0),
            target_bytes: Some(1 << 30),
        };
        let t = test_options(&o, PathBuf::from("t.mp4"), (1.0, 2.0), &p);
        assert_eq!(t.encoder, "libx264");
        assert_eq!(t.range, Some((1.0, 2.0)));
        let (w, h) = t.out_size.unwrap();
        assert!(w <= 320 && w % 2 == 0 && h % 2 == 0, "{w}x{h}");
        assert!(matches!(t.frames, FrameSource::Cpu));
        assert_eq!((t.target_bytes, t.audio, t.fps), (None, export::AudioLayout::Mix, Some(60.0)));
    }
}
