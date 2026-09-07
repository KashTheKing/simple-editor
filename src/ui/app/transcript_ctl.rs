//! ---- ws:transcript-captions ----
//! App-level glue for transcripts: the ACT_HANDLERS arm for the six unbound Actions (clip menu
//! "Transcript ▸ Transcribe… / View transcript / Export transcript…", Get captions, Remove fillers,
//! Toggle transcript), the FRAME_HOOK that runs whisper / tracking / TTS jobs started outside the
//! Subtitles pane's own button (progress toasts via `feedback::Toast`, results applied on the UI
//! thread as ONE labeled undo step), and the WINDOW_DRAWER for the non-blocking "View transcript"
//! window (`ui::transcript_ui::window`).
//!
//! Every job started here carries an OUTER `Progress` that only completes once its result is in the
//! project - so an MCP `transcribe.run`/`media.transcribe`/`tracking.run`/`tts.speak` reply
//! (`poll_mcp` replies when the job's `Progress` is done) never lands before `media.transcript` /
//! `clip.get` can see the words / keyframes.

use super::feedback::{Toast, ToastKind};
use super::*;
use crate::engine::tracking::TrackJob;
use crate::engine::transcribe::{self, Options};
use crate::engine::tts;
use crate::ui::{confirm, transcript_ui};

/// A whisper run started from the clip menu / an MCP tool (the pane's own runs live in
/// `TranscribeState`): the raw job, the source → timeline map, whether to also generate cues.
struct TranscribeRun {
    clip: Id,
    map: (f64, f64),
    job: transcribe::Job,
    outer: Arc<Progress>,
    gen_cues: bool,
    label: String,
}

/// `tracking.run`: `TrackJob` polled on a worker (see `App::start_tracking`); the points land in
/// `points` and, with `apply`, onto the clip's X/Y when done.
struct TrackingRun {
    clip: Id,
    inner: Arc<Progress>,
    points: Arc<Mutex<Vec<(f32, f32, f32)>>>,
    apply: bool,
    outer: Arc<Progress>,
}

/// A `tts::speak_to_wav` job; on success the WAV is imported and placed at `at`, linked to `link_to`.
struct TtsRun {
    inner: Arc<Progress>,
    out: PathBuf,
    link_to: Option<Id>,
    at: f64,
    outer: Arc<Progress>,
}

#[derive(Default)]
pub(super) struct TranscriptState {
    runs: Vec<TranscribeRun>,
    tracking: Vec<TrackingRun>,
    tts: Vec<TtsRun>,
    /// A model download started from the clip menu / palette; `Some(clip)` = transcribe it when done.
    download: Option<(Arc<Progress>, Option<Id>)>,
    /// `tts::voices()` running on a thread (a second of powershell - never on the UI thread, never
    /// at startup: only once the Speech panel is first opened).
    voices_rx: Option<Receiver<Vec<String>>>,
    pub(super) window: transcript_ui::TranscriptWindow,
}

/// The clip the menu/palette actions target: the first selected clip with footage, else the first
/// selected clip at all (a right-click selects the clip under the pointer first - timeline/mod.rs).
fn first_clip(app: &App) -> Option<Id> {
    app.selection
        .iter()
        .find(|&&id| app.project.clip(id).is_some_and(|c| c.uses_asset()))
        .or(app.selection.first())
        .copied()
}

/// "tiny.en" out of "tiny.en - fastest".
fn short_model(name: &str) -> &str {
    name.split(" - ").next().unwrap_or(name)
}

fn progress_toast(app: &mut App, msg: impl Into<String>, p: &Arc<Progress>) {
    let mut t = Toast::new(msg);
    t.progress = Some(p.clone());
    app.push_toast(t);
}

