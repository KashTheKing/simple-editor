//! ---- ws:perf-window ----
//! The Performance pane's App side (`ui::perf_ui` draws it): `tick` (FRAME_HOOKS) samples the live
//! numbers ~4 times a second while the pane is on screen (or a minute after `perf.report`), `draw`
//! (PANE_DRAWERS) draws it and applies a Jump / Fix, `button` is the top bar's chart button,
//! `project_rows` ranks the heaviest sources and clips, and `perf.report` / `perf.window` are the MCP twins.

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
    app.perf.shown = app.pane_drawn(Pane::Performance);
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
    bench_tick(app);
}

/// `perf.bench`: one benchmark run in the live app. MCP calls run on the UI thread, so the tool only
/// starts it (seek, Play through the play gate) and `tick` records it; `status` reads the result.
struct Bench {
    from: f64,
    to: f64,
    started: Instant,
    /// when the clock really started (the play gate may buffer first)
    rolling: Option<Instant>,
    base: (u64, u64),
    worst_gap_ms: f32,
    ui_p95_max: f32,
    decode_worst_ms: f32,
    min_fps: f32,
    buffering_samples: u32,
    samples: u32,
    result: Option<Value>,
}

static BENCH: Mutex<Option<Bench>> = Mutex::new(None);

fn bench_lock() -> std::sync::MutexGuard<'static, Option<Bench>> {
    BENCH.lock().unwrap_or_else(|e| e.into_inner())
}

/// After each live sample: accumulate the running bench, finish it at `to` (or when playback stopped).
fn bench_tick(app: &mut App) {
    let mut g = bench_lock();
    let Some(b) = g.as_mut().filter(|b| b.result.is_none()) else { return };
    app.perf.report_until = Some(Instant::now() + Duration::from_secs(60)); // keep sampling
    let playing = app.player.is_playing();
    let l = &app.perf.live;
    if playing {
        b.rolling.get_or_insert_with(Instant::now);
        b.samples += 1;
        if let Some(h) = l.hist.back() {
            b.worst_gap_ms = b.worst_gap_ms.max(h.2);
            if b.samples > 1 {
                b.min_fps = b.min_fps.min(h.0); // the first sample still holds the start-up ramp
            }
        }
        b.ui_p95_max = b.ui_p95_max.max(l.ui_p95);
        b.decode_worst_ms = b.decode_worst_ms.max(l.decode_worst);
    }
    if app.player.is_buffering() {
        b.buffering_samples += 1;
    }
    let t = app.player.time();
    // ponytail: "stopped" = not playing once it has rolled and no play-gate buffering is pending; a
    // bench the user pauses by hand ends there and says so (`stopped_early`).
    let stopped = b.rolling.is_some() && !playing && !app.play_gate.buffering();
    let timed_out = b.started.elapsed() > Duration::from_secs_f64((b.to - b.from) * 4.0 + 60.0);
    if t + 0.5 / app.project.fps.max(1.0) < b.to && !stopped && !timed_out {
        return;
    }
    if playing {
        app.player.pause();
        app.playhead = app.player.time();
    }
    let (published, dropped, ..) = app.player.perf_sample();
    let (p, d) = (published.saturating_sub(b.base.0), dropped.saturating_sub(b.base.1));
    let wall = b.rolling.map_or(0.0, |r| r.elapsed().as_secs_f64());
    let mut rows = l.rows.clone();
    let shares = perf_ui::segments(&mut rows);
    let target = app.project.fps;
    let (gap_budget, ui_budget) = (50.0f32, 8.0f32);
    let ok = d == 0 && b.worst_gap_ms <= gap_budget && b.ui_p95_max <= ui_budget && !timed_out;
    b.result = Some(json!({
        "done": true, "ok": ok, "from": b.from, "to": b.to, "reached": t,
        "stopped_early": (stopped || timed_out) && t + 0.5 < b.to, "timed_out": timed_out,
        "start_wait_ms": b.rolling.map(|r| r.saturating_duration_since(b.started).as_millis()),
        "wall_s": wall, "frames_published": p, "frames_dropped": d,
        "drop_rate": d as f64 / (p + d).max(1) as f64,
        "avg_fps": if wall > 0.0 { p as f64 / wall } else { 0.0 }, "min_fps": b.min_fps, "target_fps": target,
        "worst_gap_ms": b.worst_gap_ms, "ui_frame_p95_max_ms": b.ui_p95_max, "decode_worst_ms": b.decode_worst_ms,
        "buffering_samples": b.buffering_samples, "samples": b.samples,
        "budgets": {"dropped": 0, "worst_gap_ms": gap_budget, "ui_frame_p95_ms": ui_budget},
        "breakdown": rows_json(&rows, Some(&shares[..])),
        "hint": "perf.report's `project` rows rank what in the project costs the most, with fixes",
    }));
}

