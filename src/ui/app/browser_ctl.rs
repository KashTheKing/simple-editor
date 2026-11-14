//! ---- ws:browser-layer ----
//! The app side of browser clips (`model::browser`, `engine::browser`): the Add / Record / Waypoint
//! actions, the recorder that turns the real mouse over the viewer into cursor keys, and the
//! `browser.*` / `cursor.*` tools (seven, most with an `action`).
//!
//! Record mouse: the timeline plays, the pointer over the viewer is sampled every frame (and every
//! pointer event in between, so faster than the display), with the button and the wheel. On stop
//! the take is reduced to a few keys and click / drag / wheel events (`model::reduce_take`) and
//! written over the span it covers only, as one undo step. Nothing here runs when not recording.

use super::tools_args::Args;
use super::tools_helpers::*;
use super::*;
use crate::engine::browser as br;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::engine::export::Progress;
use crate::engine::webpage::{self, PageMap};
use crate::model::ops::browser::{CursorMove, NextPage, PageTransition};
use std::sync::{Arc, Mutex};
use crate::model::{reduce_take, BrowserStyle, BrowserTheme, CursorEventKind, CursorSample, CursorShape};
use std::time::Instant;

struct Recording {
    clip: Id,
    samples: Vec<CursorSample>,
    started: Instant,
    /// Timeline time the take began at.
    t0: f64,
    last_t: f64,
    /// Project JSON before the take touched anything (its undo point).
    before: String,
    /// The browser's scroll before the take wrote its live keys.
    scroll: crate::model::Animated,
}

/// What a background job hands back: the edit to make on the UI thread, and its toast.
type Apply = Box<dyn FnOnce(&mut App) -> Result<String, String> + Send>;

/// Work that waits on Edge (a page import, zoom details): done on a thread, applied by `tick`.
struct Job {
    prog: Arc<Progress>,
    slot: Arc<Mutex<Option<Apply>>>,
    label: &'static str,
    /// Housekeeping the user did not ask for (zoom details after an edit): no undo step, no toast.
    quiet: bool,
}

/// The Import Web Page window.
#[derive(Default)]
struct ImportWin {
    open: bool,
    src: String,
    signed_in: bool,
}

#[derive(Default)]
pub(super) struct BrowserCtl {
    rec: Option<Recording>,
    /// Waypoint mode: clicks on the viewer add cursor keys to this clip.
    waypoints: Option<Id>,
    jobs: Vec<Job>,
    /// What the last finished job said (`browser.import` action jobs), and the page asset the last import made.
    last: String,
    last_asset: Id,
    import: ImportWin,
    /// (browser, page clip) pairs to fit as pages once the page's first render says how tall it is.
    fit_due: Vec<(Id, Id)>,
    /// The undo stack as last seen: an edit since then may have changed a zoom.
    seen: (usize, usize),
    /// When to look whether zoom details need drawing again (a moment after the last edit).
    sharpen_at: Option<Instant>,
    /// Per browser clip: `sharp_state` when its zoom details were last made.
    sharp: std::collections::HashMap<Id, u64>,
}

impl BrowserCtl {
    /// Does the tick have anything to do? (Idle: no input read, no repaint.)
    fn busy(&self) -> bool {
        self.rec.is_some() || self.waypoints.is_some() || !self.jobs.is_empty() || !self.fit_due.is_empty() || self.sharpen_at.is_some()
    }
}

/// (Boxed, so the thread wrapper is compiled once and not once per kind of job: 30 KB.)
type Work = Box<dyn FnOnce(&Progress) -> Result<Apply, String> + Send>;

fn spawn(app: &mut App, label: &'static str, quiet: bool, work: Work) {
    let slot: Arc<Mutex<Option<Apply>>> = Arc::new(Mutex::new(None));
    let s2 = slot.clone();
    let prog = crate::engine::export::spawn_job("browser-layer", move |p| {
        let apply = work(p)?;
        *s2.lock().unwrap_or_else(|e| e.into_inner()) = Some(apply);
        Ok(())
    });
    app.browser.jobs.push(Job { prog, slot, label, quiet });
}

/// Finished jobs make their edit (one undo step each) and say what they did.
#[inline(never)]
fn finish_jobs(app: &mut App) {
    let mut i = 0;
    while i < app.browser.jobs.len() {
        if !app.browser.jobs[i].prog.is_done() {
            i += 1;
            continue;
        }
        let job = app.browser.jobs.remove(i);
        let apply = job.slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        let before = app.project.to_json();
        let said = match (job.prog.error(), apply) {
            (Some(e), _) => Err(e),
            (None, Some(f)) => f(app),
            (None, None) => Err("nothing came back".into()),
        };
        let msg = match said {
            Ok(m) => {
                // (a job may change nothing in the project: Sharpen zooms fills an internal cache)
                if !job.quiet && app.project.to_json() != before {
                    app.push_undo_labeled(before, job.label);
                }
                app.after_edit();
                app.browser.seen = (app.undo.len(), app.redo.len());
                if job.quiet {
                    continue;
                }
                m
            }
            Err(_) if job.quiet => continue,
            Err(e) if e.contains("exited at start") => {
                format!("{}: Edge would not start - close the sign-in window (or any Edge using that profile) and try again", job.label)
            }
            Err(e) => format!("{}: {e}", job.label),
        };
        app.browser.last = msg.clone();
        app.toast(msg);
    }
}

// ---------- real pages ----------

/// The map of a page asset: beside its `.html` file for an HTML layer, beside the picture otherwise.
fn page_map(a: &crate::model::Asset) -> Option<PageMap> {
    PageMap::of(a.html.as_ref().map(|h| h.file.as_str()).filter(|f| !f.is_empty()).unwrap_or(&a.path))
}

/// The page clip under browser `id` at timeline `t`, with its map: the topmost clip there that has one.
fn page_under(app: &App, id: Id, t: f64) -> Option<(Id, PageMap)> {
    app.project.browser_pages(id).into_iter().rev().find_map(|cid| {
        let c = app.project.clip(cid).filter(|c| c.contains(t) && c.enabled)?;
        Some((cid, page_map(app.project.asset(c.asset)?)?))
    })
}

/// Import a page (an address or a saved file) on a thread; when it is in, it becomes an HTML layer
/// and, with `place`, a page clip at `at` under a browser layer (a new one if none is there).
fn start_import(app: &mut App, src: String, width: u32, at: f64, place: bool, signed_in: bool) {
    spawn(app, "Import web page", false, Box::new(move |prog: &Progress| {
        let profile = &crate::engine::html::CAPTURE_PROFILE;
        if signed_in {
            *profile.lock().unwrap_or_else(|e| e.into_inner()) = Some(webpage::sign_in_profile().join("p"));
        }
        let r = webpage::import(&src, width, prog);
        *profile.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let index = r?;
        Ok(Box::new(move |app: &mut App| place_page(app, &index, width, at, place)) as Apply)
    }));
}

#[inline(never)]
fn place_page(app: &mut App, index: &std::path::Path, width: u32, at: f64, place: bool) -> Result<String, String> {
    let map = PageMap::of(&index.to_string_lossy()).ok_or("the page map is missing")?;
    // an ordinary linked HTML layer clip: the HTML engine draws it from here on
    let cid = html_ctl::import_file_w(app, index, Some(at), true, Some(width))?;
    let aid = app.project.clip(cid).map(|c| c.asset).ok_or("could not place the page")?;
    app.browser.last_asset = aid;
    let what = if map.title.is_empty() { "the page".to_string() } else { format!("\"{}\"", map.title) };
    // said loudly: a page whose pictures did not arrive looks broken in a video
    let what = match map.missing {
        0 => what,
        n => format!("{what} - WARNING: {n} pictures had not loaded when it was saved (grey placeholders); import it again, or save the page from your own browser -"),
    };
    if !place {
        app.project.delete_clips(&[cid], false);
        return Ok(format!("Imported {what} into the library (asset {aid}, {} things to click)", map.elements.len()));
    }
    let existing =
        app.project.all_clips().filter(|(_, c)| c.kind == ClipKind::Browser && c.contains(at)).last().map(|(ti, c)| (ti, c.id, c.end()));
    let dur = existing.map_or(10.0, |(_, _, end)| (end - at).max(1.0));
    // onto the lowest free video track (under the browser when there is one)
    let from = app.project.track_of(cid).ok_or("could not place the page")?;
    let pos = app.project.tracks[from].clips.iter().position(|c| c.id == cid).ok_or("could not place the page")?;
    let mut clip = app.project.tracks[from].clips.remove(pos);
    let ti = app.project.video_tracks().into_iter().find(|&ti| app.project.tracks[ti].fits(at, dur, &[]));
    let ti = match (ti, existing) {
        (Some(ti), Some((bt, _, _))) if ti >= bt => return Err("no free track under the browser here - use Click and go to page for a next page".into()),
        (Some(ti), _) => ti,
        (None, _) => return Err("no free video track here".into()),
    };
    (clip.start, clip.duration) = (at, dur);
    app.project.tracks[ti].clips.push(clip);
    app.project.tracks[ti].sort();
    let bid = match existing {
        Some((_, id, _)) => id,
        None => app.project.add_browser_clip(at, dur),
    };
    let b = app.project.clip_mut(bid).and_then(|c| c.browser.as_mut()).ok_or("no browser")?;
    if existing.is_none() {
        if !map.url.is_empty() {
            b.url = map.url.clone();
        }
        if let Some(tab) = b.tabs.first_mut() {
            (tab.title, tab.favicon) = (map.title.clone(), map.favicon.clone());
        }
    }
    if b.auto_fit {
        app.browser.fit_due.push((bid, cid)); // once its first render says how tall the page is
    }
    app.selection = vec![bid];
    Ok(format!("Imported {what} as an HTML layer: {} things the cursor can go to by name", map.elements.len()))
}

