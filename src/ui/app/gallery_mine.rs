//! "Save to Gallery...": your own growing Gallery, App side. ONE funnel (`save` / `put`) for everything
//! that can be kept - effect stacks, single effects, transitions, looks, caption styles, speed ramps,
//! titles / templates, keyframe blocks, HTML layers, shader effects / transitions, LUTs and sounds - used by
//! the dialog (clip / effect / transition right-click, the Clip menu, the Gallery's empty-space menu), by
//! `gallery.save` over MCP and by the older save paths (`stills.save`, `keyblocks.save`, `templates.save`).
//! Files live in `%APPDATA%\SimpleEditor\gallery\<kind>\` (`engine::gallery`; library-backed kinds - shader
//! effects / transitions, HTML, LUTs - go to the existing local library, which already lists "yours").
//! Also here: the "Mine" actions (rename, tags, duplicate, replace with current, delete, reveal), the
//! importer and the sound preview.

use super::*;
use crate::engine::gallery::{self, Collide, Item, Kind as GKind};
use crate::engine::library;
use serde_json::json;

/// What a "Save to Gallery" captures. The strings are `gallery.save`'s `kind` argument.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Save {
    /// The first selected clip's effect stack (or node graph): replaces on apply.
    Look,
    /// One effect of the first selected clip (`Some(i)`), or its whole stack added on apply (`None`).
    Effect(Option<usize>),
    Transition(Id),
    Caption,
    Speed,
    Animate,
    Template,
    Sound,
    ShaderEffect(usize),
    ShaderTransition(Id),
    Html,
    Lut(usize),
}

impl Save {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Save::Look => "Look (the whole effect stack)",
            Save::Effect(Some(_)) => "This effect, with its settings",
            Save::Effect(None) => "Effects (the stack, added on use)",
            Save::Transition(_) => "Transition (length, colour, direction)",
            Save::Caption => "Caption style (the project's)",
            Save::Speed => "Speed ramp (the clip's curve)",
            Save::Animate => "Animation (the clip's keyframes)",
            Save::Template => "Template / title (the selected clips)",
            Save::Sound => "Sound (the clip's audio)",
            Save::ShaderEffect(_) => "Shader effect (the GLSL, to the library)",
            Save::ShaderTransition(_) => "Shader transition (the GLSL, to the library)",
            Save::Html => "HTML layer (to the library)",
            Save::Lut(_) => "LUT (the .cube, to the library)",
        }
    }
    /// The Gallery tab where it will show.
    pub(crate) fn tab(&self) -> &'static str {
        match self {
            Save::Look => "Looks",
            Save::Effect(_) | Save::ShaderEffect(_) => "Effects",
            Save::Transition(_) | Save::ShaderTransition(_) => "Transitions",
            Save::Caption => "Captions",
            Save::Speed => "Speed",
            Save::Animate => "Animate",
            Save::Template => "Templates",
            Save::Sound => "Sounds",
            Save::Html => "HTML",
            Save::Lut(_) => "Luts",
        }
    }
}

/// The "Save to Gallery" dialog, or the rename / tags prompt of a saved item.
#[derive(Default)]
pub struct SaveDialog {
    pub open: bool,
    pub choices: Vec<Save>,
    pub pick: usize,
    pub name: String,
    pub tags: String,
    pub replace: bool,
    /// Editing a saved item (rename / tags) instead of making a new one.
    pub edit: Option<(MineRef, bool)>,
    pub just_opened: bool,
}

/// A saved item as the Gallery names it: its kind and its display name.
#[derive(Clone, Debug, PartialEq)]
pub struct MineRef {
    pub kind: GKind,
    pub name: String,
}

/// A right-click choice on a saved ("Mine") card.
#[derive(Clone, Debug, PartialEq)]
pub enum MineAct {
    Rename,
    Tags,
    Duplicate,
    ReplaceCurrent,
    Delete,
    Reveal,
    /// ws:animator-templates: a template as one `.setemplate` file (`true` = with its footage).
    Export(bool),
}

pub(crate) fn root() -> std::path::PathBuf {
    gallery::root()
}

fn transition_of(p: &Project, id: Id) -> Option<&crate::model::Transition> {
    p.tracks.iter().flat_map(|t| t.transitions.iter()).find(|t| t.id == id)
}

