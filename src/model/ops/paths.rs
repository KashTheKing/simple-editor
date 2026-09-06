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
    /// A clip's drawing / polygon outline in canvas coordinates — the clip's own position is folded in,
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
    /// shape, adjustment, audio) or a footage clip missing its asset/sequence — mirrors
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
    /// wholesale-replaced — this is a reset, not a tween). `stretch = false` ("Fit to Screen") resets
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
        clip.rotation = a0(); // a rotated quad can't fill the canvas — "to screen" implies upright
        clip.scale_x = Animated::new(sx);
        clip.scale_y = Animated::new(sy);
        true
    }
    /// Re-bake every live link (path / expression) whose inputs changed. Called once per frame;
    /// costs one hash per linked property when nothing changed.
    pub fn refresh_links(&mut self) {
        let paths = self.paths.clone(); // ponytail: cloned to split the borrow; index it if paths grow huge
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
                for a in c.all_animated_mut() {
                    if a.link.is_none() {
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
