//! ---- ws:music-burn ----
//! App side of the music features (`ui::music_ui`): the Playlists pane, the Inspector's Tags section
//! for library songs / videos, "Make visualizer video", Burn to Disc, and their tools (`playlist.*`,
//! `media.tags_get` / `media.tags_set`, `disc.*`). Slow work (ffprobe / ffmpeg / IMAPI) runs on
//! threads that hand results back through `DONE`; the frame hook applies them.

use super::tools_helpers::*;
use super::*;
use crate::engine::disc::{self, imapi, BurnContent, BurnOptions, BurnTarget, CdTrack, DataItem, DiscSize};
use crate::engine::export::Progress;
use crate::engine::tags::{self, CoverEdit, MediaTags, TagEdit};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::music_ui::{self as mu, BurnAct, DiscKind, PlAct, TagAct};
use std::sync::{Arc, Mutex};

/// Results from worker threads, applied on the UI thread by `frame`.
enum Done {
    Tags { ids: Vec<Id>, read: Vec<(Id, Result<MediaTags, String>)>, cover: Option<PathBuf> },
    Saved { ids: Vec<Id>, result: Result<Vec<(Id, MediaTags)>, String> },
    Titles(Vec<(Id, String, String)>),
    Drives(Result<Vec<imapi::Drive>, String>),
    Media(Result<imapi::MediaInfo, String>),
    Visualizer { at: f64, songs: Vec<(Id, Option<crate::model::Asset>, String)> },
    Note(String),
}

static DONE: Mutex<Vec<Done>> = Mutex::new(Vec::new());
/// Running burns / erases / visualizer preps, for the Jobs pane.
static JOBS: Mutex<Vec<(String, Arc<Progress>)>> = Mutex::new(Vec::new());
/// Assets whose titles were asked for (read once per session).
static ASKED: Mutex<Vec<Id>> = Mutex::new(Vec::new());

fn send(d: Done, ctx: Option<&egui::Context>) {
    DONE.lock().unwrap_or_else(|e| e.into_inner()).push(d);
    if let Some(c) = ctx {
        c.request_repaint();
    }
}

/// Run `f` on a thread; its result comes back through `DONE` and wakes the UI.
fn spawn(ctx: &egui::Context, name: &str, f: impl FnOnce() -> Done + Send + 'static) {
    let c = ctx.clone();
    let _ = std::thread::Builder::new().name(name.into()).spawn(move || {
        let d = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Done::Note("That failed unexpectedly".into()));
        send(d, Some(&c));
    });
}

pub(super) fn progress_jobs() -> Vec<super::jobs_pane::ProgJob> {
    JOBS.lock()
        .map(|j| j.iter().map(|(l, p)| (crate::ui::jobs_ui::JobKind::Media, l.clone(), p.clone())).collect())
        .unwrap_or_default()
}

fn add_job(label: String, p: Arc<Progress>) {
    if let Ok(mut j) = JOBS.lock() {
        j.push((label, p));
    }
}

/// A library asset that has tags (a sound or video file, not a subclip / HTML / still).
fn is_media(a: &crate::model::Asset) -> bool {
    matches!(a.kind, ClipKind::Audio | ClipKind::Video) && a.html.is_none() && a.parent.is_none()
}

fn media_sel(app: &App) -> Vec<Id> {
    app.library.sel_ids.iter().copied().filter(|&id| app.project.asset(id).is_some_and(is_media)).collect()
}

// ------------------------------------------------------------------------------------------------ frame

pub(super) fn frame(app: &mut App, ctx: &egui::Context) {
    // which was clicked last: the Library or the timeline
    let m = &mut app.music;
    if m.ctx.is_none() {
        m.ctx = Some(ctx.clone());
    }
    if app.library.sel_ids != m.seen_lib {
        m.seen_lib = app.library.sel_ids.clone();
        m.lib_focus = !m.seen_lib.is_empty();
    }
    if app.selection != m.seen_sel {
        m.seen_sel = app.selection.clone();
        if !m.seen_sel.is_empty() {
            m.lib_focus = false;
        }
    }
    let done: Vec<Done> = std::mem::take(&mut *DONE.lock().unwrap_or_else(|e| e.into_inner()));
    for d in done {
        apply(app, ctx, d);
    }
    // finished jobs
    let finished: Vec<(String, Arc<Progress>)> = {
        let mut j = JOBS.lock().unwrap_or_else(|e| e.into_inner());
        let (f, keep): (Vec<_>, Vec<_>) = j.drain(..).partition(|(_, p)| p.is_done());
        *j = keep;
        f
    };
    for (label, p) in finished {
        let text = match p.error() {
            Some(e) if e == crate::engine::export::CANCELLED => format!("{label}: cancelled"),
            Some(e) => format!("{label}: {e}"),
            None => p.status(),
        };
        if app.music.burn.running.as_ref().is_some_and(|r| Arc::ptr_eq(r, &p)) {
            app.music.burn.running = None;
            app.music.burn.result = text.clone();
            app.music.burn.media = None;
        }
        app.toast(text);
    }
    // the Inspector's tags follow the library selection
    if app.music.lib_focus && app.pane_drawn(Pane::Inspector) {
        let ids = media_sel(app);
        if ids != app.music.tags.ids && !app.music.tags.saving {
            load_tags(app, ctx, ids);
        }
    }
    // the shown playlist's titles
    if app.pane_drawn(Pane::Playlists) {
        if let Some(pl) = app.music.playlist.and_then(|id| app.project.playlist(id)) {
            let want: Vec<(Id, String)> = {
                let asked = ASKED.lock().unwrap_or_else(|e| e.into_inner());
                pl.items
                    .iter()
                    .filter(|id| !asked.contains(id))
                    .filter_map(|&id| app.project.asset(id).map(|a| (id, a.path.clone())))
                    .collect()
            };
            if !want.is_empty() {
                ASKED.lock().unwrap_or_else(|e| e.into_inner()).extend(want.iter().map(|w| w.0));
                spawn(ctx, "tags-titles", move || {
                    Done::Titles(
                        want.into_iter()
                            .filter_map(|(id, p)| tags::read(Path::new(&p)).ok().map(|t| (id, t.get("artist").to_string(), t.get("title").to_string())))
                            .collect(),
                    )
                });
            }
        }
    }
}