/// Everything the selection (and the selected transition) can be saved as, best first.
pub(crate) fn available(app: &App) -> Vec<Save> {
    let mut v = Vec::new();
    let clip = app.selection.iter().find_map(|&id| app.project.clip(id));
    if let Some(c) = clip {
        if c.kind == ClipKind::Audio {
            v.push(Save::Sound);
        } else {
            if c.graph.is_some() || !c.effects.is_empty() {
                v.push(Save::Look);
            }
            if !c.effects.is_empty() {
                v.push(Save::Effect(None));
            }
            if !c.speed_curve.keys.is_empty() {
                v.push(Save::Speed);
            }
            if app.project.asset(c.asset).is_some_and(|a| a.html.is_some()) {
                v.push(Save::Html);
            }
            v.push(Save::Animate);
        }
        for (i, e) in c.effects.iter().enumerate() {
            match e.kind {
                EffectKind::Shader => v.push(Save::ShaderEffect(i)),
                EffectKind::Lut if !e.lut.is_empty() => v.push(Save::Lut(i)),
                _ => {}
            }
        }
        v.push(Save::Template);
    }
    if let Some(&t) = app.sel_transitions.first() {
        if let Some(tr) = transition_of(&app.project, t) {
            let shader = tr.kind == TransitionKind::Shader && !tr.shader.is_empty();
            v.insert(0, Save::Transition(t));
            if shader {
                v.insert(1, Save::ShaderTransition(t));
            }
        }
    }
    v.push(Save::Caption);
    v
}

/// What `what` captures right now: JSON for a gallery kind, GLSL / HTML text for a library kind.
enum Captured {
    Json(GKind, Value),
    Library(library::Kind, String),
    /// A sound: source file, source-time window.
    Sound(std::path::PathBuf, f64, f64, bool),
}

fn capture(app: &mut App, what: &Save, name: &str) -> Result<Captured, String> {
    let first = app.selection.iter().copied().find(|&id| app.project.clip(id).is_some());
    fn clip_of(app: &App, first: Option<Id>) -> Result<&Clip, String> {
        first.and_then(|id| app.project.clip(id)).ok_or_else(|| "Select a clip first".to_string())
    }
    Ok(match what {
        Save::Look => {
            let c = clip_of(app, first)?;
            if c.graph.is_none() && c.effects.is_empty() {
                return Err("that clip has no effects to save".into());
            }
            let p = crate::engine::presets::capture_effects(name, c);
            Captured::Json(GKind::Look, serde_json::from_str(&p.json).map_err(|e| e.to_string())?)
        }
        Save::Effect(i) => {
            let c = clip_of(app, first)?;
            let fx: Vec<&Effect> = match i {
                Some(i) => c.effects.get(*i).into_iter().collect(),
                None => c.effects.iter().collect(),
            };
            if fx.is_empty() {
                return Err("that clip has no such effect".into());
            }
            Captured::Json(GKind::Effect, json!({"effects": fx}))
        }
        Save::Transition(t) | Save::ShaderTransition(t) => {
            let tr = transition_of(&app.project, *t).ok_or("no such transition")?.clone();
            if let Save::ShaderTransition(_) = what {
                if tr.shader.is_empty() {
                    return Err("that transition is not a shader".into());
                }
                return Ok(Captured::Library(library::Kind::Transition, tr.shader));
            }
            Captured::Json(
                GKind::Transition,
                json!({"kind": tr.kind.name(), "duration": tr.duration, "color": tr.color, "direction": tr.direction,
                       "ease": tr.ease, "shader": tr.shader, "params": tr.params}),
            )
        }
        Save::Caption => {
            let mut st = app.project.subtitle_style.clone();
            st.text = name.to_string();
            Captured::Json(GKind::Caption, serde_json::to_value(&st).map_err(|e| e.to_string())?)
        }
        Save::Speed => {
            let c = clip_of(app, first)?;
            if c.speed_curve.keys.is_empty() {
                return Err("that clip has no speed ramp".into());
            }
            Captured::Json(GKind::Speed, json!({"absolute": true, "keys": c.speed_curve.keys}))
        }
        Save::Animate => {
            let id = first.ok_or("Select a clip first")?;
            let (w, h) = (app.project.width as f64, app.project.height as f64);
            let c = app.project.clip(id).ok_or("no such clip")?;
            let b = c
                .capture_block(name, 0.0, c.duration, w, h)
                .ok_or("need keys at two or more times on position / scale / rotation / opacity / volume")?;
            Captured::Json(GKind::Animate, serde_json::to_value(&b).map_err(|e| e.to_string())?)
        }
        Save::Template => {
            if app.selection.is_empty() {
                return Err("Select a clip first".into());
            }
            let t = crate::engine::presets::capture_template(name, &app.project, &app.selection);
            Captured::Json(GKind::Template, serde_json::from_str(&t.json).map_err(|e| e.to_string())?)
        }
        Save::Html => {
            let c = clip_of(app, first)?;
            let h = app.project.asset(c.asset).and_then(|a| a.html.as_ref()).ok_or("that clip is not an HTML layer")?;
            Captured::Library(library::Kind::Html, h.src.clone())
        }
        Save::ShaderEffect(i) => {
            let c = clip_of(app, first)?;
            let e = c.effects.get(*i).filter(|e| e.kind == EffectKind::Shader).ok_or("no such shader effect")?;
            Captured::Library(library::Kind::Effect, e.shader.clone())
        }
        Save::Lut(i) => {
            let c = clip_of(app, first)?;
            let e = c.effects.get(*i).filter(|e| e.kind == EffectKind::Lut).ok_or("no such LUT effect")?;
            let text = std::fs::read_to_string(&e.lut).map_err(|er| format!("{}: {er}", e.lut))?;
            Captured::Library(library::Kind::Lut, text)
        }
        Save::Sound => {
            let c = clip_of(app, first)?;
            if c.kind != ClipKind::Audio {
                return Err("select an audio clip".into());
            }
            let a = app.project.asset(c.asset).ok_or("no such asset")?;
            let whole = c.src_in <= 0.001 && (c.src_in + c.duration * c.speed - a.duration).abs() < 0.05;
            Captured::Sound(std::path::PathBuf::from(&a.path), c.src_in, c.duration * c.speed.max(0.01), whole)
        }
    })
}

