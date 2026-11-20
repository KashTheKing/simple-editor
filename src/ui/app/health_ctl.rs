//! ---- ws:forum-preview ----
//! The App side of playback health and phone footage (`media::playcheck` holds the logic):
//!
//! * `tick` (FRAME_HOOKS): pushes any playback setting a player has not been given (`reconcile`), samples
//!   the timeline player's frame counters twice a second while playing into the health `Meter`, works out
//!   the one fix on Amber (`advise`), and once per imported variable-frame-rate file says that its timing
//!   was fixed.
//! * `chip` (from the preview pane): the green dot / amber chip over the picture, and its clicks.
//! * `playback.health` (MCP): the same, as JSON.
//!
//! The proxy proposal for heavy footage at IMPORT time belongs to mixed-footage's card (`footage_ctl`);
//! this one is the measured one: it appears when playback actually drops frames, names the source that is
//! slow to decode, and says how big its proxy will be.

use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::media::playcheck::{self, Advice, Candidate, Level, Meter};
use crate::playback::Applied;
use std::collections::HashSet;

#[derive(Default)]
pub(crate) struct State {
    meter: Meter,
    /// Seconds are measured from here.
    epoch: Option<Instant>,
    next_sample: Option<Instant>,
    advice: Option<Advice>,
    /// The fix last computed at; recomputed every 2 s while Amber and playing (decode numbers come in).
    advised_at: Option<Instant>,
    /// The chip was closed: hidden until the meter goes Green and then Amber again.
    dismissed: bool,
    /// Imported VFR files already announced (lowercased paths).
    told: HashSet<String>,
    next_vfr_scan: Option<Instant>,
}

// ---- settings reconcile ----

/// What a player should be running with.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Want {
    pub backend: Backend,
    pub gpu: bool,
    pub cache_bytes: usize,
}

/// The setters to call so `applied` matches `want` (None = already right).
pub(crate) fn pushes(applied: &Applied, want: &Want) -> (Option<Backend>, Option<bool>, Option<usize>) {
    (
        (applied.backend != Some(want.backend)).then_some(want.backend),
        (applied.gpu != Some(want.gpu)).then_some(want.gpu),
        (applied.cache_bytes != Some(want.cache_bytes)).then_some(want.cache_bytes),
    )
}

fn apply_to(p: &mut Player, want: &Want) {
    let (b, g, c) = pushes(p.applied(), want);
    if let Some(b) = b {
        p.set_backend(b);
    }
    if let Some(g) = g {
        p.set_gpu(g);
    }
    if let Some(c) = c {
        p.set_cache_bytes(c);
    }
}

impl App {
    /// The timeline player's wanted settings; the Source monitor's gets a share of the cache
    /// (`gpu::configure`).
    pub(super) fn playback_want(&self, source: bool) -> Want {
        let cache = crate::playback::cache_budget_bytes(self.settings.cache_mb);
        Want {
            backend: self.backend(),
            gpu: self.gpu.is_some(),
            cache_bytes: if source { cache / super::gpu::SOURCE_CACHE_DIV } else { cache },
        }
    }
}

/// Every frame: a player that has not been given a setting gets it now - at start-up, after the Source
/// monitor opens, after a change from any path. Comparing three small values is all it costs.
pub(super) fn reconcile(app: &mut App) {
    let (main, source) = (app.playback_want(false), app.playback_want(true));
    apply_to(&mut app.player, &main);
    if let Some(s) = app.source.as_mut() {
        apply_to(&mut s.player, &source);
    }
}

// ---- health ----

fn file_name(p: &str) -> String {
    Path::new(p).file_name().map_or_else(|| p.to_string(), |n| n.to_string_lossy().into_owned())
}

