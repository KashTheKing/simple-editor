//! ---- feat/reference-clips ----
//! Find a Reference in the app (`Pane::References`, drawn by `ui::reference_ui`): the browser search
//! buttons, the GIPHY / Tenor search on a worker (with the user's own key), result stills, GIF imports,
//! the yt-dlp link import with "Just a part" (shared with the Import URL window), marking what lands as a
//! reference (`Project::set_reference`: source kept, `reference` tag, the References smart bin), the
//! automatic transcription that makes it searchable by quote, Copy Credits and the `references.*` tools.
//!
//! Network calls only ever happen on a click or a tool call, one search or one file at a time: no
//! scraping, no bulk downloads. Nothing heavy runs on the UI thread (searches, stills, downloads and
//! whisper are all workers, polled here once per frame).

use super::feedback::Toast;
use super::*;
use crate::engine::references::{self, GifHit, Provider};
use crate::engine::transcribe;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::RefSource;
use crate::ui::reference_ui::{self, Asked, RefRow, RefUi, View};
use std::collections::HashSet;
use std::sync::mpsc::{channel, Receiver};

/// How many results one search asks for.
const LIMIT: u32 = 24;

#[derive(Default)]
pub(super) struct RefState {
    pub(super) ui: RefUi,
    /// The Import URL window's "Just a part".
    part: bool,
    part_text: String,
    /// A running search: (provider, query) and its reply.
    search: Option<(Provider, String, Receiver<Result<Vec<GifHit>, String>>)>,
    /// Result stills arriving from the still worker: (url, picture).
    stills: Option<Receiver<(String, Option<crate::media::Frame>)>>,
    asked_stills: HashSet<String>,
    /// yt-dlp downloads that become references when they land (matched by their progress).
    pending: Vec<(Arc<Progress>, RefSource)>,
    /// GIF file downloads: progress, where it goes, what it is.
    gifs: Vec<(Arc<Progress>, PathBuf, RefSource)>,
    /// References waiting for their probe before transcription.
    to_transcribe: Vec<Id>,
    transcribing: Vec<(Id, transcribe::Job)>,
}

// ---------------------------------------------------------------- the Import URL window's "Just a part"

/// The "Just a part" row on the Import URL window. True when Enter was pressed in its field.
pub(super) fn part_row(ui: &mut egui::Ui, st: &mut RefState, url: &str) -> bool {
    reference_ui::part_controls(ui, &mut st.part, &mut st.part_text, url)
}

/// The range "Just a part" asks for: None when it is off, Err when the text is not a range.
pub(super) fn part_range(st: &RefState) -> Result<Option<(f64, f64)>, String> {
    range_of(st.part, &st.part_text)
}

fn range_of(on: bool, text: &str) -> Result<Option<(f64, f64)>, String> {
    if !on {
        return Ok(None);
    }
    references::parse_range(text)
        .map(Some)
        .ok_or_else(|| format!("\"{}\" is not a time range - type it as start-end, e.g. 1:23-1:45", text.trim()))
}

// ---------------------------------------------------------------- downloads -> references

/// A yt-dlp download finished and was imported as `ids` (`actions.rs`): if it was a reference import, mark it.
pub(super) fn downloaded(app: &mut App, prog: &Arc<Progress>, meta: Option<(String, String)>, ids: &[Id]) {
    let Some(i) = app.references.pending.iter().position(|(p, _)| Arc::ptr_eq(p, prog)) else { return };
    let (_, mut r) = app.references.pending.remove(i);
    let Some(&id) = ids.last() else { return };
    if let Some((title, uploader)) = meta {
        if r.title.is_empty() {
            r.title = title;
        }
        if r.credit.is_empty() {
            r.credit = uploader;
        }
    }
    mark(app, id, r);
}

/// A download failed or was cancelled: drop its pending record.
pub(super) fn forget(app: &mut App, prog: &Arc<Progress>) {
    app.references.pending.retain(|(p, _)| !Arc::ptr_eq(p, prog));
}

