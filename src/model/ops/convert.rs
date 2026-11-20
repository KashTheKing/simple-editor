//! ---- ws:animator ----
//! Whole-timeline animation tools, built for an agent that can only call tools (no file reads, no shell):
//! - the audit (`Clip::key_audit`, `Clip::hand_runs`, `Project::video_cuts`): which properties carry keys,
//!   which of those keys a block owns, the runs of keys made by hand, and the cuts with and without transitions;
//! - `convert_keys_to_blocks`: every run of hand keys becomes an animation block - a named one (Fade In, Slide,
//!   Ken Burns, Pop, Zoom Punch, Shake, ...) with controls where one reproduces the motion exactly, else a
//!   "Custom" block of those very keys. Every property is sampled before and after: a conversion that would
//!   change the motion is not made;
//! - `spans_from_transitions`: each run of cuts that already carry the same transition becomes one transition
//!   span. Hard cuts stay hard. Every span is tried on a copy first: one that would move a clip or touch a cut
//!   outside its run is split until it doesn't.

use crate::model::ops::spans::MIN_SPAN;
use crate::model::*;

/// What a block may write on a picture clip / a sound clip (`KeyBlock::write`).
pub const PICTURE_PROPS: [&str; 8] = ["x", "y", "scale", "scale_x", "scale_y", "rotation", "opacity", "speed"];
pub const SOUND_PROPS: [&str; 3] = ["volume", "pan", "speed"];

pub fn clip_props(c: &Clip) -> &'static [&'static str] {
    if c.is_visual() {
        &PICTURE_PROPS
    } else {
        &SOUND_PROPS
    }
}

/// One property's keys on a clip.
#[derive(Clone, Debug, PartialEq)]
pub struct PropAudit {
    pub prop: &'static str,
    pub keys: usize,
    /// Keys no block owns (made by hand).
    pub hand: usize,
    pub t0: f64,
    pub t1: f64,
    /// Driven by a link (path / expression / pick whip): its keys are left alone.
    pub linked: bool,
}

/// A run of keys made by hand: overlapping or touching stretches where properties change, no block in between.
#[derive(Clone, Debug, PartialEq)]
pub struct HandRun {
    pub t0: f64,
    pub t1: f64,
    pub props: Vec<String>,
    /// (property, from, to) of every moving stretch in it.
    pub segs: Vec<(String, f64, f64)>,
}

fn in_block(c: &Clip, p: &str, t0: f64, t1: f64) -> bool {
    c.blocks.iter().any(|b| b.props.iter().any(|q| q == p) && b.t <= t0 + BK_EPS && b.end() >= t1 - BK_EPS)
}

/// Stretches from one key to the next on `p` where the value changes and no block owns them.
fn hand_moves(c: &Clip, p: &str) -> Vec<(f64, f64)> {
    let Some(a) = prop_ref(c, p) else { return Vec::new() };
    if !a.link.is_none() {
        return Vec::new();
    }
    a.keys
        .windows(2)
        .filter(|w| (w[1].v - w[0].v).abs() > 1e-9 && w[1].t - w[0].t > BK_EPS && !in_block(c, p, w[0].t, w[1].t))
        .map(|w| (w[0].t, w[1].t))
        .collect()
}

impl Clip {
    /// Every keyed property of this clip (only those a block can animate on its kind).
    pub fn key_audit(&self) -> Vec<PropAudit> {
        clip_props(self)
            .iter()
            .filter_map(|&p| {
                let a = prop_ref(self, p)?;
                let (f, l) = (a.keys.first()?, a.keys.last()?);
                let hand = a.keys.iter().filter(|k| !in_block(self, p, k.t, k.t)).count();
                Some(PropAudit { prop: p, keys: a.keys.len(), hand, t0: f.t, t1: l.t, linked: !a.link.is_none() })
            })
            .collect()
    }

    /// The runs of hand keys, in time order. A run never spans a block of one of its properties.
    pub fn hand_runs(&self) -> Vec<HandRun> {
        let mut segs: Vec<(f64, f64, &str)> =
            clip_props(self).iter().flat_map(|&p| hand_moves(self, p).into_iter().map(move |(a, b)| (a, b, p))).collect();
        segs.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut runs: Vec<HandRun> = Vec::new();
        for (a, b, p) in segs {
            let joins = runs.last().is_some_and(|r| {
                a <= r.t1 + BK_EPS
                    && !self.blocks.iter().any(|bl| {
                        bl.t < b.max(r.t1) - BK_EPS
                            && bl.end() > r.t0 + BK_EPS
                            && bl.props.iter().any(|q| q == p || r.props.contains(q))
                    })
            });
            if joins {
                let r = runs.last_mut().unwrap();
                r.t1 = r.t1.max(b);
                if !r.props.iter().any(|q| q == p) {
                    r.props.push(p.to_string());
                }
                r.segs.push((p.to_string(), a, b));
            } else {
                runs.push(HandRun { t0: a, t1: b, props: vec![p.to_string()], segs: vec![(p.to_string(), a, b)] });
            }
        }
        runs
    }
}

// ---------- keys -> blocks ----------

