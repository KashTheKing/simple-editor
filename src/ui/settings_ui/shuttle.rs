//! ws:parity-io: Settings ▸ Hotkeys ▸ Jog / Shuttle - the Contour ShuttlePRO / ShuttleXpress switch and
//! what each of its buttons does (`engine::shuttle`, driven by `ui::app::io_ctl`).

use crate::engine::shuttle;
use crate::hotkeys::Action;
use crate::settings::Settings;
use eframe::egui;

pub(super) fn shuttle_section(ui: &mut egui::Ui, s: &mut Settings) -> bool {
    let mut changed = false;
    ui.add_space(8.0);
    ui.separator();
    ui.horizontal(|ui| {
        ui.strong("Jog / Shuttle");
        ui.weak(shuttle::status().unwrap_or("not connected"))
            .on_hover_text("A Contour ShuttlePRO, ShuttlePRO v2 or ShuttleXpress is found when it is plugged in");
    });
    changed |= ui
        .checkbox(&mut s.shuttle, "Use a ShuttlePRO")
        .on_hover_text("The outer ring shuttles (further = faster, either way), the inner wheel steps frames")
        .changed();
    if !s.shuttle {
        return changed;
    }
    if s.shuttle_buttons.len() < shuttle::BUTTONS {
        s.shuttle_buttons.resize(shuttle::BUTTONS, String::new());
    }
    egui::CollapsingHeader::new("Buttons").id_salt("shuttle_buttons").show(ui, |ui| {
        egui::Grid::new("shuttle_grid").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
            for (i, id) in s.shuttle_buttons.iter_mut().enumerate().take(shuttle::BUTTONS) {
                ui.label(format!("Button {}", i + 1));
                let label = Action::from_id(id).map_or("Nothing", |a| a.label());
                egui::ComboBox::from_id_salt(("shuttle_btn", i)).selected_text(label).width(260.0).show_ui(ui, |ui| {
                    if ui.selectable_label(id.is_empty(), "Nothing").clicked() {
                        id.clear();
                        changed = true;
                    }
                    for a in Action::ALL {
                        if ui.selectable_label(id == a.id(), a.label()).clicked() {
                            *id = a.id().to_string();
                            changed = true;
                        }
                    }
                });
                ui.end_row();
            }
        });
        if ui.button("Reset Buttons").clicked() {
            s.shuttle_buttons = shuttle::default_buttons();
            changed = true;
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An idle Jog / Shuttle section draws without asking for another frame (open and closed buttons list).
    #[test]
    fn assert_no_idle_repaint_shuttle_settings() {
        let ctx = egui::Context::default();
        let mut s = Settings::default();
        for on in [true, false] {
            s.shuttle = on;
            for _ in 0..20 {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let _ = shuttle_section(ui, &mut s);
                    });
                });
            }
            assert!(!ctx.has_requested_repaint(), "an idle Jog / Shuttle section must not repaint (on: {on})");
        }
        assert_eq!(s.shuttle_buttons, shuttle::default_buttons(), "drawing changes nothing");
    }
}
