//! The Animator's blocks: plain controls (strength, side, ease, physics) on top of keyframe blocks, simulated
//! blocks (shake, spring, bounce, throw, wobble), and Flow, which matches speeds where one move hands over to the
//! next (on one clip, or across a cut). Everything still writes ordinary keys: the renderer never sees a block.

use crate::model::*;
use serde::{Deserialize, Serialize};

/// The side a move comes from / goes to.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Dir {
    Left,
    Up,
    Down,
    Right,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::Left, Dir::Up, Dir::Down, Dir::Right];
    pub fn name(self) -> &'static str {
        match self {
            Dir::Left => "left",
            Dir::Up => "up",
            Dir::Down => "down",
            Dir::Right => "right",
        }
    }
    pub fn parse(s: &str) -> Option<Dir> {
        Dir::ALL.into_iter().find(|d| d.name().eq_ignore_ascii_case(s))
    }
    fn vec(self) -> (i8, i8) {
        match self {
            Dir::Left => (-1, 0),
            Dir::Up => (0, -1),
            Dir::Down => (0, 1),
            Dir::Right => (1, 0),
        }
    }
}

/// A simulated block: its keys come from a small physics / noise simulation, not a fixed shape.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Gen {
    Shake,
    SpringIn,
    BounceDrop,
    ThrowOut,
    Wobble,
}

impl Gen {
    pub const ALL: [Gen; 5] = [Gen::Shake, Gen::SpringIn, Gen::BounceDrop, Gen::ThrowOut, Gen::Wobble];
    pub fn name(self) -> &'static str {
        match self {
            Gen::Shake => "Camera Shake",
            Gen::SpringIn => "Spring In",
            Gen::BounceDrop => "Bounce Drop",
            Gen::ThrowOut => "Throw Out",
            Gen::Wobble => "Wobble",
        }
    }
    fn dur(self) -> f64 {
        match self {
            Gen::Shake => 1.0,
            Gen::SpringIn => 1.0,
            Gen::BounceDrop => 1.2,
            Gen::ThrowOut => 0.9,
            Gen::Wobble => 1.0,
        }
    }
    /// Which physics controls apply (the Inspector shows only these).
    pub fn controls(self) -> &'static [&'static str] {
        match self {
            Gen::Shake => &["freq", "damping", "spin", "cover"],
            Gen::SpringIn => &["stiffness", "damping"],
            Gen::BounceDrop => &["bounce", "gravity"],
            Gen::ThrowOut => &["gravity", "spin"],
            Gen::Wobble => &["stiffness", "damping"],
        }
    }
    pub fn has_dir(self) -> bool {
        matches!(self, Gen::SpringIn | Gen::ThrowOut)
    }
}

/// Physics / noise controls (0..1 unless noted).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Phys {
    pub stiffness: f64,
    pub damping: f64,
    pub bounce: f64,
    pub gravity: f64,
    /// -1..1: rotation (shake: tilt amount, throw: spin speed).
    pub spin: f64,
    /// Shake: jolts per second.
    pub freq: f64,
    pub seed: u32,
    /// Shake: zoom in just enough that the frame edges never show (camera shake on footage).
    pub cover: bool,
}

impl Default for Phys {
    fn default() -> Self {
        Self { stiffness: 0.55, damping: 0.3, bounce: 0.45, gravity: 0.55, spin: 0.3, freq: 12.0, seed: 1, cover: true }
    }
}

fn one() -> f64 {
    1.0
}
fn is_false(b: &bool) -> bool {
    !*b
}

/// The controls of an applied block. Changing one rewrites the block's keys from `src`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BlockCtl {
    /// The block as shipped or saved, before these controls (a simulated one has `gen` set and no tracks).
    pub src: KeyBlock,
    /// 0..2, 1 = as designed: scales how far it moves / grows / turns.
    #[serde(default = "one")]
    pub strength: f64,
    /// The side it moves from / to (blocks that move along x or y).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<Dir>,
    /// Ease of the move (every segment but a Hold): how it travels from its origin to its target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ease: Option<Ease>,
    /// Origin / target: (property, value) the block starts from / ends at, instead of the preset's own
    /// (relative) first / last key. Empty = as designed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub from: Vec<(String, f64)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<(String, f64)>,
    #[serde(default)]
    pub phys: Phys,
    /// Speed at the start / end set by Flow: (property, normalized slope of that segment).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_speed: Option<(String, f64)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_speed: Option<(String, f64)>,
    /// Its keys were edited by hand: the controls are paused until Reset.
    #[serde(default, skip_serializing_if = "is_false")]
    pub edited: bool,
}

