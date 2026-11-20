//! ---- ws:zones-markers ----
//! App side of effect zones, marker categories, export between markers and timecode links:
//! - MCP: `zones.add` / `zones.set` / `zones.remove` / `zones.list`, `markers.categories` /
//!   `markers.category_set`, `export.sections` (the `markers.*` category args live with the other marker
//!   tools in `tools_subtitles.rs`);
//! - actions: Next / Previous Marker (of the Markers pane's category), Add Effect Zone, Export Each Section;
//! - FRAME_HOOKS `frame`: a clicked timecode link (`ui::timecode_links`) seeks; picking a clip drops the
//!   picked zone;
//! - PANE_DRAWERS `draw_inspector`: the Inspector shows the picked zone's card (`ui::zones_ui`);
//! - `sections_row`: the Export pane's Export each section button.

use super::tools_helpers::*;
use super::*;
use crate::engine::sections::{self, Section};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::{Effect, EffectKind, EffectZone};
use crate::settings::ExportPresetRef;
use crate::ui::zones_ui;

// ---------- actions ----------

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::NextMarker | Action::PrevMarker => {
            let cat = app.markers.filter_category.clone();
            if let Some(t) = app.project.marker_step(app.playhead, a == Action::NextMarker, cat.as_deref()) {
                app.seek(t);
            }
            true
        }
        Action::AddEffectZone => {
            push_undo_json(&mut app.undo, &mut app.redo, app.project.to_json());
            let (t, d) = app.project.zone_range(app.playhead, &app.selection, None);
            let z = app.project.add_zone(t, d, None, Vec::new());
            zones_ui::select(Some(z));
            app.selection.clear();
            app.after_edit();
            true
        }
        Action::ExportSections => {
            if app.ffmpeg_missing() || app.timeline_is_empty() {
                return true;
            }
            let start = app.export_ui.out_dir.clone();
            let mut dlg = rfd::FileDialog::new().set_title("Export each section to");
            if let Some(d) = start {
                dlg = dlg.set_directory(d);
            }
            if let Some(folder) = dlg.pick_folder() {
                match queue_sections(app, &folder, None, None) {
                    Ok(list) => {
                        let n = list.len();
                        app.toast_with_folder(format!("{n} section{} on the render queue", if n == 1 { "" } else { "s" }), folder);
                    }
                    Err(e) => app.toast(e),
                }
            }
            true
        }
        _ => false,
    }
}

// ---------- frame hook / panes ----------

pub(super) fn frame(app: &mut App, ctx: &egui::Context) {
    if let Some(t) = crate::ui::timecode_links::take_seek(ctx) {
        app.player.pause();
        app.seek(t);
    }
    // a clip picked (timeline, library, an MCP select) wins over a picked zone
    if zones_ui::selected().is_some() && !app.selection.is_empty() {
        zones_ui::select(None);
    }
}

/// PANE_DRAWERS: the Inspector, while a zone is picked in the zone lane.
pub(super) fn draw_inspector(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::Inspector || !zones_ui::selected().is_some_and(|id| app.project.zone(id).is_some()) {
        return false;
    }
    let edited = {
        let App { project, playhead, undo, redo, palette, .. } = app;
        let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
        egui::ScrollArea::vertical().show(ui, |ui| zones_ui::card(ui, project, *playhead, palette, &mut push)).inner
    };
    if edited {
        app.after_edit();
    }
    true
}

/// The Export pane's row: Export each section (one job per section between markers).
pub(super) fn sections_row(app: &mut App, ui: &mut egui::Ui) {
    let n = if app.project.editing.is_none() {
        sections::sections(&app.project).len()
    } else {
        sections::sections(&app.export_project()).len()
    };
    ui.add_space(4.0);
    let r = ui
        .add_enabled_ui(n > 0 && app.export.is_none(), |ui| ui.button(format!("Export each section ({n})")))
        .inner
        .on_hover_text("One file per section between markers, named after the project and the marker, on the render queue")
        .on_disabled_hover_text("Add markers to cut the timeline into sections");
    if r.clicked() {
        app.act(Action::ExportSections);
    }
}

// ---------- export between markers ----------

/// The project's name for file names: its file's, else the one it carries, else "Untitled".
fn project_name(app: &App) -> String {
    app.project_path
        .as_ref()
        .and_then(|p| p.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| app.project.name.clone())
}

