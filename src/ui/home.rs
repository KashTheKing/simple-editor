//! ---- ws:layout-modes-onboarding ----
//! The central empty state / home screen: an `egui::Area` of Open / Import / Templates / Recent cards
//! floating over the panes while the project AND the library are empty (`Settings.home_screen`,
//! dismissable for the session). It never replaces the layout — the timeline, library and preview are
//! all still there behind it, so a dropped file works exactly as before and the cards simply vanish
//! the moment there is something to edit. No `App` in here (see `ui::app::layout_ctl::home_window`
//! for the glue that turns a `HomeAction` into real actions); text + path cards only — ponytail: no
//! project thumbnails until a project manifest stores one.

use crate::settings::Settings;
use crate::ui::guides::PRESETS;
use crate::ui::tools::{glyph_text_button, Glyph};
use eframe::egui;
use std::path::Path;

/// What a card asked for. `New(Some(i))` = start from `guides::PRESETS[i]`; `New(None)` = the default
/// blank 1080p60 project (which is what an empty project already is — the caller just dismisses).
#[derive(Clone, Debug, PartialEq)]
pub enum HomeAction {
    Open,
    Import,
    New(Option<usize>),
    OpenRecent(String),
    Dismiss,
}

/// The gate `show` applies: enabled in Settings, nothing to edit yet, not dismissed this session.
pub fn visible(settings: &Settings, empty: bool, dismissed: bool) -> bool {
    settings.home_screen && empty && !dismissed
}

/// Draws the cards when `visible`; `Some(action)` on a click. `empty` = project has no clips AND the
/// library has no assets (the caller computes it — see `App::project`).
pub fn show(ctx: &egui::Context, settings: &Settings, empty: bool, dismissed: bool) -> Option<HomeAction> {
    if !visible(settings, empty, dismissed) {
        return None;
    }
    let mut out = None;
    egui::Area::new(egui::Id::new("home_screen"))
        .order(egui::Order::Middle)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -40.0))
        .show(ctx, |ui| {
            egui::Frame::window(&ctx.style()).show(ui, |ui| {
                ui.set_width(440.0);
                ui.horizontal(|ui| {
                    ui.heading("Simple Editor");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if glyph_text_button(ui, Glyph::Cross, "").on_hover_text("Hide for this session").clicked() {
                            out = Some(HomeAction::Dismiss);
                        }
                    });
                });
                ui.weak("Drop video, audio or images anywhere in the window — or start here.");
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if glyph_text_button(ui, Glyph::Folder, "Open…").on_hover_text("A video or a .sedit project (Ctrl+O)").clicked() {
                        out = Some(HomeAction::Open);
                    }
                    if glyph_text_button(ui, Glyph::ImportArrow, "Import media…").on_hover_text("Into the library (Ctrl+I)").clicked() {
                        out = Some(HomeAction::Import);
                    }
                    if glyph_text_button(ui, Glyph::Clapperboard, "Blank project").on_hover_text("1920×1080 at 60 fps").clicked() {
                        out = Some(HomeAction::New(None));
                    }
                });
                ui.add_space(6.0);
                ui.strong("Start from a format");
                ui.horizontal_wrapped(|ui| {
                    for (i, p) in PRESETS.iter().enumerate() {
                        if glyph_text_button(ui, p.glyph, p.name)
                            .on_hover_text(format!("{}×{} at {:.0} fps", p.w, p.h, p.fps))
                            .clicked()
                        {
                            out = Some(HomeAction::New(Some(i)));
                        }
                    }
                });
                ui.add_space(6.0);
                ui.strong("Recent projects");
                if settings.recent_projects.is_empty() {
                    ui.weak("(none yet)");
                }
                for r in settings.recent_projects.iter().take(5) {
                    let p = Path::new(r);
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| r.clone());
                    let folder = p.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
                    let b = egui::Button::new(name).shortcut_text(folder).wrap_mode(egui::TextWrapMode::Truncate);
                    if ui.add(b).on_hover_text(r).clicked() {
                        out = Some(HomeAction::OpenRecent(r.clone()));
                    }
                }
            });
        });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_hidden_when_disabled_or_project_nonempty() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut settings = Settings::default();
        assert!(visible(&settings, true, false));
        assert!(!visible(&settings, false, false), "something to edit: no home screen");
        assert!(!visible(&settings, true, true), "dismissed for the session");
        settings.home_screen = false;
        assert!(!visible(&settings, true, false), "switched off in Settings");
        // and `show` draws nothing at all in those cases (no area, no repaint, no action)
        for (s, empty, dismissed) in [(&settings, true, false), (&Settings::default(), false, false), (&Settings::default(), true, true)] {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                assert_eq!(show(ctx, s, empty, dismissed), None);
                // egui always has a Background layer even with nothing drawn; check the home area's
                // own layer specifically rather than the whole (never-empty) visible set.
                let home_layer = egui::LayerId::new(egui::Order::Middle, egui::Id::new("home_screen"));
                assert!(!ctx.memory(|m| m.areas().visible_layer_ids().contains(&home_layer)), "the home area must not be laid out");
            });
        }
        // visible: draws, no action without a click
        let settings = Settings::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            assert_eq!(show(ctx, &settings, true, false), None);
        });
    }

    /// 30 idle frames with the cards showing (recent projects listed too): no repaint requested.
    #[test]
    fn assert_no_idle_repaint_home_open() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut settings = Settings::default();
        settings.recent_projects = vec![r"C:\edits\a.sedit".into(), r"C:\edits\b.sedit".into()];
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                let _ = show(ctx, &settings, true, false);
            });
        }
        assert!(!ctx.has_requested_repaint(), "idle home screen requested a repaint");
    }
}
