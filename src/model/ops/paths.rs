use crate::model::*;

impl Project {
    // ---------- reusable paths ----------
    pub fn path(&self, id: Id) -> Option<&PathAsset> {
        self.paths.iter().find(|p| p.id == id)
    }
    /// Keep a path on the project. An empty name gets a numbered one.
    pub fn add_path(&mut self, name: String, points: Vec<(f32, f32, f32)>) -> Id {
        let id = self.new_id();
        let name = if name.trim().is_empty() { format!("Path {}", self.paths.len() + 1) } else { name };
        self.paths.push(PathAsset { id, name, points });
        id
    }
    /// A clip's drawing / polygon outline in canvas coordinates - the clip's own position is folded in,
    /// so the path lands where the sketch is on screen.
    pub fn path_from_clip(&self, clip: Id) -> Vec<(f32, f32, f32)> {
        let Some(c) = self.clip(clip) else { return Vec::new() };
        let (ox, oy) = (c.x.value as f32, c.y.value as f32);
        let Some(s) = &c.shape else { return Vec::new() };
        s.path_points().into_iter().map(|(x, y, t)| (x + ox, y + oy, t)).collect()
    }
    /// Drive a clip's X/Y along a path over the clip's own length. False when the path is too short to
    /// animate anything.
    pub fn apply_path(&mut self, clip: Id, points: &[(f32, f32, f32)]) -> bool {
        let Some(dur) = self.clip(clip).map(|c| c.duration) else { return false };
        let (x, y) = path_to_keys(points, dur);
        if x.keys.len() < 2 {
            return false;
        }
        let Some(c) = self.clip_mut(clip) else { return false };
        c.x = x;
        c.y = y;
        true
    }
    /// Live-link a clip's X/Y to a saved path: editing the path afterwards moves the clip too
    /// (unlike `apply_path`, which bakes a one-shot copy).
    pub fn link_path(&mut self, clip: Id, path: Id) -> bool {
        if self.path(path).is_none() {
            return false;
        }
        let Some(c) = self.clip_mut(clip) else { return false };
        c.x.unlink();
        c.y.unlink();
        c.x.link = AnimLink::PathX(path);
        c.y.link = AnimLink::PathY(path);
        true
    }
    /// Native (unscaled) pixel size of a clip's own footage: the asset's size for video/image, the
    /// nested sequence's size for a `Sequence` clip. `None` for kinds with no meaningful size (text,
    /// shape, adjustment, audio) or a footage clip missing its asset/sequence - mirrors
    /// `GpuRenderer::native_size`, minus its decoded-frame fallback.
    pub fn clip_native_size(&self, clip: &Clip) -> Option<(u32, u32)> {
        let wh = match clip.kind {
            ClipKind::Video | ClipKind::Image => self.asset(clip.asset).map(|a| (a.width, a.height)),
            ClipKind::Sequence => self.sequence(clip.sequence).map(|s| (s.width, s.height)),
            _ => None,
        };
        match wh {
            Some((w, h)) if w > 0 && h > 0 => Some((w, h)),
            _ => None,
        }
    }
    /// Reset a clip's transform to fill the project canvas (any keyframes on x/y/scale/rotation are
    /// wholesale-replaced - this is a reset, not a tween). `stretch = false` ("Fit to Screen") resets
    /// x/y/scale/rotation/scale_x/scale_y to their defaults, which falls back to the engine's default
    /// "contain" placement (letterboxed, native aspect preserved, centred). `stretch = true` ("Stretch to Screen")
    /// additionally sets independent scale_x/scale_y so the footage fills the canvas edge to edge,
    /// ignoring native aspect ratio. No-op (`false`) for a clip with no native size.
    pub fn fit_clip_to_screen(&mut self, id: Id, stretch: bool) -> bool {
        let Some((nw, nh)) = self.clip(id).and_then(|c| self.clip_native_size(c)) else { return false };
        let (sx, sy) = if stretch {
            let canvas_aspect = self.width as f64 / self.height as f64;
            let native_aspect = nw as f64 / nh as f64;
            if native_aspect >= canvas_aspect {
                (1.0, native_aspect / canvas_aspect)
            } else {
                (canvas_aspect / native_aspect, 1.0)
            }
        } else {
            (1.0, 1.0)
        };
        let Some(clip) = self.clip_mut(id) else { return false };
        clip.x = a0();
        clip.y = a0();
        clip.scale = a1();
        clip.rotation = a0(); // a rotated quad can't fill the canvas - "to screen" implies upright
        clip.scale_x = Animated::new(sx);
        clip.scale_y = Animated::new(sy);
        true
    }
    /// Re-bake every live link (path / expression) whose inputs changed. Called once per frame;
    /// costs one hash per linked property when nothing changed.
    pub fn refresh_links(&mut self) {
        let paths = self.paths.clone(); // ponytail: cloned to split the borrow; index it if paths grow huge
        // ws:animator: a layer link reads another clip, so its samples are made first, from a shared view
        let layer = self.layer_link_samples();
        let mut tracks: Vec<&mut Track> = self.tracks.iter_mut().collect();
        if let Some(st) = &mut self.main_stash {
            tracks.extend(st.tracks.iter_mut());
        }
        for sq in &mut self.sequences {
            tracks.extend(sq.tracks.iter_mut());
        }
        for tr in tracks {
            for c in &mut tr.clips {
                let dur = c.duration;
                let cid = c.id;
                for (ai, a) in c.all_animated_mut().into_iter().enumerate() {
                    if a.link.is_none() {
                        continue;
                    }
                    if matches!(a.link, AnimLink::Layer { .. }) {
                        if let Some((_, _, rev, res)) = layer.iter().find(|l| l.0 == cid && l.1 == ai) {
                            a.baked_rev = *rev;
                            match res {
                                Ok(k) => (a.baked, a.link_err) = (k.clone(), None),
                                Err(e) => (a.baked, a.link_err) = (Vec::new(), Some(e.clone())),
                            }
                        }
                        continue;
                    }
                    let rev = link_rev(a, dur, &paths);
                    if rev == a.baked_rev {
                        continue;
                    }
                    a.baked_rev = rev;
                    a.link_err = None;
                    let res = match &a.link {
                        AnimLink::None => unreachable!(),
                        AnimLink::PathX(id) | AnimLink::PathY(id) => match paths.iter().find(|p| p.id == *id) {
                            Some(p) => {
                                let (x, y) = path_to_keys(&p.points, dur);
                                Ok(if matches!(a.link, AnimLink::PathX(_)) { x.keys } else { y.keys })
                            }
                            None => Err("path no longer exists".to_string()),
                        },
                        AnimLink::Expr(src) => bake_expr(src, a, dur),
                        AnimLink::PathAngle(id) => match paths.iter().find(|p| p.id == *id) {
                            Some(p) => Ok(path_angle_keys(&p.points, dur)),
                            None => Err("path no longer exists".to_string()),
                        },
                        AnimLink::Layer { .. } => unreachable!(),
                    };
                    match res {
                        Ok(keys) => a.baked = keys,
                        Err(e) => {
                            a.baked.clear();
                            a.link_err = Some(e);
                        }
                    }
                }
            }
        }
    }
}

