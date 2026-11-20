//! ws:forum-ui - the one-step answers to the jobs users otherwise ask the forums about
//! (plans/shotcut-forum-insights.md, Session A): duplicate a track, split-screen layouts, and Censor
//! (blur an area over a range, or mute the sound over it). Each is one `Project` edit, so the caller's
//! undo snapshot makes it one undo step.

use crate::model::*;

/// A split-screen arrangement of the selected visual clips (`Project::apply_layout`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Two side by side.
    TwoUp,
    /// Three side by side.
    ThreeUp,
    /// 2 × 2 grid.
    Grid2x2,
    /// 3 columns × 2 rows.
    Grid3x2,
    /// The first clip fills the frame; the second sits small in the bottom-right corner.
    Pip,
}

impl Layout {
    pub const ALL: [Layout; 5] = [Layout::TwoUp, Layout::ThreeUp, Layout::Grid2x2, Layout::Grid3x2, Layout::Pip];
    pub fn name(self) -> &'static str {
        match self {
            Layout::TwoUp => "Side by Side",
            Layout::ThreeUp => "Three Across",
            Layout::Grid2x2 => "2 × 2 Grid",
            Layout::Grid3x2 => "3 × 2 Grid",
            Layout::Pip => "Picture in Picture",
        }
    }
    /// Stable id for MCP / settings.
    pub fn id(self) -> &'static str {
        match self {
            Layout::TwoUp => "2up",
            Layout::ThreeUp => "3up",
            Layout::Grid2x2 => "2x2",
            Layout::Grid3x2 => "3x2",
            Layout::Pip => "pip",
        }
    }
    pub fn from_id(s: &str) -> Option<Layout> {
        Self::ALL.into_iter().find(|l| l.id() == s)
    }
    /// What it does (the tooltip).
    pub fn hint(self) -> &'static str {
        match self {
            Layout::TwoUp => "The first two selected clips, side by side, each cropped to fill its half",
            Layout::ThreeUp => "The first three selected clips in three columns",
            Layout::Grid2x2 => "The first four selected clips in a 2 × 2 grid",
            Layout::Grid3x2 => "The first six selected clips in three columns and two rows",
            Layout::Pip => "The first selected clip fills the frame, the second sits small in the corner",
        }
    }
    /// The cells, as (left, top, width, height) in project pixels of a `w` × `h` frame, in fill order.
    pub fn cells(self, w: f64, h: f64) -> Vec<(f64, f64, f64, f64)> {
        let grid = |cols: usize, rows: usize| {
            let (cw, ch) = (w / cols as f64, h / rows as f64);
            (0..rows).flat_map(move |r| (0..cols).map(move |c| (c as f64 * cw, r as f64 * ch, cw, ch))).collect()
        };
        match self {
            Layout::TwoUp => grid(2, 1),
            Layout::ThreeUp => grid(3, 1),
            Layout::Grid2x2 => grid(2, 2),
            Layout::Grid3x2 => grid(3, 2),
            Layout::Pip => {
                let (pw, ph) = (w / 3.0, h / 3.0);
                let m = h * 0.04;
                vec![(0.0, 0.0, w, h), (w - pw - m, h - ph - m, pw, ph)]
            }
        }
    }
}

/// The transform that makes a layer of `native` size (fitted inside the `w` × `h` frame first, as the
/// compositor does for video and images) fill the cell `(left, top, cw, ch)`: scaled to cover it and
/// cropped back to it, centred. Returns (x, y, scale, crop left/right, crop top/bottom).
pub fn fill_cell(native: (f64, f64), w: f64, h: f64, cell: (f64, f64, f64, f64)) -> (f64, f64, f64, f64, f64) {
    let (nw, nh) = (native.0.max(1.0), native.1.max(1.0));
    let fit = (w / nw).min(h / nh);
    let (fw, fh) = (nw * fit, nh * fit);
    let (left, top, cw, ch) = cell;
    let s = (cw / fw).max(ch / fh);
    let (rw, rh) = (fw * s, fh * s);
    let crop_x = ((rw - cw) / (2.0 * rw)).clamp(0.0, 0.49);
    let crop_y = ((rh - ch) / (2.0 * rh)).clamp(0.0, 0.49);
    (left + cw / 2.0 - w / 2.0, top + ch / 2.0 - h / 2.0, s, crop_x, crop_y)
}

/// What `Project::censor` hides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Censor {
    /// An adjustment layer over the range with a strong blur inside a movable box.
    Blur,
    /// Silence the sound of the clips over the range (volume keys dip to 0 and back).
    Mute,
}

/// Seconds the volume takes to dip in and out around a muted range (short enough to sound like a cut).
const MUTE_RAMP: f64 = 0.02;