/// `id` is a reference: record it (joins the import's undo step - it lands the same frame), then queue
/// its transcription.
fn mark(app: &mut App, id: Id, mut r: RefSource) {
    r.asset = id;
    let first = app.project.live_references().is_empty();
    app.project.set_reference(r);
    app.after_edit();
    if first {
        app.toast("Added to the References bin in the Library");
    }
    if app.settings.reference_transcribe && !app.references.to_transcribe.contains(&id) {
        app.references.to_transcribe.push(id);
    }
}

// ---------------------------------------------------------------- the pane

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::References {
        return false;
    }
    let busy: Vec<Id> = app.references.transcribing.iter().map(|(id, _)| *id).collect();
    let refs: Vec<RefRow> = app
        .project
        .live_references()
        .iter()
        .map(|r| RefRow {
            asset: r.asset,
            name: if r.title.is_empty() {
                app.project.asset(r.asset).map(|a| a.name()).unwrap_or_default()
            } else {
                r.title.clone()
            },
            site: r.site.clone(),
            quote: r.quote.clone(),
            transcribing: busy.contains(&r.asset) || app.references.to_transcribe.contains(&r.asset),
        })
        .collect();
    let view = View {
        has_key: [!app.settings.giphy_key.trim().is_empty(), !app.settings.tenor_key.trim().is_empty()],
        ytdlp: app.ytdlp_available.load(std::sync::atomic::Ordering::Relaxed),
        refs: &refs,
        library_open: app.pane_drawn(Pane::Library),
    };
    let asked = reference_ui::show(ui, &mut app.references.ui, &view);
    for a in asked {
        handle(app, ui.ctx(), a);
    }
    true
}

/// Imports wait for the one-time fair-use note.
fn needs_note(a: &Asked) -> bool {
    matches!(a, Asked::ImportGif(_) | Asked::ImportLink)
}

fn handle(app: &mut App, ctx: &egui::Context, a: Asked) {
    if needs_note(&a) && !app.settings.reference_note_seen {
        app.references.ui.note = Some(a);
        return;
    }
    match a {
        Asked::OpenSite(id) => {
            if let Some(s) = references::site(id) {
                crate::engine::browsers::open_url(&references::search_url(s, &app.references.ui.query));
            }
        }
        Asked::Search => start_search(app, ctx),
        Asked::ImportGif(i) => {
            if let Some(h) = app.references.ui.hits.get(i).cloned() {
                import_gif(app, &h);
            }
        }
        Asked::ImportLink => {
            let (link, on, text) = {
                let u = &app.references.ui;
                (u.link.trim().to_string(), u.part, u.part_text.clone())
            };
            match range_of(on, &text) {
                Err(e) => app.toast(e),
                Ok(range) => {
                    if import_link(app, &link, range).is_some() {
                        app.references.ui.link.clear();
                    }
                }
            }
        }
        Asked::CopyCredits => {
            ctx.copy_text(app.project.reference_credits());
            app.toast("Credits copied");
        }
        Asked::Select(id) => {
            app.library.selected = Some(id);
            app.library.tab = 0;
            app.surface(Pane::Library);
        }
        Asked::OpenSettings => app.settings_ui.open = true,
        Asked::ToggleLibrary => app.toggle_panel(Pane::Library),
        Asked::NoteOk => {
            app.settings.reference_note_seen = true;
            app.settings.save();
            if let Some(held) = app.references.ui.note.take() {
                handle(app, ctx, held);
            }
        }
    }
}

/// Start a yt-dlp download that becomes a reference. None when yt-dlp is missing (the app toasts why).
fn import_link(app: &mut App, link: &str, range: Option<(f64, f64)>) -> Option<Arc<Progress>> {
    if link.is_empty() {
        app.toast("Paste a link first");
        return None;
    }
    let prog = app.start_download(link, false, range)?;
    let r = RefSource { site: references::site_of_url(link), url: link.to_string(), range, ..Default::default() };
    app.references.pending.push((prog.clone(), r));
    Some(prog)
}

