//! Window placement: open on the monitor the user is actually on.
//!
//! eframe persists the window rect and its stored position always wins over `ViewportBuilder::position`
//! (epi_integration applies `WindowSettings::initialize_viewport_builder` last), so without this the app
//! reopens on whichever monitor it was closed on. At startup we move the window onto the monitor under the
//! mouse cursor - keeping the persisted size, and keeping the persisted position when it is already on
//! that monitor (so a deliberate arrangement is never disturbed).

use crate::settings::Settings;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::core::BOOL;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CLOAK, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER,
};

/// `SE_BACKGROUND=1`: automation runs (screenshots, docs, agents) open where the saved `window_rect`
/// says and never take focus - no move to the cursor's monitor, no activation. Interactive launches
/// are unaffected.
pub fn background() -> bool {
    std::env::var_os("SE_BACKGROUND").is_some_and(|v| v != "0")
}

/// Quiet launch, for agents and test instances: no What's New window, no welcome wizard, no "Recover
/// unsaved project?" prompt - and nothing they would have recorded (last seen version, onboarded) is
/// written, so the user's own next normal launch still gets them. On with `--quiet` (main sets
/// `SE_QUIET`), `SE_QUIET=1`, or `SE_BACKGROUND=1` (every automation launch already sets that) - and
/// always for a build run from a development worktree (`…\simple-editor-wt\…`), so agents' test windows
/// stay quiet even when a launch forgets the flag. `SE_QUIET=0` turns it off for such a build.
pub fn quiet() -> bool {
    match std::env::var_os("SE_QUIET") {
        Some(v) => v != "0",
        None => background() || std::env::current_exe().is_ok_and(|p| is_dev_build(&p)),
    }
}

/// A build inside a development worktree (`…\simple-editor-wt\<name>\target\…`), not the installed exe.
pub fn is_dev_build(exe: &std::path::Path) -> bool {
    exe.components().any(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case("simple-editor-wt"))
}

/// How long a window rect must sit still before it's worth a settings.json write.
const DEBOUNCE: Duration = Duration::from_millis(500);

pub fn hwnd_of(handle: &impl HasWindowHandle) -> Option<HWND> {
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

/// The rect can't sit inside the work area as a normal window.
fn too_big(area: RECT, w: i32, h: i32) -> bool {
    w >= area.right - area.left || h >= area.bottom - area.top
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
/// Call once, from `App::new` - before the first frame is painted, so there is no visible jump.
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

/// Seed a `ViewportBuilder`'s position/size from a saved window rect (`Settings::window_rect`), if any -
/// replaces eframe's removed `persistence` feature for the window rect specifically. `rect` is
/// `[x, y, w, h]` in physical pixels; a missing/degenerate rect (width or height <= 0) leaves the
/// builder's own defaults untouched.
pub fn apply_rect(vb: eframe::egui::ViewportBuilder, rect: Option<[i32; 4]>) -> eframe::egui::ViewportBuilder {
    match rect {
        Some([x, y, w, h]) if w > 0 && h > 0 => {
            // a maximized window's saved rect overhangs the work area (taskbar, borders); restored as a
            // normal window it would hide its bottom (transports) and right edge off-screen
            let area = work_area(POINT { x: x + w / 2, y: y + h / 2 });
            if area.is_some_and(|a| too_big(a, w, h)) {
                return start_maximized(vb.with_position([x as f32, y as f32]));
            }
            vb.with_position([x as f32, y as f32]).with_inner_size([w as f32, h as f32])
        }
        _ => vb,
    }
}

/// `Settings::start_maximized`: open maximised on the monitor under the mouse. The position only picks
/// the monitor (a window maximises on the monitor it is created on); `place_on_cursor_monitor` then
/// finds the window already there and leaves it alone.
pub fn maximized_on_cursor_monitor(vb: eframe::egui::ViewportBuilder) -> eframe::egui::ViewportBuilder {
    let mut cursor = POINT::default();
    let area = if unsafe { GetCursorPos(&mut cursor) }.is_ok() { work_area(cursor) } else { None };
    match area {
        Some(a) => start_maximized(vb.with_position([a.left as f32, a.top as f32])),
        None => start_maximized(vb),
    }
}

/// Maximise the window when it is first shown (`Reveal`), not at creation: winit maximises with
/// `ShowWindow(SW_MAXIMIZE)` right after `CreateWindowEx`, which shows the window whatever
/// `with_visible(false)` says - a blank white window before the first frame (the launch flash).
static START_MAXIMIZED: AtomicBool = AtomicBool::new(false);

fn start_maximized(vb: eframe::egui::ViewportBuilder) -> eframe::egui::ViewportBuilder {
    START_MAXIMIZED.store(true, Ordering::Relaxed);
    vb
}

fn cloak(handle: &impl HasWindowHandle, on: bool) -> bool {
    let Some(hwnd) = hwnd_of(handle) else { return false };
    let v: i32 = on.into(); // a Win32 BOOL
    unsafe {
        DwmSetWindowAttribute(hwnd, DWMWA_CLOAK, &v as *const i32 as *const _, std::mem::size_of::<i32>() as u32).is_ok()
    }
}

/// Windows 11's window frame: rounded corners (DWM clips the client to them and keeps its shadow) and the
/// outline's colour (`None`: no outline). Windows 10 has neither attribute: the calls fail, it stays as is.
/// True when DWM draws the outline (Windows 11, round corners: it draws none round a square window).
pub fn style_frame(hwnd: HWND, round: bool, border: Option<[u8; 3]>) -> bool {
    const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;
    let corner = if round { DWMWCP_ROUND } else { DWMWCP_DONOTROUND };
    // a COLORREF: 0x00BBGGRR
    let color = border.map_or(DWMWA_COLOR_NONE, |[r, g, b]| u32::from(r) | u32::from(g) << 8 | u32::from(b) << 16);
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corner as *const _ as *const _, 4);
        DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &color as *const u32 as *const _, 4).is_ok() && round
    }
}