fn apply(app: &mut App, ctx: &egui::Context, d: Done) {
    match d {
        Done::Tags { ids, read, cover } => {
            if ids != app.music.tags.ids {
                return; // the selection moved on
            }
            remember_titles(app, &read);
            let t = &mut app.music.tags;
            t.loading = false;
            t.loaded = read;
            t.preview = None;
            t.preview_for = None;
            if let Some(c) = cover {
                set_preview(ctx, t, &c);
            }
        }
        Done::Saved { ids, result } => {
            app.music.tags.saving = false;
            match result {
                Ok(read) => {
                    let n = read.len();
                    let read: Vec<(Id, Result<MediaTags, String>)> = read.into_iter().map(|(i, t)| (i, Ok(t))).collect();
                    remember_titles(app, &read);
                    if ids == app.music.tags.ids {
                        app.music.tags.loaded = read;
                        app.music.tags.reset();
                        app.music.tags.status = format!("Saved to {n} file{}", if n == 1 { "" } else { "s" });
                    }
                    // the picture may have changed: read the cover again
                    app.music.tags.ids.clear();
                }
                Err(e) => {
                    app.music.tags.status = e.clone();
                    app.toast(e);
                }
            }
        }
        Done::Titles(v) => {
            for (id, artist, title) in v {
                app.music.titles.insert(id, (artist, title));
            }
        }
        Done::Drives(r) => {
            let b = &mut app.music.burn;
            b.drives_loading = false;
            match r {
                Ok(d) => {
                    b.drives_error.clear();
                    if b.drive.is_some_and(|i| i >= d.len()) {
                        b.drive = None;
                    }
                    b.drives = d;
                }
                Err(e) => b.drives_error = e,
            }
        }
        Done::Media(r) => {
            app.music.burn.media_loading = false;
            app.music.burn.media = Some(r);
        }
        Done::Visualizer { at, songs } => make_visualizer(app, at, songs),
        Done::Note(s) => app.toast(s),
    }
}

fn remember_titles(app: &mut App, read: &[(Id, Result<MediaTags, String>)]) {
    for (id, r) in read {
        if let Ok(t) = r {
            app.music.titles.insert(*id, (t.get("artist").to_string(), t.get("title").to_string()));
        }
    }
}

fn set_preview(ctx: &egui::Context, t: &mut mu::TagState, path: &Path) {
    if let Some(f) = crate::media::thumbs::wic_thumb(&path.to_string_lossy(), 192) {
        let img = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
        let tex = ctx.load_texture("tag-cover", img, egui::TextureOptions::LINEAR);
        t.preview = Some((path.to_path_buf(), tex));
        t.preview_for = Some(path.to_path_buf());
    }
}

fn load_tags(app: &mut App, ctx: &egui::Context, ids: Vec<Id>) {
    let t = &mut app.music.tags;
    t.ids = ids.clone();
    t.loaded.clear();
    t.reset();
    t.preview = None;
    if ids.is_empty() {
        t.loading = false;
        return;
    }
    t.loading = true;
    let files: Vec<(Id, String)> = ids.iter().filter_map(|&id| app.project.asset(id).map(|a| (id, a.path.clone()))).collect();
    spawn(ctx, "tags-read", move || {
        let read: Vec<(Id, Result<MediaTags, String>)> = files.iter().map(|(id, p)| (*id, tags::read(Path::new(p)))).collect();
        let cover = files
            .iter()
            .zip(&read)
            .find(|(_, (_, r))| r.as_ref().is_ok_and(|t| t.cover.is_some()))
            .and_then(|((_, p), _)| tags::extract_cover(Path::new(p), &tags::covers_dir()).ok().flatten());
        Done::Tags { ids, read, cover }
    });
}

// ------------------------------------------------------------------------------------------------ panes

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    match pane {
        Pane::Playlists => {
            playlists(app, ui);
            true
        }
        Pane::Inspector if app.music.lib_focus && !media_sel(app).is_empty() => {
            egui::ScrollArea::vertical().id_salt("tags_scroll").show(ui, |ui| tags_inspector(app, ui));
            true
        }
        _ => false,
    }
}

fn row_label(app: &App, id: Id) -> String {
    let (artist, title) = app.music.titles.get(&id).cloned().unwrap_or_default();
    let name = app.project.asset(id).map(|a| app.project.asset_display_name(a)).unwrap_or_else(|| "(missing)".into());
    match (artist.is_empty(), title.is_empty()) {
        (_, true) => name,
        (true, false) => title,
        (false, false) => format!("{title} · {artist}"),
    }
}

fn playlists(app: &mut App, ui: &mut egui::Ui) {
    if app.music.playlist.is_none_or(|id| app.project.playlist(id).is_none()) {
        app.music.playlist = app.project.playlists.first().map(|p| p.id);
    }
    let lists: Vec<(Id, String)> = app.project.playlists.iter().map(|p| (p.id, p.name.clone())).collect();
    let (rows, total, spacing) = match app.music.playlist.and_then(|id| app.project.playlist(id)) {
        Some(p) => (
            p.items
                .iter()
                .map(|&a| mu::PlRow { label: row_label(app, a), secs: app.project.asset(a).map_or(0.0, |x| x.duration) })
                .collect::<Vec<_>>(),
            app.project.playlist_duration(p.id),
            (p.gap, p.crossfade),
        ),
        None => (Vec::new(), 0.0, (0.0, 0.0)),
    };
    let styles: Vec<String> = crate::engine::visualizer::style_names();
    if app.music.vis_style.is_empty() {
        app.music.vis_style = app.settings.visualizer_style.clone();
    }
    let lib_sel = media_sel(app).len();
    let act = mu::playlists_pane(ui, &mut app.music, &lists, &rows, total, spacing, &styles, lib_sel);
    if let Some(a) = act {
        pl_act(app, ui.ctx(), a);
    }
}

fn edit(app: &mut App, f: impl FnOnce(&mut Project) -> bool) {
    let before = app.project.to_json();
    if f(&mut app.project) {
        app.push_undo_labeled(before, "Playlist");
        app.dirty = true;
    }
}

