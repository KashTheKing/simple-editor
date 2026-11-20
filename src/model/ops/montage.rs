//! ---- ws:parity-editing ----
//! Make Montage: photos and clips from the Library laid end to end on one video track, with a slow
//! pan-and-zoom on each photo, a cross dissolve between neighbours, and optional background music the
//! photos stretch to fit (Shotcut's slideshow generator). Everything it makes is ordinary clips, keys and
//! transitions the user can still edit, and the caller wraps it in one undo step.
use crate::model::*;

#[derive(Clone, Debug, PartialEq)]
pub struct MontageOpts {
    /// Timeline second the montage starts at.
    pub at: f64,
    /// How long each photo shows (ignored when `fit_music` stretches them).
    pub photo_secs: f64,
    /// Longest stretch of each video clip (0 = the whole clip).
    pub clip_secs: f64,
    /// Cross dissolve between neighbours (0 = straight cuts).
    pub transition: f64,
    /// A slow pan-and-zoom on each photo.
    pub ken_burns: bool,
    /// An audio asset laid under the montage (None = no music).
    pub music: Option<Id>,
    /// Stretch the photos so the montage lasts as long as the music.
    pub fit_music: bool,
}

impl Default for MontageOpts {
    fn default() -> Self {
        Self { at: 0.0, photo_secs: 4.0, clip_secs: 0.0, transition: 0.5, ken_burns: true, music: None, fit_music: true }
    }
}

/// Shortest a photo may get when fitting to music.
const MIN_PHOTO: f64 = 0.5;

/// Each visual asset's length on the timeline, in order: (asset, source range).
pub fn montage_plan(p: &Project, assets: &[Id], o: &MontageOpts) -> Vec<(Id, (f64, f64))> {
    let visual: Vec<&Asset> =
        assets.iter().filter_map(|&id| p.asset(id)).filter(|a| a.has_video() && Some(a.id) != o.music).collect();
    let clip_len = |a: &Asset| {
        let (s, e) = a.range.unwrap_or((0.0, a.duration));
        let len = (e - s).max(MIN_CLIP);
        (s, if o.clip_secs > 0.0 { len.min(o.clip_secs) } else { len })
    };
    let photos = visual.iter().filter(|a| a.kind == ClipKind::Image).count();
    let mut photo = o.photo_secs.max(MIN_PHOTO);
    if let (true, Some(m)) = (o.fit_music && photos > 0, o.music.and_then(|m| p.asset(m))) {
        let clips: f64 = visual.iter().filter(|a| a.kind != ClipKind::Image).map(|a| clip_len(a).1).sum();
        let want = m.range.map_or(m.duration, |(s, e)| e - s);
        if want > clips {
            photo = ((want - clips) / photos as f64).max(MIN_PHOTO);
        }
    }
    visual
        .iter()
        .map(|a| {
            if a.kind == ClipKind::Image {
                (a.id, (0.0, photo))
            } else {
                let (s, len) = clip_len(a);
                (a.id, (s, s + len))
            }
        })
        .collect()
}

impl Project {
    /// Lay `assets` (photos, clips, and an optional music asset in `o.music`) out as a montage from
    /// `o.at`. Returns every clip it made (video, their sound, the music). Nothing to place = empty.
    pub fn make_montage(&mut self, assets: &[Id], o: &MontageOpts) -> Vec<Id> {
        let plan = montage_plan(self, assets, o);
        if plan.is_empty() {
            return Vec::new();
        }
        let total: f64 = plan.iter().map(|(_, (s, e))| e - s).sum();
        // one free video track for the whole run, so every cut is between neighbours on one track
        let ti = self.find_free_track(TrackKind::Video, o.at, total, None);
        let mut made = Vec::new();
        let mut t = o.at;
        let mut prev_len = f64::INFINITY;
        for (i, (aid, (s, e))) in plan.iter().enumerate() {
            let len = e - s;
            let image = self.asset(*aid).is_some_and(|a| a.kind == ClipKind::Image);
            let ids = self.insert_asset_clips_ranged(*aid, t, Some(ti), None, Some((*s, *e)));
            let video = ids.iter().copied().find(|&id| self.clip(id).is_some_and(|c| c.kind != ClipKind::Audio));
            if let Some(v) = video {
                if image && o.ken_burns {
                    if let Some(c) = self.clip_mut(v) {
                        ken_burns(c, i);
                    }
                }
                // a dissolve never longer than a third of either neighbour
                let tr = o.transition.min(len / 3.0).min(prev_len / 3.0);
                if i > 0 && tr >= MIN_CLIP {
                    self.add_transition(v, TransitionKind::CrossFade, tr);
                }
            }
            made.extend(ids);
            prev_len = len;
            t += len;
        }
        if let Some(m) = o.music.and_then(|m| self.asset(m).cloned()) {
            let (s, e) = m.range.unwrap_or((0.0, m.duration));
            let end = (s + total).min(e).max(s + MIN_CLIP);
            let ids = self.insert_asset_clips_ranged(m.id, o.at, None, None, Some((s, end)));
            for &id in &ids {
                if let Some(c) = self.clip_mut(id) {
                    c.fade_out = (c.duration / 4.0).min(2.0);
                }
            }
            made.extend(ids);
        }
        self.tidy();
        made
    }
}