/// A block the conversion made (or, on a dry run, would make).
#[derive(Clone, Debug, PartialEq)]
pub struct MadeBlock {
    pub clip: Id,
    pub name: String,
    pub t: f64,
    pub dur: f64,
    pub props: Vec<String>,
    /// A named block with controls (false = "Custom": the hand keys as they were).
    pub named: bool,
    pub strength: f64,
    pub ease: Option<Ease>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Skipped {
    pub clip: Id,
    pub t: f64,
    pub why: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConvertReport {
    /// Clips that got blocks.
    pub clips: usize,
    pub made: Vec<MadeBlock>,
    pub skipped: Vec<Skipped>,
    /// Clips looked at.
    pub scanned: usize,
}

/// The same motion: every property in `props` sampled over the clip (a 60 fps grid, every key time and eight
/// points between each two keys) differs by at most 1e-5 relative.
pub fn same_motion(a: &Clip, b: &Clip, props: &[&str]) -> bool {
    let dur = a.duration.max(b.duration).max(0.0);
    for &p in props {
        let (Some(x), Some(y)) = (prop_ref(a, p), prop_ref(b, p)) else { continue };
        let mut kt: Vec<f64> = x.keys.iter().chain(&y.keys).map(|k| k.t).collect();
        kt.sort_by(f64::total_cmp);
        let n = ((dur * 60.0) as usize).clamp(32, 1200);
        let mut ts: Vec<f64> = (0..=n).map(|i| dur * i as f64 / n as f64).collect();
        for w in kt.windows(2) {
            ts.extend((0..=8).map(|j| w[0] + (w[1] - w[0]) * j as f64 / 8.0));
        }
        ts.extend(kt.iter().flat_map(|&t| [t - 1e-4, t + 1e-4]));
        for t in ts {
            let (u, v) = (x.base_at(t), y.base_at(t));
            if (u - v).abs() > 1e-5 * (1.0 + u.abs().max(v.abs())) {
                return false;
            }
        }
    }
    true
}

fn unit(prop: &str, w: f64, h: f64) -> f64 {
    match prop {
        "x" => w,
        "y" => h,
        _ => 1.0,
    }
}

fn val(c: &Clip, p: &str, t: f64) -> f64 {
    prop_ref(c, p).map_or(0.0, |a| a.base_at(t))
}

fn place(c: &Clip, ctl: BlockCtl, a: f64, dur: f64, w: f64, h: f64) -> Option<Clip> {
    let mut t = c.clone();
    t.place_ctl(ctl, a, Some(dur), w, h)?;
    Some(t)
}

/// How strongly `c` moves over [a, a + dur] compared with `src` as designed: at the design key that strays
/// furthest from where it starts, the observed offset over the designed one. 1 when nothing tells.
fn strength_of(c: &Clip, src: &KeyBlock, a: f64, dur: f64, w: f64, h: f64) -> f64 {
    for tr in &src.tracks {
        let dev = |k: &BlockKey| match k.v {
            BlockVal::Add(v) => v.abs(),
            BlockVal::Mul(v) => (v - 1.0).abs(),
            BlockVal::Abs(_) => 0.0,
        };
        let Some(k) = tr.keys.iter().max_by(|x, y| dev(x).total_cmp(&dev(y))).filter(|k| dev(k) > 1e-9) else { continue };
        let (base, obs) = (val(c, &tr.prop, a), val(c, &tr.prop, a + k.f * dur));
        let s = match k.v {
            BlockVal::Add(v) => (obs - base) / unit(&tr.prop, w, h) / v,
            BlockVal::Mul(v) if base.abs() > 1e-9 => (obs / base - 1.0) / (v - 1.0),
            _ => continue,
        };
        if s.is_finite() && s > 1e-6 {
            return s;
        }
    }
    1.0
}

/// The ease every moving stretch of `props` in [a, b] starts with, when they all share one.
fn common_ease(c: &Clip, props: &[String], a: f64, b: f64) -> Option<Ease> {
    let mut e = None;
    for p in props {
        for w in prop_ref(c, p)?.keys.windows(2) {
            if w[0].t >= a - BK_EPS && w[1].t <= b + BK_EPS && (w[1].v - w[0].v).abs() > 1e-9 {
                match e {
                    None => e = Some(w[0].ease),
                    Some(x) if x != w[0].ease => return None,
                    _ => {}
                }
            }
        }
    }
    e
}

fn sign(d: f64, scale: f64) -> i8 {
    if d.abs() <= 1e-6 * (1.0 + scale) {
        0
    } else if d > 0.0 {
        1
    } else {
        -1
    }
}

/// A builtin block (only its tracks of `props`) that, with its controls set (strength, ease, origin / target),
/// gives exactly the motion `c` has over [a, b]. The motion must go the block's way (a Fade In never stands in
/// for a fade out). Returns the clip with it placed and its controls.
fn named_fit(c: &Clip, props: &[String], a: f64, b: f64, w: f64, h: f64) -> Option<(Clip, BlockCtl)> {
    let dur = b - a;
    let refs: Vec<&str> = props.iter().map(|s| s.as_str()).collect();
    let e_obs = common_ease(c, props, a, b);
    for blk in builtin_blocks() {
        if blk.gen.is_some() || !props.iter().all(|p| blk.tracks.iter().any(|t| t.prop == *p)) {
            continue;
        }
        let mut src = blk.clone();
        src.tracks.retain(|t| props.contains(&t.prop));
        let design = BlockCtl::new(src.clone());
        // the block goes the same way as the keys, property by property
        let Some(d) = place(c, design.clone(), a, dur, w, h) else { continue };
        let same_way = refs.iter().all(|p| {
            let (o0, o1, d0, d1) = (val(c, p, a), val(c, p, b), val(&d, p, a), val(&d, p, b));
            let sc = o0.abs().max(o1.abs()).max(d0.abs()).max(d1.abs());
            sign(o1 - o0, sc) == sign(d1 - d0, sc)
        });
        if !same_way {
            continue;
        }
        let k = strength_of(c, &src, a, dur, w, h);
        let mut tries: Vec<(f64, Option<Ease>, bool)> = Vec::new();
        for s in [k, 1.0] {
            for e in [None, e_obs] {
                for ft in [false, true] {
                    if !tries.contains(&(s, e, ft)) {
                        tries.push((s, e, ft));
                    }
                }
            }
        }
        for (s, e, ft) in tries {
            let mut ctl = design.clone();
            (ctl.strength, ctl.ease) = (s, e);
            if ft {
                ctl.from = props.iter().map(|p| (p.clone(), val(c, p, a))).collect();
                ctl.to = props.iter().map(|p| (p.clone(), val(c, p, b))).collect();
            }
            if let Some(t) = place(c, ctl.clone(), a, dur, w, h) {
                if same_motion(c, &t, &refs) {
                    return Some((t, ctl));
                }
            }
        }
    }
    None
}

/// The keys of `props` in [a, b] as a block: relative values (so its strength control works) or, when that
/// does not come back exactly, the absolute ones.
fn capture(c: &Clip, props: &[String], a: f64, b: f64, w: f64, h: f64, rel: bool) -> KeyBlock {
    let dur = (b - a).max(BK_EPS);
    let tracks = props
        .iter()
        .filter_map(|p| {
            let an = prop_ref(c, p)?;
            let base = an.base_at(a);
            let keys: Vec<BlockKey> = an
                .keys
                .iter()
                .filter(|k| k.t >= a - BK_EPS && k.t <= b + BK_EPS)
                .map(|k| BlockKey {
                    f: ((k.t - a) / dur).clamp(0.0, 1.0),
                    v: match p.as_str() {
                        _ if !rel => BlockVal::Abs(k.v),
                        "x" | "y" | "rotation" => BlockVal::Add((k.v - base) / unit(p, w, h)),
                        _ if base.abs() > 1e-6 => BlockVal::Mul(k.v / base),
                        _ => BlockVal::Abs(k.v),
                    },
                    ease: k.ease,
                })
                .collect();
            (!keys.is_empty()).then(|| BlockTrack { prop: p.clone(), keys })
        })
        .collect();
    KeyBlock { name: "Custom".into(), dur, tracks, gen: None, proc: None }
}

fn custom_fit(c: &Clip, props: &[String], a: f64, b: f64, w: f64, h: f64) -> Option<(Clip, BlockCtl)> {
    let refs: Vec<&str> = props.iter().map(|s| s.as_str()).collect();
    for rel in [true, false] {
        let ctl = BlockCtl::new(capture(c, props, a, b, w, h, rel));
        if let Some(t) = place(c, ctl.clone(), a, b - a, w, h) {
            if same_motion(c, &t, &refs) {
                return Some((t, ctl));
            }
        }
    }
    None
}

fn made(id: Id, ctl: &BlockCtl, a: f64, b: f64, props: &[String], named: bool) -> MadeBlock {
    MadeBlock {
        clip: id,
        name: ctl.src.name.clone(),
        t: a,
        dur: b - a,
        props: props.to_vec(),
        named,
        strength: ctl.strength,
        ease: ctl.ease,
    }
}

/// `p`'s moving stretches in `run`, touching ones joined.
fn prop_pieces(run: &HandRun, p: &str) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::new();
    for s in run.segs.iter().filter(|s| s.0 == p) {
        match out.last_mut() {
            Some(l) if s.1 <= l.1 + BK_EPS => l.1 = l.1.max(s.2),
            _ => out.push((s.1, s.2)),
        }
    }
    out
}

/// Convert one clip's hand-key runs on `cur`. Pushes what it made / skipped.
fn convert_clip(cur: &mut Clip, w: f64, h: f64, out: &mut ConvertReport) {
    let id = cur.id;
    for run in cur.hand_runs() {
        if run.t0 < -BK_EPS || run.t1 > cur.duration + BK_EPS || run.t1 - run.t0 < 0.01 {
            out.skipped.push(Skipped { clip: id, t: run.t0, why: "keys outside the clip's length (or closer than 0.01 s)".into() });
            continue;
        }
        if let Some((t, ctl)) = named_fit(cur, &run.props, run.t0, run.t1, w, h) {
            *cur = t;
            out.made.push(made(id, &ctl, run.t0, run.t1, &run.props, true));
            continue;
        }
        // properties that move together but match no one block: each property's own moves, a named block where
        // one fits, else a Custom block of those keys
        for p in &run.props {
            for (a, b) in prop_pieces(&run, p) {
                let one = std::slice::from_ref(p);
                if let Some((t, ctl)) = named_fit(cur, one, a, b, w, h) {
                    *cur = t;
                    out.made.push(made(id, &ctl, a, b, one, true));
                } else if let Some((t, ctl)) = custom_fit(cur, one, a, b, w, h) {
                    *cur = t;
                    out.made.push(made(id, &ctl, a, b, one, false));
                } else {
                    out.skipped.push(Skipped { clip: id, t: a, why: format!("no block reproduces the {p} keys exactly") });
                }
            }
        }
    }
}

impl Project {
    /// ws:animator: turn the hand keys (keys no block owns) of clips `ids` into animation blocks, the motion
    /// unchanged: a named block with controls where one fits exactly, else a Custom block of the same keys.
    /// Keys a block already owns, linked properties and keys past the clip's ends are left alone. Each clip is
    /// checked by sampling every property before and after; a clip that would change is left as it was.
    /// `dry_run`: report only. One call is one undo step (the tool's).
    pub fn convert_keys_to_blocks(&mut self, ids: &[Id], dry_run: bool) -> ConvertReport {
        let (w, h) = (self.width as f64, self.height as f64);
        let mut out = ConvertReport::default();
        for &id in ids {
            let Some(orig) = self.clip(id).cloned() else { continue };
            out.scanned += 1;
            let mut cur = orig.clone();
            let mut part = ConvertReport::default();
            convert_clip(&mut cur, w, h, &mut part);
            if part.made.is_empty() {
                out.skipped.extend(part.skipped);
                continue;
            }
            if !same_motion(&orig, &cur, clip_props(&orig)) {
                out.skipped.push(Skipped { clip: id, t: 0.0, why: "the whole-clip motion check failed: left as it was".into() });
                continue;
            }
            out.clips += 1;
            out.made.extend(part.made);
            out.skipped.extend(part.skipped);
            if !dry_run {
                if let Some(c) = self.clip_mut(id) {
                    *c = cur;
                }
            }
        }
        out
    }
}

// ---------- transitions -> spans ----------

/// A cut on a video track: two clips that sit end to end.
#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    pub track: usize,
    pub t: f64,
    pub left: Id,
    pub right: Id,
    /// The transition on it, if any.
    pub tr: Option<Transition>,
    /// The span that made that transition.
    pub span: Option<Id>,
}

