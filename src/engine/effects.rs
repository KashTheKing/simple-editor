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
pub fn apply(effect: &Effect, t: f64, scale: f32, img: &mut Frame, scratch: &mut Frame) {
    if img.is_empty() || !effect.enabled {
        return;
    }
    let e = |i: usize| effect.at(i, t);
    match effect.kind {
        EffectKind::Blur => {
            // 3 box passes of radius r/2 ≈ Gaussian with sigma ≈ r/2 (visual radius ≈ r)
            let r = (e(0) * scale as f64 * 0.5).round() as i64;
            if r > 0 {
                for _ in 0..3 {
                    blur_pass(img, scratch, r as usize);
                }
            }
        }
        EffectKind::Pixelate => pixelate(img, (e(0) * scale as f64).round().max(1.0) as u32),
        EffectKind::Tint => {
            let a = e(3).clamp(0.0, 1.0);
            let target = [e(0), e(1), e(2)];
            let mut lut = [[0u8; 256]; 3];
            for c in 0..3 {
                for (v, o) in lut[c].iter_mut().enumerate() {
                    *o = (v as f64 + (target[c].clamp(0.0, 255.0) - v as f64) * a + 0.5) as u8;
                }
            }
            for px in img.rgba.chunks_exact_mut(4) {
                px[0] = lut[0][px[0] as usize];
                px[1] = lut[1][px[1] as usize];
                px[2] = lut[2][px[2] as usize];
            }
        }
        EffectKind::Color => color(img, e(0), e(1), e(2), e(3), e(4)),
        EffectKind::Vignette => vignette(img, e(0) as f32, e(1) as f32, e(2) as f32),
        EffectKind::Sharpen => sharpen(img, scratch, e(0) as f32, (e(1) * scale as f64).round().max(1.0) as usize),
        EffectKind::Invert => {
            let a = e(0).clamp(0.0, 1.0);
            let mut lut = [0u8; 256];
            for (v, o) in lut.iter_mut().enumerate() {
                *o = (v as f64 + (255.0 - 2.0 * v as f64) * a + 0.5) as u8;
            }
            for px in img.rgba.chunks_exact_mut(4) {
                px[0] = lut[px[0] as usize];
                px[1] = lut[px[1] as usize];
                px[2] = lut[px[2] as usize];
            }
        }
        EffectKind::Grayscale => {
            let ai = (e(0).clamp(0.0, 1.0) * 256.0) as i32;
            for px in img.rgba.chunks_exact_mut(4) {
                // Rec.709 luma in 8.8 fixed point, then integer mix: v + (l - v) * a
                let l = ((px[0] as u32 * 54 + px[1] as u32 * 183 + px[2] as u32 * 19) >> 8) as i32;
                for c in 0..3 {
                    let v = px[c] as i32;
                    px[c] = (v + (((l - v) * ai) >> 8)) as u8;
                }
            }
        }
        EffectKind::Flip => flip(img, e(0) >= 0.5, e(1) >= 0.5),
        EffectKind::Crop => crop(img, e(0) as f32, e(1) as f32, e(2) as f32, e(3) as f32, e(4) as f32),
        EffectKind::Wobble => {} // geometric — handled by the compositor's placement
    }
}

/// Camera-shake deltas at clip-local time t: (dx, dy) in project px, (roll, yaw, pitch) in degrees.
/// Deterministic from the seed, smooth (sum of a few incommensurate sines), frequency in Hz.
pub fn wobble(effect: &Effect, t: f64) -> (f64, f64, f64, f64, f64) {
    let freq = effect.at(5, t).max(0.0);
    let seed = effect.at(6, t);
    let w = std::f64::consts::TAU * freq * t;
    let n = |axis: f64| -> f64 {
        let p = seed * 1.7 + axis * 13.37;
        0.5 * (w + p).sin() + 0.35 * (w * 1.618_034 + p * 2.236).sin() + 0.15 * (w * 2.718_281_8 + p * 3.19).sin()
    };
    (
        effect.at(0, t) * n(1.0),
        effect.at(1, t) * n(2.0),
        effect.at(2, t) * n(3.0),
        effect.at(3, t) * n(4.0),
        effect.at(4, t) * n(5.0),
    )
}

/// True for effects that only move the layer (handled by the compositor's placement, not `apply`).
pub fn is_geometric(kind: EffectKind) -> bool {
    kind == EffectKind::Wobble
}

