//! ---- ws:registries-schema-hooks ----
//! `AssetStatus` / `App::asset_status`: a no-op stub (always `Ready`) that ws:media-library (wave 2)
//! fills with real decode/proxy tracking. reconcile: this `ProxyBuilding(u8)` payload and the
//! `App::asset_status(&self, id)` method form (not a free fn) are the canonical shape other
//! workstreams (media-library) must converge onto, not redefine.
//!
//! ---- ws:media-library ----
//! Filled in (wave 2): `asset_status` reads one offline set (`LibraryState.offline`, rescanned every
//! 2 s by `tick`, the same cadence as the proxy scan) plus the live probe / proxy-build state, so the
//! library badge, the preview slate and the `media.status` tool all agree. `tick` is the FRAME_HOOK
//! that also polls this workstream's two background jobs (image-sequence bake, consolidate copy),
//! applies their results on the UI thread under ONE labelled undo each, fires the `import` Luau hook
//! exactly once per finished bake, and routes the library's keyboard navigation (it runs before
//! `Hotkeys::poll`, so Up/Down/Enter/Space/Delete over a hovered library never reach the timeline).
//! `act` is the ACT_HANDLERS entry for Relink / Consolidate / New Subclip; `windows` shows job progress.

use super::*;
use crate::engine::import::{detect_sequence, relink_by_duration};
use crate::media::proxy::{self, ProxyStatus};
use crate::model::Asset;
use std::collections::HashSet;

/// Per-asset media status for the library / inspector / preview badge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AssetStatus {
    Ready,
    Decoding,
    Offline,
    /// Percent complete (0..=100).
    ProxyBuilding(u8),
}

impl AssetStatus {
    pub(crate) fn name(self) -> &'static str {
        match self {
            AssetStatus::Ready => "Ready",
            AssetStatus::Decoding => "Decoding",
            AssetStatus::Offline => "Offline",
            AssetStatus::ProxyBuilding(_) => "ProxyBuilding",
        }
    }
}

impl App {
    /// The single source of truth for "can this asset be used right now": Offline (file missing, per
    /// the last 2 s rescan) > Decoding (import probe still out) > ProxyBuilding(pct) > Ready. An
    /// unknown id is `Offline` — there is nothing to decode.
    pub(crate) fn asset_status(&self, asset: Id) -> AssetStatus {
        match self.project.asset(asset) {
            Some(a) => status_of(&self.library.offline, a, self.settings.use_proxies, self.settings.proxy_height),
            None => AssetStatus::Offline,
        }
    }

    /// New Subclip from Marks for every id: ONE labelled undo snapshot pushed BEFORE any row is added
    /// (Ctrl+Z after this restores the pre-subclip project exactly), then `Project::add_subclip` per
    /// id over the timeline In/Out marks clamped to the asset's own length (default 0..duration).
    /// ponytail: In/Out are timeline marks, not source marks — source-monitor's src_in/src_out
    /// supersede them once that pane lands.
    pub(crate) fn new_subclips(&mut self, ids: &[Id]) -> Vec<Id> {
        let before = self.project.to_json();
        self.push_undo_labeled(before, "New subclip");
        let (in_t, out_t) = (self.project.in_point, self.project.out_point);
        let mut made = Vec::new();
        for &id in ids {
            let Some(a) = self.project.asset(id).cloned() else { continue };
            let dur = if a.duration > 0.0 { a.duration } else { crate::model::MIN_CLIP };
            let s = in_t.unwrap_or(0.0).clamp(0.0, dur);
            let e = out_t.unwrap_or(dur).clamp(0.0, dur);
            let name = format!("{} [{}–{}]", a.name(), crate::ui::duration_text(s), crate::ui::duration_text(e));
            if let Some(n) = self.project.add_subclip(id, s, e, Some(name)) {
                made.push(n);
            }
        }
        if made.is_empty() {
            self.undo.pop(); // nothing changed: no phantom history row
            self.toast("No subclip made — set In/Out inside the asset's length first");
        } else {
            self.after_edit();
            self.library.selected = made.last().copied();
            let s = if made.len() == 1 { "" } else { "s" };
            self.toast_undo(format!("Created {} subclip{s}", made.len()), Action::Undo);
        }
        made
    }
}

