//! ---- ws:color-engine ----
//! The 10 colour MCP tools (LUT, Primaries, Qualifier, auto/match, frame stats, Looks, master
//! effects, bypass) plus the 3 unbound Action handlers (AutoColor/ColorMatch/BypassGrade). Canonical
//! crate-wide registration point for clip.add_lut/color.auto/color.match/looks.list/looks.apply - a
//! later workstream (inspector-gallery, source-monitor, pro-monitor) that needs LUT/looks/colour-match/
//! colour-auto capability calls these fns/tools directly instead of declaring gallery.*/timeline.*
//! duplicates (see plans/ui-overhaul/issues/color-engine.md's ownership notes).

use super::tools_args::Args;
use super::tools_helpers::*;
use super::*;
use crate::engine::{effects, gpu::FrameStats, lut};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};

/// Every colour-grade-category `EffectKind` `clip.bypass`/`Action::BypassGrade` toggle.
fn is_grade_kind(k: EffectKind) -> bool {
    matches!(
        k,
        EffectKind::Color
            | EffectKind::Primaries
            | EffectKind::Curves
            | EffectKind::Levels
            | EffectKind::HueShift
            | EffectKind::Qualifier
            | EffectKind::Lut
    )
}

/// Toggle `Effect.enabled` on every grade-category effect in `effects` - shared by `clip.bypass` and
/// `Action::BypassGrade`. `on`: `Some` forces that state, `None` flips each independently.
/// ponytail: reuses the existing `enabled` field instead of a separate non-destructive preview-bypass
/// flag/render path - pro-monitor's later wipe/compare window can add a preview-only bypass without
/// touching this.
fn bypass_grade_effects(fx: &mut [Effect], on: Option<bool>) {
    for e in fx.iter_mut().filter(|e| is_grade_kind(e.kind)) {
        e.enabled = on.unwrap_or(!e.enabled);
    }
}

/// Find-or-create the clip's effect of `kind`, for tools that set individual fields on it
/// (`color.primaries`/`color.qualifier`) rather than replacing the whole effect.
fn upsert_effect(c: &mut Clip, kind: EffectKind) -> &mut Effect {
    if let Some(i) = c.effects.iter().position(|e| e.kind == kind) {
        &mut c.effects[i]
    } else {
        c.effects.push(Effect::new(kind));
        c.effects.last_mut().unwrap()
    }
}

fn set_param(e: &mut Effect, i: usize, v: Option<f64>) {
    if let Some(v) = v {
        e.params[i].value = v;
    }
}

/// Replace the clip's effect of `new.kind` wholesale (or append if it has none yet) - `color.auto`/
/// `color.match`/`Action::AutoColor`/`Action::ColorMatch` write a fresh Levels/Color/Curves this way so
/// a repeat call updates in place instead of accumulating duplicates.
fn upsert_whole(c: &mut Clip, new: Effect) {
    match c.effects.iter().position(|e| e.kind == new.kind) {
        Some(i) => c.effects[i] = new,
        None => c.effects.push(new),
    }
}

/// Render at `t` through the live-preview path (`render_preview_texture` - never `render_frame`, which
/// export/thumbnails also call) and read back `gpu.stats()`. None when there's no GPU renderer, decode
/// timed out, or the render panicked.
fn stats_at(app: &mut App, t: f64) -> Option<FrameStats> {
    let w = app.project.width.max(16);
    let h = app.project.height.max(16);
    let layers = app.player.layers_once(t, w)?;
    let App { gpu, project, .. } = app;
    let gpu = gpu.as_mut()?;
    gpu.set_stats_wanted(true);
    guarded(|| gpu.render_preview_texture(project, t, w, h, &layers));
    gpu.stats().cloned()
}

fn stats_json(s: &FrameStats) -> Value {
    // ponytail: the raw 256x144 downsample (`FrameStats.sample`) stays in-process only, for a future
    // eyedropper/scopes consumer (canvas-handles-monitor/pro-monitor) - putting ~37k RGBA samples in a
    // JSON-RPC reply would bloat every frame.stats call for no MCP/Luau caller that exists yet.
    json!({
        "p1": s.p1, "p99": s.p99, "mean": s.mean, "sample_w": s.sample_w, "sample_h": s.sample_h,
        "hist": [s.hist[0].to_vec(), s.hist[1].to_vec(), s.hist[2].to_vec()], "luma": s.luma.to_vec(),
    })
}

