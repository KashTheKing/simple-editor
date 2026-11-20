//! feat/procedural-blocks: Convert to Keyframes (a procedural block baked into ordinary keys at the project
//! frame rate) and its reverse, Make Procedural, for a converted block whose keys were not edited since.

use super::keyblocks::{abs_block, cut_at};
use crate::model::*;

impl Project {
    /// Convert procedural block `i` of clip `id` into keys at the project frame rate: it becomes a normal key
    /// block, the motion unchanged. The first and last key equal the property's own value there (the offset
    /// is zero at both ends). The caller makes it one undo step.
    pub fn bake_key_block(&mut self, id: Id, i: usize) -> bool {
        let fps = if self.fps > 0.0 { self.fps } else { 30.0 };
        let Some(c) = self.clip_mut(id) else { return false };
        let Some(b) = c.blocks.get(i).cloned() else { return false };
        let Some(pc) = b.proc().copied() else { return false };
        let (t0, t1) = (b.t, b.end());
        // the project's frames inside the block (on the timeline's grid), plus both ends exactly
        let mut times = vec![t0];
        let mut k = ((c.start + t0) * fps + 1e-6).ceil() as i64;
        loop {
            let t = k as f64 / fps - c.start;
            if t >= t1 - 1e-6 {
                break;
            }
            if t > t0 + 1e-6 {
                times.push(t);
            }
            k += 1;
        }
        times.push(t1);
        let mut under = Vec::new();
        let mut baked = Vec::new();
        for p in &b.props {
            let Some(a) = prop_mut(c, p) else { continue };
            let mine: Vec<ProcLayer> = a.procs.iter().filter(|l| l.pid == pc.pid).copied().collect();
            if mine.is_empty() {
                continue;
            }
            // what the motion is now, before any key moves
            let vals: Vec<f64> = times.iter().map(|&t| apply_layers(&mine, a.keyed_at(t), t)).collect();
            // the keys as they were: those in the block and the one before it (the cut below re-eases it)
            let before = a.keys.iter().rposition(|k| k.t < t0 - BK_EPS);
            under.extend(before.map(|j| (p.clone(), a.keys[j])));
            under.extend(a.keys.iter().filter(|k| k.t >= t0 - BK_EPS && k.t <= t1 + BK_EPS).map(|k| (p.clone(), *k)));
            // keys at the ends that keep the curve outside the block as it was
            cut_at(a, t0);
            cut_at(a, t1);
            a.keys.retain(|k| !(k.t >= t0 - BK_EPS && k.t < t1 - BK_EPS));
            for (j, (&t, &v)) in times.iter().zip(&vals).enumerate() {
                match a.key_index_at(t) {
                    // the end key's ease belongs to what follows the block: keep it
                    Some(at) if j + 1 == times.len() => a.keys[at].v = v,
                    _ => upsert(a, Keyframe { t, v, ease: Ease::Linear }),
                }
            }
            a.procs.retain(|l| l.pid != pc.pid);
            baked.extend(a.keys.iter().filter(|k| k.t >= t0 - BK_EPS && k.t <= t1 + BK_EPS).map(|k| (p.clone(), *k)));
        }
        if baked.is_empty() {
            return false;
        }
        let mut ctl = BlockCtl::new(abs_block(c, &b.name, &b.props, t0, t1));
        ctl.unbaked = Some(Box::new(Unbaked { proc: pc, under, baked }));
        c.blocks[i].ctl = Some(ctl);
        true
    }

    /// Whether block `i` of clip `id` was made by Convert to Keyframes and its keys are still exactly what it
    /// wrote (so Make Procedural gives the same motion back).
    pub fn can_unbake(&self, id: Id, i: usize) -> bool {
        let Some(c) = self.clip(id) else { return false };
        let Some(b) = c.blocks.get(i) else { return false };
        let Some(u) = b.ctl.as_ref().and_then(|c| c.unbaked.as_ref()) else { return false };
        let now: Vec<(String, Keyframe)> = b
            .props
            .iter()
            .filter_map(|p| Some((p, prop_ref(c, p)?)))
            .flat_map(|(p, a)| a.keys.iter().filter(|k| k.t >= b.t - BK_EPS && k.t <= b.end() + BK_EPS).map(move |k| (p.clone(), *k)))
            .collect();
        now == u.baked
    }

