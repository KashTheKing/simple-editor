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

/// The pictures selected in the Media Library, in selection order.
fn library_pictures(app: &App) -> Vec<Id> {
    let ids = if app.library.sel_ids.is_empty() { app.library.selected.into_iter().collect() } else { app.library.sel_ids.clone() };
    ids.into_iter().filter(|&id| is_picture(app, id)).collect()
}

/// Give `ids` to the slots from `from` on, one each.
fn fill_from(app: &mut App, from: usize, ids: &[Id]) {
    for (k, &id) in ids.iter().enumerate() {
        let name = app.project.asset(id).map(|a| a.name()).unwrap_or_default();
        if let Some(s) = app.templates.using.slots.get_mut(from + k) {
            s.fill = Some((id, name));
        }
    }
}

/// The media selected in the library as `templates.use` fills, in slot order (a template dropped on the
/// timeline takes them without asking).
pub(super) fn library_fills(app: &App, name: &str) -> Vec<Value> {
    let slots = find(app, name).map_or(0, |d| d.slot_list().len());
    library_pictures(app).into_iter().take(slots).map(|id| json!(id)).collect()
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
                sample: c.is_some_and(|c| c.asset != 0),
            }
        })
        .collect();
    app.templates.using = UseState { open: true, name: name.to_string(), slots };
    let lib = library_pictures(app);
    fill_from(app, 0, &lib);
}

/// Place template `name` at `at` (pulled to video track `track` when given) with `fills` (per slot: null,
/// an asset id or a file path), as one undo step.
pub(super) fn place(app: &mut App, name: &str, at: f64, track: Option<usize>, fills: Vec<Value>) {
    let args = json!({"name": name, "at": at, "fills": fills, "track": track});
    match app.run_tool_undoable("templates.use", &args) {
        Ok(_) => gallery_ctl::count_use(app, "Templates", name),
        Err(e) => app.toast(e),
    }
}

/// The asset a fill names: an asset id, or a file (probed now, so the slot's timing can follow its length).
fn fill_asset(app: &mut App, v: &Value) -> Result<Option<Id>, String> {
    if v.is_null() {
        return Ok(None);
    }
    if let Some(id) = v.as_u64() {
        return if is_picture(app, id) { Ok(Some(id)) } else { Err(format!("asset {id} is not a video or an image in this project")) };
    }
    let path = v.as_str().ok_or("a fill is an asset id or a file path")?;
    if let Some(a) = app.project.asset_by_path(path) {
        return Ok(Some(a.id));
    }
    let a = crate::media::probe(path, app.backend()).map_err(|e| format!("{path}: {e}"))?;
    if !matches!(a.kind, ClipKind::Video | ClipKind::Image) {
        return Err(format!("{path}: not a video or an image"));
    }
    Ok(Some(app.project.add_asset(a)))
}

fn slots_json(d: &TemplateData) -> Vec<Value> {
    d.slot_list()
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let c = d.clip(s.clip);
            json!({"index": i + 1, "name": s.name, "wants": s.want.name(), "fit": s.fit.name(),
                   "seconds": c.map_or(0.0, |c| c.duration), "start": c.map_or(0.0, |c| c.start),
                   "has_footage": c.is_some_and(|c| c.asset != 0)})
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
            slot: c.container && c.kind != ClipKind::Audio,
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
                fill_from(app, 0, &lib);
            }
            Some(UseAct::OpenLibrary) => app.surface(Pane::Library),
            Some(UseAct::Drop(i, ids)) => {
                let ids: Vec<Id> = ids.into_iter().filter(|&id| is_picture(app, id)).collect();
                fill_from(app, i, &ids);
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
        desc: "Place a template (yours or built-in) at a time with its animation blocks, keyframes, transitions and layers, and fill its slots with footage. fills: an array in slot order, or an object keyed by slot name or 1-based index; each value is a media asset id or a file path (imported), null / missing leaves that slot as a placeholder (or with the template's own footage). One undo step.",
        args: &[
            "name:string:true:template name",
            "at:number:false:timeline seconds (default: the playhead)",
            "fills:object:false:slot -> asset id or file path (or an array in slot order)",
            "track:integer:false:video track index to put its lowest layer on",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?.to_string();
            let at = arg_f64(args, "at").unwrap_or(app.playhead).max(0.0);
            let d = find(app, &name).ok_or_else(|| format!("no template '{name}'"))?;
            let slots = d.slot_list();
            let mut fills: Vec<(usize, Id)> = Vec::new();
            match args.get("fills") {
                Some(Value::Array(a)) => {
                    for (i, v) in a.iter().enumerate().take(slots.len()) {
                        fills.extend(fill_asset(app, v)?.map(|id| (i, id)));
                    }
                }
                Some(Value::Object(o)) => {
                    for (k, v) in o {
                        let i = slots
                            .iter()
                            .position(|s| s.name.eq_ignore_ascii_case(k.trim()))
                            .or_else(|| k.trim().parse::<usize>().ok().filter(|n| (1..=slots.len()).contains(n)).map(|n| n - 1))
                            .ok_or_else(|| format!("'{name}' has no slot '{k}'"))?;
                        fills.extend(fill_asset(app, v)?.map(|id| (i, id)));
                    }
                }
                _ => {}
            }
            let ids = templates::apply(&mut app.project, d, at, &fills);
            if let Some(track) = arg_u64(args, "track") {
                crate::ui::timeline::gallery_drop::pull_to_track(&mut app.project, &ids, Some(track as usize));
            }
            app.selection = ids.clone();
            done(json!({"ok": true, "clip_ids": ids, "filled": fills.len(), "slots": slots.len()}))
        },
    },
    ToolDef {
        name: "templates.build",
        desc: "Save clips of the timeline as a template in your Gallery (what the Template Builder does). slots: [{clip_id, name?, wants?: any|image|video, fit?: trim|stretch}] in fill order; placeholders: clip ids whose footage is left out (each becomes an empty slot; a sound clip is dropped). Everything else rides along as it is. Replaces a template of the same name.",
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
