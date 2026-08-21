//! Compositor: renders the timeline at time t into an RGBA canvas. Used by preview (small canvas)
//! and export (project-size canvas) — identical output, so preview == export.
//!
//! Layer order: video tracks bottom→top in `project.tracks` order (V1 first, then V2 drawn over it…).
//! For each active (`project.active(track)`), enabled, visual clip containing t:
//!   * Video/Image: `pool.video(asset.path).frame_at(clip.src_time(t), dw, dh, ..)` where (dw,dh) is the
//!     placement size (clamped to native size, min 1) — decoders scale for us.
//!   * Text: `text.render(style, canvas_w / project.width)` (already at canvas scale) — contain=false.
//!   * Transform the layer into canvas space per `placement()` (bilinear for rotation / non-integer
//!     scale; straight row copies for the common axis-aligned case), then `blend::composite`.
//! Background is opaque black. Out-of-view layers are skipped.

use crate::engine::blend;
use crate::engine::text::TextRasterizer;
use crate::media::{DecoderPool, Frame};
use crate::model::{BlendMode, Clip, ClipKind, Project, TrackKind};

/// Where a layer lands on a canvas of (cw, ch) pixels: centre, size and rotation (degrees, clockwise).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
    pub h: f32,
    pub rot: f32,
}

impl Placement {
    /// Axis-aligned bounding box (x0, y0, x1, y1).
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let (s, c) = self.rot.to_radians().sin_cos();
        let hw = self.w / 2.0;
        let hh = self.h / 2.0;
        let ex = (hw * c).abs() + (hh * s).abs();
        let ey = (hw * s).abs() + (hh * c).abs();
        (self.cx - ex, self.cy - ey, self.cx + ex, self.cy + ey)
    }
}

/// Placement of a clip's layer on a (cw, ch) canvas at timeline time t.
/// `native` = the layer image size. With `contain` (video/images) the image is first fitted inside the
/// canvas preserving aspect; text is already rendered at canvas scale so `contain = false`.
/// Then: scale about the centre by `clip.scale`, offset by (clip.x, clip.y) project pixels (scaled to
/// canvas), rotate by `clip.rotation`.
pub fn placement(
    project: &Project,
    clip: &Clip,
    t: f64,
    native: (u32, u32),
    cw: u32,
    ch: u32,
    contain: bool,
) -> Placement {
    let lt = clip.local(t);
    let s = cw as f32 / project.width.max(1) as f32;
    let (nw, nh) = (native.0.max(1) as f32, native.1.max(1) as f32);
    let fit = if contain {
        (cw as f32 / nw).min(ch as f32 / nh)
    } else {
        1.0
    };
    let k = clip.scale.at(lt) as f32;
    Placement {
        cx: cw as f32 / 2.0 + clip.x.at(lt) as f32 * s,
        cy: ch as f32 / 2.0 + clip.y.at(lt) as f32 * s,
        w: nw * fit * k,
        h: nh * fit * k,
        rot: clip.rotation.at(lt) as f32,
    }
}

/// Holds scratch buffers so rendering does not allocate per frame.
pub struct Compositor {
    layer: Frame,
    src: Frame,
}

impl Default for Compositor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compositor {
    pub fn new() -> Self {
        Self {
            layer: Frame::default(),
            src: Frame::default(),
        }
    }

