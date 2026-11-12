//! ---- ws:html-layer ----
//! HTML layers, App side: render jobs (a finished render swaps the asset's file), the HTML editor
//! window, Add ▸ HTML Layer / Scene.
//!
//! The asset's `html` (source + field values) is the truth; its `path` only follows it. `reconcile`
//! compares every HTML asset with the cache (`engine::html::cached`, a content-addressed file per
//! source + field values + pixel size + fps + duration): a cache hit re-points `path` (a project
//! load, an undo, a finished render), a miss starts a render job. It runs when something may have
//! changed - an undo entry pushed or popped, the project's size / fps / file, a field edit (after
//! a short settle, so typing renders once), a job ending - and every 2 s, never per frame. The
//! re-point is not an undo step of its own: undo restores `html`, and the next reconcile finds the
//! file that belongs to it (still in the cache, renders write new files, never over old ones).
//! While nothing is rendered yet the asset shows `pending.png` (transparent, real, so the library
//! never reports a layer that is still rendering as missing media).

use super::*;
use crate::engine::html;
use crate::model::{Asset, ClipKind, HtmlSource, TrackKind};
use crate::ui::html_ui;
use std::collections::{BTreeMap, HashSet};

/// The Add ▸ HTML Layer starting point: a lower-third style title with fields.
pub(crate) const LAYER_STARTER: &str = r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>Title</title>
<meta name="se:kind" content="layer">
<meta name="se:desc" content="A title that slides in on a coloured bar.">
<meta name="se:tags" content="title, text">
<meta name="se:size" content="1000x240">
<meta name="se:duration" content="4s">
<meta name="se:field" content="title | text | Your Title">
<meta name="se:field" content="subtitle | text | A short line under it">
<meta name="se:field" content="accent | color | #3d7bfd">
<style>
  html, body { margin: 0; background: transparent; overflow: hidden; }
  body { width: 100vw; height: 100vh; display: flex; align-items: center; padding-left: 40px;
    font-family: "Segoe UI", Arial, sans-serif; }
  .box { border-left: 12px solid {{accent}}; padding: 16px 32px; background: rgba(12, 14, 20, .82);
    border-radius: 0 12px 12px 0; box-shadow: 0 10px 30px rgba(0, 0, 0, .35);
    animation: slide .6s cubic-bezier(.2, .9, .3, 1) both; }
  h1 { margin: 0; color: #fff; font-size: 64px; font-weight: 700; }
  p { margin: 4px 0 0; color: #c9cfdc; font-size: 30px; animation: fade .5s ease-out .35s both; }
  @keyframes slide { from { transform: translateX(-110%); opacity: 0; } to { transform: none; opacity: 1; } }
  @keyframes fade { from { opacity: 0; transform: translateY(10px); } to { opacity: 1; transform: none; } }
</style>
</head>
<body><div class="box"><h1>{{title}}</h1><p>{{subtitle}}</p></div></body>
</html>
"#;

/// The Add ▸ HTML Scene starting point: a full frame with its own background.
pub(crate) const SCENE_STARTER: &str = r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>Title Card</title>
<meta name="se:kind" content="scene">
<meta name="se:desc" content="A full-frame title card on a slowly moving gradient.">
<meta name="se:tags" content="scene, title, card">
<meta name="se:size" content="1920x1080">
<meta name="se:duration" content="5s">
<meta name="se:field" content="title | text | Chapter One">
<meta name="se:field" content="from | color | #1b2a6b">
<meta name="se:field" content="to | color | #7a1f5c">
<style>
  html, body { margin: 0; overflow: hidden; }
  body { width: 100vw; height: 100vh; display: grid; place-items: center;
    background: linear-gradient(120deg, {{from}}, {{to}}); background-size: 200% 200%;
    animation: drift 5s ease-in-out both; font-family: "Segoe UI", Arial, sans-serif; }
  h1 { margin: 0; color: #fff; font-size: 140px; font-weight: 800; letter-spacing: 2px;
    text-shadow: 0 8px 40px rgba(0, 0, 0, .45); animation: rise 1s cubic-bezier(.2, .9, .3, 1) .2s both; }
  @keyframes drift { from { background-position: 0% 50%; } to { background-position: 100% 50%; } }
  @keyframes rise { from { opacity: 0; transform: translateY(40px) scale(.96); } to { opacity: 1; transform: none; } }
</style>
</head>
<body><h1>{{title}}</h1></body>
</html>
"#;

/// A field edit renders once typing pauses this long.
const SETTLE: Duration = Duration::from_millis(350);

struct RenderJob {
    base: PathBuf,
    name: String,
    prog: Arc<Progress>,
}

/// Render jobs in flight, the editor window's state.
#[derive(Default)]
pub struct HtmlState {
    jobs: Vec<RenderJob>,
    /// Renders that failed or were cancelled by the user: not retried until the source changes
    /// (another hash) or `html.render` asks.
    failed: HashSet<PathBuf>,
    /// HTML assets whose `path` is not their current render yet.
    stale: HashSet<Id>,
    /// What `reconcile` last saw of the project: (undo len, redo len, top undo time, w, h, fps, file).
    sig: Option<(usize, usize, u64, u32, u32, u64, Option<PathBuf>)>,
    /// Reconcile at this time (a settle after an edit); None = nothing scheduled.
    due: Option<Instant>,
    /// The periodic scan.
    scan_at: Option<Instant>,
    was_exporting: bool,
    pub(crate) editor: html_ui::HtmlUi,
    /// Linked `.html` files: per asset, the hash of the source as last synced and the file's mtime then.
    link: std::collections::HashMap<Id, (Option<u64>, Option<std::time::SystemTime>)>,
    /// The next look at the linked files.
    link_at: Option<Instant>,
    /// The Web Page Snapshot window.
    web: html_ui::WebUi,
    /// A snapshot being taken from an address: its progress and, when done, the saved `.mhtml`.
    web_job: Option<(Arc<Progress>, Arc<Mutex<Option<PathBuf>>>, u32)>,
    /// Downloads: the time the window opened (older files are not ours), files already taken, and size
    /// seen last look (a file still being written grows).
    web_since: Option<std::time::SystemTime>,
    web_seen: HashSet<PathBuf>,
    web_sizes: std::collections::HashMap<PathBuf, u64>,
    web_at: Option<Instant>,
}

impl Drop for HtmlState {
    fn drop(&mut self) {
        html::shutdown(); // the app is going away: close Edge (it is a separate process)
    }
}

impl HtmlState {
    /// Jobs-pane rows.
    pub(super) fn jobs(&self) -> Vec<jobs_pane::ProgJob> {
        self.jobs
            .iter()
            .map(|j| (crate::ui::jobs_ui::JobKind::Media, format!("HTML · {}", j.name), j.prog.clone()))
            .collect()
    }
}

/// `cache_dir()/html/pending.png`: a transparent picture an asset shows until its first render lands.
fn placeholder() -> String {
    let p = html::dir().join("pending.png");
    if !p.is_file() {
        let _ = std::fs::create_dir_all(html::dir());
        let _ = std::fs::write(&p, crate::mcp::png_encode(&media::Frame::new(2, 2)));
    }
    p.to_string_lossy().into_owned()
}

/// The folder of a linked file (its relative links to stylesheets and images resolve there).
pub(super) fn base_of(h: &HtmlSource) -> String {
    if h.file.is_empty() {
        return String::new();
    }
    Path::new(&h.file).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default()
}

fn spec_of(p: &Project, h: &HtmlSource) -> html::Spec {
    html::Spec {
        html: h.src.clone(),
        fields: h.field_list(),
        scale: html::scale_for(p.width, p.height),
        fps: if p.fps > 0.0 { p.fps } else { 30.0 },
        duration: h.duration,
        base: base_of(h),
        params: html::params_of(&h.src, &h.params),
        width: h.width,
        mouse: h.mouse.clone(),
    }
}

/// Compare every HTML asset with the cache: re-point hits, start renders for misses, cancel renders
/// nothing wants any more. Cheap (a parse + hash and a stat or two per HTML asset).
fn reconcile(app: &mut App) {
    let mut changed = false;
    let mut stale = HashSet::new();
    let mut wanted = HashSet::new();
    let mut start: Vec<(PathBuf, String, html::Spec)> = Vec::new();
    let mut kinds: Vec<(Id, ClipKind)> = Vec::new();
    // (asset, size before, size now): a render of another size (a web page as tall as it is)
    let mut resized: Vec<(Id, (u32, u32), (u32, u32))> = Vec::new();
    let mut pending: Option<String> = None;
    let fps = if app.project.fps > 0.0 { app.project.fps } else { 30.0 };
    for i in 0..app.project.assets.len() {
        let Some(h) = app.project.assets[i].html.clone() else { continue };
        let spec = spec_of(&app.project, &h);
        let a = &mut app.project.assets[i];
        if let Some(r) = html::cached(&spec) {
            let path = r.path.to_string_lossy().into_owned();
            if a.path != path {
                if (a.width, a.height) != (r.width, r.height) && a.width > 0 && a.height > 0 {
                    resized.push((a.id, (a.width, a.height), (r.width, r.height)));
                }
                a.path = path;
                a.kind = if r.animated { ClipKind::Video } else { ClipKind::Image };
                (a.width, a.height, a.fps, a.duration) = (r.width, r.height, fps, h.duration);
                a.codec = "png".into();
                kinds.push((a.id, a.kind));
                changed = true;
            }
            continue;
        }
        stale.insert(a.id);
        if !Path::new(&a.path).is_file() {
            // nothing to show yet (a new clip, a project from another PC): transparent, not "missing"
            a.path = pending.get_or_insert_with(placeholder).clone();
            a.kind = ClipKind::Image;
            kinds.push((a.id, a.kind));
            changed = true;
        }
        let (base, _) = html::out_base(&spec);
        wanted.insert(base.clone());
        if !app.html.failed.contains(&base)
            && !app.html.jobs.iter().any(|j| j.base == base)
            && !start.iter().any(|s| s.0 == base)
        {
            start.push((base, h.name.clone(), spec));
        }
    }
    for (aid, kind) in kinds {
        for c in app.project.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
            if c.asset == aid && matches!(c.kind, ClipKind::Video | ClipKind::Image) {
                c.kind = kind;
            }
        }
    }
    // a clip left at its natural size (1 asset pixel = 1 project pixel) keeps it when the picture's size changes
    let (pw, ph) = (app.project.width.max(1) as f64, app.project.height.max(1) as f64);
    let fit = |s: (u32, u32)| (pw / s.0 as f64).min(ph / s.1 as f64);
    for (aid, old, new) in resized {
        for c in app.project.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()) {
            if c.asset == aid && !c.scale.is_animated() && (c.scale.value - 1.0 / fit(old)).abs() < 1e-6 {
                c.scale.value = 1.0 / fit(new);
                changed = true;
            }
        }
    }
    // a render nothing wants any more (typed over, undone): stop it
    app.html.jobs.retain(|j| {
        let keep = wanted.contains(&j.base) || j.prog.is_done();
        if !keep {
            j.prog.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        keep
    });
    for (base, name, spec) in start {
        let prog = export::spawn_job("html-render", move |prog| html::render_to_file(&spec, prog).map(|_| ()));
        app.html.jobs.push(RenderJob { base, name, prog });
    }
    app.html.stale = stale;
    if changed {
        app.player.set_project(&app.project);
    }
}

fn sig(app: &App) -> (usize, usize, u64, u32, u32, u64, Option<PathBuf>) {
    let top = app.undo.last().map_or(0, |e| e.at.to_bits());
    let p = &app.project;
    (app.undo.len(), app.redo.len(), top, p.width, p.height, p.fps.to_bits(), app.project_path.clone())
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::AddHtmlLayer | Action::AddHtmlScene => {
            let src = if a == Action::AddHtmlLayer { LAYER_STARTER } else { SCENE_STARTER };
            let before = app.project.to_json();
            match app.add_html_clip(src, None, None, None, &[]) {
                Ok(id) => {
                    app.push_undo_labeled(
                        before,
                        if a == Action::AddHtmlLayer { "Add HTML layer" } else { "Add HTML scene" },
                    );
                    app.after_edit();
                    app.selection = vec![id];
                    open_editor(app, id);
                }
                Err(e) => app.toast(e),
            }
            true
        }
        Action::EditHtml => {
            match selected_html_clip(app) {
                Some(id) => open_editor(app, id),
                None => app.toast("Select an HTML layer first"),
            }
            true
        }
        Action::SnapshotWeb => {
            let w = &mut app.html.web;
            if !w.open {
                w.open = true;
                w.status.clear();
                if w.width == 0 {
                    w.width = app.project.width.clamp(320, 3840);
                }
                app.html.web_since = Some(std::time::SystemTime::now());
                app.html.web_seen.clear();
                app.html.web_sizes.clear();
            }
            true
        }
        Action::ImportHtml => {
            let pick = rfd::FileDialog::new()
                .add_filter("HTML or saved web page", &["html", "htm", "mhtml", "mht"])
                .set_title("Import an HTML file or a saved web page")
                .pick_file();
            if let Some(p) = pick {
                import_with_undo(app, &p, None);
            }
            true
        }
        Action::SaveHtmlToLibrary => {
            let Some(h) = selected_html_clip(app).and_then(|id| html_of(app, id).map(|(_, h)| h)) else {
                app.toast("Select an HTML layer first");
                return true;
            };
            let id = crate::engine::library::slug(&h.name);
            match crate::engine::library::save(crate::engine::library::Kind::Html, &id, &h.src) {
                Ok(_) => app.toast(format!("Saved \"{}\" to the library (html/{id})", h.name)),
                Err(e) => app.toast(format!("Save to library: {e}")),
            }
            true
        }
        _ => false,
    }
}