/// Parse `{kind, params}` (same shape `clip.add_effect`'s single-effect parser uses) into an `Effect` -
/// shared by `media.set_effects`' array of them.
fn parse_effect(item: &Value) -> Result<Effect, String> {
    let kind_s = item.get("kind").and_then(|v| v.as_str()).ok_or("each effect needs a 'kind'")?;
    let kind = EffectKind::ALL
        .into_iter()
        .find(|k| k.name().eq_ignore_ascii_case(kind_s) || format!("{k:?}").eq_ignore_ascii_case(kind_s))
        .ok_or_else(|| format!("unknown effect '{kind_s}'"))?;
    let mut effect = Effect::new(kind);
    if let Some(params) = item.get("params").and_then(|v| v.as_object()) {
        for (pname, pval) in params {
            let i = kind
                .params()
                .iter()
                .position(|s| s.name.eq_ignore_ascii_case(pname))
                .ok_or_else(|| format!("unknown param '{pname}' for {}", kind.name()))?;
            effect.params[i].value = pval.as_f64().ok_or("param values must be numbers")?;
        }
    }
    Ok(effect)
}

pub(super) fn dispatch(app: &mut App, name: &str, args: &Value) -> Option<Result<Value, String>> {
    let prefix = name.split('.').next().unwrap_or("");
    if !matches!(
        (prefix, name),
        ("clip", "clip.add_lut" | "clip.bypass")
            | ("color", _)
            | ("frame", _)
            | ("looks", _)
            | ("media", "media.set_effects")
    ) {
        return None;
    }
    Some(run(app, name, args))
}

