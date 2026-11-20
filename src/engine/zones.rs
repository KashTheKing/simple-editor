//! ---- ws:zones-markers ----
//! Effect zones as the renderer sees them: ordinary model data. `render_view` gives back the project with
//! - a whole-timeline zone as an Adjustment clip on a video track of its own above every other video track
//!   (the canvas below it runs through the zone's stack, exactly like a user's adjustment layer);
//! - a one-track zone as its effects appended to each visual clip of that track it overlaps, windowed to
//!   the overlap (`Effect.start` / `len`) with its keyframes shifted to clip-local time.
//!
//! Nothing new runs per frame: the view is built where the player, export and the size probe already take
//! their copy of the project (beside `browser::render_view`), so preview, export and prerender draw zones
//! through the same CPU / GPU effect path clips use.

use crate::model::{Clip, ClipKind, Effect, EffectZone, Id, Project, Track, TrackKind};
use std::borrow::Cow;

/// Ids of what `render_view` adds start here, far above any id a project hands out (and clear of
/// `browser::render_view`'s range).
const VIEW_IDS: Id = 1 << 41;

/// Is zone `z` drawn on the timeline open in `p` at all?
fn live(p: &Project, z: &EffectZone) -> bool {
    z.enabled && z.sequence == p.editing && z.dur > 0.0 && z.effects.iter().any(|e| e.enabled)
}

/// The project as it is drawn: `p` with its zones made into adjustment layers / clip effects (a copy; `p`
/// is untouched). Borrowed when there is no zone to draw.
pub fn render_view(p: &Project) -> Cow<'_, Project> {
    if !p.zones.iter().any(|z| live(p, z)) {
        return Cow::Borrowed(p);
    }
    let mut v = p.clone();
    v.next_id = v.next_id.max(VIEW_IDS);
    let zones: Vec<EffectZone> = std::mem::take(&mut v.zones);
    for z in zones.iter().filter(|z| live(p, z)) {
        match z.track {
            Some(tid) => apply_on_track(&mut v, z, tid),
            None => apply_over_all(&mut v, z),
        }
    }
    v.zones = zones;
    Cow::Owned(v)
}

/// `render_view` and then `browser::render_view`, owned: what the player and export draw.
pub fn drawn(p: &Project) -> Project {
    match render_view(p) {
        Cow::Borrowed(p) => crate::engine::browser::render_view(p).into_owned(),
        Cow::Owned(z) => {
            let b = match crate::engine::browser::render_view(&z) {
                Cow::Owned(b) => Some(b),
                Cow::Borrowed(_) => None,
            };
            b.unwrap_or(z)
        }
    }
}

/// A whole-timeline zone: an Adjustment clip on its own video track, above the others.
fn apply_over_all(v: &mut Project, z: &EffectZone) {
    let id = v.new_id();
    let mut clip = Clip::new(v.new_id(), ClipKind::Adjustment, z.label(), z.t, z.dur);
    clip.effects = z.effects.clone();
    let mut track = Track::new(id, TrackKind::Video, "Zone");
    // a solo elsewhere would hide a track that is not soloed: a zone stays on whatever is shown
    track.solo = v.tracks.iter().any(|t| t.kind == TrackKind::Video && t.solo);
    track.clips.push(clip);
    let at = v.video_tracks().last().map(|i| i + 1).unwrap_or(0);
    v.tracks.insert(at, track);
}

/// A one-track zone: its stack on every visual clip of track `tid` under it, for the overlap only.
fn apply_on_track(v: &mut Project, z: &EffectZone, tid: Id) {
    let Some(track) = v.tracks.iter_mut().find(|t| t.id == tid && t.kind == TrackKind::Video) else { return };
    for c in track.clips.iter_mut().filter(|c| c.is_visual() && c.start < z.end() && c.end() > z.t) {
        for e in &z.effects {
            if let Some(fx) = on_clip(e, z, c) {
                c.effects.push(fx);
            }
        }
    }
}

/// Zone effect `e` as an effect of clip `c`: on for the part of its own window (zone-local) that falls
/// inside the clip, keyframes moved from zone-local to clip-local time. None when that part is empty.
pub(crate) fn on_clip(e: &Effect, z: &EffectZone, c: &Clip) -> Option<Effect> {
    let from = z.t + e.start.max(0.0);
    let to = if e.len > 0.0 { (from + e.len).min(z.end()) } else { z.end() };
    let (a, b) = (from.max(c.start), to.min(c.end()));
    if b - a <= 1e-6 {
        return None;
    }
    let shift = z.t - c.start;
    let mut fx = e.clone();
    (fx.start, fx.len) = (a - c.start, b - a);
    for p in &mut fx.params {
        for k in &mut p.keys {
            k.t += shift;
        }
    }
    if let Some(m) = &mut fx.mask {
        shift_keys(m, shift);
    }
    Some(fx)
}

