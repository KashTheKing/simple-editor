//! ---- ws:browser-layer ---- edit ops for browser clips and their fake mouse (`model::browser`).
use crate::model::*;

/// How the old page gives way to the next one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PageTransition {
    #[default]
    Cut,
    Fade,
    /// The next page pushes the old one out to the left.
    Slide,
    /// A fade while the view pushes in and settles back.
    Zoom,
}

impl PageTransition {
    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "cut" | "none" => Self::Cut,
            "fade" => Self::Fade,
            "slide" => Self::Slide,
            "zoom" => Self::Zoom,
            _ => return None,
        })
    }
}

/// What "go to another page" changes besides cutting the page clips.
#[derive(Clone, Debug, Default)]
pub struct NextPage<'a> {
    /// Library asset that becomes the picture after the cut.
    pub asset: Option<Id>,
    pub url: &'a str,
    pub title: &'a str,
    pub favicon: &'a str,
    /// Load bar seconds.
    pub load: f64,
    pub transition: PageTransition,
    /// Transition seconds.
    pub secs: f64,
    /// The address types itself out (over the load time) instead of changing at once.
    pub type_url: bool,
}

/// Name of the clips `browser_add_detail` makes (and `browser_clear_details` removes).
pub const DETAIL: &str = "Zoom detail";
/// Name of a hover highlight clip.
pub const HIGHLIGHT: &str = "Hover highlight";

/// How a set of cursor waypoints is joined.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CursorMove {
    /// Stop on every point (ease in and out of each move).
    Stops,
    /// Glide through the points on one curve.
    Glide,
}

impl Project {
    fn browser_mut(&mut self, id: Id) -> Option<&mut BrowserStyle> {
        self.clip_mut(id).and_then(|c| c.browser.as_mut())
    }

    /// Add a browser window over `at..at+dur`, on a video track above everything in that range, so
    /// what is there shows inside it. The window is sized so the page area has the frame's shape.
    pub fn add_browser_clip(&mut self, at: f64, dur: f64) -> Id {
        let at = at.max(0.0);
        let dur = dur.max(MIN_CLIP);
        let ti = self.track_above_all(at, dur);
        let mut c = Clip::new(self.new_id(), ClipKind::Browser, "Browser", at, dur);
        if let Some(b) = c.browser.as_mut() {
            b.w = (self.width as f32 * 0.84).round();
            b.h = (b.w * self.height as f32 / self.width.max(1) as f32 + b.chrome_h()).round();
            // a tall frame (a Short) gets the phone-shaped window
            if self.height > self.width {
                b.theme = BrowserTheme::Mobile;
                b.corner = 36.0;
                b.h = (self.height as f32 * 0.84).round();
            }
        }
        let id = c.id;
        self.tracks[ti].clips.push(c);
        self.tracks[ti].sort();
        id
    }

    /// The lowest video track that is above everything in `at..at+dur` (a new one when there is none).
    pub(crate) fn track_above_all(&mut self, at: f64, dur: f64) -> usize {
        let busy = self.video_tracks().into_iter().filter(|&ti| !self.tracks[ti].fits(at, dur, &[])).next_back();
        let above = self.video_tracks().into_iter().find(|&ti| busy.is_none_or(|b| ti > b));
        above.unwrap_or_else(|| self.add_track(TrackKind::Video))
    }

    /// Clips on the tracks below browser `id` that overlap it in time.
    pub(crate) fn browser_pages(&self, id: Id) -> Vec<Id> {
        let (Some(ti), Some(b)) = (self.track_of(id), self.clip(id)) else { return Vec::new() };
        let (s, e) = (b.start, b.end());
        self.tracks[..ti]
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .flat_map(|t| t.clips.iter())
            .filter(|c| c.start < e - EPS && s < c.end() - EPS)
            .map(|c| c.id)
            .collect()
    }

    /// Make one clip a page: as wide as the frame, its top at the frame's top. Returns its height in
    /// project px (0 for a clip without a size of its own: text, shapes).
    fn fit_page(&mut self, cid: Id) -> f64 {
        let (pw, ph) = (self.width as f64, self.height as f64);
        let Some((nw, nh)) = self.clip(cid).and_then(|c| self.clip_native_size(c)) else { return 0.0 };
        let (nw, nh) = (nw as f64, nh as f64);
        let fit = (pw / nw).min(ph / nh);
        let k = pw / (nw * fit);
        let h = nh * fit * k;
        if let Some(c) = self.clip_mut(cid) {
            c.scale = Animated::new(k);
            (c.scale_x, c.scale_y, c.rotation) = (a1(), a1(), a0());
            c.x = a0();
            c.y = Animated::new((h - ph) / 2.0);
        }
        h
    }