fn run(app: &mut App, name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "clip.add_lut" => {
            let id = req(arg_u64(args, "clip_id"), "clip_id")?;
            let path = req(arg_str(args, "path"), "path")?.to_string();
            let intensity = arg_f64(args, "intensity").unwrap_or(1.0).clamp(0.0, 1.0);
            lut::load(&path).map_err(|e| format!("bad LUT: {e}"))?;
            let c = app.project.clip_mut(id).ok_or("no such clip")?;
            let mut e = Effect::new(EffectKind::Lut);
            e.lut = path;
            e.params[0].value = intensity;
            c.effects.push(e);
            Ok(json!({"ok": true, "index": c.effects.len() - 1}))
        }
        "color.primaries" => {
            let id = req(arg_u64(args, "clip_id"), "clip_id")?;
            let c = app.project.clip_mut(id).ok_or("no such clip")?;
            let e = upsert_effect(c, EffectKind::Primaries);
            for (i, field) in [
                "lift_r", "lift_g", "lift_b", "gamma_r", "gamma_g", "gamma_b", "gain_r", "gain_g", "gain_b", "temp",
                "tint",
            ]
            .into_iter()
            .enumerate()
            {
                set_param(e, i, arg_f64(args, field));
            }
            Ok(json!({"ok": true}))
        }
        "color.qualifier" => {
            let id = req(arg_u64(args, "clip_id"), "clip_id")?;
            let c = app.project.clip_mut(id).ok_or("no such clip")?;
            let e = upsert_effect(c, EffectKind::Qualifier);
            for (i, field) in
                ["hue", "hue_width", "sat_min", "sat_max", "lum_min", "lum_max", "softness"].into_iter().enumerate()
            {
                set_param(e, i, arg_f64(args, field));
            }
            Ok(json!({"ok": true}))
        }
        "color.auto" => {
            let id = req(arg_u64(args, "clip_id"), "clip_id")?;
            app.project.clip(id).ok_or("no such clip")?;
            let stats = stats_at(app, app.playhead).ok_or("GPU rendering not available")?;
            let (levels, color) = effects::auto_color(&stats);
            let c = app.project.clip_mut(id).ok_or("no such clip")?;
            upsert_whole(c, levels);
            upsert_whole(c, color);
            Ok(json!({"ok": true}))
        }
        "color.match" => {
            let id = req(arg_u64(args, "clip_id"), "clip_id")?;
            let rid = req(arg_u64(args, "reference_clip_id"), "reference_clip_id")?;
            // both frames rendered at each clip's own timeline start - a shared time would sample the
            // same composited frame for both and compare it to itself.
            let t_src = app.project.clip(id).ok_or("no such clip")?.start;
            let t_ref = app.project.clip(rid).ok_or("no such reference clip")?.start;
            let src = stats_at(app, t_src).ok_or("GPU rendering not available")?;
            let dst = stats_at(app, t_ref).ok_or("GPU rendering not available")?;
            let curve = effects::match_curves(&src, &dst);
            let c = app.project.clip_mut(id).ok_or("no such clip")?;
            upsert_whole(c, curve);
            Ok(json!({"ok": true}))
        }
        "frame.stats" => {
            let t = Args(args).t_or_playhead("t", app);
            Ok(stats_at(app, t).as_ref().map(stats_json).unwrap_or_else(|| json!({})))
        }
        // ---- ws:inspector-gallery ----
        // Thin wrappers around tools_gallery's `find_look`/`apply_look_by_name` - the same underlying
        // logic `gallery.list`/`gallery.apply(tab=Looks)` use - so a Look name (builtin OR user-saved)
        // resolves and applies identically no matter which tool name is called. `looks.apply` used to
        // only know builtins, so the same name could succeed via `gallery.apply` and fail here.
        "looks.list" => Ok(json!(crate::ui::gallery::card_names(crate::ui::gallery::GalleryTab::Looks, &app.settings))),
        "looks.apply" => {
            let id = req(arg_u64(args, "clip_id"), "clip_id")?;
            let name = req(arg_str(args, "name"), "name")?;
            let intensity = arg_f64(args, "intensity").unwrap_or(1.0) as f32;
            super::tools_gallery::apply_look_by_name(&mut app.project, &app.settings, name, &[id], intensity)
        }
        "media.set_effects" => {
            let id = req(arg_u64(args, "asset_id"), "asset_id")?;
            let arr = req(args.get("effects"), "effects")?.as_array().ok_or("effects: array")?;
            let fx = arr.iter().map(parse_effect).collect::<Result<Vec<_>, _>>()?;
            let count = fx.len();
            let a = app.project.asset_mut(id).ok_or("no such asset")?;
            a.effects = fx;
            Ok(json!({"ok": true, "count": count}))
        }
        "clip.bypass" => {
            let ids = Args(args).ids_or_selection("clip_ids", app);
            let on = arg_bool(args, "on");
            let mut n = 0;
            for id in &ids {
                if let Some(c) = app.project.clip_mut(*id) {
                    bypass_grade_effects(&mut c.effects, on);
                    n += 1;
                }
            }
            Ok(json!({"ok": true, "count": n}))
        }
        _ => unreachable!(),
    }
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::AutoColor => {
            let ids = app.selection.clone();
            if ids.is_empty() {
                app.toast("Select a clip first");
                return true;
            }
            let Some(stats) = stats_at(app, app.playhead) else {
                app.toast("GPU rendering not available");
                return true;
            };
            let (levels, color) = effects::auto_color(&stats);
            app.push_undo();
            for id in ids {
                if let Some(c) = app.project.clip_mut(id) {
                    upsert_whole(c, levels.clone());
                    upsert_whole(c, color.clone());
                }
            }
            app.after_edit();
            true
        }
        Action::ColorMatch => {
            let ids = app.selection.clone();
            if ids.len() < 2 {
                app.toast("Select at least 2 clips (the last is the reference)");
                return true;
            }
            // last-selected is the reference - ponytail-simple, since selection has no order today.
            let (targets, reference) = ids.split_at(ids.len() - 1);
            let reference = reference[0];
            let Some(t_ref) = app.project.clip(reference).map(|c| c.start) else { return true };
            let Some(dst) = stats_at(app, t_ref) else {
                app.toast("GPU rendering not available");
                return true;
            };
            let mut edits = Vec::new();
            for &id in targets {
                let Some(t_src) = app.project.clip(id).map(|c| c.start) else { continue };
                let Some(src) = stats_at(app, t_src) else { continue };
                edits.push((id, effects::match_curves(&src, &dst)));
            }
            if edits.is_empty() {
                app.toast("GPU rendering not available");
                return true;
            }
            app.push_undo();
            for (id, curve) in edits {
                if let Some(c) = app.project.clip_mut(id) {
                    upsert_whole(c, curve);
                }
            }
            app.after_edit();
            true
        }
        Action::BypassGrade => {
            let ids = app.selection.clone();
            if ids.is_empty() {
                app.toast("Select a clip first");
                return true;
            }
            app.push_undo();
            for id in ids {
                if let Some(c) = app.project.clip_mut(id) {
                    bypass_grade_effects(&mut c.effects, None);
                }
            }
            app.after_edit();
            true
        }
        _ => false,
    }
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "clip.add_lut",
        desc: "Add an EffectKind::Lut effect (a .cube 3D LUT) to a clip; parses the file to validate before pushing.",
        args: &["clip_id:integer:true:", "path:string:true:.cube file path", "intensity:number:false:0..1 default 1.0"],
        kind: ToolKind::Mutate,
        run: |a, v| dispatch(a, "clip.add_lut", v).unwrap().map(ToolOutcome::Done),
    },
    ToolDef {
        name: "color.primaries",
        desc: "Find-or-create the clip's Primaries (lift/gamma/gain colour wheels + temp/tint) effect and set given fields (unset fields keep their current value).",
        args: &[
            "clip_id:integer:true:",
            "lift_r:number:false:",
            "lift_g:number:false:",
            "lift_b:number:false:",
            "gamma_r:number:false:",
            "gamma_g:number:false:",
            "gamma_b:number:false:",
            "gain_r:number:false:",
            "gain_g:number:false:",
            "gain_b:number:false:",
            "temp:number:false:",
            "tint:number:false:",
        ],
        kind: ToolKind::Mutate,
        run: |a, v| dispatch(a, "color.primaries", v).unwrap().map(ToolOutcome::Done),
    },
    ToolDef {
        name: "color.qualifier",
        desc: "Find-or-create the clip's Qualifier (HSL-band secondary key) effect and set given fields.",
        args: &[
            "clip_id:integer:true:",
            "hue:number:false:",
            "hue_width:number:false:",
            "sat_min:number:false:",
            "sat_max:number:false:",
            "lum_min:number:false:",
            "lum_max:number:false:",
            "softness:number:false:",
        ],
        kind: ToolKind::Mutate,
        run: |a, v| dispatch(a, "color.qualifier", v).unwrap().map(ToolOutcome::Done),
    },
    ToolDef {
        name: "color.auto",
        desc: "Sample the frame at the playhead (via the live preview render path) and write/update an editable Levels+Color pair on the clip.",
        args: &["clip_id:integer:true:"],
        kind: ToolKind::Mutate,
        run: |a, v| dispatch(a, "color.auto", v).unwrap().map(ToolOutcome::Done),
    },
    ToolDef {
        name: "color.match",
        desc: "Match a clip's histogram to a reference clip's (both rendered at their own timeline start via the live preview path) as an editable Curves effect.",
        args: &["clip_id:integer:true:", "reference_clip_id:integer:true:"],
        kind: ToolKind::Mutate,
        run: |a, v| dispatch(a, "color.match", v).unwrap().map(ToolOutcome::Done),
    },
    ToolDef {
        name: "frame.stats",
        desc: "Histogram/percentile/mean of the frame at t (256x144 downsample), sourced from the live preview render path; {} when the GPU or the requested frame is unavailable.",
        args: &["t:number:false:defaults to playhead"],
        kind: ToolKind::Read,
        run: |a, v| dispatch(a, "frame.stats", v).unwrap().map(ToolOutcome::Done),
    },
    ToolDef {
        name: "looks.list",
        desc: "Names of every Look: the 12 built-ins plus any user-saved (non-graph) Settings.effect_presets - same list gallery.list(tab=Looks) returns.",
        args: &[],
        kind: ToolKind::Read,
        run: |a, v| dispatch(a, "looks.list", v).unwrap().map(ToolOutcome::Done),
    },
    ToolDef {
        name: "looks.apply",
        desc: "Apply a Look (built-in or user-saved) to a clip, replacing its effect stack, scaled by intensity (0 = no-op, 1 = the Look unmodified). Same resolution/result as gallery.apply(tab=Looks) for the same name.",
        args: &["clip_id:integer:true:", "name:string:true:", "intensity:number:false:default 1.0"],
        kind: ToolKind::Mutate,
        run: |a, v| dispatch(a, "looks.apply", v).unwrap().map(ToolOutcome::Done),
    },
    ToolDef {
        name: "media.set_effects",
        desc: "Replace an Asset's master effects (applied to every clip using it, prepended before clip-local effects). Effects: [{kind:string,params:object}], same shape as clip.add_effect.",
        args: &["asset_id:integer:true:", "effects:array:true:[{kind:string,params:object}]"],
        kind: ToolKind::Mutate,
        run: |a, v| dispatch(a, "media.set_effects", v).unwrap().map(ToolOutcome::Done),
    },
    ToolDef {
        name: "clip.bypass",
        desc: "Enable/disable every colour-grade effect (Color/Primaries/Curves/Levels/HueShift/Qualifier/Lut) on the given clips in one step.",
        args: &["clip_ids:array:false:defaults to selection", "on:boolean:false:toggles when omitted"],
        kind: ToolKind::Mutate,
        run: |a, v| dispatch(a, "clip.bypass", v).unwrap().map(ToolOutcome::Done),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn eff(kind: EffectKind) -> Effect {
        Effect::new(kind)
    }

    /// The pure half of `clip.bypass`/`Action::BypassGrade`: a clip with [Blur, Color, Curves] effects
    /// leaves Blur.enabled untouched and flips Color/Curves.enabled.
    #[test]
    fn clip_bypass_toggles_only_grade_effects() {
        let mut fx = vec![eff(EffectKind::Blur), eff(EffectKind::Color), eff(EffectKind::Curves)];
        assert!(fx.iter().all(|e| e.enabled));
        bypass_grade_effects(&mut fx, None);
        assert!(fx[0].enabled, "Blur must stay untouched");
        assert!(!fx[1].enabled);
        assert!(!fx[2].enabled);
        // toggling again flips back
        bypass_grade_effects(&mut fx, None);
        assert!(fx[1].enabled && fx[2].enabled);
        // an explicit `on` forces the state instead of toggling
        bypass_grade_effects(&mut fx, Some(false));
        assert!(!fx[1].enabled && !fx[2].enabled);
        bypass_grade_effects(&mut fx, Some(false)); // idempotent
        assert!(!fx[1].enabled && !fx[2].enabled);
    }

    #[test]
    fn upsert_effect_finds_or_creates() {
        let mut c = Clip::new(1, ClipKind::Video, "c", 0.0, 2.0);
        assert!(c.effects.is_empty());
        upsert_effect(&mut c, EffectKind::Primaries).params[0].value = 0.5;
        assert_eq!(c.effects.len(), 1);
        // a second call on the same kind edits the existing effect, does not add another
        upsert_effect(&mut c, EffectKind::Primaries).params[1].value = 0.25;
        assert_eq!(c.effects.len(), 1);
        assert_eq!(c.effects[0].params[0].value, 0.5);
        assert_eq!(c.effects[0].params[1].value, 0.25);
    }

    #[test]
    fn upsert_whole_replaces_same_kind_only() {
        let mut c = Clip::new(1, ClipKind::Video, "c", 0.0, 2.0);
        c.effects.push(eff(EffectKind::Blur));
        let mut levels = eff(EffectKind::Levels);
        levels.params[0].value = 0.1;
        upsert_whole(&mut c, levels);
        assert_eq!(c.effects.len(), 2, "Blur stays, Levels is appended");
        let mut levels2 = eff(EffectKind::Levels);
        levels2.params[0].value = 0.2;
        upsert_whole(&mut c, levels2);
        assert_eq!(c.effects.len(), 2, "a second Levels replaces the first, not appended");
        assert_eq!(c.effects[1].params[0].value, 0.2);
    }
}
