//! ---- ws:animator-templates ----
//! App side of templates (`engine::templates`): the Gallery's Templates tab (a click places a template, or
//! opens "Use Template" when it has slots to fill), the Template Builder ("Save as Template…"), sharing as
//! one `.setemplate` file, and the `templates.*` tools. Placing always goes through the `templates.use`
//! tool, so the UI and MCP share one code path and one undo step.

use super::tools_helpers::*;
use super::*;
use crate::engine::gallery::{self, Kind as GKind};
use crate::engine::templates::{self, Part, SlotFit, SlotWant, TemplateData};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::template_ui::{self, BuildAct, BuildRow, BuilderState, UseAct, UseSlot, UseState};

/// The two template windows.
#[derive(Default)]
pub struct State {
    pub using: UseState,
    pub build: BuilderState,
    /// A beat edit waiting for its song's analysis.
    pub pending: Option<Pending>,
    /// A template dropped on the timeline waiting for its sample footage to download.
    pub fetching: Option<Fetching>,
}

/// A template waiting for its footage: where it goes, the download's result, and `fills` when it comes from
/// Use Template (None = a timeline drop: one sequence clip).
pub struct Fetching {
    name: String,
    at: f64,
    track: Option<usize>,
    fills: Option<Vec<Value>>,
    done: std::sync::mpsc::Receiver<Result<(), String>>,
}

/// The library files of `d`'s sample footage (catalogue paths), each once.
fn sample_files(d: &TemplateData) -> Vec<String> {
    // (a local-only file is never fetched: without it, its clip is left out)
    let mut v: Vec<String> = d.samples.iter().filter(|s| !s.local).map(|s| format!("{}/{}", d.media, s.file)).collect();
    v.sort();
    v.dedup();
    v
}

/// A Gallery template dropped on the timeline: ONE sequence clip at `at` whose slots already hold the
/// template's own footage (downloaded first, in the background, when it is not on this PC yet).
pub(super) fn drop_nested(app: &mut App, name: &str, at: f64, track: Option<usize>) {
    fetch_then(app, name, at, track, None);
}

/// Place template `name` once its sample footage is on this PC (downloaded first when it is not).
fn fetch_then(app: &mut App, name: &str, at: f64, track: Option<usize>, fills: Option<Vec<Value>>) {
    let Some(d) = find(app, name) else {
        return app.toast(format!("Template '{name}' not found"));
    };
    let src = crate::engine::library::Source::of(&app.settings);
    let missing: Vec<String> = sample_files(&d).into_iter().filter(|r| sample_path(&src, r).is_none()).collect();
    if missing.is_empty() {
        return match fills {
            None => place_nested(app, name, at, track),
            Some(f) => place_now(app, name, at, track, f),
        };
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = missing.iter().try_for_each(|r| crate::engine::library::fetch_file(&src, r).map(|_| ()));
        let _ = tx.send(r);
    });
    app.templates.fetching = Some(Fetching { name: name.to_string(), at, track, fills, done: rx });
    app.toast("Downloading the template's footage…");
}

fn place_nested(app: &mut App, name: &str, at: f64, track: Option<usize>) {
    let args = json!({"name": name, "at": at, "track": track, "nested": true});
    match app.run_tool_undoable("templates.use", &args) {
        Ok(_) => gallery_ctl::count_use(app, "Templates", name),
        Err(e) => app.toast(e),
    }
}

/// Your own copy of a template's sample file, used instead of the library's: `<gallery>/templates/media/
/// <slug>/<file>` (a song you may not share stays on this PC; the shipped templates carry their own).
pub(super) fn own_sample(rel: &str) -> std::path::PathBuf {
    let rel = rel.strip_prefix("templates/").unwrap_or(rel);
    rel.split('/').fold(gallery_mine::root().join(GKind::Template.dir()).join("media"), |p, s| p.join(s))
}

/// Where sample file `rel` (a catalogue path) is on this PC: your own copy, else the library's. None = not yet.
fn sample_path(src: &crate::engine::library::Source, rel: &str) -> Option<std::path::PathBuf> {
    [own_sample(rel), src.local(rel)].into_iter().find(|p| p.is_file())
}

