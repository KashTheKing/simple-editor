//! Keyframe blocks: named, reusable animation snippets ("Fade In", "Pop In", "Shake", …). A block is a
//! few keys per property over a relative 0..1 time, each value relative to the property's value where
//! the block lands - so blocks chain end to end, each continuing from the value the last one left.
//! Applying one only writes ordinary `Keyframe`s (the renderer never sees a block); `Clip.blocks`
//! remembers where each landed so the timeline can draw it as a bar you drag, stretch and delete.

use crate::model::{Animated, Clip, Ease, Keyframe};
use serde::{Deserialize, Serialize};

/// How a block key's value relates to the property's value `base` where the block starts.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum BlockVal {
    /// Exactly this value.
    Abs(f64),
    /// `base × v` (scale, opacity, volume).
    Mul(f64),
    /// `base + v × unit` - unit = frame width for x, frame height for y, else 1 (degrees for rotation).
    Add(f64),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BlockKey {
    /// 0..1 through the block.
    pub f: f64,
    pub v: BlockVal,
    #[serde(default)]
    pub ease: Ease,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BlockTrack {
    /// A `BLOCK_PROPS` name.
    pub prop: String,
    pub keys: Vec<BlockKey>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct KeyBlock {
    pub name: String,
    /// Default length in seconds.
    pub dur: f64,
    pub tracks: Vec<BlockTrack>,
}

/// Where a block was applied on a clip (clip-local seconds) - drawn as a bar on the keyframe row.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AppliedBlock {
    pub name: String,
    pub t: f64,
    pub dur: f64,
    pub props: Vec<String>,
}

impl AppliedBlock {
    pub fn end(&self) -> f64 {
        self.t + self.dur
    }
}

/// The properties a block can animate (`Clip` field names).
pub const BLOCK_PROPS: [&str; 8] = ["x", "y", "scale", "scale_x", "scale_y", "rotation", "opacity", "volume"];

pub fn prop_ref<'a>(c: &'a Clip, name: &str) -> Option<&'a Animated> {
    Some(match name {
        "x" => &c.x,
        "y" => &c.y,
        "scale" => &c.scale,
        "scale_x" => &c.scale_x,
        "scale_y" => &c.scale_y,
        "rotation" => &c.rotation,
        "opacity" => &c.opacity,
        "volume" => &c.volume,
        _ => return None,
    })
}

pub fn prop_mut<'a>(c: &'a mut Clip, name: &str) -> Option<&'a mut Animated> {
    Some(match name {
        "x" => &mut c.x,
        "y" => &mut c.y,
        "scale" => &mut c.scale,
        "scale_x" => &mut c.scale_x,
        "scale_y" => &mut c.scale_y,
        "rotation" => &mut c.rotation,
        "opacity" => &mut c.opacity,
        "volume" => &mut c.volume,
        _ => return None,
    })
}

/// `Add` unit for `prop` in a `w`×`h` frame.
fn unit(prop: &str, w: f64, h: f64) -> f64 {
    match prop {
        "x" => w,
        "y" => h,
        _ => 1.0,
    }
}

pub(crate) const BK_EPS: f64 = 1e-4;

/// Insert `k`, replacing a key already at its time.
pub(crate) fn upsert(a: &mut Animated, k: Keyframe) {
    match a.key_index_at(k.t) {
        Some(i) => a.keys[i] = k,
        None => {
            let i = a.keys.partition_point(|o| o.t < k.t);
            a.keys.insert(i, k);
        }
    }
}

