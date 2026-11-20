//! GL half of zero-copy decode (#158). `WGL_NV_DX_interop2` (NVIDIA, AMD and Intel drivers) shares
//! the D3D11 textures of `media::d3d::GpuFrame`s - already RGB and already at the layer's size, the
//! D3D11 video processor saw to that - into this GL context, and `upload_gpu` copies one into its
//! layer texture with a framebuffer blit: no pixel crosses the bus.
//!
//! Opened once by `GpuRenderer::new`; only when that works does `media::mf::set_zero_copy` let a
//! preview pool decode zero-copy. A frame GL cannot take later (registering or locking fails) is read
//! back and uploaded the usual way, and its source goes back to CPU frames (`GpuFrame::fail`).
//!
//! Driver notes (RTX 2060S, verified): the D3D textures must be `D3D11_RESOURCE_MISC_SHARED` -
//! unregistering a plain one crashes inside `wglDXUnregisterObjectNV` - and nothing of GL's may still
//! point at a shared texture (an FBO attachment) once it is unlocked.

use super::{tex_params, GpuRenderer, LayerTex};
use crate::media::d3d::GpuFrame;
use crate::media::Frame;
use eframe::glow;
use eframe::glow::HasContext;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Arc;
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

type Handle = *mut c_void;
type OpenDevice = unsafe extern "system" fn(dx_device: *mut c_void) -> Handle;
type CloseDevice = unsafe extern "system" fn(device: Handle) -> i32;
type Register =
    unsafe extern "system" fn(device: Handle, dx_object: *mut c_void, name: u32, ty: u32, access: u32) -> Handle;
type Unregister = unsafe extern "system" fn(device: Handle, object: Handle) -> i32;
type Lock = unsafe extern "system" fn(device: Handle, count: i32, objects: *mut Handle) -> i32;

#[link(name = "opengl32")]
extern "system" {
    fn wglGetProcAddress(name: *const u8) -> *const c_void;
}

const WGL_ACCESS_READ_ONLY_NV: u32 = 0;
/// Registrations kept; past this the least recently used are let go (re-registered when seen again).
/// ponytail: a count - each decoder keeps at most `d3d::MAX_LIVE` surfaces, so this rarely trims.
const REGS: usize = 256;

struct Reg {
    tex: glow::Texture,
    handle: Handle,
    /// Holds the D3D texture so its address (the key) stays its own while registered.
    _d3d: ID3D11Texture2D,
    used: u64,
}

pub struct Interop {
    gl: Arc<glow::Context>,
    device: Handle,
    close: CloseDevice,
    register: Register,
    unregister: Unregister,
    lock: Lock,
    unlock: Lock,
    regs: HashMap<usize, Reg>,
    fbos: (glow::Framebuffer, glow::Framebuffer),
    tick: u64,
}

impl Interop {
    /// Share the decode device (`mf::d3d_device`) with the current GL context. None - and zero-copy
    /// stays off - when the extension is missing or the driver refuses (another adapter, say).
    pub fn open(gl: &Arc<glow::Context>) -> Option<Interop> {
        let device = crate::media::mf::d3d_device()?;
        unsafe {
            let f = |n: &[u8]| {
                let p = wglGetProcAddress(n.as_ptr());
                (p as usize > 3).then_some(p) // 1, 2, 3 and -1 are documented failure values too
            };
            let open: OpenDevice = std::mem::transmute(f(b"wglDXOpenDeviceNV\0")?);
            let close: CloseDevice = std::mem::transmute(f(b"wglDXCloseDeviceNV\0")?);
            let register: Register = std::mem::transmute(f(b"wglDXRegisterObjectNV\0")?);
            let unregister: Unregister = std::mem::transmute(f(b"wglDXUnregisterObjectNV\0")?);
            let lock: Lock = std::mem::transmute(f(b"wglDXLockObjectsNV\0")?);
            let unlock: Lock = std::mem::transmute(f(b"wglDXUnlockObjectsNV\0")?);
            let handle = open(device.as_raw());
            if handle.is_null() {
                return None;
            }
            let fbos = match (gl.create_framebuffer(), gl.create_framebuffer()) {
                (Ok(a), Ok(b)) => (a, b),
                _ => {
                    close(handle);
                    return None;
                }
            };
            Some(Interop {
                gl: gl.clone(),
                device: handle,
                close,
                register,
                unregister,
                lock,
                unlock,
                regs: HashMap::new(),
                fbos,
                tick: 0,
            })
        }
    }

