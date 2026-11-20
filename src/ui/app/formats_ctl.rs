//! ---- ws:parity-formats ----
//! The App side of the Shotcut-parity formats (`engine::formats`, `engine::generators`,
//! `engine::lottie`): background jobs that probe, extract, mux or draw a file and then land their
//! result on the UI thread, the Subtitle-track export option, the track chooser window, and the
//! `subtitles.extract` / `subtitles.embed` / `media.generate` / `media.import_lottie` tools.
//!
//! Every slow step (ffprobe, ffmpeg, Edge, frame synthesis) runs on a job thread (`export::spawn_job`);
//! `tick` (FRAME_HOOKS) applies what finished: cues replace the project's (one undo step), files are
//! imported into the Library. A job an MCP caller waits on ends its `done` progress only once its
//! result is applied, so the agent's next call already sees it.

use super::tools_helpers::*;
use super::*;
use crate::engine::{formats, lottie, synth};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use std::collections::HashMap;
use crate::player::tracks::Track;

type Cues = Vec<(f64, f64, String)>;

#[derive(Default)]
pub(crate) struct Formats {
    jobs: Vec<Job>,
    /// A media file with more than one text subtitle track: the chooser window lists them.
    chooser: Option<(PathBuf, Vec<Track>)>,
    /// The export running now, when it should get a subtitle track afterwards: its file and its cues
    /// (already in the file's own time).
    pending_embed: Option<(PathBuf, Cues)>,
}

enum Job {
    /// The subtitle tracks of a file the user picked (the menu path; MCP names its track).
    Probe { prog: Arc<Progress>, path: PathBuf, slot: Arc<Mutex<Vec<Track>>> },
    /// One track's cues being read. `ask` = the menu path (asks before replacing cues); MCP replaces.
    Extract { prog: Arc<Progress>, label: String, slot: Arc<Mutex<Cues>>, done: Arc<Progress>, ask: bool },
    /// Soft subtitles being muxed into a file.
    Embed { prog: Arc<Progress>, out: PathBuf, done: Arc<Progress> },
    /// A file drawn in the background (an SVG, a generator, a Lottie animation), imported into `bin`
    /// when it lands; `what` names it in the toast. `renamed`: the job names its file itself (a
    /// Lottie's extension depends on what it draws).
    Make { prog: Arc<Progress>, out: PathBuf, bin: String, what: String, done: Arc<Progress>, renamed: Option<Arc<Mutex<Option<PathBuf>>>> },
}

impl Job {
    fn prog(&self) -> &Arc<Progress> {
        match self {
            Job::Probe { prog, .. } | Job::Extract { prog, .. } | Job::Embed { prog, .. } | Job::Make { prog, .. } => prog,
        }
    }
}

