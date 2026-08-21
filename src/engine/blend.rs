//! Blend modes and the per-pixel composite of a straight-alpha RGBA layer onto an opaque canvas.

use crate::media::Frame;
use crate::model::BlendMode;

/// Blend one channel: `s` = layer (source), `d` = canvas (destination), both 0..1. Returns 0..1.
#[inline]
pub fn blend_channel(mode: BlendMode, s: f32, d: f32) -> f32 {
    use BlendMode::*;
    match mode {
        Normal => s,
        Multiply => s * d,
        Screen => 1.0 - (1.0 - s) * (1.0 - d),
        Overlay => {
            if d <= 0.5 {
                2.0 * s * d
            } else {
                1.0 - 2.0 * (1.0 - s) * (1.0 - d)
            }
        }
        Darken => s.min(d),
        Lighten => s.max(d),
        Add => (s + d).min(1.0),
        Subtract => (d - s).max(0.0),
        Difference => (s - d).abs(),
        SoftLight => {
            if s <= 0.5 {
                d - (1.0 - 2.0 * s) * d * (1.0 - d)
            } else {
                let g = if d <= 0.25 { ((16.0 * d - 12.0) * d + 4.0) * d } else { d.sqrt() };
                d + (2.0 * s - 1.0) * (g - d)
            }
        }
        HardLight => {
            if s <= 0.5 {
                2.0 * s * d
            } else {
                1.0 - 2.0 * (1.0 - s) * (1.0 - d)
            }
        }
        ColorDodge => {
            if d <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (d / (1.0 - s)).min(1.0)
            }
        }
        ColorBurn => {
            if d >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - d) / s).min(1.0)
            }
        }
    }
}

/// Composite `layer` (straight-alpha RGBA, same size as `dst`) onto `dst` (opaque canvas) with the
/// given blend mode and global opacity. Pixels with alpha 0 are skipped. Normal/opacity 1 should take
/// a fast path (alpha lerp only).
pub fn composite(_dst: &mut Frame, _layer: &Frame, _mode: BlendMode, _opacity: f32) {
    todo!("engine::blend::composite")
}