/// The x/y side a keyed block moves along: the biggest x / y offset among its keys.
pub fn native_dir(b: &KeyBlock) -> Option<Dir> {
    let mut best: Option<(f64, Dir)> = None;
    for t in &b.tracks {
        for k in &t.keys {
            let BlockVal::Add(v) = k.v else { continue };
            let d = match (t.prop.as_str(), v > 0.0) {
                ("x", false) => Dir::Left,
                ("x", true) => Dir::Right,
                ("y", false) => Dir::Up,
                ("y", true) => Dir::Down,
                _ => continue,
            };
            if v.abs() > 1e-9 && best.is_none_or(|(m, _)| v.abs() > m) {
                best = Some((v.abs(), d));
            }
        }
    }
    best.map(|b| b.1)
}

/// Slope `s` (eased progress per unit progress) at the start of a segment, keeping the handle inside the box.
pub fn ease_with_in_slope(e: Ease, s: f64) -> Ease {
    let (x1, _, x2, y2) = e.handles();
    let mut x = (x1 as f64).max(0.25);
    if (s * x).abs() > 1.0 {
        x = 1.0 / s.abs();
    }
    Ease::Bezier { x1: x as f32, y1: (s * x) as f32, x2, y2 }
}
/// Same at the end of a segment.
pub fn ease_with_out_slope(e: Ease, s: f64) -> Ease {
    let (x1, y1, x2, _) = e.handles();
    let mut w = (1.0 - x2 as f64).max(0.25);
    if (s * w).abs() > 1.0 {
        w = 1.0 / s.abs();
    }
    Ease::Bezier { x1, y1, x2: (1.0 - w) as f32, y2: (1.0 - s * w) as f32 }
}

impl BlockCtl {
    pub fn new(src: KeyBlock) -> Self {
        let dir = match src.gen {
            Some(g) if g.has_dir() => Some(Dir::Left),
            Some(_) => None,
            None => native_dir(&src),
        };
        Self {
            src,
            strength: 1.0,
            dir,
            ease: None,
            from: Vec::new(),
            to: Vec::new(),
            phys: Phys::default(),
            in_speed: None,
            out_speed: None,
            edited: false,
        }
    }
    pub fn is_sim(&self) -> bool {
        self.src.gen.is_some()
    }

    /// The block these controls make, `dur` seconds long (a simulation runs in real seconds).
    pub fn resolve(&self, dur: f64) -> KeyBlock {
        let mut b = match self.src.gen {
            Some(g) => simulate(g, &self.phys, self.strength, self.dir.unwrap_or(Dir::Left), dur),
            None => self.reshape(),
        };
        b.name = self.src.name.clone();
        for t in &mut b.tracks {
            let n = t.keys.len();
            if n < 2 {
                continue;
            }
            if let Some(e) = self.ease.filter(|_| self.src.gen.is_none()) {
                // every segment (the last key's ease belongs to what follows the block)
                for k in t.keys[..n - 1].iter_mut().filter(|k| k.ease != Ease::Hold) {
                    k.ease = e;
                }
            }
            if let Some((_, v)) = self.from.iter().find(|(p, _)| *p == t.prop) {
                t.keys[0].v = BlockVal::Abs(*v);
            }
            if let Some((_, v)) = self.to.iter().find(|(p, _)| *p == t.prop) {
                t.keys[n - 1].v = BlockVal::Abs(*v);
            }
            if let Some((p, s)) = &self.in_speed {
                if *p == t.prop {
                    t.keys[0].ease = ease_with_in_slope(t.keys[0].ease, *s);
                }
            }
            if let Some((p, s)) = &self.out_speed {
                if *p == t.prop {
                    t.keys[n - 2].ease = ease_with_out_slope(t.keys[n - 2].ease, *s);
                }
            }
        }
        b
    }

    /// A keyed block with strength and side applied.
    fn reshape(&self) -> KeyBlock {
        let mut b = self.src.clone();
        let k = self.strength.clamp(0.0, 3.0);
        // at 1 the preset comes back exactly (no 1 + (v - 1) * 1 rounding)
        for t in b.tracks.iter_mut().filter(|_| (k - 1.0).abs() > 1e-12) {
            for key in &mut t.keys {
                key.v = match key.v {
                    BlockVal::Add(v) => BlockVal::Add(v * k),
                    BlockVal::Mul(v) => BlockVal::Mul((1.0 + (v - 1.0) * k).max(if v >= 0.0 { 0.0 } else { f64::MIN })),
                    a => a,
                };
            }
        }
        if let (Some(from), Some(to)) = (native_dir(&self.src), self.dir) {
            turn(&mut b, from, to);
        }
        b
    }
}

