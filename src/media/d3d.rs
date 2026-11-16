//! D3D11 half of zero-copy decode (#158). A hardware-decoded NV12/P010 sample is converted to RGB and
//! scaled to the size asked for by the D3D11 video processor, straight into a pooled BGRA texture that
//! stays on the GPU (`GpuFrame`). `engine::gpu::interop` shares that texture with GL; `GpuFrame::read`
//! is the way back to the CPU (CPU readers of a preview frame, an interop failure).
//!
//! Why the video processor and not an NV12 shader in GL: `WGL_NV_DX_interop2` only shares RGBA-class
//! textures (an NV12 texture does not register), so the YUV->RGB step has to happen on the D3D side.
//! The video processor does it in fixed-function hardware, scales on the way, and releases the
//! decoder's surface at once (the sample is dropped right after the blit).

use super::Frame;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use windows::core::Interface;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Media::MediaFoundation::{IMFDXGIBuffer, IMFSample};

/// Output surfaces a source keeps for reuse.
const FREE: usize = 6;
/// Surfaces one source may have out at once (the read-ahead holds a second or two of frames): past
/// this `Vp::frame` says no and the caller takes the CPU path for that frame.
/// ponytail: a count per source (~128 x 3.7 MB at 720p); a VRAM budget if many 4K layers matter.
pub const MAX_LIVE: usize = 128;
/// Input views kept (one per decoder array slice); cleared when a decoder reallocates past this.
const INPUTS: usize = 64;

struct Surface {
    tex: ID3D11Texture2D,
    view: ID3D11VideoProcessorOutputView,
    w: u32,
    h: u32,
}

/// What a source's frames share with it: the free surfaces, how many are out, and whether GL gave
/// up on them (`GpuFrame::fail`: the source decodes CPU frames from then on).
#[derive(Default)]
struct Shared {
    free: Mutex<Vec<Surface>>,
    live: AtomicUsize,
    failed: AtomicBool,
}

/// A decoded, converted and scaled frame in a BGRA8 D3D11 texture (row 0 = top, alpha 255).
pub struct GpuFrame {
    surf: Option<Surface>,
    shared: Arc<Shared>,
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
}

// SAFETY: the device is multithread protected (`mf::dxgi_device_manager` turns that on), so its
// immediate context may be called from any thread; the texture is only read once written.
unsafe impl Send for GpuFrame {}
unsafe impl Sync for GpuFrame {}

impl GpuFrame {
    pub fn texture(&self) -> &ID3D11Texture2D {
        &self.surf.as_ref().expect("surface held until drop").tex
    }
    pub fn size(&self) -> (u32, u32) {
        self.surf.as_ref().map_or((0, 0), |s| (s.w, s.h))
    }
    /// GL could not use it: the source hands out CPU frames from now on.
    pub fn fail(&self) {
        self.shared.failed.store(true, Ordering::Relaxed);
    }
    pub fn failed(&self) -> bool {
        self.shared.failed.load(Ordering::Relaxed)
    }
    /// Read the pixels back as a `Frame::bgrx` frame (a staging copy; blocks until the GPU is done).
    pub fn read(&self, out: &mut Frame) -> bool {
        let (w, h) = self.size();
        unsafe {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut st = None;
            if self.device.CreateTexture2D(&desc, None, Some(&mut st)).is_err() {
                return false;
            }
            let Some(st) = st else { return false };
            self.ctx.CopyResource(&st, self.texture());
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            if self.ctx.Map(&st, 0, D3D11_MAP_READ, 0, Some(&mut m)).is_err() || m.pData.is_null() {
                return false;
            }
            out.resize_for_overwrite(w, h);
            out.bgrx = true;
            let row = w as usize * 4;
            for (y, d) in out.rgba.chunks_exact_mut(row).enumerate() {
                d.copy_from_slice(std::slice::from_raw_parts((m.pData as *const u8).add(y * m.RowPitch as usize), row));
            }
            self.ctx.Unmap(&st, 0);
        }
        true
    }
}

