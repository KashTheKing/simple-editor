//! Speech → text (whisper.cpp), the subtitle generator on top of it, and the double-take detector.
//!
//! Nothing here ships in the exe: no model, no ML crate. The model is downloaded once into
//! `Settings::cache_dir()\models` (visible progress, size named before the click) and the inference is a
//! child process, exactly like ffmpeg (`media/ffpipe.rs`) and yt-dlp: locate the exe, run it, parse its
//! stdout, never block the UI. With neither installed the app is untouched — the pane just says what to
//! install and where it looked.
//!
//! whisper.cpp prints one line per segment, `[00:00:01.000 --> 00:00:02.400]   text`, so the transcript
//! streams in while it runs and the progress fraction is the last timestamp over the audio length.
//! `-ml 1 -sow` makes those lines one word each — that is where the word timings come from, regrouped
//! into sentences by `group_words`.
//!
//! The subtitle conventions follow the usual auto-subs ones: ~42 characters a line, split on word
//! boundaries, a minimum on-screen time that never eats the next cue.

use crate::engine::export::{self, Progress};
use crate::media::ffpipe;
use crate::settings::Settings;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};

/// Downloadable whisper.cpp models: (name, file, download size in MB).
pub const MODELS: [(&str, &str, u32); 4] = [
    ("tiny.en — fastest", "ggml-tiny.en.bin", 75),
    ("base.en — recommended", "ggml-base.en.bin", 142),
    ("small.en — best", "ggml-small.en.bin", 466),
    ("base — any language", "ggml-base.bin", 142),
];

const HOST: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";

/// Fewer words than this and a repeat is just normal speech ("yeah", "okay"), not a second take.
const MIN_WORDS: usize = 2;
/// Duplicate ranges closer than this are cut as one.
const MERGE_GAP: f64 = 0.35;

// ponytail: the settings are read once per process and never written back from the pane — a
// load-modify-save here would race the app's in-memory copy. Edit settings.json to change them.
fn settings() -> &'static (String, String) {
    static S: OnceLock<(String, String)> = OnceLock::new();
    S.get_or_init(|| {
        let s = Settings::load();
        (s.whisper_dir, s.transcribe_model)
    })
}

/// Index into `MODELS` of `Settings.transcribe_model` (by name or file), else base.en.
pub fn default_model() -> usize {
    let want = settings().1.trim();
    if want.is_empty() {
        return 1;
    }
    MODELS.iter().position(|(n, f, _)| *f == want || n.starts_with(want)).unwrap_or(1)
}

/// `%LOCALAPPDATA%\SimpleEditor\cache\models` — where downloaded models live.
pub fn models_dir() -> PathBuf {
    Settings::cache_dir().join("models")
}

/// Where the app looks for a whisper binary it did not find on PATH.
pub fn exe_dir() -> PathBuf {
    Settings::cache_dir().join("whisper")
}

pub fn model_path(file: &str) -> PathBuf {
    models_dir().join(file)
}

pub fn model_url(file: &str) -> String {
    format!("{HOST}{file}")
}

/// Is the model there and not a truncated download?
pub fn have_model(file: &str) -> bool {
    std::fs::metadata(model_path(file)).map(|m| m.len() > 1_000_000).unwrap_or(false)
}

/// A whisper.cpp binary, or None. `main.exe` is only accepted from a directory we were pointed at —
/// a `main.exe` picked up off PATH would be anything at all.
pub fn exe() -> Option<PathBuf> {
    let dirs = [PathBuf::from(&settings().0), exe_dir()];
    for name in ["whisper-cli.exe", "whisper.exe", "main.exe"] {
        if let Some(p) = dirs.iter().map(|d| d.join(name)).find(|p| p.is_file()) {
            return Some(p);
        }
        if name != "main.exe" {
            if let Some(p) = ffpipe::find_exe(name, "") {
                return Some(p);
            }
        }
    }
    None
}

/// Exactly what to install and where it is expected (shown in the pane when `exe()` is None).
pub fn install_hint() -> String {
    format!(
        "whisper.cpp not found. Put whisper-cli.exe (github.com/ggml-org/whisper.cpp → Releases) in {}, \
         next to SimpleEditor.exe, or on PATH.",
        exe_dir().display()
    )
}