    /// Make every picture under the browser a page: as wide as the frame, its top at the frame's
    /// top (a tall screenshot then scrolls with the browser's Scroll). Returns how far the tallest
    /// one can scroll, in project px.
    pub fn browser_fit_pages(&mut self, id: Id) -> f64 {
        let pw = self.width as f64;
        let Some(view_h) = self.clip(id).and_then(|c| c.browser.as_ref()).map(|b| b.view_h(pw, self.height as f64)) else {
            return 0.0;
        };
        let mut max_scroll = 0.0f64;
        for cid in self.browser_pages(id) {
            max_scroll = max_scroll.max(self.fit_page(cid) - view_h);
        }
        max_scroll.max(0.0)
    }

    /// Go to another page at `t`: the clips under the browser are cut there (the right halves are
    /// the next page - `next.asset` replaces the picture of the topmost one), with `next.transition`
    /// across the cut; the load bar runs, the address / tab title / tab icon change and the scroll
    /// jumps back to the top. Returns the right halves' ids.
    pub fn browser_page_cut(&mut self, id: Id, t: f64, next: &NextPage) -> Vec<Id> {
        let Some((lt, fit)) = self
            .clip(id)
            .filter(|c| c.contains(t))
            .and_then(|c| c.browser.as_ref().map(|b| (c.local(t), b.auto_fit)))
        else {
            return Vec::new();
        };
        // (highlights and zoom details end where they end: only real pages are cut)
        let pages: Vec<Id> = self
            .browser_pages(id)
            .into_iter()
            .filter(|&c| self.clip(c).is_some_and(|c| c.name != DETAIL && c.name != HIGHLIGHT))
            .collect();
        let new = self.split_at(t, Some(&pages));
        if let Some(&top) = new.last() {
            // the next page is a page too, whatever shape the one before it had
            if next.asset.is_some_and(|a| self.replace_clip(top, a)) && fit {
                self.fit_page(top);
            }
            let secs = next.secs.clamp(0.05, 5.0);
            match next.transition {
                PageTransition::Cut => {}
                PageTransition::Slide => {
                    self.add_transition(top, TransitionKind::Push, secs);
                }
                PageTransition::Fade | PageTransition::Zoom => {
                    self.add_transition(top, TransitionKind::CrossFade, secs);
                }
            }
        }
        let Some(b) = self.browser_mut(id) else { return new };
        // (no load time: no load bar)
        let load = next.load.clamp(0.0, 10.0);
        if load > 0.0 {
            b.run_load(lt, load);
        }
        if !next.url.is_empty() || !next.title.is_empty() || !next.favicon.is_empty() {
            b.go(lt, next.url, next.title);
            if let Some(n) = b.nav.iter_mut().find(|n| (n.t - lt).abs() < 1e-4) {
                n.favicon = next.favicon.to_string();
            }
            if next.type_url {
                b.type_url(lt, load.clamp(0.3, 1.2));
            }
        }
        if next.transition == PageTransition::Zoom && (b.zoom.at(lt) - 1.0).abs() < 1e-6 {
            let h = next.secs.clamp(0.05, 5.0) / 2.0;
            put_key(&mut b.zoom, (lt - h).max(0.0), 1.0, Ease::EaseIn);
            put_key(&mut b.zoom, (lt - 1e-3).max(0.0), 1.12, Ease::Hold);
            put_key(&mut b.zoom, lt, 0.9, Ease::EaseOut);
            put_key(&mut b.zoom, lt + h, 1.0, Ease::Hold);
        }
        if b.scroll.is_animated() || b.scroll.value != 0.0 {
            let now = b.scroll.at((lt - 1e-3).max(0.0));
            put_key(&mut b.scroll, (lt - 1e-3).max(0.0), now, Ease::Hold);
            put_key(&mut b.scroll, lt, 0.0, Ease::Hold);
        }
        new
    }

    /// Where a point of clip `page` (a fraction 0..1 of its picture, from its top-left) is on the
    /// frame at timeline `t`, in project px from the frame's middle, before any browser's scroll and
    /// zoom. None for a clip without a size of its own.
    pub fn page_to_frame(&self, page: Id, t: f64, f: (f64, f64)) -> Option<(f64, f64)> {
        let c = self.clip(page)?;
        let (nw, nh) = self.clip_native_size(c)?;
        let l = c.local(t);
        let fit = (self.width as f64 / nw as f64).min(self.height as f64 / nh as f64) * c.scale.at(l);
        let (w, h) = (nw as f64 * fit * c.scale_x.at(l), nh as f64 * fit * c.scale_y.at(l));
        Some((c.x.at(l) + (f.0 - 0.5) * w, c.y.at(l) + (f.1 - 0.5) * h))
    }

