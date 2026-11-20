//! Procedural blocks: Shake, Wobble, Wiggle, Pulse and Drift as parameters, not keys. A procedural block adds
//! an offset - a function of time - on top of a property's own value for the block's span. The offset fades
//! in from exactly zero at the block's start and back to exactly zero at its end, so the clip begins and
//! finishes exactly where it is placed. The motion repeats for as long as the block lasts: stretching the
//! block makes it longer, not slower.
//!
//! Where it is evaluated: each property (`Animated`) carries the layers (`ProcLayer`) of the procedural
//! blocks on it, and `Animated::at` adds them. Every renderer (preview, export, proxies, the GPU path) reads
//! properties through `at`, so they all see it. The block itself (`AppliedBlock.ctl.proc`) holds the
//! controls; changing one rewrites its layers, the way a key block rewrites its keys.
//!
//! Cost: a layer is a few multiplies and an integer hash per sample, nothing allocated; a property with no
//! layers pays one `is_empty` check.

use crate::model::{prop_mut, prop_ref, AppliedBlock, Clip, Keyframe};
use serde::{Deserialize, Serialize};

/// The kinds of procedural block.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProcKind {
    /// Seeded smooth noise (camera shake).
    Shake,
    /// A sine on rotation and scale.
    Wobble,
    /// A smooth random walk.
    Wiggle,
    /// Scale grows and settles back, over and over.
    Pulse,
    /// Slow noise: a gentle float.
    Drift,
}

/// The properties a procedural block can move, in the Inspector's order (X, Y, Rotation, Scale).
pub const PROC_PROPS: [&str; 4] = ["x", "y", "rotation", "scale"];

/// The shortest fade, as a fraction of the block: the offset never starts or stops with a step.
pub const MIN_FADE: f64 = 0.01;

impl ProcKind {
    pub const ALL: [ProcKind; 5] = [ProcKind::Shake, ProcKind::Wobble, ProcKind::Wiggle, ProcKind::Pulse, ProcKind::Drift];
    pub fn name(self) -> &'static str {
        match self {
            ProcKind::Shake => "Shake",
            ProcKind::Wobble => "Wobble",
            ProcKind::Wiggle => "Wiggle",
            ProcKind::Pulse => "Pulse",
            ProcKind::Drift => "Drift",
        }
    }
    pub fn parse(s: &str) -> Option<ProcKind> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("camera shake") || s.eq_ignore_ascii_case("camera_shake") {
            return Some(ProcKind::Shake);
        }
        ProcKind::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(s))
    }
    /// How far each of `PROC_PROPS` moves at strength 1: x / y in frame widths / heights, rotation in degrees,
    /// scale as a fraction of the clip's own scale.
    pub fn amps(self) -> [f64; 4] {
        match self {
            ProcKind::Shake => [0.012, 0.01, 1.5, 0.02],
            ProcKind::Wobble => [0.01, 0.01, 4.0, 0.03],
            ProcKind::Wiggle => [0.02, 0.02, 6.0, 0.04],
            ProcKind::Pulse => [0.01, 0.01, 3.0, 0.08],
            ProcKind::Drift => [0.03, 0.02, 2.0, 0.03],
        }
    }
    /// The properties it moves when first added.
    pub fn default_axes(self) -> Axes {
        match self {
            ProcKind::Shake | ProcKind::Drift => Axes { x: true, y: true, rotation: false, scale: false },
            ProcKind::Wobble => Axes { x: false, y: false, rotation: true, scale: true },
            ProcKind::Wiggle => Axes { x: false, y: false, rotation: true, scale: false },
            ProcKind::Pulse => Axes { x: false, y: false, rotation: false, scale: true },
        }
    }
    /// Cycles (or noise steps) per second.
    pub fn default_speed(self) -> f64 {
        match self {
            ProcKind::Shake => 12.0,
            ProcKind::Wobble => 3.0,
            ProcKind::Wiggle => 2.0,
            ProcKind::Pulse => 1.6,
            ProcKind::Drift => 0.25,
        }
    }
    /// Default length in seconds.
    pub fn default_dur(self) -> f64 {
        match self {
            ProcKind::Shake => 1.0,
            ProcKind::Wobble => 1.5,
            ProcKind::Wiggle => 2.0,
            ProcKind::Pulse => 1.5,
            ProcKind::Drift => 4.0,
        }
    }
    /// The wave on property `axis` (an index into `PROC_PROPS`).
    fn wave(self, axis: usize) -> Wave {
        match self {
            ProcKind::Shake => Wave::Noise,
            ProcKind::Wobble => Wave::Sine,
            ProcKind::Wiggle | ProcKind::Drift => Wave::Walk,
            ProcKind::Pulse if axis == 3 => Wave::Pump,
            ProcKind::Pulse => Wave::Sine,
        }
    }
}

