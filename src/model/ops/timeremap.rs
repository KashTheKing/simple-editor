//! ---- ws:time-remap ----
//! Time remapping on the timeline: turn a clip's `TimeMap` on or off and edit it. An edit that changes
//! the clip's length ripples the rest of its track (and its linked clips' tracks) by the same amount, and
//! a linked pair (picture + sound) takes every edit together. Each op is one undo step at the caller.

use crate::model::*;

impl Project {
    /// The source frame rate a clip's map snaps its picture lookups to (its asset's; a nested sequence
    /// runs at its own rate).
    fn remap_fps(&self, c: &Clip) -> f64 {
        match c.kind {
            ClipKind::Video => self.asset(c.asset).map_or(0.0, |a| a.fps),
            ClipKind::Sequence => self.sequence(c.sequence).map_or(self.fps, |s| s.fps),
            _ => 0.0,
        }
    }

    /// Clips that may carry a time map: footage with time of its own.
    fn remappable(c: &Clip) -> bool {
        matches!(c.kind, ClipKind::Video | ClipKind::Audio | ClipKind::Sequence)
    }

    /// Turn time remapping on for `id` and its linked clips, keeping exactly what they play now (speed,
    /// ramp, reverse or freeze become keys). False if none of them can be remapped or a track is locked.
    pub fn time_remap_enable(&mut self, id: Id) -> bool {
        let ids = self.expand_links(&[id]);
        let mut any = false;
        for id in ids {
            let Some((ti, ci)) = self.find(id) else { continue };
            if self.locked_of(ti) || !Self::remappable(&self.tracks[ti].clips[ci]) {
                continue;
            }
            let fps = self.remap_fps(&self.tracks[ti].clips[ci]);
            let c = &mut self.tracks[ti].clips[ci];
            if c.time_map.is_none() {
                c.enable_time_map(fps);
                any = true;
            }
        }
        any
    }

    /// Turn time remapping off for `id` and its linked clips: one constant speed over the same source
    /// range, the same length.
    pub fn time_remap_disable(&mut self, id: Id) -> bool {
        let ids = self.expand_links(&[id]);
        let mut any = false;
        for id in ids {
            let Some((ti, ci)) = self.find(id) else { continue };
            if self.locked_of(ti) {
                continue;
            }
            let c = &mut self.tracks[ti].clips[ci];
            if c.time_map.is_some() {
                c.disable_time_map();
                any = true;
            }
        }
        any
    }

    /// Apply `edit` to the time map of `id` and of its remapped linked clips (turning remapping on for
    /// `id` first), then ripple each one's track by the change in its length. Returns that change.
    pub fn time_remap_edit(&mut self, id: Id, edit: impl Fn(&mut TimeMap)) -> Result<f64, String> {
        let c = self.clip(id).ok_or("no such clip")?;
        if !Self::remappable(c) {
            return Err("only video, audio and sequence clips can be remapped".into());
        }
        let ti = self.track_of(id).ok_or("no such clip")?;
        if self.locked_of(ti) {
            return Err("the clip's track is locked".into());
        }
        self.time_remap_enable(id);
        let ids: Vec<Id> = self.expand_links(&[id]);
        let mut delta = 0.0;
        for cid in ids {
            let Some((ti, ci)) = self.find(cid) else { continue };
            if self.locked_of(ti) || self.tracks[ti].clips[ci].time_map.is_none() {
                continue;
            }
            let old_end = self.tracks[ti].clips[ci].end();
            let c = &mut self.tracks[ti].clips[ci];
            let mut m = c.time_map.clone().unwrap();
            edit(&mut m);
            if !m.normalize() {
                return Err("a time map needs two keys or more".into());
            }
            c.time_map = Some(m);
            c.sync_time_map();
            let d = c.end() - old_end;
            if cid == id {
                delta = d;
            }
            if d.abs() > EPS {
                for o in self.tracks[ti].clips.iter_mut().filter(|o| o.id != cid && o.start >= old_end - EPS) {
                    o.start = (o.start + d).max(0.0);
                }
                self.shift_time(old_end, d, &[ti]);
            }
        }
        self.tidy();
        Ok(delta)
    }