/// "Make editable": headings, labels and short texts of the page under browser `id` at `t` become
/// fields of its HTML layer (`webpage::make_editable`), on a thread.
fn start_make_editable(app: &mut App, id: Id, t: f64, max: usize) -> Result<(), String> {
    let page = app.project.browser_pages(id).into_iter().rev().find(|&c| {
        app.project.clip(c).is_some_and(|c| c.contains(t)) && html_ctl::html_of(app, c).is_some()
    });
    let (aid, h) = page.and_then(|c| html_ctl::html_of(app, c)).ok_or("the page under the browser here is not an HTML layer")?;
    let (base, width) = (html_ctl::base_of(&h), if h.width > 0 { h.width } else { app.project.width });
    spawn(app, "Make editable", false, Box::new(move |_: &Progress| {
        let (src, n) = webpage::make_editable(&h.src, &base, width, max)?;
        Ok(Box::new(move |app: &mut App| {
            let a = app.project.asset_mut(aid).and_then(|a| a.html.as_mut()).ok_or("the page is gone")?;
            if !a.file.is_empty() {
                // the clip and its file follow each other: both get the new source at once
                std::fs::write(&a.file, &src).map_err(|e| format!("{}: {e}", a.file))?;
            }
            a.src = src;
            Ok(format!("{n} fields: select the page clip and edit them under Fields in the Inspector"))
        }) as Apply)
    }));
    Ok(())
}

/// The box of what `target` names on the page under browser `id` at timeline `t`: (page clip, box
/// in project px from the frame's middle before scroll and zoom, its text).
fn element_box(app: &App, id: Id, t: f64, target: &str, nth: usize) -> Result<(Id, (f64, f64, f64, f64), String), String> {
    let (page, map) = page_under(app, id, t).ok_or("the page under the browser here has no element map (import it with Import Web Page, or keep a .json beside the picture)")?;
    let e = map.find(target, nth).ok_or_else(|| format!("nothing called \"{target}\" on this page ({} elements)", map.elements.len()))?;
    let at = |x: f64, y: f64| app.project.page_to_frame(page, t, (x / map.width, y / map.height));
    let (a, z) = (at(e.x, e.y).ok_or("the page clip has no size")?, at(e.x + e.w, e.y + e.h).ok_or("the page clip has no size")?);
    Ok((page, (a.0, a.1, z.0, z.1), e.text.clone()))
}

/// The fake mouse goes to the page point `p` (before scroll and zoom), arriving at timeline `t`
/// after `secs`. When the page is not zoomed and the point is out of sight, the page scrolls to it
/// first. Returns where the cursor ends up (shown position).
fn move_to(app: &mut App, id: Id, t: f64, secs: f64, p: (f64, f64)) -> Result<(f64, f64), String> {
    let ph = app.project.height as f64;
    let pw = app.project.width as f64;
    let c = app.project.clip_mut(id).ok_or("no clip")?;
    let lt = t - c.start;
    let b = c.browser.as_mut().ok_or("no browser")?;
    let (vw, vh) = b.viewport();
    let view_h = vh as f64 * pw / vw as f64;
    let top = b.scroll.at(lt) - ph / 2.0;
    if (b.zoom.at(lt) - 1.0).abs() < 1e-6 && !(top + 40.0..top + view_h - 40.0).contains(&p.1) {
        let want = (p.1 + ph / 2.0 - view_h * 0.45).max(0.0);
        b.glide(false, (lt - secs - 0.5).max(0.0), 0.5, want);
    }
    let shown = b.shown(lt, p);
    app.project.cursor_move_to(id, lt, secs, shown);
    Ok(shown)
}

/// Arguments of "Click and go to page".
struct ClickGo<'a> {
    /// What to click: an element's name, or a page point.
    target: Option<&'a str>,
    nth: usize,
    point: Option<(f64, f64)>,
    /// Timeline time of the click.
    at: f64,
    next: NextPage<'a>,
}

/// The whole move in one edit: the mouse reaches the target, turns into a hand, the target lights
/// up, the click, and the page changes (`NextPage`). Returns (clips after the cut, what was clicked).
fn click_go(app: &mut App, id: Id, g: &ClickGo) -> Result<(Vec<Id>, String), String> {
    let start = app.project.clip(id).ok_or("no clip")?.start;
    let arrive = g.at - 0.3;
    let (rect, name) = match (g.target, g.point) {
        (Some(t), _) => {
            let (_, r, name) = element_box(app, id, arrive, t, g.nth)?;
            (Some(r), name)
        }
        (None, Some(_)) => (None, String::new()),
        (None, None) => return Err("pass 'target' (an element's text) or 'x' and 'y'".into()),
    };
    let p = rect.map(|r| ((r.0 + r.2) / 2.0, (r.1 + r.3) / 2.0)).or(g.point).unwrap_or((0.0, 0.0));
    move_to(app, id, arrive, 0.9, p)?;
    let lt = g.at - start;
    app.project.cursor_event(id, arrive - start - 0.05, CursorEventKind::Shape(CursorShape::Hand));
    if let Some(r) = rect {
        app.project.browser_add_highlight(id, arrive - start - 0.1, lt + 0.12, r);
    }
    app.project.cursor_event(id, lt, CursorEventKind::Click);
    // defaults for the next page come from its own map
    let map = g.next.asset.and_then(|a| app.project.asset(a)).and_then(|a| PageMap::of(&a.path)).unwrap_or_default();
    let pick = |given: &str, own: &String| if given.is_empty() { own.clone() } else { given.to_string() };
    let (url, title, favicon) = (pick(g.next.url, &map.url), pick(g.next.title, &map.title), pick(g.next.favicon, &map.favicon));
    let next = NextPage { url: &url, title: &title, favicon: &favicon, ..g.next.clone() };
    let ids = app.project.browser_page_cut(id, g.at + 0.12, &next);
    if ids.is_empty() {
        return Err("nothing under the browser to cut at the click".into());
    }
    app.project.cursor_event(id, lt + 0.14, CursorEventKind::Shape(CursorShape::Arrow));
    Ok((ids, name))
}

/// One zoomed part to draw: its span, where it goes on the frame, and what `webpage::render_region` needs.
struct Detail {
    from: f64,
    to: f64,
    frame: (f64, f64, f64, f64),
    src: String,
    fields: Vec<(String, String)>,
    base: String,
    width: u32,
    css: [f64; 4],
    scale: f64,
}

/// The zoomed parts of browser `id` that need a sharper picture than the page clip holds: for every
/// zoom level it passes through (`engine::browser::zoom_levels` - moves included, so no frame of a
/// zoom shows a picture enlarged), the part seen, to be drawn by the HTML engine at that scale. Only HTML pages (a plain picture has nothing to be drawn from).
#[inline(never)]
fn details_wanted(app: &App, id: Id) -> Vec<Detail> {
    let Some(clip) = app.project.clip(id) else { return Vec::new() };
    let start = clip.start;
    let mut todo = Vec::new();
    for r in br::zoom_levels(clip, app.project.width, app.project.height) {
        let t = start + (r.from + r.to) / 2.0;
        let Some((page, map)) = page_under(app, id, t) else { continue };
        let Some((aid, h)) = html_ctl::html_of(app, page) else { continue };
        let (Some(a), Some(z)) = (app.project.frame_to_page(page, t, (r.rect.0, r.rect.1)), app.project.frame_to_page(page, t, (r.rect.2, r.rect.3))) else { continue };
        // whole CSS px, inside the page
        let (x0, y0) = ((a.0 * map.width).floor().max(0.0), (a.1 * map.height).floor().max(0.0));
        let (x1, y1) = ((z.0 * map.width).ceil().min(map.width), (z.1 * map.height).ceil().min(map.height));
        if x1 - x0 < 2.0 || y1 - y0 < 2.0 {
            continue;
        }
        let at = |x: f64, y: f64| app.project.page_to_frame(page, t, (x / map.width, y / map.height));
        let (Some(fa), Some(fz)) = (at(x0, y0), at(x1, y1)) else { continue };
        let scale = (fz.0 - fa.0) / (x1 - x0) * r.scale;
        let have = app.project.asset(aid).map_or(1.0, |a| a.width as f64 / map.width);
        if scale <= have * 1.02 {
            continue; // the page's own render is that sharp already
        }
        let width = if h.width > 0 { h.width } else { map.width as u32 };
        todo.push(Detail {
            from: r.from,
            to: r.to,
            frame: (fa.0, fa.1, fz.0, fz.1),
            fields: h.field_list(),
            base: html_ctl::base_of(&h),
            src: h.src,
            width,
            css: [x0, y0, x1 - x0, y1 - y0],
            scale,
        });
    }
    todo
}

