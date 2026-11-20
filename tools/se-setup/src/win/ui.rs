//! One small native window: a title, a line of text, a list of checkboxes (or radio buttons) with
//! tooltips, a progress bar with a status line, and two buttons. A `Flow` decides what each page shows
//! and what the buttons do; slow work runs on a worker thread and reports back by posted messages.

use super::wide;
use crate::report::Report;
use std::cell::RefCell;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Default)]
pub struct Check {
    pub label: String,
    pub tip: String,
    pub checked: bool,
    pub enabled: bool,
    /// A radio button; consecutive radios form one group.
    pub radio: bool,
    /// A small heading shown above this row.
    pub heading: Option<String>,
}

impl Check {
    pub fn new(label: impl Into<String>, tip: impl Into<String>, checked: bool) -> Check {
        Check { label: label.into(), tip: tip.into(), checked, enabled: true, ..Default::default() }
    }
}

#[derive(Clone, Default)]
pub struct Page {
    pub title: String,
    pub body: String,
    pub checks: Vec<Check>,
    /// The default button ("Install"); None hides it.
    pub primary: Option<String>,
    pub secondary: Option<String>,
    /// Show the progress bar and status line (a job is running).
    pub busy: bool,
}

pub enum Btn {
    Primary,
    Secondary,
}

pub type Job = Box<dyn FnOnce(&Report) -> Result<String, String> + Send>;

pub enum Next {
    Show(Page),
    /// Run `job` on a worker thread while `page` shows; `Flow::done` gets its result.
    Run(Page, Job),
    Close,
}

pub trait Flow {
    fn first(&mut self) -> Next;
    /// A button was pressed; `checks` are the current boxes.
    fn button(&mut self, b: Btn, checks: &[bool]) -> Next;
    fn done(&mut self, r: Result<String, String>) -> Next;
    /// The window is closing (after any job finished).
    fn closing(&mut self) {}
}

const ID_PRIMARY: usize = 1;
const ID_SECONDARY: usize = 2;
const ID_CHECK0: usize = 100;
const WM_PROGRESS: u32 = WM_APP + 1;
const WM_DONE: u32 = WM_APP + 2;
/// Logical width of the client area (pixels at 96 DPI).
const WIDTH: i32 = 500;
const PAD: i32 = 20;
// static control styles (their windows-sys home is the large SystemServices module)
const SS_NOPREFIX: u32 = 0x80;
const SS_ENDELLIPSIS: u32 = 0x4000;

struct Shared {
    progress: Mutex<(f32, String)>,
    result: Mutex<Option<Result<String, String>>>,
    cancel: AtomicBool,
}