/// One separable box pass (radius `r`, edge-replicated): rows img→scratch, columns scratch→img.
fn blur_pass(img: &mut Frame, scratch: &mut Frame, r: usize) {
    scratch.resize(img.width, img.height);
    blur_dir(img, scratch, true, r);
    blur_dir(scratch, img, false, r);
}

/// 1-D sliding-window box blur of all 4 channels along rows (`horiz`) or columns, edge replicate.
fn blur_dir(src: &Frame, dst: &mut Frame, horiz: bool, r: usize) {
    let (w, h) = (src.width as usize, src.height as usize);
    if w == 0 || h == 0 {
        return;
    }
    let (lines, len, ls, es) = if horiz { (h, w, w, 1) } else { (w, h, 1, w) };
    let r = r.min(len - 1);
    if r == 0 {
        dst.rgba.copy_from_slice(&src.rgba);
        return;
    }
    let norm = (2 * r + 1) as u32;
    for l in 0..lines {
        let at = |j: usize| (l * ls + j * es) * 4;
        let mut sum = [0u32; 4];
        let a0 = at(0);
        for (k, s) in sum.iter_mut().enumerate() {
            *s = (r as u32 + 1) * src.rgba[a0 + k] as u32;
        }
        for j in 1..=r {
            let a = at(j.min(len - 1));
            for (k, s) in sum.iter_mut().enumerate() {
                *s += src.rgba[a + k] as u32;
            }
        }
        for j in 0..len {
            let d = at(j);
            for (k, s) in sum.iter().enumerate() {
                dst.rgba[d + k] = ((s + norm / 2) / norm) as u8;
            }
            let add = at((j + r + 1).min(len - 1));
            let sub = at(j.saturating_sub(r));
            for (k, s) in sum.iter_mut().enumerate() {
                *s += src.rgba[add + k] as u32;
                *s -= src.rgba[sub + k] as u32;
            }
        }
    }
}

/// Average each block of `block`×`block` px in place.
fn pixelate(img: &mut Frame, block: u32) {
    if block <= 1 {
        return;
    }
    let (w, h) = (img.width as usize, img.height as usize);
    let b = block as usize;
    let stride = img.stride();
    for by in (0..h).step_by(b) {
        let y1 = (by + b).min(h);
        for bx in (0..w).step_by(b) {
            let x1 = (bx + b).min(w);
            let mut sum = [0u32; 4];
            for y in by..y1 {
                let row = y * stride;
                for x in bx..x1 {
                    let p = row + x * 4;
                    for (k, s) in sum.iter_mut().enumerate() {
                        *s += img.rgba[p + k] as u32;
                    }
                }
            }
            let n = ((y1 - by) * (x1 - bx)) as u32;
            let avg = [
                ((sum[0] + n / 2) / n) as u8,
                ((sum[1] + n / 2) / n) as u8,
                ((sum[2] + n / 2) / n) as u8,
                ((sum[3] + n / 2) / n) as u8,
            ];
            for y in by..y1 {
                let row = y * stride;
                for x in bx..x1 {
                    img.rgba[row + x * 4..row + x * 4 + 4].copy_from_slice(&avg);
                }
            }
        }
    }
}

