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
        let (w, h) = (self.width as f64, self.height as f64);
        let c = self.clip_mut(id)?;
        let mut t0 = at.unwrap_or_else(|| c.blocks.iter().map(|b| b.end()).fold(0.0, f64::max));
        while let Some(b) = c.blocks.iter().find(|b| t0 >= b.t - BK_EPS && t0 < b.end() - BK_EPS) {
            t0 = b.end();
        }
        let t0 = t0.clamp(0.0, c.duration);
        let dur = dur.unwrap_or(block.dur).min(c.duration - t0);
        if dur < 0.01 {
            return None;
        }
        let props = block.write(c, t0, dur, w, h);
        if props.is_empty() {
            return None;
        }
        c.blocks.push(AppliedBlock { name: block.name.clone(), t: t0, dur, props });
        c.blocks.sort_by(|a, b| a.t.total_cmp(&b.t));
        Some((t0, dur))
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
        true
    }

    /// Delete applied block `i` and the keys it wrote (a seam key shared with a chained neighbour stays).
    pub fn remove_key_block(&mut self, id: Id, i: usize) -> bool {
        let Some(c) = self.clip_mut(id) else { return false };
        if i >= c.blocks.len() {
            return false;
        }
        for (p, k, shared) in c.block_keys(i) {
            if let Some(a) = prop_mut(c, &p) {
                if let (Some(j), false) = (a.key_index_at(k.t), shared) {
                    // the constant `value` was never touched by apply, so an emptied property falls back to it
                    a.keys.remove(j);
                }
            }
        }
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
            assert!(b.dur > 0.0 && !b.tracks.is_empty(), "{}", b.name);
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
    fn clip_without_blocks_serializes_unchanged() {
        let c = Clip::new(1, ClipKind::Video, "v", 0.0, 1.0);
        assert!(!serde_json::to_string(&c).unwrap().contains("blocks"));
    }
}
