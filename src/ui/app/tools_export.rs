//! ---- ws:export-deliver ----
//! Delivery: the render queue drain (`App.export_queue`, one job at a time through the single
//! `App.export` slot), Quick Export (Ctrl+M: re-run `Settings.last_export`, else the first platform
//! tile), Render Selection (`App::request_prerender_range`), the bake pipeline (render the selected
//! clip(s) alone → optional ffmpeg filter → swap them onto the new asset, one undo, original kept) and
//! the 11 `export.*` / `markers.*` / `render.range` MCP tools. `frame_tick` is the FRAME_HOOKS entry,
//! `act` the ACT_HANDLERS one.

use super::feedback::{Toast, ToastKind};
use super::tools_helpers::*;
use super::*;
use crate::engine::convert::{self, BakeFilter};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::settings::ExportPresetRef;
use crate::ui::markers_ui::{self, MarkerFmt};

/// What a bake does to the rendered clip(s) before swapping them in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum BakeKind {
    /// Effects/transform/speed flattened into a plain video asset.
    Render,
    /// + ffmpeg `deshake`.
    Stabilize,
    /// + ffmpeg `afftdn`.
    Denoise,
    /// + `setpts`/`minterpolate` at this speed factor (0.5 = half speed).
    SlowMo(f64),
}

impl BakeKind {
    fn label(self) -> &'static str {
        match self {
            BakeKind::Render => "Render in place",
            BakeKind::Stabilize => "Stabilize",
            BakeKind::Denoise => "Denoise",
            BakeKind::SlowMo(_) => "Slow motion",
        }
    }
    fn suffix(self) -> &'static str {
        match self {
            BakeKind::Render => "baked",
            BakeKind::Stabilize => "stabilized",
            BakeKind::Denoise => "denoised",
            BakeKind::SlowMo(_) => "slowmo",
        }
    }
    fn filter(self, fps: f64) -> Option<BakeFilter> {
        match self {
            BakeKind::Render => None,
            BakeKind::Stabilize => Some(BakeFilter::Stabilize),
            BakeKind::Denoise => Some(BakeFilter::Denoise),
            BakeKind::SlowMo(f) => Some(BakeFilter::SlowMo { factor: f, fps }),
        }
    }
    /// How much longer the baked file runs than the timeline span it came from.
    fn factor(self) -> f64 {
        match self {
            BakeKind::SlowMo(f) => f.clamp(0.05, 1.0),
            _ => 1.0,
        }
    }
}

pub(crate) enum BakeStage {
    Render(Arc<Progress>),
    Filter(Arc<Progress>),
}

/// One in-flight bake. Polled by `frame_tick`; `overall` is what an MCP `export.bake` caller waits on
/// (finished only once the swap has landed, so the reply sees the new asset).
pub(crate) struct BakeJob {
    stage: BakeStage,
    clip_ids: Vec<Id>,
    /// Earliest selected clip start — timeline time of the rendered file's t=0.
    t0: f64,
    tmp: PathBuf,
    out: PathBuf,
    kind: BakeKind,
    label: &'static str,
    overall: Arc<Progress>,
}

impl BakeJob {
    pub(crate) fn progress(&self) -> Arc<Progress> {
        match &self.stage {
            BakeStage::Render(p) | BakeStage::Filter(p) => p.clone(),
        }
    }
    pub(crate) fn title(&self) -> String {
        let stage = match self.stage {
            BakeStage::Render(_) => "rendering",
            BakeStage::Filter(_) => "filtering",
        };
        format!("{} · {stage} · {}", self.label, self.out.file_name().unwrap_or_default().to_string_lossy())
    }
}

/// `[earliest start, latest end)` of the given clips.
pub(crate) fn span(project: &Project, ids: &[Id]) -> Option<(f64, f64)> {
    let clips: Vec<&Clip> = ids.iter().filter_map(|&id| project.clip(id)).collect();
    let a = clips.iter().map(|c| c.start).fold(f64::INFINITY, f64::min);
    let b = clips.iter().map(|c| c.end()).fold(f64::NEG_INFINITY, f64::max);
    (a.is_finite() && b > a).then_some((a, b))
}

