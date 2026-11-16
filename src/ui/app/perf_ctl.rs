//! ---- ws:perf-window ----
//! The Performance window's App side (`ui::perf_ui` draws it): `tick` (FRAME_HOOKS) samples the live
//! numbers ~4 times a second while the window is open (or a minute after `perf.report`), `window`
//! (WINDOW_DRAWERS) draws it and applies a Jump / Fix, `project_rows` ranks the heaviest sources and clips,
//! and `perf.report` / `perf.window` are the MCP twins.

use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::perf_ui::{self, Click, Fix, Inputs, Row, Tab};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// Disk cache bytes (u64::MAX = not measured yet), walked on a thread of its own every 15 s at most.
static DISK: AtomicU64 = AtomicU64::new(u64::MAX);
static DISK_AT: Mutex<Option<Instant>> = Mutex::new(None);

fn refresh_disk(force: bool) {
    let mut at = DISK_AT.lock().unwrap_or_else(|e| e.into_inner());
    if !force && at.is_some_and(|t| t.elapsed() < Duration::from_secs(15)) {
        return;
    }
    *at = Some(Instant::now());
    std::thread::spawn(|| {
        crate::media::bg::lower_thread();
        DISK.store(caches::cache_bytes(), Relaxed);
    });
}

pub(super) fn tick(app: &mut App, _ctx: &egui::Context) {
    let on = app.perf.sampling();
    if crate::perf::accumulating() != on {
        crate::perf::accumulate(on);
    }
    if !on || !app.perf.live.due() {
        return;
    }
    let (published, dropped, gap_ms, ram, ram_budget, queued) = app.player.perf_sample();
    let i = Inputs {
        playing: app.player.is_playing(),
        target_fps: app.project.fps as f32,
        published,
        dropped,
        gap_ms: gap_ms as f32,
    };
    app.perf.live.sample(crate::perf::self_totals(), Instant::now(), i);
    refresh_disk(false);
    let l = &mut app.perf.live;
    (l.ui_p95, l.decode_avg, l.decode_worst, l.hit_rate) =
        (app.hud.frame.p95(), app.hud.decode_avg_ms, app.hud.decode_worst_ms, app.hud.hit_rate);
    (l.ram_mb, l.ram_budget_mb, l.queued) = (ram as f32 / 1048576.0, ram_budget as f32 / 1048576.0, queued);
    let disk = DISK.load(Relaxed);
    l.disk_gb = (disk != u64::MAX).then_some(disk as f32 / 1e9);
    l.disk_budget_gb = crate::engine::cache_lru::budget_bytes(app.settings.cache_disk_gb).map(|b| b as f32 / 1e9);
    (l.jobs, l.jobs_paused) = (app.jobs.running(), crate::media::bg::paused());
}

fn file_name(p: &str) -> String {
    Path::new(p).file_name().map_or_else(|| p.to_string(), |n| n.to_string_lossy().into_owned())
}

fn timecode(t: f64) -> String {
    format!("{}:{:02}", (t / 60.0) as u64, (t % 60.0) as u64)
}

/// Nesting depth and total track count of a sequence (a sequence inside a sequence counts deeper).
fn nest(p: &Project, seq: Id, depth: usize) -> (usize, usize) {
    let Some(s) = p.sequence(seq) else { return (depth, 0) };
    if depth > 8 {
        return (depth, s.tracks.len()); // a cycle guard; open_sequence refuses cycles anyway
    }
    let mut out = (depth, s.tracks.len());
    for c in s.tracks.iter().flat_map(|t| &t.clips).filter(|c| c.kind == ClipKind::Sequence) {
        let (d, n) = nest(p, c.sequence, depth + 1);
        out = (out.0.max(d), out.1 + n);
    }
    out
}