/// The pure half of `App::asset_status` (no live `App` needed to test it).
pub(crate) fn status_of(offline: &HashSet<Id>, a: &Asset, use_proxies: bool, proxy_height: u32) -> AssetStatus {
    if offline.contains(&a.id) {
        return AssetStatus::Offline;
    }
    if crate::engine::import::is_probing(&a.path) {
        return AssetStatus::Decoding;
    }
    match proxy::status(a, use_proxies, proxy_height) {
        ProxyStatus::Building(f) => AssetStatus::ProxyBuilding((f * 100.0).round().clamp(0.0, 100.0) as u8),
        _ => AssetStatus::Ready,
    }
}

/// Every asset whose file is not on disk right now.
/// ponytail: one `exists()` per asset on the UI thread every 2 s — the proxy scan already pays the
/// same per-asset stat; a worker + channel is the upgrade if a network drive ever makes it hitch.
pub(crate) fn offline_set(project: &Project) -> HashSet<Id> {
    project.assets.iter().filter(|a| !a.path.is_empty() && !Path::new(&a.path).exists()).map(|a| a.id).collect()
}

fn refresh_offline(app: &mut App) {
    let now = offline_set(&app.project);
    if now != app.library.offline {
        app.library.offline = now;
    }
}

/// A background job this workstream polls in `tick`.
pub(super) enum MediaJob {
    /// An image-sequence bake: `out` is imported when it lands, then `import` fires once.
    Sequence { prog: Arc<Progress>, out: PathBuf, frames: usize },
    /// Consolidate's copy phase (file I/O only); `tick` repoints + pushes one undo when it finishes.
    Consolidate { prog: Arc<Progress>, results: Arc<Mutex<Vec<(Id, PathBuf, Result<(), String>)>>> },
}

impl MediaJob {
    fn progress(&self) -> &Arc<Progress> {
        match self {
            MediaJob::Sequence { prog, .. } | MediaJob::Consolidate { prog, .. } => prog,
        }
    }
    fn label(&self) -> String {
        match self {
            MediaJob::Sequence { out, .. } => out.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            MediaJob::Consolidate { .. } => "Consolidate media".into(),
        }
    }
}

/// Drain the finished jobs out of `jobs` (each is returned exactly once — the caller applies it, so
/// a completion can never be applied, or its hook fired, twice).
pub(super) fn take_done(jobs: &mut Vec<MediaJob>) -> Vec<MediaJob> {
    let (done, pending): (Vec<_>, Vec<_>) = std::mem::take(jobs).into_iter().partition(|j| j.progress().is_done());
    *jobs = pending;
    done
}

/// FRAME_HOOK.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    if app.offline_scan_at.is_none_or(|t| t <= Instant::now()) {
        app.offline_scan_at = Some(Instant::now() + Duration::from_secs(2));
        refresh_offline(app);
    }
    // keyboard navigation over the hovered library — before Hotkeys::poll consumes the same keys
    if app.library.hovered && !ctx.wants_keyboard_input() {
        let nav = library::keyboard(&mut app.library, ctx);
        if !nav.add_to_timeline.is_empty() {
            app.push_undo();
            app.insert_at(nav.add_to_timeline, app.playhead, None);
            app.after_edit();
        }
        if !nav.remove.is_empty() {
            let before = app.project.to_json();
            let n = nav.remove.len();
            for id in nav.remove {
                app.project.remove_asset(id);
            }
            app.push_undo_labeled(before, "Remove assets");
            app.after_edit();
            let s = if n == 1 { "" } else { "s" };
            app.toast_undo(format!("Removed {n} asset{s}"), Action::Undo);
        }
    }
    for job in take_done(&mut app.media_jobs) {
        match job {
            MediaJob::Sequence { prog, out, frames } => finish_sequence(app, &prog, out, frames),
            MediaJob::Consolidate { prog, results } => {
                let results = std::mem::take(&mut *results.lock().unwrap_or_else(|e| e.into_inner()));
                finish_consolidate(app, &prog, &results);
            }
        }
    }
    if !app.media_jobs.is_empty() {
        ctx.request_repaint_after(Duration::from_millis(200));
    }
}