impl Drop for GpuFrame {
    fn drop(&mut self) {
        self.shared.live.fetch_sub(1, Ordering::Relaxed);
        if let (Some(s), Ok(mut free)) = (self.surf.take(), self.shared.free.lock()) {
            if free.len() >= FREE {
                free.remove(0);
            }
            free.push(s);
        }
    }
}

/// One source's video processor: decoder slice in, BGRA at any size out.
pub struct Vp {
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    vdev: ID3D11VideoDevice,
    vctx: ID3D11VideoContext,
    /// Built on the first frame (the decoder's texture size is only known then).
    proc: Option<(ID3D11VideoProcessorEnumerator, ID3D11VideoProcessor)>,
    /// (decoder texture, array slice) -> input view; the texture is held so its address stays its own.
    inputs: Vec<(ID3D11Texture2D, u32, ID3D11VideoProcessorInputView)>,
    shared: Arc<Shared>,
    /// The picture inside the decoder's (padded, e.g. 1920x1088) texture.
    src: (u32, u32),
    /// `D3D11_VIDEO_PROCESSOR_COLOR_SPACE` bits of the input (`colour_space`).
    colour: u32,
}

// SAFETY: see `GpuFrame`; a `Vp` is owned by one decoder, used by one thread at a time.
unsafe impl Send for Vp {}

/// Input colour space bits: YCbCr_Matrix (bit 2: 0 = BT.601, 1 = BT.709) and Nominal_Range
/// (bits 4-5: 1 = 16-235, 2 = 0-255). The output is full-range RGB (all bits 0).
pub fn colour_space(bt709: bool, full_range: bool) -> u32 {
    (if bt709 { 1 << 2 } else { 0 }) | (if full_range { 2 << 4 } else { 1 << 4 })
}

