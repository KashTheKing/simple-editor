//! ---- ws:layout-modes-onboarding ----
//! First-run welcome: a non-blocking `egui::Window` (the editor behind it stays fully usable - drop a
//! file, press Space, whatever) with three cards: (1) ffmpeg status from ONE probe when the window
//! first draws, never per frame, (2) twelve keys to know with "Show all…" opening the F1 cheat sheet,
//! (3) a starting format plus an OPT-IN checkbox for the Explorer "Edit with Simple Editor" entry - the
//! registry is written only from Finish, only when ticked, and only under the same guard `App::new`
//! used to apply unconditionally (release build, not already installed). ws:pages removed the old
//! first card (Dynamic / Classic layout): the pages replace that choice, and following the selection
//! is a Settings ▸ General checkbox. No `App` in here: `ui::app::layout_ctl` owns the glue (arming,
//! Finish side effects), so `show`/`finish` are unit-testable like every other window in this crate.

use crate::hotkeys::{Action, Hotkeys};
use crate::settings::Settings;
use crate::ui::guides::PRESETS;
use crate::ui::tools::{glyph_label, Glyph};
use eframe::egui;

/// The wizard's state while it is open (`App.onboarding`).
#[derive(Clone, Debug, PartialEq)]
pub struct Onboarding {
    /// 0..=3, one card each.
    pub step: u8,
    /// Card 1: One window instead of pages (`Settings.one_window`, applied on Finish).
    pub one_window: bool,
    /// Monitors connected when the wizard opened: the Screens card shows only with 2 or more.
    pub monitors: usize,
    /// The Screens card's pick (`Settings.screen_mode`, applied on Finish).
    pub screens: u8,
    /// Card 4's opt-in. Defaults to `Settings.context_menu` so a plain Finish reproduces the old
    /// behaviour bit-for-bit for a user who never touched the setting.
    pub install_context_menu: bool,
    /// `guides::PRESETS` index picked on card 4; `None` keeps the project's default 1080p60.
    pub template: Option<usize>,
    /// (found, status text) - probed once by `show` on the card's first draw, never per frame.
    ffmpeg: Option<(bool, String)>,
}

impl Onboarding {
    pub fn new(settings: &Settings) -> Self {
        // (tests: one monitor, whatever the machine has)
        let monitors = if cfg!(test) { 1 } else { crate::ui::screens::monitors().len() };
        Self {
            step: 0,
            one_window: settings.one_window,
            monitors,
            screens: settings.screen_mode.max(1),
            install_context_menu: settings.context_menu,
            template: None,
            ffmpeg: None,
        }
    }