/// Transcribe `clip` now, or - with no model yet - offer the download first (a non-blocking confirm
/// naming the exact size; the clip is transcribed as soon as the model lands). Never silently fetches.
fn transcribe_or_offer(app: &mut App, clip: Id) {
    if transcribe::exe().is_none() {
        app.toast(transcribe::install_hint());
        return;
    }
    if let Err(e) = transcribe::target_for(&app.project, clip) {
        app.toast(e);
        return;
    }
    let (name, file, mb) = app.subtitles_ui.transcribe.model();
    if !transcribe::have_model(file) {
        if app.transcript.download.as_ref().is_some_and(|(p, _)| !p.is_done()) {
            app.toast("The whisper model is still downloading - the clip is transcribed when it lands");
            return;
        }
        confirm::ask_app(
            "Get captions",
            format!(
                "Transcribing needs the whisper model {} - download {mb} MB once from huggingface.co into {}?\n\
                 The clip is transcribed as soon as it lands.",
                short_model(name),
                transcribe::models_dir().display()
            ),
            move |app| app.transcript.download = Some((transcribe::download_model(file), Some(clip))),
        );
        return;
    }
    if let Err(e) = app.transcribe_clip(clip, None, None, true) {
        app.toast(e);
    }
}

/// Write `words` as `format` (or by `path`'s extension, default txt). Shared by the menu's
/// "Export transcript…" and the `transcript.export` tool.
pub(super) fn write_transcript(
    clip: Id,
    words: &[(f64, f64, String)],
    path: &Path,
    format: Option<&str>,
) -> Result<&'static str, String> {
    let fmt = match format {
        Some(f) => transcribe::export_format(f).ok_or_else(|| format!("unknown format '{f}' (txt, srt or json)"))?,
        None => path.extension().and_then(|e| e.to_str()).and_then(transcribe::export_format).unwrap_or("txt"),
    };
    std::fs::write(path, transcribe::export_transcript(fmt, clip, words)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(fmt)
}

/// "Export transcript…": an rfd SAVE dialog (a file picker, not a blocking Yes/No), format by the
/// chosen extension.
fn export_dialog(app: &mut App) {
    let Some(clip) = first_clip(app) else {
        app.toast("Select a transcribed clip first");
        return;
    };
    let Some(words) = app.project.transcript(clip).map(|t| t.words.clone()) else {
        app.toast("No transcript yet - right-click the clip ▸ Transcript ▸ Transcribe… first");
        return;
    };
    let name = transcript_ui::clip_label(&app.project, clip);
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Plain text", &["txt"])
        .add_filter("SubRip subtitles", &["srt"])
        .add_filter("JSON", &["json"])
        .set_file_name(format!("{name}.txt"))
        .save_file()
    else {
        return;
    };
    match write_transcript(clip, &words, &path, None) {
        Ok(fmt) => app.toast_with_folder(format!("Transcript exported as .{fmt}"), path),
        Err(e) => app.toast(e),
    }
}

