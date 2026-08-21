//! Per-clip effects (CPU, RGBA8). `apply` runs on the decoded layer image (at its decode size, straight
//! alpha) in stack order, before placement/blending. Pixel-sized parameters (blur radius, pixel block,
//! wobble amplitude) are project pixels; `scale` = canvas px per project px converts them.
//! `Wobble` is geometric: it does not touch pixels — the compositor adds `wobble()` to the placement.
//!
//! Implementations (all O(pixels), no per-call allocation beyond `scratch`):
//!  * Blur: 3 passes of a separable box blur ≈ Gaussian (radius*scale px).
//!  * Pixelate: average over blocks of (size*scale) px.
//!  * Tint: mix each pixel towards (r,g,b) by `amount`.
//!  * Color: brightness (add), contrast (around 0.5), saturation (mix with luma), hue (rotate in YIQ),
//!    gamma (LUT); build one 256-entry LUT per channel where possible.
//!  * Vignette: darken by `strength` outside `radius` (normalised to half the diagonal) with `softness`.
//!  * Sharpen: unsharp mask (img + amount * (img - blur(radius))).
//!  * Invert / Grayscale: mix by `amount`.
//!  * Flip: horizontal / vertical mirror (params >= 0.5 = on).
//!  * Crop: fractions cut from each edge (alpha = 0), with an optional feathered edge.

use crate::media::Frame;
use crate::model::{Effect, EffectKind};

/// Apply one effect in place at clip-local time `t`. `scratch` is a reusable buffer.
pub fn apply(_effect: &Effect, _t: f64, _scale: f32, _img: &mut Frame, _scratch: &mut Frame) {
    todo!("engine::effects::apply")
}

/// Camera-shake deltas at clip-local time t: (dx, dy) in project px, (roll, yaw, pitch) in degrees.
/// Deterministic from the seed, smooth (sum of a few incommensurate sines), frequency in Hz.
pub fn wobble(_effect: &Effect, _t: f64) -> (f64, f64, f64, f64, f64) {
    todo!("engine::effects::wobble")
}

/// True for effects that only move the layer (handled by the compositor's placement, not `apply`).
pub fn is_geometric(kind: EffectKind) -> bool {
    kind == EffectKind::Wobble
}
