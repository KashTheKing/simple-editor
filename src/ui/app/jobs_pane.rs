//! ---- ws:jobs-panel ----
//! `Pane::Jobs`'s App side: an AGGREGATOR over the job holders `App` already has (export slot, render
//! queue, bakes, converts, media/MCP jobs, proxy build, downloads, whisper/tracking/TTS, recordings,
//! the waveform/thumbnail/pre-render caches, ffprobe) — no new registry, no change to any job-start
//! site. `tick` (FRAME_HOOKS) takes one snapshot per frame into `App.jobs`; `draw` (PANE_DRAWERS),
//! `indicator` (menu bar) and the `jobs.*` tools read that snapshot; `apply` honours the widget's
//! response. Worker-shared mutexes are read with `try_lock` only (a busy worker shows "…").
//!
//! deviation (see PR body): tests that the plan describes against a live `App` run against the pure
//! halves instead (`rows_from(&Holders)`, `cancel_core`, `reorder_queue`) — this crate has no
//! headless `App`-construction path (monitor.rs / tools_registry_tests.rs document the same).

use super::*;
use crate::ui::jobs_ui::{self, JobKind, JobRow, JobState, JobsResponse};
use std::collections::VecDeque;

/// A progress-backed job: (kind, label, progress).
pub(super) type ProgJob = (JobKind, String, Arc<Progress>);

/// Everything `rows_from` needs, gathered from `App` by `holders` — a plain struct so the row logic is
/// testable without an `App`.
#[derive(Default)]
pub(super) struct Holders {
    pub jobs: Vec<ProgJob>,
    /// Queued export output names, in queue order.
    pub export_queue: Vec<String>,
    /// Source paths whose proxy is not built yet, in library order (the current build excluded).
    pub proxy_queued: Vec<String>,
    /// None = the worker holds the lock this frame.
    pub waveforms: Option<usize>,
    pub thumbs: Option<usize>,
    /// Fraction of the requested pre-render that is ready; None = nothing requested.
    pub prerender: Option<f32>,
    pub probing: usize,
}

