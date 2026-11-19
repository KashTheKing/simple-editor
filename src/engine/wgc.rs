//! Application / game capture: Windows Graphics Capture (WGC) of one window.
//!
//! gdigrab reads a window through GDI, which shows black or stale pictures for GPU-rendered windows
//! (Roblox, Roblox Studio, most games) and records whatever covers it. WGC hands over the window's own
//! composed surface as a D3D11 texture - covered, moved or on another monitor.
//!
//! One thread per capture: it owns the D3D11 device, the frame pool and the session, and once per
//! `1/fps` takes the newest frame (WGC only delivers frames when something changed; the last one is
//! repeated so the encoder sees a steady rate):
//! * recording: the frame is copied to a staging texture and written to ffmpeg's stdin as raw BGRA
//!   (`-f rawvideo -use_wallclock_as_timestamps 1`, so A/V sync works like gdigrab's wall clock). The
//!   output keeps the size the window had at the start; a resized window is cropped / padded with black.
//! * preview: the frame goes into a texture with a mip chain, `GenerateMips` halves it on the GPU until
//!   it fits `PREVIEW_MAX`, and only that small level is read back (a few KB, not the full frame).
//!   Read-backs are double-buffered (map last tick's copy), so the thread never stalls on the GPU.
//!   Skipped while nobody looks (`FrameSlot::wanted`).
//! Stop = the flag; the thread drops ffmpeg's stdin (EOF ends the recording cleanly) and exits.

use crate::engine::screen_grab::{Frame, FrameSlot, PREVIEW_MAX};
use std::io::Write;
use std::process::ChildStdin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A window worth capturing, for the source list.
#[derive(Clone, Debug, PartialEq)]
pub struct AppWindow {
    pub hwnd: isize,
    pub title: String,
    /// The program's file name (`RobloxStudioBeta.exe`).
    pub exe: String,
    pub pid: u32,
}

/// What a WGC capture records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Window(isize),
    /// A whole monitor (`HMONITOR`): the cheap way to preview a screen (GPU downscale, no GDI grab).
    Monitor(isize),
}

/// A running WGC capture.
pub struct AppCapture {
    stop: Arc<AtomicBool>,
    encoder: Arc<Mutex<Option<ChildStdin>>>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Output size (even), the window's size when the capture started.
    pub size: (u32, u32),
}

