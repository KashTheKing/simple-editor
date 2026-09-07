//! ---- ws:inspector-gallery ----
//! 7 MCP tools for capabilities this workstream actually adds: gallery.list/apply/hover (the Gallery
//! pane's tool surface — the SOLE surface for Looks; color-engine's `looks.list`/`looks.apply` cover the
//! same ground for a bare clip_id/name call but this file never re-registers those names),
//! clip.reorder_effect/clip.effects_bulk (Project::reorder_effect/bulk_set_effect_params),
//! subtitles.style_preset, inspector.folds. Does NOT register clip.add_lut/color.auto/color.match
//! (color-engine's) — `tool_names_are_sole_registration` (tools_registry_tests.rs, color-engine's own
//! structural test) already pins those three to exactly one registration crate-wide.

use super::tools_args::Args;
use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::gallery::GalleryTab;

fn done(v: Value) -> Result<ToolOutcome, String> {
    Ok(ToolOutcome::Done(v))
}

/// `gallery.apply`'s per-tab body: Looks/Luts/Captions/SpeedRamps target `clip_ids` (Captions is
/// project-wide, `clip_ids` ignored), Transitions adds at the cuts around `clip_ids`. Templates are NOT
/// handled here — `gallery.rs`'s own `GalleryResponse.place` routes those through the existing
/// `App::place_template`/`templates.apply`, since a template places POSITIONALLY, not per-clip.
fn apply_card(app: &mut App, tab: GalleryTab, name: &str, clip_ids: &[Id], intensity: f32) -> Result<Value, String> {
    match tab {
        GalleryTab::Looks => {
            let preset = crate::engine::presets::builtin_looks()
                .into_iter()
                .chain(app.settings.effect_presets.iter().filter(|p| !p.is_graph()).cloned())
                .find(|p| p.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("no such Look '{name}'"))?;
            let mut n = 0;
            for &id in clip_ids {
                if crate::engine::presets::apply_look(&preset, &mut app.project, id, intensity) {
                    n += 1;
                }
            }
            Ok(json!({"ok": true, "count": n}))
        }
        GalleryTab::Luts => {
            crate::engine::lut::load(name).map_err(|e| format!("bad LUT: {e}"))?;
            let mut n = 0;
            for &id in clip_ids {
                if let Some(c) = app.project.clip_mut(id) {
                    let mut e = Effect::new(EffectKind::Lut);
                    e.lut = name.to_string();
                    e.params[0].value = intensity as f64;
                    c.effects.push(e);
                    n += 1;
                }
            }
            Ok(json!({"ok": true, "count": n}))
        }
        GalleryTab::Captions => {
            let style = crate::engine::presets::builtin_caption_styles()
                .into_iter()
                .chain(app.settings.caption_presets.iter().cloned())
                .find(|s| s.text.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("no such caption style '{name}'"))?;
            app.project.subtitle_style = style;
            Ok(json!({"ok": true}))
        }
        GalleryTab::SpeedRamps => {
            let preset = crate::engine::presets::builtin_speed_ramps()
                .into_iter()
                .find(|r| r.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("no such speed ramp '{name}'"))?;
            let mut n = 0;
            for &id in clip_ids {
                if let Some(c) = app.project.clip_mut(id) {
                    crate::engine::presets::apply_curve(&preset, &mut c.speed_curve, c.duration, false);
                    n += 1;
                }
            }
            Ok(json!({"ok": true, "count": n}))
        }
        GalleryTab::Transitions => {
            let kind = crate::model::TransitionKind::ALL
                .into_iter()
                .find(|k| k.name().eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("unknown transition '{name}'"))?;
            let dur = app.transitions_ui.duration;
            let n = crate::ui::transitions_ui::add_transitions(
                &mut app.project,
                clip_ids,
                &mut app.transitions_ui,
                kind,
                dur,
                false,
            );
            Ok(json!({"ok": true, "count": n}))
        }
        GalleryTab::Templates => Err("gallery.apply doesn't place Templates — use templates.apply".into()),
    }
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "gallery.list",
        desc: "Card names in one Gallery tab (Looks|Luts|Captions|SpeedRamps|Transitions|Templates), or every tab when omitted. Looks excludes saved node-graph presets and reuses color-engine's builtin_looks() — the sole tool surface for Looks.",
        args: &["tab:string:false:omit=all tabs"],
        kind: ToolKind::Read,
        run: |app, args| {
            let names = |t: GalleryTab| crate::ui::gallery::card_names(t, &app.settings);
            match Args(args).str("tab") {
                Some(s) => {
                    let tab = GalleryTab::from_name(s).ok_or_else(|| format!("unknown tab '{s}'"))?;
                    done(json!({tab.name(): names(tab)}))
                }
                None => {
                    let mut out = serde_json::Map::new();
                    for t in GalleryTab::ALL {
                        out.insert(t.name().to_string(), json!(names(t)));
                    }
                    done(Value::Object(out))
                }
            }
        },
    },
    ToolDef {
        name: "gallery.apply",
        desc: "Apply a Gallery card (Look/LUT/Caption/SpeedRamp/Transition) to clip_ids (default selection); Captions is project-wide. Templates place at the playhead instead — use templates.apply.",
        args: &[
            "tab:string:true:",
            "name:string:true:",
            "clip_ids:array:false:defaults to selection",
            "intensity:number:false:Looks/Luts only, 0..1 default 1.0",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let tab_s = req(arg_str(args, "tab"), "tab")?;
            let tab = GalleryTab::from_name(tab_s).ok_or_else(|| format!("unknown tab '{tab_s}'"))?;
            let name = req(arg_str(args, "name"), "name")?.to_string();
            let ids = Args(args).ids_or_selection("clip_ids", app);
            let intensity = arg_f64(args, "intensity").unwrap_or(1.0).clamp(0.0, 1.0) as f32;
            apply_card(app, tab, &name, &ids, intensity).map(ToolOutcome::Done)
        },
    },
    ToolDef {
        name: "gallery.hover",
        desc: "Set/clear the monitor's alt-render preview to a Gallery card (name:null clears). UI-only, no mutation, no undo.",
        args: &["tab:string:true:", "name:string:false:null clears"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let tab_s = req(arg_str(args, "tab"), "tab")?;
            let want = match arg_str(args, "name") {
                Some(n) => Some(monitor::AltRequest::Gallery(tab_s.to_string(), n.to_string())),
                None => None,
            };
            app.alt_render.request(want);
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "clip.reorder_effect",
        desc: "Move an effect to a new stack index (Project::reorder_effect).",
        args: &["clip_id:integer:true:", "from:integer:true:", "to:integer:true:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = req(arg_u64(args, "clip_id"), "clip_id")?;
            let from = req(arg_u64(args, "from"), "from")? as usize;
            let to = req(arg_u64(args, "to"), "to")? as usize;
            if !app.project.reorder_effect(id, from, to) {
                return Err("no such clip, or from/to out of range".into());
            }
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "clip.effects_bulk",
        desc: "Set one stack-index effect's params on every listed clip whose effect at that index shares its kind (Project::bulk_set_effect_params).",
        args: &["clip_ids:array:true:", "index:integer:true:", "params:object:true:name->value"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let ids = req(arg_ids(args, "clip_ids"), "clip_ids")?;
            let index = req(arg_u64(args, "index"), "index")? as usize;
            let obj = req(args.get("params"), "params")?.as_object().ok_or("params: object")?;
            let mut params = std::collections::HashMap::new();
            for (k, v) in obj {
                params.insert(k.clone(), v.as_f64().ok_or_else(|| format!("params.{k}: number"))?);
            }
            let n = app.project.bulk_set_effect_params(&ids, index, &params);
            done(json!({"ok": true, "count": n}))
        },
    },
    ToolDef {
        name: "subtitles.style_preset",
        desc: "Set project.subtitle_style from a named caption style (builtin or Settings.caption_presets).",
        args: &["name:string:true:builtin or Settings.caption_presets"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?;
            let style = crate::engine::presets::builtin_caption_styles()
                .into_iter()
                .chain(app.settings.caption_presets.iter().cloned())
                .find(|s| s.text.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("no such caption style '{name}'"))?;
            app.project.subtitle_style = style;
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "inspector.folds",
        desc: "Set one inspector section's remembered open/closed state (Settings.inspector_folds). No undo — Settings-level, like other UI prefs.",
        args: &["section:string:true:", "open:boolean:true:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let section = req(arg_str(args, "section"), "section")?.to_string();
            let open = req(arg_bool(args, "open"), "open")?;
            app.settings.inspector_folds.insert(section, open);
            app.settings.save();
            done(json!({"ok": true}))
        },
    },
];

// deviation (see PR body): no unit tests in this file — every `run` closure and `apply_card` need
// `&mut App`, and (per tools_registry_tests.rs's/monitor.rs's own "deviation" doc comments) this crate
// has no headless `App`-construction path anywhere. Coverage comes from the crate-wide structural tests
// (tool_names_unique_and_namespaced, every_arg_spec_parses, tool_names_are_sole_registration,
// mutate_rows_roll_back_on_error, server_end_to_end) plus `Project::reorder_effect`/
// `bulk_set_effect_params`'s own unit tests in `src/model/ops/effects.rs`, which is the pure logic these
// two tools call directly.