fn finish_sequence(app: &mut App, prog: &Progress, out: PathBuf, frames: usize) {
    if let Some(e) = prog.error() {
        app.toast(format!("Sequence import failed: {e}"));
        return;
    }
    let ids = app.import_files(std::slice::from_ref(&out));
    let Some(&id) = ids.last() else { return };
    app.library.tab = 0;
    app.library.selected = Some(id);
    app.toast_with_folder(format!("Imported {frames}-frame sequence"), out.clone());
    // exactly once per finished bake: `take_done` hands each job out a single time
    app.fire_hook("import", json!({"asset_id": id, "path": out.to_string_lossy(), "frames": frames}));
}

fn finish_consolidate(app: &mut App, prog: &Progress, results: &[(Id, PathBuf, Result<(), String>)]) {
    if let Some(e) = prog.error() {
        app.toast(format!("Consolidate failed: {e}"));
        return;
    }
    let before = app.project.to_json();
    let n = app.project.apply_consolidate(results);
    let failed = results.iter().filter(|(_, _, r)| r.is_err()).count();
    if n > 0 {
        app.push_undo_labeled(before, "Consolidate media");
        app.after_edit();
        refresh_offline(app);
    }
    let s = if n == 1 { "" } else { "s" };
    let tail = if failed > 0 { format!(" ({failed} could not be copied)") } else { String::new() };
    if n > 0 {
        app.toast_undo(format!("Consolidated {n} file{s}{tail}"), Action::Undo);
    } else {
        app.toast(format!("Nothing consolidated{tail}"));
    }
}

/// WINDOW_DRAWER: progress + Cancel for the jobs above (same window every other job uses).
pub(super) fn windows(app: &mut App, ctx: &egui::Context) {
    let jobs: Vec<(Arc<Progress>, String)> = app.media_jobs.iter().map(|j| (j.progress().clone(), j.label())).collect();
    job_window(ctx, "Media", &jobs);
}

/// ACT_HANDLERS entry.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::RelinkMedia => {
            let sel: Vec<Id> =
                app.library.sel_ids.iter().copied().filter(|id| app.library.offline.contains(id)).collect();
            let ids: Vec<Id> = if sel.is_empty() { app.library.offline.iter().copied().collect() } else { sel };
            if ids.is_empty() {
                app.toast("No offline media — every file was found");
            } else if let Some(dir) = rfd::FileDialog::new().set_title("Relink media: pick the folder").pick_folder() {
                start_relink(app, &ids, &dir);
            }
            true
        }
        Action::ConsolidateMedia => {
            ask_consolidate(app);
            true
        }
        Action::NewSubclip => {
            let ids: Vec<Id> = if app.library.sel_ids.is_empty() {
                app.library.selected.into_iter().collect()
            } else {
                app.library.sel_ids.clone()
            };
            if ids.is_empty() {
                app.toast("Select a library asset first");
            } else {
                app.new_subclips(&ids);
            }
            true
        }
        _ => false,
    }
}

// ---------- relink ----------

/// Repoint `ids` (and every asset sharing their path — subclips) to files found under `dir`, by name
/// then by duration within one frame. Pure over the project; returns (relinked, still missing).
pub(crate) fn relink_assets(project: &mut Project, ids: &[Id], dir: &Path) -> (Vec<Id>, Vec<Id>) {
    let (mut ok, mut missing) = (Vec::new(), Vec::new());
    for &id in ids {
        let Some(a) = project.asset(id).cloned() else { continue };
        if Path::new(&a.path).is_file() {
            ok.push(id);
            continue;
        }
        match relink_by_duration(dir, &a.name(), a.duration, a.fps) {
            Some(p) => {
                let new = p.to_string_lossy().into_owned();
                for b in project.assets.iter_mut().filter(|b| b.path == a.path) {
                    b.path = new.clone();
                }
                ok.push(id);
            }
            None => missing.push(id),
        }
    }
    (ok, missing)
}