fn pl_act(app: &mut App, ctx: &egui::Context, a: PlAct) {
    let cur = app.music.playlist;
    match a {
        PlAct::New => {
            let before = app.project.to_json();
            let id = app.project.add_playlist("");
            app.push_undo_labeled(before, "New playlist");
            app.dirty = true;
            app.music.playlist = Some(id);
            app.music.rename = Some((id, app.project.playlist(id).map(|p| p.name.clone()).unwrap_or_default()));
        }
        PlAct::Pick(id) => app.music.playlist = Some(id),
        PlAct::Rename(id, n) => edit(app, |p| p.rename_playlist(id, &n)),
        PlAct::Delete(id) => {
            let name = app.project.playlist(id).map(|p| p.name.clone()).unwrap_or_default();
            edit(app, |p| p.delete_playlist(id));
            app.toast_undo(format!("Deleted playlist {name}"), Action::Undo);
        }
        PlAct::Add { assets, at } => {
            if let Some(id) = cur {
                let mut n = 0;
                edit(app, |p| {
                    n = p.playlist_add(id, &assets, at);
                    n > 0
                });
                if n < assets.len() {
                    app.toast("Only sound and video files go in a playlist");
                }
            }
        }
        PlAct::AddSelection => {
            let ids = media_sel(app);
            pl_act(app, ctx, PlAct::Add { assets: ids, at: None });
        }
        PlAct::Remove(i) => {
            if let Some(id) = cur {
                edit(app, |p| p.playlist_remove(id, &[i]) > 0);
            }
        }
        PlAct::Move { from, to } => {
            if let Some(id) = cur {
                edit(app, |p| p.playlist_move(id, from, to));
            }
        }
        PlAct::Spacing { gap, crossfade } => {
            if let Some(id) = cur {
                edit(app, |p| p.playlist_set_spacing(id, gap, crossfade));
            }
        }
        PlAct::Play(i) => {
            if let Some(items) = cur.and_then(|id| app.project.playlist(id)).map(|p| p.items.clone()) {
                play(app, items.into_iter().skip(i).collect());
            }
        }
        PlAct::ToTimeline => {
            if let Some(id) = cur {
                let at = app.playhead;
                let before = app.project.to_json();
                let ids = app.project.playlist_to_timeline(id, at);
                if !ids.is_empty() {
                    app.push_undo_labeled(before, "Playlist to timeline");
                    app.after_edit();
                    app.selection = ids;
                }
            }
        }
        PlAct::Visualizer => {
            if let Some(items) = cur.and_then(|id| app.project.playlist(id)).map(|p| p.items.clone()) {
                start_visualizer(app, ctx, items);
            }
        }
        PlAct::ExportM3u8 => {
            if let Some(id) = cur {
                let name = app.project.playlist(id).map(|p| p.name.clone()).unwrap_or_default();
                if let Some(path) = rfd::FileDialog::new().set_file_name(format!("{name}.m3u8")).add_filter("Playlist", &["m3u8", "m3u"]).save_file() {
                    match export_m3u8(app, id, &path) {
                        Ok(()) => app.toast_with_folder(format!("Saved {}", path.display()), path.clone()),
                        Err(e) => app.toast(e),
                    }
                }
            }
        }
        PlAct::Burn => open_burn(app, ctx, cur),
        PlAct::OpenLibrary => app.surface(Pane::Library),
    }
}

/// Play `items` back to back in the Source monitor (its Source Tape).
fn play(app: &mut App, items: Vec<Id>) {
    if items.is_empty() {
        return;
    }
    app.source_pending = Some(super::source_pane::Pending::Tape(items));
    app.source_focus = true;
    app.surface(Pane::Source);
}

fn titles_fn(app: &App) -> impl Fn(Id) -> Option<String> + '_ {
    move |id| {
        app.music.titles.get(&id).and_then(|(a, t)| match (a.is_empty(), t.is_empty()) {
            (_, true) => None,
            (true, false) => Some(t.clone()),
            (false, false) => Some(format!("{a} - {t}")),
        })
    }
}

fn export_m3u8(app: &mut App, id: Id, path: &Path) -> Result<(), String> {
    // an export is a deliberate act: read the tags of songs the Playlists pane has not looked at yet
    let missing: Vec<(Id, String)> = app
        .project
        .playlist(id)
        .map(|p| p.items.iter().filter(|i| !app.music.titles.contains_key(i)).copied().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| app.project.asset(i).map(|a| (i, a.path.clone())))
        .collect();
    for (i, p) in missing {
        if let Ok(t) = tags::read(Path::new(&p)) {
            app.music.titles.insert(i, (t.get("artist").to_string(), t.get("title").to_string()));
        }
    }
    let titles = titles_fn(app);
    let text = app.project.playlist_m3u8(id, path.parent(), &titles).ok_or("no such playlist")?;
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

// ------------------------------------------------------------------------------------------------ tags

fn tags_inspector(app: &mut App, ui: &mut egui::Ui) {
    let ids = media_sel(app);
    let kinds = mu::Kinds {
        audio: ids.iter().any(|&id| app.project.asset(id).is_some_and(|a| !a.has_video())),
        video: ids.iter().any(|&id| app.project.asset(id).is_some_and(|a| a.has_video())),
    };
    let can_frame = frame_source(app).is_some();
    let act = mu::tags_section(ui, &mut app.music.tags, kinds, ids.len(), can_frame);
    match act {
        Some(TagAct::Save) => save_tags(app, ui.ctx()),
        Some(TagAct::Revert) => {
            app.music.tags.reset();
            app.music.tags.ids.clear(); // read again (and the picture)
        }
        Some(TagAct::CoverFile) => {
            if let Some(p) = rfd::FileDialog::new().add_filter("Pictures", &["jpg", "jpeg", "png", "webp", "bmp"]).pick_file() {
                set_preview(ui.ctx(), &mut app.music.tags, &p);
                app.music.tags.cover = CoverEdit::Set(p);
            }
        }
        Some(TagAct::CoverFrame) => {
            if let Some((path, t)) = frame_source(app) {
                let out = tags::covers_dir().join(format!("frame-{}.jpg", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis())));
                match tags::grab_frame(&path, t, &out) {
                    Ok(()) => {
                        set_preview(ui.ctx(), &mut app.music.tags, &out);
                        app.music.tags.cover = CoverEdit::Set(out);
                    }
                    Err(e) => app.toast(format!("Can't take that frame: {e}")),
                }
            }
        }
        Some(TagAct::CoverRemove) => app.music.tags.cover = CoverEdit::Remove,
        None => {}
    }
}

/// The video open in the Source monitor and its playhead (a cover frame comes from there).
fn frame_source(app: &App) -> Option<(PathBuf, f64)> {
    let s = app.source.as_ref().filter(|s| s.has_video && !s.is_image && s.tape.is_none())?;
    Some((s.path.clone(), s.player.time()))
}

fn save_tags(app: &mut App, ctx: &egui::Context) {
    let t = &mut app.music.tags;
    if !t.dirty() {
        return;
    }
    let edit = TagEdit { set: t.draft.clone(), cover: t.cover.clone() };
    t.saving = true;
    t.status = "Saving".into();
    let ids = t.ids.clone();
    let files: Vec<(Id, String)> = ids.iter().filter_map(|&id| app.project.asset(id).map(|a| (id, a.path.clone()))).collect();
    spawn(ctx, "tags-write", move || {
        let mut out = Vec::new();
        for (id, p) in &files {
            match tags::write(Path::new(p), &edit) {
                Ok(t) => out.push((*id, t)),
                Err(e) => {
                    let name = Path::new(p).file_name().map_or_else(|| p.clone(), |n| n.to_string_lossy().into_owned());
                    let done = if out.is_empty() { String::new() } else { format!(" ({} saved before it)", out.len()) };
                    return Done::Saved { ids, result: Err(format!("{name}: {e}{done}")) };
                }
            }
        }
        Done::Saved { ids, result: Ok(out) }
    });
}