/// `Action::RemoveFillers`, Mark-instead first: the first press drops a range marker per filler (so
/// the hits can be looked over), the next press cuts them and takes the marks away.
fn remove_fillers_action(app: &mut App) {
    let clip = app
        .subtitles_ui
        .transcript
        .clip
        .or_else(|| first_clip(app))
        .filter(|&c| app.project.transcript(c).is_some());
    let Some(clip) = clip else {
        app.toast("No transcript to clean - transcribe a clip first");
        return;
    };
    let words = app.project.transcript(clip).map(|t| t.words.clone()).unwrap_or_default();
    let ranges = {
        let fillers: Vec<&str> = app.settings.filler_words.iter().map(String::as_str).collect();
        transcribe::filler_ranges(&words, &fillers, app.settings.filler_pad_ms)
    };
    if ranges.is_empty() {
        app.toast("No filler words found");
        return;
    }
    let before = app.project.to_json();
    let marks = std::mem::take(&mut app.subtitles_ui.transcript.filler_marks);
    if marks.is_empty() {
        let ids = transcript_ui::mark_ranges(&mut app.project, &words, &ranges);
        app.subtitles_ui.transcript.filler_marks.clone_from(&ids);
        app.push_undo_labeled(before, "Mark Fillers");
        app.fire_markers_added(&ids);
        app.after_edit();
        app.toast(format!("Marked {} filler(s) - Remove Filler Words again to cut them", ids.len()));
        return;
    }
    for id in marks {
        app.project.remove_marker(id);
    }
    let n = app.project.cut_word_ranges(clip, &ranges);
    if n == 0 {
        app.run_rollback(before);
        app.toast("Nothing cut - the track is locked, or the clip has a speed ramp");
        return;
    }
    app.push_undo_labeled(before, "Remove Fillers");
    app.after_edit();
    app.toast_undo(format!("Removed {} filler(s)", ranges.len()), Action::Undo);
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::TranscribeClip => {
            match first_clip(app) {
                Some(c) => transcribe_or_offer(app, c),
                None => app.toast("Select a video or audio clip to transcribe"),
            }
            true
        }
        Action::ViewTranscript => {
            match first_clip(app) {
                Some(c) => {
                    let w = &mut app.transcript.window;
                    if w.clip != c {
                        w.query.clear();
                    }
                    w.clip = c;
                    w.open = true;
                }
                None => app.toast("Select a clip to view its transcript"),
            }
            true
        }
        Action::ExportTranscript => {
            export_dialog(app);
            true
        }
        Action::ToggleTranscript => {
            app.subtitles_ui.show_transcript = !app.subtitles_ui.show_transcript;
            app.surface(Pane::Subtitles);
            true
        }
        Action::GetCaptions => {
            // the button itself lives in the Subtitles pane and names the size - surface it there
            // rather than fetching from a palette row that can't show the MB before the click
            app.subtitles_ui.transcribe.open = true;
            app.surface(Pane::Subtitles);
            let (name, file, mb) = app.subtitles_ui.transcribe.model();
            if transcribe::have_model(file) {
                app.toast(format!("whisper {} is already downloaded - select a clip and Transcribe", short_model(name)));
            } else {
                app.toast(format!("Subtitles ▸ Transcribe ▸ \"Get captions\" downloads whisper {} ({mb} MB) once", short_model(name)));
            }
            true
        }
        Action::RemoveFillers => {
            remove_fillers_action(app);
            true
        }
        _ => false,
    }
}