/// The project with only `ids` left on it, shifted so the earliest clip starts at 0 — what a bake
/// renders. Every other clip and every emptied track is dropped; transitions between two kept clips
/// survive (`tidy` prunes the rest); subtitles/in-out never bake in.
/// ponytail: a whole-project clone stripped down, not a scoped single-layer compositor mode — correct
/// today, wasteful on huge projects; a scoped render path in Compositor is the upgrade if profiled.
pub(crate) fn isolated_project(base: &Project, ids: &[Id]) -> Option<Project> {
    let (t0, _) = span(base, ids)?;
    let mut p = base.clone();
    for t in &mut p.tracks {
        t.clips.retain(|c| ids.contains(&c.id));
        for c in &mut t.clips {
            c.start -= t0;
        }
    }
    p.tracks.retain(|t| !t.clips.is_empty());
    p.tidy();
    p.in_point = None;
    p.out_point = None;
    p.subtitles.clear();
    p.preview_bg = crate::model::BackgroundMode::Black;
    Some(p)
}

/// Why `ids` can't be baked as a unit, if they can't: an adjustment clip alone renders nothing
/// meaningful, and a node graph pulling another (unselected) clip's layer would silently diverge from
/// the visible composite.
pub(crate) fn bake_refusal(project: &Project, ids: &[Id]) -> Option<String> {
    for &id in ids {
        let Some(c) = project.clip(id) else { return Some(format!("no such clip {id}")) };
        if c.kind == ClipKind::Adjustment {
            return Some("Adjustment clips can't be baked alone — they only affect what is below them".into());
        }
        if let Some(g) = &c.graph {
            if g.nodes.iter().any(|n| matches!(n.kind, NodeKind::Clip(other) if !ids.contains(&other))) {
                return Some(format!("'{}' has a node graph reading another clip — select that clip too", c.name));
            }
        }
    }
    None
}

/// Point `clip_id` at the baked asset: a fresh clip at the same place/length/link/label/markers, with
/// effects, graph, mask, transform, speed and fades flattened away (they're in the pixels now) and
/// `src_in` = where its timeline start falls in the rendered file (scaled by a slow-mo factor).
/// ponytail: a local, container-agnostic field-set — swap to trim-model's `Project::replace_clip`
/// once its link-pair semantics are what a bake needs; one call site.
pub(crate) fn swap_clip_asset(project: &mut Project, clip_id: Id, new_asset: Id, t0: f64, factor: f64) -> bool {
    let Some(c) = project.clip_mut(clip_id) else { return false };
    let kind = if c.kind == ClipKind::Audio { ClipKind::Audio } else { ClipKind::Video };
    let mut fresh = Clip::new(c.id, kind, c.name.clone(), c.start, c.duration);
    fresh.link = c.link;
    fresh.label = c.label;
    fresh.bus = c.bus;
    fresh.markers = std::mem::take(&mut c.markers);
    fresh.audio_role = c.audio_role;
    fresh.asset = new_asset;
    fresh.src_in = ((c.start - t0) / factor.max(1e-6)).max(0.0);
    *c = fresh;
    true
}

/// `<dir>/<stem>.<ext>`, or `<stem>_2.<ext>`, … — never an existing file.
pub(crate) fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut p = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{stem}_{n}.{ext}"));
        n += 1;
    }
    p
}

/// The next queued job, if the export slot is free — the pure half of `frame_tick`'s drain.
pub(crate) fn next_queued<T>(queue: &mut std::collections::VecDeque<T>, slot_free: bool) -> Option<T> {
    if slot_free {
        queue.pop_front()
    } else {
        None
    }
}

/// Does delivery need the frame loop running? Only while something is queued or baking — an idle
/// editor with nothing to do must request no repaint from here.
pub(crate) fn wants_repaint(queued: usize, baking: usize) -> bool {
    queued > 0 || baking > 0
}

/// What Quick Export re-runs: the last export, else the first platform tile.
pub(crate) fn quick_target(settings: &Settings) -> Result<ExportPresetRef, &'static str> {
    if let Some(last) = &settings.last_export {
        return Ok(last.clone());
    }
    settings
        .export_presets
        .first()
        .map(|p| ExportPresetRef::Preset(p.name.clone()))
        .ok_or("No export presets — open Export (Ctrl+E) once, or add tiles in settings.json")
}