/// Re-point a block's x/y motion from side `from` to side `to` (a quarter turn swaps x/y and their stretches).
fn turn(b: &mut KeyBlock, from: Dir, to: Dir) {
    let (f, t) = (from.vec(), to.vec());
    if f == t {
        return;
    }
    let neg = |tr: &mut BlockTrack| {
        for k in &mut tr.keys {
            if let BlockVal::Add(v) = k.v {
                k.v = BlockVal::Add(-v);
            }
        }
    };
    if f.0 == -t.0 && f.1 == -t.1 {
        b.tracks.iter_mut().filter(|tr| tr.prop == "x" || tr.prop == "y").for_each(neg);
        return;
    }
    // quarter turn: (x, y) -> (-y, x) when that maps `from` onto `to`, else (y, -x)
    let ccw = (-f.1, f.0) == t;
    for tr in &mut b.tracks {
        let swap = match tr.prop.as_str() {
            "x" => Some("y"),
            "y" => Some("x"),
            "scale_x" => Some("scale_y"),
            "scale_y" => Some("scale_x"),
            _ => None,
        };
        let Some(np) = swap else { continue };
        // new x = -old y (ccw) or old y; new y = old x (ccw) or -old x
        let flip = (ccw && tr.prop == "y") || (!ccw && tr.prop == "x");
        tr.prop = np.into();
        if flip {
            neg(tr);
        }
    }
}

// ---------- simulation ----------

const N: usize = 120;

fn spring(dur: f64, w: f64, z: f64, x0: f64, v0: f64) -> Vec<f64> {
    let (mut x, mut v, dt) = (x0, v0, dur / N as f64 / 12.0);
    let mut out = vec![x];
    for _ in 0..N {
        for _ in 0..12 {
            v += (-w * w * x - 2.0 * z * w * v) * dt;
            x += v * dt;
        }
        out.push(x);
    }
    out
}
/// Settle the last `frac` of the samples onto `to` so the block always ends at rest.
fn settle(a: &mut [f64], to: f64, frac: f64) {
    let s = ((a.len() as f64) * (1.0 - frac)) as usize;
    let n = a.len() - 1;
    for i in s..=n {
        let q = (i - s) as f64 / (n - s).max(1) as f64;
        a[i] += (to - a[i]) * q * q * (3.0 - 2.0 * q);
    }
}
/// Deterministic noise in -1..1.
fn noise(seed: u32, i: u32) -> f64 {
    let mut h = seed.wrapping_mul(0x9E37_79B9) ^ i.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h as f64 / u32::MAX as f64) * 2.0 - 1.0
}

/// Keys through sampled values (one sample per 1/N of the block): the ends, every turn and corner, at most
/// N/10 apart, each segment a bezier whose end slopes follow the samples.
pub fn fit(prop: &str, a: &[f64], val: fn(f64) -> BlockVal) -> BlockTrack {
    let n = a.len() - 1;
    let mut idx = vec![0usize];
    for i in 1..n {
        let (l, r) = (a[i] - a[i - 1], a[i + 1] - a[i]);
        let turn = l * r < 0.0 || ((l.abs() > 1e-9) != (r.abs() > 1e-9));
        if turn || i - idx[idx.len() - 1] >= n / 10 {
            idx.push(i);
        }
    }
    idx.push(n);
    let nf = n as f64;
    let keys = idx
        .windows(2)
        .map(|w| {
            let (i, j) = (w[0], w[1]);
            let (dv, df) = (a[j] - a[i], (j - i) as f64 / nf);
            let ease = if dv.abs() < 1e-9 {
                Ease::Linear
            } else {
                let m0 = (a[i + 1] - a[i]) * nf * df / dv;
                let m1 = (a[j] - a[j - 1]) * nf * df / dv;
                let c = |y: f64| y.clamp(-3.0, 4.0) as f32;
                Ease::Bezier { x1: 1.0 / 3.0, y1: c(m0 / 3.0), x2: 2.0 / 3.0, y2: c(1.0 - m1 / 3.0) }
            };
            BlockKey { f: i as f64 / nf, v: val(a[i]), ease }
        })
        .chain(std::iter::once(BlockKey { f: 1.0, v: val(a[n]), ease: Ease::Linear }))
        .collect();
    BlockTrack { prop: prop.into(), keys }
}