/// What the zoom details of browser `id` depend on, and the ones that are there: when this changes,
/// they are drawn again.
fn sharp_state(app: &App, id: Id, todo: &[Detail]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for d in todo {
        for v in [d.from, d.to, d.frame.0, d.frame.1, d.frame.2, d.frame.3, d.css[0], d.css[1], d.css[2], d.css[3], d.scale] {
            v.to_bits().hash(&mut h);
        }
        (&d.fields, d.width, &d.src).hash(&mut h);
    }
    for d in br::zoom_details(id) {
        (d.from.to_bits(), d.to.to_bits(), &d.path).hash(&mut h);
    }
    h.finish()
}

/// Draw the zoom details of browser `id` (see `details_wanted`) on a thread and lay them over the
/// page as "Zoom detail" clips. `quiet`: housekeeping after an edit - nothing happens when they are
/// up to date, and it is no undo step. Returns how many are being drawn.
fn start_sharpen(app: &mut App, id: Id, quiet: bool) -> usize {
    let todo = details_wanted(app, id);
    let state = sharp_state(app, id, &todo);
    if quiet && app.browser.sharp.get(&id) == Some(&state) {
        return 0;
    }
    let n = todo.len();
    if n == 0 {
        br::set_zoom_details(id, Vec::new());
        app.player.set_project(&app.project);
        app.browser.sharp.insert(id, sharp_state(app, id, &[]));
        return 0;
    }
    let budget = app.settings.page_zoom_megapixels.clamp(1.0, 64.0) as f64 * 1.0e6;
    spawn(app, "Sharpen zooms", quiet, Box::new(move |prog: &Progress| {
        let mut made = Vec::new();
        for (i, d) in todo.iter().enumerate() {
            prog.set(i as f32 / n as f32, "Drawing zoomed parts of the page…");
            made.push((d.from, d.to, d.frame, webpage::render_region(&d.src, &d.fields, &d.base, d.width, d.css, d.scale, budget)?));
        }
        Ok(Box::new(move |app: &mut App| {
            // internal: drawn over the page by the player / export (`browser::render_view`), never project
            // clips or Library assets
            let mut list = Vec::new();
            for (from, to, frame, path) in &made {
                let head = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
                let size = crate::engine::html::png_size(&head).ok_or("not a PNG picture")?;
                list.push(br::ZoomDetail { from: *from, to: *to, rect: *frame, path: path.to_string_lossy().into_owned(), size });
            }
            br::set_zoom_details(id, list);
            app.player.set_project(&app.project);
            let now = sharp_state(app, id, &details_wanted(app, id));
            app.browser.sharp.insert(id, now);
            Ok(format!("{} zoomed part(s) of the page drawn at full sharpness", made.len()))
        }) as Apply)
    }));
    n
}

/// Remove the "Zoom detail" picture clips earlier builds laid under browser layers, and their pictures
/// in the Library when nothing else uses them: zooms are drawn from the HTML now. Returns how many clips.
pub(super) fn remove_legacy_details(app: &mut App) -> usize {
    let legacy = |c: &Clip| c.name == crate::model::ops::browser::DETAIL && c.kind == ClipKind::Image;
    let assets: Vec<Id> = app.project.all_clips().filter(|(_, c)| legacy(c)).map(|(_, c)| c.asset).collect();
    if assets.is_empty() {
        return 0;
    }
    let browsers: Vec<Id> = app.project.all_clips().filter(|(_, c)| c.kind == ClipKind::Browser).map(|(_, c)| c.id).collect();
    let n = browsers.into_iter().map(|b| app.project.browser_clear_details(b)).sum();
    for a in assets {
        if !app.project.all_clips().any(|(_, c)| c.asset == a) {
            app.project.remove_asset(a);
        }
    }
    n
}

/// A project was opened: zoom details belong to it (drawn again a moment later), and the picture
/// clips older builds made for them are removed (one undo step).
pub(super) fn project_loaded(app: &mut App) {
    br::clear_zoom_details();
    app.browser.sharp.clear();
    let before = app.project.to_json();
    let n = remove_legacy_details(app);
    if n > 0 {
        app.push_undo_labeled(before, "Remove old zoom detail clips");
        app.toast(format!("Removed {n} old 'Zoom detail' picture clip(s): zooms are drawn from the HTML page now"));
    }
    if app.project.all_clips().any(|(_, c)| c.kind == ClipKind::Browser) {
        app.browser.sharpen_at = Some(Instant::now() + std::time::Duration::from_millis(300));
    }
}

/// Seconds the playhead moves on after a waypoint click, so the next click is the next stop.
const WAYPOINT_STEP: f64 = 0.6;

/// The browser clip the user means: the selected one, else the topmost one under the playhead.
fn target(app: &App) -> Option<Id> {
    let is = |c: &Clip| c.kind == ClipKind::Browser;
    app.selection.iter().copied().find(|&id| app.project.clip(id).is_some_and(is)).or_else(|| {
        let t = app.playhead;
        app.project.all_clips().filter(|(_, c)| is(c) && c.contains(t)).last().map(|(_, c)| c.id)
    })
}

fn need(app: &mut App) -> Option<Id> {
    let id = target(app);
    if id.is_none() {
        app.toast("Select a browser layer first (Clip ▸ Add ▸ Browser Window)");
    }
    id
}

/// The picture's rectangle in the viewer, in points (zoom and pan applied).
fn picture(app: &App, ctx: &egui::Context) -> egui::Rect {
    let aspect = app.project.width.max(1) as f32 / app.project.height.max(1) as f32;
    let fit = crate::ui::preview::letterbox(app.preview.canvas_rect, aspect, ctx.pixels_per_point());
    crate::ui::preview::apply_view(fit, app.preview.view)
}

/// Screen point -> cursor position on browser `id`'s page at timeline `t`.
fn to_page(app: &App, id: Id, t: f64, lb: egui::Rect, p: egui::Pos2) -> Option<(f64, f64)> {
    let (pw, ph) = (app.project.width, app.project.height);
    let plan = br::plan(pw, app.project.clip(id)?, t, pw, ph)?;
    let k = pw as f32 / lb.width().max(1.0);
    Some(plan.canvas_to_page(((p.x - lb.min.x) * k, (p.y - lb.min.y) * k)))
}

fn start_recording(app: &mut App, id: Id) {
    let Some(c) = app.project.clip(id) else { return };
    let (start, end) = (c.start, c.end());
    let scroll = c.browser.as_ref().map(|b| b.scroll.clone()).unwrap_or_else(|| crate::model::Animated::new(0.0));
    if !(start..end - 0.05).contains(&app.playhead) {
        app.seek(start);
    }
    let before = app.project.to_json();
    // the old pointer would play back under the real one: hidden for the take
    if let Some(cur) = app.project.clip_mut(id).and_then(|c| c.browser.as_mut()).map(|b| b.cursor.get_or_insert_with(Default::default)) {
        cur.visible = false;
    }
    app.after_edit();
    app.browser.waypoints = None;
    app.browser.rec =
        Some(Recording { clip: id, samples: Vec::new(), started: Instant::now(), t0: app.playhead, last_t: app.playhead, before, scroll });
    app.selection = vec![id];
    if !app.player.is_playing() {
        app.player.toggle();
    }
}