// ---------------------------------------------------------------- model download

/// Download a model into the cache. `curl.exe` ships with Windows 10 1803+; progress is the size of the
/// `.part` file against the published one, because curl's own meter needs a console.
pub fn download_model(file: &str) -> Arc<Progress> {
    let file = file.to_string();
    export::spawn_job("whisper-model", move |prog| download(&file, prog))
}

fn download(file: &str, prog: &Progress) -> Result<(), String> {
    if have_model(file) {
        return Ok(());
    }
    let dir = models_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let curl = ffpipe::find_exe("curl.exe", "").ok_or("curl.exe not found (it ships with Windows 10 1803+)")?;
    let tmp = dir.join(format!("{file}.part"));
    let _ = std::fs::remove_file(&tmp);
    let expect = MODELS.iter().find(|m| m.1 == file).map_or(0.0, |m| m.2 as f64 * 1_000_000.0);
    prog.set(0.0, "Connecting…");

    let mut child = ffpipe::command(&curl)
        .args(["-L", "--fail", "-sS", "-o"])
        .arg(&tmp)
        .arg(model_url(file))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    let tail = export::stderr_tail(&mut child);
    let status = loop {
        if prog.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&tmp);
            return Err(export::CANCELLED.into());
        }
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {}
            Err(e) => return Err(format!("curl: {e}")),
        }
        let got = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0) as f64;
        let f = if expect > 0.0 { (got / expect) as f32 } else { 0.0 };
        prog.set(f.clamp(0.0, 0.99), format!("Downloading… {:.0} of {:.0} MB", got / 1e6, expect / 1e6));
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    let msg = tail.and_then(|t| t.join().ok()).unwrap_or_default();
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(if msg.is_empty() { format!("download failed ({status})") } else { msg });
    }
    std::fs::rename(&tmp, model_path(file)).map_err(|e| format!("{}: {e}", model_path(file).display()))?;
    prog.set(1.0, "Done");
    Ok(())
}

// ---------------------------------------------------------------- transcription

/// One transcribed span. `words` is filled only when the run asked for word timings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub words: Vec<(f64, f64, String)>,
}