/// A simulated block. Offsets are in frame widths (x) / heights (y), like the keyed blocks.
pub fn simulate(g: Gen, p: &Phys, strength: f64, dir: Dir, dur: f64) -> KeyBlock {
    let add: fn(f64) -> BlockVal = BlockVal::Add;
    let mul: fn(f64) -> BlockVal = BlockVal::Mul;
    let abs: fn(f64) -> BlockVal = BlockVal::Abs;
    let (dx, dy) = dir.vec();
    let (dx, dy) = (dx as f64, dy as f64);
    let s = strength.clamp(0.0, 3.0);
    let mut tracks = Vec::new();
    let axes = |tracks: &mut Vec<BlockTrack>, a: &[f64]| {
        if dx != 0.0 {
            tracks.push(fit("x", &a.iter().map(|v| v * dx).collect::<Vec<_>>(), add));
        }
        if dy != 0.0 {
            tracks.push(fit("y", &a.iter().map(|v| v * dy).collect::<Vec<_>>(), add));
        }
    };
    match g {
        Gen::Shake => {
            // jolts at `freq` per second, fading by `damping`; tilt by `spin`
            let amp = 0.03 * s;
            let jolts = ((p.freq.clamp(1.0, 40.0) * dur).round() as usize).max(2);
            let env = |f: f64| (1.0 - p.damping.clamp(0.0, 1.0) * f) * (f * 8.0).min(1.0);
            let mk = |axis: u32, scale: f64| -> BlockTrack {
                let keys = (0..=jolts)
                    .map(|i| {
                        let f = i as f64 / jolts as f64;
                        let v = if i == 0 || i == jolts {
                            0.0
                        } else {
                            noise(p.seed.wrapping_add(axis), i as u32) * scale * env(f)
                        };
                        BlockKey { f, v: BlockVal::Add(v), ease: Ease::EaseInOut }
                    })
                    .collect();
                BlockTrack { prop: ["x", "y", "rotation"][axis as usize].into(), keys }
            };
            tracks.push(mk(0, amp));
            tracks.push(mk(1, amp * 0.8));
            if p.spin.abs() > 1e-3 {
                tracks.push(mk(2, p.spin.abs() * 3.0 * s));
            }
            if p.cover {
                let z = 1.0 + amp * 2.4 + p.spin.abs() * 0.02 * s;
                let k = |f: f64, v: f64, e: Ease| BlockKey { f, v: BlockVal::Mul(v), ease: e };
                tracks.push(BlockTrack {
                    prop: "scale".into(),
                    keys: vec![
                        k(0.0, 1.0, Ease::EaseOut),
                        k(0.1, z, Ease::Linear),
                        k(0.9, z, Ease::EaseIn),
                        k(1.0, 1.0, Ease::Linear),
                    ],
                });
            }
        }
        Gen::SpringIn => {
            let mut a = spring(dur, 5.0 + p.stiffness * 22.0, 0.06 + p.damping * 0.9, s, 0.0);
            settle(&mut a, 0.0, 0.08);
            axes(&mut tracks, &a);
            let o: Vec<f64> = (0..=N).map(|i| (i as f64 / N as f64 / 0.12).min(1.0)).collect();
            tracks.push(fit("opacity", &o, abs));
        }
        Gen::BounceDrop => {
            let (mut y, mut v) = (-0.9 * s, 0.0);
            let (g, e, dt) = (2.5 + p.gravity * 3.5, 0.15 + p.bounce * 0.7, dur / N as f64 / 12.0);
            let mut a = vec![y];
            for _ in 0..N {
                for _ in 0..12 {
                    v += g * dt;
                    y += v * dt;
                    if y > 0.0 {
                        y = 0.0;
                        v = -v * e;
                        if v.abs() < 0.06 {
                            v = 0.0;
                        }
                    }
                }
                a.push(y);
            }
            settle(&mut a, 0.0, 0.1);
            tracks.push(fit("y", &a, add));
        }
        Gen::ThrowOut => {
            let sp = 0.6 + s * 1.2;
            let g = 1.0 + p.gravity * 4.0;
            let vy = dy * sp - if dy == 0.0 { sp * 0.6 } else { 0.0 };
            let t = |i: usize| i as f64 / N as f64 * dur;
            let x: Vec<f64> = (0..=N).map(|i| dx * sp * t(i)).collect();
            let y: Vec<f64> = (0..=N).map(|i| vy * t(i) + 0.5 * g * t(i) * t(i)).collect();
            if dx != 0.0 {
                tracks.push(fit("x", &x, add));
            }
            tracks.push(fit("y", &y, add));
            if p.spin.abs() > 1e-3 {
                tracks.push(fit("rotation", &(0..=N).map(|i| p.spin * 700.0 * t(i)).collect::<Vec<_>>(), add));
            }
            let o: Vec<f64> = (0..=N).map(|i| (1.0 - (i as f64 / N as f64 - 0.6).max(0.0) / 0.4).max(0.0)).collect();
            tracks.push(fit("opacity", &o, mul));
        }
        Gen::Wobble => {
            let mut r = spring(dur, 6.0 + p.stiffness * 25.0, 0.03 + p.damping * 0.7, 0.0, s * 700.0);
            settle(&mut r, 0.0, 0.1);
            tracks.push(fit("rotation", &r, add));
        }
    }
    KeyBlock { name: g.name().into(), dur, tracks, gen: None }
}

