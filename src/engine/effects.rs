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
//!  * Threshold / Levels / Curves: one 256-entry LUT per channel.
//!  * HueShift: RGB -> HSL -> RGB per pixel.
//!  * ChromaKey: Cb/Cr distance to the key colour with spill removal (no edge shrink — that needs
//!    neighbourhood taps; the GPU body does it).
//!  * RecDot: a blinking dot plus a seven-segment HH:MM:SS timecode drawn as rectangles.
//!
//! Still GPU-only (engine/shaders.rs has the fragment bodies, the CPU path leaves the layer alone):
//! Vhs, MotionBlur, EdgeGlow, JpegCompress, the BlobTrack overlay and user Shaders. `track` below is
//! the CPU half of BlobTrack — it runs anywhere, so the tracked centroid can drive properties even
//! without a GL context.

use crate::media::Frame;
use crate::model::{Effect, EffectKind};

/// A pixel-sized parameter (project px) as whole image px at `scale`, floored at `min`. A non-zero
/// request never rounds down to nothing, so a small radius still shows in the small preview canvas
/// instead of appearing only on export (preview == export).
fn px(v: f64, scale: f32, min: f64) -> usize {
    if v <= 0.0 {
        0
    } else {
        (v * scale as f64).round().max(min) as usize
    }
}

/// True for kinds that only exist as GPU shaders (engine/gpu): `apply` leaves the layer untouched, so
/// previews and exports on a machine without a GL context degrade gracefully instead of panicking.
/// Callers that must warn about what a CPU render drops (export) share this list.
pub fn gpu_only(kind: EffectKind) -> bool {
    matches!(
        kind,
        EffectKind::JpegCompress
            | EffectKind::MotionBlur
            | EffectKind::Plane3d
            | EffectKind::EdgeGlow
            | EffectKind::BlobTrack
            | EffectKind::Vhs
            | EffectKind::Shader
    )
}

/// Apply one effect in place at clip-local time `t`. `scratch` is a reusable buffer.
pub fn apply(effect: &Effect, t: f64, scale: f32, img: &mut Frame, scratch: &mut Frame) {
    if img.is_empty() || !effect.enabled || gpu_only(effect.kind) {
        return;
    }
    let e = |i: usize| effect.at(i, t);
    match effect.kind {
        EffectKind::Blur => {
            // 3 box passes of radius r/2 ≈ Gaussian with sigma ≈ r/2 (visual radius ≈ r)
            let r = px(e(0) * 0.5, scale, 1.0);
            if r > 0 {
                for _ in 0..3 {
                    blur_pass(img, scratch, r);
                }
            }
        }
        // block 1 is the no-op, so a requested block of ≥ 2 keeps at least 2 px at preview scale
        EffectKind::Pixelate => pixelate(img, if e(0) >= 2.0 { px(e(0), scale, 2.0) as u32 } else { 1 }),
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
        EffectKind::Sharpen => sharpen(img, scratch, e(0) as f32, px(e(1), scale, 1.0)),
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
        EffectKind::Threshold => threshold(img, e(0) as f32, e(1) as f32, e(2) >= 0.5),
        EffectKind::HueShift => hue_shift(img, e(0) as f32, e(1) as f32, e(2) as f32),
        EffectKind::Levels => levels(img, e(0), e(1), e(2), e(3), e(4)),
        EffectKind::Curves => curves(img, &std::array::from_fn::<f64, 12, _>(|i| e(i))),
        EffectKind::ChromaKey => {
            chroma_key(img, [e(0) as f32, e(1) as f32, e(2) as f32], e(3) as f32, e(4) as f32, e(5) >= 0.5, e(6) as f32)
        }
        EffectKind::RecDot => rec_dot(img, t, scale, e(0), e(1), e(2), e(3) >= 0.5, e(4)),
        // geometric — handled by the compositor's placement (see `wobble`)
        EffectKind::Wobble | EffectKind::Plane3d => {}
        // GPU-only kinds returned above; listed so a new kind is a compile error, not a silent no-op
        EffectKind::JpegCompress
        | EffectKind::MotionBlur
        | EffectKind::EdgeGlow
        | EffectKind::BlobTrack
        | EffectKind::Vhs
        | EffectKind::Shader => {}
    }
}