/// End the take: `keep` writes it (one undo step), otherwise the project is as it was.
fn stop_recording(app: &mut App, keep: bool) -> Option<(usize, usize)> {
    let rec = app.browser.rec.take()?;
    if app.player.is_playing() {
        app.player.toggle();
    }
    let done = (|| {
        let c = app.project.clip(rec.clip)?;
        let b = c.browser.as_ref()?;
        let (from, to) = (rec.samples.first()?.t, rec.samples.last()?.t);
        let smoothing = b.cursor.as_ref().map_or(0.35, |c| c.smoothing) as f64;
        // scroll keys written live during the take are raw; the reduced ones replace them
        let take = reduce_take(&rec.samples, smoothing, rec.scroll.at(from));
        let n = (take.keys.len(), take.events.len());
        (keep && app.project.cursor_apply_take(rec.clip, &take, from, to)).then_some(n)
    })();
    match done {
        Some(_) => {
            if let Some(cur) = app.project.clip_mut(rec.clip).and_then(|c| c.browser.as_mut()).and_then(|b| b.cursor.as_mut()) {
                cur.visible = true;
            }
            app.push_undo_labeled(rec.before, "Record mouse");
        }
        None => {
            if let Ok(p) = Project::from_json(&rec.before) {
                app.project = p;
            }
        }
    }
    app.after_edit();
    done
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::AddBrowser => {
            app.push_undo();
            let at = app.playhead;
            let rest = app.project.duration() - at;
            let id = app.project.add_browser_clip(at, if rest > 1.0 { rest } else { 5.0 });
            // the pictures below become pages straight away (Inspector ▸ Browser turns this off)
            app.project.browser_fit_pages(id);
            app.selection = vec![id];
            app.after_edit();
            app.toast("Browser added: everything on the tracks below it shows inside the window, fitted as pages");
        }
        Action::BrowserFitPages => {
            let Some(id) = need(app) else { return true };
            app.push_undo();
            let max = app.project.browser_fit_pages(id);
            app.after_edit();
            app.toast(format!("Pages fitted to the window's width - Scroll can go to {max:.0}"));
        }
        Action::BrowserNextPage => {
            let Some(id) = need(app) else { return true };
            app.push_undo();
            let next = NextPage { load: 0.6, transition: PageTransition::Fade, secs: 0.35, ..Default::default() };
            let new = app.project.browser_page_cut(id, app.playhead, &next);
            app.after_edit();
            app.toast(if new.is_empty() {
                "Nothing under the browser to cut here"
            } else {
                "Page cut here with a load bar - replace the clip after the cut with the next page"
            });
        }
        Action::CursorPress => {
            let Some(id) = need(app) else { return true };
            let before = app.project.to_json();
            if app.project.cursor_press(id, app.playhead).is_some() {
                app.push_undo_labeled(before, "Button press");
                app.after_edit();
            } else {
                app.toast("No clip under the cursor here (a full-page picture is skipped)");
            }
        }
        Action::RecordMouse => {
            if app.browser.rec.is_some() {
                if let Some((keys, events)) = stop_recording(app, true) {
                    app.toast_undo(format!("Recorded: {keys} keys, {events} clicks / events"), Action::Undo);
                }
            } else if let Some(id) = need(app) {
                start_recording(app, id);
            }
        }
        Action::ImportWebPage => app.browser.import.open = true,
        Action::SharpenZooms => {
            let Some(id) = need(app) else { return true };
            let before = app.project.to_json();
            if remove_legacy_details(app) > 0 {
                app.push_undo_labeled(before, "Remove old zoom detail clips");
                app.after_edit();
            }
            match start_sharpen(app, id, false) {
                0 => app.toast("Nothing to sharpen: no zoom goes past what the page holds (or the page is a plain picture)"),
                n => app.toast(format!("Drawing {n} zoomed part(s) of the page…")),
            }
        }
        Action::PageMakeEditable => {
            let Some(id) = need(app) else { return true };
            match start_make_editable(app, id, app.playhead, 24) {
                Ok(()) => app.toast("Finding the page's headings and labels…"),
                Err(e) => app.toast(e),
            }
        }
        Action::CursorWaypoints => {
            if app.browser.waypoints.take().is_none() {
                if let Some(id) = need(app) {
                    stop_recording(app, false);
                    app.browser.waypoints = Some(id);
                    app.selection = vec![id];
                }
            }
        }
        _ => return false,
    }
    true
}

/// Recorder / waypoint overlay. Only while one of them is on: an idle app gets nothing from here.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    if let Some(r) = crate::ui::browser_ui::take_request() {
        run_request(app, r);
    }
    // an edit may have changed a zoom: a moment later the zoom details are checked (and only then)
    let seen = (app.undo.len(), app.redo.len());
    if seen != app.browser.seen {
        app.browser.seen = seen;
        if app.project.all_clips().any(|(_, c)| c.kind == ClipKind::Browser) {
            app.browser.sharpen_at = Some(Instant::now() + std::time::Duration::from_millis(700));
        }
    }
    if !app.browser.busy() {
        return;
    }
    if !app.browser.jobs.is_empty() {
        finish_jobs(app);
    }
    // pages waiting for their first render: fitted once the page's real height is known
    let ready: Vec<Id> = app.browser.fit_due.iter().filter(|(_, c)| html_ctl::rendered(app, *c)).map(|(b, _)| *b).collect();
    if !ready.is_empty() {
        app.browser.fit_due.retain(|(b, c)| !ready.contains(b) && app.project.clip(*c).is_some());
        for b in ready {
            app.project.browser_fit_pages(b);
        }
        app.after_edit();
    }
    app.browser.fit_due.retain(|(b, c)| app.project.clip(*b).is_some() && app.project.clip(*c).is_some());
    if let Some(at) = app.browser.sharpen_at {
        let idle = app.browser.jobs.is_empty() && app.browser.fit_due.is_empty() && !ctx.input(|i| i.pointer.any_down());
        if Instant::now() >= at && idle {
            app.browser.sharpen_at = None;
            let ids: Vec<Id> = app.project.all_clips().filter(|(_, c)| c.kind == ClipKind::Browser).map(|(_, c)| c.id).collect();
            for id in ids {
                start_sharpen(app, id, true);
            }
        }
    }
    if !app.browser.jobs.is_empty() || !app.browser.fit_due.is_empty() || app.browser.sharpen_at.is_some() {
        app.animate_until(ctx, Instant::now() + std::time::Duration::from_millis(200));
    }
    if app.browser.rec.is_none() && app.browser.waypoints.is_none() {
        return;
    }
    let lb = picture(app, ctx);
    let area = app.preview.canvas_rect;
    if !area.is_positive() {
        return;
    }
    // an invisible sheet over the viewer takes the clicks, so a take never moves a clip
    let mut stop = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let mut clicked: Option<(egui::Pos2, bool)> = None;
    let recording = app.browser.rec.is_some();
    egui::Area::new(egui::Id::new("browser_layer_sheet")).order(egui::Order::Foreground).fixed_pos(area.min).show(ctx, |ui| {
        let (rect, resp) = ui.allocate_exact_size(area.size(), egui::Sense::click_and_drag());
        let label = if recording {
            "● Recording the mouse - Esc or click here to stop"
        } else {
            "Click to place the cursor (Shift: with a click) - Esc or click here when done"
        };
        let pos = rect.left_top() + egui::vec2(10.0, 10.0);
        let color = if recording { egui::Color32::from_rgb(230, 60, 60) } else { egui::Color32::from_rgb(60, 130, 230) };
        let galley = ui.painter().layout_no_wrap(label.into(), egui::FontId::proportional(13.0), egui::Color32::WHITE);
        let badge = egui::Rect::from_min_size(pos, galley.size() + egui::vec2(16.0, 10.0));
        ui.painter().rect_filled(badge, 6.0, color);
        ui.painter().galley(badge.min + egui::vec2(8.0, 5.0), galley, egui::Color32::WHITE);
        if let Some(p) = resp.interact_pointer_pos().filter(|_| resp.clicked()) {
            if badge.contains(p) {
                stop = true;
            } else {
                clicked = Some((p, ui.input(|i| i.modifiers.shift)));
            }
        }
        // waypoints placed so far
        if let (false, Some(id)) = (recording, app.browser.waypoints) {
            let (pw, ph) = (app.project.width, app.project.height);
            let c = app.project.clip(id);
            let plan = c.and_then(|c| br::plan(pw, c, app.playhead, pw, ph));
            let cur = c.and_then(|c| c.browser.as_ref()).and_then(|b| b.cursor.as_ref());
            if let (Some(plan), Some(cur)) = (plan, cur) {
                let k = lb.width() / pw.max(1) as f32;
                let pts: Vec<egui::Pos2> = cur
                    .x
                    .keys
                    .iter()
                    .map(|key| {
                        let p = plan.page_to_canvas(cur.pos(key.t));
                        lb.min + egui::vec2(p.0 * k, p.1 * k)
                    })
                    .collect();
                ui.painter().add(egui::Shape::line(pts.clone(), egui::Stroke::new(1.5, color)));
                for p in pts {
                    ui.painter().circle_filled(p, 3.5, color);
                }
            }
        }
    });
    if recording {
        ctx.request_repaint(); // sampling: every frame, only while a take runs
        record_frame(app, ctx, lb, area);
        let over = app.browser.rec.as_ref().is_some_and(|r| app.project.clip(r.clip).is_none_or(|c| r.last_t >= c.end() - 1e-3));
        if stop || over {
            act(app, Action::RecordMouse);
        }
        return;
    }
    let Some(id) = app.browser.waypoints else { return };
    if stop || app.project.clip(id).is_none() {
        app.browser.waypoints = None;
        return;
    }
    if let Some((p, with_click)) = clicked {
        let t = app.playhead;
        let Some(c) = app.project.clip(id).filter(|c| c.contains(t)) else {
            app.toast("Move the playhead onto the browser layer");
            return;
        };
        let lt = c.local(t);
        let Some((x, y)) = to_page(app, id, t, lb, p) else { return };
        app.push_undo();
        app.project.cursor_set_path(id, &[(lt, x, y)], CursorMove::Stops);
        if with_click {
            app.project.cursor_event(id, lt, CursorEventKind::Click);
        }
        app.after_edit();
        app.seek(t + WAYPOINT_STEP);
    }
}