// ------------------------------------------------------------------------------------------------ visualizer video

/// Read tags and covers off the UI thread (and warm the song analysis), then build on the UI thread.
fn start_visualizer(app: &mut App, ctx: &egui::Context, items: Vec<Id>) {
    let songs: Vec<(Id, String)> = items.iter().filter_map(|&id| app.project.asset(id).filter(|a| !a.audio_streams.is_empty()).map(|a| (id, a.path.clone()))).collect();
    if songs.is_empty() {
        app.toast("No songs with sound to make a video of");
        return;
    }
    let names: Vec<String> = songs.iter().map(|(id, _)| app.project.asset(*id).map(|a| app.project.asset_display_name(a)).unwrap_or_default()).collect();
    let backend = app.backend();
    let at = app.playhead;
    let prog = Progress::new();
    add_job("Visualizer video · reading songs".into(), prog.clone());
    spawn(ctx, "vis-prep", move || {
        let n = songs.len();
        let mut out = Vec::new();
        for (i, ((id, path), name)) in songs.into_iter().zip(names).enumerate() {
            prog.set(i as f32 / n as f32, format!("Song {} of {n}", i + 1));
            let p = Path::new(&path);
            let t = tags::read(p).unwrap_or_default();
            let title = crate::engine::mixvideo::title_line(t.get("artist"), if t.get("title").is_empty() { &name } else { t.get("title") });
            let cover = tags::extract_cover(p, &tags::covers_dir())
                .ok()
                .flatten()
                .and_then(|c| crate::media::probe(&c.to_string_lossy(), backend).ok());
            let _ = crate::engine::visualizer::analysis(&path, 0, backend);
            out.push((id, cover, title));
        }
        prog.set(1.0, "Visualizer video: songs read");
        prog.finish(None);
        Done::Visualizer { at, songs: out }
    });
}

fn make_visualizer(app: &mut App, at: f64, songs: Vec<(Id, Option<crate::model::Asset>, String)>) {
    let before = app.project.to_json();
    let mut list = Vec::new();
    for (id, cover, title) in songs {
        let cover = cover.map(|mut a| {
            a.folder = "Covers".into();
            app.project.add_asset(a)
        });
        list.push(crate::engine::mixvideo::Song { asset: id, cover, title });
    }
    let style = crate::engine::visualizer::last_used(&app.music.vis_style);
    let gap = app.music.playlist.and_then(|id| app.project.playlist(id)).map_or((0.0, 0.0), |p| (p.gap, p.crossfade));
    let opts = crate::engine::mixvideo::MixOptions { gap: gap.0, crossfade: gap.1, cover: app.music.vis_cover, title_card: app.music.vis_title };
    let backend = app.backend();
    match crate::engine::mixvideo::build(&mut app.project, &list, at, &style, &opts, backend) {
        Ok(made) => {
            app.push_undo_labeled(before, "Make visualizer video");
            if app.settings.visualizer_style != style.name {
                app.settings.visualizer_style = style.name.clone();
                app.settings.save();
            }
            app.after_edit();
            app.selection = made.iter().map(|m| m.visualizer).collect();
            app.toast(format!("Visualizer video: {} song{}", made.len(), if made.len() == 1 { "" } else { "s" }));
        }
        Err(e) => {
            app.project = Project::from_json(&before).unwrap_or_else(|_| app.project.clone());
            app.toast(e);
        }
    }
}

// ------------------------------------------------------------------------------------------------ burn

fn open_burn(app: &mut App, ctx: &egui::Context, playlist: Option<Id>) {
    let b = &mut app.music.burn;
    b.open = true;
    b.result.clear();
    if playlist.is_some() {
        b.playlist = playlist;
    }
    if b.title.is_empty() {
        b.title = b.playlist.and_then(|id| app.project.playlist(id)).map(|p| p.name.clone()).unwrap_or_default();
    }
    refresh_drives(app, ctx);
}

fn refresh_drives(app: &mut App, ctx: &egui::Context) {
    if app.music.burn.drives_loading {
        return;
    }
    app.music.burn.drives_loading = true;
    spawn(ctx, "disc-drives", || Done::Drives(imapi::drives()));
}

fn check_disc(app: &mut App, ctx: &egui::Context) {
    let Some(d) = app.music.burn.drive.and_then(|i| app.music.burn.drives.get(i)).cloned() else { return };
    app.music.burn.media_loading = true;
    spawn(ctx, "disc-media", move || Done::Media(imapi::media_info(&d.id)));
}

/// The audio CD a playlist makes.
fn audio_cd(app: &App, pl: Id, title: &str, performer: &str, gap: f64) -> Option<disc::AudioCd> {
    let p = app.project.playlist(pl)?;
    let tracks = p
        .items
        .iter()
        .filter_map(|&id| app.project.asset(id).map(|a| (id, a)))
        .map(|(id, a)| {
            let (artist, t) = app.music.titles.get(&id).cloned().unwrap_or_default();
            CdTrack {
                path: PathBuf::from(&a.path),
                title: if t.is_empty() { app.project.asset_display_name(a) } else { t },
                performer: if artist.is_empty() { performer.to_string() } else { artist },
                seconds: a.duration,
            }
        })
        .collect();
    Some(disc::AudioCd { title: title.to_string(), performer: performer.to_string(), tracks, gap })
}

/// The data disc: each playlist's songs in its own folder with an `.m3u8`, then the extra files.
fn data_items(app: &App, pl: Option<Id>, extra: &[PathBuf]) -> Result<Vec<DataItem>, String> {
    let mut items = Vec::new();
    if let Some(p) = pl.and_then(|id| app.project.playlist(id)) {
        let folder = sanitize(&p.name);
        let mut seen = Vec::new();
        let mut entries = Vec::new();
        for &id in &p.items {
            let Some(a) = app.project.asset(id) else { continue };
            let src = PathBuf::from(&a.path);
            let file = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let title = titles_fn(app)(id).unwrap_or_else(|| app.project.asset_display_name(a));
            entries.push((title, a.duration, file.clone()));
            if !seen.contains(&file) {
                seen.push(file);
                items.push(DataItem { src, dest: folder.clone() });
            }
        }
        let dir = crate::settings::Settings::cache_dir().join("burn");
        let _ = std::fs::create_dir_all(&dir);
        let m3u = dir.join(format!("{folder}.m3u8"));
        std::fs::write(&m3u, disc::disc_m3u8(&entries)).map_err(|e| e.to_string())?;
        items.push(DataItem { src: m3u, dest: folder });
    }
    items.extend(extra.iter().map(|p| DataItem { src: p.clone(), dest: String::new() }));
    Ok(items)
}