/// Queue one export per section into `folder`: the Export pane's settings, or `preset` (+ `ext`) when
/// given. Returns what was queued.
fn queue_sections(
    app: &mut App,
    folder: &std::path::Path,
    preset: Option<ExportPresetRef>,
    ext: Option<&str>,
) -> Result<Vec<(Section, PathBuf)>, String> {
    let project = app.export_project();
    let secs = sections::sections(&project);
    if secs.is_empty() {
        return Err("No sections: add markers to cut the timeline into sections".into());
    }
    let ext = ext.map(str::to_string).unwrap_or_else(|| {
        if app.export_ui.ext.is_empty() {
            "mp4".into()
        } else {
            app.export_ui.ext.clone()
        }
    });
    let names = sections::file_names(&project_name(app), &secs, &ext);
    let mut out = Vec::new();
    for (s, name) in secs.into_iter().zip(names) {
        let path = folder.join(name);
        let mut c = match &preset {
            Some(t) => super::tools_export::quick_choice(t, &project, &app.settings, path.clone())?.0,
            None => crate::ui::export_ui::build_choice(&app.export_ui, &project, &app.settings, path.clone()),
        };
        c.opts.range = Some((s.from, s.to));
        c.lossless = false;
        if files::refuses_source(&app.project, &path) {
            return Err(format!("{} is a source of this project", path.display()));
        }
        app.export_queue.push_back(c);
        out.push((s, path));
    }
    Ok(out)
}

// ---------- MCP ----------

/// An effect kind by its name ("Blur", "hue shift", "Hue Shift"), case and spaces ignored.
fn kind_named(s: &str) -> Option<EffectKind> {
    let norm = |x: &str| x.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    let want = norm(s);
    EffectKind::ALL.iter().copied().find(|k| norm(&format!("{k:?}")) == want || norm(&Effect::new(*k).name()) == want)
}

/// `effects` items: a kind name, or {kind, params: [number | {value, keys}], start, len, enabled}.
fn effect_arg(v: &Value) -> Result<Effect, String> {
    if let Some(s) = v.as_str() {
        return kind_named(s).map(Effect::new).ok_or_else(|| format!("no effect kind '{s}'"));
    }
    let kind = req(arg_str(v, "kind"), "effects[].kind")?;
    let mut fx = kind_named(kind).map(Effect::new).ok_or_else(|| format!("no effect kind '{kind}'"))?;
    if let Some(ps) = v.get("params").and_then(|p| p.as_array()) {
        for (slot, p) in fx.params.iter_mut().zip(ps) {
            match p.as_f64() {
                Some(x) => slot.value = x,
                // through text: from_value would compile a second copy of the deserialiser
                None => *slot = serde_json::from_str(&p.to_string()).map_err(|e| format!("effects[].params: {e}"))?,
            }
        }
    }
    if let Some(s) = arg_f64(v, "start") {
        fx.start = s.max(0.0);
    }
    if let Some(l) = arg_f64(v, "len") {
        fx.len = l.max(0.0);
    }
    if let Some(e) = arg_bool(v, "enabled") {
        fx.enabled = e;
    }
    Ok(fx)
}

fn effects_arg(args: &Value) -> Result<Option<Vec<Effect>>, String> {
    match args.get("effects") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) => a.iter().map(effect_arg).collect::<Result<Vec<_>, _>>().map(Some),
        Some(_) => Err("effects must be an array".into()),
    }
}

/// `track`: a video track id, its name ("V2"), or "all" / null for the whole timeline. Outer None = not given.
fn track_arg(p: &Project, args: &Value) -> Result<Option<Option<Id>>, String> {
    match args.get("track") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(v) => {
            if let Some(s) = v.as_str() {
                if s.eq_ignore_ascii_case("all") || s.is_empty() {
                    return Ok(Some(None));
                }
                let t = p.tracks.iter().find(|t| t.kind == TrackKind::Video && t.name.eq_ignore_ascii_case(s));
                return t.map(|t| Some(Some(t.id))).ok_or_else(|| format!("no video track '{s}'"));
            }
            let id = v.as_u64().ok_or("track: a track id, its name or \"all\"")?;
            p.tracks
                .iter()
                .find(|t| t.id == id && t.kind == TrackKind::Video)
                .map(|_| Some(Some(id)))
                .ok_or_else(|| format!("no video track {id}"))
        }
    }
}

