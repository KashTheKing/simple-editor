//! ---- ws:forum-export ----
//! The App side of `engine::export_doctor` (plans/shotcut-forum-insights.md, Session B):
//! - a failed export says what went wrong in one sentence, with the one button that fixes it;
//! - Retry Export in Software (the failure toast, a stuck Jobs row, the palette) re-runs the same export
//!   on x264, stopping a running one first;
//! - Find Export Problem bisects the export at test quality on a job thread and selects the clip;
//! - the pre-export check offers Relink… for missing files and Go to Clip for a clip that would hang
//!   the export, next to Export Anyway;
//! - a finished export's toast plays it, and its speed feeds the Export pane's time estimate;
//! - each project file remembers the preset card it last exported with.

use super::*;
use crate::engine::export_doctor::{self as doctor, Culprit, Fix};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::jobs_ui::JobKind;

type Found = Arc<Mutex<Option<Option<Culprit>>>>;

#[derive(Default)]
pub(super) struct ExportFix {
    /// The last failed export: (project, options, where it went), for Retry / Find the Problem.
    failed: Option<(Project, ExportOptions, ExportKind)>,
    /// Retry in software once the running export has stopped: what to re-run.
    retry: Option<(Project, ExportOptions, ExportKind)>,
    /// The running "Find the problem" job and its answer slot.
    find: Option<(Arc<Progress>, Found)>,
    /// The last failure's raw error (`export.diagnose`).
    last_error: Option<String>,
    /// The last finished export (Play on its toast).
    last_done: Option<PathBuf>,
    /// The project file whose remembered preset card the Export pane was last opened on (applied once
    /// per project, so a card picked since then is not undone by reopening the pane).
    tile_for: Option<PathBuf>,
}

fn action_for(fix: Fix) -> Action {
    match fix {
        Fix::RetrySoftware => Action::ExportRetrySoftware,
        Fix::FindProblem => Action::ExportFindProblem,
        Fix::Relink => Action::RelinkMedia,
        Fix::ChangeSettings | Fix::ChooseAnotherFile => Action::ExportVideo,
    }
}

/// `finish_export`'s failure path (not a cancel): the plain sentence and its button.
pub(super) fn failed(app: &mut App, prog: &Arc<Progress>, kind: &ExportKind, err: &str) {
    let d = doctor::classify(err);
    app.export_fix.last_error = Some(err.to_string());
    app.export_fix.failed = doctor::recall(prog).map(|(p, o)| (p, o, kind.clone()));
    // a fix that needs the remembered export falls back to the Export pane when there is none
    let fix = match d.fix {
        Fix::RetrySoftware | Fix::FindProblem if app.export_fix.failed.is_none() => Fix::ChangeSettings,
        f => f,
    };
    let t = feedback::Toast::new(format!("Export failed. {}", d.sentence))
        .kind(feedback::ToastKind::Error)
        .action(fix.label(), action_for(fix));
    app.push_toast(t);
}

/// `finish_export`'s cancel path: true = the cancel was Retry in Software's, so keep the queue and
/// say nothing.
pub(super) fn retrying(app: &App) -> bool {
    app.export_fix.retry.is_some()
}

/// `finish_export`'s success path: learn this PC's export speed and offer Play on the toast.
pub(super) fn succeeded(app: &mut App, prog: &Arc<Progress>, path: &Path, t: feedback::Toast) -> feedback::Toast {
    // ws:smart-render: a smart render copied most frames, so its time isn't this PC's render speed
    let shortcut = prog.shortcut.load(std::sync::atomic::Ordering::SeqCst);
    if let Some((p, o)) = doctor::recall(prog).filter(|_| !shortcut) {
        let (w, h) = o.out_size.unwrap_or((p.width, p.height));
        let fps = o.fps.unwrap_or(p.fps);
        let dur = o.range.map(|(a, b)| b - a).unwrap_or_else(|| p.duration());
        let audio_only = crate::engine::export::AUDIO_EXTS.contains(&crate::engine::export::ext_of(path).as_str());
        if !audio_only {
            let r = &mut app.settings.export_mpx_rate;
            *r = doctor::blend_rate(*r, w, h, fps, dur, prog.elapsed());
        }
    }
    app.export_fix.failed = None;
    app.export_fix.last_done = Some(path.to_path_buf());
    t.action("Play", Action::PlayLastExport)
}