impl Project {
    /// ws:animator: (clip, index into `Clip::animated`, stamp, samples) for every layer link on the main
    /// timeline whose stamp changed. A link to the clip itself or to something gone reports an error.
    fn layer_link_samples(&self) -> Vec<(Id, usize, u64, Result<Vec<Keyframe>, String>)> {
        use std::hash::{Hash, Hasher};
        let mut out = Vec::new();
        for (_, c) in self.all_clips() {
            for (ai, a) in c.animated().iter().enumerate() {
                let AnimLink::Layer { clip, prop, mul, add } = &a.link else { continue };
                let src = self.clip(*clip).filter(|s| s.id != c.id);
                let sa = src.and_then(|s| prop_ref(s, prop).map(|p| (s, p)));
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (clip, prop, mul.to_bits(), add.to_bits(), c.start.to_bits(), c.duration.to_bits()).hash(&mut h);
                if let Some((s, p)) = sa {
                    (s.start.to_bits(), p.value.to_bits(), p.baked_rev).hash(&mut h);
                    for k in &p.keys {
                        (k.t.to_bits(), k.v.to_bits(), format!("{:?}", k.ease)).hash(&mut h);
                    }
                }
                let rev = h.finish().max(1);
                if rev == a.baked_rev {
                    continue;
                }
                let res = match sa {
                    Some((s, p)) => {
                        // ponytail: 30 samples a second like an expression; follow the keys exactly if that shows
                        let n = ((c.duration * 30.0).ceil() as usize).clamp(1, 4096) + 1;
                        Ok((0..n)
                            .map(|i| {
                                let t = c.duration * i as f64 / (n - 1) as f64;
                                Keyframe { t, v: p.at(c.start + t - s.start) * mul + add, ease: Ease::Linear }
                            })
                            .collect())
                    }
                    None => Err("the linked clip or property is gone".to_string()),
                };
                out.push((c.id, ai, rev, res));
            }
        }
        out
    }
}

/// Rotation (degrees, unwrapped) facing along a path over `duration` - "turn along the path".
pub fn path_angle_keys(points: &[(f32, f32, f32)], duration: f64) -> Vec<Keyframe> {
    let (x, y) = path_to_keys(points, duration);
    let n = ((duration * 30.0).ceil() as usize).clamp(2, 4096);
    let e = duration / n as f64 * 0.5;
    let mut last: Option<f64> = None;
    (0..=n)
        .map(|i| {
            let t = duration * i as f64 / n as f64;
            let (t0, t1) = ((t - e).max(0.0), (t + e).min(duration));
            let (dx, dy) = (x.at(t1) - x.at(t0), y.at(t1) - y.at(t0));
            let mut a = if dx.abs() + dy.abs() > 1e-9 { dy.atan2(dx).to_degrees() } else { last.unwrap_or(0.0) };
            if let Some(l) = last {
                while a - l > 180.0 {
                    a -= 360.0;
                }
                while a - l < -180.0 {
                    a += 360.0;
                }
            }
            last = Some(a);
            Keyframe { t, v: a, ease: Ease::Linear }
        })
        .collect()
}