/// Build the `ExportChoice` a target describes, plus a short label for the toast. `settings` is
/// cloned on purpose: a tile's crf/loudnorm apply to this export, not to the user's saved settings.
pub(crate) fn quick_choice(
    target: &ExportPresetRef,
    project: &Project,
    settings: &Settings,
    out_path: PathBuf,
) -> Result<(export_ui::ExportChoice, String), String> {
    let mut st =
        export_ui::ExportUi { preset: "project".into(), custom: (project.width, project.height), ..Default::default() };
    let mut s = settings.clone();
    let label = match target {
        ExportPresetRef::Preset(name) => {
            let p = s
                .export_presets
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(name))
                .cloned()
                .ok_or_else(|| format!("no export preset '{name}'"))?;
            export_ui::apply_preset(&mut st, &mut s, &p);
            p.name.clone()
        }
        ExportPresetRef::Custom { ext, width, height } => {
            if *width > 0 && *height > 0 {
                st.preset = "custom".into();
                st.custom = (*width, *height);
            }
            st.ext = ext.clone();
            format!("{ext} · {}", if *width > 0 { format!("{width}×{height}") } else { "project size".into() })
        }
    };
    Ok((export_ui::build_choice(&st, project, &s, out_path), label))
}

/// Default output folder for a Quick Export / bake: beside the project file, else beside the source
/// video, else the download folder.
fn default_out_dir(app: &App) -> PathBuf {
    app.project_path
        .as_ref()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .or_else(|| app.project.source_video.as_ref().and_then(|s| Path::new(s).parent().map(Path::to_path_buf)))
        .unwrap_or_else(|| app.download_dir())
}

/// Quick Export: re-run `target` (default: `quick_target`) to `path` (default: a fresh file beside the
/// project). Starts now, or — with an export already running — joins the queue.
/// Returns the started job, or None when it was queued (or refused with a toast).
pub(crate) fn quick_export(
    app: &mut App,
    target: Option<ExportPresetRef>,
    path: Option<PathBuf>,
) -> Result<Option<(Arc<Progress>, PathBuf)>, String> {
    if app.timeline_is_empty() {
        return Err("Nothing to export — the timeline is empty".into());
    }
    if media::ffpipe::ffmpeg_exe().is_none() {
        return Err("ffmpeg.exe not found".into());
    }
    let target = match target {
        Some(t) => t,
        None => quick_target(&app.settings).map_err(str::to_string)?,
    };
    let project = app.export_project();
    let out = match path {
        Some(p) => p,
        None => {
            let (suffix, ext) = match &target {
                ExportPresetRef::Preset(name) => {
                    let ext = app
                        .settings
                        .export_presets
                        .iter()
                        .find(|p| p.name.eq_ignore_ascii_case(name))
                        .map(|p| p.ext.clone())
                        .unwrap_or_else(|| "mp4".into());
                    (export_ui::slug(name), ext)
                }
                ExportPresetRef::Custom { ext, .. } => ("edit".into(), ext.clone()),
            };
            unique_path(&default_out_dir(app), &format!("{}_{suffix}", project.name), &ext)
        }
    };
    let (choice, label) = quick_choice(&target, &project, &app.settings, out.clone())?;
    if app.export.is_some() || !app.bake_jobs.is_empty() {
        if files::refuses_source(&app.project, &choice.opts.out_path) {
            return Err("That file is a source of this project — use Overwrite Original Video (Ctrl+S) instead".into());
        }
        app.export_queue.push_back(choice);
        app.toast(format!("Quick Export queued ({label}) — {} waiting", app.export_queue.len()));
        return Ok(None);
    }
    let prog = app.start_export_choice(choice).ok_or("Export could not start")?;
    app.toast(format!("Quick Export: {label} → {}", out.file_name().unwrap_or_default().to_string_lossy()));
    Ok(Some((prog, out)))
}