/// The project assets of `d`'s sample footage (file name -> asset), imported once: a second drop reuses them.
fn sample_assets(app: &mut App, d: &TemplateData) -> Result<Vec<(String, Id)>, String> {
    let src = crate::engine::library::Source::of(&app.settings);
    let mut out: Vec<(String, Id)> = Vec::new();
    for s in &d.samples {
        if out.iter().any(|o| o.0 == s.file) {
            continue;
        }
        let rel = format!("{}/{}", d.media, s.file);
        let path = sample_path(&src, &rel).unwrap_or_else(|| src.local(&rel));
        let path = path.to_string_lossy().into_owned();
        let id = match app.project.asset_by_path(&path) {
            Some(a) => a.id,
            None => {
                if s.local && !std::path::Path::new(&path).is_file() {
                    continue;
                }
                if !std::path::Path::new(&path).is_file() {
                    return Err(format!("{path}: the template's footage is not downloaded"));
                }
                let a = crate::media::probe(&path, app.backend()).map_err(|e| format!("{path}: {e}"))?;
                app.project.add_asset(a)
            }
        };
        out.push((s.file.clone(), id));
    }
    Ok(out)
}

fn done(v: Value) -> Result<ToolOutcome, String> {
    Ok(ToolOutcome::Done(v))
}

/// A template by name: yours, then the built-in ones (made for this project's frame).
pub(super) fn find(app: &App, name: &str) -> Option<TemplateData> {
    templates::find(&app.settings.templates, name, app.project.width, app.project.height)
}

fn is_picture(app: &App, id: Id) -> bool {
    app.project.asset(id).is_some_and(|a| matches!(a.kind, ClipKind::Video | ClipKind::Image))
}

fn is_sound(app: &App, id: Id) -> bool {
    app.project.asset(id).is_some_and(|a| a.kind == ClipKind::Audio)
}

/// The media to fill slots from, in order: what is selected in the Media Library, else the footage and
/// sound of the clips selected on the timeline (in time order, each file once).
fn library_pictures(app: &App) -> Vec<Id> {
    let ids: Vec<Id> = if app.library.sel_ids.is_empty() { app.library.selected.into_iter().collect() } else { app.library.sel_ids.clone() };
    let mut ids: Vec<Id> = ids.into_iter().filter(|&id| is_picture(app, id) || is_sound(app, id)).collect();
    if ids.is_empty() {
        let mut clips: Vec<&Clip> = app.selection.iter().filter_map(|&id| app.project.clip(id)).filter(|c| c.uses_asset() && c.asset != 0).collect();
        clips.sort_by(|a, b| a.start.total_cmp(&b.start));
        for c in clips {
            // (the sound half of a video clip is the same file)
            if !ids.contains(&c.asset) && (is_picture(app, c.asset) || is_sound(app, c.asset)) {
                ids.push(c.asset);
            }
        }
    }
    ids
}

/// Give `ids` to the slots from `from` on (`templates::assign`: sounds to sound slots, pictures to the
/// others, a beat edit's pictures again in turn when it has more shots than clips).
fn fill_from(app: &mut App, from: usize, ids: &[Id], cycle: bool) {
    let media: Vec<(Id, bool)> = ids.iter().map(|&id| (id, is_sound(app, id))).collect();
    let wants: Vec<SlotWant> = app.templates.using.slots.iter().skip(from).map(|s| s.want).collect();
    for (k, got) in templates::assign(&wants, &media, cycle).into_iter().enumerate() {
        let Some(id) = got else { continue };
        let name = app.project.asset(id).map(|a| a.name()).unwrap_or_default();
        if let Some(s) = app.templates.using.slots.get_mut(from + k) {
            s.fill = Some((id, name));
        }
    }
}

/// The media selected in the library (or on the timeline) as `templates.use` fills, in slot order (a
/// template dropped on the timeline takes them without asking).
pub(super) fn library_fills(app: &App, name: &str) -> Vec<Value> {
    let Some(d) = find(app, name) else { return Vec::new() };
    let media: Vec<(Id, bool)> = library_pictures(app).into_iter().map(|id| (id, is_sound(app, id))).collect();
    let wants: Vec<SlotWant> = d.slot_list().iter().map(|s| s.want).collect();
    let mut fills: Vec<Value> =
        templates::assign(&wants, &media, d.sync.is_some()).into_iter().map(|g| g.map_or(Value::Null, |id| json!(id))).collect();
    while fills.last().is_some_and(Value::is_null) {
        fills.pop();
    }
    fills
}