fn bench(app: &mut App, args: &Value) -> Result<Value, String> {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("start");
    match action {
        "status" => Ok(match bench_lock().as_ref() {
            None => json!({"done": false, "running": false, "note": "no benchmark yet: perf.bench {action: start}"}),
            Some(b) => b.result.clone().unwrap_or_else(|| {
                json!({"done": false, "running": true, "t": app.player.time(), "from": b.from, "to": b.to,
                       "elapsed_s": b.started.elapsed().as_secs_f64(), "buffering": app.player.is_buffering()})
            }),
        }),
        "stop" => {
            if bench_lock().as_ref().is_some_and(|b| b.result.is_none()) {
                app.player.pause();
                app.playhead = app.player.time();
            }
            *bench_lock() = None;
            Ok(json!({"ok": true}))
        }
        "start" => {
            let dur = app.project.duration();
            let from = args.get("from").and_then(|v| v.as_f64()).unwrap_or(app.playhead).clamp(0.0, dur);
            let to = args.get("to").and_then(|v| v.as_f64()).unwrap_or(from + 10.0).min(dur);
            if to <= from {
                return Err(format!("to ({to}) must be after from ({from}); the timeline is {dur:.2} s"));
            }
            app.player.pause();
            app.seek(from);
            app.perf.report_until = Some(Instant::now() + Duration::from_secs(60));
            crate::perf::accumulate(true);
            let (published, dropped, ..) = app.player.perf_sample();
            *bench_lock() = Some(Bench {
                from, to, started: Instant::now(), rolling: None, base: (published, dropped),
                worst_gap_ms: 0.0, ui_p95_max: 0.0, decode_worst_ms: 0.0, min_fps: f32::MAX,
                buffering_samples: 0, samples: 0, result: None,
            });
            app.play_gated(1.0);
            Ok(json!({"ok": true, "started": true, "from": from, "to": to,
                      "note": "playing; poll perf.bench {action: status} until done is true"}))
        }
        a => Err(format!("action: start | status | stop, not '{a}'")),
    }
}

fn file_name(p: &str) -> String {
    Path::new(p).file_name().map_or_else(|| p.to_string(), |n| n.to_string_lossy().into_owned())
}

fn timecode(t: f64) -> String {
    format!("{}:{:02}", (t / 60.0) as u64, (t % 60.0) as u64)
}

/// Nesting depth and total track count of a sequence (a sequence inside a sequence counts deeper).
/// (nesting depth, tracks summed over every nested instance) of sequence `seq`. Each sequence is measured
/// once (`memo`): walking it again for every clip that uses it was exponential in the nesting and froze
/// the UI thread on a project with many nests. A cycle is cut where it closes.
fn nest(p: &Project, seq: Id, memo: &mut std::collections::HashMap<Id, (usize, usize)>, stack: &mut Vec<Id>) -> (usize, usize) {
    if let Some(&v) = memo.get(&seq) {
        return v;
    }
    let Some(s) = p.sequence(seq) else { return (1, 0) };
    if stack.contains(&seq) {
        return (1, s.tracks.len()); // open_sequence refuses cycles anyway
    }
    stack.push(seq);
    let mut out = (1, s.tracks.len());
    for c in s.tracks.iter().flat_map(|t| &t.clips).filter(|c| c.kind == ClipKind::Sequence) {
        let (d, n) = nest(p, c.sequence, memo, stack);
        out = (out.0.max(d + 1), out.1.saturating_add(n));
    }
    stack.pop();
    memo.insert(seq, out);
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
    let mut nests = std::collections::HashMap::new();
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
            let (depth, tracks) = if c.kind == ClipKind::Sequence { nest(p, c.sequence, &mut nests, &mut Vec::new()) } else { (0, 0) };
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

/// PANE_DRAWERS entry: the pane (scrolls when it is shorter or narrower than the card).
pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::Performance {
        return false;
    }
    if app.perf.tab == Tab::Project && app.perf.project_at.is_none_or(|t| t.elapsed() > Duration::from_secs(2)) {
        refresh_project(app);
    }
    let tab = app.perf.tab;
    let click = egui::ScrollArea::both()
        .id_salt("perf-pane")
        .auto_shrink([false, false])
        .show(ui, |ui| perf_ui::body(ui, &mut app.perf))
        .inner;
    if app.perf.tab != tab {
        ui.ctx().request_repaint(); // the other tab's rows are made at the start of the next frame
    }
    if let Some(c) = click {
        apply(app, c);
        ui.ctx().request_repaint();
    }
    true
}