    /// The fraction of clip `page`'s picture at a frame point: the inverse of `page_to_frame`.
    pub fn frame_to_page(&self, page: Id, t: f64, p: (f64, f64)) -> Option<(f64, f64)> {
        let (a, b) = (self.page_to_frame(page, t, (0.0, 0.0))?, self.page_to_frame(page, t, (1.0, 1.0))?);
        Some(((p.0 - a.0) / (b.0 - a.0), (p.1 - a.1) / (b.1 - a.1)))
    }

    /// The fake mouse goes to `to` (shown position: project px from the frame's middle), arriving
    /// at clip-local `t` after `secs`: it leaves gently, runs a little past the target and settles
    /// on it, like a hand does. Keys inside the move are replaced; the rest stays.
    pub fn cursor_move_to(&mut self, id: Id, t: f64, secs: f64, to: (f64, f64)) -> bool {
        let Some(b) = self.browser_mut(id) else { return false };
        let c = b.cursor.get_or_insert_with(Cursor::default);
        c.visible = true;
        let t0 = (t - secs.max(0.05)).max(0.0);
        let from = c.pos(t0);
        // ease in, about 4 % past the target, back onto it
        let reach = Ease::Bezier { x1: 0.3, y1: 0.0, x2: 0.45, y2: 1.3 };
        for (a, v0, v1) in [(&mut c.x, from.0, to.0), (&mut c.y, from.1, to.1)] {
            a.unlink();
            if !a.is_animated() {
                a.keys.push(Keyframe { t: 0.0, v: a.value, ease: Ease::Hold });
            }
            a.keys.retain(|k| k.t < t0 - 1e-4 || k.t > t + 1e-4);
            for (kt, v, ease) in [(t0, v0, reach), (t, v1, Ease::PRESETS[0].1)] {
                let i = a.keys.partition_point(|k| k.t < kt);
                a.keys.insert(i, Keyframe { t: kt, v, ease });
            }
        }
        true
    }

    /// The video track between the pages and browser `id` that is free over `at..at+dur` (a new one
    /// right under the browser when there is none): where details and highlights go.
    fn track_under_browser(&mut self, id: Id, at: f64, dur: f64) -> Option<usize> {
        let bt = self.track_of(id)?;
        let floor = self
            .browser_pages(id)
            .into_iter()
            .filter(|&c| self.clip(c).is_some_and(|c| c.name != DETAIL && c.name != HIGHLIGHT))
            .filter_map(|c| self.track_of(c))
            .max();
        let free = (0..bt).rev().find(|&ti| {
            self.tracks[ti].kind == TrackKind::Video && floor.is_none_or(|f| ti > f) && self.tracks[ti].fits(at, dur, &[])
        });
        if free.is_some() {
            return free;
        }
        // a new track is made on top; walk it down to just under the browser
        let mut ti = self.add_track(TrackKind::Video);
        while ti > bt {
            if !self.move_track(ti, true) {
                break;
            }
            ti -= 1;
        }
        Some(ti)
    }

    /// Put picture `asset` over the part `rect` (project px from the frame's middle, before scroll
    /// and zoom) of the page under browser `id` for clip-local `from..to`: a sharper picture of that
    /// part, which scrolls and zooms with the page. Returns the new clip.
    pub fn browser_add_detail(&mut self, id: Id, asset: Id, from: f64, to: f64, rect: (f64, f64, f64, f64)) -> Option<Id> {
        let start = self.clip(id)?.start;
        let (nw, nh) = self.asset(asset).map(|a| (a.width as f64, a.height as f64)).filter(|s| s.0 > 0.0 && s.1 > 0.0)?;
        let (at, dur) = (start + from, (to - from).max(MIN_CLIP));
        let ti = self.track_under_browser(id, at, dur)?;
        let mut c = Clip::new(self.new_id(), ClipKind::Image, DETAIL, at, dur);
        c.asset = asset;
        let fit = (self.width as f64 / nw).min(self.height as f64 / nh);
        c.scale = Animated::new((rect.2 - rect.0) / (nw * fit));
        c.scale_y = Animated::new(((rect.3 - rect.1) / (nh * fit)) / c.scale.value);
        (c.x, c.y) = (Animated::new((rect.0 + rect.2) / 2.0), Animated::new((rect.1 + rect.3) / 2.0));
        let cid = c.id;
        self.tracks[ti].clips.push(c);
        self.tracks[ti].sort();
        Some(cid)
    }