/// ACT_HANDLERS entry.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::ExportRetrySoftware => retry_in_software(app),
        Action::ExportFindProblem => start_find(app),
        Action::PlayLastExport => match app.export_fix.last_done.clone().filter(|p| p.exists()) {
            Some(p) => quick_export::open_in_player(&p),
            None => app.toast("No finished export to play yet"),
        },
        _ => return false,
    }
    true
}

/// Re-run the running (stuck) or last failed export on software x264.
fn retry_in_software(app: &mut App) {
    if let Some((prog, kind)) = &app.export {
        let Some((p, o)) = doctor::recall(prog) else {
            app.toast("This export can't be retried here - cancel it and export again");
            return;
        };
        app.export_fix.retry = Some((p, o, kind.clone()));
        prog.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        return; // `tick` starts it once the slot is free
    }
    match app.export_fix.failed.take() {
        Some(r) => {
            app.export_fix.retry = Some(r);
            start_pending(app);
        }
        None => app.toast("No failed export to retry"),
    }
}

fn software(mut o: ExportOptions, frames: crate::engine::export::FrameSource) -> ExportOptions {
    o.encoder = "libx264".into();
    o.frames = frames;
    o
}

/// Start the pending software retry once the export slot is free.
fn start_pending(app: &mut App) {
    if app.export.is_some() {
        return;
    }
    if let Some((p, o, kind)) = app.export_fix.retry.take() {
        let o = software(o, app.export_frames());
        let prog = crate::engine::export::start_export(p, o, app.text.clone());
        prog.set(0.0, "Retrying in software");
        app.export = Some((prog, kind));
    }
}

/// FRAME_HOOKS entry: start a pending retry once the export slot is free; land Find the Problem.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    start_pending(app);
    if app.export_fix.find.as_ref().is_some_and(|(p, _)| !p.is_done()) {
        ctx.request_repaint_after(Duration::from_millis(250));
    }
    let Some((prog, found)) = &app.export_fix.find else { return };
    if !prog.is_done() {
        return;
    }
    let (prog, found) = (prog.clone(), found.clone());
    app.export_fix.find = None;
    let answer = found.lock().unwrap_or_else(|e| e.into_inner()).take();
    match (prog.error(), answer) {
        (Some(e), _) if e == crate::engine::export::CANCELLED => {}
        (Some(e), _) => app.toast(format!("Find the problem stopped: {}", doctor::classify(&e).sentence)),
        (None, Some(Some(c))) => show_culprit(app, &c),
        (None, _) => {
            let t = feedback::Toast::new(
                "A test export of the whole timeline worked, so the clips are fine. The encoder or the size is the problem.",
            )
            .kind(feedback::ToastKind::Warn)
            .action(Fix::RetrySoftware.label(), Action::ExportRetrySoftware);
            app.push_toast(t);
        }
    }
}

fn clip_name(app: &App, id: crate::model::Id) -> String {
    match app.project.clip(id) {
        Some(c) if !c.name.is_empty() => format!("\"{}\"", c.name),
        _ => "A clip".into(),
    }
}

fn show_culprit(app: &mut App, c: &Culprit) {
    if !c.clips.is_empty() {
        app.selection = c.clips.clone();
    }
    app.seek(c.range.0);
    let at = doctor::clock(c.range.0);
    let msg = match c.clips.first() {
        Some(&id) => format!("Found it: {} at {at} makes the export fail. {}", clip_name(app, id), c.why.sentence),
        None => format!("Found it: the section at {at} makes the export fail. {}", c.why.sentence),
    };
    app.push_toast(feedback::Toast::new(msg).kind(feedback::ToastKind::Warn));
}

