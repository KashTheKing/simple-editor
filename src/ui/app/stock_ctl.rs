//! ---- ws:gallery-stock ----
//! Gallery ▸ Stock, the App side: runs searches off the UI thread (cached for ten minutes), loads thumbnails
//! lazily (only for cards on screen, a few at a time), makes hover previews (a few frames of a video, a quiet
//! snippet of a sound, both with ffmpeg reading only the first seconds), downloads with progress as Jobs,
//! imports the file into the Library with its credit, and answers `stock.search` / `stock.add` /
//! `stock.credits`. The site requests and parsers are `engine::stock`; the drawing is `ui::stock_ui`.
//!
//! All state lives in a thread-local on the UI thread (no `App` field), so this workstream adds no shared
//! struct field besides `GalleryState.stock`.

use super::*;
use crate::engine::export::Progress;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::engine::stock::{self, Hit, Kind, Query, Source};
use crate::ui::stock_ui::{self, Act, Status};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a search result stays good.
const CACHE_FOR: Duration = Duration::from_secs(600);
/// Thumbnail / preview workers at once (be kind to the sites).
const WORKERS: usize = 3;

type Found = (Vec<Hit>, Vec<String>);

struct Download {
    hit: Hit,
    prog: Arc<Progress>,
    out: Arc<Mutex<Option<PathBuf>>>,
    /// Where the file is expected (what a dragged card stood for).
    planned: PathBuf,
    /// Place it at this time once imported (Add at playhead).
    place: Option<f64>,
    /// Placeholder assets a drop made before the file arrived: probed when it lands.
    deferred: Vec<Id>,
    /// `stock.add`: finished after the import, with the asset in its status.
    mcp: Option<Arc<Progress>>,
}

#[derive(Default)]
struct Ctl {
    hits: Vec<Hit>,
    status: Status,
    shown: Option<Query>,
    more: bool,
    pending: Option<(Query, Receiver<Found>, Option<Arc<Progress>>)>,
    cache: HashMap<(Query, Vec<Source>), (Instant, Found)>,
    /// Every hit seen this session: what `stock.add` and the drop path look up.
    known: HashMap<String, Hit>,
    thumbs: HashMap<String, Option<PathBuf>>,
    /// (hit id, preview frames | audio snippet), None = failed.
    previews: HashMap<String, Option<Vec<PathBuf>>>,
    downloads: Vec<Download>,
    playing: Option<String>,
    hover_since: Option<(String, f64)>,
    /// Results already downloaded into `.0` (one disk check per result when the list, the folder or a
    /// download changes, never per frame). None = check again.
    have: Option<(PathBuf, HashSet<String>)>,
}

thread_local! {
    static CTL: RefCell<Ctl> = RefCell::default();
}

/// Background work done, keyed by what asked for it: thumbnails and previews land here.
#[derive(Default)]
struct Shared {
    queue: VecDeque<Task>,
    running: usize,
    thumbs: Vec<(String, Option<PathBuf>)>,
    previews: Vec<(String, Option<Vec<PathBuf>>)>,
}

enum Task {
    Thumb(String, String),
    Preview(Hit),
}

static SHARED: Mutex<Option<Shared>> = Mutex::new(None);

fn with_shared<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    let mut g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Shared::default))
}