fn file_name(p: &Path) -> String {
    p.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

/// Walk every holder on `App`. Cheap: a handful of `Progress` mutex reads, no allocation beyond labels.
pub(super) fn progress_jobs(app: &App) -> Vec<ProgJob> {
    let mut v: Vec<ProgJob> = Vec::new();
    if let Some((p, kind)) = &app.export {
        let label = match kind {
            ExportKind::File { path } => format!("Export · {}", file_name(path)),
            ExportKind::Overwrite { original, .. } => format!("Overwrite · {}", file_name(original)),
        };
        v.push((JobKind::Export, label, p.clone()));
    }
    for j in &app.bake_jobs {
        v.push((JobKind::Bake, j.title(), j.progress()));
    }
    for (p, out) in &app.convert_jobs {
        v.push((JobKind::Convert, format!("Convert · {}", file_name(out)), p.clone()));
    }
    for j in &app.media_jobs {
        v.push((JobKind::Media, j.label(), j.progress().clone()));
    }
    for j in &app.mcp_jobs {
        v.push((JobKind::Mcp, format!("MCP · {}", file_name(&j.out)), j.prog.clone()));
    }
    if let Some((src, _, p)) = &app.proxy_job {
        v.push((JobKind::Proxy, format!("Proxy · {}", file_name(Path::new(src))), p.clone()));
    }
    for d in &app.downloads {
        v.push((JobKind::Download, format!("Download · {}", d.url), d.progress.clone()));
    }
    v.extend(app.transcript.jobs());
    if let Some((c, out)) = &app.screen_rec {
        v.push((JobKind::ScreenRec, format!("Recording screen · {}", file_name(out)), c.progress()));
    }
    if let Some((c, out, _)) = &app.voice_rec {
        v.push((JobKind::VoiceRec, format!("Recording voiceover · {}", file_name(out)), c.progress()));
    }
    v
}

fn holders(app: &App) -> Holders {
    let pre = (!app.prerender.segments().is_empty()).then(|| app.prerender.progress()).filter(|&f| f < 1.0);
    Holders {
        jobs: progress_jobs(app),
        export_queue: app.export_queue.iter().map(|c| file_name(&c.opts.out_path)).collect(),
        proxy_queued: app.jobs.proxy_queued.1.clone(),
        waveforms: app.waveforms.pending_count(),
        thumbs: app.thumbs.queue_len(),
        prerender: pre,
        probing: crate::engine::import::probing_count(),
    }
}

/// Pure: one `JobRow` per holder entry, ids `"<kind>:<index within kind>"`.
pub(super) fn rows_from(h: &Holders) -> Vec<JobRow> {
    let mut rows = Vec::new();
    let mut count = std::collections::HashMap::<JobKind, usize>::new();
    for (kind, label, p) in &h.jobs {
        let i = count.entry(*kind).or_default();
        let mut r = JobRow::new(*kind, *i, label.clone());
        *i += 1;
        r.fraction = Some(p.fraction());
        r.status = p.status();
        r.eta = p.eta();
        r.elapsed = p.elapsed();
        r.state = if !p.is_done() {
            JobState::Running
        } else if p.is_cancelled() {
            JobState::Cancelled
        } else if let Some(e) = p.error() {
            JobState::Failed(e)
        } else {
            JobState::Done
        };
        rows.push(r);
    }
    for (i, name) in h.export_queue.iter().enumerate() {
        let mut r = JobRow::new(JobKind::QueuedExport, i, format!("Export · {name}"));
        r.state = JobState::Queued;
        r.status = format!("#{} in the render queue", i + 1);
        rows.push(r);
    }
    for (i, src) in h.proxy_queued.iter().enumerate() {
        let mut r = JobRow::new(JobKind::QueuedProxy, i, format!("Proxy · {}", file_name(Path::new(src))));
        r.state = JobState::Queued;
        r.status = src.clone();
        rows.push(r);
    }
    let cache = |kind: JobKind, label: &str, n: Option<usize>| {
        let mut r = JobRow::new(kind, 0, label);
        r.status = match n {
            Some(n) => format!("{n} queued"),
            None => "…".into(),
        };
        r
    };
    if h.waveforms != Some(0) {
        rows.push(cache(JobKind::Waveforms, "Waveforms", h.waveforms));
    }
    if h.thumbs != Some(0) {
        rows.push(cache(JobKind::Thumbnails, "Thumbnails", h.thumbs));
    }
    if let Some(f) = h.prerender {
        let mut r = JobRow::new(JobKind::Prerender, 0, "Pre-render");
        r.fraction = Some(f);
        rows.push(r);
    }
    if h.probing > 0 {
        let mut r = JobRow::new(JobKind::Probes, 0, "Probing media");
        r.status = format!("{} file(s)", h.probing);
        rows.push(r);
    }
    rows
}

pub(super) fn rows(app: &App) -> Vec<JobRow> {
    rows_from(&holders(app))
}

/// FRAME_HOOKS entry: snapshot, recent-log upkeep, auto-reveal and the repaint cadence.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    let drew = std::mem::take(&mut app.jobs.drew);
    let now = Instant::now();
    if !app.jobs.booted {
        app.jobs.booted = true;
        // screenshot / manual-check hook, like SE_LAYOUT: start with the Jobs tab in front
        if std::env::var("SE_JOBS_PANE").is_ok_and(|v| v == "1") {
            app.surface(Pane::Jobs);
        }
    }
    // queued proxies need a stat per asset — only while the pane is actually shown, every 2 s
    if drew && app.settings.use_proxies {
        let scan = &mut app.jobs.proxy_queued;
        if scan.0.is_none_or(|t| t <= now) {
            let h = app.settings.proxy_height.max(120);
            let cur = app.proxy_job.as_ref().map(|(s, _, _)| s.as_str());
            scan.1 = jobs::proxy_candidates(&app.project.assets, h)
                .into_iter()
                .map(|(s, _)| s)
                .filter(|s| Some(s.as_str()) != cur)
                .collect();
            scan.0 = Some(now + Duration::from_secs(2));
        }
    } else {
        app.jobs.proxy_queued = (None, Vec::new());
    }
    let rows = rows(app);
    let fresh = app.jobs.note(rows, now);
    if app.settings.jobs_auto_reveal && fresh.iter().any(|id| !id.starts_with("mcp:")) {
        // Dynamic: switch the tab (pin-aware); Granular: glow only. Never re-opens a hidden pane —
        // the menu-bar indicator covers that case (goals.md "panels that jump around").
        layout_ctl::surface(app, Pane::Jobs);
    }
    if drew && app.jobs.running() > 0 {
        app.animate_until(ctx, now + Duration::from_millis(150));
    }
}