    /// Make Procedural: turn block `i` of clip `id` back into the procedural block it was converted from (only
    /// while its keys are unedited, `can_unbake`). The keys it sampled over come back.
    pub fn unbake_key_block(&mut self, id: Id, i: usize) -> bool {
        if !self.can_unbake(id, i) {
            return false;
        }
        let (w, h) = (self.width as f64, self.height as f64);
        let Some(c) = self.clip_mut(id) else { return false };
        let b = c.blocks[i].clone();
        let Some(u) = b.ctl.as_ref().and_then(|c| c.unbaked.clone()) else { return false };
        let (t0, t1) = (b.t, b.end());
        for p in &b.props {
            let Some(a) = prop_mut(c, p) else { continue };
            a.keys.retain(|k| !(k.t >= t0 - BK_EPS && k.t <= t1 + BK_EPS));
            for (_, k) in u.under.iter().filter(|(q, _)| q == p) {
                upsert(a, *k);
            }
        }
        let src = KeyBlock { name: b.name.clone(), dur: b.dur, proc: Some(u.proc.kind), ..Default::default() };
        let mut ctl = BlockCtl::new(src);
        let mut pc = u.proc;
        pc.pid = c.next_pid();
        ctl.proc = Some(pc);
        let props = c.write_proc(&pc, t0, b.dur, w, h);
        let ab = &mut c.blocks[i];
        ab.props = props;
        ab.ctl = Some(ctl);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proj() -> (Project, Id) {
        let mut p = Project::new();
        p.fps = 30.0;
        let c = Clip::new(900, ClipKind::Video, "v", 0.0, 6.0);
        p.tracks[0].clips.push(c);
        (p, 900)
    }
    fn find(name: &str) -> KeyBlock {
        builtin_blocks().into_iter().find(|b| b.name == name).unwrap()
    }

    #[test]
    fn applying_a_procedural_block_writes_no_keys() {
        let (mut p, id) = proj();
        let (x0, y0) = (p.clip(id).unwrap().x.value, p.clip(id).unwrap().y.value);
        p.apply_key_block(id, &find("Camera Shake"), Some(1.0), Some(2.0)).unwrap();
        let c = p.clip(id).unwrap();
        assert!(c.x.keys.is_empty() && c.y.keys.is_empty() && c.scale.keys.is_empty() && c.rotation.keys.is_empty());
        assert!(c.blocks[0].is_proc());
        assert_eq!(c.x.at(1.0), x0);
        assert_eq!(c.x.at(3.0), x0);
        assert_eq!(c.y.at(3.0), y0);
        assert_ne!(c.x.at(2.0), x0, "it moves in the middle");
        assert!(c.scale.at(2.0) > 1.0, "Camera Shake zooms to cover the edges");
        assert_eq!(c.scale.at(1.0), 1.0);
        assert_eq!(c.scale.at(3.0), 1.0);
    }

    #[test]
    fn a_bake_equals_sampling_at_frame_times() {
        let (mut p, id) = proj();
        p.clip_mut(id).unwrap().x.key_at(0.0, 0.0);
        p.clip_mut(id).unwrap().x.key_at(5.0, 100.0);
        p.apply_key_block(id, &find("Shake"), Some(1.0), Some(1.5)).unwrap();
        let before = p.clip(id).unwrap().clone();
        assert!(p.bake_key_block(id, 0));
        let c = p.clip(id).unwrap();
        assert!(c.x.procs.is_empty() && c.y.procs.is_empty());
        assert!(!c.blocks[0].is_proc() && c.blocks[0].ctl.as_ref().unwrap().unbaked.is_some());
        // every frame inside the block: the keys give what the procedural block gave
        for f in 30..=75 {
            let t = f as f64 / 30.0;
            for (a, b) in [(&before.x, &c.x), (&before.y, &c.y)] {
                assert!((a.at(t) - b.at(t)).abs() < 1e-9, "frame {f}: {} vs {}", a.at(t), b.at(t));
            }
        }
        // first and last key equal the property's own value there
        let ks: Vec<&Keyframe> = c.x.keys.iter().filter(|k| k.t >= 1.0 - 1e-9 && k.t <= 2.5 + 1e-9).collect();
        assert!((ks[0].v - before.x.keyed_at(1.0)).abs() < 1e-9);
        assert!((ks.last().unwrap().v - before.x.keyed_at(2.5)).abs() < 1e-9);
        // outside the block nothing changed
        for t in [0.0, 0.5, 0.99, 2.51, 4.0, 5.0] {
            assert!((before.x.at(t) - c.x.at(t)).abs() < 1e-9, "{t}");
        }
    }

    #[test]
    fn make_procedural_undoes_an_unedited_bake_only() {
        let (mut p, id) = proj();
        p.apply_key_block(id, &find("Wobble"), Some(1.0), Some(2.0)).unwrap();
        let before = p.clip(id).unwrap().clone();
        assert!(p.bake_key_block(id, 0));
        assert!(p.can_unbake(id, 0));
        assert!(p.unbake_key_block(id, 0));
        let c = p.clip(id).unwrap();
        assert!(c.blocks[0].is_proc() && c.rotation.keys.is_empty() && c.scale.keys.is_empty());
        for i in 0..100 {
            let t = i as f64 * 0.04;
            assert!((before.rotation.at(t) - c.rotation.at(t)).abs() < 1e-9);
        }
        // edited keys: no way back
        assert!(p.bake_key_block(id, 0));
        let k = p.clip(id).unwrap().rotation.keys[3].t;
        p.clip_mut(id).unwrap().rotation.key_at(k, 45.0);
        assert!(!p.can_unbake(id, 0) && !p.unbake_key_block(id, 0));
    }

    #[test]
    fn moving_stretching_and_removing_carry_the_layers() {
        let (mut p, id) = proj();
        p.apply_key_block(id, &find("Wobble"), Some(1.0), Some(1.0)).unwrap();
        let speed = p.clip(id).unwrap().blocks[0].proc().unwrap().speed;
        assert!(p.retime_key_block(id, 0, 2.0, 3.0));
        let c = p.clip(id).unwrap();
        let l = c.rotation.procs[0];
        assert_eq!((l.t, l.dur, l.speed), (2.0, 3.0, speed), "longer, same speed");
        assert_eq!(c.rotation.at(2.0), 0.0);
        assert_eq!(c.rotation.at(5.0), 0.0);
        assert!(p.remove_key_block(id, 0));
        let c = p.clip(id).unwrap();
        assert!(c.rotation.procs.is_empty() && c.scale.procs.is_empty() && c.blocks.is_empty());
    }

    #[test]
    fn controls_rewrite_the_layers() {
        let (mut p, id) = proj();
        p.apply_key_block(id, &find("Shake"), Some(0.0), Some(2.0)).unwrap();
        let mut ctl = p.clip(id).unwrap().blocks[0].ctl.clone().unwrap();
        let pid = ctl.proc.unwrap().pid;
        let a = p.clip(id).unwrap().x.at(1.0);
        {
            let pc = ctl.proc.as_mut().unwrap();
            pc.strength = 2.0;
            pc.axes.rotation = true;
        }
        assert!(p.set_block_ctl(id, 0, ctl));
        let c = p.clip(id).unwrap();
        assert!((c.x.at(1.0) - 2.0 * a).abs() < 1e-9, "twice as far");
        assert_eq!(c.rotation.procs.len(), 1);
        assert_eq!(c.blocks[0].proc().unwrap().pid, pid);
        assert!(c.blocks[0].props.contains(&"rotation".to_string()));
    }

    #[test]
    fn procedural_blocks_survive_save_and_load() {
        let (mut p, id) = proj();
        p.apply_key_block(id, &find("Camera Shake"), Some(0.5), Some(2.0)).unwrap();
        let json = serde_json::to_string(&p).unwrap();
        let q: Project = serde_json::from_str(&json).unwrap();
        let (a, b) = (p.clip(id).unwrap(), q.clip(id).unwrap());
        for i in 0..60 {
            let t = i as f64 * 0.05;
            assert_eq!(a.x.at(t), b.x.at(t));
            assert_eq!(a.scale.at(t), b.scale.at(t));
        }
        assert_eq!(a.blocks, b.blocks);
    }

    #[test]
    fn old_projects_with_baked_shake_keys_load_unchanged() {
        // a clip as an older build saved it: Camera Shake as a simulated block with its keys written
        let (mut p, id) = proj();
        let old = KeyBlock { name: "Camera Shake".into(), dur: 1.0, gen: Some(Gen::Shake), ..Default::default() };
        p.place_key_block(id, &old, 1.0, Some(1.0)).unwrap();
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("procs") && !json.contains("\"proc\""), "nothing procedural in an old project");
        let q: Project = serde_json::from_str(&json).unwrap();
        let (a, b) = (p.clip(id).unwrap(), q.clip(id).unwrap());
        assert!(!a.x.keys.is_empty() && a.x.keys == b.x.keys);
        assert!(!b.blocks[0].is_proc() && b.x.procs.is_empty());
        for i in 0..60 {
            let t = i as f64 * 0.05;
            assert_eq!(a.x.at(t), b.x.at(t));
        }
        // and its controls still re-run the old simulation
        let ctl = b.blocks[0].ctl.clone().unwrap();
        let mut q = q;
        assert!(q.set_block_ctl(id, 0, ctl));
        assert!(!q.clip(id).unwrap().x.keys.is_empty());
    }
}