impl Project {
    /// Duplicate track `ti` (its clips, transitions and settings) right next to it. The copies get new
    /// ids and lose links to clips on other tracks. Returns the new track's index.
    pub fn duplicate_track(&mut self, ti: usize) -> Option<usize> {
        let mut t = self.tracks.get(ti)?.clone();
        t.id = self.new_id();
        t.name = format!("{} copy", t.name);
        t.solo = false;
        let mut remap: Vec<(Id, Id)> = Vec::with_capacity(t.clips.len());
        for c in &mut t.clips {
            let n = self.new_id();
            remap.push((c.id, n));
            c.id = n;
            c.link = 0;
        }
        let new_of = |id: Id| remap.iter().find(|(o, _)| *o == id).map(|&(_, n)| n);
        t.transitions.retain(|tr| new_of(tr.right).is_some());
        for tr in &mut t.transitions {
            tr.right = new_of(tr.right).unwrap_or(tr.right);
        }
        for tr in &mut t.transitions {
            tr.id = self.new_id();
        }
        // video displays in reversed index order (V2 above V1): ti + 1 is just above for video and just
        // below for audio, and keeps both kinds contiguous either way
        self.tracks.insert(ti + 1, t);
        Some(ti + 1)
    }

    /// Arrange the visual clips `ids` (in that order) into `layout`'s cells: each is moved, scaled to cover
    /// its cell and cropped to it. Their position, scale and crop keyframes are replaced. Clips beyond the
    /// layout's cell count are left alone. False when no clip moved.
    pub fn apply_layout(&mut self, ids: &[Id], layout: Layout) -> bool {
        let (w, h) = (self.width as f64, self.height as f64);
        let cells = layout.cells(w, h);
        let mut changed = false;
        let mut i = 0;
        for &id in ids {
            if i >= cells.len() {
                break;
            }
            let Some(c) = self.clip(id) else { continue };
            if !c.draws() {
                continue;
            }
            // text and shapes are drawn at frame scale: treat them as frame-sized
            let native = match c.kind {
                ClipKind::Video | ClipKind::Image => self
                    .asset(c.asset)
                    .filter(|a| a.width > 0 && a.height > 0)
                    .map_or((w, h), |a| (a.width as f64, a.height as f64)),
                _ => (w, h),
            };
            let (x, y, s, cx, cy) = fill_cell(native, w, h, cells[i]);
            let c = self.clip_mut(id).unwrap();
            c.x = Animated::new(x);
            c.y = Animated::new(y);
            c.scale = Animated::new(s);
            c.scale_x = Animated::new(1.0);
            c.scale_y = Animated::new(1.0);
            c.rotation = Animated::new(0.0);
            c.crop_left = Animated::new(cx);
            c.crop_right = Animated::new(cx);
            c.crop_top = Animated::new(cy);
            c.crop_bottom = Animated::new(cy);
            changed = true;
            i += 1;
        }
        changed
    }

    /// Point asset `id` (and the subclips sharing its file) at the file `path`. Every clip using it keeps
    /// its cuts, effects and keyframes. False when the asset is missing or already there.
    pub fn replace_asset_path(&mut self, id: Id, path: &str) -> bool {
        let Some(old) = self.asset(id).map(|a| a.path.clone()) else { return false };
        if old == path || path.is_empty() {
            return false;
        }
        for a in self.assets.iter_mut().filter(|a| a.path == old) {
            a.path = path.to_string();
        }
        true
    }