/// The simulated blocks as presets (Gallery and Animator).
pub fn sim_blocks() -> Vec<KeyBlock> {
    Gen::ALL
        .iter()
        .map(|&g| KeyBlock { name: g.name().into(), dur: g.dur(), tracks: Vec::new(), gen: Some(g) })
        .collect()
}

// ---------- flow: speeds where one move hands over to the next ----------

/// Properties whose speed Flow compares (opacity / volume changes are not motion).
pub const MOTION_PROPS: [&str; 6] = ["x", "y", "scale", "scale_x", "scale_y", "rotation"];

/// How fast `prop` of `c` moves at clip-local `t`, from the left (`side` < 0) or the right. Scale is a log
/// rate in %/s (a zoom feels the same speed at any size); the rest is units per second.
pub fn speed(c: &Clip, prop: &str, t: f64, side: f64) -> f64 {
    let Some(a) = prop_ref(c, prop) else { return 0.0 };
    let e = 1.0 / 600.0;
    let (t0, t1) = if side < 0.0 { (t - 2.0 * e, t - e) } else { (t + e, t + 2.0 * e) };
    let v = |t: f64| {
        let v = a.at(t);
        if prop.starts_with("scale") {
            v.max(1e-4).ln() * 100.0
        } else {
            v
        }
    };
    (v(t1) - v(t0)) / e
}

/// The exact speed at a key (`start` = of the segment it starts, else of the one it ends), read off the
/// segment's ease like `set_block_speed` writes it, in `speed`'s units; `speed` (sampled) when no key is there.
pub fn edge_speed(c: &Clip, prop: &str, t: f64, start: bool) -> f64 {
    let Some(a) = prop_ref(c, prop) else { return 0.0 };
    let seg = a.key_index_at(t).and_then(|k| if start { Some(k) } else { k.checked_sub(1) });
    let (Some(ka), Some(kb)) = (seg.and_then(|s| a.keys.get(s)), seg.and_then(|s| a.keys.get(s + 1))) else {
        return speed(c, prop, t, if start { 1.0 } else { -1.0 });
    };
    let slope = match ka.ease {
        Ease::Linear => 1.0,
        Ease::Hold => 0.0,
        Ease::EaseIn => {
            if start {
                0.0
            } else {
                2.0
            }
        }
        Ease::EaseOut => {
            if start {
                2.0
            } else {
                0.0
            }
        }
        Ease::EaseInOut => 0.0,
        Ease::Bezier { x1, y1, x2, y2 } => {
            if start {
                if x1 > 1e-4 {
                    y1 as f64 / x1 as f64
                } else {
                    0.0
                }
            } else if x2 < 1.0 - 1e-4 {
                (1.0 - y2 as f64) / (1.0 - x2 as f64)
            } else {
                0.0
            }
        }
    };
    let v = slope * (kb.v - ka.v) / (kb.t - ka.t).max(1e-9);
    if prop.starts_with("scale") {
        v / (if start { ka.v } else { kb.v }).max(1e-4) * 100.0
    } else {
        v
    }
}

/// One move ending where the next starts: on one clip, or a block ending at a clip's end and one starting at
/// the next clip's start on the same track (a cut).
#[derive(Clone, Debug, PartialEq)]
pub struct Seam {
    pub left: (Id, usize),
    pub right: (Id, usize),
    /// Timeline seconds.
    pub t: f64,
    pub prop: String,
    pub v_left: f64,
    pub v_right: f64,
    pub smooth: bool,
}

