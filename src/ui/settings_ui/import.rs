//! Hotkeys tab ▸ "Import from…" menu and its preview/confirm window (`keymaps::import` does the reading
//! and applying). Apply keeps a `Snapshot` so "Undo import" restores exactly what it overwrote.

use super::SettingsUi;
use crate::hotkeys::Hotkeys;
use crate::keymaps::import::{self, App, Plan};
use crate::settings::Settings;
use eframe::egui;

/// The menu button row. Returns true when settings changed (undo).
pub(super) fn import_row(ui: &mut egui::Ui, state: &mut SettingsUi, s: &mut Settings, hk: &mut Hotkeys) -> bool {
    let mut changed = false;
    crate::ui::menu::button(ui, "Import from…", |ui| {
        for app in App::ALL {
            let found = if import::installed(app) { "  (found)" } else { "" };
            if ui.button(format!("{}{found}", app.name())).clicked() {
                state.import_plan = Some(import::detect(app));
                ui.close();
            }
        }
        ui.separator();
        if ui.button("Choose file…").clicked() {
            ui.close();
            let pick = rfd::FileDialog::new().add_filter("Preferences / shortcuts", &["kys", "txt", "xml"]);
            if let Some(p) = pick.add_filter("Any", &["*"]).pick_file() {
                state.import_plan = Some(import::from_file(&p));
            }
        }
    });
    if state.import_undo.is_some() && ui.button("Undo import").clicked() {
        if let Some(snap) = state.import_undo.take() {
            snap.restore(s, hk);
            changed = true;
        }
    }
    changed
}

/// The confirm window, shown while `state.import_plan` is set. Returns true on Apply.
pub(super) fn import_window(ctx: &egui::Context, state: &mut SettingsUi, s: &mut Settings, hk: &mut Hotkeys) -> bool {
    let Some(plan) = &state.import_plan else { return false };
    let (mut apply, mut close) = (false, false);
    let title = format!("Import from {}", plan.app.map_or("file", App::name));
    egui::Window::new(title).collapsible(false).default_size([520.0, 480.0]).show(ctx, |ui| {
        body(ui, plan, hk);
        ui.separator();
        ui.horizontal(|ui| {
            if ui.add_enabled(!plan.is_empty(), egui::Button::new("Apply")).clicked() {
                apply = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if apply {
        if let Some(plan) = state.import_plan.take() {
            state.import_undo = Some(import::apply(&plan, s, hk));
            ctx.set_zoom_factor(s.ui_scale);
        }
    } else if close {
        state.import_plan = None;
    }
    apply
}

fn body(ui: &mut egui::Ui, plan: &Plan, hk: &Hotkeys) {
    for p in &plan.sources {
        ui.weak(format!("Read {}", p.display()));
    }
    for n in &plan.notes {
        ui.colored_label(egui::Color32::from_rgb(224, 150, 40), n);
    }
    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
        if let Some(p) = plan.preset {
            ui.strong(format!("Keyboard: built-in \"{p}\" preset (no custom shortcut file)"));
        }
        if !plan.keys.is_empty() {
            ui.strong(format!("Shortcuts ({})", plan.keys.len()));
            for (a, k, theirs) in &plan.keys {
                // `Hotkeys::set` unbinds whoever holds the chord now - say so before Apply
                let steals = hk.conflict(*k).filter(|o| o != a && !plan.keys.iter().any(|(b, ..)| b == o));
                let note = steals.map(|o| format!("  (unbinds {})", o.label())).unwrap_or_default();
                ui.label(format!("{}  →  {}{note}", a.label(), Hotkeys::format(k))).on_hover_text(theirs);
            }
        }
        if !plan.settings.is_empty() {
            ui.strong("Settings");
            for c in &plan.settings {
                ui.label(c.label());
            }
        }
        if !plan.skipped.is_empty() {
            ui.collapsing(format!("Skipped ({})", plan.skipped.len()), |ui| {
                for s in &plan.skipped {
                    ui.weak(s);
                }
            });
        }
        if plan.is_empty() {
            ui.label("Nothing to import.");
        }
    });
}