/// The video sources showing at the playhead, weighed for `advise`.
fn candidates(app: &App) -> Vec<Candidate> {
    let p = &app.project;
    let t = app.playhead;
    let ph = app.settings.proxy_height.max(120);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for tr in p.tracks.iter().filter(|t| t.kind == TrackKind::Video) {
        for c in tr.clips.iter().filter(|c| c.enabled && c.start <= t && t < c.start + c.duration) {
            let Some(a) = p.asset(c.asset).filter(|a| a.kind == ClipKind::Video && a.html.is_none()) else { continue };
            if !seen.insert(a.id) {
                continue;
            }
            use crate::media::proxy::{status, ProxyStatus};
            let st = status(a, app.settings.use_proxies, ph);
            let settled = a.proxy == Some(false)
                || (app.settings.use_proxies && matches!(st, ProxyStatus::Ready | ProxyStatus::Queued | ProxyStatus::Building(_)));
            let measured_ms =
                crate::perf::source_stat(&a.path).filter(|s| s.n > 0).map(|s| s.us as f32 / s.n as f32 / 1000.0);
            out.push(Candidate {
                asset: a.id,
                name: file_name(&a.path),
                score: playcheck::cost_score(&a.codec, a.width, a.height, a.fps, 0.0, false),
                measured_ms,
                settled,
                bytes: playcheck::proxy_bytes(a.width, a.height, a.fps, a.duration, ph),
            });
        }
    }
    out
}

fn advise_now(app: &App) -> Advice {
    let frame_ms = 1000.0 / app.project.fps.max(1.0) as f32;
    playcheck::advise(&candidates(app), frame_ms, app.settings.preview_quality, &crate::ui::preview::QUALITIES)
}