    /// The cards in order: Screens only with more than one monitor.
    pub fn cards(&self) -> Vec<Card> {
        let mut c = vec![Card::Workspace];
        if self.monitors >= 2 {
            c.push(Card::Screens);
        }
        c.extend([Card::Ffmpeg, Card::Keys, Card::Format]);
        c
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Card {
    Workspace,
    Screens,
    Ffmpeg,
    Keys,
    Format,
}

/// What the wizard asked the app to do this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Finish pressed: apply (`finish`) and close.
    Finish,
    /// The window's X: close and don't ask again (nothing is applied, nothing is installed).
    Dismiss,
    /// "Show all…" on the keys card: open the F1 cheat sheet, keep the wizard open.
    ShowCheatSheet,
}

/// The twelve keys card 3 lists (their LIVE bindings via `Hotkeys::text`, so a remap shows through).
pub const KEYS: [Action; 12] = [
    Action::PlayPause,
    Action::ShuttleBack,
    Action::Stop,
    Action::ShuttleFwd,
    Action::MarkIn,
    Action::MarkOut,
    Action::Split,
    Action::TrimLeft1,
    Action::TrimRight1,
    Action::CommandPalette,
    Action::Undo,
    Action::Save,
];

/// The existing `App::new` guard, minus the consent that now comes from the checkbox: never write the
/// registry from a debug build, never when the entry already points at this exe.
pub fn install_allowed(debug_build: bool, installed: bool) -> bool {
    !debug_build && !installed
}

/// Finish: persist the choices and call `install` iff the box is ticked AND `allowed`
/// (`install_allowed` evaluated by the caller). Returns whether `install` ran. `Settings.context_menu`
/// records the answer either way, so the Settings tab and `boot::run`'s consented re-point agree with
/// it. The layout is never touched (a first run starts on the Edit page).
pub fn finish(st: &Onboarding, settings: &mut Settings, allowed: bool, install: &mut dyn FnMut()) -> bool {
    settings.onboarded = true;
    settings.context_menu = st.install_context_menu;
    let installing = st.install_context_menu && allowed;
    if installing {
        install();
    }
    installing
}

fn ffmpeg_status() -> (bool, String) {
    match crate::media::ffpipe::ffmpeg_exe() {
        Some(p) => (true, format!("ffmpeg found: {}", p.display())),
        None => (false, "ffmpeg not found".into()),
    }
}

/// Draws the wizard while `st` exists. `Some(outcome)` when the user finished, closed or asked for the
/// cheat sheet - the caller applies it (see `Outcome`). Reads settings only; `finish` mutates.
pub fn show(ctx: &egui::Context, st: &mut Onboarding, settings: &Settings, hotkeys: &Hotkeys) -> Option<Outcome> {
    let mut out = None;
    let mut open = true;
    egui::Window::new("Welcome to Simple Editor")
        .id(egui::Id::new("onboarding_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center() - egui::vec2(0.0, 30.0))
        .default_width(470.0)
        .show(ctx, |ui| {
            ui.set_width(470.0);
            let cards = st.cards();
            let n = cards.len() as u8;
            st.step = st.step.min(n - 1);
            match cards[st.step as usize] {
                Card::Workspace => workspace_card(ui, st),
                Card::Screens => {
                    ui.heading("Screens");
                    ui.add_space(4.0);
                    ui.label(format!("{} monitors connected", st.monitors));
                    ui.add_space(6.0);
                    if let Some(pick) = crate::ui::screens::choices(ui, st.monitors, st.screens) {
                        st.screens = pick;
                    }
                }
                Card::Ffmpeg => ffmpeg_card(ui, st),
                Card::Keys => {
                    if keys_card(ui, hotkeys) {
                        out = Some(Outcome::ShowCheatSheet);
                    }
                }
                Card::Format => template_card(ui, st, settings),
            }
            ui.add_space(6.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.weak(format!("{} / {n}", st.step + 1));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if st.step >= n - 1 {
                        if ui.button("Finish").clicked() {
                            out = Some(Outcome::Finish);
                        }
                    } else if ui.button("Next").clicked() {
                        st.step += 1;
                    }
                    if st.step > 0 && ui.button("Back").clicked() {
                        st.step -= 1;
                    }
                });
            });
        });
    if !open && out.is_none() {
        out = Some(Outcome::Dismiss);
    }
    out
}

/// Pages or One window, each a small picture of the window to pick.
fn workspace_card(ui: &mut egui::Ui, st: &mut Onboarding) {
    ui.heading("Your workspace");
    ui.add_space(4.0);
    ui.columns(2, |cols| {
        let cards = [
            (false, "Pages", "Media, Plan, Edit, Animate, Color, Audio and Export tabs in the top bar, each with its own layout"),
            (true, "One window", "One workspace, no page tabs: every panel is in Window and Ctrl+P, and stays where you dock it"),
        ];
        for (ui, (one, name, text)) in cols.iter_mut().zip(cards) {
            let on = st.one_window == one;
            let fill = if on { ui.visuals().selection.bg_fill.gamma_multiply(0.35) } else { ui.visuals().faint_bg_color };
            let stroke = if on { ui.visuals().selection.stroke } else { ui.visuals().widgets.noninteractive.bg_stroke };
            let r = egui::Frame::group(ui.style()).fill(fill).stroke(stroke).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let (pic, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 96.0), egui::Sense::hover());
                workspace_picture(ui, pic, one);
                ui.radio(on, name);
                ui.weak(text);
            });
            let card = ui.interact(r.response.rect, ui.id().with(("workspace", one)), egui::Sense::click());
            if card.on_hover_text("Window ▸ Layout switches between them any time").clicked() {
                st.one_window = one;
            }
        }
    });
}