/// Start a bake of `ids` (expanded through links). Stage 1 renders the isolated clips with the same
/// export path everything else uses; `frame_tick` runs stage 2 (filter) and the swap.
pub(crate) fn start_bake(app: &mut App, ids: &[Id], kind: BakeKind) -> Result<(Arc<Progress>, PathBuf), String> {
    if media::ffpipe::ffmpeg_exe().is_none() {
        return Err("ffmpeg.exe not found".into());
    }
    if app.export.is_some() || !app.bake_jobs.is_empty() {
        return Err("An export or bake is already running — try again when it finishes".into());
    }
    let ids = app.project.expand_links(ids);
    if ids.is_empty() {
        return Err("Select a clip first".into());
    }
    // the live project, not `export_project()`: a bake inside an open sequence renders that sequence's
    // own tracks, which is where the selected ids live
    let base = app.project.clone();
    if let Some(why) = bake_refusal(&base, &ids) {
        return Err(why);
    }
    let iso = isolated_project(&base, &ids).ok_or("Nothing to bake")?;
    let (t0, _) = span(&base, &ids).ok_or("Nothing to bake")?;
    let first = base.clip(ids[0]).ok_or("no such clip")?;
    let (dir, stem) = base
        .asset(first.asset)
        .map(|a| Path::new(&a.path))
        .and_then(|p| Some((p.parent()?.to_path_buf(), p.file_stem()?.to_string_lossy().into_owned())))
        .unwrap_or_else(|| (default_out_dir(app), first.name.clone()));
    let out = unique_path(&dir, &format!("{stem}_{}", kind.suffix()), "mp4");
    let tmp = if kind.filter(base.fps).is_some() {
        out.with_file_name(format!(".{}.bake-tmp.mp4", out.file_stem().unwrap_or_default().to_string_lossy()))
    } else {
        out.clone()
    };
    app.player.pause();
    let mut opts = app.export_opts(tmp.clone());
    opts.loudnorm = false; // a bake is a picture/sound replacement, not a delivery master
    let prog = export::start_export(iso, opts, app.text.clone());
    let overall = Progress::new();
    overall.set(0.0, kind.label());
    app.bake_jobs.push(BakeJob {
        stage: BakeStage::Render(prog),
        clip_ids: ids,
        t0,
        tmp,
        out: out.clone(),
        kind,
        label: kind.label(),
        overall: overall.clone(),
    });
    Ok((overall, out))
}

/// A bake stage finished: fail out, start the filter stage, or swap the clips onto the new asset.
fn advance_bake(app: &mut App, mut job: BakeJob) {
    let prog = job.progress();
    if let Some(e) = prog.error() {
        if job.tmp != job.out {
            let _ = std::fs::remove_file(&job.tmp);
        }
        let msg =
            if prog.is_cancelled() { format!("{} cancelled", job.label) } else { format!("{} failed: {e}", job.label) };
        app.push_toast(Toast::new(msg.clone()).kind(ToastKind::Error));
        job.overall.finish(Some(msg));
        return;
    }
    if matches!(job.stage, BakeStage::Render(_)) {
        if let Some(filter) = job.kind.filter(app.project.fps) {
            let s = &app.settings;
            let p = convert::start_bake_filter(
                job.tmp.clone(),
                job.out.clone(),
                filter,
                s.encoder.clone(),
                s.crf,
                s.preset.clone(),
            );
            job.overall.set(0.5, "Filtering…");
            job.stage = BakeStage::Filter(p);
            app.bake_jobs.push(job);
            return;
        }
    }
    if job.tmp != job.out {
        let _ = std::fs::remove_file(&job.tmp);
    }
    match finish_bake(app, &job) {
        Ok(n) => {
            let name = job.out.file_name().unwrap_or_default().to_string_lossy().into_owned();
            app.push_toast(
                Toast::with_folder(
                    format!("{}: {n} clip(s) now play {name} (original kept in the library)", job.label),
                    job.out.clone(),
                )
                .kind(ToastKind::Success)
                .undo(Action::Undo),
            );
            job.overall.set(1.0, "Done");
            job.overall.finish(None);
        }
        Err(e) => {
            app.push_toast(Toast::new(format!("{} failed: {e}", job.label)).kind(ToastKind::Error));
            job.overall.finish(Some(e));
        }
    }
}

/// Import the rendered file as an asset and re-point the clips — one labelled undo step.
fn finish_bake(app: &mut App, job: &BakeJob) -> Result<usize, String> {
    let asset = media::probe(&job.out.to_string_lossy(), app.backend())?;
    let before = app.project.to_json();
    let aid = app.project.add_asset(asset);
    let factor = job.kind.factor();
    let n = job.clip_ids.iter().filter(|&&id| swap_clip_asset(&mut app.project, id, aid, job.t0, factor)).count();
    if n == 0 {
        return Err("the clips are no longer on the timeline".into());
    }
    app.push_undo_labeled(before, job.label);
    app.after_edit();
    Ok(n)
}

