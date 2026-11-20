//! ---- ws:gallery-stock ----
//! Settings ▸ Stock: which stock sources Gallery ▸ Stock searches, the user's own keys for the keyed ones
//! (Windows Credential Manager, `engine::stock::keys`), and where downloads go.

use crate::engine::stock::{keys, Source};
use crate::settings::Settings;
use eframe::egui;

fn draft_id(s: Source) -> egui::Id {
    egui::Id::new(("stock_key_draft", s.id()))
}

pub(super) fn stock_tab(ui: &mut egui::Ui, s: &mut Settings) -> bool {
    let mut changed = false;
    ui.strong("Sources");
    egui::Grid::new("stock_sources").num_columns(4).spacing([12.0, 6.0]).striped(true).show(ui, |ui| {
        for src in Source::ALL {
            let id = src.id().to_string();
            ui.label(src.name()).on_hover_text(src.site());
            let mut on = if src.needs_key() { s.stock_on.contains(&id) } else { !s.stock_off.contains(&id) };
            if ui.checkbox(&mut on, "On").changed() {
                if src.needs_key() {
                    s.stock_on.retain(|x| *x != id);
                    if on {
                        s.stock_on.push(id.clone());
                    }
                } else {
                    s.stock_off.retain(|x| *x != id);
                    if !on {
                        s.stock_off.push(id.clone());
                    }
                }
                changed = true;
            }
            if src.needs_key() {
                let have = keys::get(src).is_some();
                ui.horizontal(|ui| {
                    let mut draft: String = ui.ctx().data(|d| d.get_temp(draft_id(src))).unwrap_or_default();
                    let hint = if have { "Key saved" } else { "Paste your free API key" };
                    let r = ui.add(egui::TextEdit::singleline(&mut draft).password(true).hint_text(hint).desired_width(200.0));
                    if r.changed() {
                        ui.ctx().data_mut(|d| d.insert_temp(draft_id(src), draft.clone()));
                    }
                    let save = ui.add_enabled(!draft.trim().is_empty(), egui::Button::new("Save"));
                    if save.on_hover_text("Kept in Windows Credential Manager on this PC").clicked() {
                        if keys::set(src, &draft).is_ok() {
                            ui.ctx().data_mut(|d| d.remove::<String>(draft_id(src)));
                            if !s.stock_on.contains(&id) {
                                s.stock_on.push(id.clone());
                                changed = true;
                            }
                        }
                    }
                    if have && ui.button("Remove").on_hover_text("Forget the saved key").clicked() {
                        let _ = keys::set(src, "");
                    }
                });
                if ui.button("Get a key").on_hover_text(src.site()).clicked() {
                    crate::engine::browsers::open_url(src.site());
                }
            } else {
                ui.weak("No key needed");
                if ui.button("Open site").on_hover_text(src.site()).clicked() {
                    crate::engine::browsers::open_url(src.site());
                }
            }
            ui.end_row();
        }
    });
    ui.add_space(8.0);
    ui.strong("Downloads");
    changed |= ui
        .checkbox(&mut s.stock_in_project, "Save in the project's folder")
        .on_hover_text("A Stock Footage folder beside the saved project. Unsaved projects use the app's own Stock Footage folder.")
        .changed();
    if ui.button("Open Stock Footage folder").clicked() {
        let d = Settings::dir().join("Stock Footage");
        let _ = std::fs::create_dir_all(&d);
        let _ = std::process::Command::new("explorer").arg(&d).spawn();
    }
    changed |= ui.checkbox(&mut s.stock_preview, "Preview on hover").on_hover_text("A few frames of a video, or a quiet snippet of a sound").changed();
    ui.add_space(8.0);
    ui.strong("More sites");
    ui.horizontal_wrapped(|ui| {
        for (name, url) in crate::engine::stock::LINK_ONLY {
            if ui.button(name).on_hover_text(url).clicked() {
                crate::engine::browsers::open_url(url);
            }
        }
    });
    changed
}
