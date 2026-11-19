//! ---- ws:panels-multiscreen ----
//! Multi-screen mode, the App half (the pure half is `ui::screens`):
//! - `tick` (FRAME_HOOK): looks at the monitors at most every 3 s, and only on frames drawn anyway (no
//!   repaint of its own); asks "Use 2-screen mode?" when more monitors are connected than it was answered
//!   for (never on a quiet launch); gives each screen window its monitor, folding back the ones whose
//!   monitor is gone; shows the Audio page's "new layout" note once.
//! - `swap_screen_mode`: each page keeps an arrangement per screen count (`Settings.screen_layouts`);
//!   switching count stashes this count's trees and brings the other count's back (or the built-in one,
//!   `Layout::for_screens`).
//! - `drop_outside`: a tab dragged out of the main window lands on the screen under the mouse.

use super::*;
use crate::ui::screens::{self, Monitor};

/// How often a drawn frame re-reads the monitors (a monitor added or removed).
const RECHECK: Duration = Duration::from_secs(3);

/// Multi-screen state on `App`.
#[derive(Default)]
pub(crate) struct ScreensState {
    pub ui: screens::ScreensUi,
    /// Connected monitors, Screen 1 (the main window's) first.
    pub monitors: Vec<Monitor>,
    checked: Option<Instant>,
}

fn key(count: u8, page: &str) -> String {
    format!("{count}|{page}")
}

/// Switch every page to its `n`-screen arrangement: this count's trees go to `screen_layouts`, the new
/// count's come back (missing = the page's start, laid out for `n`: `layout_ctl::page_start`). The pages
/// that had no tree stay without one (they start fresh, for `n`, on their next visit). False = already `n`.
pub(super) fn swap_screen_mode(layout: &mut Layout, settings: &mut Settings, n: u8) -> bool {
    let old = settings.screen_mode.max(1);
    let n = n.clamp(1, 3);
    if old == n {
        return false;
    }
    let mut cur = layout.clone();
    cur.unmaximize();
    let page = settings.page.clone();
    settings.screen_layouts.insert(key(old, &page), cur.to_json());
    for (pg, json) in std::mem::take(&mut settings.page_layouts) {
        settings.screen_layouts.insert(key(old, &pg), json);
    }
    settings.screen_mode = n;
    for pg in settings.pages.clone() {
        if let Some(json) = settings.screen_layouts.remove(&key(n, &pg)) {
            settings.page_layouts.insert(pg, json);
        }
    }
    let stored = settings.page_layouts.remove(&page);
    let fresh =
        stored.as_deref().and_then(Layout::from_json).unwrap_or_else(|| layout_ctl::page_start(settings, &page));
    layout.reset(fresh);
    settings.layout = layout.to_json();
    true
}

/// Use `n` screens (the setup window's pick): swap the layouts, place the windows, remember the answer.
pub(super) fn set_mode(app: &mut App, n: u8) {
    let monitors = app.screens.monitors.len().max(1) as u8;
    app.settings.screens_answered = app.settings.screens_answered.max(monitors);
    let changed = {
        let App { layout, settings, undo, redo, .. } = app;
        let c = swap_screen_mode(layout, settings, n);
        if c {
            undo.retain(|e| e.json != LAYOUT_STEP);
            redo.retain(|e| e.json != LAYOUT_STEP);
        }
        c
    };
    app.layout_json = app.settings.layout.clone();
    if changed {
        app.layout_dirty = true;
        let msg = match n {
            1 => "One screen: everything is back in this window".to_string(),
            n => format!("{n}-screen mode: each page now has a window on each screen"),
        };
        app.toast(msg);
    }
    if !crate::winpos::quiet() {
        app.settings.save();
    }
}

/// The main window's centre in desktop pixels (what decides which monitor is Screen 1).
fn main_centre(ctx: &egui::Context) -> Option<(i32, i32)> {
    let r = ctx.input(|i| i.viewport().outer_rect)?;
    let ppp = ctx.pixels_per_point();
    Some(((r.center().x * ppp) as i32, (r.center().y * ppp) as i32))
}

