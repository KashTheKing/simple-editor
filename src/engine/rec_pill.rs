//! Hide on Record: a tiny always-on-top Stop pill and a global stop hotkey, on their own Win32 thread.
//!
//! When Simple Editor hides itself to record, its egui windows are minimised and eframe may not run
//! frames, so the way back can't be an egui window: a plain Win32 popup (Stop button + timer) and
//! `RegisterHotKey` share one message loop here. The pill is excluded from every capture
//! (`WDA_EXCLUDEFROMCAPTURE`), so it never shows in the recording. Stop - button or hotkey - restores
//! the hidden windows itself (Win32, no egui frame needed), sets the flag and wakes the app, which
//! then stops the recording as usual. Without Hide on Record the thread keeps only the hotkey.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// A parsed hotkey: Win32 MOD_* flags and a virtual-key code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub mods: u32,
    pub vk: u32,
}

/// "Ctrl+Shift+F9" → a hotkey (Ctrl, Shift, Alt, Win + F1-F24, A-Z, 0-9). None when it doesn't parse
/// or has no modifier (a bare key would be stolen from every other app).
pub fn parse_hotkey(s: &str) -> Option<Hotkey> {
    let mut mods = 0u32;
    let mut vk = None;
    for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= 0x2,
            "shift" => mods |= 0x4,
            "alt" => mods |= 0x1,
            "win" => mods |= 0x8,
            k => {
                if vk.is_some() {
                    return None;
                }
                vk = Some(match k.as_bytes() {
                    [b'f', rest @ ..] if !rest.is_empty() => {
                        let n: u32 = k[1..].parse().ok()?;
                        if !(1..=24).contains(&n) {
                            return None;
                        }
                        0x70 + n - 1 // VK_F1..
                    }
                    [c] if c.is_ascii_alphanumeric() => c.to_ascii_uppercase() as u32,
                    _ => return None,
                });
            }
        }
    }
    (mods != 0).then_some(Hotkey { mods, vk: vk? })
}

/// The running pill / hotkey thread. Dropping it restores whatever it hid and ends the thread.
pub struct Pill {
    stop: Arc<AtomicBool>,
    hwnd: Arc<AtomicIsize>,
    thread: Option<std::thread::JoinHandle<()>>,
    hidden: Vec<isize>,
    /// Why the hotkey couldn't be registered (another app owns it), if so.
    pub hotkey_error: Option<String>,
}