/// Every keyframe inside a mask, moved by `shift` seconds (through its JSON: a mask has several kinds of
/// animated fields and this runs only when the project changes).
fn shift_keys<T: serde::Serialize + serde::de::DeserializeOwned>(v: &mut T, shift: f64) {
    fn walk(x: &mut serde_json::Value, shift: f64) {
        match x {
            serde_json::Value::Object(o) => {
                if let Some(serde_json::Value::Array(keys)) = o.get_mut("keys") {
                    for k in keys.iter_mut() {
                        if let Some(t) = k.get("t").and_then(|t| t.as_f64()) {
                            k["t"] = serde_json::json!(t + shift);
                        }
                    }
                }
                o.values_mut().for_each(|y| walk(y, shift));
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(|y| walk(y, shift)),
            _ => {}
        }
    }
    let Ok(mut j) = serde_json::to_value(&*v) else { return };
    walk(&mut j, shift);
    // through text: from_value would compile a second copy of the deserialiser
    if let Ok(back) = serde_json::from_str(&j.to_string()) {
        *v = back;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EffectKind, Ease, Keyframe};

    fn project() -> (Project, Id) {
        let mut p = Project::new();
        let v1 = p.video_tracks()[0];
        p.tracks[v1].clips.push(Clip::new(1, ClipKind::Video, "a", 0.0, 10.0));
        p.tracks[v1].clips.push(Clip::new(2, ClipKind::Video, "b", 10.0, 10.0));
        p.next_id = 100;
        (p.clone(), p.tracks[v1].id)
    }

    #[test]
    fn nothing_to_draw_borrows() {
        let (mut p, _) = project();
        assert!(matches!(render_view(&p), Cow::Borrowed(_)));
        let z = p.add_zone(1.0, 2.0, None, vec![Effect::new(EffectKind::Invert)]);
        p.zone_mut(z).unwrap().enabled = false;
        assert!(matches!(render_view(&p), Cow::Borrowed(_)), "a switched-off zone draws nothing");
    }

    #[test]
    fn whole_timeline_zone_is_an_adjustment_layer_on_top() {
        let (mut p, _) = project();
        p.add_track(TrackKind::Video);
        p.add_zone(30.0, 15.0, None, vec![Effect::new(EffectKind::Grayscale)]);
        let v = render_view(&p).into_owned();
        assert_eq!(v.tracks.len(), p.tracks.len() + 1);
        let top = *v.video_tracks().last().unwrap();
        let c = &v.tracks[top].clips[0];
        assert_eq!((c.kind, c.start, c.duration), (ClipKind::Adjustment, 30.0, 15.0));
        assert_eq!(c.effects[0].kind, EffectKind::Grayscale);
        assert!(v.tracks[top + 1..].iter().all(|t| t.kind == TrackKind::Audio), "video tracks stay before audio");
        assert_eq!(p.zones.len(), 1, "the original is untouched");
    }

    #[test]
    fn track_zone_windows_effects_and_shifts_keys_per_clip() {
        let (mut p, v1) = project();
        let mut fx = Effect::new(EffectKind::Blur);
        fx.params[0].keys = vec![Keyframe { t: 0.0, v: 0.0, ease: Ease::Linear }, Keyframe { t: 8.0, v: 30.0, ease: Ease::Linear }];
        p.add_zone(6.0, 8.0, Some(v1), vec![fx]);
        let v = render_view(&p).into_owned();
        assert_eq!(v.tracks.len(), p.tracks.len(), "no track added");
        let a = v.clip(1).unwrap().effects.last().unwrap();
        assert_eq!((a.start, a.len), (6.0, 4.0), "clip a: 6..10");
        assert_eq!(a.params[0].keys[1].t, 14.0, "zone-local 8 s = clip a's 14 s");
        let b = v.clip(2).unwrap().effects.last().unwrap();
        assert_eq!((b.start, b.len), (0.0, 4.0), "clip b: 10..14");
        assert_eq!(b.params[0].keys[0].t, -4.0);
        // the same value at the same timeline time on both sides of the cut
        assert!((a.at(0, 9.0) - b.at(0, -1.0)).abs() < 1e-9);
        assert!((b.at(0, 2.0) - 30.0 * 6.0 / 8.0).abs() < 1e-9, "12 s on the timeline = 6 s into the zone");
    }

    #[test]
    fn zone_of_another_sequence_is_not_drawn_here() {
        let (mut p, _) = project();
        let z = p.add_zone(0.0, 2.0, None, vec![Effect::new(EffectKind::Invert)]);
        p.zone_mut(z).unwrap().sequence = Some(999);
        assert!(matches!(render_view(&p), Cow::Borrowed(_)));
    }
}
