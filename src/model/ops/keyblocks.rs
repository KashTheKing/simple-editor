//! Keyframe-block edits: apply (chaining), move, stretch, re-ease and remove an applied block. Every
//! one only rewrites ordinary keys plus the clip's `blocks` bookkeeping.

use crate::model::*;

impl Project {
    /// Apply `block` to clip `id`. `at` = clip-local start; `None` chains after the clip's last block
    /// (or starts at 0). A start inside an existing block snaps to that block's end, so repeated clicks
    /// at the playhead chain too. `dur` overrides the block's default length. Returns (start, length).
    pub fn apply_key_block(
        &mut self,
        id: Id,
        block: &KeyBlock,
        at: Option<f64>,
        dur: Option<f64>,
    ) -> Option<(f64, f64)> {
        let c = self.clip(id)?;
        let mut t0 = at.unwrap_or_else(|| c.blocks.iter().map(|b| b.end()).fold(0.0, f64::max));
        while let Some(b) = c.blocks.iter().find(|b| t0 >= b.t - BK_EPS && t0 < b.end() - BK_EPS) {
            t0 = b.end();
        }
        self.place_key_block(id, block, t0, dur)
    }

    /// ws:animator: put `block` exactly at clip-local `t0` (no chaining: a speed ramp or a shake may run during
    /// a zoom - that's the point of separate blocks). `dur` defaults to the block's own length.
    pub(crate) fn place_key_block(&mut self, id: Id, block: &KeyBlock, t0: f64, dur: Option<f64>) -> Option<(f64, f64)> {
        let (w, h) = (self.width as f64, self.height as f64);
        self.clip_mut(id)?.place_block(block, t0, dur, w, h)
    }

    /// Re-time applied block `i` of clip `id` to start `t` and last `dur` (both clamped inside the clip),
    /// carrying its keys along - the timeline bar's drag (move) and edge drag (stretch).
    pub fn retime_key_block(&mut self, id: Id, i: usize, t: f64, dur: f64) -> bool {
        let Some(c) = self.clip_mut(id) else { return false };
        let Some(b) = c.blocks.get(i).cloned() else { return false };
        let dur = dur.clamp(0.05, c.duration.max(0.05));
        let t = t.clamp(0.0, (c.duration - dur).max(0.0));
        if (t - b.t).abs() < 1e-9 && (dur - b.dur).abs() < 1e-9 {
            return false;
        }
        let keys = c.block_keys(i);
        for (p, k, shared) in &keys {
            if let Some(a) = prop_mut(c, p) {
                if let (Some(j), false) = (a.key_index_at(k.t), shared) {
                    a.keys.remove(j);
                }
            }
        }
        let s = dur / b.dur.max(1e-9);
        for (p, k, _) in keys {
            if let Some(a) = prop_mut(c, &p) {
                upsert(a, Keyframe { t: t + (k.t - b.t) * s, ..k });
            }
        }
        c.blocks[i].t = t;
        c.blocks[i].dur = dur;
        // a simulation runs in real seconds: re-run it at the new length rather than stretching it
        if let Some(ctl) = c.blocks[i].ctl.clone().filter(|c| c.is_sim() && !c.edited) {
            self.set_block_ctl(id, i, ctl);
        }
        true
    }

    /// ws:animator: turn the keys made by hand in clip-local [from, to] into a block with controls (strength,
    /// ease, side when it moves along x / y) - the motion is unchanged until a control is. `None` span = every
    /// loose span (`Clip::loose_key_spans`). Returns how many blocks it made.
    pub fn block_from_keys(&mut self, id: Id, span: Option<(f64, f64)>, name: &str) -> usize {
        let (w, h) = (self.width as f64, self.height as f64);
        let Some(c) = self.clip_mut(id) else { return 0 };
        let spans = match span {
            Some(s) => vec![s],
            None => c.loose_key_spans(),
        };
        let mut n = 0;
        for (from, to) in spans {
            let bare = without_block_keys(c);
            let Some(block) = bare.capture_block(name, from, to, w, h) else { continue };
            // the captured block starts / ends at its first / last key, which is the span we replace
            let (t0, dur) = (from.max(0.0), block.dur);
            n += c.place_block(&block, t0, Some(dur), w, h).is_some() as usize;
        }
        n
    }

