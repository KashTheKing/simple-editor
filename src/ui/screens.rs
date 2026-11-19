//! ---- ws:panels-multiscreen ----
//! Multi-screen mode, the App-free half: the monitors (Win32 `EnumDisplayMonitors`), which one is "Screen
//! N", when to ask "Use 2-screen mode?", which monitor each screen window gets (or none: it folds back
//! into the main window), and the setup window with a small picture of each arrangement.
//! `ui::app::screens_ctl` does the rest (the per-count layouts, placing the windows, the drop of a tab
//! dragged out of the main window).

use eframe::egui;

/// One monitor, in physical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    /// The device name (`\\.\DISPLAY2`): how a window remembers its monitor.
    pub name: String,
    /// `[x, y, w, h]` of the whole monitor.
    pub rect: [i32; 4],
    /// `[x, y, w, h]` without the taskbar.
    pub work: [i32; 4],
    pub primary: bool,
}

impl Monitor {
    pub fn contains(&self, (x, y): (i32, i32)) -> bool {
        let [l, t, w, h] = self.rect;
        x >= l && x < l + w && y >= t && y < t + h
    }
}

/// The monitors connected now, primary first then left to right (a microsecond call, no cache needed).
pub fn monitors() -> Vec<Monitor> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW};
    unsafe extern "system" fn each(mon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let out = unsafe { &mut *(data.0 as *mut Vec<Monitor>) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(mon, &mut info.monitorInfo) }.as_bool() {
            let r = |r: RECT| [r.left, r.top, r.right - r.left, r.bottom - r.top];
            let end = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
            out.push(Monitor {
                name: String::from_utf16_lossy(&info.szDevice[..end]),
                rect: r(info.monitorInfo.rcMonitor),
                work: r(info.monitorInfo.rcWork),
                primary: info.monitorInfo.dwFlags & 1 != 0, // MONITORINFOF_PRIMARY
            });
        }
        true.into()
    }
    let mut out: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut out as *mut Vec<Monitor> as isize));
    }
    out.sort_by_key(|m| (!m.primary, m.rect[0], m.rect[1]));
    out
}

/// `monitors` numbered as screens: the one under `main` (the main window's centre, physical px) is
/// Screen 1, the rest follow left to right.
pub fn ordered(mut monitors: Vec<Monitor>, main: Option<(i32, i32)>) -> Vec<Monitor> {
    monitors.sort_by_key(|m| (!main.is_some_and(|p| m.contains(p)), !m.primary, m.rect[0], m.rect[1]));
    monitors
}

/// The screen count to offer ("Use 2-screen mode?" / 3-screen), or `None` to ask nothing: one monitor, a
/// quiet launch (`--quiet`, `SE_QUIET`, `SE_BACKGROUND`), or already answered for this many monitors (it
/// asks again when a monitor is added beyond the count it was answered for).
pub fn prompt(monitors: usize, quiet: bool, answered: u8) -> Option<u8> {
    if monitors < 2 || quiet || monitors <= answered as usize {
        return None;
    }
    Some(monitors.min(3) as u8)
}

/// Which monitor (index into `ordered`, never 0 = the main window's) each screen window opens on: its
/// remembered one when connected, else the monitor its number names, else any free one. `None` = no
/// monitor left: the window folds back into the main one.
pub fn assign(windows: &[(u8, &str)], ordered: &[Monitor]) -> Vec<Option<usize>> {
    let mut used = vec![false; ordered.len()];
    if !used.is_empty() {
        used[0] = true;
    }
    let mut out: Vec<Option<usize>> = vec![None; windows.len()];
    // remembered monitors first, so a window keeps its monitor when another one comes and goes
    for (k, (_, name)) in windows.iter().enumerate() {
        if let Some(i) = ordered.iter().position(|m| !name.is_empty() && m.name == *name).filter(|&i| !used[i]) {
            used[i] = true;
            out[k] = Some(i);
        }
    }
    for (k, (n, _)) in windows.iter().enumerate() {
        if out[k].is_some() {
            continue;
        }
        let by_number = (*n as usize).checked_sub(1).filter(|&i| i < ordered.len() && !used[i]);
        if let Some(i) = by_number.or_else(|| used.iter().position(|u| !u)) {
            used[i] = true;
            out[k] = Some(i);
        }
    }
    out
}