fn enqueue(ctx: &egui::Context, t: Task) {
    let spawn = with_shared(|s| {
        s.queue.push_back(t);
        if s.running < WORKERS {
            s.running += 1;
            true
        } else {
            false
        }
    });
    if spawn {
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("stock-fetch".into()).spawn(move || loop {
            let Some(t) = with_shared(|s| {
                let t = s.queue.pop_back(); // newest first: the cards the user is looking at now
                if t.is_none() {
                    s.running -= 1;
                }
                t
            }) else {
                return;
            };
            match t {
                Task::Thumb(id, url) => {
                    let r = fetch_thumb(&url);
                    with_shared(|s| s.thumbs.push((id, r)));
                }
                Task::Preview(h) => {
                    let r = make_preview(&h);
                    with_shared(|s| s.previews.push((h.id.clone(), r)));
                }
            }
            ctx.request_repaint();
        });
    }
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn cache_dir() -> PathBuf {
    crate::settings::Settings::cache_dir().join("stock")
}

fn fetch_thumb(url: &str) -> Option<PathBuf> {
    let p = cache_dir().join("thumbs").join(format!("{:016x}.jpg", hash(url)));
    if p.is_file() {
        return Some(p);
    }
    let body = stock::get(&stock::Request { url: stock::sources::safe_url(url), headers: Vec::new() }, 20).ok()?;
    if body.len() < 64 {
        return None;
    }
    std::fs::create_dir_all(p.parent()?).ok()?;
    std::fs::write(&p, body).ok()?;
    Some(p)
}

/// A video: 6 frames from its first 12 s; a sound: its first 8 s as a quiet WAV (about -30 LUFS, faded in).
fn make_preview(h: &Hit) -> Option<Vec<PathBuf>> {
    let url = stock::sources::preview_url(h).ok()?;
    let ffmpeg = crate::media::ffpipe::ffmpeg_exe()?;
    let dir = cache_dir().join("previews").join(format!("{:016x}", hash(&h.id)));
    std::fs::create_dir_all(&dir).ok()?;
    let ua = stock::user_agent();
    let mut c = crate::media::ffpipe::command(&ffmpeg);
    c.args(["-y", "-v", "error", "-rw_timeout", "15000000", "-user_agent", &ua]);
    if h.kind.is_audio() {
        let out = dir.join("snippet.wav");
        if !out.is_file() {
            c.args(["-t", "8", "-i", &url, "-vn", "-af", "loudnorm=I=-30:TP=-9,afade=t=in:d=0.4,afade=t=out:st=7.4:d=0.6", "-ac", "2", "-ar", "44100"])
                .arg(&out);
            let ok = c.stdin(std::process::Stdio::null()).output().ok()?.status.success();
            if !ok {
                return None;
            }
        }
        return out.is_file().then(|| vec![out]);
    }
    let frames = |d: &Path| -> Vec<PathBuf> {
        (1..=6).map(|i| d.join(format!("f{i}.jpg"))).filter(|p| p.is_file()).collect()
    };
    if frames(&dir).len() < 6 {
        c.args(["-t", "12", "-i", &url, "-an", "-vf", "fps=0.5,scale=320:-2", "-frames:v", "6", "-q:v", "5"])
            .arg(dir.join("f%d.jpg"));
        let _ = c.stdin(std::process::Stdio::null()).output().ok()?;
    }
    let f = frames(&dir);
    (!f.is_empty()).then_some(f)
}

// ------------------------------------------------------------------------------------------- keys + settings

fn key(s: Source) -> Option<String> {
    stock::keys::get(s)
}

/// A source is on: not switched off in Settings, and a keyed one has its key.
pub(crate) fn usable(settings: &crate::settings::Settings, s: Source) -> (bool, bool) {
    let on = !settings.stock_off.iter().any(|x| x == s.id()) && (!s.needs_key() || settings.stock_on.iter().any(|x| x == s.id()));
    let ok = !s.needs_key() || key(s).is_some();
    (on, ok)
}

fn sources_on(settings: &crate::settings::Settings) -> Vec<Source> {
    Source::ALL.into_iter().filter(|s| usable(settings, *s) == (true, true)).collect()
}

/// Where downloads go: `Stock Footage` beside a saved project, else in the app's data folder.
pub(crate) fn target_dir(app: &App) -> PathBuf {
    match app.project_path.as_ref().and_then(|p| p.parent()) {
        Some(d) if app.settings.stock_in_project => d.join("Stock Footage"),
        _ => crate::settings::Settings::dir().join("Stock Footage"),
    }
}

/// The file a hit becomes (before its real extension is known: the URL's, else the kind's usual one).
fn planned(dir: &Path, h: &Hit) -> PathBuf {
    let ext = if h.file_url.is_empty() {
        match h.kind {
            Kind::Video => "mp4".to_string(),
            Kind::Image => "jpg".to_string(),
            _ => "mp3".to_string(),
        }
    } else {
        stock::url_ext(&h.file_url)
    };
    dir.join(stock::file_name(h, &ext))
}

// ------------------------------------------------------------------------------------------- search

fn start_search(ctx: Option<&egui::Context>, settings: &crate::settings::Settings, q: Query, reply: Option<Arc<Progress>>) {
    let srcs = sources_on(settings);
    CTL.with(|c| {
        let mut c = c.borrow_mut();
        let ck = (q.clone(), srcs.clone());
        if let Some((t, found)) = c.cache.get(&ck).cloned() {
            if t.elapsed() < CACHE_FOR {
                land(&mut c, q, found, reply);
                return;
            }
        }
        if srcs.is_empty() {
            land(&mut c, q, (Vec::new(), vec!["No stock source is on: turn one on in Settings ▸ Stock".into()]), reply);
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let qq = q.clone();
        let ctx = ctx.cloned();
        let keys: HashMap<Source, String> = srcs.iter().filter_map(|s| Some((*s, key(*s)?))).collect();
        let _ = std::thread::Builder::new().name("stock-search".into()).spawn(move || {
            let k = move |s: Source| keys.get(&s).cloned();
            let r = stock::search(&qq, &srcs, &k);
            let _ = tx.send(r);
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
        if q.page() == 1 {
            c.status = Status::Loading;
        }
        c.pending = Some((q, rx, reply));
    });
}

fn land(c: &mut Ctl, q: Query, found: Found, reply: Option<Arc<Progress>>) {
    for h in &found.0 {
        c.known.insert(h.id.clone(), h.clone());
    }
    let more = !found.0.is_empty();
    if let Some(r) = reply {
        let list: Vec<Value> = found.0.iter().map(hit_json).collect();
        let v = json!({"query": q, "results": list, "notes": found.1});
        let path = cache_dir().join("last-search.json");
        let _ = std::fs::create_dir_all(cache_dir());
        let _ = std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap_or_default());
        r.set(1.0, v.to_string());
        r.finish(None);
    }
    if q.page() > 1 && c.shown.as_ref().is_some_and(|s| Query { page: 1, ..s.clone() } == Query { page: 1, ..q.clone() }) {
        let seen: HashSet<String> = c.hits.iter().map(|h| h.id.clone()).collect();
        c.hits.extend(found.0.into_iter().filter(|h| !seen.contains(&h.id)));
    } else {
        c.hits = found.0;
    }
    c.more = more;
    c.have = None;
    c.status = Status::Done(found.1);
    c.shown = Some(q);
}

fn hit_json(h: &Hit) -> Value {
    json!({
        "id": h.id, "kind": h.kind, "title": h.title, "author": h.author, "source": h.source.map(Source::name),
        "licence": h.licence_name, "needs_credit": h.licence().needs_credit(), "page": h.page_url,
        "width": h.width, "height": h.height, "duration": h.duration, "thumb": h.thumb_url,
        "attribution": stock::attribution(h),
    })
}

fn poll_search(settings: &crate::settings::Settings) {
    let srcs = sources_on(settings);
    CTL.with(|c| {
        let mut c = c.borrow_mut();
        let Some((_, rx, _)) = &c.pending else { return };
        let Ok(found) = rx.try_recv() else { return };
        let (q, _, reply) = c.pending.take().unwrap();
        // a result with a failure is not kept (the next search tries again)
        if found.1.is_empty() {
            c.cache.insert((q.clone(), srcs), (Instant::now(), found.clone()));
        }
        c.cache.retain(|_, (t, _)| t.elapsed() < CACHE_FOR);
        land(&mut c, q, found, reply);
    });
}

// ------------------------------------------------------------------------------------------- downloads

/// Start downloading `id` (a hit seen in a search). Returns the planned path. Reuses a running download.
fn start_download(app: &App, id: &str, place: Option<f64>, mcp: Option<Arc<Progress>>) -> Result<PathBuf, String> {
    let dir = target_dir(app);
    CTL.with(|c| {
        let mut c = c.borrow_mut();
        let hit = c.known.get(id).cloned().ok_or_else(|| format!("'{id}' is not in a recent stock search: search first"))?;
        let planned = planned(&dir, &hit);
        if let Some(d) = c.downloads.iter_mut().find(|d| d.hit.id == hit.id) {
            d.place = d.place.or(place);
            if mcp.is_some() {
                d.mcp = mcp;
            }
            return Ok(d.planned.clone());
        }
        let out = Arc::new(Mutex::new(None));
        let o = out.clone();
        let h = hit.clone();
        let key = key_of(&hit);
        let prog = crate::engine::export::spawn_job("stock-download", move |prog| {
            let p = stock::fetch(&h, &dir, key.as_deref(), prog)?;
            *o.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
            Ok(())
        });
        c.downloads.push(Download { hit, prog, out, planned: planned.clone(), place, deferred: Vec::new(), mcp });
        Ok(planned)
    })
}

fn key_of(h: &Hit) -> Option<String> {
    h.source.and_then(key)
}

/// Jobs pane rows.
pub(super) fn progress_jobs() -> Vec<jobs_pane::ProgJob> {
    CTL.with(|c| {
        c.borrow()
            .downloads
            .iter()
            .map(|d| (crate::ui::jobs_ui::JobKind::Download, format!("Stock · {}", d.hit.title), d.prog.clone()))
            .collect()
    })
}

/// `App::import_files`: a file a Stock card stood for that is still downloading. The placeholder asset is
/// kept (and placed by the drop), and probed once the file lands. True = do not probe it now.
pub(super) fn defer(id: Id, path: &str) -> bool {
    CTL.with(|c| {
        let mut c = c.borrow_mut();
        match c.downloads.iter_mut().find(|d| d.planned.to_string_lossy() == path && !d.prog.is_done()) {
            Some(d) => {
                d.deferred.push(id);
                true
            }
            None => false,
        }
    })
}

/// A stock file's credit onto its Library asset (description + tags), from the `<file>.stock.json` beside it.
pub(super) fn stamp(project: &mut Project, id: Id, credit: &stock::Credit) {
    if let Some(a) = project.asset_mut(id) {
        a.description = stock::asset_description(credit);
        for t in [stock::TAG.to_string(), credit.source.to_ascii_lowercase()] {
            if !t.is_empty() && !a.tags.contains(&t) {
                a.tags.push(t);
            }
        }
    }
}

/// `App::import_files`, for every new asset: a stock file gets its credit.
pub(super) fn stamp_path(project: &mut Project, id: Id, path: &str) {
    if let Some(c) = stock::read_credit(Path::new(path)) {
        stamp(project, id, &c);
    }
}

fn poll_downloads(app: &mut App) {
    let done: Vec<Download> = CTL.with(|c| {
        let mut c = c.borrow_mut();
        let (done, keep): (Vec<Download>, Vec<Download>) = std::mem::take(&mut c.downloads).into_iter().partition(|d| d.prog.is_done());
        c.downloads = keep;
        done
    });
    for d in done {
        let file = d.out.lock().unwrap_or_else(|e| e.into_inner()).clone();
        CTL.with(|c| c.borrow_mut().have = None);
        let file = match (d.prog.error(), file) {
            (None, Some(f)) => f,
            (e, _) => {
                let e = e.unwrap_or_else(|| "download failed".into());
                // the drop's placeholders go again, like any import that could not be read
                for id in &d.deferred {
                    app.project.remove_asset(*id);
                }
                if !d.deferred.is_empty() {
                    app.after_edit();
                }
                if e != crate::engine::export::CANCELLED {
                    app.toast(format!("Couldn't add \"{}\": {e}", d.hit.title));
                }
                if let Some(m) = d.mcp {
                    m.finish(Some(e));
                }
                continue;
            }
        };
        let fs = file.to_string_lossy().into_owned();
        let credit = stock::read_credit(&file).unwrap_or_else(|| stock::Credit::of(&d.hit));
        let ids: Vec<Id> = if d.deferred.is_empty() {
            let ids = app.import_files(std::slice::from_ref(&file));
            for id in &ids {
                stamp(&mut app.project, *id, &credit);
            }
            if let Some(at) = d.place {
                app.place_assets(&ids, at, None, DropMode::Place);
            }
            ids
        } else {
            // a drop placed placeholders while the file was on its way: point them at it and probe now
            let mut fresh = Vec::new();
            for id in &d.deferred {
                if let Some(a) = app.project.asset_mut(*id) {
                    a.path = fs.clone();
                    fresh.push((*id, fs.clone()));
                }
                stamp(&mut app.project, *id, &credit);
            }
            if !fresh.is_empty() {
                app.probes.push(crate::engine::import::probe_async(fresh, app.backend()));
            }
            d.deferred.clone()
        };
        app.after_edit();
        app.toast(format!("Added \"{}\" to the Library", d.hit.title));
        if let Some(m) = d.mcp {
            m.set(1.0, json!({"asset_ids": ids, "path": fs, "attribution": credit.attribution, "licence": credit.licence}).to_string());
            m.finish(None);
        }
    }
}

/// Every stock file in the project, with its credit (in Library order).
pub(crate) fn project_credits(project: &Project) -> Vec<(String, stock::Credit)> {
    project
        .assets
        .iter()
        .filter_map(|a| {
            let p = Path::new(&a.path);
            let c = stock::read_credit(p)?;
            Some((p.file_name()?.to_string_lossy().into_owned(), c))
        })
        .collect()
}

fn export_credits(app: &mut App) {
    let name = app.project_path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let text = stock::credits_text(&project_credits(&app.project), &name);
    let mut dlg = rfd::FileDialog::new().set_title("Export credits").add_filter("Text", &["txt"]).set_file_name("Credits.txt");
    if let Some(d) = app.project_path.as_ref().and_then(|p| p.parent()) {
        dlg = dlg.set_directory(d);
    }
    if let Some(p) = dlg.save_file() {
        match std::fs::write(&p, text) {
            Ok(()) => app.toast(format!("Credits saved to {}", p.display())),
            Err(e) => app.toast(format!("Couldn't save the credits: {e}")),
        }
    }
}

// ------------------------------------------------------------------------------------------- frame

/// FRAME_HOOKS: searches, downloads and the hover sound, also while the Gallery is closed.
pub(super) fn frame(app: &mut App, ctx: &egui::Context) {
    poll_search(&app.settings);
    let busy = CTL.with(|c| !c.borrow().downloads.is_empty() || c.borrow().pending.is_some());
    poll_downloads(app);
    if busy {
        ctx.request_repaint_after(Duration::from_millis(200));
    }
    // background results into the per-id maps
    let (th, pv) = with_shared(|s| (std::mem::take(&mut s.thumbs), std::mem::take(&mut s.previews)));
    if !th.is_empty() || !pv.is_empty() {
        CTL.with(|c| {
            let mut c = c.borrow_mut();
            c.thumbs.extend(th);
            c.previews.extend(pv);
        });
    }
}

fn sound_stop(c: &mut Ctl) {
    if c.playing.take().is_some() {
        super::gallery_mine::preview::stop();
    }
}

/// Draw the Stock tab (called by `gallery_ctl::draw` under the Gallery's tab strip).
pub(super) fn draw(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let now = ctx.input(|i| i.time);
    let dir = target_dir(app);
    let sources: Vec<(Source, bool, bool)> =
        Source::ALL.into_iter().map(|s| {
            let (on, ok) = usable(&app.settings, s);
            (s, on && ok, ok)
        }).collect();
    let hover_preview = app.settings.stock_preview && !app.gallery.no_hover_preview;
    let resp = CTL.with(|c| {
        let mut c = c.borrow_mut();
        let busy: HashMap<String, f32> = c.downloads.iter().map(|d| (d.hit.id.clone(), d.prog.fraction())).collect();
        if c.have.as_ref().is_none_or(|(d, _)| *d != dir) {
            let set = c.hits.iter().filter(|h| planned(&dir, h).is_file()).map(|h| h.id.clone()).collect();
            c.have = Some((dir.clone(), set));
        }
        let Ctl { hits, status, more, thumbs, previews, hover_since, have, .. } = &mut *c;
        let have: &HashSet<String> = have.as_ref().map(|h| &h.1).expect("filled above");
        let mut want_thumbs: Vec<(String, String)> = Vec::new();
        let tc = RefCell::new(&mut app.thumbs);
        let mut thumb = |h: &Hit| -> Option<(egui::TextureId, [u32; 2])> {
            match thumbs.get(&h.id) {
                Some(Some(p)) => tc.borrow_mut().texture(&ctx, &p.to_string_lossy(), 0.0, 180),
                Some(None) => None,
                None => {
                    if !h.thumb_url.is_empty() {
                        want_thumbs.push((h.id.clone(), h.thumb_url.clone()));
                    }
                    None
                }
            }
        };
        let hovered = hover_since.clone();
        let mut frame = |h: &Hit| -> Option<(egui::TextureId, [u32; 2])> {
            let (id, since) = hovered.as_ref()?;
            if id != &h.id || h.kind != Kind::Video {
                return None;
            }
            let frames = previews.get(&h.id)?.as_ref()?;
            let i = (((now - since) * 1.5) as usize) % frames.len().max(1);
            ctx.request_repaint_after(Duration::from_millis(300));
            tc.borrow_mut().texture(&ctx, &frames.get(i)?.to_string_lossy(), 0.0, 180)
        };
        let drag_path = |h: &Hit| planned(&dir, h).to_string_lossy().into_owned();
        let mut view = stock_ui::View {
            hits,
            status: status.clone(),
            more: *more,
            busy: &busy,
            have,
            sources,
            thumb: &mut thumb,
            frame: &mut frame,
            drag_path: &drag_path,
            hover_preview,
        };
        let r = stock_ui::show(ui, &mut app.gallery.stock_ui, &mut view);
        for (id, url) in want_thumbs {
            // asked once: None until the worker answers (a failure stays the type icon)
            thumbs.insert(id.clone(), None);
            enqueue(&ctx, Task::Thumb(id, url));
        }
        r
    });
    // hover: start a preview, hear a sound (quietly), stop when the pointer leaves
    CTL.with(|c| {
        let mut c = c.borrow_mut();
        match &resp.hover {
            Some(id) => {
                if c.hover_since.as_ref().map(|h| &h.0) != Some(id) {
                    c.hover_since = Some((id.clone(), now));
                    sound_stop(&mut c);
                }
                let hit = c.known.get(id).cloned();
                if let Some(h) = hit.filter(|h| h.kind != Kind::Image) {
                    match c.previews.get(id) {
                        None => {
                            // None = in flight (or failed): the worker's answer replaces it
                            c.previews.insert(id.clone(), None);
                            enqueue(&ctx, Task::Preview(h));
                        }
                        Some(Some(files)) if h.kind.is_audio() && c.playing.as_deref() != Some(id.as_str()) => {
                            super::gallery_mine::preview::play(&files[0]);
                            c.playing = Some(id.clone());
                        }
                        _ => {}
                    }
                }
            }
            None => {
                c.hover_since = None;
                sound_stop(&mut c);
            }
        }
    });
    for act in resp.acts {
        match act {
            Act::Search => {
                let mut q = app.gallery.stock_ui.q.clone();
                q.page = 1;
                start_search(Some(&ctx), &app.settings, q, None);
            }
            Act::More => {
                let next = CTL.with(|c| c.borrow().shown.clone()).map(|mut q| {
                    q.page = q.page() + 1;
                    q
                });
                if let Some(q) = next {
                    start_search(Some(&ctx), &app.settings, q, None);
                }
            }
            Act::Add(id, at_playhead) => {
                let at = at_playhead.then_some(app.playhead);
                if let Err(e) = start_download(app, &id, at, None) {
                    app.toast(e);
                }
            }
            Act::DragStart(id) => {
                let _ = start_download(app, &id, None, None);
            }
            Act::Open(url) => {
                if url.starts_with("https://") || url.starts_with("http://") {
                    crate::engine::browsers::open_url(&url);
                }
            }
            Act::Copy(text) => {
                ctx.copy_text(text);
                app.toast("Credit copied");
            }
            Act::ToggleSource(s) => toggle_source(&mut app.settings, s),
            Act::Settings => open_settings(app),
            Act::Credits => export_credits(app),
            Act::HoverPreview(on) => {
                app.settings.stock_preview = on;
                app.settings.save();
            }
        }
    }
}

fn toggle_source(settings: &mut crate::settings::Settings, s: Source) {
    let id = s.id().to_string();
    if s.needs_key() {
        if let Some(i) = settings.stock_on.iter().position(|x| *x == id) {
            settings.stock_on.remove(i);
        } else {
            settings.stock_on.push(id.clone());
        }
        settings.stock_off.retain(|x| *x != id);
    } else if let Some(i) = settings.stock_off.iter().position(|x| *x == id) {
        settings.stock_off.remove(i);
    } else {
        settings.stock_off.push(id);
    }
    settings.save();
}

/// Settings ▸ Stock.
pub(super) fn open_settings(app: &mut App) {
    app.settings_ui.open = true;
    app.settings_ui.tab = crate::ui::settings_ui::STOCK_TAB;
}

// ------------------------------------------------------------------------------------------- MCP

fn query_of(args: &Value) -> Result<Query, String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mut q = Query { text: s("query"), page: args.get("page").and_then(Value::as_u64).unwrap_or(1) as u32, ..Default::default() };
    if !s("kind").is_empty() {
        q.kind = Kind::from_id(&s("kind")).ok_or("kind: video | image | music | sfx")?;
    }
    if !s("orientation").is_empty() {
        q.orientation = stock::Orientation::from_id(&s("orientation")).ok_or("orientation: any | landscape | portrait | square")?;
    }
    if !s("licence").is_empty() || !s("license").is_empty() {
        let l = if s("licence").is_empty() { s("license") } else { s("licence") };
        q.licence = stock::LicenceLevel::from_id(&l).ok_or("licence: pd | by | by-sa")?;
    }
    q.min_res = args.get("min_res").and_then(Value::as_u64).unwrap_or(0) as u32;
    Ok(q)
}

pub(super) const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "stock.search",
        desc: "Search free stock media online (Gallery ▸ Stock): Wikimedia Commons, the Internet Archive, NASA and Openverse with no key, plus Pexels / Pixabay / Freesound when the user added their own key in Settings ▸ Stock. Runs in the background and replies with `status` = JSON {query, results: [{id, kind, title, author, source, licence, needs_credit, page, width, height, duration, attribution}], notes} (also written to `path`). Results are cached for 10 minutes and shown in the Gallery's Stock tab. licence: pd (default: CC0 / public domain / no credit needed) | by | by-sa (each includes the ones before; NC / ND licences are never returned).",
        args: &[
            "query:string:true:words to search for",
            "kind:string:false:video (default) | image | music | sfx",
            "orientation:string:false:any | landscape | portrait | square",
            "min_res:number:false:shorter side at least this many pixels (720, 1080, 2160)",
            "licence:string:false:pd | by | by-sa",
            "page:number:false:1-based page",
        ],
        kind: ToolKind::Job,
        run: |app, args| {
            let q = query_of(args)?;
            app.gallery.stock_ui.q = Query { page: 1, ..q.clone() };
            let prog = Progress::new();
            start_search(None, &app.settings, q, Some(prog.clone()));
            Ok(ToolOutcome::Job(prog, cache_dir().join("last-search.json")))
        },
    },
    ToolDef {
        name: "stock.add",
        desc: "Download a stock result (an id from stock.search) into the project's Stock Footage folder and import it into the Library with its credit (source URL, licence and attribution in the asset's description, `stock` tag, `<file>.stock.json` beside the file). Replies when it is imported with `status` = JSON {asset_ids, path, attribution, licence}. place_at (seconds) also puts it on the timeline there.",
        args: &["id:string:true:a stock.search result id, e.g. nasa:PIA01973", "place_at:number:false:timeline time to place it at"],
        kind: ToolKind::Job,
        run: |app, args| {
            let id = args.get("id").and_then(Value::as_str).ok_or("id: string required")?;
            let at = args.get("place_at").and_then(Value::as_f64);
            let prog = Progress::new();
            let path = start_download(app, id, at, Some(prog.clone()))?;
            Ok(ToolOutcome::Job(prog, path))
        },
    },
    ToolDef {
        name: "stock.credits",
        desc: "The project's stock credits as text: the attribution of every stock file in the Library (the ones that must be credited first), as Gallery ▸ Stock ▸ Export credits writes it.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| {
            let name = app.project_path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let list = project_credits(&app.project);
            Ok(ToolOutcome::Done(json!({"text": stock::credits_text(&list, &name), "count": list.len()})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: &str) -> Hit {
        Hit { id: id.into(), title: "Sea".into(), source: Some(Source::Nasa), kind: Kind::Video, licence: Some(stock::Licence::Pd), ..Default::default() }
    }

    #[test]
    fn stock_planned_paths_use_the_url_or_the_kind() {
        let dir = Path::new("C:/p/Stock Footage");
        let mut h = hit("nasa:abc");
        assert_eq!(planned(dir, &h), dir.join("nasa-sea-abc.mp4"));
        h.file_url = "https://x/y.webm".into();
        assert_eq!(planned(dir, &h), dir.join("nasa-sea-abc.webm"));
    }

    #[test]
    fn stock_sources_on_by_default_are_the_keyless_ones() {
        let s = crate::settings::Settings::default();
        assert_eq!(sources_on(&s), vec![Source::Commons, Source::Archive, Source::Nasa, Source::Openverse]);
        let mut off = s.clone();
        toggle_source_no_save(&mut off, Source::Nasa);
        assert!(!sources_on(&off).contains(&Source::Nasa));
        // a keyed source switched on still needs its key (tests never read the real store)
        let mut on = s.clone();
        on.stock_on.push("pexels".into());
        assert!(!sources_on(&on).contains(&Source::Pexels));
    }

    fn toggle_source_no_save(s: &mut crate::settings::Settings, src: Source) {
        s.stock_off.push(src.id().into());
    }

    #[test]
    fn stock_search_results_are_cached_and_known() {
        let s = crate::settings::Settings::default();
        let q = Query { text: "zz-cached".into(), ..Default::default() };
        CTL.with(|c| {
            let mut c = c.borrow_mut();
            c.cache.insert((q.clone(), sources_on(&s)), (Instant::now(), (vec![hit("nasa:1")], vec![])));
        });
        start_search(None, &s, q.clone(), None);
        CTL.with(|c| {
            let c = c.borrow();
            assert!(c.pending.is_none(), "served from the cache, no network");
            assert_eq!(c.hits.len(), 1);
            assert!(c.known.contains_key("nasa:1"));
            assert_eq!(c.status, Status::Done(vec![]));
        });
    }

    #[test]
    fn stock_mcp_query_args() {
        let q = query_of(&json!({"query": "rain", "kind": "sfx", "licence": "by", "min_res": 1080})).unwrap();
        assert_eq!((q.kind, q.licence, q.min_res, q.text.as_str()), (Kind::Sfx, stock::LicenceLevel::By, 1080, "rain"));
        assert!(query_of(&json!({"kind": "nope"})).is_err());
        assert_eq!(query_of(&json!({})).unwrap().licence, stock::LicenceLevel::Pd);
    }

    #[test]
    fn stock_stamp_writes_the_credit_onto_the_asset() {
        let mut p = Project::new();
        let id = p.add_asset(crate::engine::import::placeholder("C:/x/nasa-sea-1.mp4"));
        let c = stock::Credit::of(&hit("nasa:1"));
        stamp(&mut p, id, &c);
        stamp(&mut p, id, &c);
        let a = p.asset(id).unwrap();
        assert!(a.description.starts_with("Stock: \"Sea\""), "{}", a.description);
        assert_eq!(a.tags, vec!["stock".to_string(), "nasa".to_string()], "tags once");
    }
}