impl Segment {
    fn plain(start: f64, end: f64, text: String) -> Self {
        Self { start, end, text, words: Vec::new() }
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub path: String,
    /// Source seconds to transcribe (the clip's own range).
    pub src_start: f64,
    pub src_duration: f64,
    /// Model file name inside `models_dir()`.
    pub model: String,
    /// "auto", "en", "de", …
    pub language: String,
    /// Ask whisper for one segment per word and regroup here.
    pub words: bool,
    /// whisper.cpp `--prompt`: vocabulary/style hints (names, jargon, punctuation style). Not commands —
    /// the model only mimics it, it does not follow instructions.
    pub prompt: String,
}

/// A running transcription. `segments()` grows while it runs; `progress` drives the UI and cancels it.
pub struct Job {
    pub progress: Arc<Progress>,
    out: Arc<Mutex<Vec<Segment>>>,
}

impl Job {
    pub fn segments(&self) -> Vec<Segment> {
        self.out.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn cancel(&self) {
        self.progress.cancel.store(true, Ordering::SeqCst);
    }
}

pub fn start(opts: Options) -> Job {
    let out: Arc<Mutex<Vec<Segment>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = out.clone();
    let progress = export::spawn_job("transcribe", move |prog| run(&opts, prog, &sink));
    Job { progress, out }
}

/// 16 kHz mono wav of the clip's range, through the ffmpeg we already ship next to.
fn extract_wav(opts: &Options, prog: &Progress) -> Result<PathBuf, String> {
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let wav = std::env::temp_dir().join(format!("se-transcribe-{}.wav", std::process::id()));
    let mut child = ffpipe::command(&ffmpeg)
        .args(["-v", "error", "-y", "-ss"])
        .arg(format!("{:.3}", opts.src_start.max(0.0)))
        .arg("-t")
        .arg(format!("{:.3}", opts.src_duration.max(0.1)))
        .arg("-i")
        .arg(&opts.path)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-f", "wav"])
        .arg(&wav)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let tail = export::stderr_tail(&mut child);
    export::wait_ffmpeg(&mut child, tail, prog)?;
    if std::fs::metadata(&wav).map(|m| m.len()).unwrap_or(0) < 1024 {
        return Err("that clip has no audio to transcribe".into());
    }
    Ok(wav)
}

fn run(opts: &Options, prog: &Progress, sink: &Mutex<Vec<Segment>>) -> Result<(), String> {
    let exe = exe().ok_or_else(install_hint)?;
    let model = model_path(&opts.model);
    if !model.is_file() {
        return Err(format!("model {} is not downloaded", opts.model));
    }
    prog.set(0.01, "Extracting audio…");
    let wav = extract_wav(opts, prog)?;
    prog.set(0.05, "Transcribing…");

    let lang = if opts.language.trim().is_empty() { "auto".to_string() } else { opts.language.trim().to_string() };
    let mut cmd = ffpipe::command(&exe);
    cmd.arg("-m").arg(&model).arg("-f").arg(&wav).args(["-l", &lang]);
    if opts.words {
        cmd.args(["-ml", "1", "-sow"]);
    }
    if !opts.prompt.trim().is_empty() {
        cmd.arg("--prompt").arg(opts.prompt.trim());
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", exe.display()))?;
    let tail = export::stderr_tail(&mut child);
    let total = opts.src_duration.max(0.1);
    let mut raw: Vec<(f64, f64, String)> = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if prog.is_cancelled() {
                break; // wait_ffmpeg kills the child and reports CANCELLED
            }
            let Some(seg) = parse_line(&line) else { continue };
            prog.set((0.05 + 0.94 * (seg.1 / total)).clamp(0.05, 0.99) as f32, "Transcribing…");
            raw.push(seg.clone());
            // raw segments, one word each with -ml 1: the UI regroups them into sentences, so the
            // grouping can be re-run with new parameters without re-transcribing
            sink.lock().unwrap_or_else(|e| e.into_inner()).push(Segment::plain(seg.0, seg.1, seg.2));
        }
    }
    let r = export::wait_ffmpeg(&mut child, tail, prog);
    let _ = std::fs::remove_file(&wav);
    r?;
    if raw.is_empty() {
        return Err("nothing was transcribed (no speech, or the model failed to load)".into());
    }
    prog.set(1.0, format!("{} segments", raw.len()));
    Ok(())
}

// ---------------------------------------------------------------- parsing

/// `[00:00:01.000 --> 00:00:02.400]   Hello there` → (1.0, 2.4, "Hello there").
/// Anything else (whisper's banner, timings, blank-audio markers) is None.
pub fn parse_line(line: &str) -> Option<(f64, f64, String)> {
    let (span, text) = line.trim().strip_prefix('[')?.split_once(']')?;
    let (a, b) = span.split_once("-->")?;
    let start = crate::engine::subtitles::parse_time(a.trim())?;
    let end = crate::engine::subtitles::parse_time(b.trim())?;
    let text = text.trim();
    // "[BLANK_AUDIO]", "(silence)", "[MUSIC]" — a whole-line annotation is not speech
    let noise = text.is_empty()
        || (text.starts_with('[') && text.ends_with(']'))
        || (text.starts_with('(') && text.ends_with(')'));
    if end < start || noise {
        return None;
    }
    Some((start, end, text.to_string()))
}

/// How one-word segments are regrouped into sentences — every knob the "Regenerate" pass turns.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupOpts {
    /// A sentence ends once it reaches this many characters.
    pub max_chars: usize,
    /// … or after a silence longer than this (seconds).
    pub max_gap: f64,
    /// … or on a word ending with any of these characters ("" = never split on punctuation).
    pub punct: String,
    /// … or after this many words (0 = no word limit).
    pub max_words: usize,
}

impl Default for GroupOpts {
    fn default() -> Self {
        Self { max_chars: 120, max_gap: 0.8, punct: ".?!".into(), max_words: 0 }
    }
}