impl App {
    /// Start whisper on `clip` as a background job (word timings always on): the words are written
    /// to `Project.transcripts` when it finishes (`finish_transcribe`), with cues generated too when
    /// `gen_cues`. `model` = a `transcribe::MODELS` name/file (default: the pane's pick). The returned
    /// `Progress` is the OUTER one (done only once the project holds the result).
    pub(super) fn transcribe_clip(
        &mut self,
        clip: Id,
        model: Option<&str>,
        language: Option<&str>,
        gen_cues: bool,
    ) -> Result<Arc<Progress>, String> {
        if transcribe::exe().is_none() {
            return Err(transcribe::install_hint());
        }
        let t = transcribe::target_for(&self.project, clip)?;
        let file: &'static str = match model.map(str::trim).filter(|m| !m.is_empty()) {
            Some(m) => transcribe::MODELS
                .iter()
                .find(|(n, f, _)| *f == m || n.starts_with(m) || short_model(n) == m)
                .map(|m| m.1)
                .ok_or_else(|| {
                    let names: Vec<&str> = transcribe::MODELS.iter().map(|m| short_model(m.0)).collect();
                    format!("unknown model '{m}' (one of: {})", names.join(", "))
                })?,
            None => self.subtitles_ui.transcribe.model().1,
        };
        if !transcribe::have_model(file) {
            return Err(format!("model {file} is not downloaded (transcribe.install, or Subtitles ▸ Get captions)"));
        }
        if self.transcript.runs.iter().any(|r| r.clip == clip) {
            return Err("that clip is already being transcribed".into());
        }
        let st = &self.subtitles_ui.transcribe;
        let job = transcribe::start(Options {
            path: t.path,
            src_start: t.src_start,
            src_duration: t.src_dur,
            model: file.to_string(),
            language: language.map(str::to_string).unwrap_or_else(|| st.language.clone()),
            words: true,
            prompt: st.prompt.clone(),
        });
        let outer = Progress::new();
        let label = format!("Transcribing {}…", transcript_ui::clip_label(&self.project, clip));
        self.transcript.runs.push(TranscribeRun { clip, map: (t.offset, t.scale), job, outer: outer.clone(), gen_cues, label });
        Ok(outer)
    }

    /// `tracking.run`: NCC point-track `clip` on a worker (`TrackJob` polled there, its progress
    /// forwarded), the path applied as X/Y keyframes when done if `apply`.
    pub(super) fn start_tracking(
        &mut self,
        clip: Id,
        rect: (f32, f32, f32, f32),
        search: f32,
        backward: bool,
        apply: bool,
    ) -> Result<Arc<Progress>, String> {
        let mut job = TrackJob::start(&self.project, clip, rect, search, 0, backward, self.backend())?;
        let points: Arc<Mutex<Vec<(f32, f32, f32)>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = points.clone();
        // ponytail: busy-poll the channel-based TrackJob at a short interval instead of restructuring
        // it around Progress - smallest diff that presents the uniform Job contract.
        let inner = export::spawn_job("tracking", move |prog| {
            loop {
                if prog.is_cancelled() {
                    return Err(export::CANCELLED.into());
                }
                let running = job.poll();
                prog.set(job.progress.clamp(0.0, 0.99), "Tracking…");
                if !running {
                    break;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
            if job.points.len() < 2 {
                return Err("nothing was tracked (no frames decoded)".into());
            }
            *sink.lock().unwrap_or_else(|e| e.into_inner()) = std::mem::take(&mut job.points);
            prog.set(1.0, "Done");
            Ok(())
        });
        let outer = Progress::new();
        self.transcript.tracking.push(TrackingRun { clip, inner, points, apply, outer: outer.clone() });
        Ok(outer)
    }

    /// `tts.speak` / the Speech panel: synthesize `text` into a cached WAV, imported at `at` (linked
    /// to `link_to`, a text clip) once it is written.
    pub(super) fn speak(
        &mut self,
        text: &str,
        voice: Option<&str>,
        link_to: Option<Id>,
        at: f64,
    ) -> Result<Arc<Progress>, String> {
        if text.trim().is_empty() {
            return Err("nothing to say".into());
        }
        let out = Settings::cache_dir().join("tts").join(format!("tts-{}.wav", Settings::now()));
        let inner = tts::speak_to_wav(text, voice, &out);
        let outer = Progress::new();
        self.transcript.tts.push(TtsRun { inner, out, link_to, at: at.max(0.0), outer: outer.clone() });
        Ok(outer)
    }
}

fn finish_transcribe(app: &mut App, run: TranscribeRun) {
    if let Some(e) = run.job.progress.error() {
        app.push_toast(Toast::new(format!("Transcribe failed: {e}")).kind(ToastKind::Error));
        run.outer.finish(Some(e));
        return;
    }
    if app.project.clip(run.clip).is_none() {
        run.outer.finish(Some("the clip was deleted while it was being transcribed".into()));
        return;
    }
    let mut segs = run.job.segments();
    transcribe::retime(&mut segs, run.map.0, run.map.1);
    let words: Vec<(f64, f64, String)> = segs.iter().map(|s| (s.start, s.end, s.text.clone())).collect();
    let before = app.project.to_json();
    app.project.set_transcript(run.clip, words.clone());
    let mut generated = Vec::new();
    if run.gen_cues {
        let (group, max_chars, lines, min_dur) = {
            let st = &app.subtitles_ui.transcribe;
            (st.group.clone(), st.max_chars, st.lines, st.min_dur)
        };
        let sentences = transcribe::group_words(&words, &group);
        let cont = (app.project.subtitle_cont_prefix.clone(), app.project.subtitle_cont_suffix.clone());
        let cues = transcribe::to_cues(&sentences, max_chars, lines, min_dur, (&cont.0, &cont.1));
        generated = cues.iter().map(|(s, e, t)| app.project.add_cue(*s, *e, t.clone())).collect();
    }
    app.subtitles_ui.transcribe.adopt(run.clip, run.map, words.clone(), generated.clone());
    app.push_undo_labeled(before, "Transcribe");
    app.after_edit();
    let name = transcript_ui::clip_label(&app.project, run.clip);
    let cues = if generated.is_empty() { String::new() } else { format!(", {} caption(s)", generated.len()) };
    app.push_toast(Toast::new(format!("Transcribed {name}: {} words{cues}", words.len())).kind(ToastKind::Success));
    run.outer.finish(None);
}

fn finish_tracking(app: &mut App, run: TrackingRun) {
    if let Some(e) = run.inner.error() {
        app.push_toast(Toast::new(format!("Tracking failed: {e}")).kind(ToastKind::Error));
        run.outer.finish(Some(e));
        return;
    }
    let points = std::mem::take(&mut *run.points.lock().unwrap_or_else(|e| e.into_inner()));
    if run.apply {
        let before = app.project.to_json();
        if app.project.apply_path(run.clip, &points) {
            app.push_undo_labeled(before, "Track");
            app.after_edit();
            app.push_toast(Toast::new(format!("Tracked {} frames onto the clip's X/Y", points.len())).kind(ToastKind::Success));
        } else {
            app.toast("Tracking found too few points to write a path");
        }
    }
    run.outer.finish(None);
}

fn finish_tts(app: &mut App, run: TtsRun) {
    if let Some(e) = run.inner.error() {
        app.push_toast(Toast::new(format!("Speech failed: {e}")).kind(ToastKind::Error));
        run.outer.finish(Some(e));
        return;
    }
    // a small local WAV: probe it right here (synchronously, like open_media) so the clip ids are
    // final and the link below survives - an async import's later `adopt` would re-create them
    let asset = match media::probe(&run.out.to_string_lossy(), app.backend()) {
        Ok(a) => a,
        Err(e) => {
            app.push_toast(Toast::new(format!("Speech WAV unreadable: {e}")).kind(ToastKind::Error));
            run.outer.finish(Some(e));
            return;
        }
    };
    let before = app.project.to_json();
    let aid = app.project.add_asset(asset);
    let new = app.project.insert_asset_clips(aid, run.at, None);
    if let Some(tc) = run.link_to.filter(|&tc| app.project.clip(tc).is_some()) {
        let link = match app.project.clip(tc).map(|c| c.link) {
            Some(l) if l != 0 => l,
            _ => app.project.new_id(),
        };
        for id in new.iter().copied().chain(std::iter::once(tc)) {
            if let Some(c) = app.project.clip_mut(id) {
                c.link = link;
            }
        }
    }
    app.push_undo_labeled(before, "Speak");
    app.after_edit();
    app.toast_with_folder("Speech placed on the timeline", run.out.clone());
    run.outer.finish(None);
}

/// FRAME_HOOK: settings ↔ section sync, the section's app-level requests, and every running job.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    // Settings.filler_words <-> the section's chip list (this module owns both ends; the section has
    // no Settings)
    {
        let st = &mut app.subtitles_ui.transcript;
        if st.fillers_dirty {
            app.settings.filler_words.clone_from(&st.fillers);
            app.settings.filler_pad_ms = st.filler_pad_ms;
            app.settings.save();
            st.fillers_dirty = false;
        } else if st.fillers != app.settings.filler_words || st.filler_pad_ms != app.settings.filler_pad_ms {
            st.fillers.clone_from(&app.settings.filler_words);
            st.filler_pad_ms = app.settings.filler_pad_ms;
        }
    }
    if let Some(clip) = app.subtitles_ui.transcript.want_transcribe.take() {
        transcribe_or_offer(app, clip);
    }
    if let Some((text, voice)) = app.subtitles_ui.transcript.tts_request.take() {
        let link_to =
            app.selection.iter().find(|&&id| app.project.clip(id).is_some_and(|c| c.kind == ClipKind::Text)).copied();
        let at = app.playhead;
        match app.speak(&text, voice.as_deref(), link_to, at) {
            Ok(p) => progress_toast(app, "Speaking…", &p),
            Err(e) => app.toast(e),
        }
    }
    if app.subtitles_ui.transcript.show_tts && app.subtitles_ui.transcript.tts_voices.is_none() {
        match &app.transcript.voices_rx {
            None => {
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(tts::voices().to_vec());
                });
                app.transcript.voices_rx = Some(rx);
            }
            Some(rx) => {
                if let Ok(v) = rx.try_recv() {
                    app.subtitles_ui.transcript.tts_voices = Some(v);
                    app.transcript.voices_rx = None;
                }
            }
        }
    }
    if let Some((p, clip)) = app.transcript.download.clone() {
        if p.is_done() {
            app.transcript.download = None;
            match p.error() {
                Some(e) => app.push_toast(Toast::new(format!("Model download failed: {e}")).kind(ToastKind::Error)),
                None => {
                    app.push_toast(Toast::new("whisper model downloaded").kind(ToastKind::Success));
                    if let Some(c) = clip {
                        transcribe_or_offer(app, c);
                    }
                }
            }
        } else {
            progress_toast(app, "Downloading the whisper model…", &p);
        }
    }
    let mut i = 0;
    while i < app.transcript.runs.len() {
        let r = &app.transcript.runs[i];
        if r.job.progress.is_done() {
            let run = app.transcript.runs.remove(i);
            finish_transcribe(app, run);
        } else {
            let (label, p) = (r.label.clone(), r.job.progress.clone());
            progress_toast(app, label, &p);
            i += 1;
        }
    }
    let mut i = 0;
    while i < app.transcript.tracking.len() {
        if app.transcript.tracking[i].inner.is_done() {
            let run = app.transcript.tracking.remove(i);
            finish_tracking(app, run);
        } else {
            let p = app.transcript.tracking[i].inner.clone();
            progress_toast(app, "Tracking…", &p);
            i += 1;
        }
    }
    let mut i = 0;
    while i < app.transcript.tts.len() {
        if app.transcript.tts[i].inner.is_done() {
            let run = app.transcript.tts.remove(i);
            finish_tts(app, run);
        } else {
            i += 1;
        }
    }
    let t = &app.transcript;
    let busy = t.download.is_some()
        || !t.runs.is_empty()
        || !t.tracking.is_empty()
        || !t.tts.is_empty()
        || t.voices_rx.is_some();
    if busy {
        ctx.request_repaint_after(Duration::from_millis(150));
    }
}