/// PANE_DRAWERS entry.
pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::Jobs {
        return false;
    }
    app.jobs.drew = true;
    let rows = std::mem::take(&mut app.jobs.last);
    let resp = jobs_ui::show(ui, &mut app.jobs, &rows, &app.palette);
    app.jobs.last = rows;
    apply(app, resp);
    true
}

/// Menu-bar indicator: the queue glyph + running count while anything runs; click toggles the pane.
pub(super) fn indicator(app: &mut App, ui: &mut egui::Ui) {
    let n = app.jobs.running();
    if n == 0 {
        return;
    }
    app.jobs.drew = true;
    let r = ui.add(egui::Button::new(format!("{n}")).small()).on_hover_text("Background jobs — show the Jobs pane");
    tools::glyph_label(ui, tools::Glyph::Queue, ui.visuals().text_color());
    if r.clicked() {
        app.toggle_pane(Pane::Jobs);
    }
}

/// ACT_HANDLERS entry.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::ToggleJobs => {
            app.toggle_pane(Pane::Jobs);
            true
        }
        _ => false,
    }
}

/// `"<kind>:<i>"` → (kind name, i).
fn split_id(id: &str) -> Result<(&str, usize), String> {
    let (k, i) = id.rsplit_once(':').ok_or_else(|| format!("unknown job id '{id}'"))?;
    let i = i.parse().map_err(|_| format!("unknown job id '{id}'"))?;
    Ok((k, i))
}

/// Pure half of Cancel: sets the progress flag / drops the queued export. `Ok(Some(kind))` = a
/// recording — the caller must stop its `Capture` (that takes `&mut App`).
pub(super) fn cancel_core<T>(jobs: &[ProgJob], queue: &mut VecDeque<T>, id: &str) -> Result<Option<JobKind>, String> {
    let (k, i) = split_id(id)?;
    if k == JobKind::QueuedExport.name() {
        return match queue.remove(i) {
            Some(_) => Ok(None),
            None => Err(format!("unknown job id '{id}'")),
        };
    }
    let (kind, _, p) =
        jobs.iter().filter(|(kind, ..)| kind.name() == k).nth(i).ok_or_else(|| format!("unknown job id '{id}'"))?;
    if !kind.can_cancel() {
        return Err(kind.cancel_hint().unwrap_or("this job cannot be cancelled").into());
    }
    if matches!(kind, JobKind::ScreenRec | JobKind::VoiceRec) {
        return Ok(Some(*kind));
    }
    p.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(None)
}

/// Pure half of ▲▼: swap a queued export with its neighbour (clamped at the ends). False = no-op.
pub(super) fn reorder_queue<T>(queue: &mut VecDeque<T>, id: &str, delta: isize) -> bool {
    let Ok((k, i)) = split_id(id) else { return false };
    if k != JobKind::QueuedExport.name() || i >= queue.len() {
        return false;
    }
    let j = (i as isize + delta).clamp(0, queue.len() as isize - 1) as usize;
    if j == i {
        return false;
    }
    queue.swap(i, j);
    true
}