/// Open the pane, or bring it to the front of its tab group.
fn open(app: &mut App) {
    app.surface(Pane::Performance);
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    if a != Action::PerfWindow {
        return false;
    }
    open(app);
    true
}

/// Top bar: the Performance button (beside Jobs and Quick Export). It toggles the pane and is lit while
/// the pane is on screen.
pub(super) fn button(app: &mut App, ui: &mut egui::Ui) {
    let id = egui::Id::new("se-perf-button");
    let open = app.pane_drawn(Pane::Performance);
    if crate::ui::titlebar::panel_toggle_button(ui, &app.palette, id, tools::Glyph::Perf, "Performance", open).clicked() {
        app.toggle_panel(Pane::Performance);
    }
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
        "note": "live numbers are sampled ~4x a second while the Performance pane is on screen, or for 60 s after this call: play, then call again",
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
        desc: "What the Performance pane shows, as JSON: `live` (fps, dropped frames, worst gap, UI p95, decode, caches, jobs, and `breakdown`: ms per frame by category with its share) and `project` (the heaviest sources and clips, ms per frame - `estimate` true when not measured - with the clip to jump to and the fix on offer). Starts sampling for 60 s: play, then call again for live numbers.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| Ok(ToolOutcome::Done(report(app))),
    },
    ToolDef {
        name: "perf.bench",
        desc: "Benchmark playback in the live app: `start` seeks to `from` (default: the playhead) and plays to `to` (default from + 10 s) through the play gate, recording as the Performance pane does; poll `status` until `done` for frames published / dropped, avg and min fps, worst frame gap, UI frame p95, decode worst, start wait, buffering, `ok` against the budgets (0 dropped, gap <= 50 ms, UI p95 <= 8 ms) and the per-category `breakdown`; `stop` ends it. Pair with perf.report for the heaviest project items and their fixes.",
        args: &["action:string:false:start | status | stop (default start)", "from:number:false:", "to:number:false:"],
        kind: ToolKind::Ui,
        run: |app, args| bench(app, args).map(ToolOutcome::Done),
    },
    // the ToolDef twin of Action::PerfWindow
    ToolDef {
        name: "perf.window",
        desc: "Open the Performance pane or bring it to the front (`on: false` hides it); `tab` picks live | project.",
        args: &["on:boolean:false:", "tab:string:false:live | project"],
        kind: ToolKind::Ui,
        run: |app, args| {
            match args.get("tab").and_then(|v| v.as_str()) {
                Some("live") => app.perf.tab = Tab::Live,
                Some("project") => app.perf.tab = Tab::Project,
                Some(t) => return Err(format!("tab: live | project, not '{t}'")),
                None => {}
            }
            let on = args.get("on").and_then(|v| v.as_bool()).unwrap_or(true);
            if on {
                open(app);
            } else if app.layout.is_visible(Pane::Performance) {
                app.toggle_pane(Pane::Performance);
            }
            Ok(ToolOutcome::Done(json!({"ok": true, "open": on})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::layout::Layout;

    /// Many clips of nested sequences that nest each other (and a cycle): measured in a blink, not the
    /// exponential walk that froze the editor when the Performance tab opened.
    #[test]
    fn nest_depth_is_fast_on_wide_nesting_and_cycles() {
        let path = crate::media::ffpipe::tests::test_mp4();
        let mut p = Project::from_media(crate::media::probe(&path, crate::media::Backend::Auto).unwrap());
        let mut clip = p.tracks[0].clips[0].clone();
        clip.kind = ClipKind::Sequence;
        let track = p.tracks[0].clone();
        // sequence k holds 30 clips of sequence k+1; the last one holds 30 clips of the first (a cycle)
        let ids: Vec<Id> = (0..10).map(|i| 1000 + i).collect();
        for (i, &id) in ids.iter().enumerate() {
            let mut t = track.clone();
            t.clips = (0..30).map(|_| Clip { sequence: ids[(i + 1) % ids.len()], ..clip.clone() }).collect();
            p.sequences.push(crate::model::Sequence { id, name: format!("S{i}"), width: 1920, height: 1080, fps: 30.0, tracks: vec![t], max_duration: None });
        }
        let t0 = Instant::now();
        let (depth, tracks) = nest(&p, ids[0], &mut Default::default(), &mut Vec::new());
        assert!(t0.elapsed() < Duration::from_millis(200), "took {:?}", t0.elapsed());
        assert_eq!(depth, 11, "ten sequences, plus the one level where the cycle closes");
        assert!(tracks >= 10);
    }

    /// The top bar's Performance button: an icon button named "Performance" that answers a click and
    /// runs `Action::PerfWindow`, which opens / focuses the pane (no headless `App`: the wiring is pinned
    /// on the source, like `quick_export`'s button test).
    #[test]
    fn perf_button_triggers_the_action() {
        let pal = crate::theme::Palette::new(true, egui::Color32::LIGHT_BLUE);
        let id = egui::Id::new("se-perf-button");
        let draw = |ui: &mut egui::Ui| crate::ui::titlebar::panel_toggle_button(ui, &pal, id, tools::Glyph::Perf, "Performance", false);
        assert!(super::super::jobs_pane::tests::click_widget(draw, |r| r.center()));
        let src = include_str!("perf_ctl.rs");
        let body = |f: &str| {
            let s = &src[src.find(f).unwrap()..];
            s[..s.find("\n}\n").unwrap()].to_string()
        };
        let button = body("pub(super) fn button");
        assert!(button.contains("\"Performance\"") && button.contains("app.toggle_panel(Pane::Performance)"));
        assert!(button.contains("let open = app.pane_drawn(Pane::Performance)"), "lit while the pane is on screen");
        assert!(body("pub(super) fn act").contains("open(app)"));
        assert!(body("fn open(app").contains("app.surface(Pane::Performance)"));
        let menus = include_str!("menus.rs");
        assert!(menus.contains("perf_ctl::button(self, ui)"), "the top bar draws the button");
        assert!(menus.contains("Pane::Performance => PerfWindow"), "the Window menu and the Ctrl+P palette row");
        let reg = include_str!("mod.rs");
        assert!(!reg.contains("    perf_ctl::window,") && reg.contains("    perf_ctl::draw,"), "a pane, not a window");
        let window = concat!("egui::", "Window"); // (not the literal: this test is in `src`)
        assert!(!src.contains(window) && !include_str!("../perf_ui.rs").contains(window));
        assert_eq!(Action::from_id("performance"), Some(Action::PerfWindow));
        assert_eq!(Pane::by_title("performance"), Some(Pane::Performance));
    }

    /// `perf.window` / the button reveal the pane: hidden or not, it ends up visible and in front.
    #[test]
    fn opening_reveals_the_pane_on_any_page() {
        for mut l in [Layout::export_layout(), Layout::blank_layout()] {
            l.reveal(Pane::Performance);
            assert!(l.is_visible(Pane::Performance));
        }
    }

    /// The top bar's Performance / Jobs buttons toggle their pane: closed (or a tab behind another) ->
    /// open in front; on screen -> closed. Closed some other way (Window menu, its tab), the next click
    /// opens it again instead of "closing" it twice: the button goes by what is on screen.
    #[test]
    fn the_panel_buttons_toggle_their_pane() {
        for p in [Pane::Performance, Pane::Jobs] {
            for mut l in [Layout::export_layout(), Layout::blank_layout()] {
                l.toggle_shown(p, false);
                assert!(l.is_visible(p), "{p:?} opened");
                l.toggle_shown(p, true);
                assert!(!l.is_visible(p), "{p:?} closed by the second click");
                l.toggle_shown(p, false);
                assert!(l.is_visible(p), "{p:?} opened again");
                l.toggle(p); // the Window menu closes it
                l.toggle_shown(p, false);
                assert!(l.is_visible(p), "{p:?}: a click after closing it elsewhere opens it");
            }
        }
        // on the Export page Jobs and Performance share a tab group: a click on the one behind brings it
        // to the front (it was not on screen) instead of closing it
        let mut l = Layout::export_layout();
        l.reveal(Pane::Jobs);
        assert_eq!(l.stacked_front(Pane::Jobs, Pane::Performance), Some(Pane::Jobs));
        l.toggle_shown(Pane::Performance, false);
        assert_eq!(l.stacked_front(Pane::Jobs, Pane::Performance), Some(Pane::Performance));
        assert!(l.is_visible(Pane::Performance));
    }

    /// Lit (the accent) only while its panel is open; the click still lands either way.
    #[test]
    fn a_panel_button_is_lit_while_its_panel_is_open() {
        let pal = crate::theme::Palette::new(true, egui::Color32::LIGHT_BLUE);
        let id = egui::Id::new("se-perf-button");
        for open in [true, false] {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::theme::test_fonts());
            let out = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    crate::ui::titlebar::panel_toggle_button(ui, &pal, id, tools::Glyph::Perf, "Performance", open);
                });
            });
            let lit = out.shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.fill == pal.accent));
            assert_eq!(lit, open);
            let draw = |ui: &mut egui::Ui| crate::ui::titlebar::panel_toggle_button(ui, &pal, id, tools::Glyph::Perf, "Performance", open);
            assert!(super::super::jobs_pane::tests::click_widget(draw, |r| r.center()));
        }
    }
}