/// One-word segments (`-ml 1 -sow`) → sentences, keeping the word timings. A sentence ends per
/// `GroupOpts`: on punctuation, on a long gap, at `max_chars`, or at `max_words`.
pub fn group_words(words: &[(f64, f64, String)], opts: &GroupOpts) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let mut cur = Segment::default();
    for (i, (a, b, w)) in words.iter().enumerate() {
        let w = w.trim();
        if w.is_empty() {
            continue;
        }
        if cur.words.is_empty() {
            cur.start = *a;
        }
        if !cur.text.is_empty() {
            cur.text.push(' ');
        }
        cur.text.push_str(w);
        cur.end = *b;
        cur.words.push((*a, *b, w.to_string()));
        let gap = words.get(i + 1).map_or(f64::INFINITY, |n| n.0 - *b);
        let full =
            cur.text.chars().count() >= opts.max_chars || (opts.max_words > 0 && cur.words.len() >= opts.max_words);
        if w.ends_with(|c: char| opts.punct.contains(c)) || gap > opts.max_gap || full {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.words.is_empty() {
        out.push(cur);
    }
    out
}

/// Source seconds → timeline seconds: the clip's speed is one linear map over its whole range.
pub fn retime(segs: &mut [Segment], offset: f64, scale: f64) {
    let map = |t: &mut f64| *t = offset + *t * scale;
    for s in segs {
        map(&mut s.start);
        map(&mut s.end);
        for w in &mut s.words {
            map(&mut w.0);
            map(&mut w.1);
        }
    }
}

// ---------------------------------------------------------------- auto-subs

/// Greedy line wrap on word boundaries; a single word longer than `max_chars` gets its own line.
fn wrap_words(text: &str, max_chars: usize) -> Vec<Vec<&str>> {
    let mut out: Vec<Vec<&str>> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut len = 0usize;
    for w in text.split_whitespace() {
        let n = w.chars().count();
        if !cur.is_empty() && len + 1 + n > max_chars {
            out.push(std::mem::take(&mut cur));
            len = 0;
        }
        len += if cur.is_empty() { n } else { n + 1 };
        cur.push(w);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Segments → subtitle cues: wrapped to `max_chars`, timed by the word timings when there are any and
/// proportionally to the characters otherwise, each held for at least `min_dur` unless the next cue
/// needs the time.
pub fn to_cues(segs: &[Segment], max_chars: usize, min_dur: f64) -> Vec<(f64, f64, String)> {
    let max_chars = max_chars.max(8);
    let mut out: Vec<(f64, f64, String)> = Vec::new();
    for s in segs {
        let chunks = wrap_words(&s.text, max_chars);
        if chunks.is_empty() {
            continue;
        }
        let span = (s.end - s.start).max(0.0);
        let total: usize = chunks.iter().map(|c| c.join(" ").chars().count()).sum::<usize>().max(1);
        let (mut cum, mut cursor, mut t0) = (0usize, 0usize, s.start);
        for (i, ch) in chunks.iter().enumerate() {
            let text = ch.join(" ");
            cum += text.chars().count();
            let last = i + 1 == chunks.len();
            let (mut a, mut b) = (t0, if last { s.end } else { s.start + span * cum as f64 / total as f64 });
            if cursor + ch.len() <= s.words.len() {
                a = s.words[cursor].0;
                b = s.words[cursor + ch.len() - 1].1;
                cursor += ch.len();
            }
            t0 = b;
            out.push((a, b.max(a), text));
        }
    }
    for i in 0..out.len() {
        let next = out.get(i + 1).map_or(f64::INFINITY, |c| c.0);
        let want = out[i].0 + min_dur;
        if out[i].1 < want {
            out[i].1 = want.min(next.max(out[i].1));
        }
    }
    out
}

// ---------------------------------------------------------------- double takes

/// Lower-cased words with the punctuation stripped ("So, THAT one!" → ["so", "that", "one"]).
pub fn normalize(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_alphanumeric() || *c == '\'').collect::<String>().to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

/// How alike two token lists are, 0..=1: twice their longest common subsequence over their combined
/// length. Order-aware (unlike a bag overlap) and forgiving of the "um"s a retake adds.
pub fn similarity(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut prev = vec![0u32; b.len() + 1];
    let mut cur = vec![0u32; b.len() + 1];
    for x in a {
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = if x == y { prev[j] + 1 } else { cur[j].max(prev[j + 1]) };
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    2.0 * prev[b.len()] as f32 / (a.len() + b.len()) as f32
}

/// Groups of segments that say the same thing again — a flubbed line and its retakes. Each group is in
/// time order with at least two members and the LAST one is the keeper. `window` is how long a silence
/// may sit between one take and the next.
///
/// ponytail: segment against segment, so a take spanning several segments only matches segment-wise.
/// Align token runs across segment boundaries if multi-sentence takes ever need catching.
pub fn duplicate_takes(segs: &[Segment], threshold: f32, window: f64) -> Vec<Vec<usize>> {
    let toks: Vec<Vec<String>> = segs.iter().map(|s| normalize(&s.text)).collect();
    let mut taken = vec![false; segs.len()];
    let mut out: Vec<Vec<usize>> = Vec::new();
    for i in 0..segs.len() {
        if taken[i] || toks[i].len() < MIN_WORDS {
            continue;
        }
        let mut group = vec![i];
        let mut last_end = segs[i].end;
        for j in i + 1..segs.len() {
            if segs[j].start - last_end > window {
                break;
            }
            if taken[j] || toks[j].len() < MIN_WORDS {
                continue;
            }
            if similarity(&toks[i], &toks[j]) >= threshold {
                group.push(j);
                last_end = segs[j].end;
            }
        }
        if group.len() > 1 {
            for &g in &group {
                taken[g] = true;
            }
            out.push(group);
        }
    }
    out
}

/// Timeline ranges of every take but the last of each group — what "cut the duplicates" removes.
/// Sorted and merged so two duplicates in a row are one cut.
pub fn dup_ranges(segs: &[Segment], groups: &[Vec<usize>]) -> Vec<(f64, f64)> {
    let mut r: Vec<(f64, f64)> = groups
        .iter()
        .flat_map(|g| g[..g.len().saturating_sub(1)].iter())
        .filter_map(|&i| segs.get(i))
        .map(|s| (s.start, s.end))
        .collect();
    r.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (a, b) in r {
        match out.last_mut() {
            Some(l) if a <= l.1 + MERGE_GAP => l.1 = l.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Where `t` ends up after those ranges are rippled out — None when `t` was inside one of them.
/// Used to drag the cues and the transcript along with the cut.
pub fn ripple_time(t: f64, removed: &[(f64, f64)]) -> Option<f64> {
    let mut shift = 0.0;
    for &(a, b) in removed {
        if t >= b {
            shift += b - a;
        } else if t >= a {
            return None; // the ranges are half-open: the start goes, the end survives
        }
    }
    Some(t - shift)
}

/// Name for a marker on a duplicate take: the transcript, short enough to read on the timeline.
pub fn short_label(text: &str, max: usize) -> String {
    let mut s: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, end: f64, text: &str) -> Segment {
        Segment::plain(start, end, text.into())
    }

    #[test]
    fn parses_whisper_lines() {
        assert_eq!(
            parse_line("[00:00:01.000 --> 00:00:02.400]   Hello there"),
            Some((1.0, 2.4, "Hello there".to_string()))
        );
        // whisper's own chatter, blank audio and reversed spans are not transcript
        assert_eq!(parse_line("whisper_init_from_file_with_params_no_state: loading model"), None);
        assert_eq!(parse_line("[00:00:03.000 --> 00:00:05.000]   [BLANK_AUDIO]"), None);
        assert_eq!(parse_line("[00:00:03.000 --> 00:00:05.000]   (soft music)"), None);
        assert_eq!(parse_line("[00:00:03.000 --> 00:00:05.000]"), None);
        assert_eq!(parse_line("[00:00:05.000 --> 00:00:03.000]  backwards"), None);
        assert_eq!(parse_line(""), None);
    }

    #[test]
    fn parses_whole_output() {
        let out = "whisper_model_load: loading model\n\n[00:00:00.000 --> 00:00:02.000]   One two.\n\
                   [00:00:02.000 --> 00:00:04.000]   [BLANK_AUDIO]\n[00:00:04.000 --> 00:00:06.000]   Three.\n\
                   whisper_print_timings: total time = 1234 ms\n";
        // exactly what the worker does with the child's stdout
        let segs: Vec<Segment> = out.lines().filter_map(parse_line).map(|(a, b, t)| Segment::plain(a, b, t)).collect();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0], seg(0.0, 2.0, "One two."));
        assert_eq!(segs[1].start, 4.0);
    }

    #[test]
    fn word_segments_group_into_sentences() {
        let words: Vec<(f64, f64, String)> = [
            (0.0, 0.3, "One"),
            (0.3, 0.6, "two."),
            (0.7, 1.0, "Then"),
            (1.0, 1.3, "three"),
            (5.0, 5.4, "later"), // a long gap starts a new sentence
        ]
        .iter()
        .map(|(a, b, w)| (*a, *b, w.to_string()))
        .collect();
        let segs = group_words(&words, &GroupOpts::default());
        assert_eq!(segs.len(), 3, "{segs:?}");
        assert_eq!(segs[0].text, "One two.");
        assert_eq!((segs[0].start, segs[0].end), (0.0, 0.6));
        assert_eq!(segs[1].text, "Then three");
        assert_eq!(segs[2].text, "later");
        assert_eq!(segs[1].words.len(), 2, "word timings survive the grouping");
        // max_chars breaks a run that never punctuates
        let long: Vec<(f64, f64, String)> = (0..10).map(|i| (i as f64, i as f64 + 0.5, "word".to_string())).collect();
        let by_chars = GroupOpts { max_chars: 12, max_gap: 5.0, ..GroupOpts::default() };
        assert!(group_words(&long, &by_chars).len() > 2);
        // max_words and custom punctuation are further splits
        let by_words = GroupOpts { max_gap: 5.0, max_words: 3, ..GroupOpts::default() };
        assert!(group_words(&long, &by_words).iter().all(|s| s.words.len() <= 3));
        let commas: Vec<(f64, f64, String)> =
            [(0.0, 0.4, "so,"), (0.5, 0.9, "yes")].iter().map(|(a, b, w)| (*a, *b, w.to_string())).collect();
        let on_comma = GroupOpts { punct: ".?!,".into(), max_gap: 5.0, ..GroupOpts::default() };
        assert_eq!(group_words(&commas, &on_comma).len(), 2);
        let no_punct = GroupOpts { punct: String::new(), max_gap: 5.0, ..GroupOpts::default() };
        assert_eq!(group_words(&commas, &no_punct).len(), 1);
    }

    #[test]
    fn cues_wrap_on_words_and_respect_the_minimum() {
        let segs = vec![seg(0.0, 4.0, "the quick brown fox jumps over the lazy dog")];
        let cues = to_cues(&segs, 20, 0.5);
        assert!(cues.len() >= 2, "{cues:?}");
        for c in &cues {
            assert!(c.2.chars().count() <= 20, "line too long: {:?}", c.2);
            assert!(c.1 > c.0);
        }
        assert_eq!(cues.iter().map(|c| c.2.clone()).collect::<Vec<_>>().join(" "), segs[0].text);
        assert_eq!(cues[0].0, 0.0);
        // the tail word "dog" is too short to read, so it lingers past the segment (nothing follows it)
        let tail = cues.last().unwrap();
        assert!(tail.1 >= 4.0 && tail.1 <= 4.5, "{cues:?}");
        // a one-word flash is held for min_dur, but never past the next cue
        let segs = vec![seg(0.0, 0.2, "Hi"), seg(0.5, 3.0, "there")];
        let cues = to_cues(&segs, 42, 1.0);
        assert_eq!(cues.len(), 2);
        assert!((cues[0].1 - 0.5).abs() < 1e-9, "clamped to the next cue: {cues:?}");
        assert!((cues[1].1 - 3.0).abs() < 1e-9);
        assert!(to_cues(&[seg(0.0, 1.0, "   ")], 42, 1.0).is_empty());
    }

    #[test]
    fn cues_use_word_timings_when_present() {
        let mut s = seg(0.0, 6.0, "one two three four");
        s.words = vec![
            (0.0, 0.5, "one".into()),
            (0.5, 1.0, "two".into()),
            (4.0, 4.5, "three".into()),
            (4.5, 6.0, "four".into()),
        ];
        let cues = to_cues(&[s], 12, 0.1);
        assert_eq!(cues.len(), 2, "{cues:?}");
        assert_eq!(cues[0], (0.0, 1.0, "one two".to_string()), "the pause is not covered by cue 1");
        assert_eq!(cues[1], (4.0, 6.0, "three four".to_string()));
    }

    #[test]
    fn similarity_is_order_aware() {
        let n = normalize;
        assert!((similarity(&n("So this is the take"), &n("So this is the take")) - 1.0).abs() < 1e-6);
        // a retake with a filler word is still the same line
        assert!(similarity(&n("So this is the take"), &n("So uh this is the take")) > 0.9);
        assert!(similarity(&n("So this is the take"), &n("Completely different words here")) < 0.3);
        assert_eq!(similarity(&n("hello"), &[]), 0.0);
        // punctuation and case do not count
        assert!((similarity(&n("Hello, world!"), &n("hello world")) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn duplicate_takes_group_retakes_and_keep_the_last() {
        let segs = vec![
            seg(0.0, 2.0, "Welcome to the channel"),
            seg(2.5, 4.5, "Welcome to the, uh, channel"), // retake
            seg(5.0, 7.0, "Welcome to the channel!"),     // final take
            seg(8.0, 10.0, "Today we are building a video editor"),
            seg(60.0, 62.0, "Welcome to the channel"), // a minute later: not the same take
        ];
        let groups = duplicate_takes(&segs, 0.8, 10.0);
        assert_eq!(groups, vec![vec![0, 1, 2]], "{groups:?}");
        let ranges = dup_ranges(&segs, &groups);
        assert_eq!(ranges, vec![(0.0, 2.0), (2.5, 4.5)], "both flubbed takes go, the keeper stays");
        // takes back to back are cut as one range
        let tight = vec![seg(0.0, 2.0, "Welcome to the channel"), seg(2.1, 4.0, "Welcome to the channel")];
        let g = duplicate_takes(&tight, 0.8, 10.0);
        assert_eq!(dup_ranges(&tight, &g), vec![(0.0, 2.0)]);
        // raising the bar past the filler word splits the group
        assert_eq!(duplicate_takes(&segs, 0.99, 10.0), vec![vec![0, 2]]);
        // "yeah" twice is not a double take
        let short = vec![seg(0.0, 0.5, "Yeah"), seg(1.0, 1.5, "Yeah")];
        assert!(duplicate_takes(&short, 0.8, 10.0).is_empty());
        assert!(duplicate_takes(&[], 0.8, 10.0).is_empty());
    }

    #[test]
    fn ripple_time_follows_the_cut() {
        let removed = [(1.0, 3.0), (5.0, 6.0)];
        assert_eq!(ripple_time(0.5, &removed), Some(0.5));
        assert_eq!(ripple_time(2.0, &removed), None, "inside a cut");
        assert_eq!(ripple_time(1.0, &removed), None, "the cut's own start goes with it");
        assert_eq!(ripple_time(3.0, &removed), Some(1.0), "its end is the first surviving instant");
        assert_eq!(ripple_time(4.0, &removed), Some(2.0));
        assert_eq!(ripple_time(7.0, &removed), Some(4.0));
        assert_eq!(ripple_time(7.0, &[]), Some(7.0));
    }

    #[test]
    fn model_lookup_is_sane() {
        assert!(MODELS.iter().all(|(_, f, mb)| f.starts_with("ggml-") && *mb > 10));
        assert!(model_url("ggml-base.en.bin").ends_with("/ggml-base.en.bin"));
        assert!(model_path("ggml-base.en.bin").ends_with("ggml-base.en.bin"));
        assert!(!have_model("ggml-nope-does-not-exist.bin"));
        assert!(install_hint().contains("whisper-cli.exe"));
        assert_eq!(short_label("a very long transcript line", 6), "a very…");
        assert_eq!(short_label("short", 20), "short");
    }
}
