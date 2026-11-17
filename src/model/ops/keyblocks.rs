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
        let at = self.clip_mut(id)?.place_block(block, t0, dur, w, h)?;
        // a simulation with walls runs again now that it can measure the clip's room in the frame
        if block.gen.is_some_and(|g| g.walls()) {
            let c = self.clip(id)?;
            let mine = |b: &AppliedBlock| (b.t - at.0).abs() < 1e-9 && b.ctl.as_ref().is_some_and(|c| c.src.gen == block.gen);
            if let Some((i, ctl)) = c.blocks.iter().position(mine).and_then(|i| Some((i, c.blocks[i].ctl.clone()?))) {
                self.set_block_ctl(id, i, ctl);
            }
        }
        Some(at)
    }

    /// ws:animator: a "Path" block on clip `id` from clip-local `t0`, `dur` seconds long: the clip's Position
    /// (and Rotation with `path.turn`) travels the stroke `path.points`. The block keeps the stroke, so its
    /// Timing / Smoothing / Turn controls (`set_block_ctl`) make the keys again. Returns the block's index.
    pub fn add_path_block(&mut self, id: Id, path: PathCtl, t0: f64, dur: f64) -> Option<usize> {
        let (w, h) = (self.width as f64, self.height as f64);
        if path.points.len() < 2 {
            return None;
        }
        let c = self.clip_mut(id).filter(|c| c.is_visual())?;
        let mut ctl = BlockCtl::new(KeyBlock { name: "Path".into(), dur, ..Default::default() });
        ctl.path = Some(path);
        let (t, _) = c.place_ctl(ctl, t0, Some(dur), w, h)?;
        c.blocks.iter().position(|b| (b.t - t).abs() < 1e-9 && b.ctl.as_ref().is_some_and(|c| c.path.is_some()))
    }

    /// A clip's picture on screen at clip-local `lt`: centre and half size in project px, the frame's centre
    /// at (0, 0).
    /// ponytail: the media fitted to the frame (or the shape's own size) times the clip's scale. Crop, rotation
    /// and a text's real extents are not measured (text and anything without media counts as a 30% x 20% box);
    /// measure the rendered layer if a simulation ever needs the exact outline.
    fn body(&self, c: &Clip, lt: f64) -> (f64, f64, f64, f64) {
        let (w, h) = (self.width as f64, self.height as f64);
        let (hw, hh) = match (&c.shape, self.asset(c.asset)) {
            (Some(s), _) => (s.w.at(lt).abs().max(4.0), s.h.at(lt).abs().max(4.0)),
            (None, Some(a)) if a.width > 0 && a.height > 0 => {
                let fit = (w / a.width as f64).min(h / a.height as f64);
                (a.width as f64 * fit / 2.0, a.height as f64 * fit / 2.0)
            }
            _ => (w * 0.15, h * 0.1),
        };
        let k = c.scale.at(lt);
        let half = |v: f64, max: f64| v.abs().min(max / 2.0);
        (c.x.at(lt), c.y.at(lt), half(hw * k * c.scale_x.at(lt), w), half(hh * k * c.scale_y.at(lt), h))
    }

    /// How far clip `id`'s edges are from the frame's left, right, top and bottom edges at clip-local `lt`,
    /// in frame widths / heights (`Phys::room`).
    fn room(&self, id: Id, lt: f64) -> Option<[f64; 4]> {
        let (w, h) = (self.width as f64, self.height as f64);
        let (x, y, hw, hh) = self.body(self.clip(id)?, lt);
        Some([(x - hw + w / 2.0) / w, (w / 2.0 - x - hw) / w, (y - hh + h / 2.0) / h, (h / 2.0 - y - hh) / h].map(|v| v.max(0.0)))
    }

    /// Loop applied block `i`: play it `count` times (1 = once, up to 50), every other time backwards with
    /// `pingpong`. The block grows to `count` times one cycle's length (as far as the clip lasts; past that the
    /// cycles get shorter). A block of plain or hand-edited keys loops those keys as they are.
    pub fn loop_key_block(&mut self, id: Id, i: usize, count: u32, pingpong: bool) -> bool {
        let Some(c) = self.clip_mut(id) else { return false };
        let Some(b) = c.blocks.get(i).cloned() else { return false };
        let mut ctl = match b.ctl.clone().filter(|c| !c.edited) {
            Some(ctl) => ctl,
            None => BlockCtl::new(abs_block(c, &b.name, &b.props, b.t, b.end())),
        };
        let cycle = b.dur / ctl.cycles() as f64;
        (ctl.loops, ctl.pingpong) = (count.clamp(1, 50), pingpong);
        clear_block_keys(c, i);
        c.blocks[i].dur = (cycle * ctl.cycles() as f64).min(c.duration - b.t).max(0.05);
        self.set_block_ctl(id, i, ctl)
    }

    /// Repeat the keys of `props` in clip-local [from, to] `count` more times right after `to` (keys already
    /// there are replaced; copies past the clip's end are dropped). Where the span ends on another value than
    /// it starts from, the jump back takes one frame. Returns how many keys it added.
    pub fn loop_keys(&mut self, id: Id, props: &[String], from: f64, to: f64, count: u32) -> usize {
        let jump = 1.0 / self.fps.max(1.0);
        let Some(c) = self.clip_mut(id) else { return 0 };
        let (len, count, dur) = (to - from, count.min(50), c.duration);
        if len < 2.0 * jump {
            return 0;
        }
        let end = (to + len * count as f64).min(dur);
        let mut n = 0;
        for p in props {
            let Some(a) = prop_mut(c, p) else { continue };
            let src: Vec<Keyframe> = a.keys.iter().filter(|k| k.t >= from - BK_EPS && k.t <= to + BK_EPS).copied().collect();
            if src.len() < 2 {
                continue;
            }
            a.keys.retain(|k| k.t <= to + BK_EPS || k.t > end + BK_EPS);
            let before = n;
            for j in 1..=count {
                for (i, k) in src.iter().enumerate() {
                    let mut t = k.t + len * j as f64;
                    // the copy's first key lands on the key before it: the same value is one key
                    if let Some(at) = a.key_index_at(t).filter(|_| i == 0) {
                        if (a.keys[at].v - k.v).abs() < 1e-9 {
                            a.keys[at].ease = k.ease;
                            continue;
                        }
                        t += jump;
                    }
                    if t <= dur + BK_EPS {
                        upsert(a, Keyframe { t, ..*k });
                        n += 1;
                    }
                }
            }
            // a block whose span the copies landed in has hand-made keys now: its controls pause
            for b in c.blocks.iter_mut().filter(|b| n > before && b.props.contains(p) && b.end() > to + BK_EPS && b.t < end) {
                if let Some(ctl) = b.ctl.as_mut() {
                    ctl.edited = true;
                }
            }
        }
        n
    }

    /// "Drop with collisions": `ids`' pictures fall from where they are at timeline `t` under gravity, land on
    /// the frame's edge, stay inside it and push each other apart, for `dur` seconds (default 4, at most until
    /// the first of them ends). One simulation, run here at edit time; each clip gets its x / y (and rotation
    /// when it spins) as one "Collide" block of plain keys - one clip's motion depends on the others, so the
    /// block has no live Strength / Ease, only its rigid-body controls (`recollide` runs the drop again).
    /// Every body is a rigid body with `phys` (mass, gravity and its direction, elasticity, friction, air
    /// drag, initial velocity, spin); a clip that already has a Collide block at `t` keeps that block's own.
    /// Returns how many clips it animated.
    /// ponytail: axis-aligned boxes (`body`), mass = area x Mass, a spin that contacts only slow down (no
    /// torque, no tipping), a fixed step (720 steps), impulses along the axis of least overlap. Good for a
    /// handful of layers tumbling into a pile; rotated or round bodies and tall stacks want a real solver.
    pub fn collide_drop(&mut self, ids: &[Id], t: f64, dur: Option<f64>, phys: &Phys) -> usize {
        const N: usize = 120;
        const STEPS: usize = 6;
        struct Body {
            id: Id,
            lt: f64,
            ph: Phys,
            dir: Option<Dir>,
            p: [f64; 2],
            v: [f64; 2],
            half: [f64; 2],
            mass: f64,
            g: [f64; 2],
            w: f64,
            a: f64,
            path: [Vec<f64>; 3],
        }
        let size = [self.width as f64, self.height as f64];
        // who takes part, each with its own controls (an earlier drop's block is replaced)
        let mut who: Vec<(Id, Phys, Option<Dir>)> = Vec::new();
        for &id in ids {
            let Some(c) = self.clip(id).filter(|c| c.is_visual() && c.contains(t)) else { continue };
            if who.iter().any(|w| w.0 == id) {
                continue;
            }
            let lt = t - c.start;
            let old = c.blocks.iter().position(|b| b.name == "Collide" && (b.t - lt).abs() < 1e-6);
            let own = old.and_then(|i| c.blocks[i].ctl.as_ref()).map(|ctl| (ctl.phys, ctl.dir));
            if let Some(i) = old {
                self.remove_key_block(id, i);
            }
            let (ph, dir) = own.unwrap_or((*phys, None));
            who.push((id, ph, dir));
        }
        let mut left = f64::MAX;
        let mut bodies: Vec<Body> = Vec::new();
        for (id, ph, dir) in who {
            let Some(c) = self.clip(id) else { continue };
            let lt = t - c.start;
            let (x, y, hw, hh) = self.body(c, lt);
            left = left.min(c.duration - lt);
            let (gx, gy) = ph.gravity_dir.vec();
            let grav = (1.5 + ph.gravity.clamp(0.0, 1.0) * 4.5) * size[1];
            // the initial velocity: Velocity along the block's direction (none = it just drops)
            let sp = ph.throw.clamp(0.0, 1.0) * 2.0;
            let (dx, dy) = dir.map_or((0.0, 0.0), |d| match d {
                Dir::Left => (-1.0, 0.0),
                Dir::Right => (1.0, 0.0),
                Dir::Up => (0.0, -1.0),
                Dir::Down => (0.0, 1.0),
            });
            bodies.push(Body {
                id,
                lt,
                ph,
                dir,
                p: [x, y],
                v: [dx * sp * size[0], dy * sp * size[1]],
                half: [hw, hh],
                mass: hw * hh * ph.mass.clamp(0.05, 100.0),
                g: [gx as f64 * grav, gy as f64 * grav],
                w: ph.spin * 360.0,
                a: 0.0,
                path: [vec![0.0], vec![0.0], vec![0.0]],
            });
        }
        let dur = dur.unwrap_or(4.0).min(left);
        if bodies.len() < 2 || dur < 0.2 {
            return 0;
        }
        let dt = dur / (N * STEPS) as f64;
        let e_of = |ph: &Phys| 0.1 + ph.bounce.clamp(0.0, 1.0) * 0.7;
        let grip_of = |ph: &Phys| 1.0 - (ph.friction.clamp(0.0, 1.0) * 10.0 * dt).min(1.0);
        // slower than this a contact does not bounce (a pile comes to rest instead of buzzing), and boxes that
        // start inside each other are pushed apart at this many px per pass, not teleported
        let gmax = bodies.iter().map(|b| b.g[0].abs() + b.g[1].abs()).fold(0.0, f64::max);
        let (rest, push) = (gmax * dt * 4.0, 1.5 * size[1] * dt);
        let start: Vec<[f64; 2]> = bodies.iter().map(|b| b.p).collect();
        for _ in 0..N {
            for _ in 0..STEPS {
                for b in &mut bodies {
                    // air drag: the heavier (per area) the body, the less it is slowed
                    let air = 1.0 - (b.ph.drag.clamp(0.0, 1.0) * 3.0 / b.ph.mass.clamp(0.05, 100.0) * dt).min(1.0);
                    for ax in 0..2 {
                        b.v[ax] = (b.v[ax] + b.g[ax] * dt) * air;
                        b.p[ax] += b.v[ax] * dt;
                    }
                    b.w *= air;
                    b.a += b.w * dt;
                }
                for _ in 0..4 {
                    for j in 1..bodies.len() {
                        let (head, tail) = bodies.split_at_mut(j);
                        let b = &mut tail[0];
                        for a in head {
                            let d = [b.p[0] - a.p[0], b.p[1] - a.p[1]];
                            let over = [a.half[0] + b.half[0] - d[0].abs(), a.half[1] + b.half[1] - d[1].abs()];
                            if over[0] <= 0.0 || over[1] <= 0.0 {
                                continue;
                            }
                            // apart along the axis they overlap least on; the impulse is weighted by mass, so
                            // the lighter one moves (and bounces) more
                            let ax = (over[1] < over[0]) as usize;
                            let n = if d[ax] >= 0.0 { 1.0 } else { -1.0 };
                            let (ka, kb) = (b.mass / (a.mass + b.mass), a.mass / (a.mass + b.mass));
                            let fix = over[ax].min(push);
                            a.p[ax] -= n * fix * ka;
                            b.p[ax] += n * fix * kb;
                            let vn = (b.v[ax] - a.v[ax]) * n;
                            if vn < 0.0 {
                                let e = (e_of(&a.ph) + e_of(&b.ph)) / 2.0;
                                let k = -(1.0 + if vn.abs() < rest { 0.0 } else { e }) * vn;
                                a.v[ax] -= n * k * ka;
                                b.v[ax] += n * k * kb;
                                // friction between them: their sliding speeds close in, their spins slow
                                let keep = (grip_of(&a.ph) + grip_of(&b.ph)) / 2.0;
                                let slide = (b.v[1 - ax] - a.v[1 - ax]) * (1.0 - keep);
                                a.v[1 - ax] += slide * ka;
                                b.v[1 - ax] -= slide * kb;
                                a.w *= keep;
                                b.w *= keep;
                            }
                        }
                    }
                    // the frame's edges
                    for b in &mut bodies {
                        for ax in 0..2 {
                            let lim = (size[ax] / 2.0 - b.half[ax]).max(0.0);
                            if b.p[ax].abs() > lim {
                                let s = b.p[ax].signum();
                                b.p[ax] = s * lim;
                                if b.v[ax] * s > 0.0 {
                                    b.v[ax] = if b.v[ax].abs() < rest { 0.0 } else { -b.v[ax] * e_of(&b.ph) };
                                }
                                let keep = grip_of(&b.ph);
                                b.v[1 - ax] *= keep;
                                b.w *= keep;
                            }
                        }
                    }
                }
            }
            for (b, s) in bodies.iter_mut().zip(&start) {
                for ax in 0..2 {
                    b.path[ax].push((b.p[ax] - s[ax]) / size[ax]);
                }
                b.path[2].push(b.a);
            }
        }
        let mut done = 0;
        for b in bodies {
            let mut tracks = vec![fit("y", &b.path[1], BlockVal::Add)];
            if b.path[0].iter().any(|v| v.abs() > 1e-9) {
                tracks.insert(0, fit("x", &b.path[0], BlockVal::Add));
            }
            if b.path[2].iter().any(|v| v.abs() > 1e-6) {
                tracks.push(fit("rotation", &b.path[2], BlockVal::Add));
            }
            let block = KeyBlock { name: "Collide".into(), dur, tracks, gen: None };
            if self.place_key_block(b.id, &block, b.lt, Some(dur)).is_none() {
                continue;
            }
            let made = self.clip_mut(b.id).and_then(|c| c.blocks.iter_mut().find(|x| (x.t - b.lt).abs() < 1e-9 && x.name == "Collide"));
            if let Some(ctl) = made.and_then(|x| x.ctl.as_mut()) {
                // Strength / Ease would bend a motion that only fits together as simulated; the block keeps
                // its body's controls for the next run
                (ctl.edited, ctl.phys, ctl.dir) = (true, b.ph, b.dir);
            }
            done += 1;
        }
        done
    }

    /// Change the rigid-body controls of Collide block `i` of clip `id` (its mass, elasticity, velocity ...)
    /// and run that drop again for every clip that took part in it (each keeps its own controls). Returns how
    /// many clips it animated (0 = not a Collide block).
    pub fn recollide(&mut self, id: Id, i: usize, phys: Phys, dir: Option<Dir>) -> usize {
        let Some(c) = self.clip_mut(id) else { return 0 };
        let start = c.start;
        let Some(b) = c.blocks.get_mut(i).filter(|b| b.name == "Collide") else { return 0 };
        let (t, dur) = (start + b.t, b.dur);
        if let Some(ctl) = b.ctl.as_mut() {
            (ctl.phys, ctl.dir) = (phys, dir);
        }
        let took_part = |c: &Clip| c.blocks.iter().any(|b| b.name == "Collide" && (c.start + b.t - t).abs() < 1e-6);
        let ids: Vec<Id> = self.all_clips().filter(|(_, c)| took_part(c)).map(|(_, c)| c.id).collect();
        self.collide_drop(&ids, t, Some(dur), &phys)
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
        // a simulation with walls: how far the clip is from the frame's edges as the block starts
        if let Some(room) = ctl.src.gen.filter(|g| g.walls()).and_then(|_| self.room(id, b.t)) {
            ctl.phys.room = room;
        }
        let Some(c) = self.clip_mut(id) else { return false };
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
    /// (`match_seam`; across a cut only the speeds). Blocks on different clips that run at the same time (one
    /// layer pops in over another popping out) are locked together instead: the later one takes the earlier
    /// one's start, length and - when one was picked by hand - ease. Returns how many pairs it joined.
    pub fn flow_key_blocks(&mut self, sel: &[(Id, usize)], mode: Match) -> usize {
        let at = |p: &Project, (id, i): (Id, usize)| p.clip(id).and_then(|c| Some(c.start + c.blocks.get(i)?.t));
        let mut sel: Vec<(Id, usize)> = sel.iter().copied().filter(|&s| at(self, s).is_some()).collect();
        sel.sort_by(|a, b| at(self, *a).unwrap_or(0.0).total_cmp(&at(self, *b).unwrap_or(0.0)));
        let mut n = 0;
        for w in sel.windows(2) {
            let (l, r) = (w[0], w[1]);
            let span = |p: &Project, (id, i): (Id, usize)| {
                let c = p.clip(id).unwrap();
                (c.start + c.blocks[i].t, c.start + c.blocks[i].end())
            };
            let (l0, l1) = span(self, l);
            let (lc, rc) = (self.clip(l.0).unwrap(), self.clip(r.0).unwrap());
            if l.0 != r.0 && rc.start < lc.end() - BK_EPS && lc.start < rc.end() - BK_EPS {
                let ease = self.clip(l.0).unwrap().blocks[l.1].ctl.as_ref().and_then(|c| c.ease);
                let rs = self.clip(r.0).unwrap().start;
                self.retime_key_block(r.0, r.1, l0 - rs, l1 - l0);
                let ctl = self.clip(r.0).unwrap().blocks[r.1].ctl.clone().filter(|c| !c.edited && !c.is_sim());
                if let (Some(e), Some(mut ctl)) = (ease, ctl) {
                    (ctl.ease, ctl.in_speed, ctl.out_speed) = (Some(e), None, None);
                    self.set_block_ctl(r.0, r.1, ctl);
                }
                n += 1;
                continue;
            }
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
    fn looping_a_block_grows_it_and_looping_keys_repeats_them() {
        let (mut p, id) = proj();
        p.apply_key_block(id, &find("Pulse"), Some(1.0), None).unwrap(); // 0.6 s, scale 1 -> 1.08 -> 1
        assert!(p.loop_key_block(id, 0, 3, false));
        let c = p.clip(id).unwrap();
        assert!((c.blocks[0].dur - 1.8).abs() < 1e-9 && c.blocks[0].ctl.as_ref().unwrap().loops == 3);
        let ts: Vec<f64> = c.scale.keys.iter().map(|k| k.t).collect();
        assert_eq!(ts.len(), 7, "{ts:?}");
        assert!((c.scale.at(1.3) - 1.08).abs() < 1e-9 && (c.scale.at(2.5) - 1.08).abs() < 1e-9);
        // back to once: one cycle's length again; a count the clip has no room for is clamped to the clip
        assert!(p.loop_key_block(id, 0, 1, false));
        assert!((p.clip(id).unwrap().blocks[0].dur - 0.6).abs() < 1e-9 && p.clip(id).unwrap().scale.keys.len() == 3);
        assert!(p.loop_key_block(id, 0, 50, true));
        let b = &p.clip(id).unwrap().blocks[0];
        assert!((b.end() - 6.0).abs() < 1e-9 && b.ctl.as_ref().unwrap().pingpong, "{b:?}");
        // a block of plain keys (no controls) loops the keys it has
        let (mut p, id) = proj();
        p.apply_key_block(id, &find("Fade In"), Some(0.0), None).unwrap();
        p.clip_mut(id).unwrap().blocks[0].ctl = None;
        assert!(p.loop_key_block(id, 0, 2, true));
        let c = p.clip(id).unwrap();
        assert_eq!(c.opacity.keys.iter().map(|k| k.v).collect::<Vec<_>>(), vec![0.0, 1.0, 0.0]);
        assert!((c.blocks[0].dur - 1.0).abs() < 1e-9);
        // keys made by hand: x goes 0 -> 100 over a second, three more times after it, each jumping back
        let (mut p, id) = proj();
        let c = p.clip_mut(id).unwrap();
        c.x.key_at(1.0, 0.0);
        c.x.key_at(2.0, 100.0);
        assert_eq!(p.loop_keys(id, &["x".into()], 1.0, 2.0, 3), 6);
        let c = p.clip(id).unwrap();
        assert_eq!(c.x.keys.len(), 8);
        assert!((c.x.at(2.5) - 50.0).abs() < 2.0 && (c.x.at(4.99) - 100.0).abs() < 2.0, "{:?}", c.x.keys);
        // a wave that returns to its start shares the seam key; copies past the clip's end are dropped
        let (mut p, id) = proj();
        let c = p.clip_mut(id).unwrap();
        for (t, v) in [(0.0, 0.0), (1.0, 50.0), (2.0, 0.0)] {
            c.y.key_at(t, v);
        }
        assert_eq!(p.loop_keys(id, &["y".into()], 0.0, 2.0, 5), 4);
        let ts: Vec<f64> = p.clip(id).unwrap().y.keys.iter().map(|k| k.t).collect();
        assert_eq!(ts, vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn gravity_fall_measures_the_clips_room_and_lands_on_the_frames_bottom_edge() {
        let (mut p, id) = proj();
        let (w, h) = (p.width as f64, p.height as f64);
        // no media: a 30% x 20% box, moved up and left of the centre
        let c = p.clip_mut(id).unwrap();
        (c.x.value, c.y.value) = (-0.2 * w, -0.25 * h);
        p.apply_key_block(id, &find("Gravity Fall"), Some(0.5), None).unwrap();
        let c = p.clip(id).unwrap();
        let room = c.blocks[0].ctl.as_ref().unwrap().phys.room;
        for (got, want) in room.iter().zip([0.15, 0.55, 0.15, 0.65]) {
            assert!((got - want).abs() < 1e-9, "{room:?}");
        }
        // it ends resting with its bottom edge on the frame's: centre at h/2 - 10% of h
        let end = c.blocks[0].end();
        assert!((c.y.at(end) - 0.4 * h).abs() < 1e-6 && (c.y.at(end - 0.05) - 0.4 * h).abs() < 0.02 * h, "{}", c.y.at(end));
        assert!(c.y.keys.iter().all(|k| k.v <= 0.4 * h + 1e-6), "never below the frame");
        assert!(!c.x.is_animated(), "a straight drop leaves x alone");
        // moved lower, a re-run (any control change) measures the room again
        let mut ctl = c.blocks[0].ctl.clone().unwrap();
        ctl.phys.bounce = 0.9;
        p.clip_mut(id).unwrap().y.value = 0.0;
        p.set_block_ctl(id, 0, ctl);
        assert!((p.clip(id).unwrap().blocks[0].ctl.as_ref().unwrap().phys.room[3] - 0.4).abs() < 1e-9);
    }

    #[test]
    fn drop_with_collisions_ends_with_no_overlap_and_everything_inside_the_frame() {
        let mut p = Project::new();
        let (w, h) = (p.width as f64, p.height as f64);
        // four layers at the same time, on four tracks, their boxes overlapping where they start
        for (n, (x, y, s)) in [(-0.1, -0.3, 1.0), (0.0, -0.1, 1.2), (0.12, -0.35, 0.8), (0.05, 0.2, 1.0)].into_iter().enumerate() {
            let mut c = Clip::new(10 + n as Id, ClipKind::Image, format!("layer {n}"), 0.0, 6.0);
            (c.x.value, c.y.value, c.scale.value) = (x * w, y * h, s);
            if p.tracks.len() <= n {
                p.tracks.push(p.tracks[0].clone());
                p.tracks[n].clips.clear();
            }
            p.tracks[n].clips.push(c);
        }
        let ids: Vec<Id> = (10..14).collect();
        let drop = Phys { bounce: 0.4, gravity: 0.6, throw: 0.0, spin: 0.0, ..Phys::default() };
        assert_eq!(p.collide_drop(&ids, 1.0, None, &drop), 4);
        let boxes: Vec<(f64, f64, f64, f64)> = ids.iter().map(|&id| p.body(p.clip(id).unwrap(), 5.0)).collect();
        for (i, a) in boxes.iter().enumerate() {
            assert!(a.0.abs() + a.2 <= w / 2.0 + 1.0 && a.1.abs() + a.3 <= h / 2.0 + 1.0, "{i} is outside the frame: {a:?}");
            for (j, b) in boxes.iter().enumerate().skip(i + 1) {
                let apart = (a.0 - b.0).abs() >= a.2 + b.2 - 2.0 || (a.1 - b.1).abs() >= a.3 + b.3 - 2.0;
                assert!(apart, "{i} and {j} overlap at the end: {a:?} {b:?}");
            }
        }
        // every key stays inside the frame, not just the end; each clip got one Collide block over the 4 s
        for &id in &ids {
            let c = p.clip(id).unwrap();
            let (_, _, hw, hh) = p.body(c, 1.0);
            assert!(c.x.keys.iter().all(|k| k.v.abs() + hw <= w / 2.0 + 1.0), "{id} x");
            assert!(c.y.keys.iter().all(|k| k.v.abs() + hh <= h / 2.0 + 1.0), "{id} y");
            assert_eq!(c.blocks.len(), 1);
            let b = &c.blocks[0];
            assert!(b.name == "Collide" && (b.t - 1.0).abs() < 1e-9 && (b.dur - 4.0).abs() < 1e-9, "{b:?}");
            assert!(b.ctl.as_ref().is_some_and(|c| c.edited), "no live controls: the motion depends on the others");
        }
        // at rest: nothing moves in the last tenth of a second
        for &id in &ids {
            let c = p.clip(id).unwrap();
            assert!((c.y.at(5.0) - c.y.at(4.9)).abs() < 2.0 && (c.x.at(5.0) - c.x.at(4.9)).abs() < 2.0, "{id} still moves");
        }
        // fewer than two pictures under t: nothing to collide
        assert_eq!(p.collide_drop(&ids[..1], 1.0, None, &drop), 0);
        assert_eq!(p.collide_drop(&ids, 9.0, None, &drop), 0);
    }

    /// Two equal boxes start half inside each other, side by side: the collision pushes the light one further
    /// than the heavy one, and changing a body's controls runs the drop again for both.
    #[test]
    fn mass_weighs_the_collision_and_a_collide_block_keeps_its_rigid_body_controls() {
        let mut p = Project::new();
        let w = p.width as f64;
        for (id, x) in [(20, -0.1), (21, 0.1)] {
            let mut c = Clip::new(id, ClipKind::Image, "box", 0.0, 6.0);
            c.x.value = x * w;
            p.tracks[0].clips.push(c);
        }
        let drop = Phys { throw: 0.0, spin: 0.0, ..Phys::default() };
        assert_eq!(p.collide_drop(&[20, 21], 0.0, Some(3.0), &drop), 2);
        let moved = |p: &Project, id: Id, x0: f64| (p.clip(id).unwrap().x.at(3.0) - x0 * w).abs();
        let (a, b) = (moved(&p, 20, -0.1), moved(&p, 21, 0.1));
        assert!(a > 0.02 * w && (a - b).abs() < 0.01 * w, "equal masses part equally: {a} {b}");
        // the left box five times as heavy: it gives way less, the other more
        let heavy = Phys { mass: 5.0, spin: 0.4, ..drop };
        assert_eq!(p.recollide(20, 0, heavy, None), 2);
        let (a2, b2) = (moved(&p, 20, -0.1), moved(&p, 21, 0.1));
        assert!(a2 < a * 0.6 && b2 > b * 1.3 && b2 > a2 * 3.0, "{a2} {b2} (were {a} {b})");
        // each block remembers its own body; the heavy one spins (rotation baked), the other does not
        let ctl = |id: Id| p.clip(id).unwrap().blocks[0].ctl.clone().unwrap();
        assert_eq!((ctl(20).phys.mass, ctl(21).phys.mass, ctl(20).edited), (5.0, 1.0, true));
        assert!(p.clip(20).unwrap().rotation.is_animated() && !p.clip(21).unwrap().rotation.is_animated());
        assert_eq!(p.clip(20).unwrap().blocks.len(), 1, "the earlier drop's block is replaced, not stacked");
        // a block that is not a Collide block is left alone
        p.apply_key_block(21, &find("Fade In"), Some(5.0), None).unwrap();
        let i = p.clip(21).unwrap().blocks.iter().position(|b| b.name == "Fade In").unwrap();
        assert_eq!(p.recollide(21, i, drop, None), 0);
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
    fn flowing_blocks_on_two_layers_locks_them_to_the_same_time() {
        let mut p = Project::new();
        p.tracks[0].clips.push(Clip::new(1, ClipKind::Video, "a", 0.0, 5.0));
        p.tracks[0].clips.push(Clip::new(2, ClipKind::Video, "b", 0.5, 5.0)); // which track does not matter here
        p.place_key_block(1, &find("Pop In"), 1.0, Some(0.4));
        p.place_key_block(2, &find("Pop Out"), 1.0, Some(0.7));
        assert_eq!(p.flow_key_blocks(&[(1, 0), (2, 0)], Match::Both), 1);
        // and gaps between clips on a track close, leaving cuts a transition can sit on
        let mut q = Project::new();
        q.tracks[0].clips.push(Clip::new(1, ClipKind::Video, "a", 0.0, 2.0));
        q.tracks[0].clips.push(Clip::new(2, ClipKind::Video, "b", 3.0, 2.0));
        q.tracks[0].clips.push(Clip::new(3, ClipKind::Video, "c", 9.0, 1.0));
        assert_eq!(q.make_transitionable(&[]), 2);
        assert_eq!((q.clip(2).unwrap().start, q.clip(3).unwrap().start), (2.0, 4.0));
        let (a, b) = (&p.clip(1).unwrap().blocks[0], &p.clip(2).unwrap().blocks[0]);
        assert!((a.t - 1.0).abs() < 1e-9 && (a.dur - 0.4).abs() < 1e-9, "{a:?}");
        // the same timeline time: 1.0 on a clip that starts at 0 = 0.5 on one that starts at 0.5
        assert!((b.t - 0.5).abs() < 1e-9 && (b.dur - 0.4).abs() < 1e-9, "{b:?}");
    }

    /// A stroke: right 300 px in 1 s, a 2 s pause, then down 300 px in 1 s (a point every 10 px, none while
    /// the pen stands still - what the Draw tool records).
    fn stroke() -> Vec<(f32, f32, f32)> {
        let mut v: Vec<(f32, f32, f32)> = (0..=30).map(|i| (i as f32 * 10.0, 0.0, i as f32 / 30.0)).collect();
        v.extend((1..=30).map(|i| (300.0, i as f32 * 10.0, 3.0 + i as f32 / 30.0)));
        v
    }

    #[test]
    fn a_path_block_travels_its_shape_at_one_speed_or_as_it_was_drawn() {
        let (mut p, id) = proj();
        let path = PathCtl { points: stroke(), ..Default::default() };
        let i = p.add_path_block(id, path.clone(), 1.0, 4.0).expect("a block");
        let at = |p: &Project, t: f64| (p.clip(id).unwrap().x.at(t), p.clip(id).unwrap().y.at(t));
        let b = p.clip(id).unwrap().blocks[i].clone();
        assert_eq!((b.name.as_str(), b.t, b.dur), ("Path", 1.0, 4.0));
        assert_eq!(b.props, ["x", "y"]);
        // Shape: the same distance in every tenth of the block (600 px over 4 s), corner or not
        let steps: Vec<f64> = (0..40)
            .map(|k| {
                let (a, b) = (at(&p, 1.0 + k as f64 * 0.1), at(&p, 1.1 + k as f64 * 0.1));
                (b.0 - a.0).abs() + (b.1 - a.1).abs() // the path only runs along the axes
            })
            .collect();
        assert!(steps.iter().all(|d| (d - 15.0).abs() < 0.5), "{steps:?}");
        assert_eq!((at(&p, 1.0), at(&p, 5.0)), ((0.0, 0.0), (300.0, 300.0)));
        // an ease shapes the travel along the same path: slow at both ends, the middle where it was
        let mut ctl = b.ctl.clone().unwrap();
        ctl.ease = Some(Ease::EaseInOut);
        p.set_block_ctl(id, i, ctl.clone());
        assert!(at(&p, 1.4).0 < 30.0, "eased in: {:?}", at(&p, 1.4));
        let mid = at(&p, 3.0);
        assert!((mid.0 - 300.0).abs() < 1.0 && mid.1.abs() < 1.0, "{mid:?}");
        // As drawn: the stroke's 4 s over the block's 4 s - it stands at the corner through the pause
        ctl.path.as_mut().unwrap().as_drawn = true;
        p.set_block_ctl(id, i, ctl.clone());
        for t in [2.1, 3.0, 3.9] {
            let v = at(&p, t);
            assert!((v.0 - 300.0).abs() < 1e-6 && v.1 < 11.0, "paused at the corner at {t}: {v:?}");
        }
        assert!((at(&p, 1.5).0 - 150.0).abs() < 1.0);
        // stretched to twice as long, the pause is twice as long too
        p.retime_key_block(id, i, 0.0, 6.0);
        assert!((at(&p, 3.0).0 - 300.0).abs() < 1e-6 && at(&p, 4.4).1 < 11.0);
        // Turn along path: it faces right, then down
        ctl.path.as_mut().unwrap().turn = true;
        p.set_block_ctl(id, i, ctl);
        let c = p.clip(id).unwrap();
        assert_eq!(c.blocks[i].props, ["x", "y", "rotation"]);
        assert!(c.rotation.at(0.5).abs() < 1.0 && (c.rotation.at(5.5) - 90.0).abs() < 1.0);
        // deleting the block takes its keys
        p.remove_key_block(id, i);
        let c = p.clip(id).unwrap();
        assert!(!c.x.is_animated() && !c.y.is_animated() && !c.rotation.is_animated());
    }

    #[test]
    fn smoothing_a_path_rounds_it_and_keeps_its_ends() {
        let raw = PathCtl { points: stroke(), ..Default::default() };
        let soft = PathCtl { smoothing: 1.0, ..raw.clone() };
        assert_eq!(raw.smoothed().len(), raw.points.len());
        let (a, b) = (raw.smoothed(), soft.smoothed());
        assert_eq!((a[0], a[60]), ((0.0, 0.0, 0.0), (300.0, 300.0, 4.0)), "0 = the points as drawn");
        assert_eq!((b[0], b[60]), (a[0], a[60]), "the ends stay");
        // the corner at (300, 0) is cut: the smoothed path passes inside it
        assert!(b[30].0 < 290.0 && b[30].1 > 10.0, "{:?}", b[30]);
        // and the block made from it still starts and ends where the stroke does
        let (mut p, id) = proj();
        p.add_path_block(id, soft, 0.0, 2.0).unwrap();
        let c = p.clip(id).unwrap();
        assert_eq!(((c.x.at(0.0), c.y.at(0.0)), (c.x.at(2.0), c.y.at(2.0))), ((0.0, 0.0), (300.0, 300.0)));
        // a block saved before Path blocks existed loads (no `path`)
        let old: BlockCtl = serde_json::from_str(r#"{"src":{"name":"Fade In","dur":0.5,"tracks":[]}}"#).unwrap();
        assert!(old.path.is_none());
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