    /// Change applied block `i`'s controls (strength, side, ease, physics) and rewrite its keys from them.
    /// Setting controls on a hand-edited block resets it to them.
    pub fn set_block_ctl(&mut self, id: Id, i: usize, mut ctl: BlockCtl) -> bool {
        let (w, h) = (self.width as f64, self.height as f64);
        let Some(c) = self.clip_mut(id) else { return false };
        let Some(b) = c.blocks.get(i).cloned() else { return false };
        clear_block_keys(c, i);
        ctl.edited = false;
        let props = ctl.resolve(b.dur).write(c, b.t, b.dur, w, h);
        let ab = &mut c.blocks[i];
        ab.props = props;
        ab.ctl = Some(ctl);
        true
    }

    /// Delete applied block `i` and the keys it wrote (a seam key shared with a chained neighbour stays).
    pub fn remove_key_block(&mut self, id: Id, i: usize) -> bool {
        let Some(c) = self.clip_mut(id) else { return false };
        if i >= c.blocks.len() {
            return false;
        }
        clear_block_keys(c, i);
        c.blocks.remove(i);
        true
    }

    /// Set the easing of every segment inside applied block `i` (its last key's ease belongs to what
    /// follows, so it is left alone). A block with live controls takes it as its Ease control, so the
    /// Inspector shows it and the next control change keeps it.
    pub fn ease_key_block(&mut self, id: Id, i: usize, ease: Ease) -> bool {
        let live = self.clip(id).and_then(|c| c.blocks.get(i)).and_then(|b| b.ctl.clone());
        if let Some(mut ctl) = live.filter(|c| !c.edited && !c.is_sim()) {
            // an ease picked by hand replaces what Flow bent the ends to
            (ctl.ease, ctl.in_speed, ctl.out_speed) = (Some(ease), None, None);
            return self.set_block_ctl(id, i, ctl);
        }
        let Some(c) = self.clip_mut(id) else { return false };
        let Some(end) = c.blocks.get(i).map(|b| b.end()) else { return false };
        for (p, k, _) in c.block_keys(i) {
            if (k.t - end).abs() < BK_EPS {
                continue;
            }
            if let Some(a) = prop_mut(c, &p) {
                a.set_ease_at(k.t, ease);
            }
        }
        true
    }

    /// Split applied block `i` at clip-local `t` into two blocks, the motion unchanged: the first now ends at
    /// the value there (its target), the second starts from it (its origin) and keeps the old target.
    pub fn split_key_block(&mut self, id: Id, i: usize, t: f64) -> bool {
        let Some(c) = self.clip_mut(id) else { return false };
        let Some(b) = c.blocks.get(i).cloned() else { return false };
        if t < b.t + 0.02 || t > b.end() - 0.02 {
            return false;
        }
        // the halves as plain origin -> target moves: each segment's ease cut at the split
        for p in &b.props {
            if let Some(a) = prop_mut(c, p) {
                for e in [b.t, t, b.end()] {
                    cut_at(a, e);
                }
            }
        }
        let (l, r) = (abs_block(c, &b.name, &b.props, b.t, t), abs_block(c, &b.name, &b.props, t, b.end()));
        let part = |src: KeyBlock, t: f64| AppliedBlock {
            name: b.name.clone(),
            t,
            dur: src.dur,
            props: b.props.clone(),
            ctl: Some(BlockCtl::new(src)),
        };
        c.blocks[i] = part(l, b.t);
        c.blocks.insert(i + 1, part(r, t));
        true
    }