fn zone_json(p: &Project, z: &EffectZone) -> Value {
    let track_name = z.track.and_then(|t| p.tracks.iter().find(|x| x.id == t)).map(|t| t.name.clone());
    json!({"id": z.id, "name": z.name, "label": z.label(), "t": z.t, "dur": z.dur, "end": z.end(),
           "track": z.track, "track_name": track_name, "enabled": z.enabled, "effects": z.effects})
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "zones.list",
        desc: "Effect zones of the open timeline: an effect stack over a time range of one track (track = its id) or \
               of the whole timeline (track = null). Effect keyframe times are zone-local seconds.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| {
            let p = &app.project;
            Ok(ToolOutcome::Done(json!(p.zones_here().map(|z| zone_json(p, z)).collect::<Vec<_>>())))
        },
    },
    ToolDef {
        name: "zones.add",
        desc: "Lay an effect zone over [t, t + dur] (default In/Out, else 5 s at the playhead) on the whole timeline or \
               one video track. effects: kind names (\"Blur\", \"Grayscale\") or {kind, params:[numbers], start, len}.",
        args: &[
            "t:number:false:timeline seconds",
            "dur:number:false:seconds",
            "track:string:false:video track id or name; omit or \"all\" for the whole timeline",
            "effects:array:false:",
            "name:string:false:",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let track = track_arg(&app.project, args)?.flatten();
            let (t0, d0) = app.project.zone_range(app.playhead, &[], track);
            let t = arg_f64(args, "t").unwrap_or(t0);
            let d = arg_f64(args, "dur").unwrap_or(if arg_f64(args, "t").is_some() { 5.0 } else { d0 });
            let fx = effects_arg(args)?.unwrap_or_default();
            let id = app.project.add_zone(t, d, track, fx);
            if let (Some(n), Some(z)) = (arg_str(args, "name"), app.project.zone_mut(id)) {
                z.name = n.to_string();
            }
            let z = app.project.zone(id).map(|z| zone_json(&app.project, z)).unwrap_or_default();
            Ok(ToolOutcome::Done(json!({"ok": true, "id": id, "zone": z})))
        },
    },
    ToolDef {
        name: "zones.set",
        desc: "Edit an effect zone: only the fields given change. effects replaces the stack; add_effect appends one; \
               key = {effect: index, param: name or index, t: zone-local seconds, v} sets a keyframe.",
        args: &[
            "id:integer:true:",
            "t:number:false:",
            "dur:number:false:",
            "track:string:false:video track id or name, or \"all\"",
            "name:string:false:",
            "enabled:boolean:false:",
            "effects:array:false:",
            "add_effect:object:false:",
            "key:object:false:",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = req(arg_u64(args, "id"), "id")?;
            let track = track_arg(&app.project, args)?;
            let fx = effects_arg(args)?;
            let add = args.get("add_effect").map(effect_arg).transpose()?;
            let z = app.project.zone(id).cloned().ok_or("no such zone")?;
            let (t, d) = (arg_f64(args, "t").unwrap_or(z.t), arg_f64(args, "dur").unwrap_or(z.dur));
            app.project.set_zone_range(id, t, d);
            let z = app.project.zone_mut(id).ok_or("no such zone")?;
            if let Some(tr) = track {
                z.track = tr;
            }
            if let Some(n) = arg_str(args, "name") {
                z.name = n.to_string();
            }
            if let Some(e) = arg_bool(args, "enabled") {
                z.enabled = e;
            }
            if let Some(fx) = fx {
                z.effects = fx;
            }
            if let Some(fx) = add {
                z.effects.push(fx);
            }
            if let Some(k) = args.get("key") {
                let i = req(arg_u64(k, "effect"), "key.effect")? as usize;
                let fx = z.effects.get_mut(i).ok_or("key.effect: no such effect in the stack")?;
                let pi = match k.get("param") {
                    Some(Value::String(n)) => fx
                        .specs()
                        .iter()
                        .position(|s| s.name.eq_ignore_ascii_case(n))
                        .ok_or_else(|| format!("key.param: no '{n}' on {}", fx.name()))?,
                    Some(v) => v.as_u64().ok_or("key.param: a name or an index")? as usize,
                    None => 0,
                };
                let (kt, kv) = (req(arg_f64(k, "t"), "key.t")?, req(arg_f64(k, "v"), "key.v")?);
                fx.params.get_mut(pi).ok_or("key.param: no such parameter")?.key_at(kt, kv);
            }
            let z = app.project.zone(id).map(|z| zone_json(&app.project, z)).unwrap_or_default();
            Ok(ToolOutcome::Done(json!({"ok": true, "zone": z})))
        },
    },
    ToolDef {
        name: "zones.remove",
        desc: "Delete an effect zone.",
        args: &["id:integer:true:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = req(arg_u64(args, "id"), "id")?;
            if !app.project.remove_zone(id) {
                return Err("no such zone".into());
            }
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
    ToolDef {
        name: "markers.categories",
        desc: "Marker categories (named types with a colour): name, color, and how many markers use each.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| {
            let p = &app.project;
            let count = |n: &str| {
                p.markers.iter().chain(p.all_clips().flat_map(|(_, c)| c.markers.iter())).filter(|m| m.category == n).count()
            };
            let list: Vec<Value> = p
                .marker_categories
                .iter()
                .enumerate()
                .map(|(i, c)| json!({"index": i, "name": c.name, "color": c.color, "markers": count(&c.name)}))
                .collect();
            Ok(ToolOutcome::Done(json!({"categories": list, "uncategorised": count("")})))
        },
    },
    ToolDef {
        name: "markers.category_set",
        desc: "Add a marker category (name, color), or with index rename / recolour it (markers follow a rename) or \
               remove it (remove=true; its markers become uncategorised).",
        args: &[
            "index:integer:false:0-based; omit to add",
            "name:string:false:",
            "color:array:false:[r,g,b] or \"#rrggbb\"",
            "remove:boolean:false:",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let color = args.get("color").and_then(color_arg).map(|c| [c[0], c[1], c[2]]);
            match arg_u64(args, "index") {
                Some(i) => {
                    let ok = app.project.edit_marker_category(
                        i as usize,
                        arg_str(args, "name"),
                        color,
                        arg_bool(args, "remove").unwrap_or(false),
                    );
                    if !ok {
                        return Err("no such category".into());
                    }
                    Ok(ToolOutcome::Done(json!({"ok": true})))
                }
                None => {
                    let name = req(arg_str(args, "name"), "name")?.trim().to_string();
                    if name.is_empty() {
                        return Err("name is empty".into());
                    }
                    let i = app.project.add_marker_category(name, color);
                    Ok(ToolOutcome::Done(json!({"ok": true, "index": i})))
                }
            }
        },
    },
    ToolDef {
        name: "export.sections",
        desc: "Export each section between markers (In/Out bounds them; a range marker is its own section) as its own \
               file in folder, named \"<project> - <marker name>\", queued on the render queue. dry_run lists them only.",
        args: &[
            "folder:string:true:",
            "preset:string:false:preset name, default the Export pane's settings",
            "ext:string:false:container, default the Export pane's (mp4)",
            "dry_run:boolean:false:",
        ],
        // Ui: the queue is App state, not the project
        kind: ToolKind::Ui,
        run: |app, args| {
            let folder = PathBuf::from(req(arg_str(args, "folder"), "folder")?);
            if arg_bool(args, "dry_run").unwrap_or(false) {
                let project = app.export_project();
                let secs = sections::sections(&project);
                let ext = arg_str(args, "ext").unwrap_or("mp4");
                let names = sections::file_names(&project_name(app), &secs, ext);
                let list: Vec<Value> = secs
                    .iter()
                    .zip(names)
                    .map(|(s, n)| json!({"name": s.name, "from": s.from, "to": s.to, "path": folder.join(n)}))
                    .collect();
                return Ok(ToolOutcome::Done(json!({"sections": list})));
            }
            if !folder.is_dir() {
                return Err(format!("{} is not a folder", folder.display()));
            }
            let preset = arg_str(args, "preset").map(|n| ExportPresetRef::Preset(n.to_string()));
            let queued = queue_sections(app, &folder, preset, arg_str(args, "ext"))?;
            let list: Vec<Value> =
                queued.iter().map(|(s, p)| json!({"name": s.name, "from": s.from, "to": s.to, "path": p})).collect();
            Ok(ToolOutcome::Done(json!({"ok": true, "queued": list, "waiting": app.export_queue.len()})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_args_by_name_or_object() {
        assert_eq!(kind_named("hue shift"), Some(EffectKind::HueShift));
        assert_eq!(kind_named("Grayscale"), Some(EffectKind::Grayscale));
        assert!(kind_named("nope").is_none());
        let fx = effect_arg(&json!({"kind": "Blur", "params": [12.0], "start": 1.0})).unwrap();
        assert_eq!((fx.kind, fx.params[0].value, fx.start), (EffectKind::Blur, 12.0, 1.0));
        let keyed = effect_arg(&json!({"kind": "Blur", "params": [{"value": 0.0, "keys": [{"t": 0.0, "v": 0.0}, {"t": 2.0, "v": 9.0}]}]}));
        assert_eq!(keyed.unwrap().params[0].keys.len(), 2);
        assert!(effect_arg(&json!("Sparkles")).is_err());
    }
}
