//! Blend modes and the per-pixel composite of a straight-alpha RGBA layer onto an opaque canvas.

use crate::media::Frame;
use crate::model::BlendMode;

/// Blend one channel: `s` = layer (source), `d` = canvas (destination), both 0..1. Returns 0..1.
/// The non-separable modes (Saturate and the four HSL modes) need the whole pixel: use `blend_pixel`;
/// here they return the canvas value unchanged.
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
        Exclusion => s + d - 2.0 * s * d,
        Saturate | Hue | Saturation | Color | Luminosity => d,
    }
}

/// Whether the mode needs the whole pixel (`blend_pixel`), not one channel at a time.
#[inline]
pub fn is_non_separable(mode: BlendMode) -> bool {
    matches!(mode, BlendMode::Saturate | BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity)
}

#[inline]
fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

/// W3C `ClipColor`: pull an out-of-gamut colour back into 0..1 keeping its luminosity.
fn clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut o = c;
    if n < 0.0 && l - n > 1e-6 {
        for v in &mut o {
            *v = l + (*v - l) * l / (l - n);
        }
    }
    if x > 1.0 && x - l > 1e-6 {
        for v in &mut o {
            *v = l + (*v - l) * (1.0 - l) / (x - l);
        }
    }
    o.map(|v| v.clamp(0.0, 1.0))
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d])
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

/// W3C `SetSat`: min -> 0, max -> `s`, the middle channel scaled in between.
fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let n = c[0].min(c[1]).min(c[2]);
    let r = sat(c);
    if r <= 1e-6 {
        return [0.0; 3];
    }
    c.map(|v| (v - n) * s / r)
}

/// Blend one pixel (`s` = layer, `d` = canvas, 0..1 RGB). Handles every mode, separable or not.
pub fn blend_pixel(mode: BlendMode, s: [f32; 3], d: [f32; 3]) -> [f32; 3] {
    use BlendMode::*;
    match mode {
        Saturate => {
            // as much of the layer as fits under white in every channel
            let mut k = 1.0f32;
            for i in 0..3 {
                if s[i] > 1e-6 {
                    k = k.min(((1.0 - d[i]) / s[i]).max(0.0));
                }
            }
            [0, 1, 2].map(|i| (d[i] + s[i] * k).clamp(0.0, 1.0))
        }
        Hue => set_lum(set_sat(s, sat(d)), lum(d)),
        Saturation => set_lum(set_sat(d, sat(s)), lum(d)),
        Color => set_lum(s, lum(d)),
        Luminosity => set_lum(d, lum(s)),
        _ => [0, 1, 2].map(|i| blend_channel(mode, s[i], d[i])),
    }
}

/// x / 255 rounded, for x <= 255*255.
#[inline]
fn div255(x: u32) -> u32 {
    (x + 128 + ((x + 128) >> 8)) >> 8
}