/// Find the Problem: a lint hit answers at once; otherwise bisect on a job thread.
fn start_find(app: &mut App) {
    if app.export_fix.find.is_some() {
        app.toast("Already looking for the problem - see the Jobs list");
        return;
    }
    if app.ffmpeg_missing() || app.timeline_is_empty() {
        return;
    }
    let (project, opts) = match &app.export_fix.failed {
        Some((p, o, _)) => (p.clone(), o.clone()),
        None => {
            let ext = app.export_ui.ext.clone();
            let ext = if ext.is_empty() { "mp4".to_string() } else { ext };
            let out = std::env::temp_dir().join(format!("simple-editor-test.{ext}"));
            (app.export_project(), app.export_opts(out))
        }
    };
    if let Some(s) = doctor::lint(&project).into_iter().next() {
        let range = (s.at, s.at);
        let c = Culprit {
            range,
            clips: if s.clip == 0 { Vec::new() } else { vec![s.clip] },
            why: doctor::Diagnosis { problem: doctor::Problem::Unknown, sentence: s.why, fix: Fix::ChangeSettings },
        };
        show_culprit(app, &c);
        return;
    }
    let found: Found = Arc::new(Mutex::new(None));
    let slot = found.clone();
    let text = app.text.clone();
    let prog = crate::engine::export::spawn_job("export-doctor", move |p| {
        let r = doctor::find_problem(&project, &opts, &text, p)?;
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
        Ok(())
    });
    prog.set(0.0, "Testing the export in sections");
    app.export_fix.find = Some((prog, found));
}

/// The Jobs list's row for a running Find the Problem.
pub(super) fn progress_jobs(app: &App) -> Option<jobs_pane::ProgJob> {
    let (p, _) = app.export_fix.find.as_ref()?;
    Some((JobKind::Media, "Find the problem".to_string(), p.clone()))
}

/// `act_export`'s check before the Export pane opens: missing files (Relink… / Export Anyway), then a
/// clip that would hang or balloon the export (Go to Clip / Export Anyway). True = nothing to ask.
pub(super) fn preflight(app: &mut App, offline: &[String]) -> bool {
    if !offline.is_empty() {
        let list = offline.iter().take(5).map(|p| format!("  {p}")).collect::<Vec<_>>().join("\n");
        let more = if offline.len() > 5 { format!("\n  … and {} more", offline.len() - 5) } else { String::new() };
        confirm::ask_choice(
            "Missing media",
            format!(
                "{} file(s) used on the timeline can't be found. They export as black and silence:\n{list}{more}",
                offline.len()
            ),
            ("Relink…", "Export Anyway"),
            |app| app.act(Action::RelinkMedia),
            open_pane,
        );
        return false;
    }
    let suspects = doctor::lint(&app.export_project());
    let Some(first) = suspects.first().cloned() else { return true };
    let more = if suspects.len() > 1 { format!("\n… and {} more.", suspects.len() - 1) } else { String::new() };
    confirm::ask_choice(
        "Check this clip",
        format!("{}\nIt can make the export very long or stop it.{more}", first.why),
        ("Go to Clip", "Export Anyway"),
        move |app| {
            if first.clip != 0 {
                app.selection = vec![first.clip];
            }
            app.seek(first.at);
        },
        open_pane,
    );
    false
}

/// Open the Export pane, on the preset card this project last exported with.
pub(super) fn open_pane(app: &mut App) {
    app.export_ui.open = true;
    let Some(path) = app.project_path.clone() else { return };
    if app.export_fix.tile_for.as_ref() == Some(&path) {
        return;
    }
    app.export_fix.tile_for = Some(path.clone());
    let Some(name) = app.settings.export_tile_by_project.get(&*path.to_string_lossy()).cloned() else { return };
    if app.export_ui.tile.as_deref() == Some(name.as_str()) {
        return;
    }
    if let Some(p) = app.settings.all_export_presets().into_iter().find(|p| p.name == name) {
        export_ui::apply_preset(&mut app.export_ui, &mut app.settings, &p);
    }
}