/// The first selected clip whose asset is an HTML layer.
fn selected_html_clip(app: &App) -> Option<Id> {
    app.selection.iter().copied().find(|&id| html_of(app, id).is_some())
}

/// (asset id, its HTML) of clip `id`.
pub(super) fn html_of(app: &App, clip: Id) -> Option<(Id, HtmlSource)> {
    let c = app.project.clip(clip)?;
    app.project.asset(c.asset).and_then(|a| a.html.clone().map(|h| (a.id, h)))
}

fn open_editor(app: &mut App, clip: Id) {
    if let Some((_, h)) = html_of(app, clip) {
        app.html.editor.name = h.name.clone();
        app.html.editor.base = base_of(&h);
        app.html.editor.params = html::params_of(&h.src, &h.params);
        app.html.editor.width = h.width;
        app.html.editor.mouse = h.mouse.clone();
        app.html.editor.edit(clip, &h.src, h.field_list(), h.duration);
    }
}

pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    let now = Instant::now();
    if let Some(id) = html_ui::take_edit_request() {
        open_editor(app, id);
    }
    for op in html_ui::take_ops() {
        run_op(app, op);
    }
    sync_files(app, ctx, now);
    web_tick(app, ctx, now);
    if html_ui::take_fields_edited() {
        app.html.due = Some(now + SETTLE);
    }
    let s = sig(app);
    if app.html.sig.as_ref() != Some(&s) {
        app.html.sig = Some(s);
        app.html.due = Some(app.html.due.map_or(now + SETTLE, |d| d.min(now + SETTLE)));
    }
    // finished jobs: a success is picked up by reconcile (cache hit); a failure is said once
    let (done, running): (Vec<RenderJob>, Vec<RenderJob>) =
        std::mem::take(&mut app.html.jobs).into_iter().partition(|j| j.prog.is_done());
    app.html.jobs = running;
    for j in &done {
        if let Some(e) = j.prog.error() {
            app.html.failed.insert(j.base.clone());
            if e != export::CANCELLED {
                app.toast(format!("HTML layer \"{}\" did not render: {e}", j.name));
            }
        }
    }
    let scan = app.html.scan_at.is_none_or(|t| t <= now);
    if !done.is_empty() || scan || app.html.due.is_some_and(|d| d <= now) {
        app.html.due = None;
        app.html.scan_at = Some(now + Duration::from_secs(2));
        reconcile(app);
    }
    // an export started while a layer is still rendering would bake the placeholder / old picture
    let exporting = app.export.is_some();
    if exporting && !app.html.was_exporting && !app.html.stale.is_empty() {
        reconcile(app);
        let used = app.project.used_assets();
        if app.html.stale.iter().any(|id| used.contains(id)) {
            if let Some((p, _)) = &app.export {
                p.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            app.toast(
                "Export stopped: an HTML layer is still rendering. Export again once the Jobs pane shows it done.",
            );
        }
    }
    app.html.was_exporting = exporting;
    // wake only for what can change without a frame of its own
    if !app.html.jobs.is_empty() {
        app.animate_until(ctx, now + Duration::from_millis(250));
    } else if let Some(d) = app.html.due {
        app.animate_until(ctx, d);
    }
}