    /// A soft box over `rect` of the page (as `browser_add_detail`) for clip-local `from..to`: what
    /// a link or button looks like while the mouse is on it.
    pub fn browser_add_highlight(&mut self, id: Id, from: f64, to: f64, rect: (f64, f64, f64, f64)) -> Option<Id> {
        let start = self.clip(id)?.start;
        let (at, dur) = (start + from, (to - from).max(MIN_CLIP));
        let ti = self.track_under_browser(id, at, dur)?;
        let mut c = Clip::new(self.new_id(), ClipKind::Shape, HIGHLIGHT, at, dur);
        if let Some(s) = c.shape.as_mut() {
            (s.fill, s.stroke, s.stroke_width, s.corner) = ([70, 140, 255, 46], [70, 140, 255, 190], 2.5, 8.0);
            (s.w, s.h) = (Animated::new((rect.2 - rect.0) / 2.0 + 4.0), Animated::new((rect.3 - rect.1) / 2.0 + 4.0));
        }
        (c.x, c.y) = (Animated::new((rect.0 + rect.2) / 2.0), Animated::new((rect.1 + rect.3) / 2.0));
        (c.fade_in, c.fade_out) = (0.12f64.min(dur / 3.0), 0.12f64.min(dur / 3.0));
        let cid = c.id;
        self.tracks[ti].clips.push(c);
        self.tracks[ti].sort();
        Some(cid)
    }

    /// Remove the zoom details under browser `id` (before making them again). Returns how many.
    pub fn browser_clear_details(&mut self, id: Id) -> usize {
        let old: Vec<Id> =
            self.browser_pages(id).into_iter().filter(|&c| self.clip(c).is_some_and(|c| c.name == DETAIL)).collect();
        self.delete_clips(&old, false);
        old.len()
    }

    /// Replace the cursor's path between the first and last point's times with `points`
    /// ((clip-local t, x, y), page px from the page centre). Keys outside that span stay.
    pub fn cursor_set_path(&mut self, id: Id, points: &[(f64, f64, f64)], how: CursorMove) -> bool {
        let Some(b) = self.browser_mut(id) else { return false };
        let (Some(first), Some(last)) = (points.first(), points.last()) else { return false };
        let c = b.cursor.get_or_insert_with(Cursor::default);
        let ease = if how == CursorMove::Stops { Ease::PRESETS[0].1 } else { Ease::Linear };
        for (a, pick) in [(&mut c.x, 1usize), (&mut c.y, 2usize)] {
            if !a.is_animated() && first.0 > 1e-4 {
                // the pointer waits where it was until the path begins
                a.keys.push(Keyframe { t: 0.0, v: a.value, ease });
            }
            a.unlink();
            a.keys.retain(|k| k.t < first.0 - 1e-4 || k.t > last.0 + 1e-4);
            let mut prev = f64::NEG_INFINITY;
            for p in points.iter().filter(|p| p.0.is_finite() && p.1.is_finite() && p.2.is_finite()) {
                if p.0 <= prev + 1e-4 {
                    continue;
                }
                prev = p.0;
                let v = if pick == 1 { p.1 } else { p.2 };
                let i = a.keys.partition_point(|k| k.t < p.0);
                a.keys.insert(i, Keyframe { t: p.0, v, ease });
            }
        }
        true
    }

    /// Add a cursor event (a click, a key press, a shape change...) at clip-local `t`.
    pub fn cursor_event(&mut self, id: Id, t: f64, kind: CursorEventKind) -> bool {
        let Some(b) = self.browser_mut(id) else { return false };
        let c = b.cursor.get_or_insert_with(Cursor::default);
        c.events.push(CursorEvent { t, kind });
        c.sort();
        true
    }

    /// Write a recorded take over clip-local `from..to`: the cursor keys, events and scroll keys
    /// in that span are replaced, everything outside it stays (re-recording one part of a move).
    pub fn cursor_apply_take(&mut self, id: Id, take: &Take, from: f64, to: f64) -> bool {
        let pts: Vec<(f64, f64, f64)> = take.keys.clone();
        if !self.cursor_set_path(id, &pts, CursorMove::Glide) {
            return false;
        }
        let Some(b) = self.browser_mut(id) else { return false };
        let c = b.cursor.get_or_insert_with(Cursor::default);
        c.visible = true;
        for a in [&mut c.x, &mut c.y] {
            // a take shorter than the span it replaces (the pointer stood still at the end)
            let (lo, hi) = (pts.first().map_or(from, |p| p.0), pts.last().map_or(to, |p| p.0));
            a.keys.retain(|k| k.t < from - 1e-4 || k.t > to + 1e-4 || (k.t >= lo - 1e-4 && k.t <= hi + 1e-4));
        }
        c.events.retain(|e| e.t < from - 1e-4 || e.t > to + 1e-4 || matches!(e.kind, CursorEventKind::Key(_)));
        c.events.extend(take.events.iter().cloned());
        c.sort();
        if !take.scroll.is_empty() {
            let a = &mut b.scroll;
            if !a.is_animated() {
                a.keys.push(Keyframe { t: 0.0, v: a.value, ease: Ease::Hold });
            }
            a.keys.retain(|k| k.t < from - 1e-4 || k.t > to + 1e-4);
            for (i, &(t, v)) in take.scroll.iter().enumerate() {
                // each wheel burst is (start, end): ease out of the start, hold after the end
                let ease = if i % 2 == 0 { Ease::EaseOut } else { Ease::Hold };
                let at = a.keys.partition_point(|k| k.t < t);
                a.keys.insert(at, Keyframe { t, v, ease });
            }
        }
        true
    }