/// UI path of Relink…: one labelled undo pushed before the repoint, a toast, and the offline set
/// refreshed at once so the badges clear in the same frame.
pub(super) fn start_relink(app: &mut App, ids: &[Id], dir: &Path) -> (Vec<Id>, Vec<Id>) {
    let before = app.project.to_json();
    let (ok, missing) = relink_assets(&mut app.project, ids, dir);
    if before != app.project.to_json() {
        app.push_undo_labeled(before, "Relink media");
        app.after_edit();
        refresh_offline(app);
    }
    let s = if ok.len() == 1 { "" } else { "s" };
    match missing.len() {
        0 => app.toast(format!("Relinked {} file{s}", ok.len())),
        m => app.toast(format!("Relinked {} file{s}; {m} still missing", ok.len())),
    }
    (ok, missing)
}

// ---------- consolidate ----------

/// The project's own folder — Consolidate copies into it, so an unsaved project has nowhere to go.
pub(super) fn project_dir(app: &App) -> Result<PathBuf, String> {
    app.project_path
        .as_ref()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .ok_or_else(|| "Save the project first — media is consolidated into its folder".to_string())
}

/// `(id, path)` of every asset on disk outside `dir`, one entry per distinct path (subclips share
/// their parent's file; `apply_consolidate` repoints all of them from that one copy).
pub(crate) fn consolidate_list(project: &Project, dir: &Path) -> Vec<(Id, String)> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for a in &project.assets {
        let p = Path::new(&a.path);
        if a.path.is_empty() || Project::path_is_under(p, dir) || !p.is_file() {
            continue;
        }
        let key = a.path.to_ascii_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push((a.id, a.path.clone()));
    }
    out
}

/// Spawn the copy phase; `tick` applies the result. Returns how many files are being copied.
pub(super) fn start_consolidate(app: &mut App) -> Result<usize, String> {
    let dir = project_dir(app)?;
    let list = consolidate_list(&app.project, &dir);
    if list.is_empty() {
        return Err("Every asset already lives in the project folder".into());
    }
    let n = list.len();
    let results = Arc::new(Mutex::new(Vec::new()));
    let sink = results.clone();
    let prog = export::spawn_job("consolidate", move |p| {
        let mut done = Vec::with_capacity(list.len());
        for (i, item) in list.iter().enumerate() {
            if p.is_cancelled() {
                break; // whatever copied so far is still applied (each copy is whole: fs::copy)
            }
            p.set(
                i as f32 / n as f32,
                format!("Copying {}…", Path::new(&item.1).file_name().unwrap_or_default().to_string_lossy()),
            );
            done.extend(Project::consolidate_assets_copy(&dir, std::slice::from_ref(item)));
        }
        *sink.lock().unwrap_or_else(|e| e.into_inner()) = done;
        Ok(())
    });
    app.media_jobs.push(MediaJob::Consolidate { prog, results });
    app.toast(format!("Consolidating {n} file{}…", if n == 1 { "" } else { "s" }));
    Ok(n)
}

/// Consolidate Media…: a non-blocking confirm (forgiveness) naming the count and folder, then the job.
pub(super) fn ask_consolidate(app: &mut App) {
    let dir = match project_dir(app) {
        Ok(d) => d,
        Err(e) => return app.toast(e),
    };
    let n = consolidate_list(&app.project, &dir).len();
    if n == 0 {
        return app.toast("Every asset already lives in the project folder");
    }
    let s = if n == 1 { "" } else { "s" };
    confirm::ask_app(
        "Consolidate media",
        format!(
            "Copy {n} file{s} into {}?\nOriginals stay where they are; the project then uses the copies (one Undo step).",
            dir.display()
        ),
        |app| {
            if let Err(e) = start_consolidate(app) {
                app.toast(e);
            }
        },
    );
}

