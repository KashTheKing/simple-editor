//! The Screen Recording pane's live preview and window list.
//!
//! The preview shows what the recorder records, cursor included, at 30 fps:
//! * while recording, the recording's own ffmpeg writes a second output - the captured frames scaled to
//!   `PW`x`PH` (fast bilinear, letterboxed) as raw RGBA on its stdout - so the preview IS the capture,
//!   with no second screen grab;
//! * before recording, a small ffmpeg with the same input does only that (`Preview::start`);
//! * an application source comes from `engine::wgc` (GPU-downscaled frames).
//! Frames land in a `FrameSlot`; the pane uploads the newest one into one reused texture. When nobody
//! looks (`wanted` off: pane hidden, window minimised, or Pause preview), frames are drained and
//! dropped without a copy, and the before-recording ffmpeg is not running at all.

use crate::engine::capture::CaptureSource;
use crate::media::ffpipe;
use std::io::Read;
use std::process::{Child, ChildStdout, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Longest side of an application preview frame (WGC halves down to this).
pub const PREVIEW_MAX: u32 = 480;
/// Size of the ffmpeg preview frames.
pub const PW: usize = 480;
pub const PH: usize = 270;
/// Preview rate.
pub const PREVIEW_FPS: u32 = 30;

/// A frame: RGBA, `size` = [w, h].
#[derive(Clone)]
pub struct Frame {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

/// The newest preview frame, shared between a producer thread and the pane.
#[derive(Default)]
pub struct FrameSlot {
    frame: Mutex<Option<(Frame, Instant)>>,
    generation: AtomicU64,
    /// Someone is looking: producers skip copying (and WGC skips the readback) while this is off.
    pub wanted: AtomicBool,
}

impl FrameSlot {
    pub fn new(wanted: bool) -> Arc<FrameSlot> {
        let s = Arc::new(FrameSlot::default());
        s.wanted.store(wanted, Ordering::Relaxed);
        s
    }
    pub fn put(&self, f: Frame) {
        *self.frame.lock().unwrap_or_else(|e| e.into_inner()) = Some((f, Instant::now()));
        self.generation.fetch_add(1, Ordering::Release);
    }
    /// Bumped per stored frame.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
    /// The newest frame and when it arrived.
    pub fn frame(&self) -> Option<(Frame, Instant)> {
        self.frame.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// ffmpeg output args for preview frames on stdout (after the inputs; `-map 0:v` picks the capture).
/// Every captured frame goes straight through (no `fps` filter, timestamps passed through): a rate
/// filter and constant-rate pacing held ~3 frames back (119 ms measured at 30 fps).
pub fn preview_output_args() -> Vec<String> {
    let vf = format!(
        "scale={PW}:{PH}:force_original_aspect_ratio=decrease:flags=fast_bilinear,\
         pad={PW}:{PH}:(ow-iw)/2:(oh-ih)/2,format=rgba"
    );
    ["-map", "0:v", "-vf", &vf, "-fps_mode", "passthrough", "-f", "rawvideo", "-flush_packets", "1", "pipe:1"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Read `PW`x`PH` RGBA frames from an ffmpeg stdout into `slot` until the pipe closes. Always drains
/// (a full pipe would stall the recording); copies only while `slot.wanted`.
pub fn spawn_reader(mut out: ChildStdout, slot: Arc<FrameSlot>) {
    let _ = std::thread::Builder::new().name("preview-pipe".into()).spawn(move || {
        let mut buf = vec![0u8; PW * PH * 4];
        while out.read_exact(&mut buf).is_ok() {
            if slot.wanted.load(Ordering::Relaxed) {
                slot.put(Frame { size: [PW, PH], rgba: buf.clone() });
            }
        }
    });
}

/// The before-recording preview: a preview-only ffmpeg (screen, window, region) or a WGC capture
/// (application). Dropping it stops it.
pub struct Preview {
    pub source: CaptureSource,
    pub cursor: bool,
    pub slot: Arc<FrameSlot>,
    child: Option<Child>,
    app: Option<crate::engine::wgc::AppCapture>,
}

impl Preview {
    pub fn start(source: CaptureSource, cursor: bool) -> Option<Preview> {
        let slot = FrameSlot::new(true);
        let mut p = Preview { source: source.clone(), cursor, slot: slot.clone(), child: None, app: None };
        // a screen (or a region inside one) previews through WGC too: the GPU crops and halves it, so a
        // 1440p screen at 30 fps costs what a 480 px frame costs (a gdigrab preview took ~80 % of a core)
        use crate::engine::wgc::{AppCapture, Target};
        let monitor = match &source {
            CaptureSource::Region(x, y, w, h) => crate::engine::wgc::monitor_holding([*x, *y, *w as i32, *h as i32]),
            _ => None,
        };
        match (&source, monitor) {
            (_, Some((m, crop))) => {
                p.app = AppCapture::preview(Target::Monitor(m), Some(crop), cursor, PREVIEW_FPS, slot).ok();
            }
            (CaptureSource::App { hwnd, .. }, _) => {
                p.app = AppCapture::preview(Target::Window(*hwnd), None, cursor, PREVIEW_FPS, slot).ok();
            }
            _ => {
                let exe = ffpipe::ffmpeg_exe()?;
                let mut args: Vec<String> = ["-hide_banner", "-nostats", "-loglevel", "error"].iter().map(|s| s.to_string()).collect();
                args.extend(crate::engine::capture::gdigrab_input(&source, PREVIEW_FPS, cursor));
                args.extend(preview_output_args());
                let mut child = ffpipe::command(&exe)
                    .args(&args)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .ok()?;
                crate::engine::html::tie_to_us(&child);
                spawn_reader(child.stdout.take()?, slot);
                p.child = Some(child);
            }
        }
        Some(p)
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        if let Some(c) = self.child.as_mut() {
            crate::engine::html::kill_tree(c);
            let _ = c.wait();
        }
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

/// Calls `f(hwnd, title)` for each recordable window until it returns true.
pub(crate) fn for_each_window(mut f: impl FnMut(windows::Win32::Foundation::HWND, &str) -> bool) {
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
    use std::time::Duration;

    #[test]
    fn preview_output_is_fixed_size_raw_rgba() {
        let j = preview_output_args().join(" ");
        assert!(j.starts_with("-map 0:v -vf scale=480:270:force_original_aspect_ratio=decrease"), "{j}");
        assert!(j.contains("pad=480:270") && j.ends_with("-fps_mode passthrough -f rawvideo -flush_packets 1 pipe:1"), "{j}");
    }

    /// A region inside a screen (WGC; gdigrab where WGC fails): the idle preview runs at about 30 fps; with `wanted` off it
    /// stores nothing at all, and it picks up again when someone looks.
    #[test]
    fn idle_preview_is_live_and_pauses() {
        if ffpipe::ffmpeg_exe().is_none() {
            return;
        }
        let Some(p) = Preview::start(CaptureSource::Region(0, 0, 640, 360), true) else { return };
        std::thread::sleep(Duration::from_millis(1500)); // ffmpeg start-up
        let g0 = p.slot.generation();
        std::thread::sleep(Duration::from_millis(2000));
        let fps = (p.slot.generation() - g0) as f64 / 2.0;
        assert!(fps >= 20.0, "{fps} preview fps");
        p.slot.wanted.store(false, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(200));
        let g1 = p.slot.generation();
        std::thread::sleep(Duration::from_millis(1000));
        assert_eq!(p.slot.generation(), g1, "paused: no frame stored");
        p.slot.wanted.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(500));
        assert!(p.slot.generation() > g1, "unpaused: frames again");
        let (f, _) = p.slot.frame().unwrap();
        // a region inside a screen previews through WGC: the GPU-halved crop (640x360 -> 320x180)
        assert_eq!(f.size, [320, 180]);
    }
    /// A small window in the top-left corner whose colour the bench flips (white <-> black).
    struct Flash {
        hwnd: windows::Win32::Foundation::HWND,
    }

    impl Flash {
        fn new() -> Flash {
            use windows::Win32::UI::WindowsAndMessaging::*;
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                    windows::core::w!("STATIC"),
                    windows::core::w!(""),
                    WS_POPUP | WS_VISIBLE,
                    0,
                    0,
                    160,
                    160,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap()
            };
            Flash { hwnd }
        }
        fn paint(&self, white: bool) {
            use windows::Win32::Graphics::Gdi::*;
            unsafe {
                let dc = GetDC(Some(self.hwnd));
                let brush = GetStockObject(if white { WHITE_BRUSH } else { BLACK_BRUSH });
                let r = windows::Win32::Foundation::RECT { left: 0, top: 0, right: 160, bottom: 160 };
                FillRect(dc, &r, HBRUSH(brush.0));
                ReleaseDC(Some(self.hwnd), dc);
                let _ = GdiFlush();
            }
        }
        fn pump(&self) {
            use windows::Win32::UI::WindowsAndMessaging::*;
            unsafe {
                let mut m = MSG::default();
                while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&m);
                    DispatchMessageW(&m);
                }
            }
        }
    }

    impl Drop for Flash {
        fn drop(&mut self) {
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.hwnd);
            }
        }
    }

    /// Mean brightness of the frame's centre (the flash fills it).
    fn centre_brightness(f: &Frame) -> f32 {
        let [w, h] = f.size;
        let (cx, cy) = (w / 2, h / 2);
        let mut sum = 0u32;
        for y in cy - 4..cy + 4 {
            for x in cx - 4..cx + 4 {
                let i = (y * w + x) * 4;
                sum += f.rgba[i] as u32;
            }
        }
        sum as f32 / 64.0
    }

    /// Flip the flash `n` times; per flip, the time until a stored frame shows the new colour.
    fn lag_ms(slot: &FrameSlot, flash: &Flash, n: usize) -> Vec<f64> {
        let mut out = Vec::new();
        let mut white = false;
        for _ in 0..n {
            white = !white;
            let t = Instant::now();
            flash.paint(white);
            let until = t + std::time::Duration::from_secs(2);
            while Instant::now() < until {
                flash.pump();
                if let Some((f, at)) = slot.frame() {
                    if at > t && (centre_brightness(&f) > 128.0) == white {
                        out.push(at.duration_since(t).as_secs_f64() * 1000.0);
                        break;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
            flash.paint(white);
        }
        out
    }

    fn report(name: &str, slot: &FrameSlot, flash: &Flash) {
        let g0 = slot.generation();
        let t0 = Instant::now();
        while t0.elapsed() < std::time::Duration::from_secs(3) {
            flash.pump();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let fps = (slot.generation() - g0) as f64 / t0.elapsed().as_secs_f64();
        let mut lags = lag_ms(slot, flash, 10);
        lags.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let med = lags.get(lags.len() / 2).copied().unwrap_or(f64::NAN);
        let worst = lags.last().copied().unwrap_or(f64::NAN);
        println!("{name}: {fps:.1} fps, capture-to-frame lag median {med:.0} ms, worst {worst:.0} ms ({} of 10 flips seen)", lags.len());
    }

    /// `cargo test preview_bench -- --ignored --nocapture`: flashes a 160 px square in the top-left
    /// corner for ~15 s and reports each preview path's fps and capture-to-frame lag (add up to one
    /// 33 ms UI frame for the time on screen).
    #[test]
    #[ignore]
    fn preview_bench() {
        let flash = Flash::new();
        flash.paint(false);
        let p = Preview::start(CaptureSource::Region(0, 0, 160, 160), true).expect("ffmpeg preview");
        std::thread::sleep(std::time::Duration::from_millis(1500));
        report("screen (ffmpeg gdigrab, 30 fps)", &p.slot, &flash);
        drop(p);
        let slot = FrameSlot::new(true);
        let app = crate::engine::wgc::AppCapture::preview(crate::engine::wgc::Target::Window(flash.hwnd.0 as isize), None, true, PREVIEW_FPS, slot.clone()).expect("wgc");
        std::thread::sleep(std::time::Duration::from_millis(500));
        report("application (WGC, 30 fps)", &slot, &flash);
        drop(app);
    }
}