    /// Remove jitter from the cursor's path in clip-local `from..to`: only the keys the path needs
    /// to stay within `tol` px survive (and every key an event sits on). Returns keys removed.
    pub fn cursor_tidy(&mut self, id: Id, tol: f64, from: f64, to: f64) -> usize {
        let Some(c) = self.browser_mut(id).and_then(|b| b.cursor.as_mut()) else { return 0 };
        let times: Vec<f64> = c.x.keys.iter().map(|k| k.t).filter(|t| *t >= from - 1e-4 && *t <= to + 1e-4).collect();
        let pts: Vec<(f64, f64, f64)> = times.iter().map(|&t| (c.x.at(t), c.y.at(t), t)).collect();
        let mut keep: Vec<bool> = vec![false; pts.len()];
        for i in simplify_path(&pts, tol.max(0.0)) {
            keep[i] = true;
        }
        for (i, t) in times.iter().enumerate() {
            keep[i] |= c.events.iter().any(|e| (e.t - t).abs() < 1e-3);
        }
        let drop: Vec<f64> = times.iter().zip(&keep).filter(|(_, k)| !**k).map(|(t, _)| *t).collect();
        for a in [&mut c.x, &mut c.y] {
            a.keys.retain(|k| !drop.iter().any(|t| (k.t - t).abs() < 1e-4));
        }
        drop.len()
    }