// ---------- image sequences ----------

/// `<dir>/<prefix>.mp4` for a run like `shot_*.png` → `shot.mp4`, uniquified so a bake never writes
/// over a file the project may already be using.
pub(crate) fn sequence_output(seq: &crate::engine::import::ImageSequence) -> PathBuf {
    let prefix = seq.pattern.split('*').next().unwrap_or("").trim_end_matches(['_', '-', '.', ' ']);
    let stem = if prefix.is_empty() { "sequence" } else { prefix };
    let mut out = seq.dir.join(format!("{stem}.mp4"));
    let mut n = 2;
    while out.exists() {
        out = seq.dir.join(format!("{stem}_{n}.mp4"));
        n += 1;
    }
    out
}

/// Bake the numbered run `first_frame` belongs to into one mp4 in the background; `tick` imports it
/// (and fires `import`) when it lands. Returns the job + output for the `media.import_sequence` tool.
pub(super) fn start_import_sequence(
    app: &mut App,
    first_frame: &Path,
    fps: Option<f64>,
) -> Result<(Arc<Progress>, PathBuf), String> {
    let seq = detect_sequence(first_frame)
        .ok_or_else(|| format!("{} is not part of a numbered still sequence (3+ frames)", first_frame.display()))?;
    if media::ffpipe::ffmpeg_exe().is_none() {
        return Err("ffmpeg.exe not found".into());
    }
    let out = sequence_output(&seq);
    let fps = fps.filter(|f| *f > 0.0).unwrap_or(app.project.fps);
    let frames = seq.frames.len();
    let bake_out = out.clone();
    let prog = export::spawn_job("sequence", move |p| media::ffpipe::bake_sequence(&seq, fps, &bake_out, p));
    app.media_jobs.push(MediaJob::Sequence { prog: prog.clone(), out: out.clone(), frames });
    app.toast(format!("Baking {frames}-frame sequence…"));
    Ok((prog, out))
}