/// What the Inspector's buttons ask for (`browser_ui::Request`): edits that need the whole project.
#[inline(never)]
fn run_request(app: &mut App, r: crate::ui::browser_ui::Request) {
    use crate::ui::browser_ui::Request;
    let Some(id) = need(app) else { return };
    let t = app.playhead;
    let before = app.project.to_json();
    let done: Result<(&'static str, String), String> = match r {
        Request::ZoomAtCursor(z) => (|| {
            let at = cursor_point(app, id, t, true);
            let c = app.project.clip_mut(id).ok_or("no clip")?;
            let lt = t - c.start;
            let b = c.browser.as_mut().ok_or("no browser")?;
            b.window_zoom_to(lt, 0.7, z, if z > 1.0 { at } else { None });
            Ok(("Window zoom", String::new()))
        })(),
        Request::MoveTo(target) => element_box(app, id, t, &target, 0).and_then(|(_, r, name)| {
            move_to(app, id, t, 0.9, ((r.0 + r.2) / 2.0, (r.1 + r.3) / 2.0))?;
            Ok(("Move the mouse", format!("The mouse goes to \"{name}\"")))
        }),
        Request::ClickGo { target, page, transition } => {
            let next = NextPage {
                asset: Some(page),
                load: 0.7,
                transition: PageTransition::from_name(&transition).unwrap_or(PageTransition::Fade),
                secs: 0.4,
                ..Default::default()
            };
            click_go(app, id, &ClickGo { target: Some(&target), nth: 0, point: None, at: t, next })
                .map(|(_, name)| ("Click and go to page", format!("Clicked \"{name}\" and went to the next page")))
        }
    };
    match done {
        Ok((label, msg)) => {
            app.push_undo_labeled(before, label);
            app.after_edit();
            if !msg.is_empty() {
                app.toast(msg);
            }
        }
        Err(e) => {
            if let Ok(p) = Project::from_json(&before) {
                app.project = p; // a half-made move is no move
            }
            app.toast(e);
        }
    }
}

/// The Import Web Page window.
pub(super) fn windows(app: &mut App, ctx: &egui::Context) {
    if !app.browser.import.open {
        return;
    }
    let mut open = true;
    let (mut go, mut sign_in) = (false, false);
    let busy = !app.browser.jobs.is_empty();
    egui::Window::new("Import Web Page").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
        let w = &mut app.browser.import;
        ui.label("A page address, or a page saved as a single file (.mhtml) or .html:");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut w.src).desired_width(360.0).hint_text("https://www.example.com/page"));
            if ui.button("File…").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("Web page", &["mhtml", "mht", "html", "htm"]).pick_file() {
                    w.src = p.to_string_lossy().into_owned();
                }
            }
        });
        ui.checkbox(&mut w.signed_in, "As me (use the sign-in profile)");
        ui.horizontal(|ui| {
            go = ui.add_enabled(!busy && !w.src.trim().is_empty(), egui::Button::new("Import")).clicked();
            sign_in = ui.button("Sign in…").on_hover_text("Opens a browser window of its own for you to sign in to the site").clicked();
            if busy {
                ui.spinner();
            }
        });
        ui.weak(
            "The page is saved to this computer with its pictures, styles and fonts; its scripts are removed and nothing \
             is fetched again later. It becomes a page under a browser layer, with a map of its links and buttons.\n\
             Sign in opens Microsoft Edge with a profile kept only for Simple Editor: you sign in there yourself. Simple \
             Editor never asks for, reads or stores your password. A page imported as you shows your own view of the \
             site (your name, your lists): it stays on this computer unless you share the project or the video.",
        );
    });
    if sign_in {
        let w = &app.browser.import;
        let url = if w.src.contains("://") { w.src.trim().to_string() } else { String::new() };
        if let Err(e) = webpage::open_sign_in(&url) {
            app.toast(e);
        }
    }
    if go {
        let w = &app.browser.import;
        let (src, signed) = (w.src.trim().to_string(), w.signed_in);
        let (width, at) = (app.project.width.clamp(320, 3840), app.playhead);
        start_import(app, src, width, at, true, signed);
        app.toast("Importing the page…");
        open = false;
    }
    app.browser.import.open = open;
}

/// One frame of a take: the pointer events since the last frame become samples.
#[inline(never)]
fn record_frame(app: &mut App, ctx: &egui::Context, lb: egui::Rect, area: egui::Rect) {
    let Some(rec) = app.browser.rec.as_ref() else { return };
    let id = rec.clip;
    let t = if app.player.is_playing() { app.playhead } else { rec.t0 + rec.started.elapsed().as_secs_f64() };
    let (moves, latest, down, wheel) = ctx.input(|i| {
        let moves: Vec<egui::Pos2> =
            i.events.iter().filter_map(|e| if let egui::Event::PointerMoved(p) = e { Some(*p) } else { None }).collect();
        (moves, i.pointer.latest_pos(), i.pointer.primary_down(), i.raw_scroll_delta.y)
    });
    let Some(latest) = latest.filter(|p| area.contains(*p)) else { return };
    let Some(start) = app.project.clip(id).map(|c| c.start) else { return };
    let moves = if moves.is_empty() { vec![latest] } else { moves };
    let last_t = rec.last_t.min(t);
    // one wheel notch is about 50 points; a notch scrolls about 100 project px
    let wheel = -wheel as f64 * 2.0;
    let n = moves.len();
    let mut new = Vec::with_capacity(n);
    for (i, p) in moves.into_iter().enumerate() {
        let st = last_t + (t - last_t) * (i + 1) as f64 / n as f64;
        if let Some((x, y)) = to_page(app, id, st, lb, p) {
            let last = i + 1 == n;
            new.push(CursorSample { t: st - start, x, y, down, wheel: if last { wheel } else { 0.0 } });
        }
    }
    if wheel != 0.0 {
        // live: the page scrolls while recording (raw held keys; the take's eased ones replace them)
        if let Some(b) = app.project.clip_mut(id).and_then(|c| c.browser.as_mut()) {
            let lt = t - start;
            let v = (b.scroll.at(lt) + wheel).max(0.0);
            crate::model::put_key(&mut b.scroll, lt, v, crate::model::Ease::Hold);
        }
        app.after_edit();
    }
    if let Some(rec) = app.browser.rec.as_mut() {
        rec.samples.extend(new);
        rec.last_t = t;
    }
}

// ---------- tools ----------

fn done(v: Value) -> Result<ToolOutcome, String> {
    Ok(ToolOutcome::Done(v))
}

/// The tool's browser clip: `clip`, else the selected / topmost one.
fn clip_arg(app: &App, args: &Value) -> Result<Id, String> {
    let id = Args(args).id("clip").or_else(|| target(app)).ok_or("no browser layer: pass 'clip' or add one with browser.add")?;
    match app.project.clip(id) {
        Some(c) if c.browser.is_some() => Ok(id),
        Some(_) => Err(format!("clip {id} is not a browser layer")),
        None => Err(format!("no clip {id}")),
    }
}

/// Timeline seconds at `k` (default: the playhead) as the clip's local time.
fn local(app: &App, id: Id, args: &Value, k: &str) -> f64 {
    Args(args).t_or_playhead(k, app) - app.project.clip(id).map_or(0.0, |c| c.start)
}

/// Check `new` against the fields a style really has (`have`: its own JSON) before it is taken: an
/// unknown name or a value of the wrong kind is an error that names the right ones, not a silent skip.
fn check_fields(have: &Value, new: &Value, path: &str) -> Result<(), String> {
    let (Some(have), Some(new)) = (have.as_object(), new.as_object()) else {
        return Err(format!("'{path}' must be an object"));
    };
    for (k, v) in new {
        let Some(old) = have.get(k) else {
            let known: Vec<&str> = have.keys().map(|s| s.as_str()).collect();
            return Err(format!("unknown field '{k}' in {path} (fields: {})", known.join(", ")));
        };
        let named = match (k.as_str(), v.as_str()) {
            ("theme", Some(s)) => BrowserTheme::from_name(s).is_some(),
            ("display", Some(s)) => crate::model::BrowserDisplay::from_name(s).is_some(),
            ("shape", Some(s)) => CursorShape::from_name(s).is_some(),
            _ => true,
        };
        let ok = match (old, v) {
            (_, Value::Object(_)) if k == "cursor" => {
                let base = if old.is_null() { crate::model::Cursor::default().to_json() } else { old.clone() };
                check_fields(&base, v, "cursor")?;
                true
            }
            (_, Value::Null) => k == "cursor",
            // a keyframable field: a constant, or the whole animated value
            (Value::Object(o), Value::Number(_) | Value::Object(_)) => o.contains_key("value"),
            (Value::Number(_), Value::Number(_)) | (Value::Bool(_), Value::Bool(_)) => true,
            (Value::String(_), Value::String(_)) | (Value::Array(_), Value::Array(_)) => true,
            _ => false,
        };
        if !ok || !named {
            return Err(format!("bad value for '{k}' in {path}: {v} (theme: Light, Dark, Minimal, Mobile; display: window, page; shape: Arrow, Hand, Text)"));
        }
    }
    Ok(())
}

/// `browser.set`: take `fields` into a browser style (`BrowserStyle::set_json`) once they check out.
fn apply_fields(b: &mut BrowserStyle, fields: &Value) -> Result<(), String> {
    // `mode` is another word for `display`
    let mut f = fields.clone();
    if let Some(m) = f.as_object_mut().and_then(|o| o.remove("mode")) {
        if f.get("display").is_none() {
            f["display"] = m;
        }
    }
    check_fields(&b.to_json(), &f, "fields")?;
    b.set_json(&f);
    Ok(())
}

/// Where the fake mouse of browser `id` is at timeline `t`: as a point of the page area (what the page
/// Zoom holds still), or `on_screen` as a frame point (what the Window zoom holds still). Project px
/// from the frame's middle.
fn cursor_point(app: &App, id: Id, t: f64, on_screen: bool) -> Option<(f64, f64)> {
    let c = app.project.clip(id)?;
    let pos = c.browser.as_ref()?.cursor.as_ref()?.pos(c.local(t));
    if on_screen { frame_point(app, id, t, pos) } else { Some(pos) }
}