impl AppCapture {
    /// Start capturing `hwnd`; frames go to `slot` for the preview. Fails when the window is gone or
    /// Windows refuses to capture it.
    pub fn start(hwnd: isize, cursor: bool, fps: u32, slot: Arc<FrameSlot>) -> Result<AppCapture, String> {
        Self::start_target(Target::Window(hwnd), cursor, fps, slot)
    }
    /// Start capturing a window or a monitor.
    pub fn start_target(target: Target, cursor: bool, fps: u32, slot: Arc<FrameSlot>) -> Result<AppCapture, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let encoder: Arc<Mutex<Option<ChildStdin>>> = Arc::new(Mutex::new(None));
        let (tx, rx) = mpsc::channel();
        let (s, e) = (stop.clone(), encoder.clone());
        let thread = std::thread::Builder::new()
            .name("app-capture".into())
            .spawn(move || {
                if let Err(err) = run(target, cursor, fps.clamp(1, 120), &slot, &s, &e, &tx) {
                    let _ = tx.send(Err(err));
                }
            })
            .map_err(|e| e.to_string())?;
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(size)) => Ok(AppCapture { stop, encoder, thread: Some(thread), size }),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                Err("The window didn't start capturing.".into())
            }
        }
    }
    /// Feed the recording: every frame from now on is written to this ffmpeg stdin.
    pub fn attach_encoder(&self, stdin: ChildStdin) {
        *self.encoder.lock().unwrap_or_else(|p| p.into_inner()) = Some(stdin);
    }
    /// Ask the thread to stop (returns at once; ffmpeg sees EOF when the thread lets go of stdin).
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Drop for AppCapture {
    fn drop(&mut self) {
        self.stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn err(what: &str) -> impl Fn(windows::core::Error) -> String + '_ {
    move |e| format!("{what}: {}", e.message())
}

fn run(
    target: Target,
    cursor: bool,
    fps: u32,
    slot: &FrameSlot,
    stop: &AtomicBool,
    encoder: &Mutex<Option<ChildStdin>>,
    ready: &mpsc::Sender<Result<(u32, u32), String>>,
) -> Result<(), String> {
    use windows::core::Interface;
    use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
    use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
    use windows::Graphics::DirectX::DirectXPixelFormat;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    use windows::Win32::Graphics::Dxgi::IDXGIDevice;
    use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
    use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
        let mut device = None;
        let mut ctx = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            Default::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut ctx),
        )
        .map_err(err("Direct3D"))?;
        let (device, ctx): (ID3D11Device, ID3D11DeviceContext) =
            (device.ok_or("no Direct3D device")?, ctx.ok_or("no Direct3D context")?);
        let dxgi: IDXGIDevice = device.cast().map_err(err("Direct3D"))?;
        let rt_device: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi).map_err(err("Direct3D"))?.cast().map_err(err("Direct3D"))?;
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().map_err(err("Graphics Capture"))?;
        let item: GraphicsCaptureItem = match target {
            Target::Window(hwnd) => interop
                .CreateForWindow(HWND(hwnd as *mut _))
                .map_err(|_| "This window can't be captured (it may have closed).".to_string())?,
            Target::Monitor(m) => interop
                .CreateForMonitor(windows::Win32::Graphics::Gdi::HMONITOR(m as *mut _))
                .map_err(|_| "This screen can't be captured.".to_string())?,
        };
        let mut size = item.Size().map_err(err("Graphics Capture"))?;
        let fmt = DirectXPixelFormat::B8G8R8A8UIntNormalized;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&rt_device, fmt, 2, size).map_err(err("Graphics Capture"))?;
        let session = pool.CreateCaptureSession(&item).map_err(err("Graphics Capture"))?;
        let _ = session.SetIsCursorCaptureEnabled(cursor);
        // the yellow border is only a hint for the user; Windows 11 lets an unpackaged app drop it
        let _ = session.SetIsBorderRequired(false);
        session.StartCapture().map_err(err("Graphics Capture"))?;

        let (w0, h0) = ((size.Width.max(2) as u32) & !1, (size.Height.max(2) as u32) & !1);
        let tex = |w: u32, h: u32, mips: u32, usage, bind: u32, cpu: u32, misc: u32| -> Result<ID3D11Texture2D, String> {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: mips,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: usage,
                BindFlags: bind,
                CPUAccessFlags: cpu,
                MiscFlags: misc,
            };
            let mut t = None;
            device.CreateTexture2D(&desc, None, Some(&mut t)).map_err(err("Direct3D"))?;
            t.ok_or_else(|| "no texture".to_string())
        };
        // two of each read-back texture: this tick's copy goes into one while last tick's is read from the
        // other, so the thread never waits for the GPU (one frame of extra lag, no stalls)
        let staging = [
            tex(w0, h0, 1, D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ.0 as u32, 0)?,
            tex(w0, h0, 1, D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ.0 as u32, 0)?,
        ];
        // preview: halve until it fits
        let mut level = 0u32;
        while (w0 >> level).max(h0 >> level) > PREVIEW_MAX && level < 10 {
            level += 1;
        }
        let mips = tex(
            w0,
            h0,
            level + 1,
            D3D11_USAGE_DEFAULT,
            (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
            0,
            D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32,
        )?;
        let (pw, ph) = ((w0 >> level).max(1), (h0 >> level).max(1));
        let small = [
            tex(pw, ph, 1, D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ.0 as u32, 0)?,
            tex(pw, ph, 1, D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ.0 as u32, 0)?,
        ];
        let (mut big_filled, mut small_filled) = ([false; 2], [false; 2]);
        let mut tick = 0usize;
        let mut srv = None;
        device.CreateShaderResourceView(&mips, None, Some(&mut srv)).map_err(err("Direct3D"))?;
        let srv = srv.ok_or("no view")?;
        let _ = ready.send(Ok((w0, h0)));

        let period = Duration::from_secs_f64(1.0 / fps as f64);
        let mut next = Instant::now();
        let mut have = false;
        let mut buf: Vec<u8> = Vec::with_capacity((w0 * h0 * 4) as usize);
        while !stop.load(Ordering::SeqCst) {
            // newest frame only; older ones are dropped
            let mut latest = None;
            while let Ok(f) = pool.TryGetNextFrame() {
                latest = Some(f);
            }
            if let Some(f) = latest {
                let content = f.ContentSize().unwrap_or(size);
                if let Ok(surface) = f.Surface() {
                    if let Ok(access) = surface.cast::<IDirect3DDxgiInterfaceAccess>() {
                        if let Ok(src) = access.GetInterface::<ID3D11Texture2D>() {
                            // copy what fits the fixed output size, top-left
                            let b = D3D11_BOX {
                                left: 0,
                                top: 0,
                                front: 0,
                                right: (content.Width.max(1) as u32).min(w0),
                                bottom: (content.Height.max(1) as u32).min(h0),
                                back: 1,
                            };
                            if b.right < w0 || b.bottom < h0 {
                                // a smaller window leaves the rest black, not last frame's leftovers
                                let rtv: Option<ID3D11RenderTargetView> = {
                                    let mut v = None;
                                    let _ = device.CreateRenderTargetView(&mips, None, Some(&mut v));
                                    v
                                };
                                if let Some(v) = rtv {
                                    ctx.ClearRenderTargetView(&v, &[0.0, 0.0, 0.0, 1.0]);
                                }
                            }
                            ctx.CopySubresourceRegion(&mips, 0, 0, 0, 0, &src, 0, Some(&b));
                            have = true;
                        }
                    }
                }
                // the window was resized: the pool follows it (the output keeps its size)
                if content.Width != size.Width || content.Height != size.Height {
                    size = content;
                    let _ = pool.Recreate(&rt_device, fmt, 2, size);
                }
            }
            if have {
                let feeding = encoder.lock().map(|g| g.is_some()).unwrap_or(false);
                let (cur, prev) = (tick % 2, (tick + 1) % 2);
                if feeding {
                    ctx.CopySubresourceRegion(&staging[cur], 0, 0, 0, 0, &mips, 0, None);
                    big_filled[cur] = true;
                    let mut m = D3D11_MAPPED_SUBRESOURCE::default();
                    if big_filled[prev]
                        && ctx.Map(&staging[prev], 0, D3D11_MAP_READ, 0, Some(&mut m)).is_ok()
                        && !m.pData.is_null()
                    {
                        let row = w0 as usize * 4;
                        buf.clear();
                        for y in 0..h0 as usize {
                            buf.extend_from_slice(std::slice::from_raw_parts((m.pData as *const u8).add(y * m.RowPitch as usize), row));
                        }
                        ctx.Unmap(&staging[prev], 0);
                        let mut g = encoder.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(si) = g.as_mut() {
                            if si.write_all(&buf).is_err() {
                                *g = None; // ffmpeg is gone; keep previewing
                            }
                        }
                    }
                }
                if slot.wanted.load(Ordering::Relaxed) {
                    if level > 0 {
                        ctx.GenerateMips(&srv);
                    }
                    ctx.CopySubresourceRegion(&small[cur], 0, 0, 0, 0, &mips, level, None);
                    small_filled[cur] = true;
                    let mut m = D3D11_MAPPED_SUBRESOURCE::default();
                    if small_filled[prev]
                        && ctx.Map(&small[prev], 0, D3D11_MAP_READ, 0, Some(&mut m)).is_ok()
                        && !m.pData.is_null()
                    {
                        let mut rgba = Vec::with_capacity((pw * ph * 4) as usize);
                        for y in 0..ph as usize {
                            let r = std::slice::from_raw_parts((m.pData as *const u8).add(y * m.RowPitch as usize), pw as usize * 4);
                            for p in r.chunks_exact(4) {
                                rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
                            }
                        }
                        ctx.Unmap(&small[prev], 0);
                        slot.put(Frame { size: [pw as usize, ph as usize], rgba });
                    }
                }
            }
            tick += 1;
            next += period;
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else {
                next = now; // fell behind: don't burst to catch up
            }
        }
        let _ = session.Close();
        let _ = pool.Close();
        encoder.lock().unwrap_or_else(|p| p.into_inner()).take(); // EOF: ffmpeg finishes the file
    }
    Ok(())
}