    /// Link applied blocks `idx` of clip `id` into one block over their whole span with all their properties,
    /// the motion unchanged (the keys stay; only the bookkeeping and the controls become one). Returns its index.
    pub fn link_key_blocks(&mut self, id: Id, idx: &[usize]) -> Option<usize> {
        let c = self.clip_mut(id)?;
        let mut idx: Vec<usize> = idx.iter().copied().filter(|&i| i < c.blocks.len()).collect();
        idx.sort();
        idx.dedup();
        if idx.len() < 2 {
            return None;
        }
        let parts: Vec<AppliedBlock> = idx.iter().rev().map(|&i| c.blocks.remove(i)).collect();
        let t0 = parts.iter().map(|b| b.t).fold(f64::MAX, f64::min);
        let t1 = parts.iter().map(|b| b.end()).fold(f64::MIN, f64::max);
        let (mut props, mut names): (Vec<String>, Vec<&str>) = (Vec::new(), Vec::new());
        for b in parts.iter().rev() {
            props.extend(b.props.iter().filter(|p| !props.contains(p)).cloned().collect::<Vec<_>>());
            if !names.contains(&b.name.as_str()) {
                names.push(&b.name);
            }
        }
        let name = names.join(" + ");
        let ctl = Some(BlockCtl::new(abs_block(c, &name, &props, t0, t1)));
        c.blocks.push(AppliedBlock { name, t: t0, dur: t1 - t0, props, ctl });
        c.blocks.sort_by(|a, b| a.t.total_cmp(&b.t));
        c.blocks.iter().position(|b| (b.t - t0).abs() < 1e-9 && (b.end() - t1).abs() < 1e-9)
    }

    /// Chain blocks `sel` in time order: each next one on the same clip moves to start where the one before
    /// ends, from the value it ends at (its origin = that target), and their speeds are matched at the seam
    /// (`match_seam`; across a cut only the speeds). Returns how many hand-offs it made.
    pub fn flow_key_blocks(&mut self, sel: &[(Id, usize)], mode: Match) -> usize {
        let at = |p: &Project, (id, i): (Id, usize)| p.clip(id).and_then(|c| Some(c.start + c.blocks.get(i)?.t));
        let mut sel: Vec<(Id, usize)> = sel.iter().copied().filter(|&s| at(self, s).is_some()).collect();
        sel.sort_by(|a, b| at(self, *a).unwrap_or(0.0).total_cmp(&at(self, *b).unwrap_or(0.0)));
        let mut n = 0;
        for w in sel.windows(2) {
            let (l, r) = (w[0], w[1]);
            let same = l.0 == r.0 && l.1 != r.1;
            if same {
                let c = self.clip(l.0).unwrap();
                let (a, b) = (c.blocks[l.1].clone(), c.blocks[r.1].clone());
                // where the earlier block ends, read before the next one's first key lands on that time
                let ends: Vec<(String, f64)> = a
                    .props
                    .iter()
                    .filter(|p| b.props.contains(p))
                    .filter_map(|p| Some((p.clone(), prop_ref(c, p)?.at(a.end()))))
                    .collect();
                self.retime_key_block(r.0, r.1, a.end(), b.dur);
                let c = self.clip_mut(r.0).unwrap();
                let t0 = c.blocks[r.1].t;
                match c.blocks[r.1].ctl.clone().filter(|c| !c.edited) {
                    Some(mut ctl) => {
                        ctl.from.retain(|(p, _)| !ends.iter().any(|e| e.0 == *p));
                        ctl.from.extend(ends);
                        self.set_block_ctl(r.0, r.1, ctl);
                    }
                    None => {
                        for (p, v) in ends {
                            if let Some(a) = prop_mut(c, &p) {
                                if let Some(k) = a.key_index_at(t0) {
                                    a.keys[k].v = v;
                                }
                            }
                        }
                    }
                }
            }
            n += (self.match_seam(l, r, mode).is_ok() || same) as usize;
        }
        n
    }
}

/// The keys of `props` in clip-local [from, to] as a block of absolute values: a plain origin -> target move
/// whose controls rewrite exactly these keys.
fn abs_block(c: &Clip, name: &str, props: &[String], from: f64, to: f64) -> KeyBlock {
    let track = |p: &String| {
        let keys = prop_ref(c, p)?.keys.iter().filter(|k| k.t >= from - BK_EPS && k.t <= to + BK_EPS);
        let keys = keys.map(|k| BlockKey { f: (k.t - from) / (to - from), v: BlockVal::Abs(k.v), ease: k.ease });
        Some(BlockTrack { prop: p.clone(), keys: keys.collect() })
    };
    KeyBlock { name: name.into(), dur: to - from, tracks: props.iter().filter_map(track).collect(), gen: None }
}