/// A point of browser `id`'s page area (the mouse's coordinates) as a frame point at timeline `t`.
fn frame_point(app: &App, id: Id, t: f64, pos: (f64, f64)) -> Option<(f64, f64)> {
    let (pw, ph) = (app.project.width, app.project.height);
    let p = br::plan(pw, app.project.clip(id)?, t, pw, ph)?.page_to_canvas(pos);
    Some((p.0 as f64 - pw as f64 / 2.0, p.1 as f64 - ph as f64 / 2.0))
}

/// The next-page arguments shared by browser.next_page and browser.click_go.
fn next_page(args: &Value) -> Result<NextPage<'_>, String> {
    let a = Args(args);
    let transition = match a.str("transition") {
        Some(t) => PageTransition::from_name(t).ok_or("transition: cut, fade, slide or zoom")?,
        None => PageTransition::Fade,
    };
    Ok(NextPage {
        asset: a.id("asset"),
        url: a.str("url").unwrap_or(""),
        title: a.str("title").unwrap_or(""),
        favicon: a.str("favicon").unwrap_or(""),
        load: a.f64("load").unwrap_or(0.6),
        transition,
        secs: a.f64("transition_secs").unwrap_or(0.4),
        type_url: a.bool("type_url").unwrap_or(false),
    })
}

fn event_kind(kind: &str, value: Option<&Value>) -> Result<CursorEventKind, String> {
    let text = || value.and_then(|v| v.as_str()).ok_or("this kind needs 'value' as a string");
    Ok(match kind.to_ascii_lowercase().as_str() {
        "click" => CursorEventKind::Click,
        "double" | "double_click" | "doubleclick" => CursorEventKind::DoubleClick,
        "down" => CursorEventKind::Down,
        "up" => CursorEventKind::Up,
        "scroll" => CursorEventKind::Scroll(value.and_then(|v| v.as_f64()).unwrap_or(1.0) as f32),
        "key" | "keys" => CursorEventKind::Key(text()?.to_string()),
        "shape" => CursorEventKind::Shape(CursorShape::from_name(text()?).ok_or("shape: Arrow, Hand or Text")?),
        _ => return Err("kind: click, double, down, up, scroll, key or shape".into()),
    })
}

// Seven tools for the whole feature: one per kind of work, each with an `action` where it covers several
// (they were 21: every row costs a closure, a description and an argument list in the exe).
const CLIP: &str = "clip:integer:false:browser clip (default: the selected / topmost one)";
const AT: &str = "at:number:false:seconds (default: the playhead)";
/// The tool's `action`, or the error that lists the right ones.
fn action<'a>(args: &'a Value, all: &str) -> Result<&'a str, String> {
    Args(args).str("action").ok_or_else(|| format!("action: {all}"))
}

fn bad_action(all: &str) -> Result<ToolOutcome, String> {
    Err(format!("action: {all}"))
}

/// `browser.set` / `browser.add` reply: the style, where the clip is, and the page area in cursor
/// coordinates.
fn style_reply(app: &App, id: Id) -> Result<ToolOutcome, String> {
    let c = app.project.clip(id).ok_or("no clip")?;
    let b = c.browser.as_ref().ok_or("no browser")?;
    let (vw, vh) = b.viewport();
    let k = app.project.width as f32 / vw;
    done(json!({"clip_id": id, "start": c.start, "end": c.end(), "style": b.to_json(),
        "page": {"width": app.project.width, "height": vh * k}}))
}

/// `browser.page` `elements`: what the mouse can go to on the page under the browser.
fn elements(app: &App, id: Id, args: &Value) -> Result<ToolOutcome, String> {
    let t = Args(args).t_or_playhead("at", app);
    let (page, map) = page_under(app, id, t).ok_or("the page under the browser here has no element map")?;
    let q = Args(args).str("query").unwrap_or("").to_lowercase();
    let mut list = Vec::new();
    for e in map.elements.iter().filter(|e| q.is_empty() || e.text.to_lowercase().contains(&q) || e.href.to_lowercase().contains(&q)) {
        let (Some(p), Some(z)) = (
            app.project.page_to_frame(page, t, (e.x / map.width, e.y / map.height)),
            app.project.page_to_frame(page, t, ((e.x + e.w) / map.width, (e.y + e.h) / map.height)),
        ) else {
            continue;
        };
        list.push(json!({"text": e.text, "role": e.role, "tag": e.tag, "href": e.href, "x": p.0, "y": p.1, "w": z.0 - p.0, "h": z.1 - p.1}));
        if list.len() == 200 {
            break;
        }
    }
    done(json!({"page_clip": page, "url": map.url, "title": map.title, "count": map.elements.len(), "elements": list}))
}

/// `browser.page` `click_go`.
fn click_go_tool(app: &mut App, id: Id, args: &Value) -> Result<ToolOutcome, String> {
    let a = Args(args);
    let mut next = next_page(args)?;
    next.load = a.f64("load").unwrap_or(0.7);
    let g = ClickGo { target: a.str("target"), nth: a.id("nth").unwrap_or(0) as usize, point: a.f64("x").zip(a.f64("y")), at: a.t_or_playhead("at", app), next };
    let (ids, clicked) = click_go(app, id, &g)?;
    done(json!({"clip_ids": ids, "clicked": clicked}))
}

/// `cursor.set` `move_to`.
fn move_tool(app: &mut App, id: Id, args: &Value) -> Result<ToolOutcome, String> {
    let a = Args(args);
    let t = a.t_or_playhead("at", app);
    let start = app.project.clip(id).map_or(0.0, |c| c.start);
    let (rect, text) = match (a.str("target"), a.f64("x").zip(a.f64("y"))) {
        (Some(target), _) => {
            let (_, r, text) = element_box(app, id, t, target, a.id("nth").unwrap_or(0) as usize)?;
            (Some(r), text)
        }
        (None, Some(p)) => (None, format!("{}, {}", p.0, p.1)),
        (None, None) => return Err("pass 'target' or 'x' and 'y'".into()),
    };
    let p = rect.map(|r| ((r.0 + r.2) / 2.0, (r.1 + r.3) / 2.0)).or(a.f64("x").zip(a.f64("y"))).unwrap_or((0.0, 0.0));
    let shown = move_to(app, id, t, a.f64("duration").unwrap_or(0.9), p)?;
    if let Some(r) = rect.filter(|_| a.bool("hover").unwrap_or(true)) {
        app.project.cursor_event(id, t - start - 0.05, CursorEventKind::Shape(CursorShape::Hand));
        app.project.browser_add_highlight(id, t - start - 0.1, t - start + a.f64("hold").unwrap_or(1.0), r);
    }
    if a.bool("click").unwrap_or(false) {
        app.project.cursor_event(id, t - start + 0.25, CursorEventKind::Click);
    }
    done(json!({"x": shown.0, "y": shown.1, "text": text}))
}

/// Rows of numbers (`[[t, x, y, ...], ...]`) from the argument `k`, each at least `min` long.
fn rows(args: &Value, k: &str, min: usize) -> Result<Vec<Vec<f64>>, String> {
    let list = args.get(k).and_then(|v| v.as_array()).filter(|a| !a.is_empty()).ok_or_else(|| format!("{k}: a list of rows of numbers"))?;
    list.iter()
        .map(|p| {
            let n: Vec<f64> = p.as_array().into_iter().flatten().map(|v| v.as_f64().or(v.as_bool().map(f64::from)).unwrap_or(0.0)).collect();
            if n.len() < min { Err(format!("{k}: rows of at least {min} numbers")) } else { Ok(n) }
        })
        .collect()
}

/// `cursor.set` `samples`: raw pointer samples reduced as Record mouse does.
fn samples_tool(app: &mut App, id: Id, args: &Value) -> Result<ToolOutcome, String> {
    let c = app.project.clip(id).ok_or("no clip")?;
    let b = c.browser.as_ref().ok_or("no browser")?;
    let (start, scroll, own) = (c.start, b.scroll.clone(), b.cursor.as_ref().map_or(0.35, |c| c.smoothing) as f64);
    let mut s: Vec<CursorSample> = rows(args, "samples", 3)?
        .iter()
        .map(|n| CursorSample { t: n[0] - start, x: n[1], y: n[2], down: n.get(3).is_some_and(|d| *d != 0.0), wheel: n.get(4).copied().unwrap_or(0.0) })
        .collect();
    crate::model::sort_by_time(&mut s, |s| s.t);
    let (from, to) = (s[0].t, s[s.len() - 1].t);
    let take = reduce_take(&s, Args(args).f64("smoothing").unwrap_or(own), scroll.at(from));
    app.project.cursor_apply_take(id, &take, from, to);
    done(json!({"keys": take.keys.len(), "events": take.events.len(), "samples": s.len()}))
}

