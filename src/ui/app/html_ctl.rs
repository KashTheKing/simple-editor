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

fn spec_of(p: &Project, h: &HtmlSource) -> html::Spec {
    html::Spec {
        html: h.src.clone(),
        fields: h.field_list(),
        scale: html::scale_for(p.width, p.height),
        fps: if p.fps > 0.0 { p.fps } else { 30.0 },
        duration: h.duration,
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
    let mut pending: Option<String> = None;
    let fps = if app.project.fps > 0.0 { app.project.fps } else { 30.0 };
    for i in 0..app.project.assets.len() {
        let Some(h) = app.project.assets[i].html.clone() else { continue };
        let spec = spec_of(&app.project, &h);
        let a = &mut app.project.assets[i];
        if let Some(r) = html::cached(&spec) {
            let path = r.path.to_string_lossy().into_owned();
            if a.path != path {
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
        app.html.editor.edit(clip, &h.src, h.field_list(), h.duration);
    }
}

pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    let now = Instant::now();
    if let Some(id) = html_ui::take_edit_request() {
        open_editor(app, id);
    }
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

/// "Ask the assistant": pin the clip and send the request with how to edit it.
fn ask_assistant(app: &mut App, text: &str) {
    let Some(clip) = app.html.editor.clip else { return };
    let name = html_of(app, clip).map(|(_, h)| h.name).unwrap_or_default();
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
            html: Some(HtmlSource { name: name.clone(), src: html.to_string(), fields, duration: dur }),
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

    #[test]
    fn unknown_fields_are_refused() {
        let tpl = html::parse(LAYER_STARTER);
        assert_eq!(known_fields(&tpl, &[("title".into(), "Hi".into())]).unwrap().get("title").unwrap(), "Hi");
        let e = known_fields(&tpl, &[("titel".into(), "Hi".into())]).unwrap_err();
        assert!(e.contains("titel") && e.contains("title, subtitle, accent"), "{e}");
    }
}