/// The import funnel's sequence gate: any still that belongs to a numbered run starts ONE bake for
/// that run (however many of its frames were dropped) and is dropped from the returned list; every
/// other path passes through untouched.
pub(super) fn intercept_sequences(app: &mut App, paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut started: Vec<(PathBuf, String)> = Vec::new();
    let mut rest = Vec::with_capacity(paths.len());
    for p in paths {
        let Some(seq) = detect_sequence(p) else {
            rest.push(p.clone());
            continue;
        };
        let key = (seq.dir.clone(), seq.pattern.clone());
        if started.contains(&key) {
            continue;
        }
        match start_import_sequence(app, p, None) {
            Ok(_) => started.push(key),
            Err(_) => rest.push(p.clone()), // no ffmpeg: import the still as a still
        }
    }
    rest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ClipKind;

    fn asset(id: Id, path: &str) -> Asset {
        Asset {
            id,
            path: path.into(),
            kind: ClipKind::Video,
            duration: 4.0,
            fps: 30.0,
            ..crate::engine::import::placeholder(path)
        }
    }

    /// `asset_status` is a live-`App` method (no headless `App` exists — see tools_registry_tests.rs),
    /// so this pins the pure body it delegates to: the offline set decides Offline, and once the path
    /// exists again the same asset reads Ready.
    #[test]
    fn asset_status_reflects_offline_set() {
        let dir = std::env::temp_dir().join(format!("se-status-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.mp4");
        let mut p = Project::new();
        let id = p.add_asset(asset(0, &path.to_string_lossy()));
        // one "tick": the file is not there
        let off = offline_set(&p);
        assert!(off.contains(&id));
        assert_eq!(status_of(&off, p.asset(id).unwrap(), true, 720), AssetStatus::Offline);
        // the file comes back: the next tick clears it and the asset is Ready again
        std::fs::write(&path, b"x").unwrap();
        let off = offline_set(&p);
        assert!(off.is_empty());
        assert_eq!(status_of(&off, p.asset(id).unwrap(), true, 720), AssetStatus::Ready);
        assert_eq!(AssetStatus::ProxyBuilding(40).name(), "ProxyBuilding");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The `import` hook fires once per finished bake: `take_done` hands a finished job out exactly
    /// once (a second poll returns nothing), and `tick`'s source has exactly one call site firing the
    /// `import` hook, inside the sequence arm.
    #[test]
    fn sequence_import_completion_fires_import_hook_once() {
        let prog = Progress::new();
        prog.finish(None);
        let mut jobs = vec![
            MediaJob::Sequence { prog: prog.clone(), out: PathBuf::from("C:/seq.mp4"), frames: 3 },
            MediaJob::Consolidate { prog: Progress::new(), results: Arc::new(Mutex::new(Vec::new())) },
        ];
        let done = take_done(&mut jobs);
        assert_eq!(done.len(), 1, "only the finished job");
        assert!(matches!(done[0], MediaJob::Sequence { frames: 3, .. }));
        assert_eq!(jobs.len(), 1, "the pending one stays queued");
        assert!(take_done(&mut jobs).is_empty(), "a job is never handed out twice");
        let src = include_str!("media_sync.rs");
        let body = &src[src.find("fn finish_sequence").unwrap()..src.find("fn finish_consolidate").unwrap()];
        assert_eq!(body.matches("fire_hook(\"import\"").count(), 1);
        assert_eq!(src.matches("fire_hook(\"import\"").count(), 1, "no second import call site in this file");
        assert!(
            body.find("import_files").unwrap() < body.find("fire_hook").unwrap(),
            "asset added before the hook sees its id"
        );
    }

    #[test]
    fn relink_assets_repoints_subclips_with_their_parent() {
        let dir = std::env::temp_dir().join(format!("se-relink-assets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.mp4"), b"x").unwrap();
        let mut p = Project::new();
        let a = p.add_asset(asset(0, "Z:/gone/a.mp4"));
        let sub = p.add_subclip(a, 0.0, 1.0, None).unwrap();
        let b = p.add_asset(asset(0, "Z:/gone/b.mp4"));
        let (ok, missing) = relink_assets(&mut p, &[a, b], &dir);
        assert_eq!((ok, missing), (vec![a], vec![b]));
        let want = dir.join("a.mp4").to_string_lossy().into_owned();
        assert_eq!(p.asset(a).unwrap().path, want);
        assert_eq!(p.asset(sub).unwrap().path, want, "the subclip shares the relinked file");
        assert_eq!(p.asset(b).unwrap().path, "Z:/gone/b.mp4");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn consolidate_list_skips_inside_missing_and_duplicate_paths() {
        let dir = std::env::temp_dir().join(format!("se-consolidate-list-{}", std::process::id()));
        let outside = dir.join("out");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(dir.join("in.mp4"), b"x").unwrap();
        std::fs::write(outside.join("far.mp4"), b"x").unwrap();
        let mut p = Project::new();
        p.add_asset(asset(0, &dir.join("in.mp4").to_string_lossy()));
        let far = p.add_asset(asset(0, &outside.join("far.mp4").to_string_lossy()));
        p.add_subclip(far, 0.0, 1.0, None).unwrap(); // same path: listed once
        p.add_asset(asset(0, "Z:/gone.mp4")); // offline: nothing to copy
        let list = consolidate_list(&p, &dir);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, far);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sequence_output_names_after_the_prefix_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("se-seq-out-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let seq = crate::engine::import::ImageSequence {
            dir: dir.clone(),
            pattern: "shot_*.png".into(),
            ext: "png".into(),
            frames: Vec::new(),
        };
        assert_eq!(sequence_output(&seq), dir.join("shot.mp4"));
        std::fs::write(dir.join("shot.mp4"), b"x").unwrap();
        assert_eq!(sequence_output(&seq), dir.join("shot_2.mp4"));
        let bare = crate::engine::import::ImageSequence { pattern: "*.png".into(), ..seq };
        assert_eq!(sequence_output(&bare), dir.join("sequence.mp4"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