    /// Render timeline time `t` into `out` at w×h (out is resized). Never panics on missing media:
    /// a clip whose decoder fails is simply skipped.
    pub fn render(
        &mut self,
        project: &Project,
        t: f64,
        w: u32,
        h: u32,
        pool: &mut DecoderPool,
        text: &mut TextRasterizer,
        out: &mut Frame,
    ) {
        out.resize(w, h);
        out.fill([0, 0, 0, 255]);
        out.pts = t;
        if w == 0 || h == 0 {
            return;
        }
        for (ti, track) in project.tracks.iter().enumerate() {
            if track.kind != TrackKind::Video || !project.active(ti) {
                continue;
            }
            for clip in &track.clips {
                if !clip.enabled || !clip.contains(t) {
                    continue;
                }
                let opacity = clip.opacity.at(clip.local(t)).clamp(0.0, 1.0) as f32;
                if opacity <= 0.0 {
                    continue;
                }
                match clip.kind {
                    ClipKind::Video | ClipKind::Image => {
                        let Some(asset) = project.asset(clip.asset) else {
                            continue;
                        };
                        let Some(dec) = pool.video(&asset.path) else {
                            continue;
                        };
                        let mut native = (asset.width, asset.height);
                        if native.0 == 0 || native.1 == 0 {
                            native = dec.size();
                        }
                        if native.0 == 0 || native.1 == 0 {
                            continue;
                        }
                        let p = placement(project, clip, t, native, w, h, true);
                        if !(p.w.is_finite() && p.h.is_finite()) {
                            continue;
                        }
                        let dw = (p.w.round() as u32).clamp(1, native.0);
                        let dh = (p.h.round() as u32).clamp(1, native.1);
                        if !dec.frame_at(clip.src_time(t), dw, dh, &mut self.src) {
                            continue;
                        }
                        draw_layer(out, &self.src, p, clip.blend, opacity, &mut self.layer);
                    }
                    ClipKind::Text => {
                        let Some(style) = &clip.text else { continue };
                        let img = text.render(style, w as f32 / project.width.max(1) as f32);
                        let p = placement(project, clip, t, (img.width, img.height), w, h, false);
                        draw_layer(out, &img, p, clip.blend, opacity, &mut self.layer);
                    }
                    ClipKind::Audio => {}
                }
            }
        }
    }
}