/// Write one JSON item and make the app see it right away.
pub(crate) fn put(
    app: &mut App,
    kind: GKind,
    name: &str,
    tags: &[String],
    data: Value,
    replace: bool,
) -> Result<Item, String> {
    let how = if replace { Collide::Replace } else { Collide::Number };
    let item = gallery::save(&root(), kind, name, tags, data, how)?;
    refreshed(app);
    Ok(item)
}

/// The folder changed: the Settings view of the four old kinds and every cached listing are re-read.
pub(crate) fn refreshed(app: &mut App) {
    crate::engine::gallery_sync::reload(&mut app.settings, &root());
    crate::ui::gallery::forget_mine();
}

/// Save what `what` captures under `name`. Returns the name it was saved as (numbered when taken, unless
/// `replace`) and where it shows.
pub(crate) fn save(app: &mut App, what: &Save, name: &str, tags: &[String], replace: bool) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a Gallery item needs a name".into());
    }
    let saved = match capture(app, what, name)? {
        Captured::Json(kind, data) => put(app, kind, name, tags, data, replace)?.name,
        Captured::Library(kind, text) => {
            let mut n = name.to_string();
            if !replace {
                let mut i = 2;
                while library::path_of(kind, &n).is_file() {
                    n = format!("{name} ({i})");
                    i += 1;
                }
            }
            // keep the display name: a shader / page names itself with its header, so the file name is the name
            let it = library::save(kind, &n, &text)?;
            app.online.installed.insert(library::full_id(kind, &it.id));
            crate::ui::gallery::forget_lut_listing();
            crate::ui::transitions_ui::refresh_library();
            n
        }
        Captured::Sound(src, from, len, whole) => {
            let f = sound_file(app, &src, from, len, whole, name)?;
            let dur = if whole { None } else { Some(len) };
            let it = gallery::import_file(&root(), &f, tags, dur)?;
            let it = if it.name != name && !name.is_empty() {
                gallery::rename(&root(), &it, name).unwrap_or(it)
            } else {
                it
            };
            // (only a file WE cut for this save is ours to delete - never the source itself)
            if f != src {
                let _ = std::fs::remove_file(&f);
            }
            refreshed(app);
            it.name
        }
    };
    app.gallery.tab = tab_of(what.tab()).unwrap_or(app.gallery.tab);
    Ok(saved)
}