/// Give every unplaced screen window its monitor (`screens::assign`); one with none left folds back.
pub(super) fn place_windows(app: &mut App, ppp: f32) {
    if app.layout.screens.iter().all(|s| s.place.is_some()) {
        return;
    }
    let ordered = app.screens.monitors.clone();
    let wanted: Vec<(u8, String)> = app.layout.screens.iter().map(|s| (s.screen, s.monitor.clone())).collect();
    let names: Vec<(u8, &str)> = wanted.iter().map(|(n, m)| (*n, m.as_str())).collect();
    let mut got = screens::assign(&names, &ordered);
    // automation runs (SE_BACKGROUND) keep every window on the main window's monitor, never the user's
    if crate::winpos::background() && !ordered.is_empty() {
        got = got.iter().map(|_| Some(0)).collect();
    }
    let mut fold = Vec::new();
    for (i, slot) in got.into_iter().enumerate() {
        match slot.and_then(|m| ordered.get(m)) {
            Some(m) => {
                let sw = &mut app.layout.screens[i];
                if sw.monitor != m.name {
                    sw.rect = None; // a rect from another monitor would open it there: maximise on this one
                    sw.monitor = m.name.clone();
                }
                let [x, y, w, h] = m.work;
                let s = ppp.max(0.1);
                let mut at = [x as f32 / s, y as f32 / s, w as f32 / s, h as f32 / s];
                if crate::winpos::background() {
                    // side by side on that monitor, not maximised over the main window
                    let k = sw.screen as f32 - 1.0;
                    at = [at[0] + 60.0 * k, at[1] + 60.0 * k, at[2] * 0.6, at[3] * 0.6];
                    sw.rect = Some(at);
                }
                sw.place = Some(at);
            }
            None => fold.push(i),
        }
    }
    if !fold.is_empty() {
        for i in fold.into_iter().rev() {
            app.layout.fold_screen(i);
        }
        app.toast("A screen is not connected: its panels are back in this window");
    }
    app.layout_dirty = true;
}

/// FRAME_HOOK (see the module doc).
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    let due = app.screens.checked.is_none_or(|t| t.elapsed() >= RECHECK);
    if due {
        app.screens.checked = Some(Instant::now());
        let before = app.screens.monitors.clone();
        app.screens.monitors = screens::ordered(screens::monitors(), main_centre(ctx));
        let now = &app.screens.monitors;
        let names = |v: &[Monitor]| v.iter().map(|m| m.name.clone()).collect::<Vec<_>>();
        if names(&before) != names(now) {
            // a monitor came or went: re-place every window (a missing monitor folds its window back)
            for s in &mut app.layout.screens {
                s.place = None;
            }
            // never a modal: the welcome wizard asks on a first run; later a toast offers the choice once
            // per monitor count (shown = answered, as the setup window's Not now was)
            let answered = app.settings.screens_answered;
            let ask = screens::prompt(now.len(), crate::winpos::quiet(), answered).filter(|_| app.onboarding.is_none());
            if let Some(n) = ask.filter(|&n| n != app.settings.screen_mode) {
                app.settings.screens_answered = answered.max(now.len() as u8);
                app.settings.save();
                let msg = format!("{} monitors connected", now.len());
                app.push_toast(super::feedback::Toast::new(msg).action(&format!("Use {n} screens…"), Action::ScreenSetup));
            }
        }
    }
    place_windows(app, ctx.pixels_per_point());
    audio_page_note(app);
}

/// The Audio page has a new default; a saved Audio layout is kept, so say once how to get the new one.
fn audio_page_note(app: &mut App) {
    if app.settings.audio_layout_offered || app.settings.page != "Audio" || crate::winpos::quiet() {
        return;
    }
    app.settings.audio_layout_offered = true;
    app.settings.save();
    if !app.layout.is_visible(Pane::Voiceover) {
        let t =
            feedback::Toast::new("The Audio page has a new layout: Voiceover, Transcript, music and a small preview")
                .action("Reset Page Layout", Action::ResetPageLayout);
        app.push_toast(t);
    }
}