/// A sketch of the window: the top bar (with page tabs for Pages), three panels over the timeline.
fn workspace_picture(ui: &egui::Ui, r: egui::Rect, one: bool) {
    let p = ui.painter();
    let v = ui.visuals();
    let (bg, panel, line, accent) = (v.extreme_bg_color, v.widgets.inactive.weak_bg_fill, v.weak_text_color(), v.selection.bg_fill);
    p.rect_filled(r, 4.0, bg);
    let bar = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), 12.0));
    for i in 0..4 {
        p.rect_filled(egui::Rect::from_min_size(bar.min + egui::vec2(5.0 + i as f32 * 9.0, 4.0), egui::vec2(6.0, 4.0)), 1.0, line);
    }
    if !one {
        for i in 0..5 {
            let at = egui::pos2(bar.center().x - 30.0 + i as f32 * 12.5, bar.top() + 2.5);
            p.rect_filled(egui::Rect::from_min_size(at, egui::vec2(10.0, 7.0)), 2.0, if i == 2 { accent } else { line });
        }
    }
    let body = egui::Rect::from_min_max(egui::pos2(r.left() + 3.0, bar.bottom() + 2.0), r.max - egui::vec2(3.0, 3.0));
    let top_h = body.height() * 0.58;
    let w = body.width();
    for (x0, x1) in [(0.0, 0.24), (0.25, 0.73), (0.74, 1.0)] {
        let cell = egui::Rect::from_min_max(egui::pos2(body.left() + w * x0, body.top()), egui::pos2(body.left() + w * x1, body.top() + top_h));
        p.rect_filled(cell, 2.0, panel);
    }
    p.rect_filled(egui::Rect::from_min_max(egui::pos2(body.left(), body.top() + top_h + 2.0), body.max), 2.0, panel);
}

fn ffmpeg_card(ui: &mut egui::Ui, st: &mut Onboarding) {
    ui.heading("ffmpeg");
    ui.add_space(4.0);
    let (found, text) = st.ffmpeg.get_or_insert_with(ffmpeg_status).clone();
    ui.horizontal(|ui| {
        let color = if found { ui.visuals().selection.bg_fill } else { ui.visuals().warn_fg_color };
        glyph_label(ui, if found { Glyph::Dot } else { Glyph::Cross }, color);
        ui.label(text);
    });
    if found {
        ui.weak("Exports, proxies, conversions and imports of unusual formats are all ready.");
    } else {
        ui.weak(
            "Playback of common formats works without it (Media Foundation), but exports, proxies, \
             conversions and some imports need it. Put ffmpeg.exe and ffprobe.exe next to \
             simple-editor.exe, or point Settings ▸ General ▸ ffmpeg folder at them.",
        );
    }
}

