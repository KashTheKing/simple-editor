//! Keyframe presets, motion presets and clip templates - capture from clips and apply back.
//! Curve presets store keys normalised to 0..1 of the clip length (stretched to the target clip's
//! duration when applied) or as absolute seconds (kept as saved).

use crate::model::{
    Animated, Asset, Clip, ClipKind, Ease, Effect, EffectKind, Id, Keyframe, NodeGraph, Project, TextStyle, MIN_CLIP,
};
use crate::settings::{CurvePreset, EffectPreset, MotionPreset, Template};
use std::collections::HashMap;

/// Snapshot a property's keys as a preset (normalised unless `absolute`). None if the property has no keys.
pub fn capture_curve(name: &str, anim: &Animated, clip_duration: f64, absolute: bool) -> Option<CurvePreset> {
    if anim.keys.is_empty() {
        return None;
    }
    let d = clip_duration.max(MIN_CLIP);
    let keys = anim.keys.iter().map(|k| Keyframe { t: if absolute { k.t } else { k.t / d }, ..*k }).collect();
    Some(CurvePreset { name: name.into(), keys, absolute })
}

/// Apply a curve preset to a property: normalised times are stretched to `clip_duration`, absolute
/// presets keep their seconds. Replaces existing keys.
/// `scaled` is kept for the callers' "exact" button but is only meaningful for absolute presets, which
/// ignore the duration anyway - placing a normalised preset's 0..1 times as seconds would crush the
/// whole animation into the clip's first second. See `capture_curve`: nothing but tests saves absolute.
pub fn apply_curve(preset: &CurvePreset, anim: &mut Animated, clip_duration: f64, _scaled: bool) {
    anim.keys = scaled_keys(&preset.keys, preset.absolute, clip_duration);
}

/// `apply_curve`'s sibling: ADD the preset's keys to what the property already has, shifted to start at
/// `offset` seconds, so two presets can be layered on one property without hand-editing. `span` is the
/// length a normalised preset is stretched over (callers pass the clip's remaining time so the merged
/// keys land inside it). A preset key falling on an existing key's time wins.
pub fn merge_curve(preset: &CurvePreset, anim: &mut Animated, span: f64, offset: f64) {
    for k in scaled_keys(&preset.keys, preset.absolute, span) {
        let t = k.t + offset;
        let k = Keyframe { t, ..k };
        match anim.key_index_at(t) {
            Some(i) => anim.keys[i] = k,
            None => {
                let i = anim.keys.partition_point(|o| o.t < t);
                anim.keys.insert(i, k);
            }
        }
    }
}

/// Convenience: keyframes of a preset scaled to `duration` (used by the curve editor preview).
pub fn scaled_keys(keys: &[Keyframe], absolute: bool, duration: f64) -> Vec<Keyframe> {
    keys.iter().map(|k| Keyframe { t: if absolute { k.t } else { k.t * duration }, ..*k }).collect()
}

fn motion(name: &str, props: &[(&str, &[(f64, f64, Ease)])]) -> MotionPreset {
    MotionPreset {
        name: name.into(),
        props: props
            .iter()
            .map(|&(p, keys)| {
                let keys = keys.iter().map(|&(t, v, ease)| Keyframe { t, v, ease }).collect();
                (p.to_string(), CurvePreset { name: p.into(), keys, absolute: false })
            })
            .collect(),
    }
}

/// Built-in motion presets (slide in/out from each side, zoom in/out "Ken Burns", pop, fade in/out, spin).
pub fn builtin_motions() -> Vec<MotionPreset> {
    // ponytail: slide offsets assume a ~1080p project - a wider clip just starts a bit on-screen.
    const W: f64 = 1920.0;
    const H: f64 = 1080.0;
    let smooth = Ease::PRESETS[0].1;
    let overshoot = Ease::PRESETS[4].1;
    let lin = Ease::Linear;
    vec![
        motion("Slide In Left", &[("Position X", &[(0.0, -W, smooth), (0.25, 0.0, lin)])]),
        motion("Slide In Right", &[("Position X", &[(0.0, W, smooth), (0.25, 0.0, lin)])]),
        motion("Slide In Top", &[("Position Y", &[(0.0, -H, smooth), (0.25, 0.0, lin)])]),
        motion("Slide In Bottom", &[("Position Y", &[(0.0, H, smooth), (0.25, 0.0, lin)])]),
        motion("Slide Out Left", &[("Position X", &[(0.75, 0.0, smooth), (1.0, -W, lin)])]),
        motion("Slide Out Right", &[("Position X", &[(0.75, 0.0, smooth), (1.0, W, lin)])]),
        motion("Slide Out Top", &[("Position Y", &[(0.75, 0.0, smooth), (1.0, -H, lin)])]),
        motion("Slide Out Bottom", &[("Position Y", &[(0.75, 0.0, smooth), (1.0, H, lin)])]),
        motion("Zoom In (Ken Burns)", &[("Scale", &[(0.0, 1.0, lin), (1.0, 1.15, lin)])]),
        motion("Zoom Out (Ken Burns)", &[("Scale", &[(0.0, 1.15, lin), (1.0, 1.0, lin)])]),
        motion("Pop", &[("Scale", &[(0.0, 0.0, overshoot), (0.25, 1.0, lin)])]),
        motion("Fade In", &[("Opacity", &[(0.0, 0.0, smooth), (0.25, 1.0, lin)])]),
        motion("Fade Out", &[("Opacity", &[(0.75, 1.0, smooth), (1.0, 0.0, lin)])]),
        motion("Spin", &[("Rotation", &[(0.0, 0.0, smooth), (1.0, 360.0, lin)])]),
    ]
}

/// The fixed (label, property) pairs shared by capture and apply.
const PROPS: [&str; 7] = ["Position X", "Position Y", "Scale", "Rotation", "Opacity", "Volume", "Pan"];