    /// The GL texture `g`'s D3D texture is registered as (registering it on first sight).
    fn share(&mut self, g: &GpuFrame) -> Option<(glow::Texture, Handle)> {
        self.tick += 1;
        let d3d = g.texture();
        let key = d3d.as_raw() as usize;
        if let Some(r) = self.regs.get_mut(&key) {
            r.used = self.tick;
            return Some((r.tex, r.handle));
        }
        if self.regs.len() >= REGS {
            let mut old: Vec<(usize, u64)> = self.regs.iter().map(|(k, r)| (*k, r.used)).collect();
            old.sort_by_key(|e| e.1);
            for (k, _) in old.into_iter().take(REGS / 4) {
                self.drop_reg(k);
            }
        }
        unsafe {
            let tex = self.gl.create_texture().ok()?;
            let handle =
                (self.register)(self.device, d3d.as_raw(), tex.0.get(), glow::TEXTURE_2D, WGL_ACCESS_READ_ONLY_NV);
            if handle.is_null() {
                self.gl.delete_texture(tex);
                return None;
            }
            self.regs.insert(key, Reg { tex, handle, _d3d: d3d.clone(), used: self.tick });
            Some((tex, handle))
        }
    }

    fn drop_reg(&mut self, key: usize) {
        if let Some(r) = self.regs.remove(&key) {
            unsafe {
                (self.unregister)(self.device, r.handle);
                self.gl.delete_texture(r.tex);
            }
        }
    }

    /// Copy `g` into `dst` (an RGBA8 texture of `g`'s size). False when the driver would not.
    pub fn copy(&mut self, g: &GpuFrame, dst: glow::Texture) -> bool {
        let (w, h) = g.size();
        let Some((src, mut handle)) = self.share(g) else { return false };
        let gl = self.gl.clone();
        unsafe {
            if (self.lock)(self.device, 1, &mut handle) == 0 {
                return false;
            }
            let (read, draw) = (
                gl.get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING),
                gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING),
            );
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(self.fbos.0));
            gl.framebuffer_texture_2d(glow::READ_FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(src), 0);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(self.fbos.1));
            gl.framebuffer_texture_2d(glow::DRAW_FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(dst), 0);
            let ok = gl.check_framebuffer_status(glow::READ_FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;
            if ok {
                let (w, h) = (w as i32, h as i32);
                gl.blit_framebuffer(0, 0, w, h, 0, 0, w, h, glow::COLOR_BUFFER_BIT, glow::NEAREST);
            }
            // nothing of GL's may point at the shared texture once it is unlocked
            gl.framebuffer_texture_2d(glow::READ_FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, None, 0);
            gl.framebuffer_texture_2d(glow::DRAW_FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, None, 0);
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, read);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, draw);
            (self.unlock)(self.device, 1, &mut handle);
            ok
        }
    }
}

impl Drop for Interop {
    fn drop(&mut self) {
        let keys: Vec<usize> = self.regs.keys().copied().collect();
        for k in keys {
            self.drop_reg(k);
        }
        unsafe {
            self.gl.delete_framebuffer(self.fbos.0);
            self.gl.delete_framebuffer(self.fbos.1);
            (self.close)(self.device);
        }
    }
}

impl GpuRenderer {
    /// `upload` for a zero-copy frame: a GPU-side copy into the layer texture through the interop;
    /// when that fails, the frame is read back and uploaded as bytes, and its source decodes CPU
    /// frames from then on.
    pub(super) fn upload_gpu(&mut self, key: u64, frame: &Frame, g: &Arc<GpuFrame>) -> glow::Texture {
        let key = key | self.key_tag;
        let rev = (Arc::as_ptr(g) as usize, frame.pts.to_bits());
        let (w, h) = g.size();
        let used = self.renders;
        let gl = self.gl.clone();
        let tex = match self.textures.get_mut(&key) {
            Some(lt) if (lt.w, lt.h) == (w, h) && !lt.deep => {
                lt.used = used;
                if lt.rev == rev {
                    return lt.tex;
                }
                lt.tex
            }
            _ => unsafe {
                if let Some(old) = self.textures.remove(&key) {
                    gl.delete_texture(old.tex);
                }
                let Ok(tex) = gl.create_texture() else { return self.blank };
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                tex_params(&gl);
                let fmt = glow::RGBA8 as i32;
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    fmt,
                    w as i32,
                    h as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
                self.textures.insert(key, LayerTex { tex, w, h, rev: (0, 0), used, deep: false });
                tex
            },
        };
        if self.interop.as_mut().is_some_and(|i| i.copy(g, tex)) {
            if let Some(lt) = self.textures.get_mut(&key) {
                lt.rev = rev;
            }
            return tex;
        }
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| eprintln!("zero-copy decode: GL interop failed, that source goes back to CPU frames"));
        g.fail();
        let mut cpu = Frame::default();
        if !g.read(&mut cpu) {
            return self.blank;
        }
        cpu.pts = frame.pts;
        self.upload(key & !self.key_tag, &cpu)
    }
}