/// One planned (or made) span.
#[derive(Clone, Debug, PartialEq)]
pub struct SpanPlan {
    pub t: f64,
    pub dur: f64,
    pub kind: TransitionKind,
    pub name: String,
    pub cut_dur: f64,
    pub ease: Ease,
    pub direction: u8,
    pub shader: String,
    pub params: Vec<f64>,
    /// The cuts it covers: right clip, time, and whether it was a hard cut it gives the transition to.
    pub cuts: Vec<(Id, f64, bool)>,
    /// The transitions' lengths before (min, max).
    pub before: (f64, f64),
    /// Hard cuts inside the run it gives the transition to (`include_hard_cuts` only).
    pub hard: usize,
    /// The span made (0 on a dry run).
    pub span_id: Id,
}

#[derive(Clone, Debug)]
pub struct SpanOpts {
    /// Only cuts in [from, to] (timeline s).
    pub range: Option<(f64, f64)>,
    /// Split a run where two of its cuts are further apart than this (s).
    pub merge_gap: Option<f64>,
    /// Hard cuts between faded cuts of a run get the run's transition too (default: they stay hard).
    pub include_hard_cuts: bool,
    /// How far a transition's length may be from the run's first one and still join it (fraction, default 0.3).
    pub tolerance: f64,
}