/// A Templates card was clicked: a template without slots is placed at the playhead; one with slots opens
/// "Use Template" (already filled, in order, with the media selected in the library).
pub(super) fn card_clicked(app: &mut App, name: &str) {
    let Some(d) = find(app, name) else {
        return app.toast(format!("Template '{name}' not found"));
    };
    let slots = d.slot_list();
    if slots.is_empty() {
        return place(app, name, app.playhead, None, Vec::new());
    }
    let slots = slots
        .iter()
        .map(|s| {
            let c = d.clip(s.clip);
            UseSlot {
                name: s.name.clone(),
                want: s.want,
                secs: c.map_or(0.0, |c| c.duration),
                fit: s.fit,
                fill: None,
                sample: c.is_some_and(|c| c.asset != 0) || d.samples.iter().any(|x| x.clip == s.clip),
                hint: s.hint.clone(),
            }
        })
        .collect();
    app.templates.using = UseState { open: true, name: name.to_string(), slots };
    let lib = library_pictures(app);
    fill_from(app, 0, &lib, d.sync.is_some());
}

/// Place template `name` at `at` (pulled to video track `track` when given) with `fills` (per slot: null,
/// an asset id or a file path), as one undo step.
pub(super) fn place(app: &mut App, name: &str, at: f64, track: Option<usize>, fills: Vec<Value>) {
    fetch_then(app, name, at, track, Some(fills));
}

fn place_now(app: &mut App, name: &str, at: f64, track: Option<usize>, fills: Vec<Value>) {
    // a beat edit reads its song first (in the background, the first time): placed when that is done
    if let Some(path) = song_path(app, name, &fills) {
        if !crate::engine::visualizer::analysis_ready(&path, 0) {
            app.templates.pending = Some(Pending { name: name.to_string(), at, track, fills, path });
            return app.toast("Finding the song's beat and drop…");
        }
    }
    let args = json!({"name": name, "at": at, "fills": fills, "track": track});
    match app.run_tool_undoable("templates.use", &args) {
        Ok(_) => gallery_ctl::count_use(app, "Templates", name),
        Err(e) => app.toast(e),
    }
}

/// A template waiting for its song's analysis.
pub struct Pending {
    name: String,
    at: f64,
    track: Option<usize>,
    fills: Vec<Value>,
    path: String,
}

/// The file in a beat edit's song slot among `fills` (in slot order), if it has one.
fn song_path(app: &App, name: &str, fills: &[Value]) -> Option<String> {
    let d = find(app, name)?;
    let song = d.sync.as_ref()?.song;
    let i = d.slot_list().iter().position(|s| s.clip == song)?;
    match fills.get(i)? {
        Value::String(p) => Some(p.clone()),
        v => app.project.asset(v.as_u64()?).map(|a| a.path.clone()),
    }
}

/// The asset a fill names: an asset id, or a file (probed now, so the slot's timing can follow its length).
/// `sound`: the slot takes a sound (anything with an audio track), else a picture.
fn fill_asset(app: &mut App, v: &Value, sound: bool) -> Result<Option<Id>, String> {
    if v.is_null() {
        return Ok(None);
    }
    let ok = |a: &crate::model::Asset| if sound { a.kind == ClipKind::Audio || !a.audio_streams.is_empty() } else { matches!(a.kind, ClipKind::Video | ClipKind::Image) };
    let what = if sound { "a sound (or a video with sound)" } else { "a video or an image" };
    if let Some(id) = v.as_u64() {
        return match app.project.asset(id) {
            Some(a) if ok(a) => Ok(Some(id)),
            _ => Err(format!("asset {id} is not {what} in this project")),
        };
    }
    let path = v.as_str().ok_or("a fill is an asset id or a file path")?;
    if let Some(a) = app.project.asset_by_path(path) {
        return if ok(a) { Ok(Some(a.id)) } else { Err(format!("{path}: not {what}")) };
    }
    let a = crate::media::probe(path, app.backend()).map_err(|e| format!("{path}: {e}"))?;
    if !ok(&a) {
        return Err(format!("{path}: not {what}"));
    }
    Ok(Some(app.project.add_asset(a)))
}