#[cfg(test)]
mod tests {
    use super::super::driver_tests::gl;
    use super::*;
    use crate::media::d3d::{colour_space, test_nv12, ycbcr_to_rgb, Vp};

    /// NV12 on the D3D side -> video processor -> interop -> layer texture -> glReadPixels: the
    /// colours match the CPU reference (BT.709 limited range), top stays top, and the texture is
    /// re-used, not re-registered, for the next frame.
    #[test]
    fn gpu_frames_reach_the_layer_texture_through_the_interop() {
        let (Some(gl), Some(device)) = (gl(), crate::media::mf::d3d_device()) else {
            eprintln!("SKIPPED: no OpenGL driver / D3D11 video device");
            return;
        };
        let gl = Arc::new(gl);
        let mut r = GpuRenderer::new(gl.clone()).expect("renderer");
        if r.interop.is_none() {
            eprintln!("SKIPPED: WGL_NV_DX_interop2 not available here");
            return;
        }
        assert!(crate::media::mf::zero_copy(), "an open interop turns zero-copy decode on");
        let (top, bottom) = ([145u8, 54, 34], [63u8, 102, 212]);
        let src = test_nv12(&device, 64, 32, top, bottom);
        let mut vp = Vp::new(device.clone(), (64, 32), colour_space(true, false)).unwrap();
        let (w, h) = (32u32, 16u32);
        let g = Arc::new(vp.blit(&src, 0, w, h).unwrap());
        let frame = Frame { width: w, height: h, pts: 1.0, gpu: Some(g.clone()), ..Frame::default() };
        let tex = r.upload(7, &frame);
        assert!(!g.failed(), "the interop took it");
        let mut px = vec![0u8; (w * h * 4) as usize];
        unsafe {
            let fbo = gl.create_framebuffer().unwrap();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
            gl.read_pixels(
                0,
                0,
                w as i32,
                h as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut px)),
            );
        }
        for (row, yuv) in [(2u32, top), (h - 3, bottom)] {
            let p = &px[((row * w + w / 2) * 4) as usize..][..4];
            let want = ycbcr_to_rgb(yuv[0], yuv[1], yuv[2], true);
            let off = (0..3).map(|k| (p[k] as i32 - want[k] as i32).abs()).max().unwrap();
            assert!(off <= 3 && p[3] == 255, "texel row {row}: {p:?}, want {want:?}");
        }
        let regs = r.interop.as_ref().unwrap().regs.len();
        drop((frame, g)); // its surface goes back to the pool for the next frame
        let g2 = Arc::new(vp.blit(&src, 0, w, h).unwrap());
        let next = Frame { width: w, height: h, pts: 2.0, gpu: Some(g2), ..Frame::default() };
        assert_eq!(r.upload(7, &next), tex, "same layer texture");
        assert_eq!(r.interop.as_ref().unwrap().regs.len(), regs, "the pooled surface is registered once");
    }

    /// Without the interop (another adapter, an old driver) a GPU frame still arrives: read back and
    /// uploaded as bytes, and its source is told to go back to CPU frames.
    #[test]
    fn without_the_interop_gpu_frames_fall_back_to_a_readback() {
        let (Some(gl), Some(device)) = (gl(), crate::media::mf::d3d_device()) else {
            eprintln!("SKIPPED: no OpenGL driver / D3D11 video device");
            return;
        };
        let gl = Arc::new(gl);
        let mut r = GpuRenderer::new(gl.clone()).expect("renderer");
        r.interop = None;
        let yuv = [63u8, 102, 212];
        let src = test_nv12(&device, 64, 32, yuv, yuv);
        let mut vp = Vp::new(device, (64, 32), colour_space(true, false)).unwrap();
        let g = Arc::new(vp.blit(&src, 0, 16, 8).unwrap());
        let tex = r.upload(9, &Frame { width: 16, height: 8, pts: 0.0, gpu: Some(g.clone()), ..Frame::default() });
        assert!(g.failed() && vp.failed(), "the source is told");
        let mut px = vec![0u8; 16 * 8 * 4];
        unsafe {
            let fbo = gl.create_framebuffer().unwrap();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
            gl.read_pixels(0, 0, 16, 8, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px)));
        }
        let want = ycbcr_to_rgb(yuv[0], yuv[1], yuv[2], true);
        let off = (0..3).map(|k| (px[k] as i32 - want[k] as i32).abs()).max().unwrap();
        assert!(off <= 3 && px[3] == 255, "{:?} vs {want:?}", &px[..4]);
    }
}