pub(super) fn windows(app: &mut App, ctx: &egui::Context) {
    let wr = html_ui::show_web(ctx, &mut app.html.web);
    if let Some((url, w)) = wr.capture {
        start_snapshot(app, url, w);
    }
    if wr.import_file {
        act(app, Action::ImportHtml);
    }
    if wr.save_extension {
        save_extension(app);
    }
    let r = html_ui::show(ctx, &mut app.html.editor);
    if r.apply {
        apply_editor(app);
    }
    if let Some(text) = r.ask {
        ask_assistant(app, &text);
    }
    // the window closes this frame: close Edge (a separate process) instead of leaving it to idle out
    if app.close_confirmed && ctx.input(|i| i.viewport().close_requested()) {
        html::shutdown();
    }
}

/// Apply: the editor's source onto the clip's asset, one undo step, then a re-render.
fn apply_editor(app: &mut App) {
    let Some(clip) = app.html.editor.clip else { return };
    let Some((aid, h)) = html_of(app, clip) else {
        app.toast("That HTML clip is gone");
        return;
    };
    let src = app.html.editor.src.clone();
    if src == h.src {
        return;
    }
    let before = app.project.to_json();
    if let Some(x) = app.project.asset_mut(aid).and_then(|a| a.html.as_mut()) {
        x.src = src;
    }
    app.push_undo_labeled(before, "Edit HTML");
    app.after_edit();
    reconcile(app);
}