/// A tab let go outside the main window: onto the screen under the mouse (a new window there if the page
/// has none yet); on the main window's own monitor it undocks into a window of its own instead.
pub(super) fn drop_outside(app: &mut App, pane: Pane, ppp: f32) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    if unsafe { GetCursorPos(&mut p) }.is_err() {
        return;
    }
    match screens::screen_at(&app.screens.monitors, (p.x, p.y)) {
        Some(n) if n > 1 => app.layout.move_to_screen(pane, n),
        _ => {
            app.layout.popout(pane);
            let ppp = ppp.max(0.1);
            let at = [p.x as f32 / ppp - 40.0, p.y as f32 / ppp - 12.0, 800.0, 500.0];
            app.layout.set_popped_rect(pane, at);
        }
    }
    app.layout_dirty = true;
}

/// ACT_HANDLERS: Window ▸ Screens… and Reset Page Layout.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::ScreenSetup => {
            app.screens.checked = None; // read the monitors again: the window says how many there are
            app.screens.ui = screens::ScreensUi { open: true, asked: false };
            true
        }
        Action::ResetPageLayout => {
            let page = app.settings.page.clone();
            layout_ctl::reset_page(app, &page, false);
            true
        }
        _ => false,
    }
}

/// WINDOW_DRAWERS: the setup window.
pub(super) fn window(app: &mut App, ctx: &egui::Context) {
    let n = app.screens.monitors.len().max(1);
    if let Some(pick) = screens::window(ctx, &mut app.screens.ui, n, app.settings.screen_mode.max(1)) {
        set_mode(app, pick);
    }
}

/// `layout.screens`: read the monitors and screen windows; set the screen count; move a pane to a screen.
pub const TOOLS: &[crate::mcp::tools::ToolDef] = &[crate::mcp::tools::ToolDef {
    name: "layout.screens",
    desc: "Multi-screen mode. No args: the monitors, the screen count in use and each screen window's panes.            'mode' (1, 2 or 3) lays every page out for that many screens (each page keeps an arrangement per            count); 'pane' + 'screen' moves a pane to Screen N (1 = the main window).",
    args: &[
        "mode:integer:false:1, 2 or 3 screens",
        "pane:string:false:pane title to move",
        "screen:integer:false:with pane: the screen number (1 = main window)",
        "prompt:boolean:false:open the 'Use N-screen mode?' setup window",
    ],
    kind: crate::mcp::tools::ToolKind::Ui,
    run: |app, args| {
        use super::tools_helpers::*;
        if arg_bool(args, "prompt").unwrap_or(false) {
            app.screens.ui = screens::ScreensUi { open: true, asked: true };
        }
        if let Some(n) = arg_u64(args, "mode") {
            if !(1..=3).contains(&n) {
                return Err(format!("mode must be 1, 2 or 3, got {n}"));
            }
            set_mode(app, n as u8);
        }
        if let Some(name) = arg_str(args, "pane") {
            let pane = Pane::by_title(name).ok_or_else(|| format!("unknown pane '{name}'"))?;
            let n = req(arg_u64(args, "screen"), "screen")?.clamp(1, 9) as u8;
            app.layout.move_to_screen(pane, n);
            app.layout_dirty = true;
        }
        let windows: Vec<Value> = app
            .layout
            .screens
            .iter()
            .map(|s| json!({"screen": s.screen, "monitor": s.monitor, "panes": s.panes().iter().map(|p| p.title()).collect::<Vec<_>>()}))
            .collect();
        let monitors: Vec<Value> =
            app.screens.monitors.iter().map(|m| json!({"name": m.name, "rect": m.rect, "primary": m.primary})).collect();
        Ok(crate::mcp::tools::ToolOutcome::Done(json!({
            "mode": app.settings.screen_mode.max(1),
            "monitors": monitors,
            "windows": windows,
        })))
    },
}];

#[cfg(test)]
mod tests {
    use super::*;

    /// A layout as a value, its hidden-tile set sorted (a HashSet serialises in any order).
    fn v(json: &str) -> serde_json::Value {
        let mut v: serde_json::Value = serde_json::from_str(json).unwrap();
        if let Some(serde_json::Value::Array(a)) = v.pointer_mut("/tree/tiles/invisible") {
            a.sort_by_key(|x| x.as_u64());
        }
        v
    }

