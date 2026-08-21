//! Keyframe presets, motion presets and clip templates — capture from clips and apply back.
//! Curve presets store keys normalised to 0..1 of the clip length (or absolute seconds); applying can
//! stretch them to the target clip's duration ("scaled") or keep the saved timing ("exact").

use crate::model::{Animated, Asset, Clip, Ease, Effect, EffectKind, Keyframe, Project, MIN_CLIP};
use crate::settings::{CurvePreset, MotionPreset, Template};

/// Snapshot a property's keys as a preset (normalised unless `absolute`). None if the property has no keys.
pub fn capture_curve(name: &str, anim: &Animated, clip_duration: f64, absolute: bool) -> Option<CurvePreset> {
    if anim.keys.is_empty() {
        return None;
    }
    let d = clip_duration.max(MIN_CLIP);
    let keys = anim.keys.iter().map(|k| Keyframe { t: if absolute { k.t } else { k.t / d }, ..*k }).collect();
    Some(CurvePreset { name: name.into(), keys, absolute })
}

/// Apply a curve preset to a property. `scaled` = stretch normalised times to `clip_duration`
/// (absolute presets are placed from 0 unchanged). Replaces existing keys.
pub fn apply_curve(preset: &CurvePreset, anim: &mut Animated, clip_duration: f64, scaled: bool) {
    anim.keys = if scaled { scaled_keys(&preset.keys, preset.absolute, clip_duration) } else { preset.keys.clone() };
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

/// Apply a motion preset to a clip (properties it doesn't have are skipped; effect params create the
/// effect when missing). `scaled` as in `apply_curve`.
pub fn apply_motion(preset: &MotionPreset, clip: &mut Clip, scaled: bool) {
    let dur = clip.duration;
    for (label, curve) in &preset.props {
        if let Some(anim) = prop_of(clip, label) {
            apply_curve(curve, anim, dur, scaled);
            continue;
        }
        let Some((ename, pname)) = label.split_once(": ") else { continue };
        let Some(kind) = EffectKind::ALL.into_iter().find(|k| k.name() == ename) else { continue };
        let Some(pi) = kind.params().iter().position(|p| p.name == pname) else { continue };
        if !clip.effects.iter().any(|e| e.kind == kind) {
            clip.effects.push(Effect::new(kind));
        }
        if let Some(anim) = clip.effects.iter_mut().find(|e| e.kind == kind).and_then(|e| e.params.get_mut(pi)) {
            apply_curve(curve, anim, dur, scaled);
        }
    }
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
    use crate::model::{AudioStreamInfo, ClipKind, Id};

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
        // exact (unscaled): the stored normalised times are placed as-is
        apply_curve(&p, &mut b, 4.0, false);
        assert!((b.keys[2].t - 1.0).abs() < 1e-9);
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
}