    /// A press on what the cursor is over: the topmost clip under the browser at the cursor's
    /// place at timeline `t` dips to 96 % and springs back (Scale keys around `t`). Returns that
    /// clip. ponytail: the dip is about the clip's own centre, so make the button its own clip
    /// (an HTML layer, a picture) - a whole-page screenshot would dip as a whole and is skipped.
    pub fn cursor_press(&mut self, id: Id, t: f64) -> Option<Id> {
        let br = self.clip(id).filter(|c| c.contains(t))?;
        let b = br.browser.as_ref()?;
        let lt = br.local(t);
        let (x, y) = b.page_point(lt, b.cursor.as_ref()?.pos(lt));
        let (pw, ph) = (self.width as f64, self.height as f64);
        let hit = self.browser_pages(id).into_iter().rev().find(|&cid| {
            let Some(c) = self.clip(cid).filter(|c| c.contains(t) && c.enabled) else { return false };
            let Some((nw, nh)) = self.clip_native_size(c) else { return false };
            let l = c.local(t);
            let fit = (pw / nw as f64).min(ph / nh as f64) * c.scale.at(l);
            let (hw, hh) = (nw as f64 * fit * c.scale_x.at(l) / 2.0, nh as f64 * fit * c.scale_y.at(l) / 2.0);
            let covers_page = hw * 2.0 >= pw * 0.9 && hh * 2.0 >= ph * 0.9;
            let named = c.name == DETAIL || c.name == HIGHLIGHT;
            !covers_page && !named && (x - c.x.at(l)).abs() <= hw && (y - c.y.at(l)).abs() <= hh
        })?;
        let c = self.clip_mut(hit)?;
        let l = c.local(t);
        let base = c.scale.at(l);
        if !c.scale.is_animated() {
            c.scale.keys.push(Keyframe { t: 0.0, v: base, ease: Ease::Hold });
        }
        for (dt, v, ease) in [(-0.02, base, Ease::EaseOut), (0.07, base * 0.96, Ease::EaseInOut), (0.24, base, Ease::Hold)] {
            let kt = (l + dt).clamp(0.0, c.duration);
            c.scale.keys.retain(|k| (k.t - kt).abs() > 1e-4);
            let i = c.scale.keys.partition_point(|k| k.t < kt);
            c.scale.keys.insert(i, Keyframe { t: kt, v, ease });
        }
        Some(hit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(id: Id, w: u32, h: u32) -> Asset {
        serde_json::from_value(serde_json::json!({
            "id": id, "path": format!("C:/browser-test-{id}.png"), "kind": "Image",
            "duration": 0.0, "width": w, "height": h, "fps": 0.0
        }))
        .unwrap()
    }

    fn page_project() -> (Project, Id) {
        // a 1000x3000 screenshot on V1 of a 1920x1080 project
        let mut p = Project::new();
        let aid = p.new_id();
        p.assets.push(asset(aid, 1000, 3000));
        let cid = p.new_id();
        let mut c = Clip::new(cid, ClipKind::Image, "page", 0.0, 10.0);
        c.asset = aid;
        let ti = p.video_tracks()[0];
        p.tracks[ti].clips.push(c);
        (p, cid)
    }

    #[test]
    fn a_browser_goes_above_what_is_there_and_old_files_still_load() {
        let (mut p, page) = page_project();
        let id = p.add_browser_clip(2.0, 5.0);
        assert!(p.track_of(id).unwrap() > p.track_of(page).unwrap());
        assert_eq!(p.clip(id).unwrap().kind, ClipKind::Browser);
        let b = p.clip(id).unwrap().browser.clone().unwrap();
        // the page area has the frame's shape
        let (vw, vh) = b.viewport();
        assert!((vw / vh - 1920.0 / 1080.0).abs() < 0.01, "{vw}x{vh}");
        // a second one in the same range goes above the first
        let id2 = p.add_browser_clip(3.0, 2.0);
        assert!(p.track_of(id2).unwrap() > p.track_of(id).unwrap());
        // round trip: every field survives, a clip saved before browsers existed has none
        p.cursor_event(id, 1.0, CursorEventKind::Key("Ctrl+L".into()));
        p.cursor_set_path(id, &[(0.0, 1.0, 2.0), (1.0, 30.0, 40.0)], CursorMove::Stops);
        let json = p.to_json();
        let q = Project::from_json(&json).unwrap();
        assert_eq!(q.clip(id).unwrap().browser, p.clip(id).unwrap().browser);
        assert!(q.clip(page).unwrap().browser.is_none());
        let old = serde_json::to_string(q.clip(page).unwrap()).unwrap();
        assert!(!old.contains("browser"), "other clips do not grow a browser field");
        let back: Clip = serde_json::from_str(&old).unwrap();
        assert!(back.browser.is_none());
        // a browser clip written by hand with only its kind gets every default
        let bare: BrowserStyle = serde_json::from_str("{}").unwrap();
        assert_eq!(bare, BrowserStyle::default());
    }

    #[test]
    fn fit_pages_makes_a_tall_picture_a_scrollable_page() {
        let (mut p, page) = page_project();
        let id = p.add_browser_clip(0.0, 10.0);
        let max = p.browser_fit_pages(id);
        let c = p.clip(page).unwrap();
        // contained it would be 360 px wide; as a page it is 1920 wide and 5760 tall, top at the top
        assert!((c.scale.value - 1920.0 / 360.0).abs() < 1e-6);
        assert!((c.y.value - (5760.0 - 1080.0) / 2.0).abs() < 1e-6);
        assert!((max - (5760.0 - 1080.0)).abs() < 2.0, "{max}");
    }

    #[test]
    fn page_cut_splits_the_page_and_runs_the_load_bar() {
        let (mut p, page) = page_project();
        let id = p.add_browser_clip(0.0, 10.0);
        p.clip_mut(id).unwrap().browser.as_mut().unwrap().scroll = Animated::new(300.0);
        let other = p.new_id();
        p.assets.push(asset(other, 800, 600));
        let next = NextPage { asset: Some(other), url: "https://next.page", title: "Next", favicon: "C:/n.png", load: 0.5, ..Default::default() };
        let new = p.browser_page_cut(id, 4.0, &next);
        assert_eq!(new.len(), 1);
        assert_eq!(p.clip(page).unwrap().duration, 4.0);
        assert_eq!(p.clip(new[0]).unwrap().asset, other);
        // 800x600 as a page of a 1920x1080 frame: 1920 wide (contained it is 1440), 1440 tall, top at the top
        let n = p.clip(new[0]).unwrap();
        assert!((n.scale.value - 1920.0 / 1440.0).abs() < 1e-9 && (n.y.value - 180.0).abs() < 1e-9);
        assert_eq!(p.clip(id).unwrap().duration, 10.0, "the browser itself is not cut");
        let b = p.clip(id).unwrap().browser.as_ref().unwrap();
        assert_eq!(b.load.at(3.0), 0.0);
        assert!(b.load.at(4.2) > 0.1 && b.load.at(4.2) < 1.0);
        assert_eq!(b.load.at(6.0), 1.0);
        assert_eq!((b.scroll.at(3.9), b.scroll.at(4.0)), (300.0, 0.0));
        assert_eq!((b.url_at(3.9), b.url_at(4.0), b.title_at(0, 4.0)), ("https://example.com", "https://next.page", "Next"));
        assert_eq!((b.favicon_at(0, 3.9), b.favicon_at(0, 4.0)), ("", "C:/n.png"));
        // a second page later: the bar is hidden in between and runs again
        let fade = NextPage { load: 0.5, transition: PageTransition::Fade, secs: 0.4, ..Default::default() };
        let cut = p.browser_page_cut(id, 7.0, &fade);
        assert!(p.tracks.iter().any(|t| t.transition_at(7.0).is_some_and(|(_, _, r)| r.map(|c| c.id) == cut.last().copied())), "a fade across the cut");
        let b = p.clip(id).unwrap().browser.as_ref().unwrap();
        assert_eq!(b.load.at(6.5), 1.0);
        assert!(b.load.at(7.25) > 0.0 && b.load.at(7.25) < 1.0);
    }

    #[test]
    fn the_mouse_reaches_past_its_target_and_settles() {
        let (mut p, _) = page_project();
        let id = p.add_browser_clip(0.0, 10.0);
        p.cursor_set_path(id, &[(0.0, -400.0, 0.0), (1.0, -400.0, 0.0)], CursorMove::Stops);
        assert!(p.cursor_move_to(id, 3.0, 1.0, (400.0, 200.0)));
        let c = p.clip(id).unwrap().browser.as_ref().unwrap().cursor.as_ref().unwrap();
        assert_eq!(c.pos(2.0), (-400.0, 0.0), "it waits where it was until the move begins");
        assert_eq!(c.pos(3.0), (400.0, 200.0));
        assert_eq!(c.pos(8.0), (400.0, 200.0));
        // slow to leave, and beyond the target (by a little) shortly before it arrives
        assert!(c.pos(2.1).0 < -330.0, "{:?}", c.pos(2.1));
        let far = (0..100).map(|i| c.pos(2.0 + i as f64 / 100.0).0).fold(f64::MIN, f64::max);
        assert!(far > 404.0 && far < 460.0, "overshoot {far}");
    }

    #[test]
    fn page_points_follow_the_page_clip_and_details_land_on_them() {
        let (mut p, page) = page_project();
        let id = p.add_browser_clip(0.0, 10.0);
        p.browser_fit_pages(id);
        // the 1000x3000 picture is 1920x5760 on the frame, its top at the frame's top
        assert_eq!(p.page_to_frame(page, 1.0, (0.0, 0.0)), Some((-960.0, -540.0)));
        assert_eq!(p.page_to_frame(page, 1.0, (0.5, 0.25)), Some((0.0, 900.0)));
        let back = p.frame_to_page(page, 1.0, (0.0, 900.0)).unwrap();
        assert!((back.0 - 0.5).abs() < 1e-9 && (back.1 - 0.25).abs() < 1e-9);
        // a part of the page seen from 2 s to 7 s
        struct Part {
            from: f64,
            to: f64,
            rect: (f64, f64, f64, f64),
        }
        let r = &Part { from: 2.0, to: 7.0, rect: (-90.0, -235.0, 390.0, 35.0) };
        // a detail picture of that part: a clip between the page and the browser, exactly over it
        let aid = p.new_id();
        p.assets.push(asset(aid, 1920, 1040));
        let d = p.browser_add_detail(id, aid, r.from, r.to, r.rect).unwrap();
        let (dt, pt, bt) = (p.track_of(d).unwrap(), p.track_of(page).unwrap(), p.track_of(id).unwrap());
        assert!(pt < dt && dt < bt, "page {pt} < detail {dt} < browser {bt}");
        let (a, z) = (p.page_to_frame(d, 3.0, (0.0, 0.0)).unwrap(), p.page_to_frame(d, 3.0, (1.0, 1.0)).unwrap());
        for (got, want) in [(a.0, r.rect.0), (a.1, r.rect.1), (z.0, r.rect.2), (z.1, r.rect.3)] {
            assert!((got - want).abs() < 1e-6, "{got} vs {want}");
        }
        let c = p.clip(d).unwrap();
        assert_eq!((c.start, c.duration, c.name.as_str()), (2.0, 5.0, DETAIL));
        // a highlight shares the track when it is free there, and details can be made again
        let hl = p.browser_add_highlight(id, 0.5, 1.5, (0.0, 0.0, 100.0, 40.0)).unwrap();
        assert_eq!(p.track_of(hl), Some(dt));
        assert_eq!(p.browser_clear_details(id), 1);
        assert!(p.clip(d).is_none() && p.clip(hl).is_some());
        // zoom details made by the app are not project clips: only the drawn copy has them
        use crate::engine::browser::{render_view, set_zoom_details, ZoomDetail};
        let clips = |p: &Project| p.all_clips().count();
        let (n, na) = (clips(&p), p.assets.len());
        set_zoom_details(id, vec![ZoomDetail { from: r.from, to: r.to, rect: r.rect, path: "zoom.png".into(), size: (1920, 1040) }]);
        let view = render_view(&p).into_owned();
        set_zoom_details(id, Vec::new());
        assert_eq!((clips(&p), p.assets.len()), (n, na), "the project is untouched");
        assert_eq!((clips(&view), view.assets.len()), (n + 1, na + 1), "the drawn copy has the detail");
        let v = view.all_clips().map(|(_, c)| c).find(|c| c.name == DETAIL).unwrap();
        assert!(v.id >= 1 << 40 && v.asset >= 1 << 40, "ids no project will ever hand out");
        assert!(matches!(render_view(&p), std::borrow::Cow::Borrowed(_)), "nothing to add: no copy");
        // a page cut leaves highlights alone
        let cut = p.browser_page_cut(id, 1.0, &NextPage { load: 0.5, ..Default::default() });
        assert_eq!(cut.len(), 1);
        assert_eq!(p.clip(hl).unwrap().duration, 1.0);
    }

    #[test]
    fn a_take_replaces_only_its_own_span() {
        let (mut p, _) = page_project();
        let id = p.add_browser_clip(0.0, 10.0);
        p.cursor_set_path(id, &[(0.0, 0.0, 0.0), (2.0, 100.0, 0.0), (4.0, 200.0, 0.0), (6.0, 300.0, 0.0)], CursorMove::Stops);
        p.cursor_event(id, 1.0, CursorEventKind::Click);
        p.cursor_event(id, 3.0, CursorEventKind::Click);
        let take = Take {
            keys: vec![(2.5, 0.0, 50.0), (3.0, 10.0, 60.0), (3.5, 20.0, 70.0)],
            events: vec![CursorEvent { t: 3.2, kind: CursorEventKind::DoubleClick }],
            scroll: vec![(2.6, 0.0), (2.8, 120.0)],
        };
        assert!(p.cursor_apply_take(id, &take, 2.5, 4.5));
        let b = p.clip(id).unwrap().browser.as_ref().unwrap();
        let c = b.cursor.as_ref().unwrap();
        let ts: Vec<f64> = c.x.keys.iter().map(|k| k.t).collect();
        assert_eq!(ts, vec![0.0, 2.0, 2.5, 3.0, 3.5, 6.0], "the old key at 4.0 was inside the take's span");
        let kinds: Vec<_> = c.events.iter().map(|e| (e.t, e.kind.clone())).collect();
        assert_eq!(kinds, vec![(1.0, CursorEventKind::Click), (3.2, CursorEventKind::DoubleClick)]);
        assert_eq!((b.scroll.at(2.0), b.scroll.at(5.0)), (0.0, 120.0));
        assert_eq!(c.y.keys.len(), c.x.keys.len());
    }

    #[test]
    fn tidy_drops_jitter_but_never_a_click() {
        let (mut p, _) = page_project();
        let id = p.add_browser_clip(0.0, 10.0);
        let pts: Vec<(f64, f64, f64)> =
            (0..=60).map(|i| (i as f64 / 20.0, i as f64 * 10.0, if i % 2 == 0 { 1.0 } else { -1.0 })).collect();
        p.cursor_set_path(id, &pts, CursorMove::Glide);
        p.cursor_event(id, 1.55, CursorEventKind::Click);
        let removed = p.cursor_tidy(id, 4.0, 0.0, 10.0);
        let c = p.clip(id).unwrap().browser.as_ref().unwrap().cursor.as_ref().unwrap();
        assert!(removed >= 55, "a straight, jittery line keeps its ends: removed {removed}");
        assert!(c.x.has_key_at(1.55), "the key under the click stays");
        assert_eq!(c.x.keys.len(), c.y.keys.len());
    }

    #[test]
    fn press_dips_the_button_under_the_cursor_not_the_page() {
        let (mut p, page) = page_project();
        // a 200x80 button picture on V2 at (300, 100)
        let aid = p.new_id();
        p.assets.push(asset(aid, 200, 80));
        let bid = p.new_id();
        let mut c = Clip::new(bid, ClipKind::Image, "button", 0.0, 10.0);
        c.asset = aid;
        (c.x, c.y) = (Animated::new(300.0), Animated::new(100.0));
        let ti = p.add_track(TrackKind::Video);
        p.tracks[ti].clips.push(c);
        let id = p.add_browser_clip(0.0, 10.0);
        p.browser_fit_pages(id);
        // fit_pages made the button page-wide too: put it back
        let k = 1.0 / 9.6;
        let b = p.clip_mut(bid).unwrap();
        (b.scale, b.x, b.y) = (Animated::new(k), Animated::new(300.0), Animated::new(100.0));
        p.cursor_set_path(id, &[(0.0, 0.0, 0.0), (2.0, 310.0, 95.0)], CursorMove::Stops);
        assert_eq!(p.cursor_press(id, 3.0), Some(bid));
        let b = p.clip(bid).unwrap();
        assert!(b.scale.at(3.07) < k * 0.97 && (b.scale.at(3.5) - k).abs() < 1e-9 && (b.scale.at(1.0) - k).abs() < 1e-9);
        assert!(!p.clip(page).unwrap().scale.is_animated(), "the page itself does not dip");
        // nothing under the cursor: nothing happens
        p.cursor_set_path(id, &[(4.0, -800.0, -400.0), (5.0, -800.0, -400.0)], CursorMove::Stops);
        assert_eq!(p.cursor_press(id, 5.0), None);
    }
}