    /// Each screen count keeps each page's own arrangement: 2-screen edits survive a trip to one screen and
    /// back, the one-screen tree comes back untouched, and the whole thing survives a settings save.
    #[test]
    fn a_layout_per_screen_count_round_trips_in_settings() {
        let mut s = Settings::default();
        s.page = "Edit".into();
        let mut l = layout_ctl::page_start(&s, "Edit");
        let one = l.to_json();
        assert!(swap_screen_mode(&mut l, &mut s, 2));
        assert_eq!(s.screen_mode, 2);
        assert_eq!(l.screens.len(), 1, "the built-in 2-screen Edit page has a Timeline window");
        assert_eq!(l.screens[0].panes(), vec![Pane::Timeline]);
        assert!(!l.tree.tiles.is_visible(l.tree.tiles.find_pane(&Pane::Timeline).unwrap()));
        // the user puts the Mixer on Screen 2 too
        l.move_to_screen(Pane::Mixer, 2);
        let two = l.to_json();
        // through a settings file (serde) and back
        let s2: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        let mut s = s2;
        assert!(swap_screen_mode(&mut l, &mut s, 1));
        assert!(l.screens.is_empty());
        assert_eq!(v(&l.to_json()), v(&one), "the one-screen arrangement is untouched");
        assert!(swap_screen_mode(&mut l, &mut s, 2));
        assert_eq!(v(&l.to_json()), v(&two), "the 2-screen edits came back");
        assert!(l.screen_of(Pane::Mixer).is_some());
        assert!(!swap_screen_mode(&mut l, &mut s, 2), "already there");
        // three screens: Timeline, Preview and the tools each get their own
        assert!(swap_screen_mode(&mut l, &mut s, 3));
        let mut shown: Vec<(u8, Pane)> =
            l.screens.iter().flat_map(|w| w.panes().into_iter().map(|p| (w.screen, p)).collect::<Vec<_>>()).collect();
        shown.sort_by_key(|&(n, _)| n);
        assert!(shown.contains(&(2, Pane::Timeline)) && shown.contains(&(3, Pane::Preview)), "{shown:?}");
        assert!(
            l.is_visible(Pane::Inspector) && l.screen_of(Pane::Inspector).is_none(),
            "tools stay in the main window"
        );
        assert!(Layout::from_json(&l.to_json()).is_some());
    }

    /// Another page switched to while in 2-screen mode starts from its own 2-screen default.
    #[test]
    fn a_page_first_visited_in_two_screen_mode_starts_split() {
        let mut s = Settings::default();
        s.page = "Edit".into();
        let mut l = layout_ctl::page_start(&s, "Edit");
        swap_screen_mode(&mut l, &mut s, 2);
        let (mut u, mut r) = (Vec::new(), Vec::new());
        assert!(layout_ctl::swap_page(&mut l, &mut s, &mut u, &mut r, "Audio"));
        assert_eq!(l.screen_of(Pane::Timeline), Some(0), "the Audio page's Timeline is on Screen 2");
        assert!(l.is_visible(Pane::Voiceover) && l.screen_of(Pane::Voiceover).is_none());
    }

    /// A screen window whose monitor is missing folds back: its panes reappear in the main window where
    /// they were.
    #[test]
    fn a_missing_monitor_folds_windows_back() {
        let s = Settings { screen_mode: 2, ..Settings::default() };
        let mut l = layout_ctl::page_start(&s, "Edit");
        assert_eq!(l.screen_of(Pane::Timeline), Some(0));
        let mon = |n: &str| Monitor { name: n.into(), rect: [0, 0, 10, 10], work: [0, 0, 10, 10], primary: true };
        let got = screens::assign(&[(2, "\\\\.\\DISPLAY2")], &[mon("\\\\.\\DISPLAY1")]);
        assert_eq!(got, [None]);
        l.fold_screen(0);
        assert!(l.screens.is_empty());
        let id = l.tree.tiles.find_pane(&Pane::Timeline).unwrap();
        assert!(l.tree.tiles.is_visible(id), "the Timeline is back in the main window");
        assert_eq!(
            v(&l.to_json()),
            v(&layout_ctl::page_start(&Settings::default(), "Edit").to_json()),
            "exactly where it was"
        );
    }
}