/// Placement deltas for a geometric effect at clip-local time t: (dx, dy) in project px,
/// (roll, yaw, pitch) in degrees.
///
/// `Wobble` is camera shake: deterministic from the seed, smooth (sum of a few incommensurate sines),
/// frequency in Hz. `Plane3d` is static — its yaw/pitch/roll go straight to the placement, which the
/// CPU compositor already renders as a homography. Distance / field of view / offset Z only exist in
/// the GPU path; the CPU fallback ignores them.
pub fn wobble(effect: &Effect, t: f64) -> (f64, f64, f64, f64, f64) {
    if effect.kind == EffectKind::Plane3d {
        return (0.0, 0.0, effect.at(2, t), effect.at(0, t), effect.at(1, t));
    }
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

/// `BlobTrack` on the CPU: the centroid of every pixel within `Tolerance` of the target colour, as
/// (cx, cy) in 0..1 layer coordinates plus the matched area as a fraction of the frame. `None` when
/// nothing matches. `params` is the effect's parameter list (`EffectKind::BlobTrack.params()` order).
///
/// ponytail: centroid of *all* matching pixels, not the largest connected component — upgrade to a
/// union-find labelling if a second blob of the same colour ever needs to be ignored.
pub fn track(frame: &Frame, params: &[f64]) -> Option<(f64, f64, f64)> {
    if frame.is_empty() || params.len() < 4 {
        return None;
    }
    let target = [params[0] as f32 / 255.0, params[1] as f32 / 255.0, params[2] as f32 / 255.0];
    let tol = (params[3].clamp(0.0, 1.0) as f32) * 1.2;
    let tol2 = tol * tol;
    let (w, h) = (frame.width as usize, frame.height as usize);
    let (mut sx, mut sy, mut n) = (0f64, 0f64, 0usize);
    for y in 0..h {
        let row = y * frame.stride();
        for x in 0..w {
            let p = &frame.rgba[row + x * 4..row + x * 4 + 4];
            if p[3] < 8 {
                continue;
            }
            let mut d2 = 0.0f32;
            for c in 0..3 {
                let d = p[c] as f32 / 255.0 - target[c];
                d2 += d * d;
            }
            if d2 <= tol2 {
                sx += x as f64 + 0.5;
                sy += y as f64 + 0.5;
                n += 1;
            }
        }
    }
    if n == 0 {
        return None;
    }
    let nf = n as f64;
    Some((sx / nf / w as f64, sy / nf / h as f64, nf / (w * h) as f64))
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

/// Build a 256-entry LUT from a 0..1 -> 0..1 function.
fn lut_from(f: impl Fn(f64) -> f64) -> [u8; 256] {
    std::array::from_fn(|v| (f(v as f64 / 255.0).clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
}

fn apply_lut(img: &mut Frame, lut: &[u8; 256]) {
    for px in img.rgba.chunks_exact_mut(4) {
        px[0] = lut[px[0] as usize];
        px[1] = lut[px[1] as usize];
        px[2] = lut[px[2] as usize];
    }
}

/// Posterise to black/white at `level` with a `softness`-wide ramp, on luma or per channel.
fn threshold(img: &mut Frame, level: f32, softness: f32, per_channel: bool) {
    let sm = softness.max(1e-4) * 0.5;
    let lut: [u8; 256] = lut_from(|x| smoothstep((x as f32 - (level - sm)) / (2.0 * sm)) as f64);
    if per_channel {
        apply_lut(img, &lut);
        return;
    }
    for px in img.rgba.chunks_exact_mut(4) {
        let l = ((px[0] as u32 * 54 + px[1] as u32 * 183 + px[2] as u32 * 19) >> 8).min(255) as usize;
        let v = lut[l];
        px[0] = v;
        px[1] = v;
        px[2] = v;
    }
}

/// Rec.601-free HSL round trip (matches the GPU body): hue in turns, s/l in 0..1.
fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let l = (mx + mn) * 0.5;
    let d = mx - mn;
    if d <= 1e-5 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs()).max(1e-5);
    let h = if mx == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let (s, l) = (s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
    if s <= 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let c = |t: f32| {
        let u = t.rem_euclid(1.0);
        if u < 1.0 / 6.0 {
            p + (q - p) * 6.0 * u
        } else if u < 0.5 {
            q
        } else if u < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - u) * 6.0
        } else {
            p
        }
    };
    (c(h + 1.0 / 3.0), c(h), c(h - 1.0 / 3.0))
}

/// Rotate hue by `deg`, scale saturation, offset lightness.
fn hue_shift(img: &mut Frame, deg: f32, sat: f32, light: f32) {
    let turn = deg / 360.0;
    for px in img.rgba.chunks_exact_mut(4) {
        let (h, s, l) = rgb_to_hsl(px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0);
        let (r, g, b) = hsl_to_rgb(h + turn, (s * sat.max(0.0)).clamp(0.0, 1.0), l + light);
        px[0] = (r.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        px[1] = (g.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        px[2] = (b.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    }
}

/// Remap [in_black, in_white] onto [out_black, out_white] through `gamma` (v^(1/g), like Color).
fn levels(img: &mut Frame, inb: f64, inw: f64, gamma: f64, outb: f64, outw: f64) {
    let span = (inw - inb).max(1e-4);
    let g = gamma.max(0.01);
    let lut = lut_from(|x| outb + ((x - inb) / span).clamp(0.0, 1.0).powf(1.0 / g) * (outw - outb));
    apply_lut(img, &lut);
}

/// Monotone cubic (Fritsch–Carlson) through (0,0) (0.25,a) (0.5,b) (0.75,c) (1,1) — the exact curve
/// `shaders::CURVES` evaluates on the GPU, so preview and export agree.
pub fn curve_at(x: f64, a: f64, b: f64, c: f64) -> f64 {
    let y = [0.0, a, b, c, 1.0];
    let mut d = [0.0f64; 4];
    for i in 0..4 {
        d[i] = (y[i + 1] - y[i]) * 4.0;
    }
    let mut m = [0.0f64; 5];
    m[0] = d[0];
    m[4] = d[3];
    for i in 1..4 {
        m[i] = if d[i - 1] * d[i] <= 0.0 { 0.0 } else { (d[i - 1] + d[i]) * 0.5 };
    }
    for i in 0..4 {
        if d[i].abs() < 1e-6 {
            m[i] = 0.0;
            m[i + 1] = 0.0;
        } else {
            let (ai, bi) = (m[i] / d[i], m[i + 1] / d[i]);
            let s = ai * ai + bi * bi;
            if s > 9.0 {
                let k = 3.0 / s.sqrt();
                m[i] = k * ai * d[i];
                m[i + 1] = k * bi * d[i];
            }
        }
    }
    let xc = x.clamp(0.0, 1.0);
    let i = ((xc * 4.0).floor() as usize).min(3);
    let t = xc * 4.0 - i as f64;
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * y[i]
        + (t3 - 2.0 * t2 + t) * 0.25 * m[i]
        + (-2.0 * t3 + 3.0 * t2) * y[i + 1]
        + (t3 - t2) * 0.25 * m[i + 1]
}

/// Per-channel curves then the master curve (12 knots: master, R, G, B).
fn curves(img: &mut Frame, k: &[f64; 12]) {
    let master = lut_from(|x| curve_at(x, k[0], k[1], k[2]));
    let chans: [[u8; 256]; 3] = [
        lut_from(|x| curve_at(x, k[3], k[4], k[5])),
        lut_from(|x| curve_at(x, k[6], k[7], k[8])),
        lut_from(|x| curve_at(x, k[9], k[10], k[11])),
    ];
    // fold the master into each channel LUT so it stays one lookup per pixel
    let fold: [[u8; 256]; 3] = std::array::from_fn(|c| std::array::from_fn(|v| master[chans[c][v] as usize]));
    for px in img.rgba.chunks_exact_mut(4) {
        px[0] = fold[0][px[0] as usize];
        px[1] = fold[1][px[1] as usize];
        px[2] = fold[2][px[2] as usize];
    }
}

fn to_ycc(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    (0.299 * r + 0.587 * g + 0.114 * b, -0.168736 * r - 0.331264 * g + 0.5 * b, 0.5 * r - 0.418688 * g - 0.081312 * b)
}

/// Key out colours near `key` (0..255) by their Cb/Cr distance, with spill removal. No edge shrink —
/// that needs neighbourhood taps; `shaders::CHROMA_KEY` does it on the GPU.
fn chroma_key(img: &mut Frame, key: [f32; 3], similarity: f32, smoothness: f32, show_mask: bool, spill: f32) {
    let (_, kb, kr) = to_ycc(key[0] / 255.0, key[1] / 255.0, key[2] / 255.0);
    let klen = (kb * kb + kr * kr).sqrt();
    let sim = similarity.clamp(0.0, 1.0) * 0.5;
    let width = (smoothness.clamp(0.0, 1.0) * 0.5).max(0.0005);
    let spill = spill.clamp(0.0, 1.0);
    for px in img.rgba.chunks_exact_mut(4) {
        let (r, g, b) = (px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0);
        let (y, cb, cr) = to_ycc(r, g, b);
        let a = smoothstep((((cb - kb).powi(2) + (cr - kr).powi(2)).sqrt() - sim) / width);
        if show_mask {
            let v = (a * 255.0 + 0.5) as u8;
            px[0] = v;
            px[1] = v;
            px[2] = v;
            px[3] = 255;
            continue;
        }
        if spill > 0.0 && klen > 1e-3 {
            let proj = (cb * kb + cr * kr) / klen;
            if proj > 0.0 {
                let (nb, nr) = (cb - kb / klen * proj * spill, cr - kr / klen * proj * spill);
                let rgb = [y + 1.402 * nr, y - 0.344136 * nb - 0.714136 * nr, y + 1.772 * nb];
                for c in 0..3 {
                    px[c] = (rgb[c].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
        }
        px[3] = (px[3] as f32 * a) as u8;
    }
}

/// Seven-segment masks for 0..9 (bit 0 = top, then clockwise from top-right, bit 6 = middle).
const SEVEN_SEG: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F];
/// Segment geometry in digit-cell units (cx, cy, half-w, half-h) with `T` the stroke thickness.
const SEG_T: f32 = 0.11;
const SEGMENTS: [(f32, f32, f32, f32); 7] = [
    (0.5, SEG_T, 0.5 - SEG_T, SEG_T),
    (1.0 - SEG_T, 0.25, SEG_T, 0.25 - SEG_T),
    (1.0 - SEG_T, 0.75, SEG_T, 0.25 - SEG_T),
    (0.5, 1.0 - SEG_T, 0.5 - SEG_T, SEG_T),
    (SEG_T, 0.75, SEG_T, 0.25 - SEG_T),
    (SEG_T, 0.25, SEG_T, 0.25 - SEG_T),
    (0.5, 0.5, 0.5 - SEG_T, SEG_T),
];

/// Opaque axis-aligned rectangle (canvas px, clipped).
fn fill_rect(img: &mut Frame, x0: f32, y0: f32, x1: f32, y1: f32, rgb: [u8; 3]) {
    let (w, h) = (img.width as i64, img.height as i64);
    let xs = (x0.round() as i64).clamp(0, w);
    let xe = (x1.round() as i64).clamp(0, w);
    let ys = (y0.round() as i64).clamp(0, h);
    let ye = (y1.round() as i64).clamp(0, h);
    let stride = img.stride();
    for y in ys..ye {
        let row = y as usize * stride;
        for x in xs..xe {
            let p = row + x as usize * 4;
            img.rgba[p..p + 3].copy_from_slice(&rgb);
            img.rgba[p + 3] = 255;
        }
    }
}

fn draw_digit(img: &mut Frame, x: f32, y: f32, w: f32, h: f32, digit: usize, rgb: [u8; 3]) {
    let mask = SEVEN_SEG[digit.min(9)];
    for (i, (cx, cy, hx, hy)) in SEGMENTS.iter().enumerate() {
        if mask & (1 << i) != 0 {
            fill_rect(img, x + (cx - hx) * w, y + (cy - hy) * h, x + (cx + hx) * w, y + (cy + hy) * h, rgb);
        }
    }
}

/// Blinking record dot in a corner (0 = TL, 1 = TR, 2 = BL, 3 = BR) plus an optional HH:MM:SS timecode.
#[allow(clippy::too_many_arguments)]
fn rec_dot(img: &mut Frame, t: f64, scale: f32, size: f64, hz: f64, corner: f64, timecode: bool, margin: f64) {
    let sz = (size.max(2.0) * scale as f64) as f32;
    let mg = (margin.max(0.0) * scale as f64) as f32;
    let corner = corner.clamp(0.0, 3.0).round() as u8;
    let (right, bottom) = (corner == 1 || corner == 3, corner == 2 || corner == 3);
    let (w, h) = (img.width as f32, img.height as f32);
    let ax = if right { w - mg - sz * 0.5 } else { mg + sz * 0.5 };
    let ay = if bottom { h - mg - sz * 0.5 } else { mg + sz * 0.5 };
    let on = hz <= 0.0 || (t * hz).rem_euclid(1.0) < 0.5;
    if on {
        let r = sz * 0.5;
        let stride = img.stride();
        for y in ((ay - r).max(0.0) as usize)..((ay + r).ceil().min(h) as usize) {
            let row = y * stride;
            for x in ((ax - r).max(0.0) as usize)..((ax + r).ceil().min(w) as usize) {
                let (dx, dy) = (x as f32 + 0.5 - ax, y as f32 + 0.5 - ay);
                let d = (dx * dx + dy * dy).sqrt();
                let a = (r + 0.5 - d).clamp(0.0, 1.0);
                if a <= 0.0 {
                    continue;
                }
                let p = row + x * 4;
                for (c, v) in [230u8, 30, 30].into_iter().enumerate() {
                    img.rgba[p + c] = (img.rgba[p + c] as f32 + (v as f32 - img.rgba[p + c] as f32) * a) as u8;
                }
                img.rgba[p + 3] = img.rgba[p + 3].max((a * 255.0) as u8);
            }
        }
    }
    if !timecode {
        return;
    }
    let (dh, dw, gap) = (sz, sz * 0.55, sz * 0.12);
    let (cw, colw) = (dw + gap, sz * 0.3 + gap);
    let total = 6.0 * cw + 2.0 * colw;
    let mut x = if right { ax - sz * 0.5 - gap * 2.0 - total } else { ax + sz * 0.5 + gap * 2.0 };
    let y0 = ay - dh * 0.5;
    let secs = t.max(0.0) as u64;
    let digits = [
        (secs / 3600 % 100) / 10,
        (secs / 3600 % 100) % 10,
        (secs / 60 % 60) / 10,
        (secs / 60 % 60) % 10,
        (secs % 60) / 10,
        (secs % 60) % 10,
    ];
    for (i, d) in digits.into_iter().enumerate() {
        draw_digit(img, x, y0, dw, dh, d as usize, [235, 235, 235]);
        x += cw;
        if i == 1 || i == 3 {
            let cx = x + colw * 0.5;
            let r = sz * 0.07;
            fill_rect(img, cx - r, y0 + dh * 0.33 - r, cx + r, y0 + dh * 0.33 + r, [235, 235, 235]);
            fill_rect(img, cx - r, y0 + dh * 0.7 - r, cx + r, y0 + dh * 0.7 + r, [235, 235, 235]);
            x += colw;
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
        // a small radius at preview scale still blurs (must not round down to 0 px — preview == export)
        for s in [0.05f32, 0.36, 0.5] {
            let mut small = noisy(16, 16);
            let v0 = variance(&small);
            apply(&eff(K::Blur, &[1.0]), 0.0, s, &mut small, &mut scratch);
            assert!(variance(&small) < v0, "scale {s} did not blur");
        }
    }

    #[test]
    fn pixelate_makes_blocks_uniform() {
        let mut img = noisy(8, 8);
        let mut scratch = Frame::default();
        // at preview scale a requested block keeps at least 2 px (never silently a no-op)…
        let mut small = noisy(8, 8);
        let before = small.rgba.clone();
        apply(&eff(K::Pixelate, &[4.0]), 0.0, 0.1, &mut small, &mut scratch);
        assert_ne!(small.rgba, before, "block rounded down to a no-op");
        // …while a block of 1 stays a no-op at full scale
        let mut one = noisy(8, 8);
        let before = one.rgba.clone();
        apply(&eff(K::Pixelate, &[1.0]), 0.0, 1.0, &mut one, &mut scratch);
        assert_eq!(one.rgba, before);
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
        assert!(EffectKind::Wobble.is_geometric());
        assert!(EffectKind::Plane3d.is_geometric());
        assert!(!EffectKind::Blur.is_geometric());
        // every kind `apply` no-ops is flagged gpu_only, and no other kind is
        assert!(gpu_only(EffectKind::Vhs) && gpu_only(EffectKind::Shader));
        assert!(!gpu_only(EffectKind::Blur) && !gpu_only(EffectKind::Wobble));
    }

    /// A frame of one flat colour.
    fn flat(w: u32, h: u32, rgba: [u8; 4]) -> Frame {
        let mut f = Frame::new(w, h);
        f.fill(rgba);
        f
    }

    fn run(kind: K, params: &[f64], img: &mut Frame) {
        apply(&eff(kind, params), 0.0, 1.0, img, &mut Frame::default());
    }

    #[test]
    fn threshold_splits_at_the_level() {
        // per channel, no softness: below the level -> 0, above -> 255
        let mut img = Frame::new(2, 1);
        img.rgba.copy_from_slice(&[100, 100, 100, 255, 155, 155, 155, 255]);
        run(K::Threshold, &[0.5, 0.0, 1.0], &mut img);
        assert_eq!(&img.rgba[..3], &[0, 0, 0]);
        assert_eq!(&img.rgba[4..7], &[255, 255, 255]);
        // the level moves the split
        let mut img = flat(2, 1, [100, 100, 100, 255]);
        run(K::Threshold, &[0.3, 0.0, 1.0], &mut img);
        assert_eq!(&img.rgba[..3], &[255, 255, 255]);
        // luma mode greys the output out of a colour
        let mut img = flat(1, 1, [255, 0, 0, 255]);
        run(K::Threshold, &[0.1, 0.0, 0.0], &mut img);
        assert_eq!(&img.rgba[..3], &[255, 255, 255]);
        // softness leaves mid values in between
        let mut img = flat(1, 1, [128, 128, 128, 255]);
        run(K::Threshold, &[0.5, 0.5, 1.0], &mut img);
        assert!((100..=160).contains(&img.rgba[0]), "{}", img.rgba[0]);
    }

    #[test]
    fn hue_shift_turns_red_into_green() {
        let mut img = flat(1, 1, [255, 0, 0, 255]);
        run(K::HueShift, &[120.0, 1.0, 0.0], &mut img);
        assert_eq!(&img.rgba[..3], &[0, 255, 0]);
        // and another 120 degrees is blue
        run(K::HueShift, &[120.0, 1.0, 0.0], &mut img);
        assert_eq!(&img.rgba[..3], &[0, 0, 255]);
        // saturation 0 -> grey, lightness lifts it
        let mut img = flat(1, 1, [255, 0, 0, 255]);
        run(K::HueShift, &[0.0, 0.0, 0.0], &mut img);
        assert_eq!(&img.rgba[..3], &[128, 128, 128]);
        run(K::HueShift, &[0.0, 1.0, 0.5], &mut img);
        assert!(img.rgba[0] > 200, "{}", img.rgba[0]);
    }

    #[test]
    fn levels_clamp_and_remap() {
        let mut img = Frame::new(3, 1);
        img.rgba.copy_from_slice(&[0, 0, 0, 255, 100, 100, 100, 255, 255, 255, 255, 255]);
        run(K::Levels, &[0.5, 1.0, 1.0, 0.0, 1.0], &mut img);
        assert_eq!(img.rgba[0], 0, "below in-black clamps to 0");
        assert_eq!(img.rgba[4], 0, "0.39 is below in-black 0.5");
        assert_eq!(img.rgba[8], 255, "in-white stays white");
        // output range compresses
        let mut img = flat(2, 1, [0, 255, 0, 255]);
        run(K::Levels, &[0.0, 1.0, 1.0, 0.2, 0.8], &mut img);
        assert_eq!(img.rgba[0], 51);
        assert_eq!(img.rgba[1], 204);
        // gamma > 1 lifts the mid tones
        let mut img = flat(1, 1, [128, 128, 128, 255]);
        run(K::Levels, &[0.0, 1.0, 2.0, 0.0, 1.0], &mut img);
        assert!(img.rgba[0] > 150, "{}", img.rgba[0]);
    }

    #[test]
    fn curves_are_identity_at_the_default_knots() {
        for x in [0.0, 0.25, 0.5, 0.75, 1.0, 0.1, 0.9] {
            assert!((curve_at(x, 0.25, 0.5, 0.75) - x).abs() < 1e-9, "x = {x}");
        }
        let base = noisy(8, 8);
        let mut img = base.clone();
        run(K::Curves, &[0.25, 0.5, 0.75, 0.25, 0.5, 0.75, 0.25, 0.5, 0.75, 0.25, 0.5, 0.75], &mut img);
        assert_eq!(img.rgba, base.rgba, "default knots must be a no-op");
        // a lifted master curve brightens without inverting the order
        let mut img = flat(1, 1, [128, 128, 128, 255]);
        run(K::Curves, &[0.4, 0.7, 0.9, 0.25, 0.5, 0.75, 0.25, 0.5, 0.75, 0.25, 0.5, 0.75], &mut img);
        assert!(img.rgba[0] > 160, "{}", img.rgba[0]);
        // per-channel: only red is pushed down
        let mut img = flat(1, 1, [200, 200, 200, 255]);
        run(K::Curves, &[0.25, 0.5, 0.75, 0.05, 0.1, 0.2, 0.25, 0.5, 0.75, 0.25, 0.5, 0.75], &mut img);
        assert!(img.rgba[0] < 100, "{}", img.rgba[0]);
        assert_eq!(img.rgba[1], 200);
        // knots out of order: the spline must still stay inside the knot range, never ring past it
        for i in 0..=100 {
            let v = curve_at(i as f64 / 100.0, 0.9, 0.1, 0.95);
            assert!((-1e-9..=1.0 + 1e-9).contains(&v), "out of range at {i}: {v}");
        }
        // sane knots stay monotone
        let mut prev = f64::MIN;
        for i in 0..=100 {
            let v = curve_at(i as f64 / 100.0, 0.1, 0.4, 0.8);
            assert!(v >= prev - 1e-9, "not monotone at {i}: {v} < {prev}");
            prev = v;
        }
    }

    #[test]
    fn chroma_key_removes_the_key_colour_only() {
        let mut img = Frame::new(3, 1);
        // green (the key), red, white
        img.rgba.copy_from_slice(&[0, 255, 0, 255, 255, 0, 0, 255, 255, 255, 255, 255]);
        run(K::ChromaKey, &[0.0, 255.0, 0.0, 0.4, 0.1, 0.0, 0.5, 0.0], &mut img);
        assert_eq!(img.rgba[3], 0, "the key colour is gone");
        assert_eq!(img.rgba[7], 255, "red survives");
        assert_eq!(&img.rgba[4..7], &[255, 0, 0], "red is not despilled");
        assert_eq!(img.rgba[11], 255, "white survives");
        // show mask paints the matte instead
        let mut img = Frame::new(2, 1);
        img.rgba.copy_from_slice(&[0, 255, 0, 255, 255, 0, 0, 255]);
        run(K::ChromaKey, &[0.0, 255.0, 0.0, 0.4, 0.1, 1.0, 0.5, 0.0], &mut img);
        assert_eq!(&img.rgba[..4], &[0, 0, 0, 255]);
        assert_eq!(&img.rgba[4..8], &[255, 255, 255, 255]);
        // spill removal pulls green out of a greenish skin tone but keeps it opaque
        let mut img = flat(1, 1, [200, 190, 150, 255]);
        run(K::ChromaKey, &[0.0, 255.0, 0.0, 0.4, 0.1, 0.0, 1.0, 0.0], &mut img);
        assert_eq!(img.rgba[3], 255);
    }

    #[test]
    fn blob_tracking_finds_a_red_square() {
        let mut f = flat(20, 20, [0, 0, 0, 255]);
        for y in 8..12u32 {
            for x in 8..12u32 {
                let i = ((y * 20 + x) * 4) as usize;
                f.rgba[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        let p = K::BlobTrack.params().iter().map(|s| s.default).collect::<Vec<_>>();
        let (cx, cy, area) = track(&f, &p).expect("found");
        assert!((cx - 0.5).abs() < 1e-9 && (cy - 0.5).abs() < 1e-9, "{cx} {cy}");
        assert!((area - 16.0 / 400.0).abs() < 1e-9, "{area}");
        // off-centre square moves the centroid
        let mut f2 = flat(20, 20, [0, 0, 0, 255]);
        for y in 0..4u32 {
            for x in 0..4u32 {
                let i = ((y * 20 + x) * 4) as usize;
                f2.rgba[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        let (cx2, cy2, _) = track(&f2, &p).expect("found");
        assert!(cx2 < 0.2 && cy2 < 0.2, "{cx2} {cy2}");
        // nothing matching, and a degenerate frame
        assert!(track(&flat(4, 4, [0, 0, 255, 255]), &p).is_none());
        assert!(track(&Frame::default(), &p).is_none());
        assert!(track(&f, &[]).is_none());
    }

    #[test]
    fn plane3d_drives_the_placement_on_the_cpu() {
        let e = eff(K::Plane3d, &[30.0, -10.0, 5.0, 2.0, 45.0, 0.0]);
        assert_eq!(wobble(&e, 0.0), (0.0, 0.0, 5.0, 30.0, -10.0));
    }

    #[test]
    fn rec_dot_paints_a_dot_and_a_timecode() {
        let mut img = flat(96, 40, [0, 0, 0, 255]);
        // size 12, always on, top-left, timecode, margin 4
        apply(&eff(K::RecDot, &[12.0, 0.0, 0.0, 1.0, 4.0]), 0.0, 1.0, &mut img, &mut Frame::default());
        let at = |img: &Frame, x: u32, y: u32| {
            let i = ((y * 96 + x) * 4) as usize;
            [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2]]
        };
        let dot = at(&img, 10, 10);
        assert!(dot[0] > 150 && dot[1] < 80, "dot at the top-left: {dot:?}");
        let ink = img.rgba.chunks_exact(4).filter(|p| p[0] > 200 && p[1] > 200 && p[2] > 200).count();
        assert!(ink > 40, "timecode digits drawn: {ink}");
        // blink: off during the second half of the cycle
        let mut img = flat(96, 40, [0, 0, 0, 255]);
        apply(&eff(K::RecDot, &[12.0, 1.0, 0.0, 0.0, 4.0]), 0.6, 1.0, &mut img, &mut Frame::default());
        assert_eq!(at(&img, 10, 10), [0, 0, 0], "dot is blinked off");
        // a different corner moves it
        let mut img = flat(96, 40, [0, 0, 0, 255]);
        apply(&eff(K::RecDot, &[12.0, 0.0, 3.0, 0.0, 4.0]), 0.0, 1.0, &mut img, &mut Frame::default());
        assert_eq!(at(&img, 10, 10), [0, 0, 0]);
        assert!(at(&img, 88, 30)[0] > 150, "dot at the bottom-right: {:?}", at(&img, 88, 30));
    }

    #[test]
    fn gpu_only_kinds_are_still_no_ops() {
        for k in [K::Vhs, K::MotionBlur, K::EdgeGlow, K::JpegCompress, K::BlobTrack, K::Shader] {
            let mut img = noisy(8, 8);
            let before = img.rgba.clone();
            let mut e = Effect::new(k);
            for p in e.params.iter_mut() {
                p.value = 1.0;
            }
            apply(&e, 0.0, 1.0, &mut img, &mut Frame::default());
            assert_eq!(img.rgba, before, "{k:?} must leave the CPU path alone");
        }
    }
}
