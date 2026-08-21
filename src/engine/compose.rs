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

use crate::engine::text::TextRasterizer;
use crate::media::{DecoderPool, Frame};
use crate::model::{Clip, Project};

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
pub fn placement(project: &Project, clip: &Clip, t: f64, native: (u32, u32), cw: u32, ch: u32, contain: bool) -> Placement {
    let lt = clip.local(t);
    let s = cw as f32 / project.width.max(1) as f32;
    let (nw, nh) = (native.0.max(1) as f32, native.1.max(1) as f32);
    let fit = if contain { (cw as f32 / nw).min(ch as f32 / nh) } else { 1.0 };
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
    #[allow(dead_code)]
    layer: Frame,
    #[allow(dead_code)]
    src: Frame,
}

impl Default for Compositor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compositor {
    pub fn new() -> Self {
        Self { layer: Frame::default(), src: Frame::default() }
    }

    /// Render timeline time `t` into `out` at w×h (out is resized). Never panics on missing media:
    /// a clip whose decoder fails is simply skipped.
    pub fn render(
        &mut self,
        _project: &Project,
        _t: f64,
        _w: u32,
        _h: u32,
        _pool: &mut DecoderPool,
        _text: &mut TextRasterizer,
        _out: &mut Frame,
    ) {
        todo!("engine::compose::Compositor::render")
    }
}

/// Draw `src` into `dst` (canvas) at `p` with blend mode & opacity, using `scratch` as the canvas-sized
/// layer buffer. Bilinear sampling; pixels outside `src` are transparent.
pub fn draw_layer(
    _dst: &mut Frame,
    _src: &Frame,
    _p: Placement,
    _mode: crate::model::BlendMode,
    _opacity: f32,
    _scratch: &mut Frame,
) {
    todo!("engine::compose::draw_layer")
}