struct App {
    hwnd: HWND,
    flow: Box<dyn Flow>,
    page: Page,
    controls: Vec<HWND>,
    checks: Vec<HWND>,
    progress: HWND,
    status: HWND,
    primary: HWND,
    secondary: HWND,
    tooltip: HWND,
    font: HFONT,
    title_font: HFONT,
    shared: Arc<Shared>,
    running: bool,
    close_after: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn dpi(hwnd: HWND) -> i32 {
    let d = unsafe { GetDpiForWindow(hwnd) } as i32;
    if d > 0 {
        d
    } else {
        96
    }
}

fn font(size_pt: i32, weight: i32, dpi: i32) -> HFONT {
    let face = wide("Segoe UI");
    unsafe {
        CreateFontW(-(size_pt * dpi / 72), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32, 0, face.as_ptr())
    }
}

/// Run the window until it closes.
pub fn run(title: &str, flow: Box<dyn Flow>) {
    unsafe {
        let icc = INITCOMMONCONTROLSEX { dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_PROGRESS_CLASS | ICC_STANDARD_CLASSES | ICC_BAR_CLASSES };
        InitCommonControlsEx(&icc);
        let inst = GetModuleHandleW(null());
        let class = wide("SimpleEditorSetup");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: inst,
            hIcon: LoadIconW(inst, 1 as _),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as usize as HBRUSH,
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
            hIconSm: LoadIconW(inst, 1 as _),
        };
        RegisterClassExW(&wc);
        let t = wide(title);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        // created hidden; sized and centred for its DPI by the first page
        let hwnd = CreateWindowExW(0, class.as_ptr(), t.as_ptr(), style, CW_USEDEFAULT, CW_USEDEFAULT, 400, 300, null_mut(), null_mut(), inst, null());
        if hwnd.is_null() {
            return;
        }
        let d = dpi(hwnd);
        let tooltip = CreateWindowExW(
            WS_EX_TOPMOST,
            wide("tooltips_class32").as_ptr(),
            null(),
            WS_POPUP | TTS_ALWAYSTIP as u32 | TTS_NOPREFIX as u32,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            hwnd,
            null_mut(),
            inst,
            null(),
        );
        SendMessageW(tooltip, TTM_SETMAXTIPWIDTH, 0, (360 * d / 96) as LPARAM);
        let app = App {
            hwnd,
            flow,
            page: Page::default(),
            controls: Vec::new(),
            checks: Vec::new(),
            progress: null_mut(),
            status: null_mut(),
            primary: null_mut(),
            secondary: null_mut(),
            tooltip,
            font: font(9, 400, d),
            title_font: font(14, 600, d),
            shared: Arc::new(Shared { progress: Mutex::new((0.0, String::new())), result: Mutex::new(None), cancel: AtomicBool::new(false) }),
            running: false,
            close_after: false,
        };
        APP.with(|a| *a.borrow_mut() = Some(app));
        let first = with_app(|a| a.flow.first());
        if let Some(n) = first {
            apply(n);
        }
        center(hwnd);
        ShowWindow(hwnd, SW_SHOW);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn center(hwnd: HWND) {
    unsafe {
        let mut r: RECT = std::mem::zeroed();
        GetWindowRect(hwnd, &mut r);
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(mon, &mut mi);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let wa = mi.rcWork;
        SetWindowPos(hwnd, null_mut(), wa.left + (wa.right - wa.left - w) / 2, wa.top + (wa.bottom - wa.top - h) / 3, 0, 0, SWP_NOSIZE | SWP_NOZORDER);
    }
}

/// Act on what the flow asked for. Never called with APP borrowed.
fn apply(n: Next) {
    match n {
        Next::Show(p) => {
            with_app(|a| a.show(p));
        }
        Next::Run(p, job) => {
            let started = with_app(|a| {
                a.show(p);
                a.running = true;
                a.shared.cancel.store(false, Ordering::Relaxed);
                *a.shared.progress.lock().unwrap() = (0.0, String::new());
                (a.hwnd as isize, a.shared.clone())
            });
            if let Some((hwnd, sh)) = started {
                std::thread::spawn(move || {
                    // the window handle travels as an integer (raw pointers are not Send / Sync)
                    let say = |f: f32, t: &str| {
                        *sh.progress.lock().unwrap() = (f, t.to_string());
                        unsafe { PostMessageW(hwnd as HWND, WM_PROGRESS, 0, 0) };
                    };
                    let r = job(&Report::new(&say, &sh.cancel));
                    *sh.result.lock().unwrap() = Some(r);
                    unsafe { PostMessageW(hwnd as HWND, WM_DONE, 0, 0) };
                });
            }
        }
        Next::Close => {
            let h = with_app(|a| {
                a.flow.closing();
                a.hwnd
            });
            if let Some(h) = h {
                unsafe { DestroyWindow(h) };
            }
        }
    }
}

impl App {
    fn child(&mut self, class: &str, text: &str, style: u32, id: usize, f: HFONT) -> HWND {
        let c = wide(class);
        let t = wide(text);
        let h = unsafe { CreateWindowExW(0, c.as_ptr(), t.as_ptr(), WS_CHILD | WS_VISIBLE | style, 0, 0, 10, 10, self.hwnd, id as HMENU, GetModuleHandleW(null()), null()) };
        unsafe { SendMessageW(h, WM_SETFONT, f as WPARAM, 1) };
        self.controls.push(h);
        h
    }

    /// Text height for a width, in pixels.
    fn measure(&self, text: &str, f: HFONT, width: i32) -> i32 {
        unsafe {
            let dc = GetDC(self.hwnd);
            let old = SelectObject(dc, f as HGDIOBJ);
            let mut r = RECT { left: 0, top: 0, right: width, bottom: 0 };
            let w = wide(text);
            DrawTextW(dc, w.as_ptr(), -1, &mut r, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX);
            SelectObject(dc, old);
            ReleaseDC(self.hwnd, dc);
            r.bottom - r.top
        }
    }

    fn show(&mut self, p: Page) {
        // rebuild: pages change a handful of times per run
        for h in self.controls.drain(..) {
            unsafe { DestroyWindow(h) };
        }
        self.checks.clear();
        let d = dpi(self.hwnd);
        let s = |v: i32| v * d / 96;
        let width = s(WIDTH) - 2 * s(PAD);
        let x = s(PAD);
        let mut y = s(PAD);
        let (font, title_font) = (self.font, self.title_font);

        let h = self.measure(&p.title, title_font, width);
        let t = self.child("STATIC", &p.title, SS_NOPREFIX, 0, title_font);
        unsafe { MoveWindow(t, x, y, width, h, 0) };
        y += h + s(10);

        if !p.body.is_empty() {
            let h = self.measure(&p.body, font, width);
            let b = self.child("STATIC", &p.body, SS_NOPREFIX, 0, font);
            unsafe { MoveWindow(b, x, y, width, h, 0) };
            y += h + s(12);
        }

        let row = s(22);
        let mut first_radio = true;
        for (i, c) in p.checks.iter().enumerate() {
            if let Some(hd) = &c.heading {
                let l = self.child("STATIC", hd, SS_NOPREFIX, 0, font);
                unsafe { MoveWindow(l, x, y + s(4), width, s(18), 0) };
                y += s(26);
            }
            let mut style = WS_TABSTOP as u32;
            if c.radio {
                style |= BS_AUTORADIOBUTTON as u32;
                if first_radio {
                    style |= WS_GROUP;
                    first_radio = false;
                }
            } else {
                style |= BS_AUTOCHECKBOX as u32;
                first_radio = true;
            }
            let h = self.child("BUTTON", &c.label, style, ID_CHECK0 + i, font);
            unsafe {
                MoveWindow(h, x + s(4), y, width - s(4), row, 0);
                SendMessageW(h, BM_SETCHECK, if c.checked { BST_CHECKED } else { BST_UNCHECKED } as WPARAM, 0);
                EnableWindow(h, (c.enabled && !p.busy) as i32);
            }
            if !c.tip.is_empty() {
                let tip = wide(&c.tip);
                let mut ti: TTTOOLINFOW = unsafe { std::mem::zeroed() };
                ti.cbSize = std::mem::size_of::<TTTOOLINFOW>() as u32;
                ti.uFlags = TTF_IDISHWND | TTF_SUBCLASS;
                ti.hwnd = self.hwnd;
                ti.uId = h as usize;
                ti.lpszText = tip.as_ptr() as *mut u16;
                unsafe { SendMessageW(self.tooltip, TTM_ADDTOOLW, 0, &ti as *const _ as LPARAM) };
            }
            self.checks.push(h);
            y += row + s(2);
        }
        if !p.checks.is_empty() {
            y += s(10);
        }

        if p.busy {
            self.progress = self.child("msctls_progress32", "", 0, 0, font);
            unsafe {
                MoveWindow(self.progress, x, y, width, s(14), 0);
                SendMessageW(self.progress, PBM_SETRANGE32, 0, 1000);
            }
            y += s(14) + s(8);
            self.status = self.child("STATIC", "", SS_NOPREFIX | SS_ENDELLIPSIS, 0, font);
            unsafe { MoveWindow(self.status, x, y, width, s(18), 0) };
            y += s(18) + s(12);
        } else {
            self.progress = null_mut();
            self.status = null_mut();
        }

        // buttons, right-aligned
        let (bw, bh) = (s(150), s(30));
        y += s(6);
        let mut bx = x + width;
        self.secondary = null_mut();
        self.primary = null_mut();
        if let Some(t) = &p.secondary {
            bx -= bw.min(s(110));
            self.secondary = self.child("BUTTON", t, WS_TABSTOP as u32 | BS_PUSHBUTTON as u32, ID_SECONDARY, font);
            unsafe { MoveWindow(self.secondary, bx, y, bw.min(s(110)), bh, 0) };
            bx -= s(8);
        }
        if let Some(t) = &p.primary {
            let w = (self.text_width(t, font) + s(32)).max(s(110)).min(bw + s(40));
            bx -= w;
            self.primary = self.child("BUTTON", t, WS_TABSTOP as u32 | BS_DEFPUSHBUTTON as u32, ID_PRIMARY, font);
            unsafe { MoveWindow(self.primary, bx, y, w, bh, 0) };
        }
        y += bh + s(PAD);

        unsafe {
            let mut r = RECT { left: 0, top: 0, right: s(WIDTH), bottom: y };
            let style = GetWindowLongW(self.hwnd, GWL_STYLE) as u32;
            windows_sys::Win32::UI::HiDpi::AdjustWindowRectExForDpi(&mut r, style, 0, 0, d as u32);
            SetWindowPos(self.hwnd, null_mut(), 0, 0, r.right - r.left, r.bottom - r.top, SWP_NOMOVE | SWP_NOZORDER);
            let focus = if !self.primary.is_null() { self.primary } else { self.secondary };
            if !focus.is_null() {
                SetFocus(focus);
            }
            InvalidateRect(self.hwnd, null(), 1);
        }
        self.page = p;
    }

    fn text_width(&self, text: &str, f: HFONT) -> i32 {
        unsafe {
            let dc = GetDC(self.hwnd);
            let old = SelectObject(dc, f as HGDIOBJ);
            let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            let w = wide(text);
            DrawTextW(dc, w.as_ptr(), -1, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
            SelectObject(dc, old);
            ReleaseDC(self.hwnd, dc);
            r.right - r.left
        }
    }

    fn checks_now(&self) -> Vec<bool> {
        self.checks.iter().map(|&h| unsafe { SendMessageW(h, BM_GETCHECK, 0, 0) } == BST_CHECKED as LRESULT).collect()
    }

    fn rescale(&mut self) {
        let d = dpi(self.hwnd);
        unsafe {
            DeleteObject(self.font as HGDIOBJ);
            DeleteObject(self.title_font as HGDIOBJ);
        }
        self.font = font(9, 400, d);
        self.title_font = font(14, 600, d);
        let p = std::mem::take(&mut self.page);
        self.show(p);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = (wp & 0xFFFF) as usize;
            let code = ((wp >> 16) & 0xFFFF) as u32;
            if code == BN_CLICKED && (id == ID_PRIMARY || id == ID_SECONDARY) {
                let n = with_app(|a| {
                    if a.running {
                        // the secondary button cancels a running job
                        if id == ID_SECONDARY {
                            a.shared.cancel.store(true, Ordering::Relaxed);
                            if !a.secondary.is_null() {
                                EnableWindow(a.secondary, 0);
                            }
                        }
                        return None;
                    }
                    let checks = a.checks_now();
                    Some(a.flow.button(if id == ID_PRIMARY { Btn::Primary } else { Btn::Secondary }, &checks))
                })
                .flatten();
                if let Some(n) = n {
                    apply(n);
                }
                return 0;
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_PROGRESS => {
            with_app(|a| {
                let (f, t) = a.shared.progress.lock().unwrap().clone();
                if !a.progress.is_null() {
                    SendMessageW(a.progress, PBM_SETPOS, (f * 1000.0) as WPARAM, 0);
                }
                if !a.status.is_null() {
                    let w = wide(&t);
                    SetWindowTextW(a.status, w.as_ptr());
                }
            });
            0
        }
        WM_DONE => {
            let n = with_app(|a| {
                a.running = false;
                let r = a.shared.result.lock().unwrap().take().unwrap_or_else(|| Err("stopped".into()));
                if a.close_after {
                    return Next::Close;
                }
                a.flow.done(r)
            });
            if let Some(n) = n {
                apply(n);
            }
            0
        }
        WM_CLOSE => {
            let busy = with_app(|a| {
                if a.running {
                    // finish (or cancel) the job first, then close
                    a.shared.cancel.store(true, Ordering::Relaxed);
                    a.close_after = true;
                    true
                } else {
                    false
                }
            });
            match busy {
                Some(true) => 0,
                Some(false) => {
                    apply(Next::Close);
                    0
                }
                None => DefWindowProcW(hwnd, msg, wp, lp),
            }
        }
        WM_DPICHANGED => {
            let r = &*(lp as *const RECT);
            SetWindowPos(hwnd, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            with_app(|a| a.rescale());
            0
        }
        WM_CTLCOLORSTATIC => {
            // labels on the window's own background
            SetBkMode(wp as HDC, TRANSPARENT as i32);
            GetSysColorBrush(COLOR_WINDOW) as LRESULT
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