/// FRAME_HOOKS: pop the next queued export once the slot is free (re-checking the source-overwrite
/// refusal at pop time, inside `start_export_choice`), and step every bake job.
pub(super) fn frame_tick(app: &mut App, ctx: &egui::Context) {
    let free = app.export.is_none() && app.bake_jobs.is_empty();
    if let Some(choice) = next_queued(&mut app.export_queue, free) {
        app.start_export_choice(choice);
        ctx.request_repaint();
    }
    let mut i = 0;
    while i < app.bake_jobs.len() {
        if app.bake_jobs[i].progress().is_done() {
            let job = app.bake_jobs.remove(i);
            advance_bake(app, job);
        } else {
            i += 1;
        }
    }
    if wants_repaint(app.export_queue.len(), app.bake_jobs.len()) {
        app.animate_until(ctx, Instant::now() + Duration::from_millis(150));
    }
}

/// The current in/out span, else the selection's span.
fn selection_span(app: &App) -> Option<(f64, f64)> {
    match (app.project.in_point, app.project.out_point) {
        (None, None) => span(&app.project, &app.selection),
        (a, b) => {
            let (a, b) = (a.unwrap_or(0.0), b.unwrap_or_else(|| app.project.duration()));
            (b > a).then_some((a, b))
        }
    }
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::QuickExport => {
            if let Err(e) = quick_export(app, None, None) {
                app.toast(e);
            }
            true
        }
        Action::RenderSelection => {
            match selection_span(app) {
                Some((a, b)) => match app.request_prerender_range(a, b) {
                    Ok(()) => app.toast(format!("Pre-rendering {a:.1}–{b:.1} s")),
                    Err(e) => app.toast(e),
                },
                None => app.toast("Set In/Out points or select clips to render"),
            }
            true
        }
        Action::BakeSelection => {
            let ids = app.selection.clone();
            match start_bake(app, &ids, BakeKind::Render) {
                Ok((_, out)) => {
                    app.toast(format!("Rendering in place → {}", out.file_name().unwrap_or_default().to_string_lossy()))
                }
                Err(e) => app.toast(e),
            }
            true
        }
        Action::ExportMarkers => {
            match markers_ui::export_markers_dialog(&app.project, MarkerFmt::Csv) {
                Ok(Some(p)) => app.toast_with_folder("Markers exported", p),
                Ok(None) => {}
                Err(e) => app.toast(format!("Markers export failed: {e}")),
            }
            true
        }
        _ => false,
    }
}