/// Put a key on `a` at `t` without changing the curve: the segment it lands in is cut in two, each half a
/// bezier following the old ease (de Casteljau). Nothing when a key is already there or `a` has no keys.
fn cut_at(a: &mut Animated, t: f64) {
    if a.keys.is_empty() || a.key_index_at(t).is_some() {
        return;
    }
    let i = a.keys.partition_point(|k| k.t < t);
    let mut key = Keyframe { t, v: a.base_at(t), ease: Ease::Linear };
    if i > 0 && i < a.keys.len() {
        let (k0, k1) = (a.keys[i - 1], a.keys[i]);
        let f = (t - k0.t) / (k1.t - k0.t);
        match k0.ease {
            Ease::Linear => {}
            Ease::Hold => key.ease = Ease::Hold,
            e => {
                type P = (f64, f64);
                // the fixed eases are exact cubics (not the rounder handles the curve editor shows for them)
                let (p1, p2): (P, P) = match e {
                    Ease::EaseIn => ((1.0 / 3.0, 0.0), (2.0 / 3.0, 1.0 / 3.0)),
                    Ease::EaseOut => ((1.0 / 3.0, 2.0 / 3.0), (2.0 / 3.0, 1.0)),
                    Ease::EaseInOut => ((1.0 / 3.0, 0.0), (2.0 / 3.0, 1.0)),
                    _ => {
                        let (x1, y1, x2, y2) = e.handles();
                        ((x1 as f64, y1 as f64), (x2 as f64, y2 as f64))
                    }
                };
                let mix = |a: P, b: P, u: f64| (a.0 + (b.0 - a.0) * u, a.1 + (b.1 - a.1) * u);
                // de Casteljau at u: (left handles, the split point, right handles)
                let split = |u: f64| {
                    let (q0, q1, q2) = (mix((0.0, 0.0), p1, u), mix(p1, p2, u), mix(p2, (1.0, 1.0), u));
                    let (r0, r1) = (mix(q0, q1, u), mix(q1, q2, u));
                    (q0, r0, mix(r0, r1, u), r1, q2)
                };
                // the curve parameter where x = f (x is monotonic for handles inside 0..1)
                let (mut lo, mut hi) = (0.0, 1.0);
                for _ in 0..40 {
                    let u = (lo + hi) / 2.0;
                    if split(u).2 .0 < f {
                        lo = u;
                    } else {
                        hi = u;
                    }
                }
                let (l1, l2, s, r1, r2) = split((lo + hi) / 2.0);
                // each half in its own 0..1 box (a level half has no height to scale by)
                let norm = |p: P, o: P, w: f64, h: f64| {
                    let y = if h.abs() < 1e-9 { 0.0 } else { (p.1 - o.1) / h };
                    (((p.0 - o.0) / w.max(1e-9)).clamp(0.0, 1.0) as f32, y as f32)
                };
                let bez = |a: (f32, f32), b: (f32, f32)| Ease::Bezier { x1: a.0, y1: a.1, x2: b.0, y2: b.1 };
                a.keys[i - 1].ease = bez(norm(l1, (0.0, 0.0), s.0, s.1), norm(l2, (0.0, 0.0), s.0, s.1));
                key.ease = bez(norm(r1, s, 1.0 - s.0, 1.0 - s.1), norm(r2, s, 1.0 - s.0, 1.0 - s.1));
            }
        }
    }
    a.keys.insert(i, key);
}

/// ws:animator: the clip with every block's keys taken out - what is left are the keys made by hand.
fn without_block_keys(c: &Clip) -> Clip {
    let mut k = c.clone();
    for i in (0..k.blocks.len()).rev() {
        clear_block_keys(&mut k, i);
    }
    k.blocks.clear();
    k
}