/// Draw `src` into `dst` (canvas) at `p` with blend mode & opacity, using `scratch` as the canvas-sized
/// layer buffer. Bilinear sampling; pixels outside `src` are transparent.
pub fn draw_layer(dst: &mut Frame, src: &Frame, p: Placement, mode: BlendMode, opacity: f32, scratch: &mut Frame) {
    if dst.is_empty() || src.is_empty() || !(opacity > 0.0) {
        return;
    }
    if !(p.cx.is_finite() && p.cy.is_finite() && p.w > 0.0 && p.h > 0.0 && p.w.is_finite() && p.h.is_finite()) {
        return;
    }
    let (cw, ch) = (dst.width as i64, dst.height as i64);
    let (sw, sh) = (src.width as i64, src.height as i64);

    // (a) axis-aligned, 1:1 — composite src rows straight onto the canvas at an integer offset.
    if p.rot == 0.0 && (p.w - sw as f32).abs() < 0.5 && (p.h - sh as f32).abs() < 0.5 {
        let ox = (p.cx - sw as f32 / 2.0).round() as i64;
        let oy = (p.cy - sh as f32 / 2.0).round() as i64;
        let (x0, x1) = (ox.max(0), (ox + sw).min(cw));
        let (y0, y1) = (oy.max(0), (oy + sh).min(ch));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let (ds, ss) = (dst.stride(), src.stride());
        for y in y0..y1 {
            let d = y as usize * ds;
            let s = (y - oy) as usize * ss;
            let (dx0, dx1) = (x0 as usize * 4, x1 as usize * 4);
            let (sx0, sx1) = ((x0 - ox) as usize * 4, (x1 - ox) as usize * 4);
            blend::composite_row(
                &mut dst.rgba[d + dx0..d + dx1],
                &src.rgba[s + sx0..s + sx1],
                mode,
                opacity,
            );
        }
        return;
    }

    // (b) general: inverse-map the pixels inside bounds ∩ canvas with bilinear sampling.
    let (bx0, by0, bx1, by1) = p.bounds();
    let x0 = (bx0.floor() as i64).clamp(0, cw) as u32;
    let y0 = (by0.floor() as i64).clamp(0, ch) as u32;
    let x1 = (bx1.ceil() as i64).clamp(0, cw) as u32;
    let y1 = (by1.ceil() as i64).clamp(0, ch) as u32;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    scratch.resize(dst.width, dst.height);
    let (sin, cos) = p.rot.to_radians().sin_cos();
    let (hw, hh) = (p.w / 2.0, p.h / 2.0);
    let (kx, ky) = (sw as f32 / p.w, sh as f32 / p.h);
    let (swm, shm) = ((sw - 1) as f32, (sh - 1) as f32);
    let stride = scratch.stride();
    let sstride = src.stride();
    for y in y0..y1 {
        let row = y as usize * stride;
        let out = &mut scratch.rgba[row + x0 as usize * 4..row + x1 as usize * 4];
        let dy = y as f32 + 0.5 - p.cy;
        for (i, o) in out.chunks_exact_mut(4).enumerate() {
            let dx = (x0 + i as u32) as f32 + 0.5 - p.cx;
            // canvas → layer space (undo the clockwise rotation)
            let u = dx * cos + dy * sin;
            let v = -dx * sin + dy * cos;
            // 1px feathered rectangle coverage
            let cov = ((hw - u.abs()).min(hh - v.abs()) + 0.5).clamp(0.0, 1.0);
            if cov <= 0.0 {
                o.copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            // clamped bilinear, premultiplied accumulation (straight-alpha source)
            let sx = ((u + hw) * kx - 0.5).clamp(0.0, swm);
            let sy = ((v + hh) * ky - 0.5).clamp(0.0, shm);
            let (fx, fy) = (sx.floor(), sy.floor());
            let (tx, ty) = (sx - fx, sy - fy);
            let ix0 = fx as usize;
            let iy0 = fy as usize;
            let ix1 = (ix0 + 1).min(sw as usize - 1);
            let iy1 = (iy0 + 1).min(sh as usize - 1);
            let taps = [
                (iy0 * sstride + ix0 * 4, (1.0 - tx) * (1.0 - ty)),
                (iy0 * sstride + ix1 * 4, tx * (1.0 - ty)),
                (iy1 * sstride + ix0 * 4, (1.0 - tx) * ty),
                (iy1 * sstride + ix1 * 4, tx * ty),
            ];
            let (mut r, mut g, mut b, mut a) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for (off, wgt) in taps {
                let s = &src.rgba[off..off + 4];
                let wa = wgt * s[3] as f32;
                if wa > 0.0 {
                    r += wa * s[0] as f32;
                    g += wa * s[1] as f32;
                    b += wa * s[2] as f32;
                    a += wa;
                }
            }
            if a <= 0.0 {
                o.copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            o[0] = (r / a + 0.5) as u8;
            o[1] = (g / a + 0.5) as u8;
            o[2] = (b / a + 0.5) as u8;
            o[3] = (a * cov + 0.5) as u8;
        }
    }
    blend::composite_rect(dst, scratch, mode, opacity, x0, y0, x1, y1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{Backend, VideoSource};
    use crate::model::{Asset, ClipKind, Project};

    fn px(f: &Frame, x: u32, y: u32) -> [u8; 4] {
        let i = (y * f.width + x) as usize * 4;
        [f.rgba[i], f.rgba[i + 1], f.rgba[i + 2], f.rgba[i + 3]]
    }

    fn black(w: u32, h: u32) -> Frame {
        let mut f = Frame::new(w, h);
        f.fill([0, 0, 0, 255]);
        f
    }

    #[test]
    fn draw_layer_scale_2_centred() {
        let mut dst = black(8, 8);
        let mut src = Frame::new(2, 2);
        src.fill([255, 0, 0, 255]);
        let mut scratch = Frame::default();
        let p = Placement {
            cx: 4.0,
            cy: 4.0,
            w: 4.0,
            h: 4.0,
            rot: 0.0,
        };
        draw_layer(&mut dst, &src, p, BlendMode::Normal, 1.0, &mut scratch);
        for y in 0..8 {
            for x in 0..8 {
                let inside = (2..6).contains(&x) && (2..6).contains(&y);
                let want = if inside { [255, 0, 0, 255] } else { [0, 0, 0, 255] };
                assert_eq!(px(&dst, x, y), want, "at {x},{y}");
            }
        }
    }

    #[test]
    fn draw_layer_fast_path_offset_and_clip() {
        let mut dst = black(4, 4);
        let mut src = Frame::new(2, 2);
        src.fill([0, 255, 0, 255]);
        let mut scratch = Frame::default();
        // native size at the top-left corner, 50% opacity
        let p = Placement {
            cx: 1.0,
            cy: 1.0,
            w: 2.0,
            h: 2.0,
            rot: 0.0,
        };
        draw_layer(&mut dst, &src, p, BlendMode::Normal, 0.5, &mut scratch);
        assert_eq!(px(&dst, 0, 0), [0, 128, 0, 255]);
        assert_eq!(px(&dst, 1, 1), [0, 128, 0, 255]);
        assert_eq!(px(&dst, 2, 2), [0, 0, 0, 255]);
        // partially off-canvas
        let mut dst = black(4, 4);
        let p = Placement {
            cx: 0.0,
            cy: 0.0,
            w: 2.0,
            h: 2.0,
            rot: 0.0,
        };
        draw_layer(&mut dst, &src, p, BlendMode::Normal, 1.0, &mut scratch);
        assert_eq!(px(&dst, 0, 0), [0, 255, 0, 255]);
        assert_eq!(px(&dst, 1, 0), [0, 0, 0, 255]);
        // fully off-canvas: no-op, no panic
        let p = Placement {
            cx: 100.0,
            cy: 100.0,
            w: 2.0,
            h: 2.0,
            rot: 45.0,
        };
        draw_layer(&mut dst, &src, p, BlendMode::Normal, 1.0, &mut scratch);
    }

    #[test]
    fn draw_layer_rotated_90() {
        // 4x2 source, left half red, right half blue; rotated 90° cw → top half red, bottom half blue.
        let mut dst = black(8, 8);
        let mut src = Frame::new(4, 2);
        for y in 0..2 {
            for x in 0..4 {
                let i = (y * 4 + x) * 4;
                let c = if x < 2 { [255, 0, 0, 255] } else { [0, 0, 255, 255] };
                src.rgba[i..i + 4].copy_from_slice(&c);
            }
        }
        let mut scratch = Frame::default();
        let p = Placement {
            cx: 4.0,
            cy: 4.0,
            w: 4.0,
            h: 2.0,
            rot: 90.0,
        };
        draw_layer(&mut dst, &src, p, BlendMode::Normal, 1.0, &mut scratch);
        assert_eq!(px(&dst, 3, 2), [255, 0, 0, 255]);
        assert_eq!(px(&dst, 4, 2), [255, 0, 0, 255]);
        assert_eq!(px(&dst, 3, 5), [0, 0, 255, 255]);
        assert_eq!(px(&dst, 4, 5), [0, 0, 255, 255]);
        assert_eq!(px(&dst, 0, 0), [0, 0, 0, 255]);
        assert_eq!(px(&dst, 1, 4), [0, 0, 0, 255]);
    }

    struct FakeVideo([u8; 4]);
    impl VideoSource for FakeVideo {
        fn size(&self) -> (u32, u32) {
            (320, 240)
        }
        fn duration(&self) -> f64 {
            10.0
        }
        fn fps(&self) -> f64 {
            30.0
        }
        fn frame_at(&mut self, t: f64, w: u32, h: u32, out: &mut Frame) -> bool {
            if t >= 10.0 {
                return false;
            }
            out.resize(w, h);
            out.fill(self.0);
            out.pts = t;
            true
        }
    }

    fn fake_asset() -> Asset {
        Asset {
            id: 0,
            path: "Z:\\nope\\fake.mp4".into(),
            kind: ClipKind::Video,
            duration: 10.0,
            width: 320,
            height: 240,
            fps: 30.0,
            audio_streams: Vec::new(),
            codec: "h264".into(),
        }
    }

    #[test]
    fn compositor_renders_fake_video() {
        let project = Project::from_media(fake_asset());
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_video(&project.assets[0].path, Box::new(FakeVideo([255, 0, 0, 255])));
        let mut comp = Compositor::new();
        let mut text = TextRasterizer::new();
        let mut out = Frame::default();
        comp.render(&project, 1.0, 64, 48, &mut pool, &mut text, &mut out);
        assert_eq!((out.width, out.height), (64, 48));
        assert_eq!(px(&out, 32, 24), [255, 0, 0, 255]);
        assert_eq!(px(&out, 0, 0), [255, 0, 0, 255]);
        // outside the clip → black
        comp.render(&project, 20.0, 64, 48, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 32, 24), [0, 0, 0, 255]);
        // scale 0.5 + opacity 0.5 → centred half-size block at half brightness, corners black
        let mut p2 = project.clone();
        let c = &mut p2.tracks[0].clips[0];
        c.scale.value = 0.5;
        c.opacity.value = 0.5;
        comp.render(&p2, 1.0, 64, 48, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 32, 24), [128, 0, 0, 255]);
        assert_eq!(px(&out, 0, 0), [0, 0, 0, 255]);
        // muted (disabled) video track → black
        let mut p3 = project.clone();
        p3.tracks[0].muted = true;
        comp.render(&p3, 1.0, 64, 48, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 32, 24), [0, 0, 0, 255]);
        // missing decoder → skipped, no panic
        let mut empty = DecoderPool::new(Backend::Ffmpeg);
        empty.insert_video("other", Box::new(FakeVideo([0, 0, 0, 255])));
        comp.render(&project, 1.0, 16, 16, &mut empty, &mut text, &mut out);
        assert_eq!(px(&out, 8, 8), [0, 0, 0, 255]);
    }
}