/// Moves on the OS's own fast path. Inside a move / size loop (a title-bar drag, `StartDrag`, Alt+Space
/// Move) every mouse step is a `WM_WINDOWPOSCHANGED`, which winit turns into `Moved` and eframe into a
/// whole UI frame - a debug build's 30+ ms frame per step is the lag of a dragged window. DWM moves the
/// already-painted window by itself, so a pure move (`SWP_NOSIZE`) inside the loop is held back from
/// winit and delivered once, when the loop ends; resizes pass through (they must repaint). Idempotent.
pub fn smooth_moves(hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowLongPtrW, GWLP_WNDPROC};
    // a window-procedure swap, not comctl32's SetWindowSubclass: this exe has no common-controls v6
    // manifest, and v5 exports that only by ordinal (the exe would not start)
    MOVES.with(|m| {
        let mut m = m.borrow_mut();
        let key = hwnd.0 as isize;
        if m.contains_key(&key) {
            return;
        }
        let prev = unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, move_proc as usize as isize) };
        if prev != 0 {
            m.insert(key, (prev, 0));
        }
    });
}

thread_local! {
    /// Per window (all on the UI thread): its own window procedure, and IN_LOOP / HELD.
    static MOVES: std::cell::RefCell<std::collections::HashMap<isize, (isize, usize)>> = Default::default();
}
const IN_LOOP: usize = 1;
const HELD: usize = 2;

unsafe extern "system" fn move_proc(
    h: HWND,
    msg: u32,
    w: windows::Win32::Foundation::WPARAM,
    l: LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, SWP_NOACTIVATE, SWP_NOSIZE, WINDOWPOS, WM_ENTERSIZEMOVE, WM_EXITSIZEMOVE,
        WM_NCDESTROY, WM_WINDOWPOSCHANGED, WNDPROC,
    };
    let key = h.0 as isize;
    // borrowed only to read / write, never across the call on (it re-enters this procedure)
    let Some((prev, state)) = MOVES.with(|m| m.borrow().get(&key).copied()) else {
        return unsafe { DefWindowProcW(h, msg, w, l) };
    };
    let set = |s: usize| MOVES.with(|m| drop(m.borrow_mut().insert(key, (prev, s))));
    let prev: WNDPROC = unsafe { std::mem::transmute::<isize, WNDPROC>(prev) };
    let call = |msg: u32, w, l| unsafe { CallWindowProcW(prev, h, msg, w, l) };
    match msg {
        WM_ENTERSIZEMOVE => set(IN_LOOP),
        WM_WINDOWPOSCHANGED if state & IN_LOOP != 0 => {
            let wp = unsafe { &*(l.0 as *const WINDOWPOS) };
            if wp.flags.contains(SWP_NOSIZE) {
                set(IN_LOOP | HELD);
                return unsafe { DefWindowProcW(h, msg, w, l) }; // WM_MOVE for Windows, nothing for winit
            }
        }
        WM_EXITSIZEMOVE => {
            set(0);
            let r = call(msg, w, l);
            let mut rect = RECT::default();
            if state & HELD != 0 && unsafe { GetWindowRect(h, &mut rect) }.is_ok() {
                // the one move winit hears about: where the window ended up
                let wp = WINDOWPOS {
                    hwnd: h,
                    x: rect.left,
                    y: rect.top,
                    cx: rect.right - rect.left,
                    cy: rect.bottom - rect.top,
                    flags: SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    ..Default::default()
                };
                call(WM_WINDOWPOSCHANGED, Default::default(), LPARAM(&wp as *const _ as isize));
            }
            return r;
        }
        WM_NCDESTROY => MOVES.with(|m| drop(m.borrow_mut().remove(&key))),
        _ => {}
    }
    call(msg, w, l)
}