/// A song's tempo grid and drop, source seconds: the Beat Aligner's when its sheet is this song's,
/// else detected (the analysis is computed on this thread when not cached yet). `drop` overrides.
fn song_beat(app: &App, asset: Id, drop: Option<f64>, lead: f64) -> Result<templates::SongBeat, String> {
    let a = app.project.asset(asset).ok_or("the song is not in the project")?;
    let an = crate::engine::visualizer::analysis(&a.path, 0, app.backend())?;
    let sheet = &app.project.beats;
    let mine = sheet.asset == asset && sheet.bpm > 0.0;
    let (bpm, offset) = if mine {
        (sheet.bpm, sheet.offset)
    } else {
        let mut s = crate::model::BeatSheet::default();
        beats_ctl::apply_analysis(&mut s, &an, a.duration)?;
        (s.bpm, s.offset)
    };
    let marked = mine.then(|| sheet.sections.iter().find(|s| s.name.eq_ignore_ascii_case("Drop")).map(|s| s.a)).flatten();
    let low: Vec<f32> = an.raw.iter().map(|f| f[1] + f[2] + 0.5 * f[0]).collect();
    let found = templates::find_drop(&low, crate::engine::visualizer::ANALYSIS_HZ, bpm, offset, lead * 0.72); // (the build before the drop, at its shortest tempo fit)
    let drop = drop.or(marked).or(found).unwrap_or(offset);
    // on the beat
    let p = 60.0 / bpm;
    let drop = (offset + ((drop - offset) / p).round() * p).max(0.0);
    Ok(templates::SongBeat { bpm, offset, drop, len: a.duration })
}

/// Where a sound's hit is (its loudest onset, source seconds).
fn sound_onset(app: &App, asset: Id) -> f64 {
    let Some(a) = app.project.asset(asset) else { return 0.0 };
    match crate::engine::visualizer::analysis(&a.path, 0, app.backend()) {
        Ok(an) => templates::loudest_onset(&an.raw.iter().map(|f| f[0]).collect::<Vec<f32>>(), crate::engine::visualizer::ANALYSIS_HZ),
        Err(_) => 0.0,
    }
}

/// An empty sound slot with a hint: a sound of the project whose name has the hint's words, else one from
/// your SFX / music folders (imported).
fn default_sound(app: &mut App, hint: &str) -> Option<Id> {
    let words: Vec<String> = hint.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return None;
    }
    let have = app.project.assets.iter().find(|a| {
        let n = a.name().to_lowercase();
        (a.kind == ClipKind::Audio || !a.audio_streams.is_empty()) && words.iter().all(|w| n.contains(w.as_str()))
    });
    if let Some(a) = have {
        return Some(a.id);
    }
    let path = templates::find_sound(hint, &templates::sound_roots())?;
    let a = crate::media::probe(&path.to_string_lossy(), app.backend()).ok()?;
    Some(app.project.add_asset(a))
}

fn slots_json(d: &TemplateData) -> Vec<Value> {
    d.slot_list()
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let c = d.clip(s.clip);
            json!({"index": i + 1, "name": s.name, "wants": s.want.name(), "fit": s.fit.name(),
                   "seconds": c.map_or(0.0, |c| c.duration), "start": c.map_or(0.0, |c| c.start),
                   "has_footage": c.is_some_and(|c| c.asset != 0), "hint": s.hint})
        })
        .collect()
}

// ------------------------------------------------------------------------------------------ builder

/// "Save as Template…": the builder on the selection (or the whole timeline when nothing is selected).
pub(super) fn open_builder(app: &mut App) {
    let selection: Vec<Id> =
        app.project.expand_links(&app.selection).into_iter().filter(|&id| app.project.clip(id).is_some()).collect();
    app.templates.build =
        BuilderState { open: true, just_opened: true, all: selection.is_empty(), selection, ..Default::default() };
    scope_rows(app);
    if app.templates.build.ids.is_empty() {
        app.templates.build.open = false;
        app.toast("Nothing on the timeline to save as a template");
    }
}

/// The builder's clips and footage rows for its current scope.
fn scope_rows(app: &mut App) {
    let App { project, templates, .. } = app;
    let st = &mut templates.build;
    st.ids = if st.all { project.all_clips().map(|(_, c)| c.id).collect() } else { st.selection.clone() };
    let mut clips: Vec<&Clip> = st.ids.iter().filter_map(|&id| project.clip(id)).collect();
    clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    st.start = clips.first().map_or(0.0, |c| c.start);
    st.dur = clips.iter().map(|c| c.end()).fold(0.0, f64::max) - st.start;
    st.thumb_t = st.thumb_t.clamp(0.0, st.dur.max(0.0));
    st.rows.clear();
    for c in &clips {
        let html = project.asset(c.asset).is_some_and(|a| a.html.is_some());
        // the sound half of a picture clip follows that clip
        let half = c.kind == ClipKind::Audio && c.link != 0 && clips.iter().any(|o| o.link == c.link && o.kind != ClipKind::Audio);
        if !c.uses_asset() || html || half || (c.asset == 0 && !c.container) {
            continue;
        }
        st.rows.push(BuildRow {
            clip: c.id,
            label: c.name.clone(),
            kind: c.kind,
            secs: c.duration,
            slot: c.container && c.kind == ClipKind::Video,
            slot_name: c.container_label.clone(),
            want: SlotWant::Any,
            fit: SlotFit::Trim,
            include: c.asset != 0,
            open: false,
        });
    }
    st.others = clips.iter().filter(|c| !c.uses_asset()).count();
}