/// The screen number (1-based, `ordered`'s order) of the monitor under a desktop point.
pub fn screen_at(ordered: &[Monitor], p: (i32, i32)) -> Option<u8> {
    ordered.iter().position(|m| m.contains(p)).map(|i| i as u8 + 1)
}

/// The setup window's state (Window ▸ Screens…, Settings ▸ Layout, or the prompt on its own).
#[derive(Default)]
pub struct ScreensUi {
    pub open: bool,
    /// Opened by itself (monitors added): "Not now" counts as an answer.
    pub asked: bool,
}

/// The arrangements as picture cards (1, 2 and - with three monitors - 3 screens), `current` selected.
/// Also the welcome wizard's Screens step.
pub fn choices(ui: &mut egui::Ui, monitors: usize, current: u8) -> Option<u8> {
    let mut pick = None;
    let accent = ui.visuals().selection.bg_fill;
    ui.horizontal_top(|ui| {
        for n in [2u8, 3, 1] {
            if n == 3 && monitors < 3 {
                continue;
            }
            ui.vertical(|ui| {
                ui.set_width(150.0);
                let ok = n as usize <= monitors.max(1);
                let label = match n {
                    1 => "Keep one screen".to_string(),
                    n => format!("Use {n} screens"),
                };
                let r = ui.add_enabled_ui(ok, |ui| picture(ui, n, accent)).inner;
                let b = ui.add_enabled(ok, egui::Button::new(label).selected(n == current));
                if ok && (r.clicked() || b.clicked()) {
                    pick = Some(n);
                }
                ui.weak(match n {
                    1 => "Everything in one window",
                    2 => "Timeline on its own screen",
                    _ => "Timeline, preview, tools",
                });
            });
        }
    });
    pick
}

/// One arrangement's picture: `monitors` screens side by side, each with its blocks.
fn picture(ui: &mut egui::Ui, n: u8, accent: egui::Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(150.0, 64.0), egui::Sense::click());
    let p = ui.painter();
    let fg = ui.visuals().text_color();
    let gap = 6.0;
    let w = (rect.width() - gap * (n as f32 - 1.0)) / n as f32;
    let block = |r: egui::Rect, label: &str, fill: egui::Color32| {
        p.rect_filled(r.shrink(1.5), 2.0, fill);
        p.text(r.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(9.0), fg);
    };
    let soft = accent.gamma_multiply(0.35);
    let tools = ui.visuals().widgets.inactive.bg_fill;
    for i in 0..n {
        let x = rect.left() + i as f32 * (w + gap);
        let screen = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(w, rect.height()));
        p.rect_stroke(screen, 3.0, egui::Stroke::new(1.5, fg.gamma_multiply(0.6)), egui::StrokeKind::Inside);
        let inner = screen.shrink(4.0);
        let (top, bottom) = inner.split_top_bottom_at_fraction(0.55);
        match (n, i) {
            (1, _) => {
                let (tl, tr) = top.split_left_right_at_fraction(0.6);
                block(tl, "Preview", soft);
                block(tr, "Tools", tools);
                block(bottom, "Timeline", soft);
            }
            (2, 0) => {
                block(top, "Preview", soft);
                block(bottom, "Tools", tools);
            }
            (3, 0) => block(inner, "Tools", tools),
            (3, 1) => block(inner, "Preview", soft),
            _ => block(inner, "Timeline", soft),
        }
    }
    resp
}