/// True when "Show all…" was clicked.
fn keys_card(ui: &mut egui::Ui, hotkeys: &Hotkeys) -> bool {
    ui.heading("Twelve keys to know");
    ui.add_space(4.0);
    egui::Grid::new("onboarding_keys").num_columns(4).spacing([14.0, 3.0]).show(ui, |ui| {
        for row in KEYS.chunks(2) {
            for &a in row {
                ui.label(a.label());
                let text = hotkeys.text(a);
                if text.is_empty() {
                    ui.weak(" - ");
                } else {
                    ui.monospace(text);
                }
            }
            ui.end_row();
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let tip = format!("The full keyboard map ({})", hotkeys.text(Action::CheatSheet));
        let all = ui.button("Show all…").on_hover_text(tip).clicked();
        ui.weak("Every one of them is remappable in Settings ▸ Hotkeys.");
        all
    })
    .inner
}

fn template_card(ui: &mut egui::Ui, st: &mut Onboarding, settings: &Settings) {
    ui.heading("Start with a format");
    ui.add_space(4.0);
    ui.radio_value(&mut st.template, None, "Keep the default - 1920×1080 at 60 fps");
    egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
        for (i, p) in PRESETS.iter().enumerate() {
            ui.horizontal(|ui| {
                glyph_label(ui, p.glyph, ui.visuals().text_color());
                ui.radio_value(&mut st.template, Some(i), format!("{} - {}×{} at {:.0} fps", p.name, p.w, p.h, p.fps));
            });
        }
    });
    if !settings.project_templates.is_empty() {
        ui.weak(format!(
            "Your own saved formats ({}) live in the Inspector's Format section.",
            settings.project_templates.len()
        ));
    }
    ui.add_space(6.0);
    ui.separator();
    ui.checkbox(
        &mut st.install_context_menu,
        "Add \"Edit with Simple Editor\" to Explorer's right-click menu for videos",
    );
    ui.weak(
        "Opt-in: the registry entry is written only when you press Finish with this ticked (never from a \
         debug build, never twice). Settings ▸ General adds or removes it later.",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Onboarding {
        let mut st = Onboarding::new(&Settings::default());
        st.ffmpeg = Some((true, "ffmpeg found: test".into())); // no PATH probe in tests
        st
    }

    #[test]
    fn install_guard_matches_app_new_verbatim() {
        // the pre-existing App::new guard: !cfg!(debug_assertions) && !is_installed()
        assert!(install_allowed(false, false));
        assert!(!install_allowed(true, false), "a debug build never writes the registry");
        assert!(!install_allowed(false, true), "already installed: nothing to re-point");
        assert!(!install_allowed(true, true));
    }

    #[test]
    fn onboarding_finish_installs_under_existing_guard() {
        // box ticked, guard allows: persisted, install called once
        let mut st = fresh();
        st.install_context_menu = true;
        let mut settings = Settings::default();
        let mut installs = 0;
        assert!(finish(&st, &mut settings, true, &mut || installs += 1));
        assert!(settings.onboarded);
        assert!(settings.context_menu);
        assert_eq!(installs, 1);
        assert_eq!(settings.layout_mode, "dynamic", "the wizard no longer asks about layouts");

        // ticked but the guard refuses (debug build / already installed): no install, choice still saved
        let mut settings = Settings::default();
        let mut installs = 0;
        assert!(!finish(&st, &mut settings, false, &mut || installs += 1));
        assert_eq!(installs, 0);
        assert!(settings.onboarded && settings.context_menu);

        // unticked: never installs even when allowed, and the setting records the opt-out
        let mut st = fresh();
        st.install_context_menu = false;
        let mut settings = Settings::default();
        let mut installs = 0;
        assert!(!finish(&st, &mut settings, true, &mut || installs += 1));
        assert_eq!(installs, 0);
        assert!(!settings.context_menu);
    }

    /// The Screens card is there only with two or more monitors, right after the workspace card.
    #[test]
    fn the_screens_card_shows_only_with_two_monitors() {
        let mut st = fresh();
        assert!(!st.cards().contains(&Card::Screens), "one monitor");
        st.monitors = 2;
        assert_eq!(st.cards()[..2], [Card::Workspace, Card::Screens]);
        st.monitors = 3;
        assert_eq!(st.cards().len(), 5);
    }

    #[test]
    fn new_mirrors_current_settings() {
        let mut s = Settings::default();
        s.context_menu = false;
        let st = Onboarding::new(&s);
        assert_eq!((st.step, st.install_context_menu, st.template), (0, false, None));
    }

    /// Every card lays out; Next/Back walk the steps; the X reports Dismiss; nothing is applied by
    /// `show` itself.
    #[test]
    fn wizard_walks_every_card_without_touching_settings() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let settings = Settings::default();
        let hk = Hotkeys::defaults();
        let mut st = fresh();
        for step in 0..st.cards().len() as u8 {
            st.step = step;
            for _ in 0..2 {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    assert_eq!(show(ctx, &mut st, &settings, &hk), None);
                });
            }
            assert_eq!(st.step, step);
        }
        assert!(!settings.onboarded);
        assert_eq!(st.ffmpeg.as_ref().map(|(f, _)| *f), Some(true), "the pre-seeded probe was not re-run");
    }

    /// 30 idle frames with the wizard open on each card: no repaint requested (the idle-CPU-0% gate).
    #[test]
    fn assert_no_idle_repaint_onboarding_open() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let settings = Settings::default();
        let hk = Hotkeys::defaults();
        let mut st = fresh();
        for step in 0..st.cards().len() as u8 {
            st.step = step;
            for _ in 0..30 {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    let _ = show(ctx, &mut st, &settings, &hk);
                });
            }
            assert!(!ctx.has_requested_repaint(), "idle welcome window (card {step}) requested a repaint");
        }
    }
}