fn parts_of(rows: &[BuildRow]) -> Vec<Part> {
    rows.iter()
        .map(|r| Part { clip: r.clip, slot: r.slot.then(|| (r.slot_name.clone(), r.want, r.fit)), include: r.include })
        .collect()
}

/// Build a template from `ids` and save it to your Gallery (replacing one of that name), with its picture
/// taken `thumb_t` seconds in. Returns the name it has.
fn save_built(app: &mut App, name: &str, tags: &[String], ids: &[Id], parts: &[Part], thumb_t: f64) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a template needs a name".into());
    }
    if !ids.iter().any(|&id| app.project.clip(id).is_some()) {
        return Err("no clips to save as a template".into());
    }
    let d = templates::build(&app.project, ids, parts, thumb_t);
    let start = ids.iter().filter_map(|&id| app.project.clip(id)).map(|c| c.start).fold(f64::INFINITY, f64::min);
    let thumb_t = d.thumb_t;
    let item = gallery_mine::put(app, GKind::Template, name, tags, serde_json::to_value(&d).map_err(|e| e.to_string())?, true)?;
    // the card's picture: that frame of the timeline, small (a failed render only leaves the card plain)
    let png = templates::thumb_path(&item.path);
    let _ = app.run_tool("frame.export", &json!({"path": png.to_string_lossy(), "t": start + thumb_t, "width": 320}));
    app.gallery.tab = crate::ui::gallery::GalleryTab::Templates;
    Ok(item.name)
}

// ------------------------------------------------------------------------------------------ sharing

/// Write template `name` to `path` as one file, with or without its footage.
fn export(app: &App, name: &str, path: &std::path::Path, footage: bool) -> Result<templates::Exported, String> {
    let item = gallery::find(&gallery_mine::root(), GKind::Template, name);
    let thumb = item.as_ref().map(|i| templates::thumb_path(&i.path)).filter(|p| p.is_file());
    let d = find(app, name).ok_or_else(|| format!("no template '{name}'"))?;
    templates::export_package(name, &d, footage, thumb.as_deref(), path)
}

/// "Export…" on a template card: choose where, then write the file.
pub(super) fn export_dialog(app: &mut App, name: &str, footage: bool) {
    let Some(path) = rfd::FileDialog::new()
        .set_title("Export Template")
        .set_file_name(format!("{name}.{}", templates::EXT))
        .add_filter("Template", &[templates::EXT])
        .save_file()
    else {
        return;
    };
    match export(app, name, &path, footage) {
        Ok(r) => {
            let slots = if r.empty > 0 { format!(", {} empty slot{}", r.empty, if r.empty == 1 { "" } else { "s" }) } else { String::new() };
            app.toast(format!("Exported \"{name}\" ({} footage file{}{slots})", r.files, if r.files == 1 { "" } else { "s" }));
        }
        Err(e) => app.toast(e),
    }
}

// ------------------------------------------------------------------------------------------ windows

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::SaveTemplate => {
            open_builder(app);
            true
        }
        _ => false,
    }
}