const BROWSER_ACTIONS: &str = "fit | next | click_go | elements | sharpen | cleanup | make_editable";
const CURSOR_ACTIONS: &str = "move_to | path | event | press | tidy | samples";

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "browser.add",
        desc: "Add a browser window layer: the tracks below it show inside a drawn browser (tabs, address bar, fake mouse). Move, scale and rotate it like any clip. Returns {clip_id, max_scroll}.",
        args: &[
            AT,
            "duration:number:false:seconds (default: to the end of the timeline, 5 when empty)",
            "fields:object:false:as browser.set",
            "fit:boolean:false:fit the pictures below as pages (default true)",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let at = Args(args).t_or_playhead("at", app);
            let rest = app.project.duration() - at;
            let dur = Args(args).f64("duration").unwrap_or(if rest > 1.0 { rest } else { 5.0 });
            let id = app.project.add_browser_clip(at, dur);
            if let Some(f) = args.get("fields") {
                let b = app.project.clip_mut(id).and_then(|c| c.browser.as_mut()).ok_or("no browser")?;
                apply_fields(b, f)?;
            }
            let max = if Args(args).bool("fit").unwrap_or(true) { app.project.browser_fit_pages(id) } else { 0.0 };
            done(json!({"clip_id": id, "max_scroll": max}))
        },
    },
    ToolDef {
        name: "browser.set",
        desc: "Read or change a browser layer. Without `fields`: {clip_id, start, end, style, page: {width, height}} (the page area, project px from its centre). `fields`: any key of the style - display (window | page), theme (Light | Dark | Minimal | Mobile), tabs ([\"Title\"] or [{title, favicon}]), bg / bg2 ([r,g,b,a]), cursor {...}; a number for a keyframable field (scroll, zoom, load...) is a constant, motion is browser.animate or clip.keyframe.",
        args: &[CLIP, "fields:object:false:fields to change"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = clip_arg(app, args)?;
            if let Some(f) = args.get("fields") {
                let b = app.project.clip_mut(id).and_then(|c| c.browser.as_mut()).ok_or("no browser")?;
                apply_fields(b, f)?;
            }
            style_reply(app, id)
        },
    },
    ToolDef {
        name: "browser.animate",
        desc: "Keyframe presets: type_url (`value`: a new address), load, switch_tab (`value`: the tab), scroll_to / zoom_to (`value`: where). zoom_to holds a point still: `target` (an element's text), `at_cursor`, or `x`, `y`; `whole` zooms the whole browser, else only the page.",
        args: &[
            CLIP,
            "preset:string:true:type_url | load | switch_tab | scroll_to | zoom_to",
            AT,
            "duration:number:false:seconds (default 1.2 for typing, else 0.6)",
            "value:string:false:see the preset (a number as text is fine)",
            "target:string:false:zoom_to: an element's text on the imported page (browser.page elements)",
            "at_cursor:boolean:false:zoom_to: hold the fake mouse's position (the default with whole)",
            "whole:boolean:false:zoom_to: the whole browser, like a camera moving in",
            "x:number:false:zoom_to: the point to hold, project px from the frame's middle",
            "y:number:false:zoom_to: the point to hold",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = clip_arg(app, args)?;
            let lt = local(app, id, args, "at");
            let preset = req(Args(args).str("preset"), "preset")?;
            let dur = Args(args).f64("duration");
            let value = args.get("value");
            let num = || value.and_then(|v| v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())).ok_or("this preset needs a numeric 'value'");
            let whole = Args(args).bool("whole").unwrap_or(false);
            let t = Args(args).t_or_playhead("at", app);
            // the point the zoom holds still: an element by name, else (by default for the whole browser)
            // where the mouse is
            let aimed = Args(args).str("target");
            let cursor_at = match aimed {
                Some(target) => {
                    let (_, r, _) = element_box(app, id, t, target, 0)?;
                    let c = app.project.clip(id).ok_or("no clip")?;
                    let shown = c.browser.as_ref().ok_or("no browser")?.shown(c.local(t), ((r.0 + r.2) / 2.0, (r.1 + r.3) / 2.0));
                    Some(if whole { frame_point(app, id, t, shown).ok_or("no browser")? } else { shown })
                }
                None => cursor_point(app, id, t, whole),
            };
            let b = app.project.clip_mut(id).and_then(|c| c.browser.as_mut()).ok_or("no browser")?;
            match preset {
                "type_url" => {
                    if let Some(url) = value.and_then(|v| v.as_str()) {
                        b.go(lt, url, "");
                    }
                    b.type_url(lt, dur.unwrap_or(1.2));
                }
                "load" => b.run_load(lt, dur.unwrap_or(0.6)),
                "switch_tab" => b.switch_tab(lt, num()? as usize, dur.unwrap_or(0.6)),
                "scroll_to" => b.glide(false, lt, dur.unwrap_or(0.6), num()?.max(0.0)),
                "zoom_to" => {
                    let at = match (Args(args).f64("x"), Args(args).f64("y")) {
                        (Some(x), Some(y)) => Some((x, y)),
                        // the whole browser zooms to where the mouse is unless told otherwise
                        _ if aimed.is_some() || Args(args).bool("at_cursor").unwrap_or(whole) => cursor_at,
                        _ => None,
                    };
                    if whole {
                        b.window_zoom_to(lt, dur.unwrap_or(0.6), num()?, at);
                    } else {
                        b.zoom_to(lt, dur.unwrap_or(0.6), num()?, at);
                    }
                }
                _ => return Err("preset: type_url, load, switch_tab, scroll_to or zoom_to".into()),
            }
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "browser.page",
        desc: "Pages under a browser layer. action: fit (pictures below page-wide, top at the top; {max_scroll}); next (another page at `at`: cut with a transition, load bar, new address / title / icon; {clip_ids}); click_go (the mouse goes to `target`, an element's text, or x, y, clicks at `at`, the page changes as in next; one undo step); elements (links, buttons, inputs of the imported page: boxes in project px from the frame's middle, filter `query`); sharpen (draw every zoomed part of the page again from the HTML at the scale it is shown at, into an internal cache the preview and export use - no clips; job; {regions}); cleanup (remove the picture clips older builds made for zooms; {removed}); make_editable (headings, labels, short texts become fields of the HTML layer; job).",
        args: &[
            CLIP,
            "action:string:true:fit | next | click_go | elements | sharpen | cleanup | make_editable",
            AT,
            "target:string:false:click_go: text of the link / button",
            "nth:integer:false:click_go: which match, from 0",
            "x:number:false:click_go: instead of target, project px from the frame's middle",
            "y:number:false:click_go: instead of target",
            "asset:integer:false:asset id of the next page",
            "url:string:false:the next page's address",
            "title:string:false:its tab title",
            "favicon:string:false:its tab icon (a picture file)",
            "load:number:false:load bar seconds (default 0.6; 0.7 for click_go)",
            "transition:string:false:cut | fade (default) | slide | zoom",
            "type_url:boolean:false:type the address out",
            "transition_secs:number:false:default 0.4",
            "query:string:false:elements: only those whose text or address contains this",
            "max:integer:false:make_editable: most fields (default 24)",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = clip_arg(app, args)?;
            match action(args, BROWSER_ACTIONS)? {
                "fit" => done(json!({"max_scroll": app.project.browser_fit_pages(id)})),
                "next" => {
                    let t = Args(args).t_or_playhead("at", app);
                    done(json!({"clip_ids": app.project.browser_page_cut(id, t, &next_page(args)?)}))
                }
                "click_go" => click_go_tool(app, id, args),
                "elements" => elements(app, id, args),
                "sharpen" => done(json!({"regions": start_sharpen(app, id, false)})),
                "cleanup" => done(json!({"removed": remove_legacy_details(app)})),
                "make_editable" => {
                    let a = Args(args);
                    start_make_editable(app, id, a.t_or_playhead("at", app), a.id("max").unwrap_or(24) as usize)?;
                    done(json!({"started": true}))
                }
                _ => bad_action(BROWSER_ACTIONS),
            }
        },
    },
    ToolDef {
        name: "browser.import",
        desc: "Import a real web page (action import, the default): `src` is an address (private Edge, signed out) or a saved .mhtml / .html file; stored here with scripts removed, as an HTML layer with a map of its links and buttons, placed under a browser layer at `at` unless place is false. A job: poll action jobs until pending is 0. signed_in uses the profile of action sign_in, a visible Edge window for the USER to sign in (the tool never sees a password).",
        args: &[
            "action:string:false:import (default) | jobs | sign_in",
            "src:string:false:import: address or file path; sign_in: the page to open",
            "width:integer:false:layout width in CSS px (default: the frame's width)",
            AT,
            "place:boolean:false:put it on the timeline (default true); false = library only (a next page)",
            "signed_in:boolean:false:use the sign-in profile",
        ],
        kind: ToolKind::Ui,
        run: |app, args| {
            let a = Args(args);
            match a.str("action").unwrap_or("import") {
                "jobs" => {
                    let status: Vec<String> = app.browser.jobs.iter().map(|j| j.prog.status()).collect();
                    let waiting = app.browser.jobs.len() + app.browser.fit_due.len() + usize::from(app.browser.sharpen_at.is_some());
                    done(json!({"pending": waiting, "status": status, "last": app.browser.last, "asset": app.browser.last_asset}))
                }
                "sign_in" => {
                    webpage::open_sign_in(a.str("src").unwrap_or(""))?;
                    done(json!({"opened": true}))
                }
                "import" => {
                    let src = req(a.str("src"), "src")?.trim().to_string();
                    if src.is_empty() {
                        return Err("src is empty".into());
                    }
                    let width = a.f64("width").map_or(app.project.width, |w| w as u32);
                    let at = a.t_or_playhead("at", app);
                    start_import(app, src, width, at, a.bool("place").unwrap_or(true), a.bool("signed_in").unwrap_or(false));
                    done(json!({"started": true, "pending": app.browser.jobs.len()}))
                }
                _ => bad_action("import | jobs | sign_in"),
            }
        },
    },
    ToolDef {
        name: "cursor.set",
        desc: "Edit the fake mouse (timeline seconds). action: move_to (`target` element text, or x, y in project px from the frame's middle; arrives at `at` after `duration`; hover: hand + highlight; click); path (`points` [[t, x, y], ...] in page px from its centre; `mode` stops | glide); event (`kind` click | double | down | up | scroll | key | shape, `value`: 1 / -1, keys like Ctrl+L, Arrow | Hand | Text; `press` also dips the clip under the cursor); press (that dip alone; {clip_id}); tidy (remove jitter within `tolerance` px, `from`..`to`; {removed}); samples (`samples` [[t, x, y, down, wheel], ...] reduced as Record mouse does).",
        args: &[
            CLIP,
            "action:string:true:move_to | path | event | press | tidy | samples",
            AT,
            "target:string:false:move_to: element text",
            "nth:integer:false:move_to: which match, from 0",
            "x:number:false:move_to: instead of target",
            "y:number:false:move_to: instead of target",
            "duration:number:false:move_to: seconds of the move (default 0.9)",
            "hover:boolean:false:move_to: hand + highlight (default true for a target)",
            "hold:number:false:move_to: seconds the highlight stays (default 1)",
            "click:boolean:false:move_to: click on arriving",
            "points:array:false:path: [[t, x, y], ...]",
            "mode:string:false:path: stops | glide",
            "kind:string:false:event: default click",
            "value:string:false:event: see kind",
            "press:boolean:false:event: also dip the clip under the cursor",
            "tolerance:number:false:tidy: px (default from the cursor's smoothing, 1..24)",
            "from:number:false:tidy: seconds (default: the clip's start)",
            "to:number:false:tidy: seconds (default: the clip's end)",
            "samples:array:false:samples: [[t, x, y, down, wheel], ...]",
            "smoothing:number:false:samples: 0..1 (default: the cursor's own)",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = clip_arg(app, args)?;
            let a = Args(args);
            let t = a.t_or_playhead("at", app);
            match action(args, CURSOR_ACTIONS)? {
                "move_to" => move_tool(app, id, args),
                "path" => {
                    let start = app.project.clip(id).map_or(0.0, |c| c.start);
                    let mut pts: Vec<(f64, f64, f64)> = rows(args, "points", 3)?.iter().map(|n| (n[0] - start, n[1], n[2])).collect();
                    crate::model::sort_by_time(&mut pts, |p| p.0);
                    let how = if a.str("mode") == Some("glide") { CursorMove::Glide } else { CursorMove::Stops };
                    app.project.cursor_set_path(id, &pts, how);
                    done(json!({"keys": pts.len()}))
                }
                "event" => {
                    let kind = event_kind(a.str("kind").unwrap_or("click"), args.get("value"))?;
                    app.project.cursor_event(id, local(app, id, args, "at"), kind);
                    let pressed = if a.bool("press").unwrap_or(false) { app.project.cursor_press(id, t) } else { None };
                    done(json!({"ok": true, "pressed_clip": pressed}))
                }
                "press" => done(json!({"clip_id": app.project.cursor_press(id, t)})),
                "tidy" => {
                    let c = app.project.clip(id).ok_or("no clip")?;
                    let (start, dur) = (c.start, c.duration);
                    let sm = c.browser.as_ref().and_then(|b| b.cursor.as_ref()).map_or(0.35, |c| c.smoothing) as f64;
                    let tol = a.f64("tolerance").unwrap_or(1.0 + sm * 23.0);
                    let (from, to) = (a.f64("from").map_or(0.0, |t| t - start), a.f64("to").map_or(dur, |t| t - start));
                    done(json!({"removed": app.project.cursor_tidy(id, tol, from, to)}))
                }
                "samples" => samples_tool(app, id, args),
                _ => bad_action(CURSOR_ACTIONS),
            }
        },
    },
    ToolDef {
        name: "cursor.record",
        desc: "Record mouse. action start: the timeline plays from `at` (default the playhead) and the user's real mouse over the viewer is recorded until stop, Esc or the clip's end. action stop: write the take (one undo step; `discard` drops it); {keys, events}.",
        args: &[CLIP, "action:string:true:start | stop", "at:number:false:start: seconds to start from", "discard:boolean:false:stop: throw the take away"],
        kind: ToolKind::Ui,
        run: |app, args| match action(args, "start | stop")? {
            "start" => {
                let id = clip_arg(app, args)?;
                if app.browser.rec.is_some() {
                    return Err("already recording".into());
                }
                if let Some(t) = Args(args).f64("at") {
                    app.seek(t);
                }
                start_recording(app, id);
                done(json!({"recording": true, "clip_id": id, "from": app.playhead}))
            }
            "stop" if app.browser.rec.is_none() => done(json!({"recording": false})),
            "stop" => match stop_recording(app, !Args(args).bool("discard").unwrap_or(false)) {
                Some((keys, events)) => done(json!({"keys": keys, "events": events})),
                None => done(json!({"keys": 0, "events": 0, "discarded": true})),
            },
            _ => bad_action("start | stop"),
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorder / waypoint tick does nothing at all (no input read, no sheet, no repaint) unless a
    /// take or waypoint mode is on - `tick` returns on `!busy()` before touching the context.
    #[test]
    fn assert_no_idle_repaint_browser_layer() {
        let idle = BrowserCtl::default();
        assert!(!idle.busy(), "no take, no waypoints, no job: the tick returns before it reads input or asks for a frame");
        assert!(!idle.import.open, "and the Import Web Page window draws nothing until it is opened");
        assert!(BrowserCtl { waypoints: Some(1), ..Default::default() }.busy());
        for a in [
            Action::AddBrowser,
            Action::BrowserFitPages,
            Action::BrowserNextPage,
            Action::CursorPress,
            Action::RecordMouse,
            Action::CursorWaypoints,
            Action::ImportWebPage,
            Action::SharpenZooms,
            Action::PageMakeEditable,
        ] {
            assert_eq!(Action::from_id(a.id()), Some(a));
        }
    }

    #[test]
    fn fields_merge_through_the_styles_own_json() {
        let mut b = BrowserStyle::default();
        apply_fields(
            &mut b,
            &json!({"url": "https://roblox.example/games", "theme": "dark", "tabs": ["Home", {"title": "Game", "favicon": "C:/i.png"}],
                "scroll": 240, "bg": [1, 2, 3, 255], "cursor": {"shape": "hand", "x": 12.5, "size": 1.5}}),
        )
        .unwrap();
        assert_eq!((b.url.as_str(), b.theme, b.tabs.len()), ("https://roblox.example/games", BrowserTheme::Dark, 2));
        assert_eq!((b.tabs[1].favicon.as_str(), b.scroll.value, b.bg), ("C:/i.png", 240.0, [1, 2, 3, 255]));
        let c = b.cursor.as_ref().unwrap();
        assert_eq!((c.shape, c.x.value, c.size), (CursorShape::Hand, 12.5, 1.5));
        // an unknown field is an error that names the real ones; a bad value leaves the style alone
        let before = b.clone();
        assert!(apply_fields(&mut b, &json!({"adress": "x"})).unwrap_err().contains("url"));
        assert!(apply_fields(&mut b, &json!({"w": "wide"})).is_err());
        assert!(apply_fields(&mut b, &json!({"cursor": {"colour": 1}})).unwrap_err().contains("cursor"));
        assert_eq!(b, before);
        // display: by either word, in plain words
        apply_fields(&mut b, &json!({"mode": "page only"})).unwrap();
        assert_eq!(b.display, crate::model::BrowserDisplay::Page);
        apply_fields(&mut b, &json!({"display": "window"})).unwrap();
        assert_eq!(b.display, crate::model::BrowserDisplay::Window);
        assert!(apply_fields(&mut b, &json!({"display": "kiosk"})).is_err());
        // a cursor that was removed comes back with defaults
        b.cursor = None;
        apply_fields(&mut b, &json!({"cursor": {"visible": true}})).unwrap();
        assert!(b.cursor.is_some());
    }

    #[test]
    fn event_kinds_parse() {
        assert_eq!(event_kind("Click", None).unwrap(), CursorEventKind::Click);
        assert_eq!(event_kind("double", None).unwrap(), CursorEventKind::DoubleClick);
        assert_eq!(event_kind("key", Some(&json!("Ctrl+L"))).unwrap(), CursorEventKind::Key("Ctrl+L".into()));
        assert_eq!(event_kind("shape", Some(&json!("ibeam"))).unwrap(), CursorEventKind::Shape(CursorShape::Text));
        assert!(event_kind("key", None).is_err() && event_kind("tap", None).is_err());
    }
}