impl Vp {
    pub fn new(device: ID3D11Device, src: (u32, u32), colour: u32) -> Result<Vp, String> {
        let e = |e: windows::core::Error| format!("D3D11 video: {e}");
        unsafe {
            let ctx = device.GetImmediateContext().map_err(e)?;
            Ok(Vp {
                vdev: device.cast().map_err(e)?,
                vctx: ctx.cast().map_err(e)?,
                device,
                ctx,
                proc: None,
                inputs: Vec::new(),
                shared: Arc::default(),
                src,
                colour,
            })
        }
    }
    /// GL gave up on this source's frames (`GpuFrame::fail`).
    pub fn failed(&self) -> bool {
        self.shared.failed.load(Ordering::Relaxed)
    }
    /// Frames of this source still out.
    pub fn live(&self) -> usize {
        self.shared.live.load(Ordering::Relaxed)
    }
    /// The hardware-decoded `sample` at `w` x `h`. Err when it is not a D3D11 sample (software decode).
    pub fn frame(&mut self, sample: &IMFSample, w: u32, h: u32) -> Result<GpuFrame, String> {
        let e = |e: windows::core::Error| format!("D3D11 sample: {e}");
        unsafe {
            let dx: IMFDXGIBuffer = sample.GetBufferByIndex(0).map_err(e)?.cast().map_err(e)?;
            let mut p = std::ptr::null_mut();
            dx.GetResource(&ID3D11Texture2D::IID, &mut p).map_err(e)?;
            let tex = ID3D11Texture2D::from_raw(p);
            let slice = dx.GetSubresourceIndex().map_err(e)?;
            self.blit(&tex, slice, w, h)
        }
    }
    /// Convert + scale array slice `slice` of the NV12/P010 texture `tex` into a fresh `GpuFrame`.
    pub fn blit(&mut self, tex: &ID3D11Texture2D, slice: u32, w: u32, h: u32) -> Result<GpuFrame, String> {
        let e = |e: windows::core::Error| format!("D3D11 video processor: {e}");
        if w == 0 || h == 0 {
            return Err("D3D11 video processor: empty output".into());
        }
        let surf = unsafe {
            if self.proc.is_none() {
                let mut d = D3D11_TEXTURE2D_DESC::default();
                tex.GetDesc(&mut d);
                let rate = DXGI_RATIONAL { Numerator: 30, Denominator: 1 };
                let cd = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                    InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                    InputFrameRate: rate,
                    InputWidth: d.Width,
                    InputHeight: d.Height,
                    OutputFrameRate: rate,
                    OutputWidth: self.src.0,
                    OutputHeight: self.src.1,
                    Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
                };
                let en = self.vdev.CreateVideoProcessorEnumerator(&cd).map_err(e)?;
                let vp = self.vdev.CreateVideoProcessor(&en, 0).map_err(e)?;
                let v = &self.vctx;
                v.VideoProcessorSetStreamFrameFormat(&vp, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
                v.VideoProcessorSetStreamAutoProcessingMode(&vp, 0, false);
                v.VideoProcessorSetStreamColorSpace(
                    &vp,
                    0,
                    &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: self.colour },
                );
                v.VideoProcessorSetOutputColorSpace(&vp, &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 });
                let (sw, sh) = (self.src.0.min(d.Width), self.src.1.min(d.Height));
                let r = RECT { left: 0, top: 0, right: sw as i32, bottom: sh as i32 };
                v.VideoProcessorSetStreamSourceRect(&vp, 0, true, Some(&r));
                self.proc = Some((en, vp));
            }
            let Some((en, vp)) = &self.proc else { unreachable!() };
            let i = match self.inputs.iter().position(|(t, s, _)| t == tex && *s == slice) {
                Some(i) => i,
                None => {
                    if self.inputs.len() >= INPUTS {
                        self.inputs.clear();
                    }
                    let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                        FourCC: 0,
                        ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                        Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                            Texture2D: D3D11_TEX2D_VPIV { MipSlice: 0, ArraySlice: slice },
                        },
                    };
                    let mut view = None;
                    self.vdev.CreateVideoProcessorInputView(tex, en, &desc, Some(&mut view)).map_err(e)?;
                    self.inputs.push((tex.clone(), slice, view.ok_or("D3D11: no input view")?));
                    self.inputs.len() - 1
                }
            };
            let taken = self.shared.free.lock().ok().and_then(|mut f| {
                let i = f.iter().position(|s| (s.w, s.h) == (w, h))?;
                Some(f.swap_remove(i))
            });
            let surf = match taken {
                Some(s) => s,
                None => self.surface(en, w, h)?,
            };
            let r = RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 };
            self.vctx.VideoProcessorSetStreamDestRect(vp, 0, true, Some(&r));
            self.vctx.VideoProcessorSetOutputTargetRect(vp, true, Some(&r));
            let mut streams = [D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: ManuallyDrop::new(Some(self.inputs[i].2.clone())),
                ..Default::default()
            }];
            let blt = self.vctx.VideoProcessorBlt(vp, &surf.view, 0, &streams);
            ManuallyDrop::drop(&mut streams[0].pInputSurface);
            blt.map_err(e)?;
            // queue it now: GL locks the texture from another thread and must find the work submitted
            self.ctx.Flush();
            surf
        };
        self.shared.live.fetch_add(1, Ordering::Relaxed);
        Ok(GpuFrame {
            surf: Some(surf),
            shared: self.shared.clone(),
            device: self.device.clone(),
            ctx: self.ctx.clone(),
        })
    }

    fn surface(&self, en: &ID3D11VideoProcessorEnumerator, w: u32, h: u32) -> Result<Surface, String> {
        let e = |e: windows::core::Error| format!("D3D11 surface: {e}");
        unsafe {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                CPUAccessFlags: 0,
                // WGL_NV_DX_interop2 registers it with GL - shared, or unregistering it crashes (see `engine::gpu::interop`)
                MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
            };
            let mut tex = None;
            self.device.CreateTexture2D(&desc, None, Some(&mut tex)).map_err(e)?;
            let tex = tex.ok_or("D3D11: no texture")?;
            let vd = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 } },
            };
            let mut view = None;
            self.vdev.CreateVideoProcessorOutputView(&tex, en, &vd, Some(&mut view)).map_err(e)?;
            Ok(Surface { tex, view: view.ok_or("D3D11: no output view")?, w, h })
        }
    }
}