impl Clip {
    /// ws:animator: spans (clip-local) of keys made by hand - keys no block owns - one per cluster (a gap over
    /// a second splits), each with keys at two or more times and no block on the same properties in it.
    pub fn loose_key_spans(&self) -> Vec<(f64, f64)> {
        let bare = without_block_keys(self);
        let mut times: Vec<(f64, &str)> = BLOCK_PROPS
            .iter()
            .filter_map(|&p| prop_ref(&bare, p).map(|a| (p, a)))
            .flat_map(|(p, a)| a.keys.iter().map(move |k| (k.t, p)))
            .collect();
        times.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut spans: Vec<(f64, f64, Vec<&str>)> = Vec::new();
        for (t, p) in times {
            match spans.last_mut() {
                Some(s) if t - s.1 <= 1.0 => {
                    s.1 = t;
                    if !s.2.contains(&p) {
                        s.2.push(p);
                    }
                }
                _ => spans.push((t, t, vec![p])),
            }
        }
        spans
            .into_iter()
            .filter(|(a, b, props)| {
                b - a > BK_EPS
                    && !self.blocks.iter().any(|bl| bl.t < *b && bl.end() > *a && bl.props.iter().any(|p| props.contains(&p.as_str())))
            })
            .map(|(a, b, _)| (a, b))
            .collect()
    }
}