impl Default for SpanOpts {
    fn default() -> Self {
        Self { range: None, merge_gap: None, include_hard_cuts: false, tolerance: 0.3 }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpansReport {
    pub spans: Vec<SpanPlan>,
    pub skipped: Vec<(f64, String)>,
    /// Hard cuts left as they are.
    pub hard_cuts: usize,
}

fn black(c: [u8; 4]) -> bool {
    c[..3] == [0, 0, 0]
}

/// Two transitions a span can make as one (the length aside).
fn same_look(a: &Transition, b: &Transition) -> bool {
    a.kind == b.kind
        && a.ease == b.ease
        && (!a.kind.has_direction() || a.direction == b.direction)
        && a.shader == b.shader
        && a.params == b.params
}

/// A transition a span can make: a span's Fade to Color dips to black.
fn spannable(t: &Transition) -> bool {
    t.kind != TransitionKind::FadeToColor || black(t.color)
}

impl Project {
    /// Every cut on a video track, in time order.
    pub fn video_cuts(&self) -> Vec<Cut> {
        let mut out = Vec::new();
        for ti in self.video_tracks() {
            let tr = &self.tracks[ti];
            for c in &tr.clips {
                let Some(l) = tr.left_of(c) else { continue };
                let x = tr.transitions.iter().find(|x| x.right == c.id && x.edge == TransitionEdge::Cut).cloned();
                let span = x.as_ref().and_then(|x| self.spans.iter().find(|s| s.made.contains(&x.id)).map(|s| s.id));
                out.push(Cut { track: ti, t: c.start, left: l.id, right: c.id, tr: x, span });
            }
        }
        out.sort_by(|a, b| a.t.total_cmp(&b.t).then(a.track.cmp(&b.track)));
        out
    }

    /// The spans `spans_from_transitions` would make, before they are tried (`spans_from_transitions` with
    /// `dry_run` gives the ones that survive the check).
    fn span_runs(&self, o: &SpanOpts) -> (Vec<SpanPlan>, Vec<(f64, String)>, usize) {
        let cuts = self.video_cuts();
        let inr = |t: f64| o.range.map_or(true, |(a, b)| t >= a - 1e-6 && t <= b + 1e-6);
        let mut plans = Vec::new();
        let mut skipped = Vec::new();
        let mut hard_total = 0;
        // the run being built: faded cuts (and the hard ones between them it takes) / hard cuts waiting
        let (mut run, mut pending): (Vec<&Cut>, Vec<&Cut>) = (Vec::new(), Vec::new());
        let flush = |run: &mut Vec<&Cut>, plans: &mut Vec<SpanPlan>| {
            if let Some(p) = plan_of(run) {
                plans.push(p);
            }
            run.clear();
        };
        for c in cuts.iter().filter(|c| inr(c.t)) {
            let Some(x) = &c.tr else {
                hard_total += 1;
                if o.include_hard_cuts && !run.is_empty() {
                    pending.push(c);
                } else {
                    flush(&mut run, &mut plans);
                    pending.clear();
                }
                continue;
            };
            if c.span.is_some() {
                flush(&mut run, &mut plans);
                pending.clear();
                continue;
            }
            if !spannable(x) {
                skipped.push((c.t, "a Fade to Color that is not black (a span dips to black)".to_string()));
                flush(&mut run, &mut plans);
                pending.clear();
                continue;
            }
            let joins = run.first().and_then(|f| f.tr.as_ref()).is_some_and(|f| {
                let d0 = f.duration;
                same_look(f, x) && (x.duration - d0).abs() <= (o.tolerance * d0).max(0.05)
            }) && o.merge_gap.map_or(true, |g| c.t - run.last().map_or(c.t, |l| l.t) <= g + 1e-9);
            if !joins {
                flush(&mut run, &mut plans);
            } else {
                run.append(&mut pending);
            }
            pending.clear();
            run.push(c);
        }
        flush(&mut run, &mut plans);
        (plans, skipped, hard_total)
    }

    /// ws:animator: group the per-cut transitions into transition spans: each run of consecutive cuts (in time,
    /// over every video track) that carry the same transition (kind, ease, direction, shader) of a similar length
    /// becomes one span, its transitions span-made from then on and of one length (the run's median). A hard cut
    /// ends a run and stays hard (unless `include_hard_cuts`); a gap ends it too. Every span is tried on a copy
    /// first and must not move a clip or change a transition outside its run; one that would is split in two
    /// until it doesn't. The sound crossfades of linked clips keep their curves, and none is added. `dry_run`:
    /// report only.
    pub fn spans_from_transitions(&mut self, o: &SpanOpts, dry_run: bool) -> SpansReport {
        if self.editing.is_some() {
            return SpansReport { skipped: vec![(0.0, "close the open sequence first: spans live on the main timeline".into())], ..Default::default() };
        }
        let all_t: Vec<f64> = self.video_cuts().iter().map(|c| c.t).collect();
        let (plans, mut skipped, hard_cuts) = self.span_runs(o);
        let mut work = self.clone();
        let mut done = Vec::new();
        let mut queue: std::collections::VecDeque<SpanPlan> = plans.into();
        while let Some(mut pl) = queue.pop_front() {
            margins(&mut pl, &all_t);
            let before = work.clone();
            pl.span_id = work.push_span(&pl, &before);
            if span_ok(&before, &work, &pl) {
                done.push(pl);
                continue;
            }
            work = before;
            if pl.cuts.len() < 2 {
                skipped.push((pl.t + pl.dur / 2.0, "a span here would move a clip or change another cut: left as it was".into()));
                continue;
            }
            // split in two (a half never starts or ends on a hard cut it would take)
            let half = pl.cuts.len() / 2;
            let mut b = pl.clone();
            b.cuts = pl.cuts.split_off(half);
            for p in [&mut pl, &mut b] {
                while p.cuts.last().is_some_and(|c| c.2) {
                    p.cuts.pop();
                }
                while p.cuts.first().is_some_and(|c| c.2) {
                    p.cuts.remove(0);
                }
                p.hard = p.cuts.iter().filter(|c| c.2).count();
            }
            for p in [b, pl] {
                if !p.cuts.is_empty() {
                    queue.push_front(p);
                }
            }
        }
        let taken: usize = done.iter().map(|p| p.hard).sum();
        if dry_run {
            for p in &mut done {
                p.span_id = 0;
            }
        } else {
            *self = work;
        }
        SpansReport { spans: done, skipped, hard_cuts: hard_cuts.saturating_sub(taken) }
    }

    /// Add `pl` as a span on this project and make its transitions; the sound side keeps what `before` had.
    fn push_span(&mut self, pl: &SpanPlan, before: &Project) -> Id {
        let id = self.new_id();
        self.spans.push(TransitionSpan {
            id,
            t: pl.t,
            dur: pl.dur,
            kind: pl.kind,
            cut_dur: pl.cut_dur,
            ease: pl.ease,
            direction: pl.direction,
            shader: pl.shader.clone(),
            params: pl.params.clone(),
            made: Vec::new(),
        });
        self.refresh_spans();
        // sound: a linked clip's crossfade keeps its shape; one that was not there is not added
        let Some(si) = self.spans.iter().position(|s| s.id == id) else { return id };
        let made = self.spans[si].made.clone();
        let mut drop = Vec::new();
        for (ti, t) in self.tracks.iter_mut().enumerate() {
            if t.kind != TrackKind::Audio {
                continue;
            }
            for x in t.transitions.iter_mut().filter(|x| made.contains(&x.id)) {
                let old = before.tracks.get(ti).and_then(|bt| bt.transitions.iter().find(|o| o.right == x.right && o.edge == x.edge));
                match old {
                    Some(o) => (x.kind, x.curve, x.color, x.ease) = (o.kind, o.curve, o.color, o.ease),
                    None => drop.push(x.id),
                }
            }
        }
        for tid in &drop {
            self.remove_transition(*tid);
        }
        self.spans[si].made.retain(|m| !drop.contains(m));
        id
    }
}

/// The span over a run of cuts: the run's look, the median length, the hard cuts it takes.
fn plan_of(run: &[&Cut]) -> Option<SpanPlan> {
    let first = run.iter().find_map(|c| c.tr.as_ref())?;
    let mut durs: Vec<f64> = run.iter().filter_map(|c| c.tr.as_ref().map(|x| x.duration)).collect();
    durs.sort_by(f64::total_cmp);
    let med = durs[durs.len() / 2];
    let cut_dur = ((med * 100.0).round() / 100.0).max(MIN_CLIP);
    let name = first.name();
    Some(SpanPlan {
        t: run[0].t,
        dur: 0.0,
        kind: first.kind,
        name,
        cut_dur,
        ease: first.ease,
        direction: first.direction,
        shader: first.shader.clone(),
        params: first.params.clone(),
        cuts: run.iter().map(|c| (c.right, c.t, c.tr.is_none())).collect(),
        before: (durs[0], durs[durs.len() - 1]),
        hard: run.iter().filter(|c| c.tr.is_none()).count(),
        span_id: 0,
    })
}

/// Where the span starts and ends: half a transition beyond its first and last cut, but never halfway to a cut
/// outside it.
fn margins(pl: &mut SpanPlan, all_t: &[f64]) {
    let (a, b) = (pl.cuts[0].1, pl.cuts[pl.cuts.len() - 1].1);
    let mine = |t: &f64| pl.cuts.iter().any(|c| (c.1 - t).abs() < 1e-9);
    let prev = all_t.iter().filter(|t| **t < a - 1e-9 && !mine(t)).fold(f64::NEG_INFINITY, |m, t| m.max(*t));
    let next = all_t.iter().filter(|t| **t > b + 1e-9 && !mine(t)).fold(f64::INFINITY, |m, t| m.min(*t));
    let half = (pl.cut_dur / 2.0).max(0.01);
    let ma = half.min((a - prev) / 2.0 - 1e-3).max(0.002);
    let mb = half.min((next - b) / 2.0 - 1e-3).max(0.002);
    pl.t = (a - ma).max(0.0);
    pl.dur = (b + mb - pl.t).max(MIN_SPAN);
}

/// After a span was added: every clip where it was, every video transition outside the run as it was, and every
/// cut of the run carrying one.
fn span_ok(before: &Project, after: &Project, pl: &SpanPlan) -> bool {
    if before.tracks.len() != after.tracks.len() {
        return false;
    }
    for (a, b) in before.tracks.iter().zip(&after.tracks) {
        if a.clips.len() != b.clips.len()
            || a.clips.iter().zip(&b.clips).any(|(x, y)| x.id != y.id || (x.start - y.start).abs() > 1e-9 || (x.duration - y.duration).abs() > 1e-9)
        {
            return false;
        }
        if a.kind != TrackKind::Video {
            continue;
        }
        let key = |x: &Transition| (x.right, x.edge as u8);
        let mine = |x: &Transition| x.edge == TransitionEdge::Cut && pl.cuts.iter().any(|c| c.0 == x.right);
        let strip = |x: &Transition| Transition { id: 0, ..x.clone() };
        let mut ka: Vec<Transition> = a.transitions.iter().filter(|x| !mine(x)).map(strip).collect();
        let mut kb: Vec<Transition> = b.transitions.iter().filter(|x| !mine(x)).map(strip).collect();
        ka.sort_by_key(key);
        kb.sort_by_key(key);
        if ka != kb {
            return false;
        }
    }
    let cuts = after.video_cuts();
    pl.cuts.iter().all(|(id, _, _)| cuts.iter().any(|c| c.right == *id && c.span.is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proj() -> Project {
        let mut p = Project::new();
        let v1 = p.video_tracks()[0];
        p.tracks[v1].clips.push(Clip::new(10, ClipKind::Video, "a", 0.0, 6.0));
        p
    }
    fn key(a: &mut Animated, t: f64, v: f64, ease: Ease) {
        crate::model::keyblocks::upsert(a, Keyframe { t, v, ease });
    }

    /// A clip with a hand fade in, a hand slide, a hand Ken Burns-like zoom, an odd wobble and a fade out.
    fn keyed() -> Project {
        let mut p = proj();
        let c = p.clip_mut(10).unwrap();
        key(&mut c.opacity, 0.0, 0.0, Ease::Linear);
        key(&mut c.opacity, 0.35, 1.0, Ease::Linear);
        key(&mut c.opacity, 5.6, 1.0, Ease::EaseIn);
        key(&mut c.opacity, 6.0, 0.0, Ease::Linear);
        key(&mut c.scale, 0.0, 1.0, Ease::Linear);
        key(&mut c.scale, 6.0, 1.15, Ease::Linear);
        key(&mut c.rotation, 2.0, 0.0, Ease::Linear);
        key(&mut c.rotation, 2.2, 7.0, Ease::Bezier { x1: 0.2, y1: 0.9, x2: 0.3, y2: 1.0 });
        key(&mut c.rotation, 2.5, -3.0, Ease::Linear);
        key(&mut c.rotation, 2.9, 0.0, Ease::Linear);
        p
    }

    #[test]
    fn conversion_keeps_every_sampled_value_and_names_what_it_recognises() {
        let mut p = keyed();
        let before = p.clip(10).unwrap().clone();
        let r = p.convert_keys_to_blocks(&[10], false);
        let after = p.clip(10).unwrap();
        assert!(same_motion(&before, after, &PICTURE_PROPS), "motion changed");
        // sample by hand too, at 240 points per property
        for prop in PICTURE_PROPS {
            for i in 0..=240 {
                let t = 6.0 * i as f64 / 240.0;
                let (a, b) = (prop_ref(&before, prop).unwrap().at(t), prop_ref(after, prop).unwrap().at(t));
                assert!((a - b).abs() < 1e-6, "{prop} at {t}: {a} vs {b}");
            }
        }
        let names: Vec<&str> = r.made.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"Fade In") && names.contains(&"Fade Out"), "{names:?}");
        assert!(names.contains(&"Ken Burns"), "{names:?}");
        assert!(names.contains(&"Custom"), "the wobble is no builtin: {names:?}");
        assert!(r.skipped.is_empty(), "{:?}", r.skipped);
        // every key is now a block's, so a second pass finds nothing to do
        assert!(after.hand_runs().is_empty());
        let i = after.blocks.iter().position(|b| b.name == "Ken Burns").unwrap();
        let mut ctl = after.blocks[i].ctl.clone().unwrap();
        assert!(p.convert_keys_to_blocks(&[10], false).made.is_empty());
        // and the blocks keep their controls: a stronger Ken Burns zooms further
        ctl.strength *= 2.0;
        assert!(p.set_block_ctl(10, i, ctl));
        assert!(p.clip(10).unwrap().scale.at(6.0) > 1.25);
    }

    #[test]
    fn a_dry_run_changes_nothing_and_reports_the_same() {
        let mut p = keyed();
        let json = p.to_json();
        let dry = p.convert_keys_to_blocks(&[10], true);
        assert_eq!(p.to_json(), json);
        assert!(!dry.made.is_empty());
        let real = p.convert_keys_to_blocks(&[10], false);
        assert_eq!(dry.made, real.made);
        assert_ne!(p.to_json(), json);
    }

    #[test]
    fn keys_a_block_owns_are_left_alone() {
        let mut p = proj();
        let fade = builtin_blocks().into_iter().find(|b| b.name == "Fade In").unwrap();
        p.clip_mut(10).unwrap().place_block(&fade, 0.0, Some(0.5), 1920.0, 1080.0);
        key(&mut p.clip_mut(10).unwrap().x, 2.0, 0.0, Ease::Linear);
        key(&mut p.clip_mut(10).unwrap().x, 3.0, 400.0, Ease::Linear);
        let block_keys = p.clip(10).unwrap().opacity.keys.clone();
        let r = p.convert_keys_to_blocks(&[10], false);
        assert_eq!(r.made.len(), 1);
        assert_eq!(r.made[0].props, ["x"]);
        assert_eq!(p.clip(10).unwrap().opacity.keys, block_keys);
        assert_eq!(p.clip(10).unwrap().blocks.len(), 2);
    }

    #[test]
    fn linked_properties_and_keys_past_the_end_are_not_touched() {
        let mut p = proj();
        let c = p.clip_mut(10).unwrap();
        key(&mut c.y, 5.0, 0.0, Ease::Linear);
        key(&mut c.y, 8.0, 100.0, Ease::Linear); // past the clip's 6 s
        let r = p.convert_keys_to_blocks(&[10], false);
        assert!(r.made.is_empty() && r.skipped.len() == 1);
    }

    /// V1: 0..2 | 2..4 | 4..6 | 6..8 | 8..10 | 10..12 | gap | 14..16 | 16..18. Cross fades on 2, 4, 6, a hard cut
    /// on 8, cross fades on 10 and 16 (16 after a gap) and a Wipe on 14..
    fn cuts() -> Project {
        let mut p = Project::new();
        let v1 = p.video_tracks()[0];
        for (i, s) in [0.0, 2.0, 4.0, 6.0, 8.0, 10.0, 14.0, 16.0].iter().enumerate() {
            let d = if *s == 10.0 { 2.0 } else { 2.0 };
            p.tracks[v1].clips.push(Clip::new(100 + i as Id, ClipKind::Video, "c", *s, d));
        }
        for (right, d) in [(101, 0.35), (102, 0.35), (103, 0.4)] {
            p.add_transition(right, TransitionKind::CrossFade, d);
        }
        p.add_transition(105, TransitionKind::CrossFade, 0.35); // after the hard cut at 8
        p.add_transition(107, TransitionKind::Wipe, 0.35); // 16: after the gap
        p
    }

    #[test]
    fn spans_group_runs_of_the_same_transition_and_leave_hard_cuts_and_gaps_alone() {
        let mut p = cuts();
        let starts: Vec<f64> = p.tracks.iter().flat_map(|t| t.clips.iter().map(|c| c.start)).collect();
        let dry = p.spans_from_transitions(&SpanOpts::default(), true);
        assert!(p.spans.is_empty());
        let r = p.spans_from_transitions(&SpanOpts::default(), false);
        assert_eq!(dry.spans.len(), r.spans.len());
        // [2, 4, 6] one span (0.35, 0.35, 0.4 -> one length), [10] and [16 wipe] each their own
        let groups: Vec<Vec<Id>> = r.spans.iter().map(|s| s.cuts.iter().map(|c| c.0).collect()).collect();
        assert_eq!(groups, vec![vec![101, 102, 103], vec![105], vec![107]]);
        assert_eq!(r.spans[0].cut_dur, 0.35);
        assert_eq!(r.spans[2].kind, TransitionKind::Wipe);
        assert_eq!(r.hard_cuts, 1);
        // nothing moved (the gap before 14 is still there), the hard cut at 8 has no transition
        let after: Vec<f64> = p.tracks.iter().flat_map(|t| t.clips.iter().map(|c| c.start)).collect();
        assert_eq!(starts, after);
        let cs = p.video_cuts();
        assert!(cs.iter().find(|c| c.right == 104).unwrap().tr.is_none());
        assert!(cs.iter().filter(|c| c.tr.is_some()).all(|c| c.span.is_some()), "every transition is a span's now");
        assert!(cs.iter().filter(|c| c.tr.is_some()).all(|c| (c.tr.as_ref().unwrap().duration - 0.35).abs() < 1e-9));
        // the spans stay put through the next refresh (as after any edit)
        let made: Vec<Vec<Id>> = p.spans.iter().map(|s| s.made.clone()).collect();
        p.refresh_spans();
        assert_eq!(made, p.spans.iter().map(|s| s.made.clone()).collect::<Vec<_>>());
        // a second run has nothing left to group
        assert!(p.spans_from_transitions(&SpanOpts::default(), false).spans.is_empty());
    }

    #[test]
    fn include_hard_cuts_closes_a_run_over_them() {
        let mut p = cuts();
        let r = p.spans_from_transitions(&SpanOpts { include_hard_cuts: true, ..Default::default() }, false);
        let groups: Vec<Vec<Id>> = r.spans.iter().map(|s| s.cuts.iter().map(|c| c.0).collect()).collect();
        assert_eq!(groups[0], vec![101, 102, 103, 104, 105]);
        assert_eq!(r.spans[0].hard, 1);
        assert!(p.video_cuts().iter().find(|c| c.right == 104).unwrap().tr.is_some());
    }

    #[test]
    fn a_span_that_would_touch_an_overlay_cut_is_split_around_it() {
        let mut p = cuts();
        // two abutting titles on V2 whose cut (at 3) sits inside the [2, 6] run: a span over it would give it a
        // transition too, so the run is split around it
        let v2 = p.add_track(TrackKind::Video);
        p.tracks[v2].clips.push(Clip::new(200, ClipKind::Text, "t", 2.5, 0.5));
        p.tracks[v2].clips.push(Clip::new(201, ClipKind::Text, "t", 3.0, 0.5));
        let r = p.spans_from_transitions(&SpanOpts::default(), false);
        assert!(p.video_cuts().iter().find(|c| c.right == 201).unwrap().tr.is_none(), "the overlay cut stays hard");
        let all: Vec<Id> = r.spans.iter().flat_map(|s| s.cuts.iter().map(|c| c.0)).collect();
        for id in [101, 102, 103, 105, 107] {
            assert!(all.contains(&id), "{id} still grouped: {all:?}");
        }
    }

    #[test]
    fn the_audit_finds_hand_runs_and_owned_keys() {
        let mut p = keyed();
        let a = p.clip(10).unwrap().key_audit();
        assert_eq!(a.iter().map(|x| x.prop).collect::<Vec<_>>(), ["scale", "rotation", "opacity"]);
        assert!(a.iter().all(|x| x.hand == x.keys));
        // the fade in, the Ken Burns over everything: one long run holding all three
        assert_eq!(p.clip(10).unwrap().hand_runs().len(), 1);
        p.convert_keys_to_blocks(&[10], false);
        let a = p.clip(10).unwrap().key_audit();
        assert!(a.iter().all(|x| x.hand == 0), "{a:?}");
    }
}