/// WINDOW_DRAWER: the "View transcript" window (word click = seek; "Transcribe…" when empty).
pub(super) fn window(app: &mut App, ctx: &egui::Context) {
    if !app.transcript.window.open {
        return;
    }
    let r = {
        let App { transcript, project, playhead, .. } = app;
        transcript_ui::window(ctx, &mut transcript.window, project, *playhead)
    };
    if let Some(t) = r.seek {
        app.playhead = t;
        app.player.pause();
        app.player.seek(t);
    }
    if r.transcribe {
        let c = app.transcript.window.clip;
        transcribe_or_offer(app, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_transcript_picks_the_format_by_extension_or_name() {
        let dir = std::env::temp_dir().join(format!("se-transcript-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let words = vec![(0.0, 0.4, "Hello".to_string()), (0.5, 0.9, "there.".to_string())];
        let srt = dir.join("t.srt");
        assert_eq!(write_transcript(3, &words, &srt, None), Ok("srt"));
        assert!(std::fs::read_to_string(&srt).unwrap().starts_with("1\n00:00:00,000 --> "));
        let json = dir.join("t.json");
        assert_eq!(write_transcript(3, &words, &json, None), Ok("json"));
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
        assert_eq!(v["clip_id"], 3);
        // an explicit format wins over the extension; an unknown extension means plain text
        let odd = dir.join("t.transcript");
        assert_eq!(write_transcript(3, &words, &odd, Some("srt")), Ok("srt"));
        assert_eq!(write_transcript(3, &words, &odd, None), Ok("txt"));
        assert_eq!(std::fs::read_to_string(&odd).unwrap(), "Hello there.");
        assert!(write_transcript(3, &words, &odd, Some("docx")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn short_model_name() {
        assert_eq!(short_model("tiny.en - fastest"), "tiny.en");
        assert_eq!(short_model("base"), "base");
    }
}