fn tab_of(label: &str) -> Option<crate::ui::gallery::GalleryTab> {
    use crate::ui::gallery::GalleryTab as T;
    Some(match label {
        "Looks" => T::Looks,
        "Effects" => T::Effects,
        "Transitions" => T::Transitions,
        "Captions" => T::Captions,
        "Speed" => T::SpeedRamps,
        "Animate" => T::Animate,
        "Templates" => T::Templates,
        "Sounds" => T::Sounds,
        "Luts" => T::Luts,
        _ => return None,
    })
}

/// The audio file to keep: the source itself for a whole file, else the window cut out with ffmpeg.
fn sound_file(
    app: &App,
    src: &std::path::Path,
    from: f64,
    len: f64,
    whole: bool,
    name: &str,
) -> Result<std::path::PathBuf, String> {
    if whole && gallery::is_audio(src) {
        return Ok(src.to_path_buf());
    }
    let ffmpeg = crate::media::ffpipe::ffmpeg_exe().ok_or("ffmpeg was not found")?;
    let _ = app;
    let out = std::env::temp_dir().join(format!("{}.wav", library::slug(name)));
    let mut c = std::process::Command::new(ffmpeg);
    c.args(["-y", "-v", "error", "-ss", &format!("{from:.3}"), "-t", &format!("{len:.3}"), "-i"])
        .arg(src)
        .args(["-vn", "-c:a", "pcm_s16le"])
        .arg(&out);
    hide_window(&mut c);
    let st = c.output().map_err(|e| e.to_string())?;
    if st.status.success() && out.is_file() {
        Ok(out)
    } else {
        Err(format!("could not cut the sound: {}", String::from_utf8_lossy(&st.stderr).lines().next().unwrap_or("")))
    }
}

fn hide_window(c: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let _ = c;
}

// ------------------------------------------------------------------------------------ dialog

/// Open the dialog on what the selection can be saved as (`preferred`: a choice to start on).
pub(crate) fn open_dialog(app: &mut App, preferred: Option<Save>) {
    let choices = available(app);
    if choices.is_empty() {
        app.toast("Select a clip (or a transition) to save to the Gallery");
        return;
    }
    let d = &mut app.save_dialog;
    d.pick = preferred.and_then(|p| choices.iter().position(|c| *c == p)).unwrap_or(0);
    d.choices = choices;
    d.name.clear();
    d.tags.clear();
    d.replace = false;
    d.edit = None;
    d.open = true;
    d.just_opened = true;
}

/// Rename / edit the tags of a saved item.
fn open_edit(app: &mut App, r: MineRef, tags_only: bool) {
    let item = find(&r);
    let d = &mut app.save_dialog;
    d.name = r.name.clone();
    d.tags = item.map(|i| i.tags.join(", ")).unwrap_or_default();
    d.edit = Some((r, tags_only));
    d.choices.clear();
    d.open = true;
    d.just_opened = true;
}