/// The monitor whose rectangle is exactly `[x, y, w, h]` (a "Screen N" source), as an `HMONITOR`.
pub fn monitor_at(rect: [i32; 4]) -> Option<isize> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONULL};
    let [x, y, w, h] = rect;
    unsafe {
        let m = MonitorFromPoint(POINT { x: x + w / 2, y: y + h / 2 }, MONITOR_DEFAULTTONULL);
        if m.is_invalid() {
            return None;
        }
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(m, &mut info).as_bool() {
            return None;
        }
        let r = info.rcMonitor;
        ([r.left, r.top, r.right - r.left, r.bottom - r.top] == rect).then_some(m.0 as isize)
    }
}

/// Windows that can be captured as an application, with their program names (Roblox, Studio, games...).
pub fn app_windows() -> Vec<AppWindow> {
    let mut out = Vec::new();
    crate::engine::screen_grab::for_each_window(|h, title| {
        let pid = window_pid(h.0 as isize);
        if pid != std::process::id() {
            out.push(AppWindow { hwnd: h.0 as isize, title: title.to_string(), exe: exe_name(pid), pid });
        }
        false
    });
    out
}

/// The process that owns a window.
pub fn window_pid(hwnd: isize) -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(windows::Win32::Foundation::HWND(hwnd as *mut _), Some(&mut pid));
    }
    pid
}