    /// The range Censor acts on: In→Out when both are marked, else the span of the clips `ids`.
    pub fn censor_range(&self, ids: &[Id]) -> Option<(f64, f64)> {
        if let (Some(a), Some(b)) = (self.in_point, self.out_point) {
            if b > a {
                return Some((a, b));
            }
        }
        let clips = ids.iter().filter_map(|&id| self.clip(id));
        let (t0, t1) = clips.fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c.start), b.max(c.end())));
        (t1 > t0).then_some((t0, t1))
    }

    /// Hide something over timeline `[t0, t1)`. `Blur`: an adjustment layer over the range on the top
    /// video track that has room, blurring inside a box in the middle of the frame (move or resize the
    /// box in the Preview; returns the new clip's id). `Mute`: the clips `ids` that make sound (every
    /// audio clip on the range when `ids` is empty) dip to silence over the range. False/None when
    /// nothing changed.
    pub fn censor(&mut self, what: Censor, ids: &[Id], t0: f64, t1: f64) -> Option<Id> {
        let (t0, t1) = (t0.min(t1), t0.max(t1));
        if t1 - t0 < MIN_CLIP {
            return None;
        }
        match what {
            Censor::Blur => {
                let id = self.add_adjustment_clip(t0, t1 - t0);
                let (w, h) = (self.width as f64, self.height as f64);
                let c = self.clip_mut(id)?;
                c.name = "Censor".into();
                let mut fx = Effect::new(EffectKind::Blur);
                if let Some(r) = fx.params.first_mut() {
                    *r = Animated::new(40.0);
                }
                c.effects.push(fx);
                let mut m = Mask::new(MaskShape::Rect);
                m.rx = Animated::new(w / 8.0);
                m.ry = Animated::new(h / 8.0);
                m.feather = Animated::new(6.0);
                c.mask = Some(m);
                Some(id)
            }
            Censor::Mute => {
                let targets: Vec<Id> = if ids.is_empty() {
                    self.tracks
                        .iter()
                        .flat_map(|t| t.clips.iter())
                        .filter(|c| c.kind == ClipKind::Audio && c.start < t1 && c.end() > t0)
                        .map(|c| c.id)
                        .collect()
                } else {
                    ids.to_vec()
                };
                let mut any = None;
                for id in targets {
                    let Some(c) = self.clip_mut(id) else { continue };
                    if (c.kind != ClipKind::Audio && c.kind != ClipKind::Video) || c.start >= t1 || c.end() <= t0 {
                        continue;
                    }
                    let (l0, l1) = (c.local(t0).max(0.0), c.local(t1).min(c.duration));
                    mute_span(&mut c.volume, l0, l1);
                    any = Some(id);
                }
                any
            }
        }
    }
}

