//! Window placement: open on the monitor the user is actually on.
//!
//! eframe persists the window rect and its stored position always wins over `ViewportBuilder::position`
//! (epi_integration applies `WindowSettings::initialize_viewport_builder` last), so without this the app
//! reopens on whichever monitor it was closed on. At startup we move the window onto the monitor under the
//! mouse cursor — keeping the persisted size, and keeping the persisted position when it is already on
//! that monitor (so a deliberate arrangement is never disturbed).

use crate::settings::Settings;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER,
};

/// How long a window rect must sit still before it's worth a settings.json write.
const DEBOUNCE: Duration = Duration::from_millis(500);

fn hwnd_of(handle: &impl HasWindowHandle) -> Option<HWND> {
    match handle.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut std::ffi::c_void)),
        _ => None,
    }
}

/// Work area (excludes the taskbar) of the monitor containing `p`, in physical pixels.
fn work_area(p: POINT) -> Option<RECT> {
    unsafe {
        let mon = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        GetMonitorInfoW(mon, &mut info).as_bool().then_some(info.rcWork)
    }
}

fn contains(r: RECT, p: POINT) -> bool {
    p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
}

/// New top-left for a `w`×`h` window so it sits fully inside `area`, centred when it doesn't already fit.
fn place(area: RECT, w: i32, h: i32) -> (i32, i32) {
    let (aw, ah) = (area.right - area.left, area.bottom - area.top);
    let x = area.left + ((aw - w) / 2).max(0);
    let y = area.top + ((ah - h) / 2).max(0);
    (x, y)
}

/// Move the window onto the monitor under the cursor (no-op when it is already there, or on any error).
/// Call once, from `App::new` — before the first frame is painted, so there is no visible jump.
pub fn place_on_cursor_monitor(handle: &impl HasWindowHandle) {
    let Some(hwnd) = hwnd_of(handle) else { return };
    unsafe {
        let mut cursor = POINT::default();
        if GetCursorPos(&mut cursor).is_err() {
            return;
        }
        let Some(area) = work_area(cursor) else { return };
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return;
        }
        // already on this monitor? leave the user's arrangement alone
        let centre = POINT { x: (rect.left + rect.right) / 2, y: (rect.top + rect.bottom) / 2 };
        if contains(area, centre) {
            return;
        }
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        let (x, y) = place(area, w, h);
        let _ = SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
    }
}

/// Seed a `ViewportBuilder`'s position/size from a saved window rect (`Settings::window_rect`), if any —
/// replaces eframe's removed `persistence` feature for the window rect specifically. `rect` is
/// `[x, y, w, h]` in physical pixels; a missing/degenerate rect (width or height <= 0) leaves the
/// builder's own defaults untouched.
pub fn apply_rect(vb: eframe::egui::ViewportBuilder, rect: Option<[i32; 4]>) -> eframe::egui::ViewportBuilder {
    match rect {
        Some([x, y, w, h]) if w > 0 && h > 0 => {
            vb.with_position([x as f32, y as f32]).with_inner_size([w as f32, h as f32])
        }
        _ => vb,
    }
}