pub(super) fn windows(app: &mut App, ctx: &egui::Context) {
    if !app.save_dialog.open {
        return;
    }
    let mut open = true;
    let (mut go, mut cancel) = (false, false);
    let title = match &app.save_dialog.edit {
        Some((_, true)) => "Edit tags",
        Some(_) => "Rename",
        None => "Save to Gallery",
    };
    let exists = {
        let d = &app.save_dialog;
        d.edit.is_none() && d.choices.get(d.pick).is_some_and(|w| name_taken(w, d.name.trim()))
    };
    egui::Window::new(title)
        .id(egui::Id::new("save_to_gallery"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let d = &mut app.save_dialog;
            if d.edit.is_none() {
                egui::ComboBox::from_id_salt("sg_kind")
                    .width(300.0)
                    .selected_text(d.choices.get(d.pick).map_or("", |c| c.label()))
                    .show_ui(ui, |ui| {
                        for (i, c) in d.choices.iter().enumerate() {
                            ui.selectable_value(&mut d.pick, i, c.label());
                        }
                    });
                if let Some(c) = d.choices.get(d.pick) {
                    ui.weak(format!("Shows in Gallery ▸ {} (marked Mine)", c.tab()));
                }
            }
            let tags_only = matches!(d.edit, Some((_, true)));
            ui.horizontal(|ui| {
                ui.label("Name");
                let r = ui.add_enabled(!tags_only, egui::TextEdit::singleline(&mut d.name).desired_width(240.0));
                if d.just_opened {
                    r.request_focus();
                    d.just_opened = false;
                }
                go |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            });
            ui.horizontal(|ui| {
                ui.label("Tags");
                ui.add(egui::TextEdit::singleline(&mut d.tags).hint_text("comma, separated").desired_width(240.0));
            });
            if exists {
                ui.checkbox(&mut d.replace, "Replace the one with this name (otherwise it is numbered)");
            }
            ui.horizontal(|ui| {
                let ok = !d.name.trim().is_empty();
                go |= ui.add_enabled(ok, egui::Button::new("Save")).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
    if cancel || !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.save_dialog.open = false;
        return;
    }
    if !go {
        return;
    }
    let d = std::mem::take(&mut app.save_dialog);
    let tags = gallery::parse_tags(&d.tags);
    let res = match d.edit {
        Some((r, tags_only)) => edit_saved(app, &r, d.name.trim(), &tags, tags_only),
        None => match d.choices.get(d.pick).cloned() {
            Some(w) => {
                save(app, &w, &d.name, &tags, d.replace).map(|n| format!("Saved \"{n}\" to the Gallery ({})", w.tab()))
            }
            None => Err("nothing to save".into()),
        },
    };
    match res {
        Ok(m) => app.toast(m),
        Err(e) => app.toast(e),
    }
}

fn name_taken(w: &Save, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    match w {
        Save::ShaderEffect(_) => library::path_of(library::Kind::Effect, name).is_file(),
        Save::ShaderTransition(_) => library::path_of(library::Kind::Transition, name).is_file(),
        Save::Html => library::path_of(library::Kind::Html, name).is_file(),
        Save::Lut(_) => library::path_of(library::Kind::Lut, name).is_file(),
        other => kind_of(other).is_some_and(|k| gallery::find(&root(), k, name).is_some()),
    }
}

fn kind_of(w: &Save) -> Option<GKind> {
    Some(match w {
        Save::Look => GKind::Look,
        Save::Effect(_) => GKind::Effect,
        Save::Transition(_) => GKind::Transition,
        Save::Caption => GKind::Caption,
        Save::Speed => GKind::Speed,
        Save::Animate => GKind::Animate,
        Save::Template => GKind::Template,
        Save::Sound => GKind::Sound,
        _ => return None,
    })
}

// ------------------------------------------------------------------------------------ Mine actions

fn find(r: &MineRef) -> Option<Item> {
    gallery::find(&root(), r.kind, &r.name)
}

pub(crate) fn edit_saved(
    app: &mut App,
    r: &MineRef,
    name: &str,
    tags: &[String],
    tags_only: bool,
) -> Result<String, String> {
    let item = find(r).ok_or_else(|| format!("'{}' is not in your Gallery any more", r.name))?;
    let item = gallery::set_tags(&root(), &item, tags)?;
    let item = if tags_only || name == item.name { item } else { gallery::rename(&root(), &item, name)? };
    refreshed(app);
    Ok(format!("Saved \"{}\"", item.name))
}

/// A right-click choice on a Mine card.
pub(crate) fn act(app: &mut App, what: MineAct, r: MineRef) {
    match do_act(app, what, r) {
        Ok(m) if !m.is_empty() => app.toast(m),
        Err(e) => app.toast(e),
        _ => {}
    }
}

/// `act`'s work, with the message returned (Rename / Edit tags open the dialog and say nothing).
pub(crate) fn do_act(app: &mut App, what: MineAct, r: MineRef) -> Result<String, String> {
    {
        let item = find(&r).ok_or_else(|| format!("'{}' is not in your Gallery any more", r.name))?;
        match what {
            MineAct::Rename => {
                open_edit(app, r.clone(), false);
                Ok(String::new())
            }
            MineAct::Tags => {
                open_edit(app, r.clone(), true);
                Ok(String::new())
            }
            MineAct::Duplicate => {
                let c = gallery::duplicate(&root(), &item)?;
                refreshed(app);
                Ok(format!("Duplicated as \"{}\"", c.name))
            }
            MineAct::Delete => {
                gallery::delete(&root(), &item)?;
                refreshed(app);
                Ok(format!("Removed \"{}\" from your Gallery (kept in the gallery folder's .trash)", item.name))
            }
            MineAct::Reveal => {
                reveal(&item);
                Ok(String::new())
            }
            MineAct::Export(footage) => {
                super::template_ctl::export_dialog(app, &item.name, footage);
                Ok(String::new())
            }
            MineAct::ReplaceCurrent => {
                let want = match r.kind {
                    GKind::Look => Save::Look,
                    GKind::Effect => Save::Effect(None),
                    GKind::Transition => available(app)
                        .into_iter()
                        .find(|s| matches!(s, Save::Transition(_)))
                        .ok_or("select a transition on the timeline first")?,
                    GKind::Caption => Save::Caption,
                    GKind::Speed => Save::Speed,
                    GKind::Animate => Save::Animate,
                    GKind::Template => Save::Template,
                    GKind::Sound => return Err("a sound cannot be replaced: save a new one".into()),
                };
                match capture(app, &want, &item.name)? {
                    Captured::Json(_, data) => {
                        gallery::replace_data(&root(), &item, data)?;
                        refreshed(app);
                        Ok(format!("\"{}\" now holds the current one", item.name))
                    }
                    _ => Err("not a Gallery item".into()),
                }
            }
        }
    }
}

/// Show the item's file in Explorer.
pub(crate) fn reveal(item: &Item) {
    let p = item.file.as_ref().unwrap_or(&item.path);
    if p.exists() {
        let _ = std::process::Command::new("explorer").arg("/select,").arg(p).spawn();
    } else if let Some(d) = p.parent() {
        let _ = std::process::Command::new("explorer").arg(d).spawn();
    }
}

/// Open the gallery folder (of one kind: `Some`).
pub(crate) fn open_folder(kind: Option<GKind>) {
    let dir = kind.map_or_else(root, |k| root().join(k.dir()));
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::process::Command::new("explorer").arg(&dir).spawn();
}

/// "Import to Gallery...": files, folders and .zips. Names that are taken are numbered.
pub(crate) fn import(app: &mut App, paths: &[std::path::PathBuf]) -> Value {
    let r = gallery::import(&root(), paths);
    refreshed(app);
    let skipped: Vec<Value> =
        r.skipped.iter().map(|(p, why)| json!({"file": p.display().to_string(), "why": why})).collect();
    let added: Vec<Value> = r.added.iter().map(|(k, n)| json!({"kind": k.name(), "name": n})).collect();
    app.toast(match (r.added.len(), r.skipped.len()) {
        (0, 0) => "Nothing to import".to_string(),
        (a, 0) => format!("Imported {a} item{} to the Gallery", if a == 1 { "" } else { "s" }),
        (a, s) => format!("Imported {a} to the Gallery, skipped {s} (not Gallery items)"),
    });
    json!({"added": added, "skipped": skipped})
}

/// "Import to Gallery..." buttons: pick files (items, audio, zips) or a folder.
pub(crate) fn import_dialog(app: &mut App, folder: bool) {
    let paths = if folder {
        rfd::FileDialog::new().set_title("Import a gallery folder").pick_folder().map(|f| vec![f])
    } else {
        rfd::FileDialog::new()
            .set_title("Import to Gallery")
            .add_filter(
                "Gallery items, templates, sounds, zips",
                &["json", "setemplate", "zip", "wav", "mp3", "ogg", "flac", "m4a", "aac", "opus", "aif", "aiff", "wma"],
            )
            .pick_files()
    };
    if let Some(p) = paths {
        import(app, &p);
    }
}

// ------------------------------------------------------------------------------------ sounds

/// Add the sound (by name) at `at` on audio track `track` (None: the first free one; a negative / new
/// track = `new_track`). Returns the new clip ids.
pub(crate) fn place_sound(
    app: &mut App,
    name: &str,
    at: f64,
    track: Option<usize>,
    new_track: bool,
) -> Result<Vec<Id>, String> {
    let item =
        gallery::find(&root(), GKind::Sound, name).ok_or_else(|| format!("no sound '{name}' in your Gallery"))?;
    let file = item.file.ok_or("no file")?;
    // (probed right now - a short audio file is quick - so the clip is placed with its real length and
    // streams; the media importer's background probe would swap the clip for a new one a moment later)
    let path = file.to_string_lossy().into_owned();
    let aid = match app.project.asset_by_path(&path) {
        Some(a) => a.id,
        None => {
            let a = crate::media::probe(&path, app.backend())?;
            app.project.add_asset(a)
        }
    };
    let (vt, at_tr) = if new_track { (None, Some(app.project.add_track(TrackKind::Audio))) } else { (None, track) };
    let placed = app.project.insert_asset_clips_ranged(aid, at.max(0.0), vt, at_tr, None);
    if placed.is_empty() {
        return Err("could not place the sound there".into());
    }
    Ok(placed)
}

#[cfg(windows)]
pub(crate) mod preview {
    //! Sound preview: a WAV plays straight through `PlaySound`; anything else is first cut to a short WAV
    //! in the temp folder (lazily, once per file). Nothing is decoded until a sound is previewed.
    use std::path::{Path, PathBuf};
    use windows::core::PCWSTR;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME, SND_PURGE};

    fn wav_for(src: &Path) -> Option<PathBuf> {
        if src.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav")) {
            return Some(src.to_path_buf());
        }
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            src.hash(&mut h);
            std::fs::metadata(src).and_then(|m| m.modified()).ok().hash(&mut h);
            h.finish()
        };
        let out = std::env::temp_dir().join(format!("simple-editor-sfx-preview-{key:x}.wav"));
        if out.is_file() {
            return Some(out);
        }
        let ffmpeg = crate::media::ffpipe::ffmpeg_exe()?;
        let mut c = std::process::Command::new(ffmpeg);
        c.args(["-y", "-v", "error", "-t", "20", "-i"]).arg(src).args(["-vn", "-c:a", "pcm_s16le"]).arg(&out);
        super::hide_window(&mut c);
        (c.output().ok()?.status.success() && out.is_file()).then_some(out)
    }

    pub fn play(src: &Path) {
        if let Some(w) = wav_for(src) {
            let wide: Vec<u16> = w.to_string_lossy().encode_utf16().chain(Some(0)).collect();
            unsafe {
                let _ = PlaySoundW(PCWSTR(wide.as_ptr()), None, SND_FILENAME | SND_ASYNC);
            }
        }
    }

    pub fn stop() {
        unsafe {
            let _ = PlaySoundW(PCWSTR::null(), None, SND_PURGE);
        }
    }
}