/// Remove the keys applied block `i` wrote (a seam key shared with a chained neighbour stays).
fn clear_block_keys(c: &mut Clip, i: usize) {
    for (p, k, shared) in c.block_keys(i) {
        if let Some(a) = prop_mut(c, &p) {
            if let (Some(j), false) = (a.key_index_at(k.t), shared) {
                // the constant `value` was never touched by apply, so an emptied property falls back to it
                a.keys.remove(j);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proj() -> (Project, Id) {
        let mut p = Project::new();
        let c = Clip::new(900, ClipKind::Video, "v", 0.0, 6.0);
        p.tracks[0].clips.push(c);
        (p, 900)
    }
    fn find(name: &str) -> KeyBlock {
        builtin_blocks().into_iter().find(|b| b.name == name).unwrap()
    }

    #[test]
    fn ships_about_25_unique_blocks_on_real_props() {
        let all = builtin_blocks();
        assert!(all.len() >= 25);
        let mut names: Vec<_> = all.iter().map(|b| b.name.clone()).collect();
        names.dedup();
        assert_eq!(names.len(), all.len());
        for b in &all {
            assert!(b.dur > 0.0 && (!b.tracks.is_empty() || b.gen.is_some()), "{}", b.name);
            for t in &b.tracks {
                assert!(BLOCK_PROPS.contains(&t.prop.as_str()), "{}: {}", b.name, t.prop);
            }
        }
    }

    #[test]
    fn blocks_chain_end_to_end_continuing_from_the_current_value() {
        let (mut p, id) = proj();
        let w = p.width as f64;
        assert_eq!(p.apply_key_block(id, &find("Slide In Left"), None, None), Some((0.0, 0.6)));
        // no `at`: chains after the last block
        let (t1, _) = p.apply_key_block(id, &find("Zoom Punch"), None, None).unwrap();
        assert!((t1 - 0.6).abs() < 1e-9);
        // a click inside an existing block lands at its end
        let (t2, _) = p.apply_key_block(id, &find("Fade Out"), Some(0.7), None).unwrap();
        assert!((t2 - 1.1).abs() < 1e-9);
        let c = p.clip(id).unwrap();
        assert!((c.x.at(0.0) + w).abs() < 1e-6, "slides in from one frame-width left");
        assert!(c.x.at(0.6).abs() < 1e-6, "…and ends where it was");
        assert!((c.scale.at(0.6 + 0.125) - 1.25).abs() < 0.15, "zoom punch peaks");
        assert!((c.opacity.at(1.1) - 1.0).abs() < 1e-6 && c.opacity.at(1.6).abs() < 1e-6, "fades out from 1 to 0");
        assert_eq!(c.blocks.len(), 3);
    }

    #[test]
    fn move_stretch_ease_and_remove_carry_the_keys() {
        let (mut p, id) = proj();
        p.apply_key_block(id, &find("Spin"), Some(1.0), None);
        assert!(p.retime_key_block(id, 0, 2.0, 1.6));
        let c = p.clip(id).unwrap();
        assert_eq!(c.rotation.keys.iter().map(|k| k.t).collect::<Vec<_>>(), vec![2.0, 3.6]);
        assert!((c.rotation.at(3.6) - 360.0).abs() < 1e-9);
        assert!(p.ease_key_block(id, 0, Ease::Linear));
        assert_eq!(p.clip(id).unwrap().rotation.keys[0].ease, Ease::Linear);
        assert!(p.remove_key_block(id, 0));
        let c = p.clip(id).unwrap();
        assert!(c.rotation.keys.is_empty() && c.blocks.is_empty());
        assert_eq!(c.rotation.value, 0.0);
    }

    #[test]
    fn removing_one_block_of_a_chain_keeps_the_seam_key() {
        let (mut p, id) = proj();
        p.apply_key_block(id, &find("Fade In"), None, None);
        p.apply_key_block(id, &find("Fade Out"), None, None);
        assert!(p.remove_key_block(id, 1));
        let c = p.clip(id).unwrap();
        assert_eq!(c.opacity.keys.len(), 2, "fade-in's end key (the seam) survives");
        assert!((c.opacity.at(2.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn captured_block_replays_the_same_motion_elsewhere() {
        let (mut p, id) = proj();
        let (w, h) = (p.width as f64, p.height as f64);
        p.apply_key_block(id, &find("Bounce"), Some(0.0), None);
        let saved = p.clip(id).unwrap().capture_block("Mine", 0.0, 1.0, w, h).unwrap();
        assert!((saved.dur - 0.8).abs() < 1e-9);
        let before: Vec<f64> = (0..8).map(|i| p.clip(id).unwrap().y.at(i as f64 * 0.1)).collect();
        p.apply_key_block(id, &saved, Some(3.0), None);
        let c = p.clip(id).unwrap();
        for (i, v) in before.iter().enumerate() {
            assert!((c.y.at(3.0 + i as f64 * 0.1) - v).abs() < 1e-6);
        }
    }

    #[test]
    fn controls_rewrite_the_keys_and_a_simulation_reruns_on_stretch() {
        let (mut p, id) = proj();
        let w = p.width as f64;
        p.apply_key_block(id, &find("Slide In Left"), Some(1.0), None);
        let mut ctl = p.clip(id).unwrap().blocks[0].ctl.clone().unwrap();
        ctl.dir = Some(Dir::Right);
        ctl.strength = 0.5;
        assert!(p.set_block_ctl(id, 0, ctl));
        let c = p.clip(id).unwrap();
        assert!((c.x.at(1.0) - w * 0.5).abs() < 1e-6, "now starts half a frame to the right");
        assert_eq!(c.x.keys.len(), 2, "the old keys went");
        p.apply_key_block(id, &find("Camera Shake"), Some(3.0), Some(1.0));
        let n = p.clip(id).unwrap().x.keys.len();
        assert!(p.retime_key_block(id, 1, 3.0, 2.0));
        let c = p.clip(id).unwrap();
        assert!(c.x.keys.len() > n, "twice as long = twice the jolts, not a slowed-down shake");
        assert!(c.blocks[1].props.contains(&"scale".to_string()));
    }

    #[test]
    fn placing_a_block_overlaps_where_applying_chains() {
        let (mut p, id) = proj();
        p.apply_key_block(id, &find("Ken Burns"), Some(0.0), Some(4.0));
        // a speed ramp in the middle of a zoom lands where it was asked to
        assert_eq!(p.place_key_block(id, &find("Speed Ramp Out"), 3.0, Some(0.5)), Some((3.0, 0.5)));
        // applying (the Gallery's click) still chains after what's there
        assert_eq!(p.apply_key_block(id, &find("Zoom Punch"), Some(1.0), None).map(|r| r.0), Some(4.0));
    }

    #[test]
    fn hand_made_keys_become_a_block_that_keeps_the_motion() {
        let (mut p, id) = proj();
        let w = p.width as f64;
        // hand keys: x 0 -> 300 -> 0 over 1..3 s, and a block elsewhere that must not be swallowed
        {
            let c = p.clip_mut(id).unwrap();
            for (t, v) in [(1.0, 0.0), (2.0, 300.0), (3.0, 0.0)] {
                c.x.keys.push(Keyframe { t, v, ease: Ease::EaseInOut });
            }
        }
        p.apply_key_block(id, &find("Fade Out"), Some(5.0), None);
        let before: Vec<f64> = (0..=20).map(|i| p.clip(id).unwrap().x.at(1.0 + i as f64 * 0.1)).collect();
        assert_eq!(p.clip(id).unwrap().loose_key_spans(), vec![(1.0, 3.0)]);
        assert_eq!(p.block_from_keys(id, None, "Custom"), 1);
        let c = p.clip(id).unwrap();
        assert_eq!(c.blocks.len(), 2);
        let b = c.blocks.iter().find(|b| b.name == "Custom").unwrap();
        assert!((b.t, b.dur) == (1.0, 2.0) && b.ctl.is_some(), "{b:?}");
        for (i, v) in before.iter().enumerate() {
            assert!((c.x.at(1.0 + i as f64 * 0.1) - v).abs() < 1e-6, "unchanged at {}", 1.0 + i as f64 * 0.1);
        }
        assert!(c.loose_key_spans().is_empty(), "nothing loose left");
        // and now its controls work: half strength = half the swing
        let i = c.blocks.iter().position(|b| b.name == "Custom").unwrap();
        let mut ctl = c.blocks[i].ctl.clone().unwrap();
        ctl.strength = 0.5;
        p.set_block_ctl(id, i, ctl);
        assert!((p.clip(id).unwrap().x.at(2.0) - 150.0).abs() < 1e-6);
        let _ = w;
    }

    #[test]
    fn fade_out_writes_two_opacity_keys() {
        let (mut p, id) = proj();
        p.apply_key_block(id, &find("Fade In"), Some(0.0), None);
        p.apply_key_block(id, &find("Fade Out"), Some(5.0), None).unwrap();
        let n = |p: &Project| p.clip(id).unwrap().block_keys(1).len();
        assert_eq!(n(&p), 2);
        assert_eq!(p.clip(id).unwrap().opacity.keys.len(), 4);
        // re-easing, stretching and moving it never add keys
        let mut ctl = p.clip(id).unwrap().blocks[1].ctl.clone().unwrap();
        ctl.ease = Some(Ease::PRESETS[1].1);
        p.set_block_ctl(id, 1, ctl);
        assert!(p.clip(id).unwrap().opacity.at(5.1) > 0.9, "Snap holds near the origin before it jumps");
        for k in 0..20 {
            p.retime_key_block(id, 1, 4.0 + k as f64 * 0.013, 0.5 + k as f64 * 0.031);
        }
        p.ease_key_block(id, 1, Ease::EaseOut);
        assert_eq!(n(&p), 2);
        assert_eq!(p.clip(id).unwrap().opacity.keys.len(), 4);
    }

    #[test]
    fn ease_origin_target_split_and_flow() {
        let (mut p, id) = proj();
        p.apply_key_block(id, &find("Bounce"), Some(0.0), None);
        // an ease re-eases every segment of the block and stays its control
        assert!(p.ease_key_block(id, 0, Ease::Linear));
        let c = p.clip(id).unwrap();
        assert!(c.y.keys[..4].iter().all(|k| k.ease == Ease::Linear));
        assert_eq!(c.blocks[0].ctl.as_ref().unwrap().ease, Some(Ease::Linear));
        p.remove_key_block(id, 0);
        // origin / target: the slide starts and ends where it is told to
        p.apply_key_block(id, &find("Slide In Left"), Some(0.0), Some(1.0));
        let mut ctl = p.clip(id).unwrap().blocks[0].ctl.clone().unwrap();
        (ctl.from, ctl.to) = (vec![("x".into(), -400.0)], vec![("x".into(), 200.0)]);
        p.set_block_ctl(id, 0, ctl);
        let x = |p: &Project, t: f64| p.clip(id).unwrap().x.at(t);
        assert_eq!((x(&p, 0.0), x(&p, 1.0)), (-400.0, 200.0));
        // split: two blocks, the motion unchanged, the seam is the first one's target and the second's origin
        let before: Vec<f64> = (0..=20).map(|i| x(&p, i as f64 * 0.05)).collect();
        assert!(!p.split_key_block(id, 0, 1.0) && p.split_key_block(id, 0, 0.4));
        let c = p.clip(id).unwrap();
        assert_eq!(c.blocks.iter().map(|b| (b.t, b.dur)).collect::<Vec<_>>(), vec![(0.0, 0.4), (0.4, 0.6)]);
        assert_eq!(c.x.keys.len(), 3);
        for (i, v) in before.iter().enumerate() {
            assert!((x(&p, i as f64 * 0.05) - v).abs() < 1.0, "at {}: {} vs {v}", i as f64 * 0.05, x(&p, i as f64 * 0.05));
        }
        // the second half's controls keep its origin and target when they rewrite its keys
        let ctl = p.clip(id).unwrap().blocks[1].ctl.clone().unwrap();
        p.set_block_ctl(id, 1, ctl);
        assert!((x(&p, 0.4) - before[8]).abs() < 3.0 && x(&p, 1.0) == 200.0);
        // flow together: a block placed apart moves up to the chain and starts at the target before it
        p.apply_key_block(id, &find("Slide In Left"), Some(3.0), Some(0.5));
        assert_eq!(p.flow_key_blocks(&[(id, 2), (id, 1)], Match::Next), 1);
        let c = p.clip(id).unwrap();
        assert!((c.blocks[2].t - 1.0).abs() < 1e-9 && c.x.at(1.0) == 200.0, "{:?}", c.x.keys);
        assert!(p.block_seams().iter().any(|s| s.left == (id, 1) && s.right == (id, 2) && s.smooth));
        // link: the fade and the first slide part become one block with both properties, the keys untouched
        p.place_key_block(id, &find("Fade Out"), 0.1, Some(0.3));
        let keys = |p: &Project| (p.clip(id).unwrap().x.keys.clone(), p.clip(id).unwrap().opacity.keys.clone());
        let (before, fade) = (keys(&p), p.clip(id).unwrap().blocks.iter().position(|b| b.name == "Fade Out").unwrap());
        let i = p.link_key_blocks(id, &[0, fade]).unwrap();
        let b = &p.clip(id).unwrap().blocks[i];
        assert_eq!((b.t, b.dur, b.props.len(), p.clip(id).unwrap().blocks.len()), (0.0, 0.4, 2, 3), "{b:?}");
        assert_eq!(keys(&p), before);
        let ctl = b.ctl.clone().unwrap();
        p.set_block_ctl(id, i, ctl);
        assert_eq!(keys(&p), before, "its controls rewrite the same keys");
        // controls from before origin / target load
        let old = serde_json::to_string(&BlockCtl::new(find("Spin"))).unwrap();
        assert!(!old.contains("from") && serde_json::from_str::<BlockCtl>(&old).unwrap().to.is_empty());
    }

    #[test]
    fn a_block_from_before_the_animator_loads_as_plain_keys() {
        let old = r#"{"name":"Spin","t":1.0,"dur":0.8,"props":["rotation"]}"#;
        let b: AppliedBlock = serde_json::from_str(old).unwrap();
        assert!(b.ctl.is_none());
        assert_eq!(serde_json::to_string(&b).unwrap(), old.replace("1.0", "1.0"));
    }

    #[test]
    fn clip_without_blocks_serializes_unchanged() {
        let c = Clip::new(1, ClipKind::Video, "v", 0.0, 1.0);
        assert!(!serde_json::to_string(&c).unwrap().contains("blocks"));
    }
}