/// Which properties a procedural block moves.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct Axes {
    pub x: bool,
    pub y: bool,
    pub rotation: bool,
    pub scale: bool,
}

impl Axes {
    pub fn get(&self, axis: usize) -> bool {
        [self.x, self.y, self.rotation, self.scale][axis]
    }
    pub fn set(&mut self, axis: usize, on: bool) {
        *[&mut self.x, &mut self.y, &mut self.rotation, &mut self.scale][axis] = on;
    }
    pub fn any(&self) -> bool {
        self.x || self.y || self.rotation || self.scale
    }
}

/// The shape of a layer's motion over time (before the fades). Every wave stays inside -1..1.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Wave {
    /// Smooth value noise, two octaves.
    Noise,
    /// A sine.
    Sine,
    /// A smooth random walk (three octaves of noise).
    Walk,
    /// 0..1: rises and falls back, `(1 - cos) / 2`.
    Pump,
    /// A constant 1 (Camera Shake's zoom that keeps the frame's edges out of view).
    Const,
}

/// A procedural block's controls (the Inspector's sliders).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProcCtl {
    pub kind: ProcKind,
    /// 0..3, 1 = as designed: how far it moves.
    pub strength: f64,
    /// Cycles per second. Stretching the block keeps it.
    pub speed: f64,
    /// Fade In / Fade Out as fractions of the block (they never overlap: `fades`).
    pub fade_in: f64,
    pub fade_out: f64,
    /// The motion also dies down across the block.
    pub decay: bool,
    pub seed: u32,
    pub axes: Axes,
    /// Shake: zoom in just enough that the frame's edges stay out of view (camera shake on footage). It fades
    /// in and out with the shake.
    pub cover: bool,
    /// Ties the block to its layers on the clip's properties (unique per clip).
    pub pid: u32,
}

impl Default for ProcCtl {
    fn default() -> Self {
        ProcCtl::new(ProcKind::Shake)
    }
}

impl ProcCtl {
    pub fn new(kind: ProcKind) -> Self {
        Self {
            kind,
            strength: 1.0,
            speed: kind.default_speed(),
            fade_in: 0.15,
            fade_out: 0.15,
            decay: false,
            seed: 1,
            axes: kind.default_axes(),
            cover: false,
            pid: 0,
        }
    }
    /// The controls a preset named `name` starts with ("Camera Shake" covers the frame's edges).
    pub fn preset(kind: ProcKind, name: &str) -> Self {
        let mut c = Self::new(kind);
        if kind == ProcKind::Shake && name.to_ascii_lowercase().contains("camera") {
            c.cover = true;
            c.speed = 10.0;
            c.axes.rotation = true;
        }
        c
    }
    /// Fade In and Fade Out as fractions of the block: each at least `MIN_FADE`, scaled down together when
    /// they would overlap.
    pub fn fades(&self) -> (f64, f64) {
        fades(self.fade_in, self.fade_out)
    }