pub(super) fn windows(app: &mut App, ctx: &egui::Context) {
    if let Some(r) = app.templates.fetching.as_ref().and_then(|f| f.done.try_recv().ok()) {
        if let Some(f) = app.templates.fetching.take() {
            match r {
                Ok(()) => match f.fills {
                    None => place_nested(app, &f.name, f.at, f.track),
                    Some(fills) => place_now(app, &f.name, f.at, f.track, fills),
                },
                Err(e) => app.toast(format!("Could not download the template's footage: {e}")),
            }
        }
    }
    if app.templates.fetching.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(150)); // (polled only while it downloads)
    }
    if let Some(path) = app.templates.pending.as_ref().map(|p| p.path.clone()) {
        let c = ctx.clone();
        if crate::engine::visualizer::analysis_async(&path, 0, app.backend(), move || c.request_repaint()).is_some() {
            if let Some(p) = app.templates.pending.take() {
                place(app, &p.name, p.at, p.track, p.fills);
            }
        }
    }
    if app.templates.using.open {
        let selected = library_pictures(app).len();
        match template_ui::use_window(ctx, &mut app.templates.using, selected) {
            Some(UseAct::Cancel) => app.templates.using.open = false,
            Some(UseAct::Clear(i)) => {
                if let Some(s) = app.templates.using.slots.get_mut(i) {
                    s.fill = None;
                }
            }
            Some(UseAct::Pick(i)) => {
                let picked = rfd::FileDialog::new().set_title("Footage for this slot").pick_file();
                if let (Some(p), Some(s)) = (picked, app.templates.using.slots.get_mut(i)) {
                    // (id 0: a file, made a library asset when the template is placed)
                    s.fill = Some((0, p.to_string_lossy().into_owned()));
                }
            }
            Some(UseAct::FromLibrary) => {
                let lib = library_pictures(app);
                let cycle = find(app, &app.templates.using.name.clone()).is_some_and(|d| d.sync.is_some());
                fill_from(app, 0, &lib, cycle);
            }
            Some(UseAct::OpenLibrary) => app.surface(Pane::Library),
            Some(UseAct::Drop(i, ids)) => {
                let ids: Vec<Id> = ids.into_iter().filter(|&id| is_picture(app, id) || is_sound(app, id)).collect();
                fill_from(app, i, &ids, false);
            }
            Some(UseAct::Place) => {
                let st = std::mem::take(&mut app.templates.using);
                let fills = st
                    .slots
                    .iter()
                    .map(|s| match &s.fill {
                        Some((0, path)) => json!(path),
                        Some((id, _)) => json!(id),
                        None => Value::Null,
                    })
                    .collect();
                place(app, &st.name, app.playhead, None, fills);
            }
            None => {}
        }
    }
    if app.templates.build.open {
        let name = app.templates.build.name.trim().to_string();
        app.templates.build.exists = app.settings.templates.iter().any(|t| t.name.eq_ignore_ascii_case(&name));
        match template_ui::builder_window(ctx, &mut app.templates.build) {
            Some(BuildAct::Cancel) => app.templates.build.open = false,
            Some(BuildAct::Scope) => scope_rows(app),
            Some(BuildAct::Seek) => {
                let t = app.templates.build.start + app.templates.build.thumb_t;
                app.seek(t);
            }
            Some(BuildAct::Save) => {
                let st = std::mem::take(&mut app.templates.build);
                let tags = gallery::parse_tags(&st.tags);
                match save_built(app, &st.name, &tags, &st.ids, &parts_of(&st.rows), st.thumb_t) {
                    Ok(n) => app.toast(format!("Saved \"{n}\" to the Gallery (Templates)")),
                    Err(e) => {
                        app.templates.build = st; // nothing is lost: the window stays as it was
                        app.toast(e);
                    }
                }
            }
            None => {}
        }
    }
}