/// The setup window. `monitors` = connected now (2 and 3 screens need that many). Returns the pick: 1, 2
/// or 3 screens. Closing it (or Not now) answers too when it asked by itself (`asked`).
pub fn window(ctx: &egui::Context, st: &mut ScreensUi, monitors: usize, current: u8) -> Option<u8> {
    if !st.open {
        return None;
    }
    let mut pick = None;
    let mut open = true;
    let title =
        if st.asked && monitors >= 2 { format!("Use {}-screen mode?", monitors.min(3)) } else { "Screens".into() };
    egui::Window::new(title)
        .id(egui::Id::new("screens_setup"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label(format!("{monitors} monitor{} connected", if monitors == 1 { "" } else { "s" }));
            ui.add_space(6.0);
            pick = choices(ui, monitors, current);
            ui.add_space(4.0);
            ui.weak("Each page keeps its own arrangement for each screen count. Window ▸ Screens… changes it later.");
        });
    if pick.is_some() || !open {
        st.open = false;
        if pick.is_none() && st.asked {
            pick = Some(current); // Not now = keep what you have, and don't ask again for these monitors
        }
        st.asked = false;
    }
    pick
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(name: &str, x: i32, primary: bool) -> Monitor {
        Monitor { name: name.into(), rect: [x, 0, 1920, 1080], work: [x, 0, 1920, 1040], primary }
    }

    #[test]
    fn the_prompt_asks_only_when_it_should() {
        assert_eq!(prompt(1, false, 0), None, "one monitor: nothing to ask");
        assert_eq!(prompt(2, false, 0), Some(2));
        assert_eq!(prompt(3, false, 0), Some(3));
        assert_eq!(prompt(4, false, 0), Some(3), "three screens is the most it lays out");
        assert_eq!(prompt(2, true, 0), None, "a quiet launch never asks");
        assert_eq!(prompt(3, true, 0), None);
        assert_eq!(prompt(2, false, 2), None, "already answered for two");
        assert_eq!(prompt(3, false, 2), Some(3), "a third monitor added: ask again");
        assert_eq!(prompt(2, false, 3), None, "a monitor unplugged: nothing new to ask");
    }

    #[test]
    fn screen_one_is_the_main_windows_monitor() {
        let ms = vec![mon("A", 0, true), mon("B", 1920, false), mon("C", -1920, false)];
        let o = ordered(ms.clone(), Some((2000, 500)));
        assert_eq!(o.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["B", "A", "C"]);
        let o = ordered(ms, None);
        assert_eq!(o.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["A", "C", "B"], "primary, then by x");
        assert_eq!(screen_at(&o, (-10, 10)), Some(2));
        assert_eq!(screen_at(&o, (99999, 10)), None);
    }

    /// A window keeps its remembered monitor; a missing one falls to a free monitor, and with none left
    /// it folds back (`None`).
    #[test]
    fn a_missing_monitor_folds_its_window_back() {
        let three = vec![mon("A", 0, true), mon("B", 1920, false), mon("C", 3840, false)];
        assert_eq!(assign(&[(2, "C"), (3, "B")], &three), [Some(2), Some(1)], "remembered monitors win");
        assert_eq!(assign(&[(2, ""), (3, "")], &three), [Some(1), Some(2)], "by number");
        let two = vec![mon("A", 0, true), mon("C", 3840, false)];
        assert_eq!(assign(&[(2, "B"), (3, "C")], &two), [None, Some(1)], "B is gone and C is taken: fold");
        let one = vec![mon("A", 0, true)];
        assert_eq!(assign(&[(2, "B")], &one), [None], "one monitor: every screen window folds");
        assert_eq!(assign(&[(2, "A")], &one), [None], "never the main window's monitor");
    }

    #[test]
    fn monitors_are_listed() {
        let ms = monitors();
        assert!(!ms.is_empty(), "a desktop session has at least one monitor");
        assert!(ms.iter().all(|m| m.rect[2] > 0 && m.name.starts_with("\\\\.\\")), "{ms:?}");
    }

    /// The setup window (as the prompt and as Window ▸ Screens…) lays out and picks nothing by itself.
    #[test]
    fn assert_no_idle_repaint_screens_window() {
        for (monitors, asked) in [(2, true), (3, true), (1, false), (3, false)] {
            let ctx = egui::Context::default();
            let mut st = ScreensUi { open: true, asked };
            // (the clock moves, so the window's fade-in finishes)
            let raw = |t: f64| egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
                time: Some(t),
                ..Default::default()
            };
            for f in 0..6 {
                let _ = ctx.run(raw(f as f64), |ctx| assert_eq!(window(ctx, &mut st, monitors, 1), None));
            }
            assert!(st.open && !ctx.has_requested_repaint(), "{monitors} monitors {:?}", ctx.repaint_causes());
        }
    }
}