/// "Ask the assistant" in the editor window.
fn ask_assistant(app: &mut App, text: &str) {
    if let Some(clip) = app.html.editor.clip {
        ask_about(app, clip, text);
    }
}

/// Pin the clip and send the request with how to edit it.
fn ask_about(app: &mut App, clip: Id, text: &str) {    let name = html_of(app, clip).map(|(_, h)| h.name).unwrap_or_default();
    let chip = crate::ui::assistant_data::Chip {
        label: format!("Clip: {}", crate::ui::assistant_data::short(&name, 28)),
        data: json!({"kind": "clips", "clips": [{"id": clip, "name": name, "html": true}]}),
    };
    crate::ui::assistant_data::pin_add(&mut app.assistant.pinned, chip);
    let request = format!(
        "{text}\n\n(This is about the HTML layer clip {clip}. Read it with html.get {{\"clip\": {clip}}}, change it with \
         html.set (the html source or the fields), and look at the result with html.check {{\"clip\": {clip}}} \
         before you finish.)"
    );
    if let Err(e) = assistant_ctl::ask(app, &request) {
        app.toast(e);
    }
}

// ---------------------------------------------------------------------------------------------------
// web page snapshots

/// "From a web address": a private Edge opens the page and saves it as an `.mhtml`; the clip is made
/// when it is done (`web_tick`).
pub(super) fn start_snapshot(app: &mut App, url: String, width: u32) -> bool {
    if app.html.web_job.is_some() {
        return false;
    }
    let slot: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
    let s2 = slot.clone();
    let host: String = url.split("://").last().unwrap_or("").split(['/', '?', '#']).next().unwrap_or("").to_string();
    let prog = export::spawn_job("web-snapshot", move |prog| {
        let bytes = html::capture_mhtml(&url, width, 1080, prog)?;
        let dir = crate::settings::Settings::dir().join("web").join("snapshots");
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let name = crate::engine::library::slug(&host);
        let path = dir.join(format!("{}-{secs}.mhtml", if name.is_empty() { "page" } else { &name }));
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
        Ok(())
    });
    app.html.web.busy = Some(0.0);
    app.html.web.status = "Taking the snapshot…".into();
    app.html.web_job = Some((prog, s2, width));
    true
}

/// MHTML files that appeared in `dir` since `since`, are no longer growing and are not `seen` yet.
fn fresh_snapshots(
    dir: &Path,
    since: std::time::SystemTime,
    seen: &HashSet<PathBuf>,
    sizes: &mut std::collections::HashMap<PathBuf, u64>,
) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        let is = p.extension().is_some_and(|x| x.eq_ignore_ascii_case("mhtml") || x.eq_ignore_ascii_case("mht"));
        let Ok(m) = e.metadata() else { continue };
        let fresh = m.modified().is_ok_and(|t| t >= since - Duration::from_secs(5));
        if !is || !fresh || seen.contains(&p) || m.len() == 0 {
            continue;
        }
        // taken once it has stopped growing (a browser writes it for a moment)
        if sizes.insert(p.clone(), m.len()) == Some(m.len()) {
            out.push(p);
        }
    }
    out
}

fn downloads_dir() -> PathBuf {
    std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("Downloads")).unwrap_or_default()
}