/// The Project tab: one row per video source on the timeline, one per costly clip, the top 8 by cost
/// (the rest summed); plus the rows outside the donut (disk cache, background jobs).
/// ponytail: clip and unmeasured-source costs are estimates from a few rules of thumb (marked "~");
/// measure per clip in the render thread if the estimates mislead.
pub(super) fn project_rows(app: &App) -> (Vec<Row>, Vec<Row>) {
    let p = &app.project;
    let h = app.settings.proxy_height.max(120);
    let mut rows = Vec::new();
    let first_use = |asset: Id| {
        p.all_clips().filter(|(_, c)| c.asset == asset).map(|(_, c)| (c.id, c.start)).min_by(|a, b| a.1.total_cmp(&b.1))
    };
    for a in p.assets.iter().filter(|a| a.kind == ClipKind::Video && a.html.is_none()) {
        let Some(jump) = first_use(a.id) else { continue };
        let ready =
            crate::media::proxy::status(a, app.settings.use_proxies, h) == crate::media::proxy::ProxyStatus::Ready;
        let bytes = std::fs::metadata(&a.path).map_or(0, |m| m.len());
        let mbit = if a.duration > 0.0 { bytes as f64 * 8.0 / a.duration / 1e6 } else { 0.0 };
        let heavy = crate::media::proxy::decode_heavy(&a.codec, a.width, a.height, mbit * 1e6);
        let stat = crate::perf::source_stat(&a.path);
        let (ms, est) = match &stat {
            Some(s) if s.n > 0 => (s.us as f32 / s.n as f32 / 1000.0, false),
            _ if ready => (1.0, true),
            _ => ((a.width as f32 * a.height as f32 / (1920.0 * 1080.0)) * if heavy { 8.0 } else { 2.0 }, true),
        };
        let hw = match &stat {
            Some(s) if s.hw => "hardware",
            Some(_) => "software",
            None => "not measured yet",
        };
        let proxy = if ready {
            "proxy ready"
        } else if a.proxy == Some(true) {
            "proxy queued"
        } else {
            "no proxy"
        };
        let detail = format!("{}×{} · {} · {mbit:.1} Mbit/s · {hw} · {proxy}", a.width, a.height, a.codec);
        let fix =
            (!ready && a.proxy != Some(true) && a.proxy != Some(false)).then_some((Fix::Proxy(a.id), "Make proxy"));
        rows.push(Row { label: file_name(&a.path), detail, ms, est, jump: Some(jump), fix, ..Default::default() });
    }
    for t in p.tracks.iter().filter(|t| t.kind == TrackKind::Video) {
        for c in t.clips.iter().filter(|c| c.enabled) {
            let asset = p.asset(c.asset);
            let fx =
                c.effects.iter().chain(asset.into_iter().flat_map(|a| a.effects.iter())).filter(|e| e.enabled).count();
            let (depth, tracks) = if c.kind == ClipKind::Sequence { nest(p, c.sequence, 1) } else { (0, 0) };
            let html = asset.is_some_and(|a| {
                a.html.is_some() && crate::media::proxy::status(a, true, h) != crate::media::proxy::ProxyStatus::Ready
            });
            let mid = c.start + c.duration / 2.0;
            let layers = p
                .tracks
                .iter()
                .filter(|t| t.kind == TrackKind::Video)
                .flat_map(|t| &t.clips)
                .filter(|o| o.enabled && o.start <= mid && mid < o.start + o.duration)
                .count();
            let ms = fx as f32 * 1.5 + tracks as f32 * 2.0 * depth as f32 + if html { 6.0 } else { 0.0 };
            if ms <= 0.0 {
                continue;
            }
            let mut d = Vec::new();
            if fx > 0 {
                d.push(format!("{fx} effect{}", if fx == 1 { "" } else { "s" }));
            }
            if depth > 0 {
                d.push(format!("nested {depth} deep, {tracks} tracks"));
            }
            if html {
                d.push("HTML layer still rendering".into());
            }
            d.push(format!("{layers} layers at {}", timecode(mid)));
            let label = if c.name.is_empty() {
                asset.map_or_else(|| "clip".into(), |a| file_name(&a.path))
            } else {
                c.name.clone()
            };
            rows.push(Row {
                label: format!("{label} @ {}", timecode(c.start)),
                detail: d.join(" · "),
                ms,
                est: true,
                jump: Some((c.id, c.start)),
                fix: Some((Fix::Prerender(c.start, c.start + c.duration), "Pre-render span")),
                ..Default::default()
            });
        }
    }
    perf_ui::segments(&mut rows); // (sorts)
    if rows.len() > 8 {
        let rest: f32 = rows.drain(8..).map(|r| r.ms).sum();
        rows.push(Row { label: "Everything else".into(), ms: rest, est: true, ..Default::default() });
    }
    let mut extra = Vec::new();
    let budget = crate::engine::cache_lru::budget_bytes(app.settings.cache_disk_gb);
    let disk = DISK.load(Relaxed);
    if disk != u64::MAX {
        let value = match budget {
            Some(b) => format!("{:.1} / {:.0} GB", disk as f64 / 1e9, b as f64 / 1e9),
            None => format!("{:.1} GB", disk as f64 / 1e9),
        };
        let fix = budget.is_some().then_some((Fix::PruneCache, "Free disk cache"));
        extra.push(Row {
            label: "Disk cache".into(),
            detail: "proxies, pre-renders, HTML renders, thumbnails".into(),
            value: Some(value),
            fix,
            ..Default::default()
        });
    }
    let paused = crate::media::bg::paused();
    let n = app.jobs.running();
    if n > 0 || paused {
        let (value, label) =
            if paused { ("paused".to_string(), "Resume jobs") } else { (format!("{n} running"), "Pause jobs") };
        let detail = "proxies, HTML renders, thumbnails and transcripts; they already wait while playing".into();
        extra.push(Row {
            label: "Background jobs".into(),
            detail,
            value: Some(value),
            fix: Some((Fix::PauseJobs(!paused), label)),
            ..Default::default()
        });
    }
    (rows, extra)
}