/// Brightness/contrast/gamma via one LUT; saturation + hue via one combined 3×3 matrix (Rec.709 luma).
fn color(img: &mut Frame, bright: f64, contrast: f64, sat: f64, hue_deg: f64, gamma: f64) {
    let g = if gamma > 0.001 { gamma } else { 1.0 };
    let mut lut = [0u8; 256];
    for (v, o) in lut.iter_mut().enumerate() {
        let f = ((v as f64 / 255.0 - 0.5) * contrast + 0.5 + bright).clamp(0.0, 1.0);
        *o = (f.powf(1.0 / g) * 255.0 + 0.5) as u8;
    }
    let use_mat = (sat - 1.0).abs() > 1e-3 || hue_deg.abs() > 1e-3;
    if !use_mat {
        for px in img.rgba.chunks_exact_mut(4) {
            px[0] = lut[px[0] as usize];
            px[1] = lut[px[1] as usize];
            px[2] = lut[px[2] as usize];
        }
        return;
    }
    const L: [f64; 3] = [0.2126, 0.7152, 0.0722];
    // saturation: s*I + (1-s)*luma
    let mut sm = [[0.0f64; 3]; 3];
    for (i, row) in sm.iter_mut().enumerate() {
        for (j, m) in row.iter_mut().enumerate() {
            *m = (1.0 - sat) * L[j] + if i == j { sat } else { 0.0 };
        }
    }
    // hue rotation about the gray axis (SVG feColorMatrix hueRotate)
    let (s, c) = hue_deg.to_radians().sin_cos();
    let hm = [
        [0.213 + c * 0.787 - s * 0.213, 0.715 - c * 0.715 - s * 0.715, 0.072 - c * 0.072 + s * 0.928],
        [0.213 - c * 0.213 + s * 0.143, 0.715 + c * 0.285 + s * 0.140, 0.072 - c * 0.072 - s * 0.283],
        [0.213 - c * 0.213 - s * 0.787, 0.715 - c * 0.715 + s * 0.715, 0.072 + c * 0.928 + s * 0.072],
    ];
    // combined = hue * sat
    let mut m = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = (hm[i][0] * sm[0][j] + hm[i][1] * sm[1][j] + hm[i][2] * sm[2][j]) as f32;
        }
    }
    for px in img.rgba.chunks_exact_mut(4) {
        let (r, gr, b) = (px[0] as f32, px[1] as f32, px[2] as f32);
        for (i, row) in m.iter().enumerate() {
            let v = (row[0] * r + row[1] * gr + row[2] * b).clamp(0.0, 255.0) as usize;
            px[i] = lut[v];
        }
    }
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Darken by `strength` outside `radius` (0..1.5 of the half diagonal) with a `softness` falloff.
fn vignette(img: &mut Frame, radius: f32, softness: f32, strength: f32) {
    let (w, h) = (img.width as usize, img.height as usize);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let inv_hd = 1.0 / (cx * cx + cy * cy).sqrt().max(1.0);
    let soft = softness.max(1e-3);
    let stride = img.stride();
    for y in 0..h {
        let dy = y as f32 + 0.5 - cy;
        let row = y * stride;
        for x in 0..w {
            let dx = x as f32 + 0.5 - cx;
            let d = (dx * dx + dy * dy).sqrt() * inv_hd;
            let fall = smoothstep((d - radius) / soft);
            if fall <= 0.0 {
                continue;
            }
            let k = 1.0 - strength.clamp(0.0, 1.0) * fall;
            let px = &mut img.rgba[row + x * 4..row + x * 4 + 4];
            px[0] = (px[0] as f32 * k) as u8;
            px[1] = (px[1] as f32 * k) as u8;
            px[2] = (px[2] as f32 * k) as u8;
        }
    }
}

/// Unsharp mask: img + amount * (img - box_blur(img, r)). One separable box pass; the vertical
/// pass reads the h-blurred scratch and combines with the untouched original in place.
fn sharpen(img: &mut Frame, scratch: &mut Frame, amount: f32, r: usize) {
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 || amount <= 0.0 {
        return;
    }
    scratch.resize(img.width, img.height);
    blur_dir(img, scratch, true, r);
    let r = r.min(h - 1);
    let norm = (2 * r + 1) as f32;
    for x in 0..w {
        let at = |y: usize| (y * w + x) * 4;
        let mut sum = [0f32; 4];
        let a0 = at(0);
        for (k, s) in sum.iter_mut().enumerate() {
            *s = (r as f32 + 1.0) * scratch.rgba[a0 + k] as f32;
        }
        for j in 1..=r {
            let a = at(j.min(h - 1));
            for (k, s) in sum.iter_mut().enumerate() {
                *s += scratch.rgba[a + k] as f32;
            }
        }
        for y in 0..h {
            let d = at(y);
            for k in 0..3 {
                let blurred = sum[k] / norm;
                let o = img.rgba[d + k] as f32;
                img.rgba[d + k] = (o + amount * (o - blurred)).clamp(0.0, 255.0) as u8;
            }
            let add = at((y + r + 1).min(h - 1));
            let sub = at(y.saturating_sub(r));
            for (k, s) in sum.iter_mut().enumerate() {
                *s += scratch.rgba[add + k] as f32;
                *s -= scratch.rgba[sub + k] as f32;
            }
        }
    }
}