    /// Replace `id`'s time map keys (clip-local `t`, source `s`) - the keys are sorted and the first is
    /// moved to t = 0; the last sets the clip's length (rippled). Returns the change in length.
    pub fn time_remap_set_keys(&mut self, id: Id, keys: Vec<TimeKey>) -> Result<f64, String> {
        if keys.len() < 2 {
            return Err("a time map needs two keys or more".into());
        }
        self.time_remap_edit(id, |m| m.keys = keys.clone())
    }

    /// Freeze `id`'s picture at timeline time `t` for `len` seconds (the rest of the clip and its track
    /// move later). Returns the change in length.
    pub fn time_remap_hold(&mut self, id: Id, t: f64, len: f64) -> Result<f64, String> {
        let c = self.clip(id).ok_or("no such clip")?;
        if !(len.is_finite() && len > 0.0) {
            return Err("len must be a positive number of seconds".into());
        }
        let l = c.local(t);
        self.time_remap_edit(id, |m| {
            m.hold(l, len);
        })
    }

    /// Rewind the stretch of `id` between the keys either side of timeline time `t`: it plays backwards
    /// right after itself, then forwards again (the clip grows by twice that stretch). Returns the change.
    pub fn time_remap_reverse(&mut self, id: Id, t: f64) -> Result<f64, String> {
        let c = self.clip(id).ok_or("no such clip")?;
        let l = c.local(t);
        self.time_remap_edit(id, |m| {
            let i = m.seg(l);
            m.rewind(i);
        })
    }