fn refresh_project(app: &mut App) {
    refresh_disk(false);
    let (rows, extra) = project_rows(app);
    (app.perf.project, app.perf.extra, app.perf.project_at) = (rows, extra, Some(Instant::now()));
}

fn apply(app: &mut App, c: Click) {
    match c {
        Click::Jump(id, t) => {
            app.selection = vec![id];
            app.seek(t);
        }
        Click::Fix(Fix::Proxy(id)) => {
            let Some(path) = app.project.asset(id).map(|a| a.path.clone()) else { return };
            app.push_undo();
            if let Some(a) = app.project.assets.iter_mut().find(|a| a.id == id) {
                a.proxy = Some(true);
            }
            app.after_edit();
            (app.proxy_next, app.proxy_scan_at) = (Some(path.clone()), None);
            if !app.settings.use_proxies {
                app.settings.use_proxies = true; // a proxy is only played with proxies on
                app.settings.save();
            }
            app.toast(format!("Building a proxy for {}", file_name(&path)));
        }
        Click::Fix(Fix::Prerender(a, b)) => match app.request_prerender_range(a, b) {
            Ok(()) => app.toast(format!("Pre-rendering {a:.1}–{b:.1} s")),
            Err(e) => app.toast(e),
        },
        Click::Fix(Fix::PruneCache) => {
            caches::prune_now(app);
            refresh_disk(true);
            app.toast("Freeing the disk cache down to its budget");
        }
        Click::Fix(Fix::PauseJobs(on)) => {
            crate::media::bg::set_paused(on);
            app.toast(if on { "Background jobs paused" } else { "Background jobs resumed" });
        }
    }
    app.perf.project_at = None; // re-rank on the next frame
}