/// A slow zoom (in on even photos, out on odd ones) with a slight drift sideways, keyed over the clip.
fn ken_burns(c: &mut Clip, i: usize) {
    let (z0, z1) = if i % 2 == 0 { (1.0, 1.12) } else { (1.12, 1.0) };
    let dx = if i % 4 < 2 { 30.0 } else { -30.0 };
    let d = c.duration;
    let key = |a: &mut Animated, t: f64, v: f64| {
        if !a.is_animated() {
            a.toggle_key(t);
        }
        a.key_at(t, v);
    };
    key(&mut c.scale, 0.0, z0);
    key(&mut c.scale, d, z1);
    key(&mut c.x, 0.0, -dx / 2.0);
    key(&mut c.x, d, dx / 2.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(p: &mut Project, kind: &str, dur: f64, audio: bool) -> Id {
        let a: Asset = serde_json::from_value(serde_json::json!({
            "id": 0, "path": format!("C:/m/{kind}{}.x", p.assets.len()), "kind": kind, "duration": dur,
            "width": 1920, "height": 1080, "fps": 30.0,
            "audio_streams": if audio { serde_json::json!([{"index": 0, "channels": 2, "sample_rate": 48000}]) } else { serde_json::json!([]) },
        }))
        .unwrap();
        p.add_asset(a)
    }

    #[test]
    fn montage_lays_photos_and_clips_end_to_end_with_dissolves() {
        let mut p = Project::new();
        let a = asset(&mut p, "Image", 0.0, false);
        let b = asset(&mut p, "Video", 10.0, true);
        let c = asset(&mut p, "Image", 0.0, false);
        let o = MontageOpts { clip_secs: 3.0, ..Default::default() };
        let made = p.make_montage(&[a, b, c], &o);
        let video: Vec<&Clip> = p.all_clips().map(|(_, c)| c).filter(|c| c.kind != ClipKind::Audio).collect();
        assert_eq!(video.len(), 3);
        let mut starts: Vec<(f64, f64)> = video.iter().map(|c| (c.start, c.duration)).collect();
        starts.sort_by(|x, y| x.0.total_cmp(&y.0));
        assert_eq!(starts, [(0.0, 4.0), (4.0, 3.0), (7.0, 4.0)], "photos 4 s, the clip capped at 3 s");
        let one_track = p.track_of(video[0].id);
        assert!(video.iter().all(|c| p.track_of(c.id) == one_track), "one track, so every cut can dissolve");
        let transitions: usize = p.tracks.iter().map(|t| t.transitions.len()).sum();
        assert_eq!(transitions, 2);
        assert!(video.iter().filter(|c| c.kind == ClipKind::Image).all(|c| c.scale.is_animated()), "pan and zoom");
        assert!(made.len() >= 4, "the clip's sound comes along: {made:?}");
    }

    #[test]
    fn photos_stretch_to_fit_the_music() {
        let mut p = Project::new();
        let photos: Vec<Id> = (0..4).map(|_| asset(&mut p, "Image", 0.0, false)).collect();
        let clip = asset(&mut p, "Video", 6.0, false);
        let song = asset(&mut p, "Audio", 30.0, true);
        let mut ids = photos.clone();
        ids.push(clip);
        let o = MontageOpts { music: Some(song), ..Default::default() };
        let plan = montage_plan(&p, &ids, &o);
        let total: f64 = plan.iter().map(|(_, (s, e))| e - s).sum();
        assert!((total - 30.0).abs() < 1e-9, "4 photos share the 24 s the 6 s clip leaves: {total}");
        p.make_montage(&ids, &o);
        let music = p.all_clips().map(|(_, c)| c).find(|c| c.asset == song).unwrap();
        assert!((music.duration - 30.0).abs() < 1e-9 && music.fade_out > 0.0);
        // without fitting, photos keep their own length and the music is cut to the montage
        let o = MontageOpts { fit_music: false, ..o };
        let total: f64 = montage_plan(&p, &ids, &o).iter().map(|(_, (s, e))| e - s).sum();
        assert!((total - 22.0).abs() < 1e-9);
    }

    #[test]
    fn nothing_visual_makes_nothing() {
        let mut p = Project::new();
        let song = asset(&mut p, "Audio", 30.0, true);
        assert!(p.make_montage(&[song], &MontageOpts { music: Some(song), ..Default::default() }).is_empty());
    }
}