impl KeyBlock {
    /// Write this block's keys into `c` over clip-local [t0, t0 + dur]. Keys of the same properties
    /// strictly inside that span are replaced. Returns the properties written (empty = none applied).
    pub fn write(&self, c: &mut Clip, t0: f64, dur: f64, w: f64, h: f64) -> Vec<String> {
        let mut props = Vec::new();
        let visual = c.is_visual();
        for tr in &self.tracks {
            // volume sounds only on audio clips; everything else only draws on visual ones
            if (tr.prop == "volume") == visual {
                continue;
            }
            let Some(a) = prop_mut(c, &tr.prop) else { continue };
            if !a.link.is_none() {
                a.unlink();
            }
            let base = a.at(t0);
            let u = unit(&tr.prop, w, h);
            a.keys.retain(|k| k.t <= t0 + BK_EPS || k.t > t0 + dur + BK_EPS);
            for k in &tr.keys {
                let v = match k.v {
                    BlockVal::Abs(v) => v,
                    BlockVal::Mul(v) => base * v,
                    BlockVal::Add(v) => base + v * u,
                };
                upsert(a, Keyframe { t: t0 + k.f.clamp(0.0, 1.0) * dur, v, ease: k.ease });
            }
            props.push(tr.prop.clone());
        }
        props
    }
}

impl Clip {
    /// The keys applied block `i` owns: (property, key, shared) - `shared` = the key also sits on an
    /// edge of another block of the same property (the seam of a chain), so removing block `i` keeps it.
    pub fn block_keys(&self, i: usize) -> Vec<(String, Keyframe, bool)> {
        let Some(b) = self.blocks.get(i) else { return Vec::new() };
        let mut out = Vec::new();
        for p in &b.props {
            let Some(a) = prop_ref(self, p) else { continue };
            for k in a.keys.iter().filter(|k| k.t >= b.t - BK_EPS && k.t <= b.end() + BK_EPS) {
                let shared = self.blocks.iter().enumerate().any(|(j, o)| {
                    j != i && o.props.contains(p) && ((o.t - k.t).abs() < BK_EPS || (o.end() - k.t).abs() < BK_EPS)
                });
                out.push((p.clone(), *k, shared));
            }
        }
        out
    }

    /// Save keys in clip-local [from, to] as a reusable block (the "save my own" path). None when fewer
    /// than two distinct key times fall in range.
    pub fn capture_block(&self, name: &str, from: f64, to: f64, w: f64, h: f64) -> Option<KeyBlock> {
        let inr = |k: &&Keyframe| k.t >= from - BK_EPS && k.t <= to + BK_EPS;
        let times: Vec<f64> = BLOCK_PROPS
            .iter()
            .filter_map(|p| prop_ref(self, p))
            .flat_map(|a| a.keys.iter().filter(inr).map(|k| k.t))
            .collect();
        let t0 = times.iter().copied().fold(f64::INFINITY, f64::min);
        let t1 = times.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if !(t1 - t0 > BK_EPS) {
            return None;
        }
        let tracks = BLOCK_PROPS
            .iter()
            .filter_map(|&p| {
                let a = prop_ref(self, p)?;
                let base = a.at(t0);
                let keys: Vec<BlockKey> = a
                    .keys
                    .iter()
                    .filter(inr)
                    .map(|k| BlockKey {
                        f: (k.t - t0) / (t1 - t0),
                        v: match p {
                            "x" | "y" | "rotation" => BlockVal::Add((k.v - base) / unit(p, w, h)),
                            _ if base.abs() > 1e-6 => BlockVal::Mul(k.v / base),
                            _ => BlockVal::Abs(k.v),
                        },
                        ease: k.ease,
                    })
                    .collect();
                (!keys.is_empty()).then(|| BlockTrack { prop: p.to_string(), keys })
            })
            .collect();
        Some(KeyBlock { name: name.to_string(), dur: t1 - t0, tracks })
    }
}

// ---- builtins ----

const OUT: Ease = Ease::EaseOut;
const IN: Ease = Ease::EaseIn;
const IO: Ease = Ease::EaseInOut;
const LIN: Ease = Ease::Linear;
const HOLD: Ease = Ease::Hold;
const SNAP: Ease = Ease::Bezier { x1: 0.9, y1: 0.0, x2: 0.1, y2: 1.0 };
const BACK: Ease = Ease::Bezier { x1: 0.34, y1: 1.56, x2: 0.64, y2: 1.0 };