/// Download a GIF result's MP4 into the downloads folder (a worker), imported by `tick` when it lands.
fn import_gif(app: &mut App, h: &GifHit) {
    let dest = app.download_dir().join(references::gif_file_name(h));
    let r = RefSource {
        site: h.provider.name().to_string(),
        url: if h.page.is_empty() { h.mp4.clone() } else { h.page.clone() },
        title: h.title.clone(),
        credit: h.credit.clone(),
        ..Default::default()
    };
    if dest.is_file() {
        // downloaded before: import it again rather than fetching a second copy
        let ids = app.import_files(std::slice::from_ref(&dest));
        if let Some(&id) = ids.last() {
            mark(app, id, r);
            app.library.selected = Some(id);
        }
        return;
    }
    let (url, to) = (h.mp4.clone(), dest.clone());
    let prog = crate::engine::export::spawn_job("reference-gif", move |p| {
        p.set(0.0, "Downloading…");
        references::download(&url, &to, || p.is_cancelled())
    });
    let mut t = Toast::new(format!("Downloading {}…", if h.title.is_empty() { h.provider.name() } else { &h.title }));
    t.progress = Some(prog.clone());
    app.push_toast(t);
    app.references.gifs.push((prog, dest, r));
}

fn start_search(app: &mut App, ctx: &egui::Context) {
    let st = &mut app.references.ui;
    let p = st.provider;
    let key = match p {
        Provider::Giphy => app.settings.giphy_key.clone(),
        Provider::Tenor => app.settings.tenor_key.clone(),
    };
    let query = st.query.trim().to_string();
    if key.trim().is_empty() || references::clean_query(&query).is_empty() || st.searching {
        return;
    }
    st.searching = true;
    st.status.clear();
    let (tx, rx) = channel();
    let c = ctx.clone();
    let q = query.clone();
    std::thread::spawn(move || {
        let _ = tx.send(references::search(p, &key, &q, LIMIT));
        c.request_repaint();
    });
    app.references.search = Some((p, query, rx));
}

/// Fetch and decode the stills of `hits` on one worker, one after another (never a burst of requests).
fn fetch_stills(st: &mut RefState, ctx: &egui::Context) {
    let urls: Vec<String> = st
        .ui
        .hits
        .iter()
        .map(|h| h.still.clone())
        .filter(|u| u.starts_with("https://") && !st.asked_stills.contains(u) && !st.ui.thumbs.contains_key(u))
        .collect();
    if urls.is_empty() {
        return;
    }
    st.asked_stills.extend(urls.iter().cloned());
    let (tx, rx) = channel();
    let c = ctx.clone();
    let dir = crate::settings::Settings::cache_dir().join("references");
    std::thread::spawn(move || {
        for u in urls {
            let file = dir.join(still_name(&u));
            let ok = file.is_file() || references::download(&u, &file, || false).is_ok();
            let frame = if ok { crate::media::thumbs::wic_thumb(&file.to_string_lossy(), 168) } else { None };
            if tx.send((u, frame)).is_err() {
                return; // the pane moved on to another search
            }
            c.request_repaint();
        }
    });
    st.stills = Some(rx);
}

/// A cache file name for a still: a hash of its URL plus its extension.
fn still_name(url: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    let ext = url.rsplit('.').next().filter(|e| e.len() <= 4 && e.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or("img");
    format!("{:016x}.{ext}", h.finish())
}

// ---------------------------------------------------------------- per frame

/// FRAME_HOOK: search replies, stills, GIF downloads and reference transcriptions. Cheap when idle (a few
/// empty-vec checks); repaints come from the workers themselves.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    poll_search(app, ctx);
    poll_stills(app, ctx);
    poll_gifs(app);
    poll_transcripts(app);
}

fn poll_search(app: &mut App, ctx: &egui::Context) {
    let Some((p, q, rx)) = &app.references.search else { return };
    let Ok(res) = rx.try_recv() else { return };
    let (p, q) = (*p, q.clone());
    app.references.search = None;
    let st = &mut app.references.ui;
    st.searching = false;
    if st.provider != p {
        return; // switched service while it ran
    }
    match res {
        Ok(hits) => {
            st.status = if hits.is_empty() { format!("No {} results for \"{q}\"", p.name()) } else { String::new() };
            st.hits = hits;
            st.hits_for = q;
            fetch_stills(&mut app.references, ctx);
        }
        Err(e) => {
            st.hits.clear();
            st.status = e;
        }
    }
}