impl Formats {
    /// For the Jobs pane: what is running.
    pub(super) fn jobs(&self) -> Vec<jobs_pane::ProgJob> {
        self.jobs
            .iter()
            .filter_map(|j| match j {
                Job::Extract { prog, label, .. } => Some((crate::ui::jobs_ui::JobKind::Media, format!("Subtitles · {label}"), prog.clone())),
                Job::Embed { prog, out, .. } => {
                    Some((crate::ui::jobs_ui::JobKind::Media, format!("Subtitle track · {}", file_name(out)), prog.clone()))
                }
                Job::Make { prog, what, .. } => Some((crate::ui::jobs_ui::JobKind::Media, what.clone(), prog.clone())),
                Job::Probe { .. } => None,
            })
            .collect()
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

/// Where made media goes: `Generated\` beside the saved project, else in the app's data folder (as
/// pasted pictures do, `os_paste`).
pub(super) fn generated_dir(app: &App) -> PathBuf {
    match app.project_path.as_ref().and_then(|p| p.parent()) {
        Some(d) => d.join("Generated"),
        None => crate::settings::Settings::dir().join("Generated"),
    }
}

/// Files the Import dialog offers beyond `MEDIA_EXTS`: converted on the way in by `intercept`.
pub(super) const CONVERTED_EXTS: &[&str] = &["svg", "json", "lottie"];

/// The video containers a subtitle extraction offers in its file dialog.
const SUB_SOURCE_EXTS: &[&str] = &["mkv", "mp4", "m4v", "mov", "webm", "avi", "ts", "m2ts", "mts", "ogv", "wmv", "flv"];

// ---------------------------------------------------------------------------------------------------
// subtitle extraction

/// Subtitles ▸ Import from Media File…: pick a video, list its text subtitle tracks off the UI thread;
/// one track is read straight away, several open the chooser.
pub(super) fn extract_dialog(app: &mut App) {
    if app.ffmpeg_missing() {
        return;
    }
    let Some(path) = rfd::FileDialog::new().add_filter("Media files", SUB_SOURCE_EXTS).pick_file() else { return };
    let slot: Arc<Mutex<Vec<Track>>> = Arc::default();
    let (p, s) = (path.clone(), slot.clone());
    let prog = export::spawn_job("subtitle-probe", move |_| {
        *s.lock().unwrap_or_else(|e| e.into_inner()) = crate::player::tracks::probe(&p).subtitles;
        Ok(())
    });
    app.formats.jobs.push(Job::Probe { prog, path, slot });
}

/// The text subtitle tracks of `tracks` (picture subtitles cannot become text), side-car files too.
fn text_tracks(tracks: &[Track]) -> Vec<Track> {
    tracks.iter().filter(|t| !t.bitmap).cloned().collect()
}

/// Which track an extraction reads: one already chosen, or (MCP) an index into the file's subtitle
/// tracks / the first text track, looked up on the job thread.
enum Pick {
    Track(Track),
    Index(usize),
    FirstText,
}

/// The track `pick` names among `tracks`.
fn pick_track(tracks: &[Track], pick: Pick) -> Result<Track, String> {
    let t = match pick {
        Pick::Track(t) => t,
        Pick::Index(i) => tracks.get(i).cloned().ok_or_else(|| format!("no track {i} ({} found)", tracks.len()))?,
        Pick::FirstText => text_tracks(tracks).into_iter().next().ok_or("no text subtitle track in that file")?,
    };
    if t.bitmap {
        return Err("that track is picture subtitles, which can't become text".into());
    }
    Ok(t)
}

/// Start reading `track` of `path`. Returns the progress an MCP caller waits on.
fn start_extract(app: &mut App, path: &Path, track: Track, ask: bool) -> Arc<Progress> {
    let label = format!("{} · {}", file_name(path), track.label);
    start_extract_pick(app, path, Pick::Track(track), label, ask)
}

fn start_extract_pick(app: &mut App, path: &Path, pick: Pick, label: String, ask: bool) -> Arc<Progress> {
    let slot: Arc<Mutex<Cues>> = Arc::default();
    let (p, s) = (path.to_path_buf(), slot.clone());
    let prog = export::spawn_job("subtitle-extract", move |prog| {
        prog.set(0.1, "Reading subtitles…");
        let t = match pick {
            Pick::Track(t) => t,
            other => pick_track(&crate::player::tracks::probe(&p).subtitles, other)?,
        };
        let cues = crate::player::tracks::cues(&p, &t)?;
        if cues.is_empty() {
            return Err("that track has no cues".into());
        }
        *s.lock().unwrap_or_else(|e| e.into_inner()) = cues;
        Ok(())
    });
    let done = Progress::new();
    app.formats.jobs.push(Job::Extract { prog, label, slot, done: done.clone(), ask });
    done
}

/// WINDOW_DRAWER: the track chooser, while a file with several text tracks waits for a pick.
pub(super) fn windows(app: &mut App, ctx: &egui::Context) {
    let Some((path, tracks)) = app.formats.chooser.clone() else { return };
    let (open, pick) = track_chooser(ctx, &path, &tracks);
    if let Some(t) = pick {
        app.formats.chooser = None;
        start_extract(app, &path, t, true);
    } else if !open {
        app.formats.chooser = None;
    }
}

/// The chooser: one button per text track of the picked file. Returns (still open, the track picked).
fn track_chooser(ctx: &egui::Context, path: &Path, tracks: &[Track]) -> (bool, Option<Track>) {
    let mut open = true;
    let mut pick: Option<Track> = None;
    egui::Window::new("Subtitle tracks")
        .id(egui::Id::new("formats-subtitle-tracks"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label(file_name(path)).on_hover_text(path.display().to_string());
            ui.add_space(4.0);
            for t in tracks {
                let r = ui.add_sized([260.0, 24.0], egui::Button::new(&t.label));
                let r = if t.file.is_some() { r.on_hover_text("A subtitle file beside the video") } else { r };
                if r.clicked() {
                    pick = Some(t.clone());
                }
            }
        });
    (open, pick)
}

// ---------------------------------------------------------------------------------------------------
// soft subtitles

/// The project's cues as tuples, in timeline order.
fn project_cues(app: &App) -> Cues {
    app.project.subtitles.iter().map(|c| (c.start, c.end, c.text.clone())).collect()
}

fn srt_of(cues: &[(f64, f64, String)]) -> String {
    let cues: Vec<crate::model::Cue> =
        cues.iter().enumerate().map(|(i, (s, e, t))| crate::model::Cue { id: i as Id + 1, start: *s, end: *e, text: t.clone() }).collect();
    crate::engine::subtitles::to_srt(&cues)
}

/// An export is starting: remember its file and cues when the Subtitle track option applies to it.
pub(super) fn export_started(app: &mut App, out: &Path, range: Option<(f64, f64)>) {
    app.formats.pending_embed = None;
    let ext = export::ext_of(out);
    if !app.settings.export_subtitle_track || app.project.subtitles.is_empty() || formats::soft_sub_codec(&ext).is_none() {
        return;
    }
    let cues = formats::cues_in_range(&project_cues(app), range);
    if !cues.is_empty() {
        app.formats.pending_embed = Some((out.to_path_buf(), cues));
    }
}

/// An export finished: add its subtitle track (in place) when `export_started` asked for one.
pub(super) fn export_finished(app: &mut App, out: &Path, ok: bool) {
    let Some((path, cues)) = app.formats.pending_embed.take() else { return };
    if ok && path == out {
        start_embed(app, &path, &cues, None);
    }
}

/// Mux `cues` into `video` (`into` = None: in place). Returns the progress an MCP caller waits on.
fn start_embed(app: &mut App, video: &Path, cues: &[(f64, f64, String)], into: Option<PathBuf>) -> Arc<Progress> {
    let srt = srt_of(cues);
    // ponytail: no project language setting exists, so the track is tagged "und"; a `lang` arg is
    // the upgrade path if an agent or a user ever needs it named
    let lang = String::new();
    let (v, dst) = (video.to_path_buf(), into.clone());
    let prog = export::spawn_job("subtitle-embed", move |prog| {
        prog.set(0.1, "Adding the subtitle track…");
        formats::embed_subtitles(&v, &srt, dst.as_deref(), &lang).map(|_| ())
    });
    let done = Progress::new();
    let out = into.unwrap_or_else(|| video.to_path_buf());
    app.formats.jobs.push(Job::Embed { prog, out, done: done.clone() });
    done
}

/// Subtitles ▸ Embed in Video File…: pick a video, write `<name> (subtitled).<ext>` beside it with
/// the project's cues as a subtitle track. The picked file is never changed.
pub(super) fn embed_dialog(app: &mut App) {
    if app.ffmpeg_missing() {
        return;
    }
    if app.project.subtitles.is_empty() {
        app.toast("There are no subtitles to embed");
        return;
    }
    let Some(video) = rfd::FileDialog::new().add_filter("Video", &["mp4", "m4v", "mov", "mkv", "webm"]).pick_file() else {
        return;
    };
    let out = formats::subtitled_copy(&video);
    let cues = project_cues(app);
    start_embed(app, &video, &cues, Some(out));
}

// ---------------------------------------------------------------------------------------------------
// made files (SVG, generators, Lottie)

/// Draw something into a file on a job thread, then import it into library bin `bin`. `make` writes
/// the file at the path it is given. Returns (the progress an MCP caller waits on, the output path).
pub(super) fn start_make(
    app: &mut App,
    what: String,
    out: PathBuf,
    bin: &str,
    make: impl FnOnce(&Path, &Progress) -> Result<(), String> + Send + 'static,
) -> (Arc<Progress>, PathBuf) {
    let o = out.clone();
    let prog = export::spawn_job("make-media", move |p| {
        if let Some(dir) = o.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        make(&o, p)
    });
    let done = Progress::new();
    app.formats.jobs.push(Job::Make { prog, out: out.clone(), bin: bin.to_string(), what, done: done.clone(), renamed: None });
    (done, out)
}

/// Generate `spec` (`engine::synth`) into `Generated\` and import it into the Library's "Generated"
/// bin. Returns (the progress an MCP caller waits on, the output path).
pub(super) fn start_generate(app: &mut App, spec: synth::GenSpec) -> (Arc<Progress>, PathBuf) {
    let spec = spec.clamped();
    let name = spec.kind.label();
    let out = formats::converted_path(&generated_dir(app), Path::new(name), spec.file_ext());
    let text = app.text.clone();
    start_make(app, format!("Generate · {name}"), out, GENERATED_BIN, move |o, prog| {
        prog.set(0.0, format!("Generating {name}…"));
        let glyphs = if spec.kind == synth::GenKind::Counter { counter_glyphs(&text, &spec) } else { HashMap::new() };
        if spec.kind == synth::GenKind::Counter && glyphs.is_empty() {
            return Err("no font could draw the digits".into());
        }
        synth::encode(o, (spec.width, spec.height), spec.fps, spec.frames(), spec.still(), spec.alpha(), prog, |i, b| {
            synth::draw(&spec, i, &glyphs, b);
            Ok(())
        })
    })
}

/// The Library bin generators land in.
const GENERATED_BIN: &str = "Generated";

/// The counter's characters, drawn once by the app's text rasteriser (a bold monospaced face, a fifth
/// of the frame high) so each frame only copies pixels.
fn counter_glyphs(text: &Arc<Mutex<crate::engine::text::TextRasterizer>>, spec: &synth::GenSpec) -> HashMap<char, synth::Glyph> {
    let px = (spec.height as f64 * 0.22).round().max(8.0);
    let mut tr = text.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = HashMap::new();
    for c in synth::COUNTER_CHARS.chars() {
        let style = crate::model::TextStyle {
            text: c.to_string(),
            font: "Consolas, Cascadia Mono, Segoe UI".into(),
            size: crate::model::Animated::new(px),
            bold: true,
            color: spec.color,
            ..Default::default()
        };
        let f = tr.render(&style, 1.0, 0.0);
        if f.width > 1 && f.rgba.len() == (f.width * f.height * 4) as usize && !f.bgrx {
            out.insert(c, synth::Glyph { w: f.width, h: f.height, rgba: f.rgba.clone() });
        }
    }
    out
}

/// Render a Lottie animation (`.json` or `.lottie`) into `Generated\` and import it. Returns (the
/// progress an MCP caller waits on, the output path) - the path's extension is decided on the job
/// thread (still / transparent), so the returned one is the folder's name stem.
pub(super) fn start_lottie(app: &mut App, src: &Path) -> (Arc<Progress>, PathBuf) {
    let dir = generated_dir(app);
    let stem = PathBuf::from(src.file_stem().unwrap_or_default());
    let src_path = src.to_path_buf();
    // the output's own name is only known once the animation is read: the job writes it into `named`
    let named: Arc<Mutex<Option<PathBuf>>> = Arc::default();
    let n = named.clone();
    let placeholder = dir.join(format!("{}.lottie-pending", stem.display()));
    let (done, _) = start_make(app, format!("Lottie · {}", file_name(src)), placeholder.clone(), "", move |_, prog| {
        prog.set(0.0, "Reading the animation…");
        let comp = lottie::Comp::parse(&lottie::read_file(&src_path)?)?;
        let alpha = synth::lottie_alpha(&comp);
        let out = formats::converted_path(&dir, &stem, synth::output_ext(!comp.animated(), alpha));
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        synth::encode_lottie(&comp, &out, alpha, prog)?;
        *n.lock().unwrap_or_else(|e| e.into_inner()) = Some(out);
        if !comp.unsupported.is_empty() {
            prog.set(1.0, format!("{NOTE}{}", comp.unsupported.join(", ")));
        }
        Ok(())
    });
    if let Some(Job::Make { renamed, .. }) = app.formats.jobs.last_mut() {
        *renamed = Some(named);
    }
    (done, placeholder)
}

/// A made job's status that is a note for the user (what a Lottie file was drawn without).
const NOTE: &str = "Drawn without: ";

/// Is `p` a Lottie animation? `.lottie` always; a `.json` when its first 64 KB has Bodymovin's keys.
fn is_lottie_file(p: &Path) -> bool {
    match p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).as_deref() {
        Some("lottie") => true,
        Some("json") => {
            use std::io::Read;
            let mut head = vec![0u8; 64 * 1024];
            let n = std::fs::File::open(p).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
            lottie::is_lottie(&String::from_utf8_lossy(&head[..n]))
        }
        _ => false,
    }
}

/// The import funnel's gate for the formats that are converted first: an `.svg` is drawn to a PNG and
/// a Lottie animation rendered to a clip (in `Generated\`, imported when they land); every other
/// path passes through untouched.
pub(super) fn intercept(app: &mut App, paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut rest = Vec::with_capacity(paths.len());
    for p in paths {
        if is_lottie_file(p) {
            if !app.ffmpeg_missing() {
                start_lottie(app, p);
            }
        } else if formats::is_svg(p) {
            let out = formats::converted_path(&generated_dir(app), p, "png");
            let src = p.clone();
            start_make(app, format!("SVG · {}", file_name(p)), out, "", move |o, prog| {
                prog.set(0.2, "Drawing the SVG…");
                formats::svg_to_png(&src, o).map(|_| ())
            });
        } else {
            rest.push(p.clone());
        }
    }
    rest
}

// ---------------------------------------------------------------------------------------------------
// the frame hook

/// FRAME_HOOK: land finished jobs.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    if app.formats.jobs.is_empty() {
        return;
    }
    let (done, pending): (Vec<Job>, Vec<Job>) = std::mem::take(&mut app.formats.jobs).into_iter().partition(|j| j.prog().is_done());
    app.formats.jobs = pending;
    for job in done {
        let err = job.prog().error();
        match job {
            Job::Probe { path, slot, .. } => {
                let all = std::mem::take(&mut *slot.lock().unwrap_or_else(|e| e.into_inner()));
                let text = text_tracks(&all);
                match text.len() {
                    0 if all.is_empty() => app.toast(format!("{} has no subtitles", file_name(&path))),
                    0 => app.toast(format!("{} only has picture subtitles, which can't become text", file_name(&path))),
                    1 => {
                        let t = text.into_iter().next().unwrap_or_else(|| all[0].clone());
                        start_extract(app, &path, t, true);
                    }
                    _ => app.formats.chooser = Some((path, text)),
                }
            }
            Job::Extract { slot, done, ask, label, .. } => {
                let cues = std::mem::take(&mut *slot.lock().unwrap_or_else(|e| e.into_inner()));
                match err {
                    Some(e) => {
                        app.toast(format!("Couldn't read the subtitles of {label}: {e}"));
                        done.finish(Some(e));
                    }
                    None => {
                        let n = cues.len();
                        if ask {
                            timeline_pane::offer_cues(app, cues);
                        } else {
                            app.push_undo();
                            crate::ui::subtitles_ui::apply_import(&mut app.project, &cues, true);
                            app.after_edit();
                        }
                        done.set(1.0, format!("{n} cues"));
                        done.finish(None);
                    }
                }
            }
            Job::Embed { out, done, .. } => match err {
                Some(e) => {
                    app.push_toast(feedback::Toast::new(format!("Subtitle track failed: {e}")).kind(feedback::ToastKind::Error));
                    done.finish(Some(e));
                }
                None => {
                    app.toast_with_folder(format!("Added a subtitle track to {}", file_name(&out)), out.clone());
                    done.finish(None);
                }
            },
            Job::Make { prog, out, bin, what, done, renamed } => match err {
                Some(e) => {
                    app.toast(format!("{what} failed: {e}"));
                    done.finish(Some(e));
                }
                None => {
                    let out = renamed.and_then(|r| r.lock().unwrap_or_else(|e| e.into_inner()).take()).unwrap_or(out);
                    if let Some(note) = prog.status().strip_prefix(NOTE) {
                        app.toast(format!("{} is drawn without its {note}", file_name(&out)));
                    }
                    let ids = app.import_files(std::slice::from_ref(&out));
                    if let (Some(&id), false) = (ids.last(), bin.is_empty()) {
                        app.project.add_folder(&bin);
                        if let Some(a) = app.project.asset_mut(id) {
                            a.folder = bin.clone();
                        }
                    }
                    if let Some(&id) = ids.last() {
                        app.library.selected = Some(id);
                        done.set(1.0, format!("asset {id}"));
                        app.toast_with_folder(format!("{} is in the Library", file_name(&out)), out.clone());
                    }
                    app.fire_hook("import", json!({"asset_ids": ids, "path": out.to_string_lossy()}));
                    done.finish(None);
                }
            },
        }
    }
    if !app.formats.jobs.is_empty() {
        ctx.request_repaint_after(Duration::from_millis(200));
    }
}