fn tr(prop: &str, keys: &[(f64, BlockVal, Ease)]) -> BlockTrack {
    BlockTrack { prop: prop.into(), keys: keys.iter().map(|&(f, v, ease)| BlockKey { f, v, ease }).collect() }
}
fn blk(name: &str, dur: f64, tracks: Vec<BlockTrack>) -> KeyBlock {
    KeyBlock { name: name.into(), dur, tracks }
}
/// `n` alternating `Add` offsets of ±`amp` settling back to 0 - shake / wiggle.
fn jitter(prop: &str, n: usize, amp: f64, phase: f64, ease: Ease) -> BlockTrack {
    let mut k: Vec<(f64, BlockVal, Ease)> = (0..n)
        .map(|i| {
            let s = if (i + phase as usize) % 2 == 0 { 1.0 } else { -1.0 };
            let decay = 1.0 - i as f64 / n as f64;
            (i as f64 / n as f64, BlockVal::Add(if i == 0 { 0.0 } else { s * amp * decay }), ease)
        })
        .collect();
    k.push((1.0, BlockVal::Add(0.0), ease));
    tr(prop, &k)
}

/// The shipped blocks, in Gallery order.
pub fn builtin_blocks() -> Vec<KeyBlock> {
    use BlockVal::*;
    let slide_in =
        |name: &str, p: &str, d: f64| blk(name, 0.6, vec![tr(p, &[(0.0, Add(d), OUT), (1.0, Add(0.0), LIN)])]);
    let slide_out =
        |name: &str, p: &str, d: f64| blk(name, 0.6, vec![tr(p, &[(0.0, Add(0.0), IN), (1.0, Add(d), LIN)])]);
    vec![
        blk("Fade In", 0.5, vec![tr("opacity", &[(0.0, Abs(0.0), OUT), (1.0, Abs(1.0), LIN)])]),
        blk("Fade Out", 0.5, vec![tr("opacity", &[(0.0, Mul(1.0), IN), (1.0, Abs(0.0), LIN)])]),
        blk(
            "Pop In",
            0.4,
            vec![
                tr("scale", &[(0.0, Mul(0.0), OUT), (0.7, Mul(1.12), IO), (1.0, Mul(1.0), LIN)]),
                tr("opacity", &[(0.0, Abs(0.0), OUT), (0.3, Abs(1.0), LIN)]),
            ],
        ),
        blk(
            "Pop Out",
            0.35,
            vec![
                tr("scale", &[(0.0, Mul(1.0), OUT), (0.3, Mul(1.1), IN), (1.0, Mul(0.0), LIN)]),
                tr("opacity", &[(0.7, Mul(1.0), LIN), (1.0, Abs(0.0), LIN)]),
            ],
        ),
        slide_in("Slide In Left", "x", -1.0),
        slide_in("Slide In Right", "x", 1.0),
        slide_in("Slide In Top", "y", -1.0),
        slide_in("Slide In Bottom", "y", 1.0),
        slide_out("Slide Out Left", "x", -1.0),
        slide_out("Slide Out Right", "x", 1.0),
        slide_out("Slide Out Top", "y", -1.0),
        slide_out("Slide Out Bottom", "y", 1.0),
        blk("Zoom Punch", 0.5, vec![tr("scale", &[(0.0, Mul(1.0), OUT), (0.25, Mul(1.25), IO), (1.0, Mul(1.0), LIN)])]),
        blk("Shake", 0.5, vec![jitter("x", 10, 0.012, 0.0, IO), jitter("y", 10, 0.01, 1.0, IO)]),
        blk("Spin", 0.8, vec![tr("rotation", &[(0.0, Add(0.0), IO), (1.0, Add(360.0), LIN)])]),
        blk(
            "Bounce",
            0.8,
            vec![tr(
                "y",
                &[
                    (0.0, Add(0.0), OUT),
                    (0.3, Add(-0.08), IN),
                    (0.6, Add(0.0), OUT),
                    (0.8, Add(-0.025), IN),
                    (1.0, Add(0.0), LIN),
                ],
            )],
        ),
        blk("Pulse", 0.6, vec![tr("scale", &[(0.0, Mul(1.0), IO), (0.5, Mul(1.08), IO), (1.0, Mul(1.0), LIN)])]),
        blk(
            "Whip",
            0.3,
            vec![
                tr("x", &[(0.0, Add(0.0), SNAP), (1.0, Add(1.2), LIN)]),
                tr("scale_x", &[(0.0, Mul(1.0), IO), (0.5, Mul(1.3), IO), (1.0, Mul(1.0), LIN)]),
            ],
        ),
        blk(
            "Ken Burns",
            5.0,
            vec![
                tr("scale", &[(0.0, Mul(1.0), LIN), (1.0, Mul(1.2), LIN)]),
                tr("x", &[(0.0, Add(0.0), LIN), (1.0, Add(-0.04), LIN)]),
                tr("y", &[(0.0, Add(0.0), LIN), (1.0, Add(-0.03), LIN)]),
            ],
        ),
        blk(
            "Typewriter",
            1.0,
            vec![tr(
                "opacity",
                &[
                    (0.0, Abs(0.0), HOLD),
                    (0.2, Abs(0.25), HOLD),
                    (0.4, Abs(0.5), HOLD),
                    (0.6, Abs(0.75), HOLD),
                    (0.8, Abs(1.0), HOLD),
                ],
            )],
        ),
        blk(
            "Flash",
            0.5,
            vec![tr(
                "opacity",
                &[
                    (0.0, Mul(1.0), HOLD),
                    (0.2, Abs(0.0), HOLD),
                    (0.4, Mul(1.0), HOLD),
                    (0.6, Abs(0.0), HOLD),
                    (0.8, Mul(1.0), HOLD),
                ],
            )],
        ),
        blk(
            "Drift",
            3.0,
            vec![
                tr("x", &[(0.0, Add(0.0), IO), (1.0, Add(0.03), LIN)]),
                tr("y", &[(0.0, Add(0.0), IO), (1.0, Add(-0.02), LIN)]),
            ],
        ),
        blk(
            "Elastic",
            0.8,
            vec![tr(
                "scale",
                &[
                    (0.0, Mul(0.0), OUT),
                    (0.4, Mul(1.2), IO),
                    (0.6, Mul(0.92), IO),
                    (0.8, Mul(1.04), IO),
                    (1.0, Mul(1.0), LIN),
                ],
            )],
        ),
        blk("Wiggle", 1.0, vec![jitter("rotation", 8, 4.0, 0.0, IO)]),
        blk("Drop In", 0.6, vec![tr("y", &[(0.0, Add(-1.0), IN), (0.75, Add(0.02), OUT), (1.0, Add(0.0), LIN)])]),
        blk("Swing In", 0.6, vec![tr("rotation", &[(0.0, Add(-90.0), BACK), (1.0, Add(0.0), LIN)])]),
        blk("Sound Swell", 1.0, vec![tr("volume", &[(0.0, Abs(0.0), OUT), (1.0, Abs(1.0), LIN)])]),
        blk("Sound Fade", 1.0, vec![tr("volume", &[(0.0, Mul(1.0), IN), (1.0, Abs(0.0), LIN)])]),
    ]
}

/// Deterministic bar/card colour for a block name.
pub fn block_color(name: &str) -> [u8; 3] {
    let h = name.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    crate::model::LABEL_COLORS[h as usize % crate::model::LABEL_COLORS.len()].1
}

/// A block by name (case-insensitive): the user's saved ones first (so a save can shadow a builtin),
/// then the builtins.
pub fn find_block(user: &[KeyBlock], name: &str) -> Option<KeyBlock> {
    user.iter().cloned().chain(builtin_blocks()).find(|b| b.name.eq_ignore_ascii_case(name))
}