    /// The layers this block puts on each property, over clip-local [t, t + dur] in a `w` x `h` frame.
    pub fn layers(&self, t: f64, dur: f64, w: f64, h: f64) -> Vec<(&'static str, ProcLayer)> {
        let (fi, fo) = self.fades();
        let amps = self.kind.amps();
        let s = self.strength.clamp(0.0, 3.0);
        let unit = [w, h, 1.0, 1.0];
        let mk = |axis: usize, wave: Wave, amp: f64| ProcLayer {
            pid: self.pid,
            t,
            dur,
            wave,
            amp,
            speed: self.speed.clamp(0.01, 60.0),
            fade_in: fi,
            fade_out: fo,
            decay: self.decay,
            // each property its own stream of noise
            seed: self.seed.wrapping_mul(0x9E37_79B9).wrapping_add((axis as u32).wrapping_mul(0x85EB_CA6B)),
            mul: axis == 3,
        };
        let mut out: Vec<(&'static str, ProcLayer)> = (0..4)
            .filter(|&a| self.axes.get(a))
            .map(|a| (PROC_PROPS[a], mk(a, self.kind.wave(a), amps[a] * unit[a] * s)))
            .collect();
        if self.cover && self.kind == ProcKind::Shake {
            // far enough that the moved frame still covers the view (x / y and a tilt)
            let mv = |a: usize| if self.axes.get(a) { amps[a] * s } else { 0.0 };
            let z = 2.0 * mv(0).max(mv(1)) + mv(2).to_radians() * 0.6;
            if z > 1e-6 {
                out.push(("scale", ProcLayer { wave: Wave::Const, decay: false, ..mk(3, Wave::Const, z) }));
            }
        }
        out
    }
}

/// See `ProcCtl::fades`.
pub fn fades(fade_in: f64, fade_out: f64) -> (f64, f64) {
    let fi = if fade_in.is_finite() { fade_in.clamp(MIN_FADE, 1.0) } else { 0.15 };
    let fo = if fade_out.is_finite() { fade_out.clamp(MIN_FADE, 1.0) } else { 0.15 };
    let sum = fi + fo;
    if sum > 1.0 {
        (fi / sum, fo / sum)
    } else {
        (fi, fo)
    }
}

/// One procedural block's motion on one property. Stored on the property (`Animated::procs`).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProcLayer {
    /// The block it belongs to (`ProcCtl::pid`).
    pub pid: u32,
    /// Clip-local start and length in seconds.
    pub t: f64,
    pub dur: f64,
    pub wave: Wave,
    /// Peak offset in the property's units (project px, degrees), or a fraction of the value when `mul`.
    pub amp: f64,
    pub speed: f64,
    /// Fractions of `dur` (already made valid by `fades`).
    pub fade_in: f64,
    pub fade_out: f64,
    #[serde(default)]
    pub decay: bool,
    pub seed: u32,
    /// Scales the value (`v * (1 + offset)`) instead of adding to it.
    #[serde(default)]
    pub mul: bool,
}

impl ProcLayer {
    pub fn end(&self) -> f64 {
        self.t + self.dur
    }
    /// The offset at clip-local `t`: exactly 0 at and outside the block's two ends.
    #[inline]
    pub fn offset(&self, t: f64) -> f64 {
        if !(t > self.t && t < self.t + self.dur) || self.dur <= 0.0 {
            return 0.0;
        }
        let u = (t - self.t) / self.dur;
        let mut e = envelope(u, self.fade_in, self.fade_out);
        if self.decay {
            e *= (1.0 - u) * (1.0 - u);
        }
        if e == 0.0 {
            return 0.0;
        }
        self.amp * e * wave(self.wave, self.seed, (t - self.t) * self.speed)
    }
    /// `v` with this layer's offset at `t`.
    #[inline]
    pub fn apply(&self, v: f64, t: f64) -> f64 {
        let o = self.offset(t);
        if o == 0.0 {
            v
        } else if self.mul {
            v * (1.0 + o)
        } else {
            v + o
        }
    }
}

/// `v` with every layer in `layers` applied at `t` (in order).
#[inline]
pub fn apply_layers(layers: &[ProcLayer], v: f64, t: f64) -> f64 {
    layers.iter().fold(v, |v, l| l.apply(v, t))
}

/// 0 at u <= 0 and u >= 1, rising over the first `fi` of the block and falling over the last `fo` with a
/// smoothstep (no step and no kink), 1 between. `fi + fo <= 1`.
#[inline]
pub fn envelope(u: f64, fi: f64, fo: f64) -> f64 {
    if !(u > 0.0 && u < 1.0) {
        return 0.0;
    }
    let ss = |x: f64| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    ss(u / fi.max(1e-9)) * ss((1.0 - u) / fo.max(1e-9))
}

/// A wave at phase `x` (cycles), -1..1 (`Pump` 0..1).
#[inline]
pub fn wave(w: Wave, seed: u32, x: f64) -> f64 {
    use std::f64::consts::TAU;
    let phase = hash(seed, -7) * 0.5 + 0.5; // 0..1: where in its cycle a sine starts
    match w {
        Wave::Noise => 0.75 * noise1(seed, x) + 0.25 * noise1(seed ^ 0x5bd1_e995, x * 2.13 + 3.7),
        Wave::Sine => (TAU * (x + phase)).sin(),
        Wave::Walk => (noise1(seed, x) + 0.5 * noise1(seed ^ 0x27d4_eb2d, x * 2.03 + 11.0) + 0.25 * noise1(seed ^ 0x1656_67b1, x * 4.07 + 5.0)) / 1.75,
        Wave::Pump => 0.5 - 0.5 * (TAU * x).cos(),
        Wave::Const => 1.0,
    }
}

