//! Window placement: open on the monitor the user is actually on.
//!
//! eframe persists the window rect and its stored position always wins over `ViewportBuilder::position`
//! (epi_integration applies `WindowSettings::initialize_viewport_builder` last), so without this the app
//! reopens on whichever monitor it was closed on. At startup we move the window onto the monitor under the
//! mouse cursor — keeping the persisted size, and keeping the persisted position when it is already on
//! that monitor (so a deliberate arrangement is never disturbed).

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER,
};

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
}