/// True while `hwnd` is still a window.
pub fn window_alive(hwnd: isize) -> bool {
    unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(windows::Win32::Foundation::HWND(hwnd as *mut _))).as_bool() }
}

/// A process's file name, "" when it can't be read.
pub fn exe_name(pid: u32) -> String {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 512];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..n as usize]);
        path.rsplit('\\').next().unwrap_or("").to_string()
    }
}

/// A window's icon as 32x32 RGBA (None when it has none).
pub fn window_icon(hwnd: isize) -> Option<Frame> {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe {
        let h = HWND(hwnd as *mut _);
        let mut res = 0usize;
        let _ = SendMessageTimeoutW(h, WM_GETICON, WPARAM(ICON_BIG as usize), LPARAM(0), SMTO_ABORTIFHUNG, 50, Some(&mut res));
        if res == 0 {
            res = GetClassLongPtrW(h, GCLP_HICON) as usize;
        }
        if res == 0 {
            return None;
        }
        let hicon = HICON(res as *mut _);
        const S: i32 = 32;
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: S,
                biHeight: -S,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let out = match CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(dib) if !bits.is_null() => {
                let old = SelectObject(mem, dib.into());
                let ok = DrawIconEx(mem, 0, 0, hicon, S, S, 0, None, DI_NORMAL).is_ok();
                let px = std::slice::from_raw_parts(bits as *const u8, (S * S * 4) as usize);
                let frame = ok.then(|| {
                    let mut rgba = Vec::with_capacity(px.len());
                    for p in px.chunks_exact(4) {
                        rgba.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
                    }
                    Frame { size: [S as usize, S as usize], rgba }
                });
                SelectObject(mem, old);
                let _ = DeleteObject(dib.into());
                frame
            }
            _ => None,
        };
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list never offers our own windows (recording the recorder is never what you want).
    #[test]
    fn app_list_skips_our_own_windows() {
        let me = std::process::id();
        assert!(app_windows().iter().all(|w| w.pid != me && !w.title.is_empty()));
        assert_eq!(exe_name(me).to_ascii_lowercase().ends_with(".exe"), true, "{}", exe_name(me));
    }
}