/// (chip reason, fix label, fix tooltip) for `advice`.
fn words(advice: &Advice, quality: u32, proxy_h: u32) -> (String, Option<(&'static str, String)>) {
    match advice {
        Advice::Proxy { name, bytes, .. } => (
            format!("Playback is dropping frames: {name} is too heavy to decode in real time on this PC"),
            Some((
                "Make proxy",
                format!(
                    "Builds a {proxy_h}p edit copy of {name} in the background (about {}). Playback uses it; export still uses the original",
                    playcheck::fmt_bytes(*bytes)
                ),
            )),
        ),
        Advice::Quality(q) => (
            format!("Playback is dropping frames at {quality} % resolution"),
            Some(("Lower resolution", format!("Plays at {q} % resolution. Export is not affected"))),
        ),
        Advice::Open => {
            ("Playback is dropping frames. Click to see what costs the most".to_string(), None)
        }
    }
}

pub(super) fn tick(app: &mut App, _ctx: &egui::Context) {
    reconcile(app);
    let now = Instant::now();
    let epoch = *app.health.epoch.get_or_insert(now);
    let playing = app.player.is_playing();
    if app.health.next_sample.is_none_or(|t| now >= t) {
        app.health.next_sample = Some(now + Duration::from_millis(500));
        let before = app.health.meter.level();
        let (published, dropped) = (app.player.published_frames(), app.player.dropped_frames());
        let level = app.health.meter.sample((now - epoch).as_secs_f64(), published, dropped, playing);
        match level {
            Level::Green => {
                app.health.advice = None;
                app.health.dismissed = false;
            }
            Level::Amber => {
                if before != Level::Amber {
                    // measure what each source costs to decode while the user is still playing, so the
                    // advice can name the slow one (the Performance pane's sampling, for 15 s)
                    app.perf.report_until = Some(now + Duration::from_secs(15));
                }
                let stale = app.health.advised_at.is_none_or(|t| now - t > Duration::from_secs(2));
                if app.health.advice.is_none() || (playing && stale) {
                    app.health.advice = Some(advise_now(app));
                    app.health.advised_at = Some(now);
                }
            }
        }
    }
    if !playing && app.health.next_vfr_scan.is_none_or(|t| now >= t) {
        app.health.next_vfr_scan = Some(now + Duration::from_secs(1));
        announce_vfr(app);
    }
}

/// Say once per file that a freshly imported phone / screen recording plays from an even-rate proxy.
fn announce_vfr(app: &mut App) {
    if !app.settings.use_proxies {
        return; // nothing is conformed with proxies off: claim nothing
    }
    let mut names = Vec::new();
    for a in app.project.assets.iter().filter(|a| a.kind == ClipKind::Video && a.html.is_none()) {
        let key = a.path.to_ascii_lowercase();
        if app.health.told.contains(&key) || !playcheck::announced(&a.path) || !playcheck::is_vfr(&a.path) {
            continue;
        }
        app.health.told.insert(key);
        names.push(file_name(&a.path));
    }
    match names.len() {
        0 => {}
        1 => app.toast(format!("Fixed phone footage timing: {}", names[0])),
        n => app.toast(format!("Fixed phone footage timing: {n} clips")),
    }
}

/// From the preview pane, after the picture: the dot / chip over `app.preview.canvas_rect`.
pub(super) fn chip(app: &mut App, ui: &mut egui::Ui) {
    let level = app.health.meter.level();
    let amber = level == Level::Amber && !app.health.dismissed;
    let playing = app.player.is_playing();
    if !amber && !playing {
        return;
    }
    let advice = app.health.advice.clone().unwrap_or(Advice::Open);
    let ph = app.settings.proxy_height.max(120).min(1080);
    let (reason, fix) = words(&advice, app.settings.preview_quality, ph);
    let perf_open = app.pane_drawn(Pane::Performance);
    let out = crate::ui::health_chip::draw(
        ui,
        app.preview.canvas_rect,
        &crate::ui::health_chip::Chip {
            amber,
            playing,
            reason: &reason,
            fix: fix.as_ref().map(|(l, t)| (*l, t.as_str())),
            perf_open,
        },
    );
    if out.toggle_perf {
        app.perf.tab = crate::ui::perf_ui::Tab::Project;
        app.toggle_panel(Pane::Performance);
    }
    if out.dismiss {
        app.health.dismissed = true;
    }
    if out.fix {
        apply_fix(app, &advice);
    }
}

fn apply_fix(app: &mut App, advice: &Advice) {
    match advice {
        Advice::Proxy { asset, .. } => super::perf_ctl::make_proxy(app, *asset),
        Advice::Quality(q) => {
            app.settings.preview_quality = *q;
            app.settings.save();
            app.toast(format!("Playback resolution {q} %"));
        }
        Advice::Open => {
            app.surface(Pane::Performance);
            return;
        }
    }
    app.health.meter.reset();
    app.health.advice = None;
}

// ---- MCP ----

fn advice_json(a: &Advice) -> Value {
    match a {
        Advice::Proxy { asset, name, bytes } => json!({"fix": "proxy", "asset_id": asset, "name": name, "proxy_bytes": bytes}),
        Advice::Quality(q) => json!({"fix": "quality", "to": q}),
        Advice::Open => json!({"fix": "none"}),
    }
}

fn report(app: &mut App, args: &Value) -> Result<Value, String> {
    let level = app.health.meter.level();
    let advice = advise_now(app);
    let applied = if args.get("fix").and_then(Value::as_bool).unwrap_or(false) {
        apply_fix(app, &advice);
        true
    } else {
        false
    };
    let vfr: Vec<Value> = app
        .project
        .assets
        .iter()
        .filter(|a| a.kind == ClipKind::Video)
        .filter_map(|a| playcheck::traits(&a.path).map(|t| (a, t)))
        .filter(|(_, t)| t.vfr)
        .map(|(a, t)| json!({"asset_id": a.id, "name": file_name(&a.path), "conform_fps": t.fps}))
        .collect();
    Ok(json!({
        "level": if level == Level::Amber { "amber" } else { "green" },
        "dropped_frames": app.player.dropped_frames(),
        "advice": advice_json(&advice),
        "fix_applied": applied,
        "vfr_sources": vfr,
        "sources_at_playhead": candidates(app).iter().map(|c| json!({
            "asset_id": c.asset, "name": c.name, "cost": c.score, "decode_ms": c.measured_ms,
            "settled": c.settled, "proxy_bytes": c.bytes,
        })).collect::<Vec<_>>(),
    }))
}

pub const TOOLS: &[ToolDef] = &[ToolDef {
    name: "playback.health",
    desc: "Playback health as the viewer's chip shows it: `level` green | amber (frames dropped in the last few seconds of play), the one `advice` (a proxy for the source that is slow to decode, with its estimated size, or a lower playback resolution), the video sources at the playhead with their decode cost, and the variable-frame-rate sources that play from a constant-rate proxy. `fix: true` applies the advice.",
    args: &["fix:boolean:false:apply the advice"],
    kind: ToolKind::Ui,
    run: |app, args| report(app, args).map(ToolOutcome::Done),
}];

#[cfg(test)]
mod tests {
    use super::*;

    /// A player made later, or a setting changed by any path, is brought in line: only what differs is
    /// pushed, and nothing once it matches (the stale-state class from forum thread 50776).
    #[test]
    fn reconcile_pushes_only_what_a_player_lacks() {
        let want = Want { backend: Backend::Auto, gpu: true, cache_bytes: 512 << 20 };
        // a fresh player knows only its backend
        let fresh = Applied { backend: Some(Backend::Auto), ..Default::default() };
        assert_eq!(pushes(&fresh, &want), (None, Some(true), Some(512 << 20)));
        let done = Applied { backend: Some(Backend::Auto), gpu: Some(true), cache_bytes: Some(512 << 20) };
        assert_eq!(pushes(&done, &want), (None, None, None), "settled: nothing sent, no re-render");
        // the decoder changed in Settings: only the backend goes
        let want2 = Want { backend: Backend::Ffmpeg, ..want.clone() };
        assert_eq!(pushes(&done, &want2), (Some(Backend::Ffmpeg), None, None));
    }

    /// A real player: after one reconcile pass it reports exactly the wanted settings, and a second pass
    /// finds nothing to send - so settings are applied on open, not on the first change.
    #[test]
    fn a_new_player_gets_its_settings_on_the_first_pass() {
        let mut p = Player::new(
            egui::Context::default(),
            Backend::Auto,
            Arc::new(Mutex::new(crate::engine::text::TextRasterizer::new())),
        );
        let want = Want { backend: Backend::Mf, gpu: false, cache_bytes: 64 << 20 };
        apply_to(&mut p, &want);
        assert_eq!(p.applied(), &Applied { backend: Some(Backend::Mf), gpu: Some(false), cache_bytes: Some(64 << 20) });
        assert_eq!(pushes(p.applied(), &want), (None, None, None));
    }

    /// The tick is registered, runs `reconcile` first, and the Source monitor is covered too.
    #[test]
    fn reconcile_runs_every_frame_for_both_players() {
        let src = include_str!("health_ctl.rs");
        let tick = &src[src.find("pub(super) fn tick").unwrap()..];
        assert!(tick[..200].contains("reconcile(app);"));
        let rec = &src[src.find("pub(super) fn reconcile").unwrap()..];
        assert!(rec.contains("apply_to(&mut app.player") && rec.contains("apply_to(&mut s.player"));
        assert!(include_str!("mod.rs").contains("    health_ctl::tick,"));
    }

    #[test]
    fn words_say_what_and_how_big() {
        let (why, fix) = words(&Advice::Proxy { asset: 1, name: "IMG_0042.MOV".into(), bytes: 1_240_000_000 }, 100, 720);
        assert!(why.contains("IMG_0042.MOV"));
        let (label, tip) = fix.unwrap();
        assert_eq!(label, "Make proxy", "the label is just the name");
        assert!(tip.contains("1.2 GB") && tip.contains("720p"), "{tip}");
        let (_, fix) = words(&Advice::Quality(50), 75, 720);
        assert_eq!(fix.unwrap().0, "Lower resolution");
        assert!(words(&Advice::Open, 25, 720).1.is_none());
    }

    /// Undo on a 3-hour, 3000-clip timeline: the snapshot taken on an edit and the project restored by
    /// Undo, under 200 ms together (Session D's budget). Ignored: a bench, run by hand in a dev build.
    #[test]
    #[ignore]
    fn bench_undo_three_hour_timeline() {
        let mut p = Project::new();
        let aid = p.add_asset(crate::model::Asset {
            kind: ClipKind::Video,
            duration: 3.0 * 3600.0,
            width: 1920,
            height: 1080,
            fps: 30.0,
            ..crate::engine::import::placeholder("C:/long.mkv")
        });
        for i in 0..3000 {
            let mut c = crate::model::Clip::new(p.new_id(), ClipKind::Video, "C:/long.mkv", i as f64 * 3.6, 3.6);
            c.asset = aid;
            c.src_in = i as f64 * 3.6;
            p.tracks[0].clips.push(c);
        }
        let mut worst = Duration::ZERO;
        for _ in 0..10 {
            let t0 = Instant::now();
            let snap = Snap::of(p.clone()); // the commit path: a clone, never serialised
            let back = snap.project().expect("restores");
            let dt = t0.elapsed();
            assert_eq!(back.tracks[0].clips.len(), 3000);
            worst = worst.max(dt);
        }
        eprintln!("undo on a 3 h / 3000-clip timeline: worst {worst:?}");
        assert!(worst < Duration::from_millis(200), "undo took {worst:?}");
    }
}