fn poll_stills(app: &mut App, ctx: &egui::Context) {
    let Some(rx) = &app.references.stills else { return };
    let mut got = Vec::new();
    while let Ok(x) = rx.try_recv() {
        got.push(x);
    }
    for (url, frame) in got {
        let Some(f) = frame else { continue };
        let (w, h) = (f.width as usize, f.height as usize);
        if w == 0 || h == 0 || f.rgba.len() != w * h * 4 {
            continue;
        }
        let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &f.rgba);
        let tex = ctx.load_texture(format!("ref-still-{url}"), img, egui::TextureOptions::LINEAR);
        app.references.ui.thumbs.insert(url, tex);
    }
    // keep only the current results' stills on the GPU
    let keep: HashSet<&str> = app.references.ui.hits.iter().map(|h| h.still.as_str()).collect();
    app.references.ui.thumbs.retain(|u, _| keep.contains(u.as_str()));
}

fn poll_gifs(app: &mut App) {
    if app.references.gifs.is_empty() {
        return;
    }
    let (done, running): (Vec<_>, Vec<_>) = std::mem::take(&mut app.references.gifs).into_iter().partition(|(p, _, _)| p.is_done());
    app.references.gifs = running;
    for (prog, dest, r) in done {
        match prog.error() {
            None => {
                let ids = app.import_files(std::slice::from_ref(&dest));
                if let Some(&id) = ids.last() {
                    mark(app, id, r);
                    app.library.selected = Some(id);
                    app.library.tab = 0;
                }
            }
            Some(_) if prog.is_cancelled() => app.toast("Download cancelled"),
            Some(e) => app.toast(format!("Download failed: {e}")),
        }
    }
}

fn poll_transcripts(app: &mut App) {
    if app.references.to_transcribe.is_empty() && app.references.transcribing.is_empty() {
        return;
    }
    // start the ones whose probe is in (a placeholder has no duration yet)
    let queued = std::mem::take(&mut app.references.to_transcribe);
    for id in queued {
        let Some(a) = app.project.asset(id) else { continue };
        if a.duration <= 0.0 {
            app.references.to_transcribe.push(id); // still probing
            continue;
        }
        let silent = a.audio_streams.is_empty() || !matches!(a.kind, ClipKind::Video | ClipKind::Audio);
        if silent || transcribe::exe().is_none() {
            continue; // nothing to hear (a GIF), or no whisper: the reference is still found by name / tag
        }
        let model = app.subtitles_ui.transcribe.model().1;
        if !transcribe::have_model(model) {
            continue;
        }
        let job = transcribe::start(transcribe::Options {
            path: a.path.clone(),
            src_start: 0.0,
            src_duration: a.duration,
            model: model.to_string(),
            language: app.subtitles_ui.transcribe.language.clone(),
            words: false,
            prompt: String::new(),
            islands: Vec::new(),
        });
        app.references.transcribing.push((id, job));
    }
    let (done, running): (Vec<_>, Vec<_>) =
        std::mem::take(&mut app.references.transcribing).into_iter().partition(|(_, j)| j.progress.is_done());
    app.references.transcribing = running;
    for (id, job) in done {
        if job.progress.error().is_some() {
            continue; // the row's tooltip says nothing was transcribed; the reference itself is fine
        }
        let text = job.segments().iter().map(|s| s.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");
        app.project.set_reference_quote(id, &text);
        app.after_edit();
    }
}

// ---------------------------------------------------------------- MCP tools

fn ref_json(p: &Project, r: &RefSource) -> Value {
    json!({
        "asset_id": r.asset,
        "name": p.asset(r.asset).map(|a| a.name()).unwrap_or_default(),
        "site": r.site,
        "url": r.url,
        "title": r.title,
        "credit": r.credit,
        "range": r.range.map(|(a, b)| json!([a, b])),
        "quote": r.quote,
    })
}

fn links_json(query: &str, sites: &[&references::Site]) -> Value {
    Value::Array(
        sites
            .iter()
            .map(|s| json!({"site": s.id, "name": s.name, "what": s.what, "url": references::search_url(s, query)}))
            .collect(),
    )
}

fn arg_str<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k).and_then(Value::as_str)
}

