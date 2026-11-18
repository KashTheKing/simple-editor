//! The Screen Recording pane's live preview and window list.
//!
//! `Preview` grabs what the recorder records - a desktop rectangle, or a window's client area exactly
//! like gdigrab's `title=` does - with one GDI `StretchBlt` (HALFTONE) straight into a small DIB, on
//! its own thread, a few times a second. Nothing full-size is ever copied, encoded or uploaded, so it
//! costs the recording and the editor next to nothing (~2-6 ms of GDI per grab, measured at 4K).
//! The thread stops when the `Preview` is dropped (the pane hides it when it isn't on screen).

use crate::engine::capture::CaptureSource;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Longest side of a preview frame, in pixels.
pub const PREVIEW_MAX: u32 = 360;

/// A grabbed frame: RGBA, `size` = [w, h].
#[derive(Clone)]
pub struct Frame {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

#[derive(Default)]
struct Shared {
    source: Mutex<Option<CaptureSource>>,
    frame: Mutex<Option<Frame>>,
    /// Bumped per new frame, so the UI only re-uploads its texture when something changed.
    generation: AtomicU64,
    stop: AtomicBool,
    interval_ms: AtomicU64,
}

pub struct Preview {
    shared: Arc<Shared>,
}

impl Preview {
    pub fn start(source: CaptureSource) -> Preview {
        let shared = Arc::new(Shared::default());
        *shared.source.lock().unwrap() = Some(source);
        shared.interval_ms.store(330, Ordering::Relaxed);
        let s = shared.clone();
        let _ = std::thread::Builder::new().name("screen-preview".into()).spawn(move || {
            while !s.stop.load(Ordering::Relaxed) {
                let src = s.source.lock().unwrap_or_else(|e| e.into_inner()).clone();
                if let Some(f) = src.and_then(|src| grab(&src, PREVIEW_MAX)) {
                    *s.frame.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
                    s.generation.fetch_add(1, Ordering::Release);
                }
                std::thread::sleep(Duration::from_millis(s.interval_ms.load(Ordering::Relaxed)));
            }
        });
        Preview { shared }
    }
    pub fn set_source(&self, source: CaptureSource) {
        *self.shared.source.lock().unwrap_or_else(|e| e.into_inner()) = Some(source);
    }
    /// Slower while recording: the preview must never compete with the encoder.
    pub fn set_interval(&self, ms: u64) {
        self.shared.interval_ms.store(ms.max(50), Ordering::Relaxed);
    }
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Acquire)
    }
    pub fn frame(&self) -> Option<Frame> {
        self.shared.frame.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

/// `(w, h)` scaled so the longest side is at most `max`, keeping the aspect (never upscaled).
pub(crate) fn fit(w: i32, h: i32, max: u32) -> (i32, i32) {
    let (w, h) = (w.max(1), h.max(1));
    let k = (max as f64 / w.max(h) as f64).min(1.0);
    (((w as f64 * k).round() as i32).max(1), ((h as f64 * k).round() as i32).max(1))
}

/// One downscaled grab of `src`. None when the window is gone or minimised.
pub fn grab(src: &CaptureSource, max: u32) -> Option<Frame> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    unsafe {
        // (source window or the screen, source rect in that DC)
        let (hwnd, x, y, w, h) = match src {
            CaptureSource::Desktop => (
                HWND::default(),
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            ),
            CaptureSource::Region(x, y, w, h) => (HWND::default(), *x, *y, *w as i32, *h as i32),
            CaptureSource::Window(title) => {
                let hwnd = find_window(title)?;
                let mut r = RECT::default();
                GetClientRect(hwnd, &mut r).ok()?;
                (hwnd, 0, 0, r.right - r.left, r.bottom - r.top)
            }
        };
        if w <= 0 || h <= 0 {
            return None;
        }
        let (dw, dh) = fit(w, h, max);
        let src_dc = GetDC(Some(hwnd));
        if src_dc.is_invalid() {
            return None;
        }
        let mem = CreateCompatibleDC(Some(src_dc));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: dw,
                biHeight: -dh, // top-down
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
                SetStretchBltMode(mem, HALFTONE);
                let ok = StretchBlt(mem, 0, 0, dw, dh, Some(src_dc), x, y, w, h, SRCCOPY).as_bool();
                let px = (dw * dh) as usize;
                let frame = ok.then(|| {
                    let bgra = std::slice::from_raw_parts(bits as *const u8, px * 4);
                    let mut rgba = Vec::with_capacity(px * 4);
                    for p in bgra.chunks_exact(4) {
                        rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
                    }
                    Frame { size: [dw as usize, dh as usize], rgba }
                });
                SelectObject(mem, old);
                let _ = DeleteObject(dib.into());
                frame
            }
            _ => None,
        };
        let _ = DeleteDC(mem);
        ReleaseDC(Some(hwnd), src_dc);
        out
    }
}

/// Top-level windows worth recording: visible, titled, not minimised, not cloaked (hidden UWP shells),
/// not tool windows. Titles, in Z order (frontmost first).
pub fn windows() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for_each_window(|_, title| {
        if !out.iter().any(|t| t == title) {
            out.push(title.to_string());
        }
        false
    });
    out
}

/// The first recordable window with exactly this title (what gdigrab's `title=` picks too).
fn find_window(title: &str) -> Option<windows::Win32::Foundation::HWND> {
    let mut found = None;
    for_each_window(|h, t| {
        if t == title {
            found = Some(h);
            return true;
        }
        false
    });
    found
}

/// Calls `f(hwnd, title)` for each recordable window until it returns true.
fn for_each_window(mut f: impl FnMut(windows::Win32::Foundation::HWND, &str) -> bool) {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::UI::WindowsAndMessaging::*;
    type Cb<'a> = &'a mut dyn FnMut(HWND, &str) -> bool;
    unsafe extern "system" fn each(h: HWND, data: LPARAM) -> BOOL {
        let f = unsafe { &mut *(data.0 as *mut Cb) };
        unsafe {
            if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
                return true.into();
            }
            if GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
                return true.into();
            }
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4);
            if cloaked != 0 {
                return true.into();
            }
            let mut buf = [0u16; 256];
            let n = GetWindowTextW(h, &mut buf);
            if n <= 0 {
                return true.into();
            }
            let title = String::from_utf16_lossy(&buf[..n as usize]);
            (!f(h, title.trim_end())).into()
        }
    }
    let mut cb: Cb = &mut f;
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut cb as *mut Cb as isize));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_aspect_and_never_upscales() {
        assert_eq!(fit(3840, 2160, 360), (360, 203));
        assert_eq!(fit(1080, 1920, 360), (203, 360));
        assert_eq!(fit(200, 100, 360), (200, 100));
        assert_eq!(fit(0, 0, 360), (1, 1));
    }

    /// Needs a desktop session: a grab of the top-left corner comes back small and opaque.
    #[test]
    fn grab_region_is_small() {
        let Some(f) = grab(&CaptureSource::Region(0, 0, 1280, 720), PREVIEW_MAX) else { return };
        assert_eq!(f.size, [360, 203]);
        assert_eq!(f.rgba.len(), 360 * 203 * 4);
        assert!(f.rgba.chunks(4).all(|p| p[3] == 255));
    }
}