/// Dip `vol` to 0 over clip-local `[l0, l1]`, keeping its value on both sides (keys inside are replaced).
fn mute_span(vol: &mut Animated, l0: f64, l1: f64) {
    let before = vol.at(l0 - MUTE_RAMP);
    let after = vol.at(l1 + MUTE_RAMP);
    vol.link = AnimLink::None;
    vol.keys.retain(|k| k.t < l0 - MUTE_RAMP - 1e-6 || k.t > l1 + MUTE_RAMP + 1e-6);
    if l0 > MUTE_RAMP {
        vol.key_at(l0 - MUTE_RAMP, before);
    }
    vol.key_at(l0, 0.0);
    vol.key_at(l1, 0.0);
    vol.key_at(l1 + MUTE_RAMP, after);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Project {
        let mut p = Project::default();
        p.width = 1920;
        p.height = 1080;
        p
    }

    fn video(p: &mut Project, ti: usize, start: f64, dur: f64) -> Id {
        let c = Clip::new(p.new_id(), ClipKind::Video, "v", start, dur);
        let id = c.id;
        p.tracks[ti].clips.push(c);
        id
    }

    #[test]
    fn duplicate_track_copies_clips_with_new_ids_next_to_it() {
        let mut p = project();
        let v = p.add_track(TrackKind::Video);
        let a = video(&mut p, v, 0.0, 2.0);
        p.tracks[v].muted = true;
        let n = p.tracks.len();
        let d = p.duplicate_track(v).unwrap();
        assert_eq!(p.tracks.len(), n + 1);
        assert_eq!(d, v + 1);
        assert_eq!(p.tracks[d].kind, TrackKind::Video);
        assert!(p.tracks[d].muted, "settings come along");
        assert_eq!(p.tracks[d].clips.len(), 1);
        assert_ne!(p.tracks[d].clips[0].id, a, "the copy has its own id");
        assert_ne!(p.tracks[d].id, p.tracks[v].id);
        assert!(p.tracks[d].name.ends_with("copy"));
        // kinds stay contiguous
        let kinds: Vec<_> = p.tracks.iter().map(|t| t.kind).collect();
        let first_audio = kinds.iter().position(|&k| k == TrackKind::Audio).unwrap_or(kinds.len());
        assert!(kinds[..first_audio].iter().all(|&k| k == TrackKind::Video));
    }

    #[test]
    fn duplicate_track_out_of_range_is_none() {
        let mut p = project();
        assert!(p.duplicate_track(99).is_none());
    }

    #[test]
    fn layout_cells_tile_the_frame() {
        for l in Layout::ALL {
            let cells = l.cells(1920.0, 1080.0);
            assert!(!cells.is_empty());
            for (x, y, w, h) in cells {
                assert!(x >= 0.0 && y >= 0.0 && x + w <= 1920.0 + 1e-6 && y + h <= 1080.0 + 1e-6, "{l:?}");
            }
            assert_eq!(Layout::from_id(l.id()), Some(l));
        }
        assert_eq!(Layout::Grid3x2.cells(1920.0, 1080.0).len(), 6);
    }

    #[test]
    fn fill_cell_covers_the_half_and_crops_the_rest() {
        // a 16:9 clip into the left half of a 16:9 frame: scale 1 (height fits), half the width cropped off
        let (x, y, s, cx, cy) = fill_cell((1920.0, 1080.0), 1920.0, 1080.0, (0.0, 0.0, 960.0, 1080.0));
        assert!((s - 1.0).abs() < 1e-9);
        assert!((x + 480.0).abs() < 1e-9 && y.abs() < 1e-9);
        assert!((cx - 0.25).abs() < 1e-9 && cy.abs() < 1e-9);
        // visible width after crop = the cell's width
        assert!((1920.0 * s * (1.0 - 2.0 * cx) - 960.0).abs() < 1e-6);
        // a quarter cell: scale 0.5, nothing cropped
        let (_, _, s, cx, cy) = fill_cell((1920.0, 1080.0), 1920.0, 1080.0, (960.0, 540.0, 960.0, 540.0));
        assert!((s - 0.5).abs() < 1e-9 && cx.abs() < 1e-9 && cy.abs() < 1e-9);
    }

    #[test]
    fn apply_layout_places_clips_in_order() {
        let mut p = project();
        let v1 = p.add_track(TrackKind::Video);
        let v2 = p.add_track(TrackKind::Video);
        let a = video(&mut p, v1, 0.0, 2.0);
        let b = video(&mut p, v2, 0.0, 2.0);
        assert!(p.apply_layout(&[a, b], Layout::TwoUp));
        assert!(p.clip(a).unwrap().x.value < 0.0, "first clip on the left");
        assert!(p.clip(b).unwrap().x.value > 0.0, "second clip on the right");
        assert!(!p.apply_layout(&[], Layout::TwoUp));
    }

    #[test]
    fn censor_blur_adds_a_masked_blur_layer_over_the_range() {
        let mut p = project();
        let v = p.add_track(TrackKind::Video);
        video(&mut p, v, 0.0, 10.0);
        let id = p.censor(Censor::Blur, &[], 2.0, 4.0).unwrap();
        let c = p.clip(id).unwrap();
        assert_eq!(c.kind, ClipKind::Adjustment);
        assert!((c.start - 2.0).abs() < 1e-9 && (c.duration - 2.0).abs() < 1e-9);
        assert_eq!(c.effects[0].kind, EffectKind::Blur);
        assert!(c.mask.is_some());
        assert!(p.censor(Censor::Blur, &[], 2.0, 2.0).is_none(), "an empty range does nothing");
        p.in_point = Some(1.0);
        p.out_point = Some(3.0);
        assert_eq!(p.censor_range(&[]), Some((1.0, 3.0)), "In/Out wins");
        p.out_point = None;
        assert_eq!(p.censor_range(&[id]), Some((2.0, 4.0)), "else the selection's span");
    }

    #[test]
    fn replace_asset_path_repoints_the_file_and_its_subclips() {
        let mut p = project();
        // the JSON form fills every other field with its default
        let mut mk = |path: &str| {
            let id = p.new_id();
            let a: Asset = serde_json::from_value(serde_json::json!({"id": id, "path": path, "kind": "Video", "duration": 10.0, "width": 1280, "height": 720, "fps": 30.0})).unwrap();
            p.assets.push(a);
            id
        };
        let (a, sub, other) = (mk("C:/old.mp4"), mk("C:/old.mp4"), mk("C:/other.mp4"));
        assert!(p.replace_asset_path(a, "C:/new.mp4"));
        assert_eq!(p.asset(a).unwrap().path, "C:/new.mp4");
        assert_eq!(p.asset(sub).unwrap().path, "C:/new.mp4", "the subclip follows its file");
        assert_eq!(p.asset(other).unwrap().path, "C:/other.mp4");
        assert!(!p.replace_asset_path(a, "C:/new.mp4"), "already there");
        assert!(!p.replace_asset_path(999, "C:/x.mp4"));
    }

    #[test]
    fn censor_mute_silences_only_the_range() {
        let mut p = project();
        let at = p.add_track(TrackKind::Audio);
        let c = Clip::new(p.new_id(), ClipKind::Audio, "a", 0.0, 10.0);
        let id = c.id;
        p.tracks[at].clips.push(c);
        assert_eq!(p.censor(Censor::Mute, &[], 3.0, 5.0), Some(id));
        let vol = &p.clip(id).unwrap().volume;
        assert!(vol.at(4.0).abs() < 1e-9, "silent inside");
        assert!((vol.at(1.0) - 1.0).abs() < 1e-9, "full before");
        assert!((vol.at(8.0) - 1.0).abs() < 1e-9, "full after");
    }
}