/// Which side Flow eases to make two speeds meet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Match {
    /// The later move starts at the speed the earlier one ends at.
    #[default]
    Next,
    /// The earlier move ends at the speed the later one starts at.
    Earlier,
    /// Both meet halfway.
    Both,
}

impl Match {
    pub fn parse(s: &str) -> Option<Match> {
        match s.to_ascii_lowercase().as_str() {
            "next" | "right" => Some(Match::Next),
            "earlier" | "left" | "previous" => Some(Match::Earlier),
            "both" | "meet" => Some(Match::Both),
            _ => None,
        }
    }
}

fn weight(p: &str) -> f64 {
    match p {
        "x" | "y" => 0.25,
        "rotation" => 0.5,
        _ => 1.0,
    }
}
fn threshold(p: &str) -> f64 {
    match p {
        "x" | "y" => 15.0,
        "rotation" => 10.0,
        _ => 3.0,
    }
}

impl Project {
    /// Every hand-off between two moves, with both speeds of the property that moves most there.
    pub fn block_seams(&self) -> Vec<Seam> {
        let mut out = Vec::new();
        let eps = 1.5 / self.fps.max(1.0);
        let mut push = |lc: &Clip, li: usize, rc: &Clip, ri: usize, t: f64| {
            let (lb, rb) = (&lc.blocks[li], &rc.blocks[ri]);
            let props: Vec<&str> = MOTION_PROPS
                .into_iter()
                .filter(|p| lb.props.iter().any(|q| q == p) && rb.props.iter().any(|q| q == p))
                .collect();
            let best = props
                .into_iter()
                .map(|p| (p, edge_speed(lc, p, lb.end(), false), edge_speed(rc, p, rb.t, true)))
                .max_by(|a, b| {
                    ((a.1.abs() + a.2.abs()) * weight(a.0)).total_cmp(&((b.1.abs() + b.2.abs()) * weight(b.0)))
                });
            if let Some((p, vl, vr)) = best {
                let smooth = (vl - vr).abs() <= 0.1 * vl.abs().max(vr.abs()) + threshold(p);
                out.push(Seam {
                    left: (lc.id, li),
                    right: (rc.id, ri),
                    t,
                    prop: p.into(),
                    v_left: vl,
                    v_right: vr,
                    smooth,
                });
            }
        };
        for tr in &self.tracks {
            for c in &tr.clips {
                for (i, a) in c.blocks.iter().enumerate() {
                    for (j, b) in c.blocks.iter().enumerate() {
                        if i != j && (a.end() - b.t).abs() < BK_EPS.max(1e-3) {
                            push(c, i, c, j, c.start + b.t);
                        }
                    }
                }
            }
            // cuts: the last block of a clip ends at its end, the first block of the next clip starts at 0
            for a in &tr.clips {
                let Some(next) =
                    tr.clips.iter().find(|b| b.id != a.id && (b.start - (a.start + a.duration)).abs() < eps)
                else {
                    continue;
                };
                let li = a.blocks.iter().position(|b| (b.end() - a.duration).abs() < eps);
                let ri = next.blocks.iter().position(|b| b.t < eps);
                if let (Some(li), Some(ri)) = (li, ri) {
                    push(a, li, next, ri, next.start);
                }
            }
        }
        out
    }

    /// Ease one or both moves of a seam so their speeds meet. Returns the speed they meet at.
    pub fn match_seam(&mut self, left: (Id, usize), right: (Id, usize), mode: Match) -> Result<f64, String> {
        let seam = self
            .block_seams()
            .into_iter()
            .find(|s| s.left == left && s.right == right)
            .ok_or("those two blocks do not hand over to each other")?;
        let snapshot = (self.clip(left.0).cloned(), self.clip(right.0).cloned());
        let p = seam.prop.clone();
        let res = match mode {
            Match::Next => self
                .set_block_speed(right, &p, true, seam.v_left)
                .or_else(|_| self.set_block_speed(left, &p, false, seam.v_right).map(|_| seam.v_right)),
            Match::Earlier => self
                .set_block_speed(left, &p, false, seam.v_right)
                .or_else(|_| self.set_block_speed(right, &p, true, seam.v_left).map(|_| seam.v_left)),
            Match::Both => {
                let m = (seam.v_left + seam.v_right) / 2.0;
                self.set_block_speed(left, &p, false, m).and_then(|_| self.set_block_speed(right, &p, true, m))
            }
        };
        if res.is_err() {
            for c in [snapshot.0, snapshot.1].into_iter().flatten() {
                if let Some(slot) = self.clip_mut(c.id) {
                    *slot = c;
                }
            }
        }
        res
    }