fn web_tick(app: &mut App, ctx: &egui::Context, now: Instant) {
    // a snapshot from an address, in progress or done
    if let Some((prog, slot, width)) = app.html.web_job.clone() {
        app.html.web.busy = Some(prog.fraction());
        if prog.is_done() {
            app.html.web_job = None;
            app.html.web.busy = None;
            let path = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
            match (prog.error(), path) {
                (Some(e), _) if e != export::CANCELLED => app.html.web.status = format!("Snapshot failed: {e}"),
                (None, Some(p)) => {
                    app.html.web.status = "Done: the page is on the timeline.".into();
                    import_with_undo_at(app, &p, None, Some(width));
                }
                _ => {}
            }
        } else {
            app.animate_until(ctx, now + Duration::from_millis(200));
        }
    }
    // pages saved by the user's browser while the window is open
    if app.html.web.open {
        if app.html.web_at.is_none_or(|t| t <= now) {
            app.html.web_at = Some(now + Duration::from_secs(1));
            if let Some(since) = app.html.web_since {
                let (seen, mut sizes) = (app.html.web_seen.clone(), std::mem::take(&mut app.html.web_sizes));
                let found = fresh_snapshots(&downloads_dir(), since, &seen, &mut sizes);
                app.html.web_sizes = sizes;
                for p in found {
                    app.html.web_seen.insert(p.clone());
                    app.html.web.status = format!("Imported {}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    import_with_undo_at(app, &p, None, None);
                }
            }
        }
        app.animate_until(ctx, app.html.web_at.unwrap_or(now) + Duration::from_millis(50));
    } else {
        app.html.web_at = None;
    }
}

/// Write the browser extension's files to `%APPDATA%\SimpleEditor\browser-extension` and show the folder.
fn save_extension(app: &mut App) {
    let dir = crate::settings::Settings::dir().join("browser-extension");
    let files = [
        ("manifest.json", include_str!("../../../tools/browser-extension/manifest.json")),
        ("popup.html", include_str!("../../../tools/browser-extension/popup.html")),
        ("popup.js", include_str!("../../../tools/browser-extension/popup.js")),
    ];
    let r = std::fs::create_dir_all(&dir).and_then(|_| files.iter().try_for_each(|(n, c)| std::fs::write(dir.join(n), c)));
    match r {
        Ok(()) => {
            let _ = std::process::Command::new("explorer").arg(&dir).spawn();
            app.html.web.status = format!("Extension saved to {} - load that folder in your browser's extensions page (Developer mode, Load unpacked).", dir.display());
        }
        Err(e) => app.html.web.status = format!("Could not save the extension: {e}"),
    }
}

// ---------------------------------------------------------------------------------------------------
// files: import, link, sync, open in a code editor

fn hash_of(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn mtime(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

pub(super) fn is_html_file(p: &Path) -> bool {
    p.extension().is_some_and(|e| ["html", "htm", "mhtml", "mht"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

fn is_mhtml(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mhtml") || e.eq_ignore_ascii_case("mht"))
}

/// A `.html` file as a new HTML clip at `at` (the playhead when None). With `link` the file and the
/// clip stay in sync. No undo of its own (a `Mutate` tool wraps it; `import_with_undo` for the UI).
pub(super) fn import_file(app: &mut App, path: &Path, at: Option<f64>, link: bool) -> Result<Id, String> {
    import_file_w(app, path, at, link, None)
}

/// `import_file` with a layout width for an ordinary web page (default: the frame's width).
pub(super) fn import_file_w(app: &mut App, path: &Path, at: Option<f64>, link: bool, width: Option<u32>) -> Result<Id, String> {
    // a page saved as one .mhtml file is unpacked first (index.html + files/), then imported like any page
    let unpacked;
    let path = if is_mhtml(path) {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let stem = crate::engine::library::slug(&path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
        let dir = crate::settings::Settings::dir().join("web").join(format!("{}-{:08x}", if stem.is_empty() { "page" } else { &stem }, hash_of(&path.to_string_lossy()) as u32));
        let _ = std::fs::remove_dir_all(&dir);
        unpacked = crate::engine::webcap::mhtml_to_folder(&data, &dir)?;
        unpacked.as_path()
    } else {
        path
    };
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // a plain page has no <title>: the file name will do
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned());
    let name = if html::parse(&text).title.is_empty() { stem.as_deref() } else { None };
    let id = app.add_html_clip(&text, name, at, None, &[])?;
    if link {
        if let Some((aid, _)) = html_of(app, id) {
            if let Some(h) = app.project.asset_mut(aid).and_then(|a| a.html.as_mut()) {
                h.file = path.to_string_lossy().into_owned();
            }
            // an ordinary web page is laid out at the width of the frame (a Short: a phone-width page)
            if html::parse(&text).fullpage {
                let w = width.unwrap_or(app.project.width).clamp(320, 3840);
                if let Some(h) = app.project.asset_mut(aid).and_then(|a| a.html.as_mut()) {
                    h.width = w;
                }
                reconcile(app);
            }
        }
    }
    Ok(id)
}

pub(super) fn import_with_undo(app: &mut App, path: &Path, at: Option<f64>) {
    import_with_undo_at(app, path, at, None);
}

pub(super) fn import_with_undo_at(app: &mut App, path: &Path, at: Option<f64>, width: Option<u32>) {
    let before = app.project.to_json();
    match import_file_w(app, path, at, true, width) {
        Ok(id) => {
            app.push_undo_labeled(before, "Import HTML file");
            app.after_edit();
            app.selection = vec![id];
            app.toast(if is_mhtml(path) {
                format!("Imported the page {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
            } else {
                format!("Imported {} - edit the file in any code editor and the clip follows", path.display())
            });
        }
        Err(e) => app.toast(format!("Import HTML: {e}")),
    }
}

/// `html.link`: with a path, make it the clip's file - `write` puts the clip's source into it (a new
/// file or an overwrite), otherwise the file's text becomes the source. No path unlinks. No undo of
/// its own.
pub(super) fn link_file(app: &mut App, clip: Id, path: Option<&Path>, write: bool) -> Result<(), String> {
    let (aid, h) = html_of(app, clip).ok_or_else(|| format!("clip {clip} is not an HTML layer"))?;
    let mut new = h.clone();
    match path {
        None => new.file.clear(),
        Some(p) => {
            if write {
                if let Some(d) = p.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                std::fs::write(p, &h.src).map_err(|e| format!("{}: {e}", p.display()))?;
            } else {
                new.src = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
                if new.src.trim().is_empty() {
                    return Err(format!("{} is empty", p.display()));
                }
                let tpl = html::parse(&new.src);
                new.fields.retain(|k, _| tpl.fields.iter().any(|f| f.name == *k));
            }
            new.file = p.to_string_lossy().into_owned();
        }
    }
    if let Some(a) = app.project.asset_mut(aid) {
        a.html = Some(new);
    }
    app.html.link.remove(&aid); // the next look adopts the new pair
    reconcile(app);
    Ok(())
}

fn link_with_undo(app: &mut App, clip: Id, path: Option<&Path>, write: bool, label: &'static str) {
    let before = app.project.to_json();
    match link_file(app, clip, path, write) {
        Ok(()) => {
            app.push_undo_labeled(before, label);
            app.after_edit();
        }
        Err(e) => app.toast(e),
    }
}

/// The file an HTML clip is linked to, made on first use under `%APPDATA%\SimpleEditor\html`.
fn file_for(app: &mut App, clip: Id) -> Result<PathBuf, String> {
    let (aid, h) = html_of(app, clip).ok_or("that HTML clip is gone")?;
    if !h.file.is_empty() && Path::new(&h.file).is_file() {
        return Ok(PathBuf::from(&h.file));
    }
    let dir = crate::settings::Settings::dir().join("html");
    let slug = crate::engine::library::slug(&h.name);
    let slug = if slug.is_empty() { "html-layer".to_string() } else { slug };
    let path = dir.join(format!("{slug}-{aid}.html"));
    link_with_undo(app, clip, Some(&path), true, "Link HTML file");
    Ok(path)
}

/// VS Code when it is installed (or the program in `Settings.html_editor`), else Notepad.
/// ponytail: a path in settings.json, no Settings UI - add a picker if people ask for another editor.
fn open_in_editor(path: &Path, custom: &str) -> String {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default();
    let pf = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_default();
    let exe: Option<(PathBuf, &str)> = if !custom.trim().is_empty() {
        Some((PathBuf::from(custom.trim()), "your code editor"))
    } else {
        [local.join("Programs/Microsoft VS Code/Code.exe"), pf.join("Microsoft VS Code/Code.exe")]
            .into_iter()
            .find(|p| p.is_file())
            .map(|p| (p, "VS Code"))
    };
    let (exe, what) = exe.unwrap_or((PathBuf::from("notepad.exe"), "Notepad"));
    match std::process::Command::new(&exe).arg(path).spawn() {
        Ok(_) => format!("Opened {} in {what} - save it there and the clip updates", path.display()),
        Err(e) => format!("Couldn't start {}: {e}", exe.display()),
    }
}

fn run_op(app: &mut App, op: html_ui::Op) {
    use html_ui::Op;
    match op {
        Op::OpenExternal(clip) => match file_for(app, clip) {
            Ok(p) => {
                let msg = open_in_editor(&p, &app.settings.html_editor.clone());
                app.toast(msg);
            }
            Err(e) => app.toast(format!("Open in code editor: {e}")),
        },
        Op::LinkPick(clip) => {
            let pick =
                rfd::FileDialog::new().add_filter("HTML", &["html", "htm"]).set_title("Link an HTML file").pick_file();
            if let Some(p) = pick {
                link_with_undo(app, clip, Some(&p), false, "Link HTML file");
            }
        }
        Op::SaveAs(clip) => {
            let name = html_of(app, clip).map(|(_, h)| h.name).unwrap_or_default();
            let pick = rfd::FileDialog::new()
                .add_filter("HTML", &["html"])
                .set_file_name(format!("{}.html", crate::engine::library::slug(&name)))
                .save_file();
            if let Some(p) = pick {
                link_with_undo(app, clip, Some(&p), true, "Save HTML as file");
            }
        }
        Op::Unlink(clip) => link_with_undo(app, clip, None, false, "Unlink HTML file"),
        Op::Reveal(clip) => {
            if let Some((_, h)) = html_of(app, clip).filter(|(_, h)| !h.file.is_empty()) {
                let _ = std::process::Command::new("explorer").arg("/select,").arg(&h.file).spawn();
            }
        }
        Op::Ask(clip, text) => ask_about(app, clip, &text),
        Op::Scroll(clip) => scroll_page(app, clip),
    }
}

/// Position Y keys that carry a page taller than the frame from its top edge to its bottom edge over
/// the clip (ease in-out; Position Y grows downward, from the clip's centre).
fn scroll_page(app: &mut App, clip: Id) {
    let (pw, ph) = (app.project.width.max(1) as f64, app.project.height.max(1) as f64);
    let Some(c) = app.project.clip(clip) else { return };
    let Some(a) = app.project.asset(c.asset).filter(|a| a.width > 0 && a.height > 0) else { return };
    // how tall the page is on screen: the asset fitted into the frame, times the clip's scale
    let fit = (pw / a.width as f64).min(ph / a.height as f64);
    let shown = a.height as f64 * fit * c.scale.at(0.0);
    let over = (shown - ph) / 2.0;
    if over <= 1.0 {
        app.toast("The page already fits the frame - nothing to scroll");
        return;
    }
    let dur = c.duration;
    let before = app.project.to_json();
    if let Some(c) = app.project.clip_mut(clip) {
        let mut y = crate::model::Animated::new(over);
        y.keys = vec![
            crate::model::Keyframe { t: 0.0, v: over, ease: crate::model::Ease::EaseInOut },
            crate::model::Keyframe { t: dur, v: -over, ease: crate::model::Ease::Linear },
        ];
        c.y = y;
    }
    app.push_undo_labeled(before, "Scroll page");
    app.after_edit();
    app.toast("Scrolling the page top to bottom over the clip - change the keys on Position Y to speed it up, pause or reverse");
}

const LINK_POLL: Duration = Duration::from_millis(700);

/// Per linked clip: the hash of the source as last synced, and the file's modified time then.
type Link = (Option<u64>, Option<std::time::SystemTime>);

#[derive(Debug, PartialEq)]
enum Step {
    Nothing,
    /// The clip changed: write its source to the file.
    WriteFile,
    /// The file changed: this is its text.
    Reload(String),
}

/// One look at one linked pair. `mtime_now` is the file's modified time; `read` reads its text (only
/// asked when the time moved, or the pair was never seen). Pure, so the rules are testable:
/// the clip changed since the last sync -> the file follows; the file changed -> the clip follows;
/// first sight of a pair -> the file wins (a project just opened); an empty file (a half-finished
/// save) is ignored until the next save.
fn sync_step(st: &mut Link, src: &str, mtime_now: Option<std::time::SystemTime>, read: &mut dyn FnMut() -> Option<String>) -> Step {
    let cur = hash_of(src);
    if st.0.is_some_and(|synced| synced != cur) {
        return Step::WriteFile;
    }
    if st.0.is_none() || mtime_now != st.1 {
        if let Some(text) = read() {
            st.1 = mtime_now;
            if text.trim().is_empty() {
                return Step::Nothing;
            }
            if text != src {
                return Step::Reload(text);
            }
            st.0 = Some(cur);
        }
    }
    Step::Nothing
}

/// Keep linked files and their clips equal. A change made in the app (the Inspector, the editor
/// window, an agent, undo) is written to the file; a change made to the file is read into the clip as
/// one undo step. First sight of a pair (a project just opened) lets the file win. Runs only while a
/// linked clip exists.
fn sync_files(app: &mut App, ctx: &egui::Context, now: Instant) {
    if !app.project.assets.iter().any(|a| a.html.as_ref().is_some_and(|h| !h.file.is_empty())) {
        app.html.link.clear();
        return;
    }
    if let Some(t) = app.html.link_at.filter(|t| *t > now) {
        app.animate_until(ctx, t);
        return;
    }
    app.html.link_at = Some(now + LINK_POLL);
    app.animate_until(ctx, now + LINK_POLL);
    let mut reload: Vec<(Id, String)> = Vec::new();
    for a in &app.project.assets {
        let Some(h) = a.html.as_ref().filter(|h| !h.file.is_empty()) else { continue };
        let path = Path::new(&h.file);
        let st = app.html.link.entry(a.id).or_default();
        match sync_step(st, &h.src, mtime(path), &mut || std::fs::read_to_string(path).ok()) {
            Step::Nothing => {}
            Step::WriteFile => {
                if std::fs::write(path, &h.src).is_ok() {
                    *st = (Some(hash_of(&h.src)), mtime(path));
                }
            }
            Step::Reload(text) => reload.push((a.id, text)),
        }
    }
    for (aid, text) in reload {
        let before = app.project.to_json();
        let tpl = html::parse(&text);
        let mut file_time = None;
        if let Some(h) = app.project.asset_mut(aid).and_then(|a| a.html.as_mut()) {
            h.fields.retain(|k, _| tpl.fields.iter().any(|f| f.name == *k));
            h.src = text.clone();
            file_time = mtime(Path::new(&h.file));
        }
        app.html.link.insert(aid, (Some(hash_of(&text)), file_time));
        app.push_undo_labeled(before, "Reload HTML from file");
        app.after_edit();
        reconcile(app);
    }
}

impl App {
    /// CONTRACT (used by the library's "apply" and by `html.create`): add an HTML layer / scene clip
    /// from template source `html` (the format in `library/README.md`) and start rendering it.
    /// `at` = timeline second (None: the playhead), `duration` (None: the template's `se:duration`),
    /// `fields` = values overriding the template's defaults. Returns the new clip's id. Does NOT push
    /// undo itself: a `ToolKind::Mutate` tool or the caller does.
    pub(crate) fn add_html_clip(
        &mut self,
        html: &str,
        name: Option<&str>,
        at: Option<f64>,
        duration: Option<f64>,
        fields: &[(String, String)],
    ) -> Result<Id, String> {
        if html.trim().is_empty() {
            return Err("empty HTML".into());
        }
        let tpl = html::parse(html);
        let fields = known_fields(&tpl, fields)?;
        let dur = duration.unwrap_or(tpl.duration);
        if !(dur.is_finite() && dur > 0.0) {
            return Err(format!("bad duration {dur}"));
        }
        let dur = dur.clamp(0.1, 3600.0);
        let at = at.unwrap_or(self.playhead).max(0.0);
        let name = name.map(str::trim).filter(|n| !n.is_empty()).map(String::from).unwrap_or_else(|| {
            if tpl.title.is_empty() {
                if tpl.scene { "HTML Scene" } else { "HTML Layer" }.into()
            } else {
                tpl.title.clone()
            }
        });
        let p = &mut self.project;
        let (pw, ph) = (p.width.max(2), p.height.max(2));
        let (w, h) = html::pixel_size(tpl.size, html::scale_for(pw, ph));
        let aid = p.new_id();
        // pushed directly, not `add_asset`: that de-duplicates by path, and every HTML asset is its own
        p.assets.push(Asset {
            id: aid,
            path: placeholder(),
            kind: ClipKind::Image, // until the render says whether it moves
            duration: dur,
            width: w,
            height: h,
            fps: p.fps,
            audio_streams: Vec::new(),
            codec: "png".into(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
            html: Some(HtmlSource { name: name.clone(), src: html.to_string(), fields, duration: dur, ..Default::default() }),
        });
        // on top, like a text clip: a layer sits over the footage
        let prefer = p.video_tracks().last().copied();
        let ti = p.find_free_track(TrackKind::Video, at, dur, prefer);
        let mut c = crate::model::Clip::new(p.new_id(), ClipKind::Image, name, at, dur);
        c.asset = aid;
        // natural size: scale 1.0 fits the asset inside the frame, so undo that fit (a 640x220
        // button stays 640x220 of a 1080p frame; a 1920x1080 layer fills a 16:9 frame)
        let fit = (pw as f64 / w as f64).min(ph as f64 / h as f64);
        if fit.is_finite() && fit > 0.0 {
            c.scale.value = 1.0 / fit;
        }
        let id = c.id;
        p.tracks[ti].clips.push(c);
        p.tracks[ti].sort();
        reconcile(self); // the render starts now (no frame may follow to start it)
        Ok(id)
    }
}

/// `fields` restricted to the template's declared names; an unknown name is an error.
pub(super) fn known_fields(
    tpl: &html::Template,
    fields: &[(String, String)],
) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for (k, v) in fields {
        if !tpl.fields.iter().any(|f| f.name == *k) {
            let names: Vec<&str> = tpl.fields.iter().map(|f| f.name.as_str()).collect();
            return Err(format!(
                "unknown field '{k}' (this template has: {})",
                if names.is_empty() { "none".into() } else { names.join(", ") }
            ));
        }
        out.insert(k.clone(), v.clone());
    }
    Ok(out)
}

/// `html.render`: forget a failure for this clip's asset and render it again if it is not cached.
pub(super) fn rerender(app: &mut App, clip: Id) -> Result<(), String> {
    let (_, h) = html_of(app, clip).ok_or_else(|| format!("clip {clip} is not an HTML layer"))?;
    let (base, _) = html::out_base(&spec_of(&app.project, &h));
    app.html.failed.remove(&base);
    reconcile(app);
    Ok(())
}

/// Is clip `clip`'s render done (its asset shows its current source)?
pub(super) fn rendered(app: &App, clip: Id) -> bool {
    html_of(app, clip).is_some_and(|(aid, _)| !app.html.stale.contains(&aid))
}

/// The render scale and the spec of a clip, for `html.check`.
pub(super) fn spec_of_clip(app: &App, clip: Id) -> Option<html::Spec> {
    html_of(app, clip).map(|(_, h)| spec_of(&app.project, &h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starters_follow_the_library_rules() {
        for s in [LAYER_STARTER, SCENE_STARTER] {
            assert!(html::lint(s).is_empty(), "{:?}", html::lint(s));
        }
        assert!(!html::parse(LAYER_STARTER).scene && html::parse(SCENE_STARTER).scene);
    }

    fn at(secs: u64) -> Option<std::time::SystemTime> {
        Some(std::time::UNIX_EPOCH + Duration::from_secs(secs))
    }

    /// The sync rules, one case each (no files, no App: the decision is a pure function).
    #[test]
    fn linked_files_and_clips_follow_each_other() {
        let mut never = || -> Option<String> { panic!("the file must not be read when nothing moved") };
        // in sync, file untouched: nothing read, nothing done
        let mut st: Link = (Some(hash_of("a")), at(1));
        assert_eq!(sync_step(&mut st, "a", at(1), &mut never), Step::Nothing);
        // saved in a code editor: the clip follows
        let mut st: Link = (Some(hash_of("a")), at(1));
        assert_eq!(sync_step(&mut st, "a", at(2), &mut || Some("b".into())), Step::Reload("b".into()));
        // edited in the app (or by an agent, or undone): the file follows, and the file is not read
        let mut st: Link = (Some(hash_of("a")), at(1));
        assert_eq!(sync_step(&mut st, "c", at(1), &mut never), Step::WriteFile);
        // a half-written (empty) file is ignored
        let mut st: Link = (Some(hash_of("a")), at(1));
        assert_eq!(sync_step(&mut st, "a", at(2), &mut || Some("  \n".into())), Step::Nothing);
        // first sight, file differs: the file wins
        let mut st: Link = (None, None);
        assert_eq!(sync_step(&mut st, "old", at(5), &mut || Some("new".into())), Step::Reload("new".into()));
        // first sight, same text: adopted, so the next edit in the app is a WriteFile
        let mut st: Link = (None, None);
        assert_eq!(sync_step(&mut st, "same", at(5), &mut || Some("same".into())), Step::Nothing);
        assert_eq!(st.0, Some(hash_of("same")));
        assert_eq!(sync_step(&mut st, "edited", at(5), &mut never), Step::WriteFile);
        // the file is gone: nothing happens (the clip keeps its source)
        let mut st: Link = (Some(hash_of("a")), at(1));
        assert_eq!(sync_step(&mut st, "a", None, &mut || None), Step::Nothing);
    }

    #[test]
    fn html_files_are_recognised() {
        assert!(is_html_file(Path::new("C:/x/Card.HTML")) && is_html_file(Path::new("a.htm")));
        assert!(!is_html_file(Path::new("a.mp4")) && !is_html_file(Path::new("html")));
    }

    #[test]
    fn unknown_fields_are_refused() {
        let tpl = html::parse(LAYER_STARTER);
        assert_eq!(known_fields(&tpl, &[("title".into(), "Hi".into())]).unwrap().get("title").unwrap(), "Hi");
        let e = known_fields(&tpl, &[("titel".into(), "Hi".into())]).unwrap_err();
        assert!(e.contains("titel") && e.contains("title, subtitle, accent"), "{e}");
    }
}