    /// Add a key to `id`'s time map at timeline time `t` without changing what plays. Returns its index.
    pub fn time_remap_add_key(&mut self, id: Id, t: f64) -> Result<usize, String> {
        let l = self.clip(id).ok_or("no such clip")?.local(t);
        self.time_remap_edit(id, |m| {
            m.insert_exact(l);
        })?;
        let m = self.clip(id).and_then(|c| c.time_map.as_ref()).ok_or("no time map")?;
        Ok(m.keys.iter().position(|k| (k.t - l).abs() < MIN_SEG).unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proj() -> (Project, Id, Id, Id) {
        let a = Asset {
            id: 1,
            path: "C:/v.mp4".into(),
            kind: ClipKind::Video,
            duration: 60.0,
            width: 1280,
            height: 720,
            fps: 30.0,
            audio_streams: vec![AudioStreamInfo { index: 0, channels: 2, sample_rate: 48000, ..Default::default() }],
            codec: "h264".into(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
            html: None,
            proxy: None,
        };
        let mut p = Project::from_media(a);
        let (v, s) = (p.tracks[0].clips[0].id, p.tracks[1].clips[0].id);
        for ti in 0..2 {
            let c = &mut p.tracks[ti].clips[0];
            c.duration = 4.0;
            c.src_in = 10.0;
        }
        let next = p.new_id();
        let mut n = Clip::new(next, ClipKind::Video, "next", 4.0, 2.0);
        n.asset = 1;
        p.tracks[0].clips.push(n);
        (p, v, s, next)
    }

    #[test]
    fn enable_keeps_what_plays_and_disable_restores_speed() {
        let (mut p, v, s, _) = proj();
        p.set_speed(&[v], 2.0, false);
        let before: Vec<f64> = (0..20).map(|i| p.clip(v).unwrap().src_time(i as f64 * 0.1)).collect();
        assert!(p.time_remap_enable(v));
        let c = p.clip(v).unwrap();
        assert!(c.time_map.is_some() && c.speed == 1.0 && (c.duration - 2.0).abs() < 1e-9);
        for (i, b) in before.iter().enumerate() {
            // the map lands on the middle of the source frame the old mapping fell in
            let got = c.src_time(i as f64 * 0.1);
            assert!((got - b).abs() <= 0.5 / 30.0 + 1e-9, "{got} vs {b}");
            assert_eq!((got * 30.0).floor(), (b * 30.0 + 1e-6).floor());
        }
        assert!(p.clip(s).unwrap().time_map.is_some(), "the linked sound follows");
        assert!(p.time_remap_disable(v));
        let c = p.clip(v).unwrap();
        assert!(c.time_map.is_none() && (c.speed - 2.0).abs() < 1e-9 && (c.src_in - 10.0).abs() < 1e-9);
    }

    #[test]
    fn hold_ripples_the_track_and_the_linked_clip() {
        let (mut p, v, a, next) = proj();
        let d = p.time_remap_hold(v, 1.0, 0.5).unwrap();
        assert!((d - 0.5).abs() < 1e-9);
        assert!((p.clip(v).unwrap().duration - 4.5).abs() < 1e-9);
        assert!((p.clip(a).unwrap().duration - 4.5).abs() < 1e-9, "linked audio holds too");
        assert!((p.clip(next).unwrap().start - 4.5).abs() < 1e-9, "the next clip rippled");
        let c = p.clip(v).unwrap();
        let held = c.src_time(1.2);
        assert_eq!(held.to_bits(), c.src_time(1.45).to_bits(), "one frame for the whole hold");
        assert!((c.src_time(2.0) - c.time_map.as_ref().unwrap().frame_time(2.0)).abs() < 1e-12);
        assert!((c.time_map.as_ref().unwrap().eval(2.0) - 11.5).abs() < 1e-9);
    }

    #[test]
    fn reverse_section_grows_the_clip_and_plays_back() {
        let (mut p, v, _, next) = proj();
        p.time_remap_add_key(v, 1.0).unwrap();
        p.time_remap_add_key(v, 2.0).unwrap();
        let d = p.time_remap_reverse(v, 1.5).unwrap();
        assert!((d - 2.0).abs() < 1e-9);
        let m = p.clip(v).unwrap().time_map.clone().unwrap();
        assert!((m.eval(3.0) - 11.0).abs() < 1e-9 && m.speed(2.5) < 0.0);
        assert!((p.clip(next).unwrap().start - 6.0).abs() < 1e-9);
    }

    #[test]
    fn split_and_trim_keep_the_mapping() {
        let (mut p, v, _, _) = proj();
        p.time_remap_hold(v, 1.0, 1.0).unwrap(); // 0..1 plays 10..11, 1..2 holds 11, 2..5 plays 11..14
        let whole = p.clip(v).unwrap().clone();
        let mut left = whole.clone();
        let right = left.split(1.5, 99).unwrap();
        for i in 0..50 {
            let t = i as f64 * 0.1;
            let c = if t < 1.5 { &left } else { &right };
            assert_eq!(c.src_time(t).to_bits(), whole.src_time(t).to_bits(), "at {t}");
        }
        assert!((right.src_in - 11.0).abs() < 1e-9);
        let mut tr = whole.clone();
        tr.trim_start(0.5, f64::INFINITY);
        tr.trim_end(4.0, f64::INFINITY);
        assert!((tr.duration - 3.5).abs() < 1e-9);
        assert!((tr.src_time(3.0) - whole.src_time(3.0)).abs() < 1e-9);
        assert!((tr.time_map.as_ref().unwrap().duration() - 3.5).abs() < 1e-9);
    }

    #[test]
    fn freeze_frame_on_a_remapped_clip_shows_the_frame() {
        let (mut p, v, _, _) = proj();
        p.time_remap_hold(v, 1.0, 0.5).unwrap();
        let want = p.clip(v).unwrap().src_time(2.5);
        let frozen = p.freeze_at(2.5, &[v]);
        let c = frozen.iter().filter_map(|&f| p.clip(f)).find(|c| c.kind == ClipKind::Video).unwrap();
        assert!(c.time_map.is_none() && c.freeze == Some(want));
        assert_eq!(c.src_time(c.start + 0.7).to_bits(), want.to_bits(), "a still, not the map");
    }

    #[test]
    fn retime_reverse_leaves_a_remapped_clip_alone() {
        let (mut p, v, _, _) = proj();
        p.time_remap_enable(v);
        let before = p.clip(v).unwrap().src_time(1.0);
        assert!(p.set_speed(&[v], 1.0, true));
        let c = p.clip(v).unwrap();
        assert!(!c.reverse && (c.src_time(1.0) - before).abs() < 1e-9);
    }

    #[test]
    fn paste_speed_from_a_plain_clip_drops_the_map_and_sizes_the_clip() {
        let (mut p, v, _, next) = proj();
        p.time_remap_hold(v, 1.0, 0.5).unwrap();
        p.set_speed(&[next], 2.0, false);
        let src = p.copy_attributes(next).unwrap();
        let set = AttrSet { speed: true, ..Default::default() };
        p.paste_attributes(&src, &[v], set);
        let c = p.clip(v).unwrap();
        assert!(c.time_map.is_none() && (c.speed - 2.0).abs() < 1e-9);
        assert!((c.duration * c.speed - c.src_len()).abs() < 1e-9, "length matches the source window at 2x");
    }

    #[test]
    fn locked_track_refuses() {
        let (mut p, v, _, _) = proj();
        let ti = p.track_of(v).unwrap();
        p.tracks[ti].locked = true;
        assert!(p.time_remap_hold(v, 1.0, 1.0).is_err());
    }

    #[test]
    fn save_load_keeps_the_map_and_old_projects_load_without_one() {
        let (mut p, v, _, _) = proj();
        let old = p.to_json();
        assert!(!old.contains("time_map"), "nothing written for a plain clip");
        let back = Project::from_json(&old).unwrap();
        assert!(back.clip(v).unwrap().time_map.is_none());
        p.time_remap_hold(v, 1.0, 0.5).unwrap();
        p.time_remap_reverse(v, 3.0).unwrap();
        let back = Project::from_json(&p.to_json()).unwrap();
        let (a, b) = (p.clip(v).unwrap(), back.clip(v).unwrap());
        assert_eq!(a.time_map, b.time_map);
        assert!((a.duration - b.duration).abs() < 1e-12);
        for i in 0..80 {
            assert_eq!(a.src_time(i as f64 * 0.1).to_bits(), b.src_time(i as f64 * 0.1).to_bits());
        }
        // a broken map in a hand-edited file is dropped, the clip still loads
        let compact = serde_json::to_string(&p).unwrap();
        let bad = compact.replacen("\"time_map\":{\"keys\":[", "\"time_map\":{\"keys\":[{\"t\":0.0,\"s\":1.0}],\"x_keys\":[", 1);
        assert_ne!(bad, compact);
        let back = Project::from_json(&bad).unwrap();
        assert!(back.clip(v).unwrap().time_map.is_none());
    }

    #[test]
    fn speed_ramp_blocks_still_work_and_convert() {
        let (mut p, v, _, _) = proj();
        let block = find_block(&[], "Slow-mo Hit").unwrap();
        assert!(p.apply_key_block(v, &block, Some(1.0), Some(2.0)).is_some());
        let c = p.clip(v).unwrap().clone();
        assert!(c.speed_curve.is_animated() && c.time_map.is_none(), "the block writes the speed ramp, as before");
        assert!(c.rate(2.0) < 0.5, "slow in the middle of the hit: {}", c.rate(2.0));
        p.time_remap_enable(v);
        let r = p.clip(v).unwrap();
        let m = r.time_map.as_ref().unwrap();
        for i in 0..40 {
            let t = i as f64 * 0.1;
            // eased ramp keys become smooth segments: within a frame of the ramp it replaced
            assert!((m.eval(t) - c.src_time(t)).abs() < 1.0 / 30.0, "at {t}: {} vs {}", m.eval(t), c.src_time(t));
        }
        assert!(m.speed(2.0) < 0.5);
    }
}