fn prop_of<'a>(clip: &'a mut Clip, label: &str) -> Option<&'a mut Animated> {
    match label {
        "Position X" => Some(&mut clip.x),
        "Position Y" => Some(&mut clip.y),
        "Scale" => Some(&mut clip.scale),
        "Rotation" => Some(&mut clip.rotation),
        "Opacity" => Some(&mut clip.opacity),
        "Volume" => Some(&mut clip.volume),
        "Pan" => Some(&mut clip.pan),
        _ => None,
    }
}

/// Capture every animated property of `clip` (incl. effect params as "<Effect>: <Param>").
pub fn capture_motion(name: &str, clip: &Clip) -> MotionPreset {
    let mut props = Vec::new();
    let fixed = [&clip.x, &clip.y, &clip.scale, &clip.rotation, &clip.opacity, &clip.volume, &clip.pan];
    for (label, anim) in PROPS.iter().zip(fixed) {
        if let Some(c) = capture_curve(label, anim, clip.duration, false) {
            props.push((label.to_string(), c));
        }
    }
    // ponytail: duplicate effects of the same kind share a label - the first instance wins on apply.
    for e in &clip.effects {
        for (i, spec) in e.specs().iter().enumerate() {
            let Some(anim) = e.params.get(i) else { continue };
            let label = format!("{}: {}", e.kind.name(), spec.name);
            if let Some(c) = capture_curve(&label, anim, clip.duration, false) {
                props.push((label, c));
            }
        }
    }
    MotionPreset { name: name.into(), props }
}

/// Resolve a motion preset label to the clip's property, creating the effect an "<Effect>: <Param>"
/// label needs. None for labels this clip cannot hold.
fn motion_prop<'a>(clip: &'a mut Clip, label: &str) -> Option<&'a mut Animated> {
    if prop_of(clip, label).is_some() {
        return prop_of(clip, label);
    }
    let (ename, pname) = label.split_once(": ")?;
    let kind = EffectKind::ALL.into_iter().find(|k| k.name() == ename)?;
    let pi = kind.params().iter().position(|p| p.name == pname)?;
    if !clip.effects.iter().any(|e| e.kind == kind) {
        clip.effects.push(Effect::new(kind));
    }
    clip.effects.iter_mut().find(|e| e.kind == kind).and_then(|e| e.params.get_mut(pi))
}

/// Apply a motion preset to a clip (properties it doesn't have are skipped; effect params create the
/// effect when missing). `scaled` as in `apply_curve`.
pub fn apply_motion(preset: &MotionPreset, clip: &mut Clip, scaled: bool) {
    let dur = clip.duration;
    for (label, curve) in &preset.props {
        if let Some(anim) = motion_prop(clip, label) {
            apply_curve(curve, anim, dur, scaled);
        }
    }
}

/// Layer a motion preset on top of the clip's existing keys, starting at clip-local `offset` and
/// stretched over the time left in the clip (so "slide in left" then "slide in right" can be stacked).
pub fn merge_motion(preset: &MotionPreset, clip: &mut Clip, offset: f64) {
    let span = (clip.duration - offset).max(MIN_CLIP);
    for (label, curve) in &preset.props {
        if let Some(anim) = motion_prop(clip, label) {
            merge_curve(curve, anim, span, offset);
        }
    }
}

/// Snapshot a clip's node graph if it has one, else its effect stack. The JSON's shape is what tells
/// the two apart afterwards (`EffectPreset::is_graph`) - nothing else has to be stored.
pub fn capture_effects(name: &str, clip: &Clip) -> EffectPreset {
    let json = match clip.graph.as_ref().filter(|_| clip.uses_graph()) {
        Some(g) => serde_json::to_string(g),
        None => serde_json::to_string(&clip.effects),
    };
    EffectPreset { name: name.into(), json: json.unwrap_or_default() }
}

/// Apply a preset to `clip`: a graph replaces the clip's graph (with fresh node ids, so a later
/// `add_node` cannot hand out one the preset already used), a stack replaces the effects *and* drops
/// the graph, which would otherwise keep shadowing them. False when the JSON is not what it claims.
pub fn apply_effects(preset: &EffectPreset, project: &mut Project, clip: Id) -> bool {
    if preset.is_graph() {
        let Ok(mut g) = serde_json::from_str::<NodeGraph>(&preset.json) else { return false };
        let map: HashMap<Id, Id> = g.nodes.iter().map(|n| (n.id, project.new_id())).collect();
        g.edges.retain(|e| map.contains_key(&e.from) && map.contains_key(&e.to));
        for n in &mut g.nodes {
            n.id = map[&n.id];
        }
        for e in &mut g.edges {
            e.from = map[&e.from];
            e.to = map[&e.to];
        }
        let Some(c) = project.clip_mut(clip) else { return false };
        c.graph = Some(g);
    } else {
        let Ok(fx) = serde_json::from_str::<Vec<Effect>>(&preset.json) else { return false };
        let Some(c) = project.clip_mut(clip) else { return false };
        c.effects = fx;
        c.graph = None;
    }
    true
}

