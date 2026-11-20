//! Object masks (`MaskShape::Object`, `engine::sam2`) on the GPU. The cached mask frame covers the
//! layer's source frame, so it is applied in layer space: a clip's object mask multiplies the layer's
//! alpha before the composite (no canvas-space coverage, so any transform keeps it on the object), and
//! an effect's object mask is drawn as a coverage target of the layer's size. The mask frame is an R8
//! texture in `textures` (per clip), uploaded only when the frame changes. Nothing here decodes:
//! `sam2::frame_for` hands back what is in memory while playing.

use super::*;
use crate::engine::sam2;
use crate::model::{Mask, MaskShape};

pub(super) const K_OBJECT: u64 = 14; // free: 6..=14 (EFFECT_BASE is 16)
/// Mode 0: the layer with its alpha times the mask. Mode 1: the coverage itself (an effect's mask).
pub(super) const OBJECT_BODY: &str = r#"
uniform sampler2D u_b;
uniform int u_obj_invert;
uniform float u_obj_opacity;
uniform int u_obj_mode;
vec4 effect(vec4 src, vec2 uv) {
    float k = texture(u_b, uv).r;
    if (u_obj_invert != 0) { k = 1.0 - k; }
    k = clamp(k * u_obj_opacity, 0.0, 1.0);
    return (u_obj_mode == 0) ? vec4(src.rgb, src.a * k) : vec4(k, k, k, 1.0);
}
"#;

/// The texture key of `clip`'s mask frame (apart from every layer key).
fn key(clip: Id, tag: u64) -> u64 {
    (clip ^ 0x0b1e_c700_0000_0000) | tag
}

/// `mask` is an object mask that is on.
pub(super) fn is_object(mask: &Mask) -> bool {
    mask.enabled && mask.shape == MaskShape::Object
}

impl GpuRenderer {
    /// The R8 texture of `m` for `clip` (re-uploaded only when the frame changes).
    fn object_texture(&mut self, clip: Id, m: &sam2::Matte) -> glow::Texture {
        let key = key(clip, self.key_tag);
        let gl = self.gl.clone();
        let rev = (m as *const sam2::Matte as usize, m.idx as u64);
        let (w, h) = (m.w, m.h);
        let used = self.renders;
        unsafe {
            if let Some(lt) = self.textures.get_mut(&key) {
                if lt.w == w && lt.h == h {
                    lt.used = used;
                    if lt.rev != rev {
                        lt.rev = rev;
                        gl.bind_texture(glow::TEXTURE_2D, Some(lt.tex));
                        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                        gl.tex_sub_image_2d(
                            glow::TEXTURE_2D,
                            0,
                            0,
                            0,
                            w as i32,
                            h as i32,
                            glow::RED,
                            glow::UNSIGNED_BYTE,
                            glow::PixelUnpackData::Slice(Some(&m.px)),
                        );
                        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
                    }
                    return lt.tex;
                }
            }
            if let Some(old) = self.textures.remove(&key) {
                gl.delete_texture(old.tex);
            }
            let Ok(tex) = gl.create_texture() else { return self.blank };
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            tex_params(&gl);
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::R8 as i32,
                w as i32,
                h as i32,
                0,
                glow::RED,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&m.px)),
            );
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            self.textures.insert(key, LayerTex { tex, w, h, rev, used, deep: false });
            tex
        }
    }

    /// One K_OBJECT pass over `src` (size) with `clip`'s current mask frame. None = no frame yet (the
    /// caller then draws as if there were no mask).
    fn object_pass(&mut self, src: glow::Texture, size: (u32, u32), mask: &Mask, clip: &Clip, t: f64, mode: i32) -> Option<Target> {
        let m = sam2::frame_for(mask, clip, t)?;
        self.ensure(K_OBJECT).ok()?;
        let b = self.object_texture(clip.id, &m);
        let dst = self.acquire(size.0, size.1)?;
        let gl = self.gl.clone();
        let op = mask.opacity.at(clip.local(t)).clamp(0.0, 1.0) as f32;
        self.draw(K_OBJECT, &dst, src, None, |p| unsafe {
            gl.uniform_2_f32(p.uni.get("u_res"), size.0 as f32, size.1 as f32);
            gl.active_texture(glow::TEXTURE0 + U_B);
            gl.bind_texture(glow::TEXTURE_2D, Some(b));
            gl.uniform_1_i32(p.uni.get("u_b"), U_B as i32);
            gl.uniform_1_i32(p.uni.get("u_obj_invert"), i32::from(mask.invert));
            gl.uniform_1_f32(p.uni.get("u_obj_opacity"), op);
            gl.uniform_1_i32(p.uni.get("u_obj_mode"), mode);
        });
        Some(dst)
    }

    /// `src` with `clip`'s object mask multiplied into its alpha (layer space).
    pub(super) fn object_layer(&mut self, src: glow::Texture, size: (u32, u32), mask: &Mask, clip: &Clip, t: f64) -> Option<Target> {
        self.object_pass(src, size, mask, clip, t, 0)
    }

    /// An effect's mask target of the layer's size: the object's coverage for an object mask, else the
    /// shape (`render_mask`, centred on the layer).
    pub(super) fn layer_mask(&mut self, mask: &Mask, clip: &Clip, t: f64, size: (u32, u32), scale: f32) -> Option<Target> {
        if mask.shape == MaskShape::Object {
            let blank = self.blank;
            return self.object_pass(blank, size, mask, clip, t, 1);
        }
        self.render_mask(mask, size, clip.local(t), scale, (size.0 as f32 / 2.0, size.1 as f32 / 2.0))
    }
}