fn bake_tool(app: &mut App, args: &Value, kind: BakeKind) -> Result<ToolOutcome, String> {
    let ids = req(arg_ids(args, "clip_ids"), "clip_ids")?;
    start_bake(app, &ids, kind).map(|(p, out)| ToolOutcome::Job(p, out))
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "export.presets",
        desc: "List platform export presets (name, ext, size, crf, loudnorm).",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| {
            let list: Vec<Value> = app
                .settings
                .export_presets
                .iter()
                .map(|p| json!({"name": p.name, "ext": p.ext, "width": p.width, "height": p.height, "crf": p.crf, "loudnorm": p.loudnorm}))
                .collect();
            Ok(ToolOutcome::Done(json!(list)))
        },
    },
    ToolDef {
        name: "export.quick",
        desc: "Export with the last-used (or given) preset/options; blocks until the file is written (up to 30 min).",
        args: &["preset:string:false:preset name, default last used", "path:string:false:default alongside the project/source"],
        kind: ToolKind::Job,
        run: |app, args| {
            if app.export.is_some() || !app.bake_jobs.is_empty() {
                return Err("an export is already running".into());
            }
            let target = arg_str(args, "preset").map(|n| ExportPresetRef::Preset(n.to_string()));
            let path = arg_str(args, "path").map(PathBuf::from);
            match quick_export(app, target, path)? {
                Some((p, out)) => Ok(ToolOutcome::Job(p, out)),
                None => Err("export was queued instead of started".into()),
            }
        },
    },
    ToolDef {
        name: "export.queue",
        desc: "Append an export job to the render queue; returns immediately with its queue position.",
        args: &[
            "preset:string:false:preset name, default last used",
            "path:string:true:",
            "range_in:number:false:seconds",
            "range_out:number:false:seconds",
        ],
        // Ui, not Mutate (the issue table says Mutate): the queue is App state, not the Project — the
        // Mutate path's unconditional after_edit would dirty a clean project for no project change.
        kind: ToolKind::Ui,
        run: |app, args| {
            let path = PathBuf::from(req(arg_str(args, "path"), "path")?);
            let target = match arg_str(args, "preset") {
                Some(n) => ExportPresetRef::Preset(n.to_string()),
                None => quick_target(&app.settings)?,
            };
            let project = app.export_project();
            let (mut choice, _) = quick_choice(&target, &project, &app.settings, path)?;
            if let (Some(a), Some(b)) = (arg_f64(args, "range_in"), arg_f64(args, "range_out")) {
                if b <= a {
                    return Err("range_out must be after range_in".into());
                }
                choice.opts.range = Some((a, b));
                choice.lossless = false;
            }
            if files::refuses_source(&app.project, &choice.opts.out_path) {
                return Err("that path is a source of this project".into());
            }
            app.export_queue.push_back(choice);
            Ok(ToolOutcome::Done(json!({"ok": true, "position": app.export_queue.len()})))
        },
    },
    ToolDef {
        name: "export.status",
        desc: "Current export progress (fraction, status, ETA seconds) and the queue / bake counts.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| {
            let (running, fraction, status, eta) = match &app.export {
                Some((p, _)) => (true, p.fraction(), p.status(), p.eta().map(|d| d.as_secs_f64())),
                None => (false, 0.0, String::new(), None),
            };
            Ok(ToolOutcome::Done(json!({
                "running": running, "fraction": fraction, "status": status, "eta_secs": eta,
                "queued": app.export_queue.len(), "bakes": app.bake_jobs.len(),
            })))
        },
    },
    ToolDef {
        name: "export.bake",
        desc: "Render the given clip(s) in place (effects flattened into a new asset) and swap them onto it; the original asset stays in the library. Blocks until the swap lands.",
        args: &["clip_ids:array:true:"],
        kind: ToolKind::Job,
        run: |app, args| bake_tool(app, args, BakeKind::Render),
    },
    ToolDef {
        name: "export.stabilize",
        desc: "Bake the clip(s) through ffmpeg deshake, then swap them onto the result.",
        args: &["clip_ids:array:true:"],
        kind: ToolKind::Job,
        run: |app, args| bake_tool(app, args, BakeKind::Stabilize),
    },
    ToolDef {
        name: "export.slowmo",
        desc: "Bake the clip(s) through setpts+minterpolate optical-flow slow motion, then swap them onto the result (the clip keeps its length and now shows the first part of the slowed footage — extend its end to reveal the rest).",
        args: &["clip_ids:array:true:", "factor:number:false:speed, default 0.5 (= half speed)"],
        kind: ToolKind::Job,
        run: |app, args| bake_tool(app, args, BakeKind::SlowMo(arg_f64(args, "factor").unwrap_or(0.5))),
    },
    ToolDef {
        name: "export.denoise",
        desc: "Bake the clip(s) through ffmpeg afftdn spectral audio denoise, then swap them onto the result.",
        args: &["clip_ids:array:true:"],
        kind: ToolKind::Job,
        run: |app, args| bake_tool(app, args, BakeKind::Denoise),
    },
    ToolDef {
        name: "markers.export",
        desc: "Write every marker in timeline order to a CSV or a YouTube-chapters text file.",
        args: &["path:string:true:", "format:string:false:csv | youtube_chapters, default csv"],
        kind: ToolKind::Read,
        run: |app, args| {
            let path = PathBuf::from(req(arg_str(args, "path"), "path")?);
            let fmt = match arg_str(args, "format") {
                None => MarkerFmt::Csv,
                Some(s) => MarkerFmt::parse(s).ok_or("format must be csv | youtube_chapters")?,
            };
            let text = markers_ui::export_markers(&app.project, app.project.fps, fmt);
            std::fs::write(&path, &text).map_err(|e| e.to_string())?;
            let count = app.project.markers_in_timeline().len();
            Ok(ToolOutcome::Done(json!({"ok": true, "path": path.to_string_lossy(), "count": count})))
        },
    },
    ToolDef {
        name: "markers.import",
        desc: "Add project markers from a CSV file (time,name[,note,label] — or markers.export's own header form).",
        args: &["path:string:true:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let path = req(arg_str(args, "path"), "path")?;
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            let added = markers_ui::import_markers_csv(&mut app.project, &text);
            Ok(ToolOutcome::Done(json!({"ok": true, "added": added})))
        },
    },
    ToolDef {
        name: "render.range",
        desc: "Pre-render [a, b) into the movie-mode cache without touching the in/out points.",
        args: &["a:number:true:seconds", "b:number:true:seconds"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let a = req(arg_f64(args, "a"), "a")?;
            let b = req(arg_f64(args, "b"), "b")?;
            app.request_prerender_range(a, b)?;
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Asset;

    fn asset(id: Id, path: &str) -> Asset {
        Asset {
            id,
            path: path.into(),
            kind: ClipKind::Video,
            duration: 10.0,
            width: 320,
            height: 240,
            fps: 30.0,
            audio_streams: Vec::new(),
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
        }
    }

    /// Two video tracks + an audio track; the selection is clips 2 and 3 (V1 at 2–5 s, V2 at 3–4 s).
    fn project() -> Project {
        let mut p = Project::new();
        p.assets.push(asset(7, "C:/x.mp4"));
        p.add_track(TrackKind::Video);
        let mut a = Clip::new(1, ClipKind::Video, "a", 0.0, 2.0);
        a.asset = 7;
        let mut b = Clip::new(2, ClipKind::Video, "b", 2.0, 3.0);
        b.asset = 7;
        b.effects.push(Effect::new(EffectKind::Blur));
        p.tracks[0].clips.extend([a, b]);
        let mut c = Clip::new(3, ClipKind::Video, "c", 3.0, 1.0);
        c.asset = 7;
        p.tracks[2].clips.push(c);
        p.tracks[1].clips.push(Clip::new(4, ClipKind::Audio, "music", 0.0, 8.0));
        p
    }

    #[test]
    fn isolated_project_zero_shifts_selection_only() {
        let base = project();
        assert_eq!(span(&base, &[2, 3]), Some((2.0, 5.0)));
        let iso = isolated_project(&base, &[2, 3]).expect("two clips");
        let ids: Vec<Id> = iso.all_clips().map(|(_, c)| c.id).collect();
        assert_eq!(ids, vec![2, 3], "only the selection survives");
        assert_eq!(iso.tracks.len(), 2, "the emptied audio track is dropped");
        assert_eq!(iso.clip(2).unwrap().start, 0.0, "earliest start becomes 0");
        assert_eq!(iso.clip(3).unwrap().start, 1.0);
        assert!((iso.duration() - 3.0).abs() < 1e-9, "duration = the selection's span");
        assert!(iso.clip(2).unwrap().has_effects(), "effects stay — they're what gets baked");
        assert!(iso.in_point.is_none() && iso.subtitles.is_empty());
        assert!(isolated_project(&base, &[999]).is_none());
        assert!(base.clip(4).is_some(), "the base project is untouched");
    }

    #[test]
    fn swap_clip_asset_flattens_onto_the_new_asset() {
        let mut p = project();
        p.clip_mut(2).unwrap().scale.value = 2.0;
        p.clip_mut(2).unwrap().speed = 2.0;
        p.clip_mut(2).unwrap().markers.push(crate::model::Marker { id: 50, t: 1.0, ..Default::default() });
        let aid = p.add_asset(asset(0, "C:/x_baked.mp4"));
        assert!(swap_clip_asset(&mut p, 2, aid, 2.0, 1.0));
        assert!(swap_clip_asset(&mut p, 3, aid, 2.0, 1.0));
        let b = p.clip(2).unwrap();
        assert_eq!((b.asset, b.src_in, b.start, b.duration), (aid, 0.0, 2.0, 3.0));
        assert!(!b.has_effects() && b.speed == 1.0 && b.scale.value == 1.0, "flattened");
        assert_eq!(b.markers.len(), 1, "clip markers ride along");
        assert_eq!(p.clip(3).unwrap().src_in, 1.0, "V2's clip starts 1 s into the rendered file");
        // half-speed bake: the timeline offset lands twice as far into the (twice as long) file
        assert!(swap_clip_asset(&mut p, 3, aid, 2.0, 0.5));
        assert_eq!(p.clip(3).unwrap().src_in, 2.0);
        assert!(!swap_clip_asset(&mut p, 999, aid, 0.0, 1.0));
        assert!(p.asset(7).is_some(), "the original asset is kept");
    }

    #[test]
    fn bake_refuses_cross_clip_graphs_and_adjustment_clips() {
        let mut p = project();
        assert_eq!(bake_refusal(&p, &[2, 3]), None);
        let mut next = 500u64;
        let mut g = crate::model::NodeGraph::new(&mut || {
            next += 1;
            next
        });
        g.nodes.push(crate::model::Node { id: 999, kind: NodeKind::Clip(1), x: 0.0, y: 0.0, enabled: true });
        p.clip_mut(2).unwrap().graph = Some(g);
        assert!(bake_refusal(&p, &[2]).is_some(), "reads clip 1, which isn't selected");
        assert_eq!(bake_refusal(&p, &[1, 2]), None, "selecting the referenced clip too is fine");
        p.tracks[2].clips.push(Clip::new(9, ClipKind::Adjustment, "adj", 0.0, 1.0));
        assert!(bake_refusal(&p, &[9]).is_some());
        assert!(bake_refusal(&p, &[123]).is_some(), "unknown id");
    }

    /// Two queued jobs pop strictly in order and only while the slot is free; a queued path that
    /// resolves to a project source is refused (the same check `start_export_choice` runs at pop).
    #[test]
    fn queue_drains_in_order_and_refuses_source_overwrite() {
        let mut q: std::collections::VecDeque<&str> = ["first", "second"].into_iter().collect();
        assert_eq!(next_queued(&mut q, false), None, "nothing pops while an export runs");
        assert_eq!(q.len(), 2);
        assert_eq!(next_queued(&mut q, true), Some("first"));
        assert_eq!(next_queued(&mut q, true), Some("second"));
        assert_eq!(next_queued(&mut q, true), None);

        let dir = std::env::temp_dir().join(format!("se-queue-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.mp4");
        std::fs::write(&src, b"x").unwrap();
        let mut p = Project::new();
        p.assets.push(asset(1, &src.to_string_lossy()));
        assert!(files::refuses_source(&p, &src), "writing over a source asset is refused");
        assert!(
            files::refuses_source(&p, &dir.join("..").join(dir.file_name().unwrap()).join("src.mp4")),
            "canonicalised"
        );
        assert!(!files::refuses_source(&p, &dir.join("out.mp4")));
        assert!(
            !files::refuses_source(&p, &dir.join("missing.mp4")),
            "a path that doesn't exist yet can't be a source"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quick_export_falls_back_to_first_preset() {
        let mut s = Settings::default();
        assert_eq!(quick_target(&s), Ok(ExportPresetRef::Preset("YouTube 1080p".into())));
        let custom = ExportPresetRef::Custom { ext: "webm".into(), width: 640, height: 360 };
        s.last_export = Some(custom.clone());
        assert_eq!(quick_target(&s), Ok(custom.clone()));
        s.export_presets.clear();
        s.last_export = None;
        assert!(quick_target(&s).is_err());

        // the fallback tile really drives the options (the toast names it via `label`)
        let s = Settings::default();
        let mut p = Project::new();
        p.width = 1280;
        p.height = 720;
        let (choice, label) = quick_choice(&quick_target(&s).unwrap(), &p, &s, "out.mp4".into()).unwrap();
        assert_eq!(label, "YouTube 1080p");
        assert_eq!(choice.opts.out_size, Some((1920, 1080)));
        assert!(choice.opts.letterbox && choice.opts.loudnorm);
        assert_eq!(choice.opts.crf, 18);
        assert_eq!(choice.last, ExportPresetRef::Preset("YouTube 1080p".into()));
        let (c2, _) = quick_choice(&custom, &p, &s, "out.webm".into()).unwrap();
        assert_eq!(c2.opts.out_size, Some((640, 360)));
        assert!(!c2.opts.letterbox, "a custom size keeps the plain stretch");
        assert!(quick_choice(&ExportPresetRef::Preset("nope".into()), &p, &s, "x.mp4".into()).is_err());
    }

    #[test]
    fn unique_path_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("se-unique-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "a", "mp4"), dir.join("a.mp4"));
        std::fs::write(dir.join("a.mp4"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a", "mp4"), dir.join("a_2.mp4"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bake_kind_filters_and_factors() {
        assert_eq!(BakeKind::Render.filter(30.0), None);
        assert_eq!(BakeKind::Stabilize.filter(30.0), Some(BakeFilter::Stabilize));
        assert_eq!(BakeKind::SlowMo(0.5).filter(25.0), Some(BakeFilter::SlowMo { factor: 0.5, fps: 25.0 }));
        assert_eq!(BakeKind::SlowMo(0.25).factor(), 0.25);
        assert_eq!(BakeKind::Denoise.factor(), 1.0);
    }
}