#[cfg(not(windows))]
pub(crate) mod preview {
    pub fn play(_: &std::path::Path) {}
    pub fn stop() {}
}

/// Frame hook: start / stop the hover preview when the Gallery's wish changed (`gallery::sound_wish`).
pub(super) fn frame(app: &mut App, _ctx: &egui::Context) {
    let want = crate::ui::gallery::take_sound_wish();
    if want != app.sound_playing {
        match &want {
            Some(p) => preview::play(p),
            None => preview::stop(),
        }
        app.sound_playing = want;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_save_kind_has_a_string_label_and_tab() {
        for s in [
            Save::Look,
            Save::Effect(None),
            Save::Transition(1),
            Save::Caption,
            Save::Speed,
            Save::Animate,
            Save::Template,
            Save::Sound,
            Save::ShaderEffect(0),
            Save::ShaderTransition(1),
            Save::Html,
            Save::Lut(0),
        ] {
            assert!(!s.label().is_empty() && !s.tab().is_empty());
        }
        assert_eq!(tab_of("Speed"), Some(crate::ui::gallery::GalleryTab::SpeedRamps));
        assert_eq!(Save::Transition(3).tab(), "Transitions");
    }

    #[test]
    fn name_taken_looks_in_the_right_place() {
        // nothing saved under this name in the (test) gallery or library
        assert!(!name_taken(&Save::Look, "zz-never-saved-look"));
        assert!(!name_taken(&Save::Html, "zz-never-saved-html"));
        assert!(!name_taken(&Save::Look, ""));
    }
}