// ---- ws:color-engine ----
/// One built-in Look: a name plus a small effect recipe (kind, tweaked param values by name).
struct LookDef {
    name: &'static str,
    effects: &'static [(EffectKind, &'static [(&'static str, f64)])],
}

/// Build one effect of `kind` with the named params overridden (unknown names are ignored - a typo here
/// would otherwise silently no-op instead of failing loudly at the one call site that builds all 12).
fn look_effect(kind: EffectKind, params: &[(&str, f64)]) -> Effect {
    let mut e = Effect::new(kind);
    for &(name, v) in params {
        if let Some(i) = kind.params().iter().position(|p| p.name == name) {
            e.params[i].value = v;
        }
    }
    e
}

/// 12 built-in Looks, each a couple of tweaked `Primaries`/`Curves`/`Vignette` instances reusing the
/// existing `EffectPreset{name,json}` shape and `apply_effects()` verbatim (same machinery
/// `clip.apply_motion`/templates already use). First and sole declaration of `builtin_looks`/
/// `apply_look` in this file - later workstreams (inspector-gallery's Gallery ▸ Looks tab) reuse these
/// two fns unchanged rather than adding their own Looks-apply logic.
pub fn builtin_looks() -> Vec<EffectPreset> {
    const LOOKS: &[LookDef] = &[
        LookDef {
            name: "Teal-Orange",
            effects: &[
                (EffectKind::Primaries, &[("Lift B", 0.04), ("Gain R", 1.12), ("Gain B", 0.88), ("Temp", 18.0)]),
                (EffectKind::Vignette, &[("Strength", 0.35)]),
            ],
        },
        LookDef {
            name: "Vintage",
            effects: &[
                (EffectKind::Primaries, &[("Lift R", 0.05), ("Lift G", 0.03), ("Gain B", 0.85), ("Temp", 10.0)]),
                (EffectKind::Curves, &[("Master 1/4", 0.3), ("Master 3/4", 0.7)]),
            ],
        },
        LookDef { name: "Cool", effects: &[(EffectKind::Primaries, &[("Temp", -30.0), ("Gain B", 1.1)])] },
        LookDef { name: "Warm", effects: &[(EffectKind::Primaries, &[("Temp", 30.0), ("Gain R", 1.1)])] },
        LookDef {
            name: "B&W Film",
            effects: &[
                (EffectKind::Grayscale, &[("Amount", 1.0)]),
                (EffectKind::Curves, &[("Master 1/4", 0.2), ("Master 3/4", 0.82)]),
                (EffectKind::Vignette, &[("Strength", 0.3)]),
            ],
        },
        LookDef {
            name: "Bleach",
            effects: &[
                (EffectKind::Curves, &[("Master 1/4", 0.32), ("Master 3/4", 0.72)]),
                (EffectKind::Primaries, &[("Gain R", 0.95), ("Gain G", 0.95), ("Gain B", 0.95)]),
            ],
        },
        LookDef {
            name: "Matte",
            effects: &[(EffectKind::Primaries, &[("Lift R", 0.08), ("Lift G", 0.08), ("Lift B", 0.08)])],
        },
        LookDef { name: "Punchy", effects: &[(EffectKind::Curves, &[("Master 1/4", 0.18), ("Master 3/4", 0.85)])] },
        LookDef {
            name: "Pastel",
            effects: &[(
                EffectKind::Primaries,
                &[("Lift R", 0.1), ("Lift G", 0.1), ("Lift B", 0.1), ("Gain R", 0.9), ("Gain G", 0.9), ("Gain B", 0.9)],
            )],
        },
        LookDef {
            name: "Noir",
            effects: &[
                (EffectKind::Grayscale, &[("Amount", 1.0)]),
                (EffectKind::Vignette, &[("Strength", 0.55), ("Radius", 0.6)]),
            ],
        },
        LookDef {
            name: "Sunset",
            effects: &[(EffectKind::Primaries, &[("Gain R", 1.15), ("Gain B", 0.85), ("Temp", 20.0), ("Tint", 10.0)])],
        },
        LookDef {
            name: "Clean",
            effects: &[(EffectKind::Primaries, &[("Gain R", 1.02), ("Gain G", 1.02), ("Gain B", 1.02)])],
        },
    ];
    LOOKS
        .iter()
        .map(|l| {
            let fx: Vec<Effect> = l.effects.iter().map(|&(k, p)| look_effect(k, p)).collect();
            EffectPreset { name: l.name.into(), json: serde_json::to_string(&fx).unwrap_or_default() }
        })
        .collect()
}

/// Apply a built-in (or saved) Look to a clip, replacing its effect stack (`apply_effects`'s existing
/// template-apply semantics - see its own doc comment on why a manual effect the user already added is
/// lost), then mixing every param of the newly-set effects toward `kind.params()[i].default` by
/// `1 - intensity` (only the base `.value` - keyframes, which a Look never sets, are untouched).
/// `intensity` 0.0 is therefore exactly identity, `1.0` is the Look unmodified.
pub fn apply_look(preset: &EffectPreset, project: &mut Project, clip: Id, intensity: f32) -> bool {
    if !apply_effects(preset, project, clip) {
        return false;
    }
    let f = intensity.clamp(0.0, 1.0) as f64;
    if let Some(c) = project.clip_mut(clip) {
        for e in &mut c.effects {
            let specs = e.kind.params();
            for (i, anim) in e.params.iter_mut().enumerate() {
                let default = specs.get(i).map(|s| s.default).unwrap_or(anim.value);
                anim.value = default + (anim.value - default) * f;
            }
        }
    }
    true
}

/// True when a template holds nothing but adjustment layers - the Presets pane gives those their own
/// section. ponytail: decodes the JSON per frame the list is drawn; templates are a handful of small
/// blobs, memoise if that stops being true.
pub fn is_adjustment_template(t: &Template) -> bool {
    decode_template(t).is_some_and(|(c, _)| !c.is_empty() && c.iter().all(|c| c.kind == ClipKind::Adjustment))
}

/// True when a template holds at least one container clip.
pub fn is_container_template(t: &Template) -> bool {
    decode_template(t).is_some_and(|(c, _)| c.iter().any(|c| c.container))
}

// ---- ws:text-titles ----
/// True when a template holds ONLY Text/Shape/Adjustment clips - the Gallery Titles tab filter for user
/// templates. Mirrors `is_adjustment_template`/`is_container_template`; these three kinds are exactly
/// the ones `Project::place_clips` never `continue`s past (it only skips a clip whose asset didn't remap,
/// or a dangling `Sequence` - model/ops/templates.rs), so a Titles-tab template always places 1:1.
pub fn is_text_template(t: &Template) -> bool {
    decode_template(t).is_some_and(|(c, _)| {
        !c.is_empty() && c.iter().all(|c| matches!(c.kind, ClipKind::Text | ClipKind::Shape | ClipKind::Adjustment))
    })
}

// ---- ws:inspector-gallery ----
/// One normalised (0..1 clip-relative) speed-ramp curve for `builtin_speed_ramps`.
fn ramp(name: &str, points: &[(f64, f64)]) -> CurvePreset {
    let keys = points.iter().map(|&(t, v)| Keyframe { t, v, ease: Ease::EaseInOut }).collect();
    CurvePreset { name: name.into(), keys, absolute: false }
}

/// 5 built-in speed-ramp presets, fed straight into `apply_curve(preset, &mut clip.speed_curve, dur,
/// false)` - normalised keys are stretched to the clip's actual duration on apply. Values are speed
/// multipliers (1.0 = normal), matching `Clip::speed_curve`'s existing convention.
pub fn builtin_speed_ramps() -> Vec<CurvePreset> {
    vec![
        // quick alternating up-tempo cuts
        ramp("Montage", &[(0.0, 1.0), (0.15, 2.5), (0.3, 1.0), (0.6, 3.0), (0.75, 1.0), (1.0, 1.5)]),
        // slow-motion highlight in the middle third
        ramp("Hero", &[(0.0, 1.0), (0.35, 1.0), (0.5, 0.25), (0.65, 1.0), (1.0, 1.0)]),
        // extreme "bullet time" dip
        ramp("Bullet", &[(0.0, 1.0), (0.45, 1.0), (0.5, 0.05), (0.55, 1.0), (1.0, 1.0)]),
        // sudden speed jump near the end (a hard cut in perceived pace, not the length)
        ramp("Jump", &[(0.0, 1.0), (0.7, 1.0), (0.72, 4.0), (1.0, 4.0)]),
        // brief fast flash-forward
        ramp("Flash", &[(0.0, 1.0), (0.4, 1.0), (0.5, 6.0), (0.6, 1.0), (1.0, 1.0)]),
    ]
}

fn caption(name: &str, f: impl FnOnce(&mut TextStyle)) -> TextStyle {
    let mut t = TextStyle { text: name.into(), ..Default::default() };
    f(&mut t);
    t
}

/// 8 built-in caption styles, settable onto `Project.subtitle_style` via `subtitles.style_preset`.
pub fn builtin_caption_styles() -> Vec<TextStyle> {
    vec![
        caption("Classic", |_| {}),
        caption("Bold Outline", |t| {
            t.bold = true;
            t.outline_width.value = 3.0;
        }),
        caption("Boxed", |t| {
            t.box_color = [0, 0, 0, 200];
            t.box_padding = 8.0;
        }),
        caption("Yellow Pop", |t| {
            t.color = [255, 220, 0, 255];
            t.bold = true;
            t.outline_width.value = 2.0;
        }),
        caption("Soft Shadow", |t| {
            t.shadow = true;
            t.shadow_blur = 4.0;
            t.shadow_x = 1.0;
            t.shadow_y = 2.0;
        }),
        caption("Minimal", |t| {
            t.outline_width.value = 0.0;
            t.size.value = 32.0;
        }),
        caption("Big Impact", |t| {
            t.bold = true;
            t.size.value = 56.0;
            t.outline_width.value = 4.0;
        }),
        caption("Karaoke", |t| {
            t.color = [255, 255, 255, 255];
            t.outline_color = [0, 120, 255, 255];
            t.outline_width.value = 3.0;
        }),
    ]
}

#[derive(serde::Serialize, serde::Deserialize)]
struct TemplateData {
    clips: Vec<Clip>,
    assets: Vec<Asset>,
}

/// Serialise a group of clips (+ the assets they use) into a Template (times relative to the earliest start).
pub fn capture_template(name: &str, project: &Project, clip_ids: &[crate::model::Id]) -> Template {
    let mut clips: Vec<Clip> = clip_ids.iter().filter_map(|&id| project.clip(id)).cloned().collect();
    let start = clips.iter().map(|c| c.start).fold(f64::INFINITY, f64::min);
    let mut assets = Vec::new();
    for c in &mut clips {
        c.start -= if start.is_finite() { start } else { 0.0 };
        if c.uses_asset() && !assets.iter().any(|a: &Asset| a.id == c.asset) {
            if let Some(a) = project.asset(c.asset) {
                assets.push(a.clone());
            }
        }
    }
    let json = serde_json::to_string(&TemplateData { clips, assets }).unwrap_or_default();
    Template { name: name.into(), json }
}

/// Decode a template into (clips, assets) ready for `Project::place_clips`. None on malformed JSON.
pub fn decode_template(t: &Template) -> Option<(Vec<Clip>, Vec<Asset>)> {
    serde_json::from_str::<TemplateData>(&t.json).ok().map(|d| (d.clips, d.assets))
}

// ---- ws:text-titles ----
/// 3 tiny built-in title templates for the Gallery Titles tab, literal Rust-constructed `TemplateData`
/// (not JSON asset files - each encodes to well under 1 KB) built the same way `capture_template`
/// assembles a user one. Every clip id is a placeholder (`0`) - `Project::place_clips` assigns fresh ids
/// on placement, exactly as it does for a captured user template. Each has at least one clip with a
/// non-empty `exposed` so Placing one always gives the Gallery's Customize panel something to show.
pub fn builtin_titles() -> Vec<Template> {
    fn encode(clips: Vec<Clip>) -> String {
        serde_json::to_string(&TemplateData { clips, assets: Vec::new() }).unwrap_or_default()
    }
    let lower_third = {
        let mut bar = Clip::new(0, ClipKind::Shape, "Bar", 0.0, 4.0);
        if let Some(s) = bar.shape.as_mut() {
            s.fill = [15, 15, 20, 210];
            s.w = Animated::new(420.0);
            s.h = Animated::new(70.0);
        }
        bar.x = Animated::new(-460.0);
        bar.y = Animated::new(400.0);
        let mut headline = Clip::new(0, ClipKind::Text, "Headline", 0.0, 4.0);
        if let Some(t) = headline.text.as_mut() {
            t.text = "Name Here".into();
            t.size = Animated::new(36.0);
            t.align = 0;
        }
        headline.x = Animated::new(-440.0);
        headline.y = Animated::new(400.0);
        headline.exposed = vec!["text.text".into(), "text.color".into()];
        vec![bar, headline]
    };
    let title_card = {
        let mut headline = Clip::new(0, ClipKind::Text, "Title", 0.0, 3.0);
        if let Some(t) = headline.text.as_mut() {
            t.text = "Your Title Here".into();
            t.size = Animated::new(96.0);
            t.bold = true;
            t.box_color = [0, 0, 0, 160];
            t.box_padding = 24.0;
        }
        headline.exposed = vec!["text.text".into(), "text.color".into(), "text.size".into()];
        vec![headline]
    };
    let caption_box = {
        let mut cap = Clip::new(0, ClipKind::Text, "Caption", 0.0, 4.0);
        if let Some(t) = cap.text.as_mut() {
            t.text = "Caption text".into();
            t.size = Animated::new(30.0);
            t.box_color = [0, 0, 0, 180];
            t.box_padding = 10.0;
        }
        cap.y = Animated::new(420.0);
        cap.exposed = vec!["text.text".into()];
        vec![cap]
    };
    vec![
        Template { name: "Lower Third".into(), json: encode(lower_third) },
        Template { name: "Title Card".into(), json: encode(title_card) },
        Template { name: "Caption Box".into(), json: encode(caption_box) },
    ]
}

/// Rewrite `t`'s captured clips, ADDING each `(clip_index, field)` pair's `field` to that clip's
/// `Clip.exposed` (position within the template's own clip list, not a live id - `clip_index` addresses
/// `decode_template(t)`'s clip Vec by position). The `templates.expose` MCP tool's entire pure body.
/// `None` if `t.json` doesn't decode, or any `clip_index` is out of range - no partial rewrite either
/// way, matching `decode_template`'s own all-or-nothing shape.
pub fn expose_fields(t: &Template, fields: &[(usize, String)]) -> Option<Template> {
    let (mut clips, assets) = decode_template(t)?;
    for (i, field) in fields {
        let c = clips.get_mut(*i)?;
        if !c.exposed.iter().any(|e| e == field) {
            c.exposed.push(field.clone());
        }
    }
    let json = serde_json::to_string(&TemplateData { clips, assets }).ok()?;
    Some(Template { name: t.name.clone(), json })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AudioStreamInfo, ClipKind, Id, NodeKind};
    use crate::settings::Settings;

    fn asset(id: Id, dur: f64, streams: usize) -> Asset {
        Asset {
            id,
            path: format!("C:/preset-test-{id}.mp4"),
            kind: ClipKind::Video,
            duration: dur,
            width: 1280,
            height: 720,
            fps: 30.0,
            audio_streams: (0..streams)
                .map(|i| AudioStreamInfo { index: i, channels: 2, sample_rate: 48000, ..Default::default() })
                .collect(),
            codec: "h264".into(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
        }
    }

    #[test]
    fn curve_roundtrip_scaled_and_exact() {
        let mut a = Animated::new(0.0);
        a.toggle_key(0.0);
        a.set_at(1.0, 5.0);
        a.set_at(2.0, 10.0);
        a.set_ease_at(0.0, Ease::EaseIn);
        let p = capture_curve("c", &a, 2.0, false).unwrap();
        assert!(!p.absolute);
        assert!((p.keys[1].t - 0.5).abs() < 1e-9, "normalised to 0..1");
        // apply scaled back to the same duration → identical values at sample times, ease kept
        let mut b = Animated::new(0.0);
        apply_curve(&p, &mut b, 2.0, true);
        for t in [0.0, 0.3, 0.5, 1.0, 1.7, 2.0] {
            assert!((a.at(t) - b.at(t)).abs() < 1e-9, "t={t}");
        }
        assert_eq!(b.keys[0].ease, Ease::EaseIn);
        // scaled to twice the duration → same values at proportional times
        apply_curve(&p, &mut b, 4.0, true);
        assert!((b.at(2.0) - a.at(1.0)).abs() < 1e-9);
        assert!((b.keys[2].t - 4.0).abs() < 1e-9);
        // "exact" on a normalised preset still stretches - placing 0..1 as seconds would squash the
        // whole animation into the first second of the clip
        apply_curve(&p, &mut b, 4.0, false);
        assert!((b.keys[2].t - 4.0).abs() < 1e-9);
        // absolute preset: seconds survive any target duration, scaled or not
        let pa = capture_curve("c", &a, 2.0, true).unwrap();
        let mut c = Animated::new(0.0);
        apply_curve(&pa, &mut c, 10.0, true);
        assert!((c.keys[1].t - 1.0).abs() < 1e-9);
        assert!((c.at(1.0) - 5.0).abs() < 1e-9);
        // no keys → no preset
        assert!(capture_curve("x", &Animated::new(1.0), 2.0, false).is_none());
        // scaled_keys helper
        let sk = scaled_keys(&p.keys, p.absolute, 8.0);
        assert!((sk[1].t - 4.0).abs() < 1e-9);
        let sk = scaled_keys(&pa.keys, pa.absolute, 8.0);
        assert!((sk[1].t - 1.0).abs() < 1e-9);
    }

    #[test]
    fn motion_capture_apply_with_effects() {
        let mut c = Clip::new(1, ClipKind::Video, "c", 0.0, 2.0);
        c.x.toggle_key(0.0);
        c.x.set_at(2.0, 100.0);
        c.effects.push(Effect::new(EffectKind::Blur));
        c.effects[0].params[0].toggle_key(0.0);
        c.effects[0].params[0].set_at(2.0, 20.0);
        let m = capture_motion("m", &c);
        let names: Vec<&str> = m.props.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["Position X", "Blur: Radius"]);
        // apply to a fresh clip of another length: effect created, values reproduced at the scaled times
        let mut d = Clip::new(2, ClipKind::Video, "d", 0.0, 4.0);
        apply_motion(&m, &mut d, true);
        assert!((d.x.at(4.0) - 100.0).abs() < 1e-9);
        assert!((d.x.at(2.0) - c.x.at(1.0)).abs() < 1e-9);
        assert_eq!(d.effects.len(), 1);
        assert_eq!(d.effects[0].kind, EffectKind::Blur);
        assert!((d.effects[0].params[0].at(4.0) - 20.0).abs() < 1e-9);
        // unknown properties are skipped without touching the clip
        let bogus = MotionPreset {
            name: "b".into(),
            props: vec![("Nope: Nothing".into(), CurvePreset::default()), ("Nonsense".into(), CurvePreset::default())],
        };
        let before = d.effects.len();
        apply_motion(&bogus, &mut d, true);
        assert_eq!(d.effects.len(), before);
    }

    #[test]
    fn merge_keeps_existing_keys_and_layers_at_the_offset() {
        // a property already animated 0..1 s
        let mut a = Animated::new(0.0);
        a.toggle_key(0.0);
        a.set_at(1.0, 5.0);
        let p = CurvePreset {
            name: "slide".into(),
            keys: vec![
                Keyframe { t: 0.0, v: -100.0, ease: Ease::EaseIn },
                Keyframe { t: 0.5, v: 0.0, ease: Ease::Linear },
            ],
            absolute: false,
        };
        // merged over the 2 s left after the offset: keys at 2.0 and 3.0, the old two untouched
        merge_curve(&p, &mut a, 2.0, 2.0);
        let ts: Vec<f64> = a.keys.iter().map(|k| k.t).collect();
        assert_eq!(ts, vec![0.0, 1.0, 2.0, 3.0]);
        assert!((a.at(1.0) - 5.0).abs() < 1e-9, "existing keys survive");
        assert!((a.at(2.0) + 100.0).abs() < 1e-9);
        assert_eq!(a.keys[2].ease, Ease::EaseIn);
        // a preset key landing on an existing time wins, and stays sorted
        merge_curve(&p, &mut a, 2.0, 1.0);
        let ts: Vec<f64> = a.keys.iter().map(|k| k.t).collect();
        assert_eq!(ts, vec![0.0, 1.0, 2.0, 3.0]);
        assert!((a.at(1.0) + 100.0).abs() < 1e-9, "preset overwrote the key at 1.0");
        // motion level: layering two presets on the same property keeps both
        let mut c = Clip::new(1, ClipKind::Video, "c", 0.0, 4.0);
        let m = MotionPreset { name: "m".into(), props: vec![("Position X".into(), p.clone())] };
        merge_motion(&m, &mut c, 0.0); // 0.0 + 2.0
        merge_motion(&m, &mut c, 2.5); // 2.5 + 3.25
        assert_eq!(c.x.keys.len(), 4, "both layers are there: {:?}", c.x.keys);
        assert!(c.x.keys.windows(2).all(|w| w[0].t < w[1].t), "sorted");
        // effect params create their effect just like apply_motion
        let m = MotionPreset { name: "m".into(), props: vec![("Blur: Radius".into(), p)] };
        merge_motion(&m, &mut c, 0.0);
        assert_eq!(c.effects.len(), 1);
        assert!(c.effects[0].params[0].is_animated());
    }

    #[test]
    fn builtin_motions_apply_clean() {
        let motions = builtin_motions();
        assert!(motions.len() >= 12);
        for m in motions {
            let mut c = Clip::new(1, ClipKind::Video, "c", 0.0, 3.0);
            apply_motion(&m, &mut c, true);
            assert!(c.all_animated().iter().any(|a| a.is_animated()), "{} did nothing", m.name);
            for a in c.all_animated() {
                for k in &a.keys {
                    assert!(k.t >= -1e-9 && k.t <= 3.0 + 1e-9, "{}: key at {}", m.name, k.t);
                }
            }
        }
    }

    #[test]
    fn template_roundtrip_place() {
        let mut p = Project::from_media(asset(0, 10.0, 1));
        let ids: Vec<Id> = p.all_clips().map(|(_, c)| c.id).collect();
        assert_eq!(ids.len(), 2); // video + audio
        let t = capture_template("Pair", &p, &ids);
        let (clips, assets) = decode_template(&t).unwrap();
        assert_eq!((clips.len(), assets.len()), (2, 1));
        assert!(clips.iter().all(|c| c.start.abs() < 1e-9));
        // place into a different project: fresh ids, asset re-added by path, clips at the new time
        let mut q = Project::from_media(asset(7, 5.0, 0));
        let new = q.place_clips(clips, assets, 20.0);
        assert_eq!(new.len(), 2);
        for id in &new {
            let c = q.clip(*id).unwrap();
            assert!((c.start - 20.0).abs() < 1e-9);
            let a = q.asset(c.asset).expect("asset remapped");
            assert_eq!(a.path, "C:/preset-test-0.mp4");
        }
        assert_eq!(q.assets.len(), 2);
        // times are stored relative to the earliest start
        let mut r = Project::new();
        let a = r.add_text_clip(3.0, 2.0);
        let b = r.add_text_clip(6.0, 1.0);
        let t = capture_template("Texts", &r, &[a, b]);
        let (clips, _) = decode_template(&t).unwrap();
        assert!(clips[0].start.abs() < 1e-9);
        assert!((clips[1].start - 3.0).abs() < 1e-9);
        // malformed JSON → None
        assert!(decode_template(&Template { name: "x".into(), json: "{ nope".into() }).is_none());
    }
    /// Capture -> settings.json -> apply, for both flavours of effect preset.
    #[test]
    fn effect_presets_round_trip_through_settings() {
        let mut p = Project::from_media(asset(0, 10.0, 0));
        let id = p.all_clips().next().unwrap().1.id;
        let c = p.clip_mut(id).unwrap();
        c.effects.push(Effect::new(EffectKind::Blur));
        c.effects[0].params[0].set_at(0.0, 12.0);

        let mut s = Settings::default();
        s.effect_presets.push(capture_effects("Look", p.clip(id).unwrap()));
        p.ensure_graph(id);
        s.effect_presets.push(capture_effects("Graph", p.clip(id).unwrap()));
        // machine-local: the presets ride in settings.json, not the project file
        let s: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert!(!s.effect_presets[0].is_graph() && s.effect_presets[1].is_graph());

        // apply into a different project
        let mut q = Project::from_media(asset(1, 4.0, 0));
        let qid = q.all_clips().next().unwrap().1.id;
        assert!(apply_effects(&s.effect_presets[0], &mut q, qid));
        assert_eq!(q.clip(qid).unwrap().effects, p.clip(id).unwrap().effects);
        assert!(q.clip(qid).unwrap().graph.is_none(), "a stack preset clears the graph shadowing it");
        assert!(apply_effects(&s.effect_presets[1], &mut q, qid));
        let g = q.clip(qid).unwrap().graph.clone().unwrap();
        assert_eq!(g.nodes.len(), p.clip(id).unwrap().graph.as_ref().unwrap().nodes.len());
        assert!(g.edges.iter().all(|e| g.nodes.iter().any(|n| n.id == e.from)), "edges kept their nodes");
        // the pasted nodes got project ids, so the next node added is not a duplicate
        let fresh = q.add_node(qid, NodeKind::Color([0; 4]), 0.0, 0.0).unwrap();
        assert!(g.nodes.iter().all(|n| n.id != fresh));
        // JSON that is not what it claims to be changes nothing
        assert!(!apply_effects(&EffectPreset { name: "x".into(), json: "{ nope".into() }, &mut q, qid));
        assert!(!apply_effects(&EffectPreset { name: "x".into(), json: "[1,2]".into() }, &mut q, qid));

        // adjustment layers are simply templates holding nothing else (own section in the pane)
        let a = q.add_adjustment_clip(0.0, 2.0);
        assert!(is_adjustment_template(&capture_template("Adj", &q, &[a])));
        assert!(!is_adjustment_template(&capture_template("Clip", &q, &[qid])));
    }

    // ---- ws:color-engine ----
    #[test]
    fn builtin_looks_all_parse_and_apply() {
        let looks = builtin_looks();
        assert_eq!(looks.len(), 12);
        for look in &looks {
            assert!(serde_json::from_str::<Vec<Effect>>(&look.json).is_ok(), "{}: bad json", look.name);
            let mut p = Project::from_media(asset(0, 5.0, 0));
            let id = p.all_clips().next().unwrap().1.id;
            let before = p.clip(id).unwrap().effects.clone();
            assert!(apply_look(look, &mut p, id, 1.0), "{}: intensity 1.0", look.name);
            assert_ne!(p.clip(id).unwrap().effects, before, "{}: applying a look must change something", look.name);

            let mut p2 = Project::from_media(asset(1, 5.0, 0));
            let id2 = p2.all_clips().next().unwrap().1.id;
            let before2 = p2.clip(id2).unwrap().effects.clone();
            assert!(apply_look(look, &mut p2, id2, 0.0), "{}: intensity 0.0", look.name);
            assert_ne!(p2.clip(id2).unwrap().effects, before2, "{}: even at 0.0 the kinds/count change", look.name);
        }
    }

    #[test]
    fn apply_look_intensity_zero_is_a_no_op_on_defaults() {
        for look in builtin_looks() {
            let mut p = Project::from_media(asset(0, 5.0, 0));
            let id = p.all_clips().next().unwrap().1.id;
            assert!(apply_look(&look, &mut p, id, 0.0));
            for e in &p.clip(id).unwrap().effects {
                for (i, spec) in e.kind.params().iter().enumerate() {
                    let v = e.params[i].value;
                    assert!(
                        (v - spec.default).abs() < 1e-9,
                        "{}: {} param {} = {v}, want default {}",
                        look.name,
                        e.kind.name(),
                        spec.name,
                        spec.default
                    );
                }
            }
        }
    }

    #[test]
    fn container_roundtrip_template() {
        let mut p = Project::new();
        let (vid, aid) = p.add_container_clip(0.0, 5.0);
        if let Some(c) = p.clip_mut(vid) {
            c.container_label = "Hero Shot".into();
        }
        let t = capture_template("ContainerTpl", &p, &[vid, aid]);
        assert!(is_container_template(&t));
        assert!(!is_adjustment_template(&t));

        let (clips, assets) = decode_template(&t).unwrap();
        assert_eq!(clips.len(), 2);
        assert!(clips[0].container);
        assert!(clips[1].container);

        let mut q = Project::new();
        let placed = q.place_clips(clips, assets, 10.0);
        assert_eq!(placed.len(), 2);
        let v = q.clip(placed[0]).unwrap();
        assert!(v.container);
        assert_eq!(v.container_label, "Hero Shot");
        assert_eq!(v.start, 10.0);
        assert_eq!(v.duration, 5.0);
        assert!(v.is_empty_container());
    }

    #[test]
    fn replace_container_preserves_effects_and_transforms() {
        let mut p = Project::new();
        let (vid, _) = p.add_container_clip(2.0, 6.0);
        let a1 = p.add_asset(asset(1, 10.0, 1));
        p.replace_container_media(vid, a1);

        // Add effects, keyframes, transform changes
        let c = p.clip_mut(vid).unwrap();
        c.x.set_at(0.0, 100.0);
        c.scale.set_at(0.0, 1.5);
        c.opacity.set_at(0.0, 0.8);
        c.effects.push(Effect::new(EffectKind::Blur));
        c.effects[0].params[0].set_at(0.0, 15.0);

        // Replace with another asset
        let a2 = p.add_asset(asset(2, 20.0, 1));
        assert!(p.replace_container_media(vid, a2));

        let c2 = p.clip(vid).unwrap();
        assert_eq!(c2.asset, a2);
        assert_eq!(c2.start, 2.0);
        assert_eq!(c2.duration, 6.0);
        assert_eq!(c2.src_in, 0.0);
        assert_eq!(c2.x.at(0.0), 100.0);
        assert_eq!(c2.scale.at(0.0), 1.5);
        assert_eq!(c2.opacity.at(0.0), 0.8);
        assert_eq!(c2.effects.len(), 1);
        assert_eq!(c2.effects[0].kind, EffectKind::Blur);
        assert_eq!(c2.effects[0].params[0].at(0.0), 15.0);
    }

    #[test]
    fn replace_container_pair_syncs_audio() {
        let mut p = Project::new();
        let (vid, aid) = p.add_container_clip(0.0, 8.0);
        let a = p.add_asset(asset(1, 15.0, 2));

        assert!(p.replace_container_pair(vid, a));

        let vc = p.clip(vid).unwrap();
        let ac = p.clip(aid).unwrap();
        assert_eq!(vc.asset, a);
        assert_eq!(ac.asset, a);
        assert_eq!(vc.link, ac.link);
        assert_eq!(ac.audio_stream, 0);
    }

    #[test]
    fn make_and_unmake_container() {
        let mut p = Project::from_media(asset(1, 10.0, 1));
        let ids: Vec<Id> = p.all_clips().map(|(_, c)| c.id).collect();
        assert_eq!(ids.len(), 2);
        assert!(!p.clip(ids[0]).unwrap().container);

        p.make_container(&[ids[0]]);
        assert!(p.clip(ids[0]).unwrap().container);
        assert!(p.clip(ids[1]).unwrap().container, "linked audio converted together");

        if let Some(c) = p.clip_mut(ids[0]) {
            c.container_label = "Slot A".into();
        }
        p.unmake_container(&[ids[0]]);
        assert!(!p.clip(ids[0]).unwrap().container);
        assert!(p.clip(ids[0]).unwrap().container_label.is_empty());
        assert!(!p.clip(ids[1]).unwrap().container);
    }

    #[test]
    fn empty_container_serialization_roundtrip() {
        let mut p = Project::new();
        let (vid, aid) = p.add_container_clip(1.0, 4.0);
        if let Some(c) = p.clip_mut(vid) {
            c.container_label = "Intro".into();
        }
        let json = p.to_json();
        let p2 = Project::from_json(&json).unwrap();
        let vc = p2.clip(vid).unwrap();
        let ac = p2.clip(aid).unwrap();
        assert!(vc.container);
        assert!(ac.container);
        assert!(vc.is_empty_container());
        assert!(ac.is_empty_container());
        assert_eq!(vc.container_label, "Intro");
        assert_eq!(vc.link, ac.link);
    }

    // ---- ws:inspector-gallery ----
    #[test]
    fn speed_ramps_are_named_and_start_end_near_normal() {
        let ramps = builtin_speed_ramps();
        assert_eq!(ramps.len(), 5);
        let names: Vec<&str> = ramps.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Montage", "Hero", "Bullet", "Jump", "Flash"]);
        for r in &ramps {
            assert!(!r.absolute, "{}: normalised, so it scales to any clip duration", r.name);
            assert!(r.keys.first().unwrap().t == 0.0 && r.keys.last().unwrap().t == 1.0);
        }
    }

    #[test]
    fn speed_ramp_scales_to_clip_duration() {
        let hero = builtin_speed_ramps().into_iter().find(|r| r.name == "Hero").unwrap();
        let mut anim = Animated::new(1.0);
        apply_curve(&hero, &mut anim, 10.0, false);
        // normalised t=0.5 scales to 5.0s on a 10s clip
        assert!(anim.keys.iter().any(|k| (k.t - 5.0).abs() < 1e-9));
    }

    #[test]
    fn caption_styles_are_named_and_distinct() {
        let styles = builtin_caption_styles();
        assert_eq!(styles.len(), 8);
        let mut names: Vec<&str> = styles.iter().map(|s| s.text.as_str()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 8, "every style has a unique name");
    }

    // ---- ws:text-titles ----

    #[test]
    fn builtin_titles_decode_and_are_text_templates() {
        let titles = builtin_titles();
        assert_eq!(titles.len(), 3);
        let mut names: Vec<&str> = titles.iter().map(|t| t.name.as_str()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 3, "every title has a unique name");
        for t in &titles {
            let (clips, assets) = decode_template(t).unwrap_or_else(|| panic!("{} failed to decode", t.name));
            assert!(assets.is_empty(), "{}: titles carry no media assets", t.name);
            assert!(!clips.is_empty(), "{}: at least one clip", t.name);
            assert!(is_text_template(t), "{}: must be a Text/Shape/Adjustment-only template", t.name);
            assert!(
                clips.iter().any(|c| !c.exposed.is_empty()),
                "{}: at least one clip exposes a Customize field",
                t.name
            );
        }
    }

    #[test]
    fn is_text_template_rejects_video_clips() {
        let p = Project::from_media(asset(1, 5.0, 1));
        let vid_id = p.all_clips().find(|(_, c)| c.kind == ClipKind::Video).unwrap().1.id;
        let vid_template = capture_template("Vid", &p, &[vid_id]);
        assert!(!is_text_template(&vid_template));
    }
}
