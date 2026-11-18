//! Getting a decoded frame onto the window. A zero-copy frame (D3D11, already RGB at the window's size
//! thanks to the video processor) is shared into GL (`WGL_NV_DX_interop2`, `engine::gpu::interop`) and
//! blitted GPU-to-GPU into one texture egui paints as a single quad - no compositor, no pixel crosses
//! the bus. Anything else (software decode, no interop, a headless test) is uploaded as an egui texture.

use crate::engine::gpu::interop::Interop;
use crate::media::Frame;
use eframe::egui;
use eframe::glow::{self, HasContext};
use std::sync::Arc;

#[derive(Default)]
pub struct VideoTex {
    /// GL side: the shared-texture copier, tried once.
    interop: Option<Interop>,
    interop_tried: bool,
    /// The GL texture frames are blitted into, at the largest size seen so far (eframe takes ownership
    /// of a registered texture and offers no unregister, so it grows, never churns).
    gl_tex: Option<(glow::Texture, (u32, u32), egui::TextureId)>,
    /// CPU side.
    cpu: Option<egui::TextureHandle>,
    scratch: Frame,
    /// What is on screen: (texture, pixel size, the part of the texture it fills).
    pub shown: Option<(egui::TextureId, (u32, u32), egui::Rect)>,
    /// The last frame went GPU-to-GPU.
    pub zero_copy: bool,
    /// Source time of the frame on screen.
    pub pts: f64,
}

impl VideoTex {
    /// Put `f` on screen. `gl` / `frame` are None in headless tests (CPU path only).
    pub fn show(&mut self, ctx: &egui::Context, f: &Frame, gl: Option<&Arc<glow::Context>>, frame: Option<&mut eframe::Frame>) {
        self.pts = f.pts;
        if let (Some(g), Some(gl), Some(frame)) = (f.gpu.as_ref(), gl, frame) {
            if !self.interop_tried {
                self.interop_tried = true;
                self.interop = Interop::open(gl);
            }
            if let Some(interop) = self.interop.as_mut() {
                let size = g.size();
                if self.gl_tex.is_none_or(|(_, s, _)| s.0 < size.0 || s.1 < size.1) {
                    let grown = self.gl_tex.map_or(size, |(_, s, _)| (s.0.max(size.0), s.1.max(size.1)));
                    if let Some(t) = new_texture(gl, grown) {
                        let id = frame.register_native_glow_texture(t);
                        self.gl_tex = Some((t, grown, id));
                    }
                }
                if let Some((t, tsize, id)) = self.gl_tex {
                    if interop.copy(g, t) {
                        let uv = egui::Rect::from_min_max(
                            egui::pos2(0.0, 0.0),
                            egui::pos2(size.0 as f32 / tsize.0 as f32, size.1 as f32 / tsize.1 as f32),
                        );
                        self.shown = Some((id, size, uv));
                        self.zero_copy = true;
                        return;
                    }
                }
                g.fail(); // the driver would not: CPU frames from here on
                self.interop = None;
            }
        }
        self.zero_copy = false;
        let rgba = if let Some(g) = &f.gpu {
            if !g.read(&mut self.scratch) {
                return;
            }
            self.scratch.make_rgba();
            &self.scratch
        } else if f.bgrx {
            self.scratch.copy_from(f);
            self.scratch.make_rgba();
            &self.scratch
        } else {
            f
        };
        let img = egui::ColorImage::from_rgba_unmultiplied([rgba.width as usize, rgba.height as usize], &rgba.rgba);
        let size = (rgba.width, rgba.height);
        match &mut self.cpu {
            Some(t) => t.set(img, egui::TextureOptions::LINEAR),
            None => self.cpu = Some(ctx.load_texture("player-video", img, egui::TextureOptions::LINEAR)),
        }
        let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        self.shown = self.cpu.as_ref().map(|t| (t.id(), size, full));
    }

    pub fn clear(&mut self) {
        self.shown = None;
    }
}

fn new_texture(gl: &glow::Context, (w, h): (u32, u32)) -> Option<glow::Texture> {
    unsafe {
        let t = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(t));
        for (k, v) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
        }
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            w as i32,
            h as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        gl.bind_texture(glow::TEXTURE_2D, None);
        Some(t)
    }
}