/// Composite one row of straight-alpha RGBA pixels (`src`) onto one row of opaque canvas pixels (`dst`).
/// Both slices are `4 * n` bytes; `opacity` is 0..1. Canvas alpha stays 255.
pub fn composite_row(dst: &mut [u8], src: &[u8], mode: BlendMode, opacity: f32) {
    let op = (opacity.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    if op == 0 {
        return;
    }
    let px = dst.chunks_exact_mut(4).zip(src.chunks_exact(4));
    if mode == BlendMode::Normal {
        for (d, s) in px {
            let a = div255(s[3] as u32 * op);
            if a == 0 {
                continue;
            }
            if a == 255 {
                d[..3].copy_from_slice(&s[..3]);
            } else {
                let ia = 255 - a;
                d[0] = div255(d[0] as u32 * ia + s[0] as u32 * a) as u8;
                d[1] = div255(d[1] as u32 * ia + s[1] as u32 * a) as u8;
                d[2] = div255(d[2] as u32 * ia + s[2] as u32 * a) as u8;
            }
            d[3] = 255;
        }
    } else {
        let opf = op as f32 / 255.0;
        let whole = is_non_separable(mode);
        for (d, s) in px {
            if s[3] == 0 {
                continue;
            }
            let a = s[3] as f32 / 255.0 * opf;
            if whole {
                let sv = [s[0], s[1], s[2]].map(|v| v as f32 / 255.0);
                let dv = [d[0], d[1], d[2]].map(|v| v as f32 / 255.0);
                let b = blend_pixel(mode, sv, dv);
                for i in 0..3 {
                    d[i] = ((dv[i] + (b[i] - dv[i]) * a) * 255.0 + 0.5) as u8;
                }
            } else {
                for i in 0..3 {
                    let sv = s[i] as f32 / 255.0;
                    let dv = d[i] as f32 / 255.0;
                    let b = blend_channel(mode, sv, dv);
                    d[i] = ((dv + (b - dv) * a) * 255.0 + 0.5) as u8;
                }
            }
            d[3] = 255;
        }
    }
}

/// Composite `layer` (straight-alpha RGBA, same size as `dst`) onto `dst` (opaque canvas) with the
/// given blend mode and global opacity, restricted to the pixel rect [x0, x1) × [y0, y1) (clamped to
/// the canvas). Pixels with alpha 0 are skipped.
pub fn composite_rect(
    dst: &mut Frame,
    layer: &Frame,
    mode: BlendMode,
    opacity: f32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
) {
    if dst.width != layer.width || dst.height != layer.height {
        return;
    }
    let stride = dst.stride();
    let (x0, x1) = (x0.min(dst.width) as usize * 4, x1.min(dst.width) as usize * 4);
    if x0 >= x1 {
        return;
    }
    for y in y0..y1.min(dst.height) {
        let row = y as usize * stride;
        composite_row(&mut dst.rgba[row + x0..row + x1], &layer.rgba[row + x0..row + x1], mode, opacity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_formulas() {
        assert!((blend_channel(BlendMode::Multiply, 0.5, 0.5) - 0.25).abs() < 1e-6);
        assert!((blend_channel(BlendMode::Screen, 0.5, 0.5) - 0.75).abs() < 1e-6);
        assert!((blend_channel(BlendMode::Add, 0.7, 0.7) - 1.0).abs() < 1e-6);
        assert!((blend_channel(BlendMode::Difference, 0.2, 0.7) - 0.5).abs() < 1e-6);
        assert!((blend_channel(BlendMode::Normal, 0.3, 0.9) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn composite_half_alpha_and_modes() {
        let mut dst = Frame::new(2, 1);
        dst.fill([100, 100, 100, 255]);
        let mut layer = Frame::new(2, 1);
        layer.rgba.copy_from_slice(&[200, 200, 200, 128, 50, 50, 50, 0]);
        composite_rect(&mut dst, &layer, BlendMode::Normal, 1.0, 0, 0, 2, 1);
        // 100 + (200-100)*128/255 = 150.2
        assert_eq!(&dst.rgba[..4], &[150, 150, 150, 255]);
        // alpha 0 skipped
        assert_eq!(&dst.rgba[4..], &[100, 100, 100, 255]);

        // opacity 0.5 on an opaque layer
        dst.fill([100, 100, 100, 255]);
        layer.rgba[3] = 255;
        composite_rect(&mut dst, &layer, BlendMode::Normal, 0.5, 0, 0, 2, 1);
        assert_eq!(dst.rgba[0], 150);

        // multiply: 0.5*0.5 = 0.25 → 64
        dst.fill([128, 128, 128, 255]);
        layer.rgba[..4].copy_from_slice(&[128, 128, 128, 255]);
        composite_rect(&mut dst, &layer, BlendMode::Multiply, 1.0, 0, 0, 2, 1);
        assert!((dst.rgba[0] as i32 - 64).abs() <= 1, "{}", dst.rgba[0]);
        // screen: 1-(0.5*0.5) = 0.75 → 191
        dst.fill([128, 128, 128, 255]);
        composite_rect(&mut dst, &layer, BlendMode::Screen, 1.0, 0, 0, 2, 1);
        assert!((dst.rgba[0] as i32 - 191).abs() <= 1, "{}", dst.rgba[0]);
        assert_eq!(dst.rgba[3], 255);
    }

    #[test]
    fn rect_limited() {
        let mut dst = Frame::new(4, 4);
        dst.fill([0, 0, 0, 255]);
        let mut layer = Frame::new(4, 4);
        layer.fill([255, 0, 0, 255]);
        composite_rect(&mut dst, &layer, BlendMode::Normal, 1.0, 1, 1, 3, 3);
        let px = |x: usize, y: usize| dst.rgba[(y * 4 + x) * 4];
        assert_eq!(px(0, 0), 0);
        assert_eq!(px(1, 1), 255);
        assert_eq!(px(2, 2), 255);
        assert_eq!(px(3, 3), 0);
    }

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3)
    }

    #[test]
    fn exclusion_and_saturate() {
        assert!((blend_channel(BlendMode::Exclusion, 0.5, 0.5) - 0.5).abs() < 1e-6);
        assert!((blend_channel(BlendMode::Exclusion, 1.0, 0.25) - 0.75).abs() < 1e-6);
        assert!((blend_channel(BlendMode::Exclusion, 0.0, 0.3) - 0.3).abs() < 1e-6);
        // saturate never clips: the red channel has 0.2 of room, so only 0.2/0.5 of the layer goes in
        let o = blend_pixel(BlendMode::Saturate, [0.5, 0.5, 0.5], [0.8, 0.2, 0.2]);
        assert!(near(o, [1.0, 0.4, 0.4]), "{o:?}");
        // a dark canvas takes the whole layer, like Add
        assert!(near(blend_pixel(BlendMode::Saturate, [0.3, 0.2, 0.1], [0.1, 0.1, 0.1]), [0.4, 0.3, 0.2]));
        // black adds nothing
        assert!(near(blend_pixel(BlendMode::Saturate, [0.0; 3], [0.6, 0.5, 0.4]), [0.6, 0.5, 0.4]));
    }

    #[test]
    fn hsl_modes_follow_the_w3c_formulas() {
        let red = [1.0, 0.0, 0.0];
        let grey = [0.5, 0.5, 0.5];
        // Hue / Color onto grey: grey has no saturation, so Hue stays grey and Color takes red's hue
        assert!(near(blend_pixel(BlendMode::Hue, red, grey), grey));
        let c = blend_pixel(BlendMode::Color, red, grey);
        assert!((lum(c) - 0.5).abs() < 1e-3 && c[0] > c[1] && (c[1] - c[2]).abs() < 1e-3, "{c:?}");
        // Saturation from grey drains the colour, keeping luminosity
        let s = blend_pixel(BlendMode::Saturation, grey, [0.8, 0.4, 0.2]);
        assert!((s[0] - s[1]).abs() < 1e-3 && (s[1] - s[2]).abs() < 1e-3);
        assert!((lum(s) - lum([0.8, 0.4, 0.2])).abs() < 1e-3);
        // Luminosity: the canvas colour at the layer's brightness
        let l = blend_pixel(BlendMode::Luminosity, [0.2, 0.2, 0.2], [0.8, 0.4, 0.2]);
        assert!((lum(l) - 0.2).abs() < 1e-3 && l[0] > l[2], "{l:?}");
        // every result is in gamut
        for m in BlendMode::ALL {
            for o in [blend_pixel(m, red, [0.1, 0.9, 0.3]), blend_pixel(m, [0.9, 0.95, 0.1], [0.05, 0.0, 0.6])] {
                assert!(o.iter().all(|v| (0.0..=1.0).contains(v)), "{} -> {o:?}", m.name());
            }
        }
    }

    #[test]
    fn composite_uses_the_whole_pixel_for_hsl_modes() {
        let mut dst = Frame::new(1, 1);
        dst.fill([128, 128, 128, 255]);
        let mut layer = Frame::new(1, 1);
        layer.fill([255, 0, 0, 255]);
        composite_rect(&mut dst, &layer, BlendMode::Color, 1.0, 0, 0, 1, 1);
        assert!(dst.rgba[0] > dst.rgba[1] + 40, "Color must tint grey red: {:?}", dst.rgba);
        assert_eq!(dst.rgba[1], dst.rgba[2]);
    }
}
