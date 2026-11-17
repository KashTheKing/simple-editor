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
    /// follows, so it is left alone).
    pub fn ease_key_block(&mut self, id: Id, i: usize, ease: Ease) -> bool {
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