pub(crate) const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "references.suggest",
        desc: "Find a Reference for a line, reaction or scene: the project's own references whose transcribed quote / title matches, the guessed kind (quote / reaction / video), the search links best-first, and whether in-app GIPHY / Tenor search has a key. Never searches the web itself.",
        args: &["query:string:true:the line, reaction or scene"],
        kind: ToolKind::Read,
        run: |app, args| {
            let q = arg_str(args, "query").unwrap_or_default();
            if references::clean_query(q).is_empty() {
                return Err("query is empty".into());
            }
            let kind = match references::guess_kind(q) {
                references::Kind::Quote => "quote",
                references::Kind::Reaction => "reaction",
                references::Kind::Video => "video",
            };
            let have: Vec<Value> = app.project.find_references(q).iter().map(|r| ref_json(&app.project, r)).collect();
            Ok(ToolOutcome::Done(json!({
                "kind": kind,
                "in_project": have,
                "links": links_json(q, &references::suggest(q)),
                "gif_search": {"giphy": !app.settings.giphy_key.trim().is_empty(), "tenor": !app.settings.tenor_key.trim().is_empty()},
            })))
        },
    },
    ToolDef {
        name: "references.search_links",
        desc: "The search address on Yarn, PlayPhrase, Clip.cafe, Tenor, GIPHY and YouTube for a query (or one site), for the user to open. open=true opens it (or the best one) in the user's browser and shows the Find a Reference pane with the query.",
        args: &[
            "query:string:true:what to look for",
            "site:string:false:yarn|playphrase|clipcafe|tenor|giphy|youtube (default: all, best first)",
            "open:boolean:false:open it in the browser",
        ],
        kind: ToolKind::Ui,
        run: |app, args| {
            let q = arg_str(args, "query").unwrap_or_default();
            if references::clean_query(q).is_empty() {
                return Err("query is empty".into());
            }
            let sites: Vec<&references::Site> = match arg_str(args, "site") {
                Some(s) => vec![references::site(s).ok_or_else(|| format!("unknown site '{s}' (yarn, playphrase, clipcafe, tenor, giphy, youtube)"))?],
                None => references::suggest(q),
            };
            if args.get("open").and_then(Value::as_bool) == Some(true) {
                crate::engine::browsers::open_url(&references::search_url(sites[0], q));
                app.references.ui.query = q.to_string();
                app.surface(Pane::References);
            }
            Ok(ToolOutcome::Done(json!({"links": links_json(q, &sites)})))
        },
    },
    ToolDef {
        name: "references.import_range",
        desc: "Import a link with yt-dlp as a reference clip: only start..end (seconds, or range \"1:23-1:45\") when given, else the whole thing. The source, title and uploader are kept for Copy Credits, it is tagged 'reference' (the References bin) and transcribed so it is found by quote. Downloads one link the user chose (the guard asks first); never searches or bulk-downloads. Replies when the download is done; the import lands the next frame.",
        args: &[
            "url:string:true:the page or file link",
            "start:number:false:seconds",
            "end:number:false:seconds",
            "range:string:false:\"1:23-1:45\" (instead of start / end)",
        ],
        kind: ToolKind::Job,
        run: |app, args| {
            let url = arg_str(args, "url").unwrap_or_default().trim().to_string();
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err("url must be an http(s) link".into());
            }
            let range = match (arg_str(args, "range"), args.get("start").and_then(Value::as_f64), args.get("end").and_then(Value::as_f64)) {
                (Some(r), _, _) => Some(references::parse_range(r).ok_or_else(|| format!("'{r}' is not a range like 1:23-1:45"))?),
                (None, Some(a), Some(b)) if b > a && a >= 0.0 => Some((a, b)),
                (None, None, None) => None,
                _ => return Err("give both start and end (end after start), or neither".into()),
            };
            if !app.ytdlp_available.load(std::sync::atomic::Ordering::Relaxed) {
                return Err("yt-dlp is not installed (Settings ▸ General ▸ yt-dlp folder)".into());
            }
            let prog = import_link(app, &url, range).ok_or("the download did not start")?;
            Ok(ToolOutcome::Job(prog, app.download_dir()))
        },
    },
    ToolDef {
        name: "references.list",
        desc: "The project's reference clips: source site and link, title, uploader, the part downloaded and the transcribed quote. query filters by quote / title / file name.",
        args: &["query:string:false:filter"],
        kind: ToolKind::Read,
        run: |app, args| {
            let q = arg_str(args, "query").unwrap_or_default();
            let list: Vec<Value> = app.project.find_references(q).iter().map(|r| ref_json(&app.project, r)).collect();
            Ok(ToolOutcome::Done(json!({"references": list})))
        },
    },
    ToolDef {
        name: "references.set",
        desc: "Mark a library asset as a reference or edit its record: site, url, title, credit, quote. Fields left out keep their value. Tags it 'reference' and makes the References smart bin if needed.",
        args: &[
            "asset_id:integer:true:library asset",
            "site:string:false:",
            "url:string:false:",
            "title:string:false:",
            "credit:string:false:",
            "quote:string:false:what is said in it",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = args.get("asset_id").and_then(Value::as_u64).ok_or("asset_id is required")?;
            if app.project.asset(id).is_none() {
                return Err(format!("no asset {id}"));
            }
            let mut r = app.project.reference(id).cloned().unwrap_or(RefSource { asset: id, ..Default::default() });
            for (k, f) in [("site", &mut r.site), ("url", &mut r.url), ("title", &mut r.title), ("credit", &mut r.credit)] {
                if let Some(v) = arg_str(args, k) {
                    *f = v.trim().to_string();
                }
            }
            app.project.set_reference(r);
            if let Some(q) = arg_str(args, "quote") {
                app.project.set_reference_quote(id, q);
            }
            Ok(ToolOutcome::Done(ref_json(&app.project, app.project.reference(id).expect("just set"))))
        },
    },
    ToolDef {
        name: "references.credits",
        desc: "Copy Credits as text: one line per reference (title - site (uploader), part: link).",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| Ok(ToolOutcome::Done(json!({"text": app.project.reference_credits()}))),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_range_reads_only_when_ticked() {
        assert_eq!(range_of(false, "garbage"), Ok(None));
        assert_eq!(range_of(true, "1:23-1:45"), Ok(Some((83.0, 105.0))));
        assert!(range_of(true, "1:23").unwrap_err().contains("start-end"));
    }

    #[test]
    fn imports_wait_for_the_note_but_browsing_does_not() {
        assert!(needs_note(&Asked::ImportLink) && needs_note(&Asked::ImportGif(0)));
        assert!(!needs_note(&Asked::OpenSite("yarn")) && !needs_note(&Asked::Search) && !needs_note(&Asked::CopyCredits));
    }

    #[test]
    fn still_names_are_stable_and_safe() {
        let a = still_name("https://media.giphy.com/media/abc/200_s.gif");
        assert_eq!(a, still_name("https://media.giphy.com/media/abc/200_s.gif"));
        assert!(a.ends_with(".gif") && !a.contains('/'));
        assert!(still_name("https://x/y?z=1").ends_with(".img"));
    }

    #[test]
    fn tools_are_registered_once() {
        for n in ["references.suggest", "references.search_links", "references.import_range", "references.list", "references.set", "references.credits"] {
            assert_eq!(crate::mcp::tools::all().filter(|t| t.name == n).count(), 1, "{n}");
        }
        assert!(crate::mcp::tools::leaves_project("references.import_range", &json!({})), "a download waits for Allow");
    }
}