    /// Make block `b` start (`start`) or end with `prop` moving at `speed` (as `animate::speed` measures it).
    fn set_block_speed(&mut self, (id, i): (Id, usize), prop: &str, start: bool, speed_v: f64) -> Result<f64, String> {
        let c = self.clip(id).ok_or("no such clip")?;
        let b = c.blocks.get(i).ok_or("no such block")?.clone();
        let a = prop_ref(c, prop).ok_or("no such property")?;
        let edge = if start { b.t } else { b.end() };
        let k = a.key_index_at(edge).ok_or_else(|| format!("{} does not move {prop}", b.name))?;
        let (k0, k1) = if start { (k, k + 1) } else { (k.wrapping_sub(1), k) };
        let (Some(&ka), Some(&kb)) = (a.keys.get(k0), a.keys.get(k1)) else {
            return Err(format!("{} does not move {prop} there", b.name));
        };
        let (dv, dt) = (kb.v - ka.v, kb.t - ka.t);
        if dv.abs() < 1e-9 || dt < 1e-6 {
            return Err(format!("{} holds {prop} still there", b.name));
        }
        // undo the log rate for scale: d(ln s)/dt * 100 -> ds/dt
        let v = if prop.starts_with("scale") { speed_v / 100.0 * (if start { ka.v } else { kb.v }) } else { speed_v };
        let slope = v * dt / dv;
        let c = self.clip_mut(id).ok_or("no such clip")?;
        if let Some(ctl) = c.blocks[i].ctl.as_mut().filter(|c| !c.edited) {
            let rec = Some((prop.to_string(), slope));
            if start {
                ctl.in_speed = rec;
            } else {
                ctl.out_speed = rec;
            }
        }
        let a = prop_mut(c, prop).ok_or("no such property")?;
        let e = a.keys[k0].ease;
        a.keys[k0].ease = if start { ease_with_in_slope(e, slope) } else { ease_with_out_slope(e, slope) };
        Ok(speed_v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proj() -> (Project, Id, Id) {
        let mut p = Project::new();
        p.tracks[0].clips.push(Clip::new(1, ClipKind::Video, "a", 0.0, 6.0));
        p.tracks[0].clips.push(Clip::new(2, ClipKind::Video, "b", 6.0, 6.0));
        (p, 1, 2)
    }
    fn block(name: &str) -> KeyBlock {
        find_block(&[], name).unwrap()
    }

    #[test]
    fn controls_at_rest_reproduce_the_preset_exactly() {
        for b in builtin_blocks().into_iter().filter(|b| b.gen.is_none()) {
            let ctl = BlockCtl::new(b.clone());
            assert_eq!(ctl.resolve(b.dur).tracks, b.tracks, "{}", b.name);
        }
    }

    #[test]
    fn side_turns_a_slide_and_strength_scales_it() {
        let mut ctl = BlockCtl::new(block("Slide In Left"));
        assert_eq!(ctl.dir, Some(Dir::Left));
        ctl.dir = Some(Dir::Down);
        ctl.strength = 0.5;
        let b = ctl.resolve(0.6);
        assert_eq!(b.tracks[0].prop, "y");
        assert_eq!(b.tracks[0].keys[0].v, BlockVal::Add(0.5), "starts half a frame below");
        ctl.dir = Some(Dir::Right);
        assert_eq!(ctl.resolve(0.6).tracks[0].keys[0].v, BlockVal::Add(0.5));
        // a pop at half strength grows from half size, never a negative scale
        let mut pop = BlockCtl::new(block("Pop In"));
        pop.strength = 2.5;
        assert!(pop.resolve(0.4).tracks[0].keys.iter().all(|k| !matches!(k.v, BlockVal::Mul(v) if v < 0.0)));
    }

    #[test]
    fn simulated_blocks_end_at_rest_and_shake_is_repeatable() {
        for g in Gen::ALL {
            let b = simulate(g, &Phys::default(), 1.0, Dir::Left, 1.0);
            assert!(!b.tracks.is_empty(), "{g:?}");
            for t in &b.tracks {
                assert!(t.keys.len() >= 2 && t.keys[0].f == 0.0 && t.keys.last().unwrap().f == 1.0);
                if g != Gen::ThrowOut && t.prop != "opacity" {
                    let end = t.keys.last().unwrap().v;
                    assert!(
                        matches!(end, BlockVal::Add(v) if v.abs() < 1e-3) || end == BlockVal::Mul(1.0),
                        "{g:?} {}",
                        t.prop
                    );
                }
            }
        }
        let a = simulate(Gen::Shake, &Phys::default(), 1.0, Dir::Left, 1.0);
        assert_eq!(a, simulate(Gen::Shake, &Phys::default(), 1.0, Dir::Left, 1.0));
        let other = Phys { seed: 7, ..Phys::default() };
        assert_ne!(a, simulate(Gen::Shake, &other, 1.0, Dir::Left, 1.0));
        assert!(a.tracks.iter().any(|t| t.prop == "scale"), "camera shake zooms to cover the edges");
    }

    #[test]
    fn a_fitted_curve_follows_the_simulation() {
        let mut a = spring(1.0, 12.0, 0.2, 1.0, 0.0);
        settle(&mut a, 0.0, 0.08);
        let tr = fit("x", &a, BlockVal::Add);
        let mut anim = Animated::new(0.0);
        for k in &tr.keys {
            let BlockVal::Add(v) = k.v else { unreachable!() };
            anim.keys.push(Keyframe { t: k.f, v, ease: k.ease });
        }
        for i in 0..=N {
            let f = i as f64 / N as f64;
            assert!((anim.at(f) - a[i]).abs() < 0.03, "at {f}: {} vs {}", anim.at(f), a[i]);
        }
    }

    #[test]
    fn flow_matches_a_zoom_across_a_cut() {
        let (mut p, a, b) = proj();
        let zoom = KeyBlock {
            name: "Zoom".into(),
            dur: 1.0,
            tracks: vec![BlockTrack {
                prop: "scale".into(),
                keys: vec![
                    BlockKey { f: 0.0, v: BlockVal::Mul(1.0), ease: Ease::EaseIn },
                    BlockKey { f: 1.0, v: BlockVal::Mul(1.4), ease: Ease::Linear },
                ],
            }],
            gen: None,
        };
        p.apply_key_block(a, &zoom, Some(5.0), None).unwrap();
        let mut smooth = zoom.clone();
        smooth.tracks[0].keys[0].ease = Ease::EaseInOut;
        p.apply_key_block(b, &smooth, Some(0.0), None).unwrap();
        let s = p.block_seams();
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].left, s[0].right, s[0].prop.as_str()), ((a, 0), (b, 0), "scale"));
        assert!(!s[0].smooth && s[0].v_left > 30.0 && s[0].v_right.abs() < 1.0);
        let v = p.match_seam((a, 0), (b, 0), Match::Next).unwrap();
        let s = &p.block_seams()[0];
        assert!(s.smooth, "{s:?}");
        assert!((s.v_right - v).abs() < v * 0.03, "{} vs {v}", s.v_right);
        assert!((s.v_left - v).abs() < 1e-6, "the earlier move is untouched");
    }

    #[test]
    fn flow_on_one_clip_and_meeting_halfway() {
        let (mut p, a, _) = proj();
        p.apply_key_block(a, &block("Slide In Left"), Some(0.0), None).unwrap();
        p.apply_key_block(a, &block("Slide Out Right"), Some(0.6), None).unwrap();
        let s = p.block_seams();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].prop, "x");
        let m = p.match_seam((a, 0), (a, 1), Match::Both).unwrap();
        let s = &p.block_seams()[0];
        // speeds are measured a frame-fraction away from the seam, so "equal" is within Flow's own tolerance
        assert!(
            s.smooth && (s.v_left - m).abs() < m.abs() * 0.05 + 5.0 && (s.v_right - m).abs() < m.abs() * 0.05 + 5.0,
            "{s:?} {m}"
        );
    }

    #[test]
    fn a_hold_cannot_be_matched_and_nothing_changes() {
        let (mut p, a, b) = proj();
        p.apply_key_block(a, &block("Zoom Punch"), Some(5.5), None).unwrap();
        p.apply_key_block(b, &block("Typewriter"), Some(0.0), None);
        let before = p.clone();
        // Typewriter moves no motion property: no seam at all
        assert!(p.block_seams().is_empty());
        assert!(p.match_seam((a, 0), (b, 0), Match::Next).is_err());
        assert_eq!(serde_json::to_string(&p).unwrap(), serde_json::to_string(&before).unwrap());
    }
}