// ---------------------------------------------------------------------------------------------------
// tools

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "subtitles.tracks",
        desc: "List the subtitle tracks of a media file (index, label, picture = can't become text), side-car .srt/.vtt/.ass/.ssa files included.",
        args: &["path:string:true:a video file"],
        kind: ToolKind::Read,
        run: |_app, args| {
            let path = PathBuf::from(req(arg_str(args, "path"), "path")?);
            let tracks = crate::player::tracks::probe(&path).subtitles;
            let list: Vec<Value> = tracks
                .iter()
                .enumerate()
                .map(|(i, t)| json!({"index": i, "label": t.label, "picture": t.bitmap, "file": t.file.as_ref().map(|f| f.to_string_lossy())}))
                .collect();
            Ok(ToolOutcome::Done(json!(list)))
        },
    },
    ToolDef {
        name: "subtitles.extract",
        desc: "Read a text subtitle track out of a media file (MKV / MP4 / MOV / WebM streams, or a side-car file) into the project's subtitles, replacing them (one undo step). track = an index from subtitles.tracks (default: the first text track).",
        args: &["path:string:true:a video file", "track:integer:false:index from subtitles.tracks"],
        kind: ToolKind::Job,
        run: |app, args| {
            let path = PathBuf::from(req(arg_str(args, "path"), "path")?);
            if crate::media::ffpipe::ffmpeg_exe().is_none() {
                return Err("ffmpeg.exe not found".into());
            }
            if !path.is_file() {
                return Err(format!("{} not found", path.display()));
            }
            // the ffprobe runs on the job thread, not here
            let pick = arg_u64(args, "track").map_or(Pick::FirstText, |i| Pick::Index(i as usize));
            let done = start_extract_pick(app, &path, pick, file_name(&path), false);
            Ok(ToolOutcome::Job(done, path))
        },
    },
    ToolDef {
        name: "subtitles.embed",
        desc: "Add the project's subtitles to a video file as a soft subtitle track viewers switch on in their player (mov_text in MP4/MOV, SubRip in MKV, WebVTT in WebM). Writes '<name> (subtitled).<ext>' beside it unless out is given; the source file is never changed. The Export window's Subtitle track option does this to every export.",
        args: &["path:string:true:an MP4, MOV, MKV or WebM file", "out:string:false:output path (same container)"],
        kind: ToolKind::Job,
        run: |app, args| {
            let video = PathBuf::from(req(arg_str(args, "path"), "path")?);
            if app.project.subtitles.is_empty() {
                return Err("the project has no subtitles".into());
            }
            formats::soft_sub_codec(&export::ext_of(&video)).ok_or("only MP4, MOV, MKV and WebM files carry a subtitle track")?;
            let out = arg_str(args, "out").map(PathBuf::from).unwrap_or_else(|| formats::subtitled_copy(&video));
            if out == video {
                return Err("out must be a different file from path".into());
            }
            let cues = project_cues(app);
            let done = start_embed(app, &video, &cues, Some(out.clone()));
            Ok(ToolOutcome::Job(done, out))
        },
    },
    ToolDef {
        name: "media.generate",
        desc: "Generate a clip into the Library's Generated bin (the file goes in Generated\\ beside the project): kind = bars (SMPTE colour bars, a still picture) | noise (TV static) | plasma | counter (a timer over a transparent background). Size and fps default to the project's, duration to 5 s (counter 10 s). Counter: down=true counts down from start (default: the duration) to 0; format = clock (M:SS) | precise (M:SS.cc) | seconds | frames; color / background [r,g,b,a] (background alpha 0 = transparent). Replies with the asset id when it is in.",
        args: &[
            "kind:string:true:bars | noise | plasma | counter",
            "duration:number:false:seconds",
            "width:integer:false:",
            "height:integer:false:",
            "fps:number:false:",
            "colour:boolean:false:noise: coloured speckles",
            "speed:number:false:plasma: flow speed (1 = default, 0 = still)",
            "down:boolean:false:counter: count down",
            "start:number:false:counter: value at the first frame, seconds",
            "format:string:false:counter: clock | precise | seconds | frames",
            "color:array:false:counter: text [r,g,b,a]",
            "background:array:false:counter: [r,g,b,a]",
            "seed:integer:false:noise: pattern seed",
        ],
        kind: ToolKind::Job,
        run: |app, args| {
            let kind = req(arg_str(args, "kind"), "kind")?;
            let kind = synth::GenKind::parse(kind).ok_or_else(|| format!("unknown kind '{kind}' (bars, noise, plasma, counter)"))?;
            if crate::media::ffpipe::ffmpeg_exe().is_none() {
                return Err("ffmpeg.exe not found".into());
            }
            let p = &app.project;
            let mut spec = synth::GenSpec::new(kind, p.width, p.height, p.fps);
            if let Some(v) = arg_f64(args, "duration") {
                spec.duration = v;
            }
            if let Some(v) = arg_u64(args, "width") {
                spec.width = v as u32;
            }
            if let Some(v) = arg_u64(args, "height") {
                spec.height = v as u32;
            }
            if let Some(v) = arg_f64(args, "fps") {
                spec.fps = v;
            }
            spec.colour = arg_bool(args, "colour").unwrap_or(false);
            if let Some(v) = arg_f64(args, "speed") {
                spec.speed = v;
            }
            spec.down = arg_bool(args, "down").unwrap_or(false);
            spec.start = arg_f64(args, "start");
            if let Some(f) = arg_str(args, "format") {
                spec.format = synth::CounterFormat::parse(f).ok_or_else(|| format!("unknown format '{f}'"))?;
            }
            if let Some(c) = args.get("color").and_then(color_arg) {
                spec.color = c;
            }
            if let Some(c) = args.get("background").and_then(color_arg) {
                spec.background = c;
            }
            if let Some(v) = arg_u64(args, "seed") {
                spec.seed = v;
            }
            let (done, out) = start_generate(app, spec);
            Ok(ToolOutcome::Job(done, out))
        },
    },
    ToolDef {
        name: "media.import_lottie",
        desc: "Import a Lottie animation (Bodymovin .json or a .lottie package): rendered to a clip in Generated\\ beside the project (transparent where the animation is) and added to the Library; replies with the asset id when it is in. Shapes, solids, precomps, parenting, gradients, trim paths and eased keyframes are drawn; masks, mattes, text / image layers, expressions and effects are not (the reply's toast names what a file uses of them). media.import takes these files too, without waiting.",
        args: &["path:string:true:a .json or .lottie file"],
        kind: ToolKind::Job,
        run: |app, args| {
            let path = PathBuf::from(req(arg_str(args, "path"), "path")?);
            if !path.is_file() {
                return Err(format!("{} not found", path.display()));
            }
            if crate::media::ffpipe::ffmpeg_exe().is_none() {
                return Err("ffmpeg.exe not found".into());
            }
            let (done, out) = start_lottie(app, &path);
            Ok(ToolOutcome::Job(done, out))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn track(index: usize, bitmap: bool) -> Track {
        Track { index, label: format!("T{index}"), file: None, bitmap }
    }

    #[test]
    fn assert_no_idle_repaint_subtitle_tracks() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let tracks = vec![track(0, false), Track { file: Some(PathBuf::from("C:/v/film.ass")), ..track(1, false) }];
        let path = Path::new("C:/v/film.mkv");
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                let (open, pick) = track_chooser(ctx, path, &tracks);
                assert!(open && pick.is_none());
            });
        }
        assert!(!ctx.has_requested_repaint(), "an idle track chooser must not repaint");
    }

    #[test]
    fn only_text_tracks_are_offered() {
        let all = vec![track(0, true), track(1, false), track(2, true), track(3, false)];
        let t = text_tracks(&all);
        assert_eq!(t.iter().map(|t| t.index).collect::<Vec<_>>(), vec![1, 3], "indices stay the stream's own");
        assert_eq!(pick_track(&all, Pick::FirstText).unwrap().index, 1);
        assert_eq!(pick_track(&all, Pick::Index(3)).unwrap().index, 3);
        assert!(pick_track(&all, Pick::Index(2)).unwrap_err().contains("picture"));
        assert!(pick_track(&all, Pick::Index(9)).unwrap_err().contains("no track 9"));
        assert!(pick_track(&[track(0, true)], Pick::FirstText).is_err());
    }

    #[test]
    fn srt_of_numbers_from_one() {
        let s = srt_of(&[(0.5, 1.0, "A".into()), (2.0, 3.0, "B".into())]);
        assert!(s.starts_with("1\n00:00:00,500 --> 00:00:01,000\nA\n\n2\n"), "{s}");
    }

    #[test]
    fn lottie_files_are_recognised_by_content() {
        let dir = std::env::temp_dir().join(format!("se-lottie-sniff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let anim = dir.join("anim.json");
        std::fs::write(&anim, r#"{"v":"5.7","fr":30,"ip":0,"op":60,"w":512,"h":512,"layers":[]}"#).unwrap();
        let other = dir.join("package.json");
        std::fs::write(&other, r#"{"name":"x","version":"1.0.0"}"#).unwrap();
        assert!(is_lottie_file(&anim));
        assert!(!is_lottie_file(&other), "an ordinary JSON file is left alone");
        assert!(is_lottie_file(Path::new("C:/x/missing.lottie")), "a .lottie package by its extension");
        assert!(!is_lottie_file(Path::new("C:/x/a.mp4")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tools_are_registered() {
        for name in ["subtitles.tracks", "subtitles.extract", "subtitles.embed", "media.generate", "media.import_lottie"] {
            assert!(TOOLS.iter().any(|t| t.name == name), "{name}");
        }
    }
}
