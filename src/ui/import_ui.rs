//! Import report window: after `engine::import::import_file`, show what came through and what did not —
//! a summary line (clips / tracks / missing media), then a scrollable table of `Issue`s grouped by level
//! (Ok / Warning / Skipped) with the subject and detail. Buttons: "Use this project" (replaces the current
//! one, with the usual unsaved-changes prompt), "Copy report" (markdown) and Close. Non-modal.

use crate::engine::import::{ImportReport, Level};
use crate::theme::Palette;
use eframe::egui;

#[derive(Default)]
pub struct ImportUi {
    pub open: bool,
    pub report: Option<ImportReport>,
}

/// True when the user accepted the imported project (the app swaps it in).
pub fn show(ctx: &egui::Context, state: &mut ImportUi, palette: &Palette) -> bool {
    if !state.open {
        return false;
    }
    let mut accepted = false;
    let mut open = state.open;
    let mut close = false;
    egui::Window::new("Import Report").open(&mut open).default_width(520.0).default_height(360.0).show(ctx, |ui| {
        let Some(r) = &state.report else {
            ui.weak("Nothing imported.");
            return;
        };
        ui.horizontal(|ui| {
            ui.strong(&r.project.name);
            ui.weak(format!("{}×{} @ {:.3} fps", r.project.width, r.project.height, r.project.fps));
        });
        ui.label(format!("{} clips on {} tracks", r.clips, r.tracks));
        ui.horizontal(|ui| {
            if r.missing_media > 0 {
                ui.colored_label(ui.visuals().warn_fg_color, format!("{} missing files", r.missing_media));
            }
            ui.weak(format!("{} problems, {} notes", r.problems(), r.ok()));
        });
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(220.0).show(ui, |ui| {
            egui::Grid::new("import_issues").num_columns(3).striped(true).spacing([10.0, 2.0]).show(ui, |ui| {
                // worst first: what the user has to act on is at the top
                for level in [Level::Skipped, Level::Warning, Level::Ok] {
                    for i in r.issues.iter().filter(|i| i.level == level) {
                        ui.colored_label(level_color(level, ui, palette), level_name(level));
                        ui.label(&i.subject);
                        ui.label(&i.detail);
                        ui.end_row();
                    }
                }
                if r.issues.is_empty() {
                    ui.weak("Everything mapped cleanly.");
                    ui.end_row();
                }
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.add_enabled(r.clips > 0, egui::Button::new("Use this project")).clicked() {
                accepted = true;
                close = true;
            }
            if ui.button("Copy report").clicked() {
                ui.ctx().copy_text(r.to_markdown());
            }
            if ui.button("Close").clicked() {
                close = true;
            }
        });
    });
    state.open = open && !close;
    if !state.open && !accepted {
        state.report = None;
    }
    accepted
}

fn level_name(level: Level) -> &'static str {
    match level {
        Level::Ok => "Ok",
        Level::Warning => "Warning",
        Level::Skipped => "Skipped",
    }
}

fn level_color(level: Level, ui: &egui::Ui, palette: &Palette) -> egui::Color32 {
    match level {
        Level::Ok => palette.text_dim,
        Level::Warning => ui.visuals().warn_fg_color,
        Level::Skipped => ui.visuals().error_fg_color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::import::Issue;
    use crate::model::Project;

    fn report(clips: usize) -> ImportReport {
        ImportReport {
            project: Project::new(),
            issues: vec![
                Issue { level: Level::Warning, subject: "clip 'a.mp4'".into(), detail: "media not found".into() },
                Issue { level: Level::Ok, subject: "sequence".into(), detail: "1920×1080".into() },
            ],
            clips,
            tracks: 2,
            missing_media: 1,
        }
    }

    /// Headless: lays out with and without a report, and stays closed once closed.
    #[test]
    fn show_headless() {
        let ctx = egui::Context::default();
        let palette = Palette::new(true, egui::Color32::BLUE);
        let mut state = ImportUi { open: true, report: Some(report(3)) };
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| assert!(!show(ctx, &mut state, &palette)));
        }
        assert!(state.open && state.report.is_some());
        // closed → the report is dropped, and a closed window reports nothing
        state.open = false;
        let _ = ctx.run(egui::RawInput::default(), |ctx| assert!(!show(ctx, &mut state, &palette)));
        assert!(state.report.is_some(), "a closed window is not asked to clean up");
        let mut empty = ImportUi { open: true, report: None };
        let _ = ctx.run(egui::RawInput::default(), |ctx| assert!(!show(ctx, &mut empty, &palette)));
    }
}