impl Pill {
    /// `hide`: our windows to minimise and keep out of the capture (empty = only the hotkey, no pill).
    pub fn start(wake: impl Fn() + Send + 'static, hotkey: Option<Hotkey>, hide: Vec<isize>) -> Pill {
        let stop = Arc::new(AtomicBool::new(false));
        let hwnd = Arc::new(AtomicIsize::new(0));
        if !hide.is_empty() {
            hide_windows(&hide);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let (s, h, hidden, show) = (stop.clone(), hwnd.clone(), hide.clone(), !hide.is_empty());
        let thread = std::thread::Builder::new()
            .name("rec-pill".into())
            .spawn(move || run(s, h, hotkey, show, hidden, Box::new(wake), tx))
            .ok();
        let hotkey_error = rx.recv_timeout(std::time::Duration::from_secs(2)).ok().flatten();
        Pill { stop, hwnd, thread, hidden: hide, hotkey_error }
    }
    /// The pill's Stop or the hotkey was pressed.
    pub fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
    /// The pill window (0 when there is none).
    #[cfg(test)]
    pub fn window(&self) -> isize {
        self.hwnd.load(Ordering::SeqCst)
    }
}

impl Drop for Pill {
    fn drop(&mut self) {
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
        let h = self.hwnd.load(Ordering::SeqCst);
        if h != 0 {
            unsafe {
                let _ = PostMessageW(Some(HWND(h as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        restore_windows(&self.hidden);
    }
}

/// Our process's visible top-level windows (the editor and any popped-out panes).
pub fn our_windows() -> Vec<isize> {
    let me = std::process::id();
    let mut v = Vec::new();
    crate::engine::screen_grab::for_each_window(|h, _| {
        if crate::engine::wgc::window_pid(h.0 as isize) == me {
            v.push(h.0 as isize);
        }
        false
    });
    v
}

/// Keep `hwnds` out of every screen capture and minimise them.
pub fn hide_windows(hwnds: &[isize]) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::*;
    for &h in hwnds {
        let h = HWND(h as *mut _);
        unsafe {
            let _ = SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE);
            // async: the window belongs to the UI thread, and this may run on the pill's
            let _ = ShowWindowAsync(h, SW_MINIMIZE);
        }
    }
}

/// Undo `hide_windows`.
pub fn restore_windows(hwnds: &[isize]) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::*;
    for &h in hwnds.iter().rev() {
        let h = HWND(h as *mut _);
        unsafe {
            if IsWindow(Some(h)).as_bool() {
                let _ = SetWindowDisplayAffinity(h, WDA_NONE);
                if IsIconic(h).as_bool() {
                    // async: never wait on a thread that is busy (or blocked waiting on us)
                    let _ = ShowWindowAsync(h, SW_RESTORE);
                }
            }
        }
    }
    if let Some(&main) = hwnds.first() {
        unsafe {
            let _ = SetForegroundWindow(HWND(main as *mut _));
        }
    }
}

struct State {
    stop: Arc<AtomicBool>,
    wake: Box<dyn Fn() + Send>,
    hidden: Vec<isize>,
    start: Instant,
    label: isize,
}

const ID_STOP: usize = 1;
const TIMER: usize = 7;
const HOTKEY_ID: i32 = 0x5E01;

fn press(st: &State) {
    if !st.stop.swap(true, Ordering::SeqCst) {
        restore_windows(&st.hidden);
        (st.wake)();
    }
}

unsafe extern "system" fn wndproc(
    h: windows::Win32::Foundation::HWND,
    msg: u32,
    w: windows::Win32::Foundation::WPARAM,
    l: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::{HWND, LRESULT};
    use windows::Win32::UI::WindowsAndMessaging::*;
    let st = unsafe { GetWindowLongPtrW(h, GWLP_USERDATA) } as *const State;
    match msg {
        WM_COMMAND if w.0 & 0xffff == ID_STOP && !st.is_null() => {
            press(unsafe { &*st });
            LRESULT(0)
        }
        WM_HOTKEY if !st.is_null() => {
            press(unsafe { &*st });
            LRESULT(0)
        }
        WM_TIMER if !st.is_null() => {
            let st = unsafe { &*st };
            let s = st.start.elapsed().as_secs();
            let text: Vec<u16> = format!("● REC {}:{:02}", s / 60, s % 60).encode_utf16().chain([0]).collect();
            unsafe {
                let _ = SetWindowTextW(HWND(st.label as *mut _), windows::core::PCWSTR(text.as_ptr()));
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(h);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(h, msg, w, l) },
    }
}

fn run(
    stop: Arc<AtomicBool>,
    hwnd_out: Arc<AtomicIsize>,
    hotkey: Option<Hotkey>,
    show: bool,
    hidden: Vec<isize>,
    wake: Box<dyn Fn() + Send>,
    ready: std::sync::mpsc::Sender<Option<String>>,
) {
    use windows::core::w;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT};
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe {
        // no module handle needed: a null instance registers the class for this process's exe
        let inst = windows::Win32::Foundation::HMODULE::default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            lpszClassName: w!("SimpleEditorRecPill"),
            hbrBackground: windows::Win32::Graphics::Gdi::GetSysColorBrush(windows::Win32::Graphics::Gdi::SYS_COLOR_INDEX(15)),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&class); // already registered by an earlier recording: fine
        let (pw, ph) = (210, 40);
        let x = (GetSystemMetrics(SM_CXSCREEN) - pw) / 2;
        let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW;
        let style = WS_POPUP | WS_BORDER | if show { WS_VISIBLE } else { WINDOW_STYLE(0) };
        let Ok(h) = CreateWindowExW(ex, w!("SimpleEditorRecPill"), w!("Simple Editor recording"), style, x, 12, pw, ph, None, None, Some(inst.into()), None) else {
            let _ = ready.send(Some("the stop pill couldn't be created".into()));
            return;
        };
        let _ = SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE);
        let label = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("● REC 0:00"),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x0200), // SS_CENTERIMAGE
            10,
            8,
            110,
            24,
            Some(h),
            None,
            Some(inst.into()),
            None,
        )
        .unwrap_or_default();
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!("Stop"),
            WS_CHILD | WS_VISIBLE,
            124,
            6,
            76,
            28,
            Some(h),
            Some(HMENU(ID_STOP as *mut _)),
            Some(inst.into()),
            None,
        );
        let st = Box::new(State { stop, wake, hidden, start: Instant::now(), label: label.0 as isize });
        SetWindowLongPtrW(h, GWLP_USERDATA, &*st as *const State as isize);
        let _ = SetTimer(Some(h), TIMER, 500, None);
        let err = match hotkey {
            Some(k) => RegisterHotKey(Some(h), HOTKEY_ID, HOT_KEY_MODIFIERS(k.mods) | MOD_NOREPEAT, k.vk)
                .err()
                .map(|_| "The stop hotkey is taken by another app.".to_string()),
            None => None,
        };
        hwnd_out.store(h.0 as isize, Ordering::SeqCst);
        let _ = ready.send(err);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnregisterHotKey(Some(h), HOTKEY_ID);
        hwnd_out.store(0, Ordering::SeqCst);
        let _ = HWND::default();
        drop(st);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::*;

    #[test]
    fn hotkeys_parse() {
        assert_eq!(parse_hotkey("Ctrl+Shift+F9"), Some(Hotkey { mods: 0x6, vk: 0x78 }));
        assert_eq!(parse_hotkey(" alt + r "), Some(Hotkey { mods: 0x1, vk: b'R' as u32 }));
        assert_eq!(parse_hotkey("Win+Ctrl+0"), Some(Hotkey { mods: 0xA, vk: b'0' as u32 }));
        assert_eq!(parse_hotkey("F9"), None, "a bare key would be taken from every app");
        assert_eq!(parse_hotkey("Ctrl+F25"), None);
        assert_eq!(parse_hotkey("Ctrl+A+B"), None);
        assert_eq!(parse_hotkey(""), None);
    }

    /// Let this thread's windows handle what was posted to them (the async show / hide).
    fn pump() {
        unsafe {
            let mut m = MSG::default();
            for _ in 0..20 {
                while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&m);
                    DispatchMessageW(&m);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }

    /// A stand-in for an editor window: a plain visible top-level window of this process.
    fn test_window() -> HWND {
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!("se-pill-test"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                -2000,
                -2000,
                200,
                100,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        }
    }

    /// Hide on Record: the windows are minimised and kept out of the capture while the pill runs; the
    /// hotkey (simulated: WM_HOTKEY to the pill) asks to stop, wakes the app and brings them back.
    #[test]
    fn hide_then_hotkey_stops_and_restores() {
        let win = test_window();
        let woke = Arc::new(AtomicBool::new(false));
        let w2 = woke.clone();
        let pill = Pill::start(move || w2.store(true, Ordering::SeqCst), parse_hotkey("Ctrl+Shift+F23"), vec![win.0 as isize]);
        pump();
        unsafe {
            assert!(IsIconic(win).as_bool(), "hidden while recording");
            let mut aff = 0u32;
            let _ = GetWindowDisplayAffinity(win, &mut aff);
            assert_eq!(aff, WDA_EXCLUDEFROMCAPTURE.0, "kept out of the capture");
            let mut paff = 0u32;
            let _ = GetWindowDisplayAffinity(HWND(pill.window() as *mut _), &mut paff);
            assert_eq!(paff, WDA_EXCLUDEFROMCAPTURE.0, "the pill never shows in the recording");
        }
        assert!(!pill.stop_requested());
        unsafe {
            PostMessageW(Some(HWND(pill.window() as *mut _)), WM_HOTKEY, WPARAM(HOTKEY_ID as usize), LPARAM(0)).unwrap();
        }
        for _ in 0..50 {
            if pill.stop_requested() {
                break;
            }
            pump();
        }
        pump();
        assert!(pill.stop_requested(), "the hotkey stops the recording");
        assert!(woke.load(Ordering::SeqCst), "and wakes the app");
        unsafe {
            assert!(!IsIconic(win).as_bool(), "restored on stop");
            let mut aff = 1u32;
            let _ = GetWindowDisplayAffinity(win, &mut aff);
            assert_eq!(aff, WDA_NONE.0);
        }
        drop(pill);
        unsafe {
            let _ = DestroyWindow(win);
        }
    }
}