fn sanitize(s: &str) -> String {
    let t: String = s.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c }).collect();
    let t = t.trim().trim_end_matches('.').to_string();
    if t.is_empty() {
        "Playlist".into()
    } else {
        t
    }
}

fn summary(app: &App) -> mu::BurnSummary {
    let b = &app.music.burn;
    let mut s = mu::BurnSummary { audio_secs: 0.0, data_bytes: 0, problem: None };
    match b.kind {
        DiscKind::Audio => match b.playlist.and_then(|id| audio_cd(app, id, &b.title, &b.performer, b.gap)) {
            Some(cd) if !cd.tracks.is_empty() => {
                let lens: Vec<u64> = cd.tracks.iter().map(|t| disc::sectors_for_secs(t.seconds)).collect();
                let places = disc::layout(&lens, cd.gap);
                s.audio_secs = disc::total_sectors(&places) as f64 / 75.0;
                s.problem = disc::check_audio_capacity(&places, b.size).err();
            }
            _ => s.problem = Some("Pick a playlist with songs".into()),
        },
        DiscKind::Data => {
            let pl_bytes: u64 = b
                .playlist
                .and_then(|id| app.project.playlist(id))
                .map(|p| p.items.iter().filter_map(|&id| app.project.asset(id)).map(|a| file_len(&a.path)).sum())
                .unwrap_or(0);
            s.data_bytes = pl_bytes + b.extra.iter().map(|p| cached_tree_bytes(p)).sum::<u64>();
            if s.data_bytes == 0 && b.extra.is_empty() && b.playlist.is_none() {
                s.problem = Some("Pick a playlist or add files".into());
            } else {
                s.problem = disc::check_data_capacity(s.data_bytes.div_ceil(2048) + 1024, b.size).err();
            }
        }
    }
    if let Some(Ok(m)) = b.drive.and(b.media.as_ref()) {
        let r = if b.kind == DiscKind::Audio { m.check_audio() } else { m.check_data() };
        if s.problem.is_none() {
            s.problem = r.err();
        }
    }
    s
}

fn file_len(p: &str) -> u64 {
    std::fs::metadata(p).map_or(0, |m| m.len())
}

