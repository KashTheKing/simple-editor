//! ---- ws:online-library ----
//! The online library, App side: the catalogue (loaded on a background thread when a Gallery library
//! tab or a `library.*` tool first needs it), downloads (`export::spawn_job`, so the Jobs machinery and
//! `library.install`'s MCP reply share them), thumbnails fetched lazily for visible cells only, and
//! `apply`, the one place an item becomes an edit.

use super::*;
use crate::engine::library::{self, Kind, Row, Source};
use std::collections::{HashMap, HashSet};

/// What happens when a download finishes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Then {
    Install,
    Apply { clip: Option<Id>, at: Option<f64> },
    View,
}

struct Dl {
    id: String,
    prog: Arc<Progress>,
    then: Then,
}

/// One message from the catalogue thread (it sends the cached copy first, then a fresh one).
struct Loaded {
    cat: Result<library::Catalogue, String>,
    installed: HashSet<String>,
    /// id -> thumbnail file already on this PC
    thumbs: HashMap<String, PathBuf>,
}

/// At most this many thumbnail downloads at once.
const THUMB_FETCHES: usize = 4;

#[derive(Default)]
pub struct OnlineState {
    pub cat: Option<library::Catalogue>,
    /// Installed items as full ids (refreshed by every catalogue load, kept current by install / remove).
    pub installed: HashSet<String>,
    /// One quiet line when the catalogue could not be fetched (empty = fine).
    pub note: String,
    loading: Option<Receiver<Loaded>>,
    loaded_for: Option<Source>,
    jobs: Vec<Dl>,
    /// id -> (thumbnail path relative to the library, its file on this PC once known; None = not yet
    /// / none)
    thumbs: HashMap<String, (String, Option<PathBuf>)>,
    /// ids whose thumbnail was asked for and is not on this PC yet (fetched THUMB_FETCHES at a time)
    thumb_wait: Vec<String>,
    thumb_tried: HashSet<String>,
    thumb_rx: Option<(Sender<(String, Option<PathBuf>)>, Receiver<(String, Option<PathBuf>)>)>,
    thumb_running: usize,
    /// "View text" window: (title, text).
    pub view: Option<(String, String)>,
}