// ------------------------------------------------------------------------------------------ tools

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "templates.slots",
        desc: "The slots of a template (yours or a built-in one; names from templates.list / gallery.list Templates): index (1-based), name, wants (any|image|video), seconds, start, fit (trim = footage is cut or slowed to the slot, stretch = the template follows the footage's length), has_footage (the template brings its own). Also its length and clip count.",
        args: &["name:string:true:template name"],
        kind: ToolKind::Read,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?;
            let d = find(app, name).ok_or_else(|| format!("no template '{name}'"))?;
            done(json!({"name": name, "seconds": d.duration(), "clips": d.clips.len(), "transitions": d.transitions.len(),
                        "slots": slots_json(&d)}))
        },
    },
    ToolDef {
        name: "templates.use",
        desc: "Place a template (yours or built-in) at a time with its animation blocks, keyframes, transitions and layers, and fill its slots with footage. fills: an array in slot order, or an object keyed by slot name or 1-based index; each value is a media asset id or a file path (imported), null / missing leaves that slot as a placeholder (or with the template's own footage). Sound slots (wants audio) take a sound: a beat edit's Song slot retimes the template to the song's tempo and cuts the song so the template's drop lands on the song's drop (detected, or the Beat Aligner's Drop section, or `drop`); a sound-effect slot lines the sound's hit up with its moment and, left empty, finds a sound named like its hint in the media library or your SFX / Music folders. One undo step.",
        args: &[
            "name:string:true:template name",
            "at:number:false:timeline seconds (default: the playhead)",
            "fills:object:false:slot -> asset id or file path (or an array in slot order)",
            "track:integer:false:video track index to put its lowest layer on",
            "drop:number:false:a beat edit: the song's drop, song seconds (default: detected)",
            "auto_sounds:boolean:false:fill empty sound-effect slots from your sounds (default true)",
            "nested:boolean:false:place it as ONE sequence clip whose slots (containers) hold the template's own sample footage, as a Gallery drag does (fills and drop are not used)",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?.to_string();
            let at = arg_f64(args, "at").unwrap_or(app.playhead).max(0.0);
            let d = find(app, &name).ok_or_else(|| format!("no template '{name}'"))?;
            if arg_bool(args, "nested").unwrap_or(false) {
                let media = sample_assets(app, &d)?;
                let track = arg_u64(args, "track").map(|t| t as usize);
                let id = templates::place_nested(&mut app.project, &name, d, at, track, &media).ok_or("could not place the template here")?;
                let seq = app.project.clip(id).map_or(0, |c| c.sequence);
                app.selection = app.project.expand_links(&[id]);
                return done(json!({"ok": true, "clip_id": id, "sequence": seq, "media": media.len()}));
            }
            let slots = d.slot_list();
            let sound = |i: usize| slots.get(i).is_some_and(|s| s.want == SlotWant::Audio);
            let mut fills: Vec<(usize, Id)> = Vec::new();
            match args.get("fills") {
                Some(Value::Array(a)) => {
                    for (i, v) in a.iter().enumerate().take(slots.len()) {
                        fills.extend(fill_asset(app, v, sound(i))?.map(|id| (i, id)));
                    }
                }
                Some(Value::Object(o)) => {
                    for (k, v) in o {
                        let i = slots
                            .iter()
                            .position(|s| s.name.eq_ignore_ascii_case(k.trim()))
                            .or_else(|| k.trim().parse::<usize>().ok().filter(|n| (1..=slots.len()).contains(n)).map(|n| n - 1))
                            .ok_or_else(|| format!("'{name}' has no slot '{k}'"))?;
                        fills.extend(fill_asset(app, v, sound(i))?.map(|id| (i, id)));
                    }
                }
                _ => {}
            }
            // an empty sound slot with a hint finds its sound in your library or sound folders
            if arg_bool(args, "auto_sounds").unwrap_or(true) {
                for (i, s) in slots.iter().enumerate() {
                    if s.want == SlotWant::Audio && !s.hint.is_empty() && !fills.iter().any(|f| f.0 == i) {
                        fills.extend(default_sound(app, &s.hint).map(|id| (i, id)));
                    }
                }
            }
            // the template's own media: its fixed parts, and every slot nothing was given for
            let mut d = d;
            if !d.samples.is_empty() {
                let media = sample_assets(app, &d)?;
                templates::fill_fixed(&mut d, &app.project, &media);
                for (i, s) in slots.iter().enumerate() {
                    let sample = d.samples.iter().find(|x| x.clip == s.clip).and_then(|x| media.iter().find(|m| m.0 == x.file));
                    if let (false, Some(m)) = (fills.iter().any(|f| f.0 == i), sample) {
                        fills.push((i, m.1));
                    }
                }
            }
            let song_slot = d.sync.as_ref().and_then(|sy| slots.iter().position(|s| s.clip == sy.song));
            let song = match fills.iter().find(|f| Some(f.0) == song_slot) {
                Some(&(_, asset)) => Some(song_beat(app, asset, arg_f64(args, "drop"), d.sync.as_ref().map_or(0.0, |s| s.drop))?),
                None => None,
            };
            let fills: Vec<templates::Fill> = fills
                .iter()
                .map(|&(slot, asset)| {
                    let onset = if sound(slot) && Some(slot) != song_slot { sound_onset(app, asset) } else { 0.0 };
                    templates::Fill { slot, asset, onset }
                })
                .collect();
            let ids = templates::apply_synced(&mut app.project, d, at, &fills, song);
            if let Some(track) = arg_u64(args, "track") {
                crate::ui::timeline::gallery_drop::pull_to_track(&mut app.project, &ids, Some(track as usize));
            }
            app.selection = ids.clone();
            let beat = song.map(|s| json!({"bpm": s.bpm, "drop": s.drop}));
            done(json!({"ok": true, "clip_ids": ids, "filled": fills.len(), "slots": slots.len(), "song": beat}))
        },
    },
    ToolDef {
        name: "templates.build",
        desc: "Save clips of the timeline as a template in your Gallery (what the Template Builder does). Only video clips can be slots. slots: [{clip_id, name?, wants?: any|image|video, fit?: trim|stretch}] in fill order; placeholders: clip ids whose footage is left out (a video clip becomes an empty slot; a sound clip is dropped). Everything else rides along as it is. Replaces a template of the same name.",
        args: &[
            "name:string:true:template name",
            "clip_ids:array:false:default: the selection, or the whole timeline when nothing is selected",
            "slots:array:false:[{clip_id, name, wants, fit}]",
            "placeholders:array:false:clip ids saved without their footage",
            "thumb_t:number:false:seconds into the template for its Gallery picture",
            "tags:string:false:comma separated",
        ],
        kind: ToolKind::Ui,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?.to_string();
            let mut ids = arg_ids(args, "clip_ids").unwrap_or_else(|| app.selection.clone());
            if ids.is_empty() {
                ids = app.project.all_clips().map(|(_, c)| c.id).collect();
            }
            let ids = app.project.expand_links(&ids);
            let out = arg_ids(args, "placeholders").unwrap_or_default();
            let mut parts: Vec<Part> = Vec::new();
            for s in args.get("slots").and_then(Value::as_array).into_iter().flatten() {
                let clip = req(arg_u64(s, "clip_id"), "slots[].clip_id")?;
                if !app.project.clip(clip).is_some_and(|c| c.kind == ClipKind::Video) {
                    return Err(format!("clip {clip}: only a video clip can be a slot (sound, music and pictures stay the template's own)"));
                }
                let want = match arg_str(s, "wants") {
                    Some(w) => SlotWant::parse(w).ok_or("wants: any | image | video")?,
                    None => SlotWant::Any,
                };
                let fit = match arg_str(s, "fit") {
                    Some(f) => SlotFit::parse(f).ok_or("fit: trim | stretch")?,
                    None => SlotFit::Trim,
                };
                let n = arg_str(s, "name").unwrap_or("").to_string();
                parts.push(Part { clip, slot: Some((n, want, fit)), include: !out.contains(&clip) });
            }
            for &clip in &out {
                if !parts.iter().any(|p| p.clip == clip) {
                    parts.push(Part { clip, slot: None, include: false });
                }
            }
            let tags = gallery::parse_tags(arg_str(args, "tags").unwrap_or(""));
            let saved = save_built(app, &name, &tags, &ids, &parts, arg_f64(args, "thumb_t").unwrap_or(0.0))?;
            let d = find(app, &saved).ok_or("the template was not saved")?;
            done(json!({"ok": true, "name": saved, "slots": slots_json(&d)}))
        },
    },
    ToolDef {
        name: "templates.export",
        desc: "Write a template to one .setemplate file to share. footage (default true): the files its clips use ride along; false (or a file that is gone) makes those clips empty slots. No folder of this PC is written into the file.",
        args: &["name:string:true:template name", "path:string:true:the .setemplate file to write", "footage:boolean:false:default true"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?;
            let path = PathBuf::from(req(arg_str(args, "path"), "path")?);
            let r = export(app, name, &path, arg_bool(args, "footage").unwrap_or(true))?;
            done(json!({"ok": true, "path": path.to_string_lossy(), "footage_files": r.files, "empty_slots": r.empty}))
        },
    },
    ToolDef {
        name: "templates.import",
        desc: "Add a shared .setemplate file to your Gallery (Templates): its footage is unpacked into the gallery folder, footage that is not in the file becomes empty slots, a taken name is numbered.",
        args: &["path:string:true:the .setemplate file"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let path = PathBuf::from(req(arg_str(args, "path"), "path")?);
            let item = templates::import_package(&gallery_mine::root(), &path)?;
            gallery_mine::refreshed(app);
            let d = find(app, &item.name).ok_or("the template was not saved")?;
            done(json!({"ok": true, "name": item.name, "slots": slots_json(&d)}))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_tools_are_registered() {
        for n in ["templates.slots", "templates.use", "templates.build", "templates.export", "templates.import"] {
            assert!(crate::mcp::tools::find(n).is_some(), "{n} must be in TOOL_TABLES");
        }
        // (the op-parity test maps Project::place_template / fill_slot to this one)
        assert!(matches!(crate::mcp::tools::find("templates.use").unwrap().kind, ToolKind::Mutate));
    }

    #[test]
    fn builder_rows_become_parts_in_slot_order() {
        let row = |clip, slot: bool, include| BuildRow {
            clip,
            label: "a".into(),
            kind: ClipKind::Video,
            secs: 1.0,
            slot,
            slot_name: "S".into(),
            want: SlotWant::Video,
            fit: SlotFit::Stretch,
            include,
            open: false,
        };
        let parts = parts_of(&[row(5, true, false), row(3, false, true)]);
        assert_eq!(parts[0], Part { clip: 5, slot: Some(("S".into(), SlotWant::Video, SlotFit::Stretch)), include: false });
        assert_eq!(parts[1], Part { clip: 3, slot: None, include: true });
    }
}