/// Debounced window-rect save, called once a frame: reads the OS-reported outer rect, (re)starts a
/// 500ms debounce whenever it has moved, and writes `settings.window_rect` + saves once it has settled.
/// Returns the next wake-up `Instant` while a debounce is still pending, `None` once settled — this fn
/// schedules nothing itself (no `ctx.request_repaint_after` anywhere in it); the caller (`whatsnew::tick`,
/// which holds `&mut App`) turns a `Some` into `app.animate_until(ctx, at)`, keeping every NEW timed
/// repaint this PR adds behind that one sanctioned funnel.
pub fn tick(
    ctx: &eframe::egui::Context,
    settings: &mut Settings,
    pending: &mut Option<(Instant, [i32; 4])>,
) -> Option<Instant> {
    let rect = ctx.input(|i| i.viewport().outer_rect)?;
    let cur = [rect.min.x as i32, rect.min.y as i32, rect.width() as i32, rect.height() as i32];
    match pending {
        Some((_, last)) if *last == cur => {}
        _ => *pending = Some((Instant::now(), cur)),
    }
    let (started, cur) = (*pending)?;
    if started.elapsed() < DEBOUNCE {
        return Some(started + DEBOUNCE);
    }
    if settings.window_rect != Some(cur) {
        settings.window_rect = Some(cur);
        settings.save();
    }
    *pending = None;
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT { left, top, right, bottom }
    }

    #[test]
    fn places_inside_the_work_area() {
        let area = r(1920, 0, 3840, 1080); // a second monitor to the right
        let (x, y) = place(area, 1400, 860);
        assert!(x >= area.left && x + 1400 <= area.right);
        assert!(y >= area.top && y + 860 <= area.bottom);
        // a window larger than the monitor still starts at the top-left corner (never off-screen)
        let (x, y) = place(area, 4000, 2000);
        assert_eq!((x, y), (area.left, area.top));
    }

    #[test]
    fn contains_matches_win32_half_open_rects() {
        let area = r(0, 0, 1920, 1080);
        assert!(contains(area, POINT { x: 0, y: 0 }));
        assert!(contains(area, POINT { x: 1919, y: 1079 }));
        assert!(!contains(area, POINT { x: 1920, y: 500 }));
        assert!(!contains(area, POINT { x: -1, y: 500 }));
    }

    #[test]
    fn apply_rect_seeds_position_and_size_only_when_valid() {
        let vb = eframe::egui::ViewportBuilder::default();
        let seeded = apply_rect(vb.clone(), Some([10, 20, 800, 600]));
        assert_eq!(seeded.position, Some(eframe::egui::pos2(10.0, 20.0)));
        assert_eq!(seeded.inner_size, Some(eframe::egui::vec2(800.0, 600.0)));
        // None, and a degenerate (zero-size) rect, both leave the builder's defaults untouched
        assert_eq!(apply_rect(vb.clone(), None).position, None);
        assert_eq!(apply_rect(vb, Some([10, 20, 0, 600])).position, None);
    }

    fn raw_input_with_rect(x: f32, y: f32, w: f32, h: f32) -> eframe::egui::RawInput {
        let mut raw = eframe::egui::RawInput::default();
        let mut vp = eframe::egui::ViewportInfo::default();
        vp.outer_rect = Some(eframe::egui::Rect::from_min_size(eframe::egui::pos2(x, y), eframe::egui::vec2(w, h)));
        raw.viewports.insert(raw.viewport_id, vp);
        raw
    }

    /// `winpos::tick` never schedules a repaint itself — see `fn tick`'s own body just above, which has
    /// no `ctx.request_repaint*` call anywhere in it (verified by the PR's own `grep`, not re-checked
    /// here: a text self-scan of this very file can't search for its own search string). It only ever
    /// returns the wake-up `Instant` and lets the caller (`whatsnew::tick`, which holds `&mut App`)
    /// route it through `App::animate_until`. This test pins the observable contract: `Some` while a
    /// rect change is still within the 500ms debounce, `None` (having saved) once it has settled.
    #[test]
    fn winpos_tick_returns_instant_not_repaint() {
        let dir = std::env::temp_dir().join(format!("se-winpos-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let prev_appdata = std::env::var_os("APPDATA");
        std::env::set_var("APPDATA", &dir);

        let ctx = eframe::egui::Context::default();
        let mut settings = Settings::default();
        let mut pending = None;

        // a freshly-moved rect starts a debounce: the caller must be told to wake up, nothing saved yet
        let mut next = None;
        let _ = ctx.run(raw_input_with_rect(10.0, 10.0, 800.0, 600.0), |ctx| {
            next = tick(ctx, &mut settings, &mut pending);
        });
        assert!(next.is_some(), "a moved rect must ask the caller to wake up again");
        assert!(settings.window_rect.is_none(), "not saved until the debounce elapses");

        // same rect, but backdated past the debounce window: settle and save
        if let Some((_, last)) = pending {
            pending = Some((Instant::now() - DEBOUNCE - Duration::from_millis(1), last));
        }
        let mut next2 = Some(Instant::now());
        let _ = ctx.run(raw_input_with_rect(10.0, 10.0, 800.0, 600.0), |ctx| {
            next2 = tick(ctx, &mut settings, &mut pending);
        });
        assert!(next2.is_none(), "settled: nothing left to wait for");
        assert_eq!(settings.window_rect, Some([10, 10, 800, 600]));
        assert!(pending.is_none());

        match prev_appdata {
            Some(v) => std::env::set_var("APPDATA", v),
            None => std::env::remove_var("APPDATA"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