impl OnlineState {
    /// Rows of `kind` (all kinds: None) matching `query`: the catalogue merged with what is installed.
    pub fn rows(&self, kind: Option<Kind>, query: &str) -> Vec<Row> {
        let items = self.cat.as_ref().map_or(&[][..], |c| &c.items[..]);
        let mut rows = library::merge(items, &self.installed);
        rows.retain(|r| kind.is_none_or(|k| r.kind == k) && r.matches(query));
        rows
    }
    pub fn loading(&self) -> bool {
        self.loading.is_some()
    }
    /// Download progress of `id` (Some while it is downloading).
    pub fn busy(&self, id: &str) -> Option<f32> {
        self.jobs.iter().find(|j| j.id == id).map(|j| j.prog.fraction())
    }
    pub fn name_of(&self, id: &str) -> String {
        let e = self.cat.as_ref().and_then(|c| c.items.iter().find(|e| e.id == id));
        e.map(|e| e.name.clone()).unwrap_or_else(|| id.rsplit('/').next().unwrap_or(id).replace('-', " "))
    }
    /// The thumbnail file of `id` once it is on this PC; asking for one that is not queues its download.
    pub fn thumb_path(&mut self, id: &str) -> Option<PathBuf> {
        let (rel, path) = self.thumbs.get(id)?;
        if path.is_none() && !rel.is_empty() && self.thumb_tried.insert(id.to_string()) {
            self.thumb_wait.push(id.to_string());
        }
        path.clone()
    }
    fn take_loaded(&mut self, l: Loaded) {
        match l.cat {
            Ok(c) => {
                for e in &c.items {
                    let path = l.thumbs.get(&e.id).cloned();
                    self.thumbs.entry(e.id.clone()).or_insert((e.thumb.clone(), None)).1 = path;
                }
                self.cat = Some(c);
                self.note.clear();
            }
            Err(e) if self.cat.is_none() => {
                // curl's "returned error: 404": the address is fine but nothing is published there
                self.note = if e.contains("404") {
                    "The online library is not published yet (no library folder on the main branch).                      Click \"Use a folder…\" and pick the library folder of the repository to use it now;                      installed items are shown meanwhile."
                        .into()
                } else {
                    format!("Library not reachable ({}): showing installed items.", first_line(&e))
                }
            }
            Err(_) => self.note = "Offline: showing the saved catalogue.".into(),
        }
        self.installed = l.installed;
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

/// Load the catalogue in the background unless it is loaded (or loading) for the current source.
/// `force` (Refresh) fetches it again.
pub(crate) fn ensure(app: &mut App, force: bool) {
    let src = Source::of(&app.settings);
    let st = &mut app.online;
    if st.loading.is_some() || (!force && st.loaded_for.as_ref() == Some(&src)) {
        return;
    }
    if st.loaded_for.as_ref() != Some(&src) {
        (st.cat, st.thumbs, st.thumb_tried) = (None, HashMap::new(), HashSet::new());
    }
    st.loaded_for = Some(src.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    st.loading = Some(rx);
    let _ = std::thread::Builder::new().name("library-catalogue".into()).spawn(move || {
        let send = |cat: Result<library::Catalogue, String>| {
            let thumbs = cat.as_ref().map_or_else(
                |_| HashMap::new(),
                |c| {
                    c.items
                        .iter()
                        .filter(|e| !e.thumb.is_empty())
                        .map(|e| (e.id.clone(), src.local(&e.thumb)))
                        .filter(|(_, p)| p.is_file())
                        .collect()
                },
            );
            let _ = tx.send(Loaded { cat, installed: library::installed_ids(), thumbs });
        };
        let cached = library::cached_catalogue(&src);
        let fresh = !force && cached.is_some() && !library::catalogue_stale(&src);
        if let Some(c) = cached {
            send(Ok(c));
        }
        if !fresh {
            send(library::fetch_catalogue(&src));
        }
    });
}

/// Download (or copy) `id` in the background, then do `then`.
pub(crate) fn start_download(app: &mut App, id: &str, then: Then) -> Arc<Progress> {
    if let Some(j) = app.online.jobs.iter().find(|j| j.id == id) {
        return j.prog.clone();
    }
    let src = Source::of(&app.settings);
    let owned = id.to_string();
    let prog = export::spawn_job("library-download", move |p| {
        p.set(0.0, "Downloading…");
        library::fetch_item(&src, &owned).map(|_| ())
    });
    app.online.jobs.push(Dl { id: id.to_string(), prog: prog.clone(), then });
    prog
}

/// A Gallery click: apply now when the file is at hand, else download first (progress on the cell).
pub(crate) fn click(app: &mut App, id: &str) {
    let then = Then::Apply { clip: None, at: Some(app.playhead) };
    let local = matches!(Source::of(&app.settings), Source::Dir(_));
    if app.online.installed.contains(id) || local {
        finish(app, id, then);
    } else {
        start_download(app, id, then);
    }
}

/// The skill "View text" window (other kinds: their text too, it is harmless).
pub(crate) fn view(app: &mut App, id: &str) {
    match library::split_id(id).map(|(k, i)| library::read(k, &i)) {
        Some(Ok(text)) => app.online.view = Some((app.online.name_of(id), text)),
        _ => match Source::of(&app.settings) {
            src @ Source::Dir(_) => {
                let text = std::fs::read_to_string(src.local(&library::file_of(id).unwrap_or_default()));
                app.online.view = Some((app.online.name_of(id), text.unwrap_or_else(|e| e.to_string())));
            }
            Source::Url(_) => {
                start_download(app, id, Then::View);
            }
        },
    }
}

/// After a download (or straight away when nothing needs downloading).
fn finish(app: &mut App, id: &str, then: Then) {
    match then {
        Then::Install => {
            let msg = format!("Installed {}", app.online.name_of(id));
            app.toast(msg);
        }
        Then::View => view(app, id),
        Then::Apply { clip, at } => {
            let args = json!({"id": id, "clip": clip, "at": at});
            match app.run_tool_undoable("library.apply", &args) {
                Ok(v) => app.toast(v["message"].as_str().unwrap_or("Applied").to_string()),
                Err(e) => app.toast(e),
            }
        }
    }
}

/// What applying an item of `kind` does (pure, so the dispatch is testable without an App).
#[derive(Debug, PartialEq)]
pub(super) enum Step {
    Transition(String),
    Effect(String),
    Lut(String),
    Html(String),
    Skill,
}

pub(super) fn step(kind: Kind, text: &str, name: &str, path: &Path) -> Step {
    match kind {
        Kind::Transition => Step::Transition(text.to_string()),
        Kind::Effect => Step::Effect(text.to_string()),
        Kind::Lut => Step::Lut(path.to_string_lossy().into_owned()),
        Kind::Html => Step::Html(text.to_string()),
        Kind::Icon => Step::Html(crate::engine::html::icon_page(name, text)),
        Kind::Skill => Step::Skill,
    }
}

/// `library.apply`'s body: install `id` when needed (a local folder: copied now; online: the caller
/// must `library.install` first, which downloads), then apply it to `clip` (None: the selection) /
/// at `at` (None: the playhead). Returns the toast line. No undo here: the tool is `Mutate`.
pub(super) fn apply(app: &mut App, id: &str, clip: Option<Id>, at: Option<f64>) -> Result<String, String> {
    let (kind, short) = library::split_id(id).ok_or_else(|| format!("no such library item '{id}'"))?;
    let path = library::path_of(kind, &short);
    if !path.is_file() {
        match Source::of(&app.settings) {
            src @ Source::Dir(_) => {
                library::fetch_item(&src, id)?;
                app.online.installed.insert(id.to_string());
            }
            Source::Url(_) => return Err(format!("{id} is not installed yet: call library.install first")),
        }
    }
    let text = library::read(kind, &short)?;
    let name = app.online.name_of(id);
    match step(kind, &text, &name, &path) {
        Step::Transition(src) => {
            app.apply_transition_shader(&src, clip).map(|_| format!("Added the {name} transition"))
        }
        Step::Effect(src) => app.apply_shader_effect(&src, clip).map(|_| format!("Added {name}")),
        Step::Lut(p) => {
            let ids = clip.map_or_else(|| app.project.expand_links(&app.selection), |c| vec![c]);
            if ids.is_empty() {
                return Err("Select a clip first".into());
            }
            for &c in &ids {
                let r = super::tools_color::dispatch(app, "clip.add_lut", &json!({"clip_id": c, "path": p}));
                r.unwrap_or_else(|| Err("clip.add_lut is missing".into()))?;
            }
            Ok(format!("Applied the {name} LUT to {} clip{}", ids.len(), if ids.len() == 1 { "" } else { "s" }))
        }
        Step::Html(html) => app.add_html_clip(&html, Some(&name), at, None, &[]).map(|_| format!("Added {name}")),
        Step::Skill => Ok(format!("Installed the skill /{short}: it is in the Assistant's Skills")),
    }
}

pub(super) fn act(_app: &mut App, _a: Action) -> bool {
    false
}

pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    let st = &mut app.online;
    if let Some(rx) = &st.loading {
        let mut got = Vec::new();
        let done = loop {
            match rx.try_recv() {
                Ok(l) => got.push(l),
                Err(std::sync::mpsc::TryRecvError::Empty) => break false,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break true,
            }
        };
        for l in got {
            st.take_loaded(l);
        }
        if done {
            st.loading = None;
            crate::ui::gallery::forget_lut_listing();
        }
    }
    // thumbnails: results in, then start the next few
    if let Some((_, rx)) = &st.thumb_rx {
        while let Ok((id, path)) = rx.try_recv() {
            st.thumb_running -= 1;
            if let Some(t) = st.thumbs.get_mut(&id) {
                t.1 = path;
            }
        }
    }
    while st.thumb_running < THUMB_FETCHES && !st.thumb_wait.is_empty() {
        let id = st.thumb_wait.remove(0);
        let Some(rel) = st.thumbs.get(&id).map(|t| t.0.clone()) else { continue };
        let Some(src) = st.loaded_for.clone() else { break };
        let tx = st.thumb_rx.get_or_insert_with(std::sync::mpsc::channel).0.clone();
        st.thumb_running += 1;
        let _ = std::thread::Builder::new().name("library-thumb".into()).spawn(move || {
            let _ = tx.send((id, library::fetch_thumb(&src, &rel).ok()));
        });
    }
    // downloads
    let mut i = 0;
    while i < app.online.jobs.len() {
        if !app.online.jobs[i].prog.is_done() {
            i += 1;
            continue;
        }
        let j = app.online.jobs.remove(i);
        match j.prog.error() {
            Some(e) => app.toast(format!("{}: {}", app.online.name_of(&j.id), first_line(&e))),
            None => {
                app.online.installed.insert(j.id.clone());
                crate::ui::gallery::forget_lut_listing();
                finish(app, &j.id, j.then);
            }
        }
    }
    let st = &app.online;
    if st.loading.is_some() || !st.jobs.is_empty() || st.thumb_running > 0 {
        ctx.request_repaint_after(Duration::from_millis(150));
    }
}

pub(super) fn windows(app: &mut App, ctx: &egui::Context) {
    let Some((title, text)) = &app.online.view else { return };
    let mut open = true;
    egui::Window::new(title.as_str())
        .id(egui::Id::new("library_view_text"))
        .open(&mut open)
        .default_size([520.0, 420.0])
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(text.as_str()).monospace()).selectable(true));
            });
        });
    if !open {
        app.online.view = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_apply_dispatches_by_kind() {
        let p = Path::new("C:/lib/luts/warm.cube");
        assert_eq!(step(Kind::Transition, "t", "T", p), Step::Transition("t".into()));
        assert_eq!(step(Kind::Effect, "e", "E", p), Step::Effect("e".into()));
        assert_eq!(step(Kind::Lut, "", "Warm", p), Step::Lut(p.to_string_lossy().into_owned()));
        assert_eq!(step(Kind::Html, "<html>", "H", p), Step::Html("<html>".into()));
        assert_eq!(step(Kind::Skill, "---", "S", p), Step::Skill);
        let Step::Html(page) = step(Kind::Icon, "<svg viewBox=\"0 0 24 24\"><path d=\"M0 0\"/></svg>", "Bell", p)
        else {
            panic!("an icon is placed as an HTML layer")
        };
        assert!(page.contains("<path d=\"M0 0\"/>") && page.contains("Bell"), "{page}");
    }

    #[test]
    fn library_rows_merge_filter_and_queue_visible_thumbs_once() {
        let mut st = OnlineState::default();
        let e = |id: &str, name: &str| library::Entry {
            id: id.into(),
            name: name.into(),
            thumb: format!("thumbs/{}.jpg", id.replace('/', "-")),
            ..Default::default()
        };
        st.take_loaded(Loaded {
            cat: Ok(library::Catalogue { version: 1, items: vec![e("luts/warm", "Warm"), e("effects/glow", "Glow")] }),
            installed: ["luts/warm".to_string(), "luts/mine".to_string()].into_iter().collect(),
            thumbs: HashMap::new(),
        });
        let ids: Vec<String> = st.rows(Some(Kind::Lut), "").into_iter().map(|r| r.id).collect();
        assert_eq!(ids, ["luts/warm", "luts/mine"]);
        assert_eq!(st.rows(None, "glow").len(), 1);
        assert_eq!(st.name_of("luts/mine"), "mine");
        assert_eq!(st.thumb_path("effects/glow"), None);
        assert_eq!(st.thumb_path("effects/glow"), None);
        assert_eq!(st.thumb_wait, ["effects/glow"], "asked twice, fetched once");
        assert_eq!(st.thumb_path("luts/mine"), None, "an item of yours has no thumbnail");
        // a failed refresh keeps the catalogue and says so in one line
        st.take_loaded(Loaded { cat: Err("curl: timeout".into()), installed: HashSet::new(), thumbs: HashMap::new() });
        assert!(st.cat.is_some() && st.note.starts_with("Offline"));
        let mut empty = OnlineState::default();
        empty.take_loaded(Loaded { cat: Err("x\ny".into()), installed: HashSet::new(), thumbs: HashMap::new() });
        assert!(empty.note.contains("(x)"), "{}", empty.note);
    }
}