thread_local! {
    static TREE: std::cell::RefCell<Vec<(PathBuf, u64, Instant)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Folder sizes for the capacity bar, re-walked at most every 10 s.
fn cached_tree_bytes(p: &Path) -> u64 {
    TREE.with(|t| {
        let mut t = t.borrow_mut();
        if let Some((_, n, at)) = t.iter().find(|(q, _, at)| q == p && at.elapsed() < Duration::from_secs(10)) {
            let _ = at;
            return *n;
        }
        let n = disc::tree_bytes(p);
        t.retain(|(q, _, _)| q != p);
        t.push((p.to_path_buf(), n, Instant::now()));
        n
    })
}

pub(super) fn windows(app: &mut App, ctx: &egui::Context) {
    if !app.music.burn.open {
        return;
    }
    let lists: Vec<(Id, String)> = app.project.playlists.iter().map(|p| (p.id, p.name.clone())).collect();
    let sum = summary(app);
    let Some(a) = mu::burn_window(ctx, &mut app.music.burn, &lists, &sum) else { return };
    let b = &mut app.music.burn;
    match a {
        BurnAct::RefreshDrives => refresh_drives(app, ctx),
        BurnAct::CheckDisc => check_disc(app, ctx),
        BurnAct::Eject => {
            if let Some(d) = b.drive.and_then(|i| b.drives.get(i)).cloned() {
                b.media = None;
                spawn(ctx, "disc-eject", move || match imapi::eject(&d.id) {
                    Ok(()) => Done::Note("Ejected".into()),
                    Err(e) => Done::Note(e),
                });
            }
        }
        BurnAct::Erase => {
            if let Some(d) = b.drive.and_then(|i| b.drives.get(i)).cloned() {
                let p = crate::engine::export::spawn_job("disc-erase", move |p| imapi::erase(&d.id, p).map(|()| p.set(1.0, "Disc erased")));
                b.running = Some(p.clone());
                add_job("Erase disc".into(), p);
            }
        }
        BurnAct::ChooseImage => {
            let audio = b.kind == DiscKind::Audio;
            let name = format!("{}.{}", sanitize(&b.title), if audio { "cue" } else { "iso" });
            let d = rfd::FileDialog::new().set_file_name(name);
            let d = if audio { d.add_filter("Audio CD image (.bin + .cue)", &["cue"]) } else { d.add_filter("Disc image", &["iso"]) };
            if let Some(p) = d.save_file() {
                b.image = p;
            }
        }
        BurnAct::AddFiles => {
            if let Some(v) = rfd::FileDialog::new().pick_files() {
                b.extra.extend(v);
            }
        }
        BurnAct::AddFolder => {
            if let Some(v) = rfd::FileDialog::new().pick_folder() {
                b.extra.push(v);
            }
        }
        BurnAct::RemoveExtra(i) => {
            if i < b.extra.len() {
                b.extra.remove(i);
            }
        }
        BurnAct::Start => {
            if b.drive.is_none() && b.image.as_os_str().is_empty() {
                let audio = b.kind == DiscKind::Audio;
                let d = rfd::FileDialog::new().set_file_name(format!("{}.{}", sanitize(&b.title), if audio { "cue" } else { "iso" }));
                match d.save_file() {
                    Some(p) => b.image = p,
                    None => return,
                }
            }
            if let Err(e) = start_burn(app) {
                app.music.burn.result = e.clone();
                app.toast(e);
            }
        }
        BurnAct::Cancel => {
            if let Some(p) = &b.running {
                p.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        BurnAct::Close => {
            // a running burn keeps going in the Jobs panel
            b.open = false;
        }
        BurnAct::OpenJobs => app.surface(Pane::Jobs),
    }
}

/// The burn the dialog describes: (what, where, how).
fn burn_request(app: &App) -> Result<(BurnContent, BurnTarget, BurnOptions, String), String> {
    let b = &app.music.burn;
    let content = match b.kind {
        DiscKind::Audio => {
            let pl = b.playlist.ok_or("Pick a playlist")?;
            BurnContent::Audio(audio_cd(app, pl, &b.title, &b.performer, b.gap).ok_or("no such playlist")?)
        }
        DiscKind::Data => BurnContent::Data { volume: if b.title.is_empty() { "DISC".into() } else { b.title.clone() }, items: data_items(app, b.playlist, &b.extra)? },
    };
    let (target, label) = match b.drive.and_then(|i| b.drives.get(i)) {
        Some(d) => (BurnTarget::Drive { id: d.id.clone(), test_only: b.test_only }, if b.test_only { format!("Test burn · {}", d.name) } else { format!("Burn · {}", d.name) }),
        None => {
            let mut p = b.image.clone();
            if b.kind == DiscKind::Data && p.extension().is_none_or(|e| !e.eq_ignore_ascii_case("iso")) {
                p.set_extension("iso");
            }
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            (BurnTarget::Image(p), format!("Disc image · {name}"))
        }
    };
    let opts = BurnOptions { size: b.size, speed: b.speed, verify: b.verify, eject: b.eject, close: true };
    Ok((content, target, opts, label))
}

fn start_burn(app: &mut App) -> Result<(), String> {
    if app.music.burn.running.is_some() {
        return Err("A burn is already running".into());
    }
    let (content, target, opts, label) = burn_request(app)?;
    let p = crate::engine::export::spawn_job("burn", move |p| disc::run(&content, &target, &opts, p).map(|msg| p.set(1.0, msg)));
    app.music.burn.running = Some(p.clone());
    app.music.burn.result.clear();
    add_job(label, p);
    Ok(())
}

// ------------------------------------------------------------------------------------------------ actions

pub(super) fn act(app: &mut App, a: Action) -> bool {
    let ctx = app.music.ctx.clone();
    match a {
        Action::BurnDisc => {
            let pl = app.music.playlist.or_else(|| app.project.playlists.first().map(|p| p.id));
            if let Some(c) = ctx {
                open_burn(app, &c, pl);
            } else {
                app.music.burn.open = true;
            }
        }
        Action::NewPlaylist => {
            app.surface(Pane::Playlists);
            if let Some(c) = ctx {
                pl_act(app, &c, PlAct::New);
                let sel = media_sel(app);
                if !sel.is_empty() {
                    pl_act(app, &c, PlAct::Add { assets: sel, at: None });
                }
            }
        }
        Action::MakeVisualizerVideo => {
            let sel = media_sel(app);
            let items = if !sel.is_empty() { sel } else { app.music.playlist.and_then(|id| app.project.playlist(id)).map(|p| p.items.clone()).unwrap_or_default() };
            match ctx {
                Some(c) if !items.is_empty() => start_visualizer(app, &c, items),
                _ => app.toast("Select songs in the Library, or open a playlist"),
            }
        }
        _ => return false,
    }
    true
}

// ------------------------------------------------------------------------------------------------ tools

fn ids_arg(args: &Value, key: &str) -> Vec<Id> {
    args.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default()
}

fn pl_json(app: &App, id: Id) -> Value {
    let Some(p) = app.project.playlist(id) else { return Value::Null };
    json!({
        "id": p.id, "name": p.name, "gap": p.gap, "crossfade": p.crossfade,
        "duration": app.project.playlist_duration(p.id),
        "items": p.items.iter().map(|&a| json!({"asset": a, "name": app.project.asset(a).map(|x| app.project.asset_display_name(x)), "duration": app.project.asset(a).map(|x| x.duration)})).collect::<Vec<_>>(),
    })
}

fn need_pl(app: &App, args: &Value) -> Result<Id, String> {
    let id = arg_u64(args, "id").ok_or("id: the playlist id (playlist.list)")?;
    app.project.playlist(id).map(|p| p.id).ok_or_else(|| format!("no playlist {id}"))
}

fn tags_json(t: &MediaTags) -> Value {
    json!({"tags": t.values, "cover": t.cover.is_some(), "duration": t.duration})
}

/// The files a tags tool acts on: `ids` (library assets) and / or `path`.
fn tag_files(app: &App, args: &Value) -> Result<Vec<(Value, PathBuf)>, String> {
    let mut v: Vec<(Value, PathBuf)> = Vec::new();
    for id in ids_arg(args, "ids").into_iter().chain(arg_u64(args, "id")) {
        let a = app.project.asset(id).ok_or_else(|| format!("no asset {id}"))?;
        v.push((json!(id), PathBuf::from(&a.path)));
    }
    if let Some(p) = arg_str(args, "path") {
        v.push((json!(p), PathBuf::from(p)));
    }
    if v.is_empty() {
        return Err("give ids (library assets) or a path".into());
    }
    Ok(v)
}

fn drive_id(args: &Value) -> Result<String, String> {
    let drives = imapi::drives()?;
    if drives.is_empty() {
        return Err("No disc burner found. Use target \"image\" to write a disc image instead.".into());
    }
    match arg_u64(args, "drive") {
        Some(i) => drives.get(i as usize).map(|d| d.id.clone()).ok_or_else(|| format!("no drive {i} (have {})", drives.len())),
        None => Ok(drives[0].id.clone()),
    }
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "playlist.list",
        desc: "Every playlist in the project: id, name, gap / crossfade seconds, total duration and its items (asset id, name, duration).",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| Ok(ToolOutcome::Done(json!(app.project.playlists.iter().map(|p| pl_json(app, p.id)).collect::<Vec<_>>()))),
    },
    ToolDef {
        name: "playlist.create",
        desc: "Make a playlist (optionally with library assets in it). Returns it.",
        args: &["name:string:false:default Playlist N", "assets:array:false:library asset ids, in order"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = app.project.add_playlist(arg_str(args, "name").unwrap_or(""));
            app.project.playlist_add(id, &ids_arg(args, "assets"), None);
            Ok(ToolOutcome::Done(pl_json(app, id)))
        },
    },
    ToolDef {
        name: "playlist.rename",
        desc: "Rename a playlist.",
        args: &["id:integer:true:", "name:string:true:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = need_pl(app, args)?;
            if !app.project.rename_playlist(id, arg_str(args, "name").unwrap_or("")) {
                return Err("name: not blank".into());
            }
            Ok(ToolOutcome::Done(pl_json(app, id)))
        },
    },
    ToolDef {
        name: "playlist.delete",
        desc: "Delete a playlist (its songs stay in the library).",
        args: &["id:integer:true:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = need_pl(app, args)?;
            app.project.delete_playlist(id);
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
    ToolDef {
        name: "playlist.add",
        desc: "Add library assets (sound or video files) to a playlist, at index `at` (default the end).",
        args: &["id:integer:true:", "assets:array:true:asset ids", "at:integer:false:index"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = need_pl(app, args)?;
            let n = app.project.playlist_add(id, &ids_arg(args, "assets"), arg_u64(args, "at").map(|x| x as usize));
            Ok(ToolOutcome::Done(json!({"added": n, "playlist": pl_json(app, id)})))
        },
    },
    ToolDef {
        name: "playlist.remove",
        desc: "Remove items from a playlist by index.",
        args: &["id:integer:true:", "indices:array:true:0-based item indices"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = need_pl(app, args)?;
            let idx: Vec<usize> = ids_arg(args, "indices").into_iter().map(|x| x as usize).collect();
            let n = app.project.playlist_remove(id, &idx);
            Ok(ToolOutcome::Done(json!({"removed": n, "playlist": pl_json(app, id)})))
        },
    },
    ToolDef {
        name: "playlist.move",
        desc: "Move a playlist item from index `from` to index `to`.",
        args: &["id:integer:true:", "from:integer:true:", "to:integer:true:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = need_pl(app, args)?;
            let (f, t) = (arg_u64(args, "from").unwrap_or(0) as usize, arg_u64(args, "to").unwrap_or(0) as usize);
            app.project.playlist_move(id, f, t);
            Ok(ToolOutcome::Done(pl_json(app, id)))
        },
    },
    ToolDef {
        name: "playlist.set",
        desc: "A playlist's spacing: gap = seconds of silence between songs, crossfade = seconds they overlap on the timeline (0 = off).",
        args: &["id:integer:true:", "gap:number:false:", "crossfade:number:false:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = need_pl(app, args)?;
            app.project.playlist_set_spacing(id, arg_f64(args, "gap"), arg_f64(args, "crossfade"));
            Ok(ToolOutcome::Done(pl_json(app, id)))
        },
    },
    ToolDef {
        name: "playlist.to_timeline",
        desc: "Put a playlist's songs on the timeline back to back from `at` (default the playhead), with its gap or crossfade. Returns the clip ids.",
        args: &["id:integer:true:", "at:number:false:timeline seconds"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = need_pl(app, args)?;
            let ids = app.project.playlist_to_timeline(id, arg_f64(args, "at").unwrap_or(app.playhead));
            Ok(ToolOutcome::Done(json!({"clips": ids})))
        },
    },
    ToolDef {
        name: "playlist.export_m3u8",
        desc: "Save a playlist as an .m3u8 file (UTF-8, #EXTINF lengths and titles, paths relative to the file's folder where possible).",
        args: &["id:integer:true:", "path:string:true:where to save, .m3u8"],
        kind: ToolKind::Read,
        run: |app, args| {
            let id = need_pl(app, args)?;
            let path = PathBuf::from(arg_str(args, "path").ok_or("path")?);
            export_m3u8(app, id, &path)?;
            Ok(ToolOutcome::Done(json!({"ok": true, "path": path})))
        },
    },
    ToolDef {
        name: "playlist.play",
        desc: "Play a playlist (from item `from`) in the Source monitor, back to back.",
        args: &["id:integer:true:", "from:integer:false:item index"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let id = need_pl(app, args)?;
            let items = app.project.playlist(id).map(|p| p.items.clone()).unwrap_or_default();
            play(app, items.into_iter().skip(arg_u64(args, "from").unwrap_or(0) as usize).collect());
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
    ToolDef {
        name: "playlist.visualizer_video",
        desc: "Make a visualizer video: the songs of playlist `id` (or library `assets`) on the timeline from `at`, each with a linked visualizer layer of `style` (audio.visualizer_styles), its cover art and a title card. Reads tags and covers on this thread.",
        args: &["id:integer:false:a playlist", "assets:array:false:library asset ids instead", "at:number:false:default the playhead", "style:string:false:default the last used", "cover:boolean:false:default true", "title_card:boolean:false:default true"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let (items, spacing) = match arg_u64(args, "id") {
                Some(_) => {
                    let id = need_pl(app, args)?;
                    let p = app.project.playlist(id).ok_or("no playlist")?;
                    (p.items.clone(), (p.gap, p.crossfade))
                }
                None => (ids_arg(args, "assets"), (0.0, 0.0)),
            };
            let style = match arg_str(args, "style") {
                Some(n) => crate::engine::visualizer::find(Some(n))?,
                None => crate::engine::visualizer::last_used(&app.settings.visualizer_style),
            };
            let backend = app.backend();
            let mut songs = Vec::new();
            for id in items {
                let a = app.project.asset(id).ok_or_else(|| format!("no asset {id}"))?.clone();
                let p = Path::new(&a.path);
                let t = tags::read(p).unwrap_or_default();
                let shown = app.project.asset_display_name(&a);
                let title = crate::engine::mixvideo::title_line(t.get("artist"), if t.get("title").is_empty() { &shown } else { t.get("title") });
                let cover = tags::extract_cover(p, &tags::covers_dir()).ok().flatten().and_then(|c| crate::media::probe(&c.to_string_lossy(), backend).ok()).map(|mut c| {
                    c.folder = "Covers".into();
                    app.project.add_asset(c)
                });
                songs.push(crate::engine::mixvideo::Song { asset: id, cover, title });
            }
            let opts = crate::engine::mixvideo::MixOptions {
                gap: spacing.0,
                crossfade: spacing.1,
                cover: arg_bool(args, "cover").unwrap_or(true),
                title_card: arg_bool(args, "title_card").unwrap_or(true),
            };
            let made = crate::engine::mixvideo::build(&mut app.project, &songs, arg_f64(args, "at").unwrap_or(app.playhead), &style, &opts, backend)?;
            Ok(ToolOutcome::Done(json!(made.iter().map(|m| json!({"audio": m.audio, "visualizer": m.visualizer, "cover": m.cover, "title": m.title})).collect::<Vec<_>>())))
        },
    },
    ToolDef {
        name: "media.tags_get",
        desc: "Read the tags of library assets (ids) or a file (path): title, artist, album, album_artist, genre, date (year), track, disc, comment, lyrics, description, rating, and whether it has a cover picture.",
        args: &["ids:array:false:library asset ids", "id:integer:false:", "path:string:false:a file instead"],
        kind: ToolKind::Read,
        run: |app, args| {
            let files = tag_files(app, args)?;
            let out: Vec<Value> = files
                .iter()
                .map(|(k, p)| match tags::read(p) {
                    Ok(t) => json!({"file": k, "tags": tags_json(&t)["tags"], "cover": t.cover.is_some(), "duration": t.duration}),
                    Err(e) => json!({"file": k, "error": e}),
                })
                .collect();
            Ok(ToolOutcome::Done(json!(out)))
        },
    },
    ToolDef {
        name: "media.tags_set",
        desc: "Write tags into files (stream copy, no re-encode; temp file + .bak swap). fields: {key: value} with keys from media.tags_get (\"\" removes one). cover: a picture path, \"remove\", or {frame: seconds} to use that frame of the (first) video. Same for every file given.",
        args: &["ids:array:false:library asset ids", "id:integer:false:", "path:string:false:a file instead", "fields:object:false:{title: ..., artist: ...}", "cover:string:false:picture path or remove"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let files = tag_files(app, args)?;
            let mut edit = TagEdit::default();
            if let Some(o) = args.get("fields").and_then(Value::as_object) {
                for (k, v) in o {
                    let v = match v {
                        Value::String(s) => s.clone(),
                        Value::Null => String::new(),
                        other => other.to_string(),
                    };
                    edit.set.insert(k.clone(), v);
                }
            }
            edit.cover = match args.get("cover") {
                None | Some(Value::Null) => CoverEdit::Keep,
                Some(Value::String(s)) if s.eq_ignore_ascii_case("remove") => CoverEdit::Remove,
                Some(Value::String(s)) => CoverEdit::Set(PathBuf::from(s)),
                Some(Value::Object(o)) => {
                    let t = o.get("frame").and_then(Value::as_f64).ok_or("cover: {frame: seconds}")?;
                    let out = tags::covers_dir().join(format!("frame-{}.jpg", tags::cover_key(&files[0].1)));
                    tags::grab_frame(&files[0].1, t, &out)?;
                    CoverEdit::Set(out)
                }
                _ => return Err("cover: a path, \"remove\" or {frame: seconds}".into()),
            };
            if edit.is_empty() {
                return Err("nothing to write: give fields and / or cover".into());
            }
            let mut out = Vec::new();
            for (k, p) in &files {
                let t = tags::write(p, &edit).map_err(|e| format!("{}: {e}", p.display()))?;
                out.push(json!({"file": k, "tags": t.values, "cover": t.cover.is_some()}));
            }
            app.music.tags.ids.clear(); // the Inspector reads them again
            Ok(ToolOutcome::Done(json!(out)))
        },
    },
    ToolDef {
        name: "disc.drives",
        desc: "Disc burners Windows can write with (index, name), and with `drive` the disc in it: type, blank, rewritable, free space, write speeds.",
        args: &["drive:integer:false:index from this list"],
        kind: ToolKind::Read,
        run: |_, args| {
            let d = imapi::drives()?;
            let list: Vec<Value> = d.iter().enumerate().map(|(i, d)| json!({"drive": i, "name": d.name})).collect();
            let media = match arg_u64(args, "drive") {
                Some(_) => {
                    let m = imapi::media_info(&drive_id(args)?)?;
                    json!({"present": m.present, "type": m.kind, "blank": m.blank, "rewritable": m.rewritable, "free_sectors": m.free_sectors, "speeds": m.speeds, "describe": m.describe()})
                }
                None => Value::Null,
            };
            Ok(ToolOutcome::Done(json!({"drives": list, "media": media})))
        },
    },
    ToolDef {
        name: "disc.burn",
        desc: "Burn a playlist: kind audio (Red Book CD: 44.1 kHz 16-bit tracks, gaps, CD-Text in the cue) or data (ISO 9660 + Joliet + UDF; the playlist's songs + an .m3u8, plus `files`). target \"image\" writes `path` (.iso, or .bin + .cue for audio) without a drive; target \"drive\" burns (drive index from disc.drives; test_only checks everything and writes nothing). Runs as a job.",
        args: &[
            "kind:string:true:audio | data",
            "playlist:integer:false:playlist id",
            "files:array:false:data: more files / folders",
            "target:string:false:image (default) | drive",
            "path:string:false:image file",
            "drive:integer:false:",
            "size:string:false:cd74 | cd80 | dvd | dvddl | bd25 (default cd80)",
            "title:string:false:CD-Text album / volume name",
            "performer:string:false:CD-Text artist",
            "gap:number:false:audio: seconds between tracks (default 2)",
            "speed:integer:false:sectors per second, 0 = fastest",
            "verify:boolean:false:",
            "eject:boolean:false:",
            "test_only:boolean:false:",
        ],
        kind: ToolKind::Job,
        run: |app, args| {
            let pl = arg_u64(args, "playlist");
            let title = arg_str(args, "title").map(String::from).unwrap_or_else(|| pl.and_then(|id| app.project.playlist(id)).map(|p| p.name.clone()).unwrap_or_default());
            let performer = arg_str(args, "performer").unwrap_or("").to_string();
            let content = match arg_str(args, "kind").unwrap_or("") {
                "audio" => BurnContent::Audio(audio_cd(app, pl.ok_or("playlist: needed for an audio CD")?, &title, &performer, arg_f64(args, "gap").unwrap_or(2.0)).ok_or("no such playlist")?),
                "data" => {
                    let files: Vec<PathBuf> = args.get("files").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(PathBuf::from).collect()).unwrap_or_default();
                    BurnContent::Data { volume: if title.is_empty() { "DISC".into() } else { title.clone() }, items: data_items(app, pl, &files)? }
                }
                _ => return Err("kind: audio | data".into()),
            };
            let size = match arg_str(args, "size") {
                Some(s) => DiscSize::from_name(s).ok_or("size: cd74 | cd80 | dvd | dvddl | bd25")?,
                None => DiscSize::Cd80,
            };
            let (target, out) = match arg_str(args, "target").unwrap_or("image") {
                "drive" => (BurnTarget::Drive { id: drive_id(args)?, test_only: arg_bool(args, "test_only").unwrap_or(false) }, PathBuf::new()),
                "image" => {
                    let p = PathBuf::from(arg_str(args, "path").ok_or("path: the image file")?);
                    let out = if matches!(content, BurnContent::Audio(_)) { p.with_extension("cue") } else { p.clone() };
                    (BurnTarget::Image(p), out)
                }
                _ => return Err("target: image | drive".into()),
            };
            let opts = BurnOptions { size, speed: arg_u64(args, "speed").unwrap_or(0) as i32, verify: arg_bool(args, "verify").unwrap_or(true), eject: arg_bool(args, "eject").unwrap_or(true), close: true };
            let p = crate::engine::export::spawn_job("burn", move |p| disc::run(&content, &target, &opts, p).map(|m| p.set(1.0, m)));
            Ok(ToolOutcome::Job(p, out))
        },
    },
];