/// This thread's top-level window titled `title` (eframe hands out no handle for a child viewport; they
/// all live on the UI thread). ponytail: by title, so two windows with one title share the first's frame.
pub fn thread_window(title: &str) -> Option<HWND> {
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{EnumThreadWindows, GetWindowTextW};
    struct Find<'a>(Vec<u16>, Option<HWND>, std::marker::PhantomData<&'a ()>);
    unsafe extern "system" fn each(h: HWND, l: LPARAM) -> BOOL {
        let f = unsafe { &mut *(l.0 as *mut Find) };
        let mut buf = [0u16; 512];
        let n = unsafe { GetWindowTextW(h, &mut buf) } as usize;
        if buf[..n] == f.0[..] {
            f.1 = Some(h);
            return false.into();
        }
        true.into()
    }
    let mut f = Find(title.encode_utf16().collect(), None, std::marker::PhantomData);
    unsafe {
        let _ = EnumThreadWindows(GetCurrentThreadId(), Some(each), LPARAM(&mut f as *mut Find as isize));
    }
    f.1
}

/// Showing the window without a flash: it opens hidden (`with_visible(false)`), and `step` - called at
/// the top of every update - shows it cloaked (DWM composes nothing of it), maximised when it should
/// be, lets one frame paint at that final size, and uncloaks once that frame has been presented. The
/// first thing on screen is the window at its final size, already painted.
#[derive(Default)]
pub struct Reveal {
    stage: u8,
    /// Updates spent waiting for the maximise to land.
    waited: u8,
    cloaked: bool,
}

impl Reveal {
    /// Advance; true on the one update where the window has just become visible (the frame before it
    /// is on screen).
    pub fn step(&mut self, ctx: &eframe::egui::Context, frame: &eframe::Frame, focus: bool) -> bool {
        use eframe::egui::ViewportCommand;
        let max = START_MAXIMIZED.load(Ordering::Relaxed);
        match self.stage {
            0 => {
                self.cloaked = cloak(frame, true);
                if max {
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(true));
                }
                ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                if focus {
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                self.stage = 1;
                ctx.request_repaint();
                // no cloak (an old Windows, no DWM): shown as before, after this frame is painted
                !self.cloaked
            }
            1 => {
                // this update paints at the size the window was shown at (maximised: once it says so)
                let sized = !max || ctx.input(|i| i.viewport().maximized == Some(true));
                self.waited += 1;
                if sized || self.waited > 10 {
                    self.stage = 2;
                }
                ctx.request_repaint();
                false
            }
            2 => {
                self.stage = 3;
                if self.cloaked {
                    cloak(frame, false);
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// Shown (cloaked) already: the next steps only wait for a frame worth revealing.
    pub fn started(&self) -> bool {
        self.stage > 0
    }

    pub fn revealed(&self) -> bool {
        self.stage >= 3 || (self.stage >= 1 && !self.cloaked)
    }
}

/// Debounced window-rect save, called once a frame: reads the OS-reported outer rect, (re)starts a
/// 500ms debounce whenever it has moved, and writes `settings.window_rect` + saves once it has settled.
/// Returns the next wake-up `Instant` while a debounce is still pending, `None` once settled - this fn
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
    fn background_reads_the_env_var() {
        // only App::new / main read it, never another test, so flipping it here is safe
        std::env::set_var("SE_BACKGROUND", "1");
        assert!(background());
        std::env::set_var("SE_BACKGROUND", "0");
        assert!(!background());
        std::env::remove_var("SE_BACKGROUND");
        assert!(!background());
        // quiet launch (no What's New / wizard / recover prompt): SE_QUIET, or implied by SE_BACKGROUND
        std::env::set_var("SE_QUIET", "0");
        assert!(!quiet(), "SE_QUIET=0 shows the launch pop-ups");
        std::env::set_var("SE_QUIET", "1");
        assert!(quiet());
        std::env::set_var("SE_QUIET", "0");
        assert!(!quiet());
        std::env::remove_var("SE_QUIET");
        std::env::set_var("SE_BACKGROUND", "1");
        assert!(quiet(), "every automation launch is quiet");
        std::env::remove_var("SE_BACKGROUND");
        std::env::remove_var("SE_QUIET");
        // builds run from a development worktree are quiet; the installed exe is not
        use std::path::Path;
        assert!(is_dev_build(Path::new("D:/Projects/simple-editor-wt/animator/target/debug/simple-editor.exe")));
        assert!(!is_dev_build(Path::new("D:/Projects/simple-editor/target/release/simple-editor.exe")));
    }

    #[test]
    fn a_maximized_sized_rect_is_too_big() {
        let area = r(0, 0, 2560, 1392); // 2560x1440 minus the taskbar
        assert!(too_big(area, 2576, 1479), "the saved rect of a maximized window");
        assert!(!too_big(area, 1400, 860));
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

    /// `winpos::tick` never schedules a repaint itself - see `fn tick`'s own body just above, which has
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