pub(super) fn window(app: &mut App, ctx: &egui::Context) {
    if !app.perf.open {
        return;
    }
    if app.perf.tab == Tab::Project && app.perf.project_at.is_none_or(|t| t.elapsed() > Duration::from_secs(2)) {
        refresh_project(app);
    }
    let (mut open, tab) = (true, app.perf.tab);
    let click = egui::Window::new("Performance")
        .open(&mut open)
        .resizable(false)
        .show(ctx, |ui| perf_ui::body(ui, &mut app.perf))
        .and_then(|r| r.inner.flatten());
    app.perf.open = open;
    if app.perf.tab != tab {
        ctx.request_repaint(); // the other tab's rows are made at the start of the next frame
    }
    if let Some(c) = click {
        apply(app, c);
        ctx.request_repaint();
    }
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    if a != Action::PerfWindow {
        return false;
    }
    app.perf.open = true;
    true
}

fn rows_json(rows: &[Row], shares: Option<&[f32]>) -> Vec<Value> {
    rows.iter()
        .enumerate()
        .map(|(i, r)| {
            json!({
                "label": r.label, "detail": r.detail, "ms_per_frame": r.ms, "estimate": r.est,
                "share": shares.map(|s| s[i]), "value": r.value,
                "clip": r.jump.map(|j| j.0), "t": r.jump.map(|j| j.1),
                "fix": r.fix.as_ref().map(|f| f.1),
            })
        })
        .collect()
}

fn report(app: &mut App) -> Value {
    app.perf.report_until = Some(Instant::now() + Duration::from_secs(60));
    crate::perf::accumulate(true);
    refresh_project(app);
    let l = &app.perf.live;
    let mut live_rows = l.rows.clone();
    let ls = perf_ui::segments(&mut live_rows);
    let mut proj = app.perf.project.clone();
    let ps = perf_ui::segments(&mut proj);
    json!({
        "measuring": !live_rows.is_empty(),
        "note": "live numbers are sampled ~4x a second while the Performance window is open, or for 60 s after this call: play, then call again",
        "live": {
            "playing": l.playing, "fps": l.fps, "target_fps": l.target_fps,
            "dropped_10s": l.dropped_10s(), "dropped_session": l.dropped_session, "worst_gap_ms": l.worst_gap(),
            "ui_frame_p95_ms": l.ui_p95, "decode_avg_ms": l.decode_avg, "decode_worst_ms": l.decode_worst,
            "cache_hit_rate": l.hit_rate, "ram_cache_mb": l.ram_mb, "ram_cache_budget_mb": l.ram_budget_mb,
            "frames_queued": l.queued, "disk_cache_gb": l.disk_gb, "disk_cache_budget_gb": l.disk_budget_gb,
            "jobs_running": l.jobs, "jobs_paused": l.jobs_paused,
            "breakdown": rows_json(&live_rows, Some(&ls[..])),
        },
        "project": { "rows": rows_json(&proj, Some(&ps[..])), "other": rows_json(&app.perf.extra, None) },
    })
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "perf.report",
        desc: "What the Performance window shows, as JSON: `live` (fps, dropped frames, worst gap, UI p95, decode, caches, jobs, and `breakdown`: ms per frame by category with its share) and `project` (the heaviest sources and clips, ms per frame - `estimate` true when not measured - with the clip to jump to and the fix on offer). Starts sampling for 60 s: play, then call again for live numbers.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| Ok(ToolOutcome::Done(report(app))),
    },
    // the ToolDef twin of Action::PerfWindow
    ToolDef {
        name: "perf.window",
        desc: "Open or close the Performance window (omit `on` to toggle); `tab` picks live | project.",
        args: &["on:boolean:false:", "tab:string:false:live | project"],
        kind: ToolKind::Ui,
        run: |app, args| {
            // picking a tab opens it; neither `on` nor `tab` toggles
            let toggle = !app.perf.open || args.get("tab").is_some();
            app.perf.open = args.get("on").and_then(|v| v.as_bool()).unwrap_or(toggle);
            match args.get("tab").and_then(|v| v.as_str()) {
                Some("live") => app.perf.tab = Tab::Live,
                Some("project") => app.perf.tab = Tab::Project,
                Some(t) => return Err(format!("tab: live | project, not '{t}'")),
                None => {}
            }
            Ok(ToolOutcome::Done(json!({"ok": true, "open": app.perf.open})))
        },
    },
];
