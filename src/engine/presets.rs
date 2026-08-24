//! Keyframe presets, motion presets and clip templates — capture from clips and apply back.
//! Curve presets store keys normalised to 0..1 of the clip length (stretched to the target clip's
//! duration when applied) or as absolute seconds (kept as saved).

use crate::model::{
    Animated, Asset, Clip, ClipKind, Ease, Effect, EffectKind, Id, Keyframe, NodeGraph, Project, MIN_CLIP,
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
/// ignore the duration anyway — placing a normalised preset's 0..1 times as seconds would crush the
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
    // ponytail: slide offsets assume a ~1080p project — a wider clip just starts a bit on-screen.
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
    // ponytail: duplicate effects of the same kind share a label — the first instance wins on apply.
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
/// the two apart afterwards (`EffectPreset::is_graph`) — nothing else has to be stored.
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

/// True when a template holds nothing but adjustment layers — the Presets pane gives those their own
/// section. ponytail: decodes the JSON per frame the list is drawn; templates are a handful of small
/// blobs, memoise if that stops being true.
pub fn is_adjustment_template(t: &Template) -> bool {
    decode_template(t).is_some_and(|(c, _)| !c.is_empty() && c.iter().all(|c| c.kind == ClipKind::Adjustment))
}

/// True when a template holds at least one container clip.
pub fn is_container_template(t: &Template) -> bool {
    decode_template(t).is_some_and(|(c, _)| c.iter().any(|c| c.container))
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
        // "exact" on a normalised preset still stretches — placing 0..1 as seconds would squash the
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
}