/// `start_export_choice`: remember the preset card for this project file.
pub(super) fn remember_tile(app: &mut App, last: &crate::settings::ExportPresetRef) {
    let (Some(path), crate::settings::ExportPresetRef::Preset(name)) = (&app.project_path, last) else { return };
    let map = &mut app.settings.export_tile_by_project;
    map.insert(path.to_string_lossy().into_owned(), name.clone());
    // keep the newest-touched few dozen (a BTreeMap has no order of use; drop arbitrary extras)
    while map.len() > 64 {
        let k = map.keys().next().cloned().unwrap_or_default();
        map.remove(&k);
    }
}

/// The Jobs row for a running export that has stopped moving: says so and offers the software retry.
pub(super) fn stall_note(idle: Duration) -> String {
    format!("No progress for {} · still working", crate::ui::mmss(idle.as_secs_f64()))
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "export.diagnose",
        desc: "Why an export would fail or did fail: clips that hang or balloon it (lint: clip id, time, sentence), missing files, and the last failure as a plain sentence with its fix.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| {
            let lint: Vec<Value> = doctor::lint(&app.export_project())
                .into_iter()
                .map(|s| json!({"clip": s.clip, "at": s.at, "why": s.why}))
                .collect();
            let last = app.export_fix.last_error.as_deref().map(|e| {
                let d = doctor::classify(e);
                json!({"sentence": d.sentence, "fix": d.fix.label(), "error": e})
            });
            let missing = files::offline_assets(&app.project);
            Ok(ToolOutcome::Done(json!({"lint": lint, "missing": missing, "last_failure": last})))
        },
    },
    ToolDef {
        name: "export.find_problem",
        desc: "Find the clip that makes the export fail: tests the timeline in halves at low quality (a background job in the Jobs list), then selects the clip and moves the playhead to it. Uses the last failed export's settings, else the current ones.",
        args: &[],
        kind: ToolKind::Ui,
        run: |app, _| {
            start_find(app);
            Ok(ToolOutcome::Done(json!({"started": app.export_fix.find.is_some()})))
        },
    },
    ToolDef {
        name: "export.retry_software",
        desc: "Retry the running (stuck) or last failed export with the software x264 encoder.",
        args: &[],
        kind: ToolKind::Ui,
        run: |app, _| {
            retry_in_software(app);
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fix_has_a_working_button() {
        // each fix maps to an Action the palette and the toast can run
        for f in [Fix::RetrySoftware, Fix::FindProblem, Fix::Relink, Fix::ChangeSettings, Fix::ChooseAnotherFile] {
            let a = action_for(f);
            assert_eq!(Action::from_id(a.id()), Some(a));
            assert!(!f.label().is_empty() && !f.label().contains(" - "), "plain label: {}", f.label());
        }
    }

    #[test]
    fn the_export_pane_shows_time_and_the_social_note() {
        use crate::ui::export_ui::{estimated_time, social_note};
        assert_eq!(estimated_time(1920, 1080, 30.0, 60.0, 100.0), "~37 s to export");
        assert_eq!(estimated_time(1920, 1080, 30.0, 600.0, 100.0), "~6:00 to export");
        assert!(social_note("Shorts / Reels / TikTok").is_some());
        assert!(social_note("YouTube 1080p").is_none());
    }

    #[test]
    fn the_stall_note_is_short() {
        let s = stall_note(Duration::from_secs(75));
        assert!(s.starts_with("No progress for") && s.len() < 50, "{s}");
    }

    #[test]
    fn finish_export_routes_failures_and_successes_through_export_fix() {
        let src = include_str!("files.rs");
        assert!(src.contains("export_fix::failed(self"), "a failure gets the plain sentence and its button");
        assert!(src.contains("export_fix::succeeded(self"), "a success offers Play and learns the speed");
        assert!(src.contains("export_fix::retrying(self)"), "a retry's cancel keeps the queue");
        assert!(src.contains("export_fix::preflight(self"), "the pre-export check offers Relink and Go to Clip");
    }
}