/// CPU reference of the conversion the video processor does: one limited-range Y'CbCr sample to
/// full-range R'G'B' (BT.709 or BT.601). Tests hold the GPU output to it.
pub fn ycbcr_to_rgb(y: u8, cb: u8, cr: u8, bt709: bool) -> [u8; 3] {
    let (kr, kb) = if bt709 { (0.2126, 0.0722) } else { (0.299, 0.114) };
    let kg = 1.0 - kr - kb;
    let y = (y as f64 - 16.0) / 219.0;
    let (pb, pr) = ((cb as f64 - 128.0) / 224.0, (cr as f64 - 128.0) / 224.0);
    let r = y + 2.0 * (1.0 - kr) * pr;
    let b = y + 2.0 * (1.0 - kb) * pb;
    let g = (y - kr * r - kb * b) / kg;
    [r, g, b].map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8)
}

/// Tests: a `w` x `h` NV12 texture (the decoder's format), Y'CbCr `top` over its top half and
/// `bottom` under it - a flipped copy shows.
#[cfg(test)]
pub fn test_nv12(device: &ID3D11Device, w: u32, h: u32, top: [u8; 3], bottom: [u8; 3]) -> ID3D11Texture2D {
    let half = |y: u32, rows: u32| if y < rows / 2 { top } else { bottom };
    let mut data: Vec<u8> = (0..h).flat_map(|y| vec![half(y, h)[0]; w as usize]).collect();
    data.extend((0..h / 2).flat_map(|y| [half(y, h / 2)[1], half(y, h / 2)[2]].repeat(w as usize / 2)));
    unsafe {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_DECODER.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let init = D3D11_SUBRESOURCE_DATA { pSysMem: data.as_ptr() as _, SysMemPitch: w, SysMemSlicePitch: 0 };
        let mut tex = None;
        device.CreateTexture2D(&desc, Some(&init), Some(&mut tex)).expect("NV12 texture");
        tex.expect("NV12 texture")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_matrix_hits_the_known_points() {
        assert_eq!(ycbcr_to_rgb(16, 128, 128, true), [0, 0, 0]);
        assert_eq!(ycbcr_to_rgb(235, 128, 128, true), [255, 255, 255]);
        // 75% red bars: BT.709 (Y 51, Cb 109, Cr 212) vs BT.601 (Y 65, Cb 100, Cr 212)
        let r709 = ycbcr_to_rgb(51, 109, 212, true);
        let r601 = ycbcr_to_rgb(65, 100, 212, false);
        for c in [r709, r601] {
            assert!((c[0] as i32 - 191).abs() <= 2 && c[1] <= 2 && c[2] <= 2, "{c:?}");
        }
        assert_eq!(colour_space(true, false), 0b01_0100);
        assert_eq!(colour_space(false, true), 0b10_0000);
    }

    /// The video processor's NV12 -> RGB (both matrices, limited range) and its scaling, read back
    /// from the GPU, match the CPU reference within 3 levels.
    #[test]
    fn video_processor_converts_nv12_like_the_reference() {
        let Some(device) = super::super::mf::d3d_device() else {
            eprintln!("SKIPPED: no D3D11 video device");
            return;
        };
        for (bt709, (y, cb, cr)) in [(true, (63u8, 102u8, 212u8)), (false, (81, 90, 240)), (true, (145, 54, 34))] {
            let tex = test_nv12(&device, 64, 32, [y, cb, cr], [y, cb, cr]);
            let mut vp = Vp::new(device.clone(), (64, 32), colour_space(bt709, false)).unwrap();
            for (w, h) in [(64, 32), (20, 10)] {
                let g = vp.blit(&tex, 0, w, h).expect("blit");
                assert_eq!(g.size(), (w, h));
                let mut f = Frame::default();
                assert!(g.read(&mut f) && f.bgrx);
                let want = ycbcr_to_rgb(y, cb, cr, bt709);
                let p = &f.rgba[((h / 2 * w + w / 2) * 4) as usize..][..4];
                let got = [p[2], p[1], p[0]];
                let off = got.iter().zip(want).map(|(a, b)| (*a as i32 - b as i32).abs()).max().unwrap();
                assert!(off <= 3, "bt709 {bt709} {w}x{h}: got {got:?}, want {want:?}");
            }
            assert_eq!(vp.live(), 0, "dropped frames go back to the pool");
        }
    }
}