/// Smooth 1D value noise, -1..1: a seeded random value at every whole `x`, eased between (quintic, so the
/// speed is continuous too).
#[inline]
pub fn noise1(seed: u32, x: f64) -> f64 {
    let i = x.floor();
    let f = x - i;
    let s = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let i = i as i64;
    let (a, b) = (hash(seed, i), hash(seed, i + 1));
    a + (b - a) * s
}

/// A seeded random number in -1..1 for integer `i` (splitmix64).
#[inline]
fn hash(seed: u32, i: i64) -> f64 {
    let mut z = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (seed as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

/// A procedural block that was turned into keys (Convert to Keyframes), kept so Make Procedural can undo it
/// while the keys are still what the conversion wrote.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Unbaked {
    pub proc: ProcCtl,
    /// The keys of its properties in [start, end] before the conversion (it sampled over them), and the key
    /// just before the start (the conversion re-eased it to keep the curve).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub under: Vec<(String, Keyframe)>,
    /// The keys the conversion wrote: while the block's keys are still exactly these, it was not edited.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub baked: Vec<(String, Keyframe)>,
}

impl AppliedBlock {
    /// A procedural block's controls.
    pub fn proc(&self) -> Option<&ProcCtl> {
        self.ctl.as_ref().and_then(|c| c.proc.as_ref())
    }
    pub fn is_proc(&self) -> bool {
        self.proc().is_some()
    }
}

impl Clip {
    /// A pid no procedural block of this clip uses yet.
    pub(crate) fn next_pid(&self) -> u32 {
        let blocks = self.blocks.iter().filter_map(|b| b.proc()).map(|p| p.pid);
        let layers = PROC_PROPS.iter().filter_map(|p| prop_ref(self, p)).flat_map(|a| a.procs.iter().map(|l| l.pid));
        blocks.chain(layers).max().unwrap_or(0) + 1
    }

    /// Remove the layers of block `pid` from every property.
    pub(crate) fn clear_proc(&mut self, pid: u32) {
        for p in PROC_PROPS {
            if let Some(a) = prop_mut(self, p) {
                a.procs.retain(|l| l.pid != pid);
            }
        }
    }

    /// Put procedural block `pc`'s layers over clip-local [t, t + dur] (replacing its old ones). Returns the
    /// properties it moves (empty on a sound clip or with no axis on).
    pub(crate) fn write_proc(&mut self, pc: &ProcCtl, t: f64, dur: f64, w: f64, h: f64) -> Vec<String> {
        self.clear_proc(pc.pid);
        if !self.is_visual() {
            return Vec::new();
        }
        let mut props: Vec<String> = Vec::new();
        for (p, l) in pc.layers(t, dur, w, h) {
            let Some(a) = prop_mut(self, p) else { continue };
            if !a.link.is_none() {
                a.unlink();
            }
            a.procs.push(l);
            if !props.iter().any(|q| q == p) {
                props.push(p.to_string());
            }
        }
        props
    }

    /// Bring every procedural layer in line with its block's place and length (after blocks were moved or
    /// stretched); a layer whose block is gone is dropped.
    pub fn sync_procs(&mut self) {
        let spans: Vec<(u32, f64, f64)> = self.blocks.iter().filter_map(|b| Some((b.proc()?.pid, b.t, b.dur))).collect();
        for p in PROC_PROPS {
            if let Some(a) = prop_mut(self, p) {
                a.procs.retain_mut(|l| match spans.iter().find(|s| s.0 == l.pid) {
                    Some(&(_, t, dur)) => {
                        (l.t, l.dur) = (t, dur);
                        true
                    }
                    None => false,
                });
            }
        }
    }

    /// Procedural block `i`'s offset alone on `prop` at clip-local `t` (in the property's units; a scale
    /// layer as a fraction). For the Animator's ghost curve and waveform.
    pub fn proc_offset(&self, i: usize, prop: &str, t: f64) -> f64 {
        let Some(pid) = self.blocks.get(i).and_then(|b| b.proc()).map(|p| p.pid) else { return 0.0 };
        prop_ref(self, prop).map_or(0.0, |a| a.procs.iter().filter(|l| l.pid == pid).map(|l| l.offset(t)).sum())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(wave: Wave) -> ProcLayer {
        ProcCtl::new(ProcKind::Shake).layers(1.0, 2.0, 1920.0, 1080.0)[0].1.clone().with_wave(wave)
    }
    impl ProcLayer {
        fn with_wave(mut self, w: Wave) -> Self {
            self.wave = w;
            self
        }
    }

    #[test]
    fn sampling_is_deterministic_for_a_seed() {
        for w in [Wave::Noise, Wave::Sine, Wave::Walk, Wave::Pump] {
            let l = layer(w);
            let a: Vec<f64> = (0..200).map(|i| l.offset(1.0 + i as f64 * 0.01)).collect();
            let b: Vec<f64> = (0..200).map(|i| l.offset(1.0 + i as f64 * 0.01)).collect();
            assert_eq!(a, b, "{w:?}");
        }
        let mut c = ProcCtl::new(ProcKind::Shake);
        let a = c.layers(0.0, 2.0, 100.0, 100.0)[0].1.offset(0.7);
        c.seed = 2;
        let b = c.layers(0.0, 2.0, 100.0, 100.0)[0].1.offset(0.7);
        assert_ne!(a, b, "another seed, another shake");
    }

    #[test]
    fn offset_is_zero_at_both_ends_and_has_no_step() {
        for k in ProcKind::ALL {
            let mut c = ProcCtl::new(k);
            c.axes = Axes { x: true, y: true, rotation: true, scale: true };
            c.cover = true;
            for (fi, fo) in [(0.15, 0.15), (0.0, 0.0), (0.9, 0.9), (0.5, 0.01)] {
                (c.fade_in, c.fade_out) = (fi, fo);
                for (p, l) in c.layers(0.5, 1.7, 1920.0, 1080.0) {
                    assert_eq!(l.offset(0.5), 0.0, "{k:?} {p} start");
                    assert_eq!(l.offset(0.5 + 1.7), 0.0, "{k:?} {p} end");
                    assert_eq!(l.apply(123.25, 0.5), 123.25);
                    assert_eq!(l.apply(123.25, 2.2), 123.25);
                    // continuous: neighbouring samples stay close everywhere, edges included
                    let n = 20_000;
                    let mut prev = l.offset(0.5 - 0.01);
                    let max = l.amp.abs().max(1e-9);
                    for i in 0..=n {
                        let t = 0.49 + 1.72 * i as f64 / n as f64;
                        let v = l.offset(t);
                        assert!((v - prev).abs() < max * 0.02, "{k:?} {p} jumps at {t}: {prev} -> {v}");
                        prev = v;
                    }
                }
            }
        }
    }

    #[test]
    fn fades_never_overlap() {
        let (a, b) = fades(0.8, 0.6);
        assert!((a + b - 1.0).abs() < 1e-12 && a > b);
        assert_eq!(fades(0.15, 0.15), (0.15, 0.15));
        assert_eq!(fades(0.0, 0.0), (MIN_FADE, MIN_FADE));
    }

    #[test]
    fn stretching_keeps_the_frequency() {
        let c = ProcCtl::new(ProcKind::Wobble);
        let short = c.layers(0.0, 2.0, 100.0, 100.0)[0].1;
        let long = c.layers(0.0, 4.0, 100.0, 100.0)[0].1;
        // in the middle of both (envelope 1), the same time gives the same wave
        for t in [0.8, 0.95, 1.1] {
            let (a, b) = (short.offset(t), long.offset(t));
            assert!((a - b).abs() < 1e-12, "{t}: {a} vs {b}");
        }
        // zero crossings per second match the speed
        let crossings = |l: &ProcLayer, from: f64, to: f64| {
            let mut n = 0;
            let mut prev = l.offset(from);
            let steps = 4000;
            for i in 1..=steps {
                let v = l.offset(from + (to - from) * i as f64 / steps as f64);
                n += (v.signum() != prev.signum() && v != 0.0 && prev != 0.0) as usize;
                prev = v;
            }
            n
        };
        assert_eq!(crossings(&long, 1.0, 3.0), (2.0 * 2.0 * c.speed) as usize);
    }

    #[test]
    fn noise_stays_in_range_and_is_smooth() {
        for i in 0..5000 {
            let x = i as f64 * 0.013 - 20.0;
            let v = noise1(7, x);
            assert!((-1.0..=1.0).contains(&v));
            assert!((noise1(7, x + 1e-6) - v).abs() < 1e-4);
        }
    }
}