fn flip(img: &mut Frame, horizontal: bool, vertical: bool) {
    let (w, h) = (img.width as usize, img.height as usize);
    let stride = img.stride();
    if horizontal {
        for y in 0..h {
            let row = &mut img.rgba[y * stride..y * stride + w * 4];
            for x in 0..w / 2 {
                let (a, b) = (x * 4, (w - 1 - x) * 4);
                for k in 0..4 {
                    row.swap(a + k, b + k);
                }
            }
        }
    }
    if vertical {
        for y in 0..h / 2 {
            let (top, rest) = img.rgba.split_at_mut((h - 1 - y) * stride);
            top[y * stride..y * stride + stride].swap_with_slice(&mut rest[..stride]);
        }
    }
}

/// Cut `l/r/t/b` fractions from the edges (alpha 0 outside), feathering over `feather * min(w,h)` px.
fn crop(img: &mut Frame, l: f32, r: f32, tp: f32, b: f32, feather: f32) {
    let (w, h) = (img.width as f32, img.height as f32);
    let (x0, x1) = (l.clamp(0.0, 0.5) * w, w - r.clamp(0.0, 0.5) * w);
    let (y0, y1) = (tp.clamp(0.0, 0.5) * h, h - b.clamp(0.0, 0.5) * h);
    let fe = feather.clamp(0.0, 0.5) * w.min(h);
    let stride = img.stride();
    for y in 0..img.height as usize {
        let yc = y as f32 + 0.5;
        let dy = (yc - y0).min(y1 - yc);
        let row = y * stride;
        for x in 0..img.width as usize {
            let xc = x as f32 + 0.5;
            let d = (xc - x0).min(x1 - xc).min(dy);
            let a = if d <= 0.0 {
                0.0
            } else if fe > 0.0 {
                (d / fe).min(1.0)
            } else {
                1.0
            };
            if a >= 1.0 {
                continue;
            }
            let p = row + x * 4 + 3;
            img.rgba[p] = (img.rgba[p] as f32 * a) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EffectKind as K;

    fn eff(kind: K, params: &[f64]) -> Effect {
        let mut e = Effect::new(kind);
        for (i, v) in params.iter().enumerate() {
            e.params[i].value = *v;
        }
        e
    }

    /// Deterministic pseudo-random test frame.
    fn noisy(w: u32, h: u32) -> Frame {
        let mut f = Frame::new(w, h);
        let mut s = 0x12345678u32;
        for px in f.rgba.chunks_exact_mut(4) {
            for c in px.iter_mut().take(3) {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                *c = (s >> 24) as u8;
            }
            px[3] = 255;
        }
        f
    }

    fn variance(f: &Frame) -> f64 {
        let vals: Vec<f64> = f.rgba.chunks_exact(4).map(|p| p[0] as f64).collect();
        let m = vals.iter().sum::<f64>() / vals.len() as f64;
        vals.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / vals.len() as f64
    }

    fn mean(f: &Frame, c: usize) -> f64 {
        let n = (f.width * f.height) as f64;
        f.rgba.chunks_exact(4).map(|p| p[c] as f64).sum::<f64>() / n
    }

    #[test]
    fn blur_lowers_variance() {
        let mut img = noisy(16, 16);
        let v0 = variance(&img);
        let mut scratch = Frame::default();
        apply(&eff(K::Blur, &[4.0]), 0.0, 1.0, &mut img, &mut scratch);
        assert!(variance(&img) < v0 * 0.5, "{} -> {}", v0, variance(&img));
        // zero radius: no-op
        let mut img2 = noisy(16, 16);
        let before = img2.rgba.clone();
        apply(&eff(K::Blur, &[0.0]), 0.0, 1.0, &mut img2, &mut scratch);
        assert_eq!(img2.rgba, before);
    }

    #[test]
    fn pixelate_makes_blocks_uniform() {
        let mut img = noisy(8, 8);
        let mut scratch = Frame::default();
        apply(&eff(K::Pixelate, &[4.0]), 0.0, 1.0, &mut img, &mut scratch);
        for by in [0u32, 4] {
            for bx in [0u32, 4] {
                let i0 = ((by * 8 + bx) * 4) as usize;
                let first = img.rgba[i0..i0 + 4].to_vec();
                for y in by..by + 4 {
                    for x in bx..bx + 4 {
                        let i = ((y * 8 + x) * 4) as usize;
                        assert_eq!(&img.rgba[i..i + 4], &first[..], "block {bx},{by} at {x},{y}");
                    }
                }
            }
        }
    }

    #[test]
    fn tint_moves_towards_colour() {
        let mut img = noisy(8, 8);
        let d0 = mean(&img, 0);
        let mut scratch = Frame::default();
        apply(&eff(K::Tint, &[255.0, 0.0, 0.0, 0.5]), 0.0, 1.0, &mut img, &mut scratch);
        assert!(mean(&img, 0) > d0, "red should rise");
        assert!(mean(&img, 1) < 128.0, "green should fall towards 0");
        // amount 1 = exactly the colour
        apply(&eff(K::Tint, &[10.0, 20.0, 30.0, 1.0]), 0.0, 1.0, &mut img, &mut scratch);
        for p in img.rgba.chunks_exact(4) {
            assert_eq!(&p[..3], &[10, 20, 30]);
        }
    }

    #[test]
    fn color_brightness_contrast_saturation() {
        let mut scratch = Frame::default();
        // brightness raises the mean monotonically
        let base = noisy(8, 8);
        let mut prev = 0.0;
        for b in [-0.5, 0.0, 0.5] {
            let mut img = base.clone();
            apply(&eff(K::Color, &[b, 1.0, 1.0, 0.0, 1.0]), 0.0, 1.0, &mut img, &mut scratch);
            let m = mean(&img, 0);
            assert!(m >= prev, "brightness {b}: {m} < {prev}");
            prev = m;
        }
        // higher contrast -> higher variance
        let mut low = base.clone();
        apply(&eff(K::Color, &[0.0, 0.5, 1.0, 0.0, 1.0]), 0.0, 1.0, &mut low, &mut scratch);
        let mut high = base.clone();
        apply(&eff(K::Color, &[0.0, 2.0, 1.0, 0.0, 1.0]), 0.0, 1.0, &mut high, &mut scratch);
        assert!(variance(&low) < variance(&base));
        assert!(variance(&high) > variance(&low));
        // saturation 0 -> gray (r == g == b)
        let mut gray = base.clone();
        apply(&eff(K::Color, &[0.0, 1.0, 0.0, 0.0, 1.0]), 0.0, 1.0, &mut gray, &mut scratch);
        for p in gray.rgba.chunks_exact(4) {
            assert!((p[0] as i32 - p[1] as i32).abs() <= 1 && (p[1] as i32 - p[2] as i32).abs() <= 1, "{p:?}");
        }
        // gamma > 1 brightens mid-tones (v^(1/g))
        let mut g = Frame::new(2, 2);
        g.fill([128, 128, 128, 255]);
        apply(&eff(K::Color, &[0.0, 1.0, 1.0, 0.0, 2.0]), 0.0, 1.0, &mut g, &mut scratch);
        assert!(g.rgba[0] > 150, "{}", g.rgba[0]);
    }

    #[test]
    fn invert_and_grayscale() {
        let mut img = Frame::new(2, 1);
        img.rgba.copy_from_slice(&[200, 50, 100, 255, 0, 255, 30, 255]);
        let mut scratch = Frame::default();
        apply(&eff(K::Invert, &[1.0]), 0.0, 1.0, &mut img, &mut scratch);
        assert_eq!(&img.rgba[..4], &[55, 205, 155, 255]);
        assert_eq!(&img.rgba[4..8], &[255, 0, 225, 255]);
        // amount 0 = no-op
        let before = img.rgba.clone();
        apply(&eff(K::Invert, &[0.0]), 0.0, 1.0, &mut img, &mut scratch);
        assert_eq!(img.rgba, before);
        // grayscale 1: channels equalish, luma-weighted
        let mut img = Frame::new(1, 1);
        img.rgba.copy_from_slice(&[255, 0, 0, 255]);
        apply(&eff(K::Grayscale, &[1.0]), 0.0, 1.0, &mut img, &mut scratch);
        let p = img.rgba[..3].to_vec();
        assert!((p[0] as i32 - p[1] as i32).abs() <= 1, "{p:?}");
        assert!(p[0] > 40 && p[0] < 70, "Rec709 red luma ~54: {p:?}");
    }

    #[test]
    fn flip_mirrors() {
        let mut img = Frame::new(2, 2);
        // TL red, TR green, BL blue, BR white
        img.rgba.copy_from_slice(&[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255]);
        let mut scratch = Frame::default();
        apply(&eff(K::Flip, &[1.0, 0.0]), 0.0, 1.0, &mut img, &mut scratch);
        assert_eq!(&img.rgba[..4], &[0, 255, 0, 255], "horizontal: TL is old TR");
        apply(&eff(K::Flip, &[0.0, 1.0]), 0.0, 1.0, &mut img, &mut scratch);
        assert_eq!(&img.rgba[..4], &[255, 255, 255, 255], "vertical after horizontal: TL is old BR");
    }

    #[test]
    fn crop_zeroes_alpha_outside() {
        let mut img = Frame::new(10, 10);
        img.fill([200, 200, 200, 255]);
        let mut scratch = Frame::default();
        apply(&eff(K::Crop, &[0.2, 0.2, 0.2, 0.2, 0.0]), 0.0, 1.0, &mut img, &mut scratch);
        let a = |img: &Frame, x: u32, y: u32| img.rgba[((y * 10 + x) * 4 + 3) as usize];
        assert_eq!(a(&img, 0, 5), 0);
        assert_eq!(a(&img, 9, 5), 0);
        assert_eq!(a(&img, 5, 0), 0);
        assert_eq!(a(&img, 5, 9), 0);
        assert_eq!(a(&img, 5, 5), 255);
        // feather: partial alpha near the edge
        let mut img = Frame::new(10, 10);
        img.fill([200, 200, 200, 255]);
        apply(&eff(K::Crop, &[0.2, 0.2, 0.2, 0.2, 0.3]), 0.0, 1.0, &mut img, &mut scratch);
        let edge = a(&img, 2, 5);
        assert!(edge > 0 && edge < 255, "feathered edge: {edge}");
        assert_eq!(a(&img, 0, 5), 0);
    }

    #[test]
    fn vignette_darkens_corners_not_centre() {
        let mut img = Frame::new(16, 16);
        img.fill([200, 200, 200, 255]);
        let mut scratch = Frame::default();
        apply(&eff(K::Vignette, &[0.5, 0.3, 1.0]), 0.0, 1.0, &mut img, &mut scratch);
        let v = |x: u32, y: u32| img.rgba[((y * 16 + x) * 4) as usize];
        assert_eq!(v(8, 8), 200, "centre untouched");
        assert!(v(0, 0) < 100, "corner darkened: {}", v(0, 0));
    }

    #[test]
    fn sharpen_raises_edge_contrast() {
        // vertical step edge
        let mut img = Frame::new(16, 8);
        for y in 0..8u32 {
            for x in 0..16u32 {
                let c = if x < 8 { 50 } else { 200 };
                let i = ((y * 16 + x) * 4) as usize;
                img.rgba[i..i + 4].copy_from_slice(&[c, c, c, 255]);
            }
        }
        let mut scratch = Frame::default();
        apply(&eff(K::Sharpen, &[1.5, 2.0]), 0.0, 1.0, &mut img, &mut scratch);
        let v = |x: u32| img.rgba[((4 * 16 + x) * 4) as usize];
        assert!(v(7) < 50, "dark side overshoots darker: {}", v(7));
        assert!(v(8) > 200, "bright side overshoots brighter: {}", v(8));
        assert_eq!(v(0), 50, "flat areas untouched");
    }

    #[test]
    fn wobble_deterministic_and_zero_at_zero_amplitude() {
        let e = eff(K::Wobble, &[20.0, 10.0, 2.0, 3.0, 3.0, 2.0, 7.0]);
        let a = wobble(&e, 0.4);
        let b = wobble(&e, 0.4);
        assert_eq!(a, b, "deterministic");
        assert!(a.0.abs() <= 20.0 && a.1.abs() <= 10.0 && a.2.abs() <= 2.0);
        // moves at some point
        let moved = (0..50).any(|i| wobble(&e, i as f64 * 0.1).0.abs() > 1.0);
        assert!(moved);
        // different seed, different path
        let e2 = eff(K::Wobble, &[20.0, 10.0, 2.0, 3.0, 3.0, 2.0, 8.0]);
        assert_ne!(wobble(&e, 0.4), wobble(&e2, 0.4));
        // zero amplitude = exactly zero
        let z = eff(K::Wobble, &[0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 7.0]);
        assert_eq!(wobble(&z, 1.23), (0.0, 0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn geometric_flag() {
        assert!(is_geometric(EffectKind::Wobble));
        assert!(!is_geometric(EffectKind::Blur));
    }
}
