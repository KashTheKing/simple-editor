//! ---- ws:inspector-gallery ----
//! 7 MCP tools for capabilities this workstream actually adds: gallery.list/apply/hover (the Gallery
//! pane's tool surface — the canonical resolution for Looks, via `find_look`/`apply_look_by_name` below;
//! color-engine's `looks.list`/`looks.apply` cover the same ground for a bare clip_id/name call, and are
//! now thin wrappers around these same two fns — see tools_color.rs — so a Look name resolves/applies
//! identically no matter which tool name is called, rather than this file re-registering those names),
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

/// Resolve a Look by name across BOTH `builtin_looks()` and non-graph `Settings.effect_presets` — the
/// single source of truth `gallery.apply(tab=Looks)` and color-engine's `looks.apply` (tools_color.rs, a
/// thin wrapper around this + `apply_look_by_name` below) both go through, so the same Look name
/// succeeds or fails identically no matter which tool name is called. Takes `&Settings` rather than
/// `&App` so it (and `apply_look_by_name`) stay unit-testable without a live App (see this file's own
/// "deviation" doc comment).
pub(super) fn find_look(settings: &Settings, name: &str) -> Option<crate::settings::EffectPreset> {
    crate::engine::presets::builtin_looks()
        .into_iter()
        .chain(settings.effect_presets.iter().filter(|p| !p.is_graph()).cloned())
        .find(|p| p.name.eq_ignore_ascii_case(name))
}

/// Apply a Look (resolved via `find_look`) to every clip in `clip_ids` at `intensity` — the pure logic
/// shared by `gallery.apply(tab=Looks)` and color-engine's `looks.apply`.
pub(super) fn apply_look_by_name(
    project: &mut Project,
    settings: &Settings,
    name: &str,
    clip_ids: &[Id],
    intensity: f32,
) -> Result<Value, String> {
    let preset = find_look(settings, name).ok_or_else(|| format!("no such Look '{name}'"))?;
    let mut n = 0;
    for &id in clip_ids {
        if crate::engine::presets::apply_look(&preset, project, id, intensity) {
            n += 1;
        }
    }
    Ok(json!({"ok": true, "count": n}))
}

/// `gallery.apply`'s per-tab body: Looks/Luts/Captions/SpeedRamps target `clip_ids` (Captions is
/// project-wide, `clip_ids` ignored), Transitions adds at the cuts around `clip_ids`. Templates are NOT
/// handled here — `gallery.rs`'s own `GalleryResponse.place` routes those through the existing
/// `App::place_template`/`templates.apply`, since a template places POSITIONALLY, not per-clip.
fn apply_card(app: &mut App, tab: GalleryTab, name: &str, clip_ids: &[Id], intensity: f32) -> Result<Value, String> {
    match tab {
        GalleryTab::Looks => apply_look_by_name(&mut app.project, &app.settings, name, clip_ids, intensity),
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
        // ws:text-titles: Titles places positionally too (and needs the exposed-field zip) — use
        // titles.place, not gallery.apply.
        GalleryTab::Titles => Err("gallery.apply doesn't place Titles — use titles.place".into()),
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

// deviation (see PR body): no unit tests for the `run` closures / `apply_card` itself — every one needs
// `&mut App`, and (per tools_registry_tests.rs's/monitor.rs's own "deviation" doc comments) this crate
// has no headless `App`-construction path anywhere. Coverage comes from the crate-wide structural tests
// (tool_names_unique_and_namespaced, every_arg_spec_parses, tool_names_are_sole_registration,
// mutate_rows_roll_back_on_error, server_end_to_end) plus `Project::reorder_effect`/
// `bulk_set_effect_params`'s own unit tests in `src/model/ops/effects.rs`, which is the pure logic these
// two tools call directly. `find_look`/`apply_look_by_name` are the one exception — deliberately typed
// over `&Settings`/`&mut Project` instead of `&mut App` so the Looks resolution both `gallery.apply` and
// color-engine's `looks.apply` share (see tools_color.rs) gets real unit tests below.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipKind};

    /// A `Vec<Effect>` Look preset, same shape `engine::presets::capture_template` writes for a
    /// user-saved (non-graph) Look — `EffectPreset::is_graph()` is false for a JSON array.
    fn user_look(name: &str) -> crate::settings::EffectPreset {
        let fx = vec![Effect::new(EffectKind::Blur)];
        crate::settings::EffectPreset { name: name.to_string(), json: serde_json::to_string(&fx).unwrap() }
    }

    /// The bug this fixes: `looks.apply` used to only search `builtin_looks()`, so a Look saved from the
    /// Gallery pane (`Settings.effect_presets`) would apply via `gallery.apply(tab=Looks)` but fail via
    /// `looks.apply` for the exact same name. `find_look` is what both now resolve through.
    #[test]
    fn find_look_resolves_both_builtins_and_user_saved_presets() {
        let mut settings = Settings::default();
        let builtin_name = crate::engine::presets::builtin_looks()[0].name.clone();
        assert!(find_look(&settings, &builtin_name).is_some(), "a builtin Look must resolve");
        assert!(find_look(&settings, "My Custom Look").is_none(), "not saved yet");
        settings.effect_presets.push(user_look("My Custom Look"));
        assert!(
            find_look(&settings, "My Custom Look").is_some(),
            "a user-saved (non-graph) preset must resolve too — this is exactly what looks.apply used to miss"
        );
    }

    /// `apply_look_by_name` is the one function both `gallery.apply(tab=Looks)` (via `apply_card`) and
    /// color-engine's `looks.apply` call — so a builtin Look name and a user-saved preset name must both
    /// apply successfully through it, producing identical results regardless of which tool name reaches it.
    #[test]
    fn apply_look_by_name_succeeds_for_both_builtin_and_user_saved_names() {
        let mut p = Project::new();
        let c = Clip::new(500, ClipKind::Video, "v", 0.0, 3.0);
        let id = c.id;
        p.tracks[0].clips.push(c);
        let mut settings = Settings::default();
        let builtin_name = crate::engine::presets::builtin_looks()[0].name.clone();
        settings.effect_presets.push(user_look("My Custom Look"));

        let r1 = apply_look_by_name(&mut p, &settings, &builtin_name, &[id], 1.0).unwrap();
        assert_eq!(r1, json!({"ok": true, "count": 1}), "a builtin Look applies");
        let r2 = apply_look_by_name(&mut p, &settings, "My Custom Look", &[id], 1.0).unwrap();
        assert_eq!(r2, json!({"ok": true, "count": 1}), "a user-saved Look now applies too, not just builtins");

        assert!(apply_look_by_name(&mut p, &settings, "no such look", &[id], 1.0).is_err());
    }
}