pub(super) fn cancel(app: &mut App, id: &str) -> Result<(), String> {
    let jobs = progress_jobs(app);
    match cancel_core(&jobs, &mut app.export_queue, id)? {
        Some(JobKind::ScreenRec) => app.stop_screen_capture(),
        Some(JobKind::VoiceRec) => app.stop_voiceover(),
        _ => {}
    }
    Ok(())
}

/// "Build next": `src` must still be a proxy candidate.
pub(super) fn proxy_next(app: &mut App, src: &str) -> Result<(), String> {
    let h = app.settings.proxy_height.max(120);
    if !jobs::proxy_candidates(&app.project.assets, h).iter().any(|(p, _)| p == src) {
        return Err(format!("'{src}' has no proxy to build (not a video above proxy size, already built, or missing)"));
    }
    app.proxy_next = Some(src.to_string());
    app.proxy_scan_at = None;
    Ok(())
}

pub(super) fn apply(app: &mut App, resp: JobsResponse) {
    for id in &resp.cancel {
        if let Err(e) = cancel(app, id) {
            app.toast(e);
        }
    }
    for (id, delta) in &resp.reorder {
        reorder_queue(&mut app.export_queue, id, *delta);
    }
    if let Some(id) = &resp.proxy_next {
        if let Some(src) = split_id(id).ok().and_then(|(_, i)| app.jobs.proxy_queued.1.get(i).cloned()) {
            if let Err(e) = proxy_next(app, &src) {
                app.toast(e);
            }
        }
    }
    if resp.clear_caches {
        app.waveforms.clear();
        app.thumbs.clear();
        app.prerender.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(kind: JobKind) -> ProgJob {
        (kind, kind.name().to_string(), Progress::new())
    }

    #[test]
    fn rows_cover_every_job_holder() {
        let kinds = [
            JobKind::Export,
            JobKind::Bake,
            JobKind::Convert,
            JobKind::Media,
            JobKind::Mcp,
            JobKind::Proxy,
            JobKind::Download,
            JobKind::Transcribe,
            JobKind::Tracking,
            JobKind::Tts,
            JobKind::ModelDownload,
            JobKind::ScreenRec,
            JobKind::VoiceRec,
        ];
        let h = Holders {
            jobs: kinds.iter().map(|&k| job(k)).collect(),
            export_queue: vec!["a.mp4".into()],
            proxy_queued: vec!["C:/b.mp4".into()],
            waveforms: Some(2),
            thumbs: None,
            prerender: Some(0.5),
            probing: 1,
        };
        let rows = rows_from(&h);
        for k in kinds.iter().copied().chain([
            JobKind::QueuedExport,
            JobKind::QueuedProxy,
            JobKind::Waveforms,
            JobKind::Thumbnails,
            JobKind::Prerender,
            JobKind::Probes,
        ]) {
            let r = rows.iter().find(|r| r.kind == k).unwrap_or_else(|| panic!("no row for {k:?}"));
            assert_eq!(r.can_cancel, k.can_cancel(), "{k:?}");
            assert_eq!(r.can_reorder, k == JobKind::QueuedExport, "{k:?}");
        }
        // the table: tts / tracking greyed out, caches + probes + queued proxies never cancellable
        for k in [
            JobKind::Tts,
            JobKind::Tracking,
            JobKind::Waveforms,
            JobKind::Thumbnails,
            JobKind::Prerender,
            JobKind::Probes,
            JobKind::QueuedProxy,
        ] {
            assert!(!k.can_cancel(), "{k:?}");
        }
        for k in [
            JobKind::Export,
            JobKind::Bake,
            JobKind::Convert,
            JobKind::Media,
            JobKind::Mcp,
            JobKind::Proxy,
            JobKind::Download,
            JobKind::Transcribe,
            JobKind::ModelDownload,
            JobKind::ScreenRec,
            JobKind::VoiceRec,
        ] {
            assert!(k.can_cancel(), "{k:?}");
        }
        let thumbs = rows.iter().find(|r| r.kind == JobKind::Thumbnails).unwrap();
        assert_eq!(thumbs.status, "…", "a locked worker shows an ellipsis, not a stall");
        assert_eq!(rows.iter().find(|r| r.kind == JobKind::QueuedExport).unwrap().state, JobState::Queued);
        assert!(rows.iter().all(|r| r.state == JobState::Running || r.state == JobState::Queued));
        // ids are stable per kind
        assert_eq!(rows[0].id, "export:0");
        // an idle cache draws no row
        let idle = Holders { waveforms: Some(0), thumbs: Some(0), ..Default::default() };
        assert!(rows_from(&idle).is_empty());
    }

    #[test]
    fn cancel_sets_the_progress_flag_and_removes_a_queued_export() {
        let jobs = vec![job(JobKind::Convert), job(JobKind::Convert), job(JobKind::Tts), job(JobKind::ScreenRec)];
        let mut q: VecDeque<u8> = VecDeque::from(vec![1, 2, 3]);
        assert_eq!(cancel_core(&jobs, &mut q, "convert:1"), Ok(None));
        assert!(jobs[1].2.is_cancelled() && !jobs[0].2.is_cancelled(), "exactly the addressed row");
        assert_eq!(cancel_core(&jobs, &mut q, "queue:1"), Ok(None));
        assert_eq!(q, VecDeque::from(vec![1, 3]));
        assert!(cancel_core(&jobs, &mut q, "tts:0").is_err(), "tts is not cancellable");
        assert!(!jobs[2].2.is_cancelled());
        assert_eq!(
            cancel_core(&jobs, &mut q, "screen-rec:0"),
            Ok(Some(JobKind::ScreenRec)),
            "recordings stop via Capture::stop"
        );
        assert!(cancel_core(&jobs, &mut q, "convert:7").is_err());
        assert!(cancel_core(&jobs, &mut q, "queue:9").is_err());
        assert!(cancel_core(&jobs, &mut q, "nonsense").is_err());
    }

    #[test]
    fn reorder_swaps_export_queue_positions() {
        let mut q: VecDeque<u8> = VecDeque::from(vec![1, 2, 3]);
        assert!(reorder_queue(&mut q, "queue:1", -1));
        assert_eq!(q, VecDeque::from(vec![2, 1, 3]));
        assert!(reorder_queue(&mut q, "queue:1", 1));
        assert_eq!(q, VecDeque::from(vec![2, 3, 1]));
        assert!(!reorder_queue(&mut q, "queue:0", -1), "clamps at the top");
        assert!(!reorder_queue(&mut q, "queue:2", 1), "clamps at the bottom");
        assert!(!reorder_queue(&mut q, "convert:0", 1), "no-op on a non-queue row");
        assert!(!reorder_queue(&mut q, "queue:5", -1));
        assert_eq!(q, VecDeque::from(vec![2, 3, 1]));
    }

    #[test]
    fn try_lock_never_blocks_the_snapshot() {
        let cache =
            Arc::new(crate::media::thumbs::ThumbCache::new(egui::Context::default(), crate::media::Backend::Ffmpeg));
        let c2 = cache.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            c2.hold_lock_for_test(|| {
                tx.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(400));
            });
        });
        rx.recv().unwrap();
        let t = Instant::now();
        assert_eq!(cache.queue_len(), None, "a locked worker reads as unknown");
        assert!(t.elapsed() < Duration::from_millis(200), "try_lock returned promptly, not after the worker");
        std::thread::sleep(Duration::from_millis(450));
        assert_eq!(cache.queue_len(), Some(0));
    }
}
