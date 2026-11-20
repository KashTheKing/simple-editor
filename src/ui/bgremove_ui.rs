//! ---- ws:addon-bg-removal ----
//! The Inspector's Remove Background section for a video / picture clip: quality, the matte's status
//! (install, computing with cancel, frames ready), Edge and Show Matte. Starting work goes through the
//! Remove Background actions (`ui::app::bgremove_ctl::act`), so the menu, palette and this section
//! behave the same.

use crate::engine::bgremove;
use crate::hotkeys::Action;
use crate::model::{BgQuality, BgRemoval, Id, Project};
use eframe::egui;
use std::sync::atomic::Ordering;

/// What the section needs from the App this frame (the section itself only edits the clip).
pub struct State {
    /// (cached, total) matte frames of what the clip shows.
    pub coverage: Option<(usize, usize)>,
    /// The job computing this clip's mattes.
    pub job: Option<std::sync::Arc<crate::engine::export::Progress>>,
}

/// Draw the section. Edits to `Clip.bg_removal` are committed straight to `project` (an undo snapshot
/// first, once per drag). Returns true when the clip changed.
pub fn section(ui: &mut egui::Ui, project: &mut Project, id: Id, st: &State, undo: &mut dyn FnMut(&Project)) -> bool {
    let Some(cur) = project.clip(id).map(|c| c.bg_removal.clone()) else { return false };
    let mut changed = false;
    let commit = |project: &mut Project, bg: Option<BgRemoval>| {
        if let Some(c) = project.clip_mut(id) {
            c.bg_removal = bg;
        }
    };
    let Some(mut bg) = cur else {
        ui.horizontal(|ui| {
            if ui.button("Fast").on_hover_text("MODNet: quick, made for people").clicked() {
                crate::ui::inspector::set_pending_action(Action::RemoveBackgroundFast);
            }
            if ui.button("Best").on_hover_text("BiRefNet: slower, any subject, finer edges").clicked() {
                crate::ui::inspector::set_pending_action(Action::RemoveBackgroundBest);
            }
        });
        return false;
    };
    egui::Grid::new(("bgremove", id)).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label("Quality");
        ui.horizontal(|ui| {
            for (q, tip) in [(BgQuality::Fast, "MODNet: quick, made for people"), (BgQuality::Best, "BiRefNet: slower, any subject, finer edges")] {
                if ui.selectable_label(bg.quality == q, q.label()).on_hover_text(tip).clicked() && bg.quality != q {
                    // changing it starts that model's frames (the action sets the quality, one undo step)
                    crate::ui::inspector::set_pending_action(match q {
                        BgQuality::Fast => Action::RemoveBackgroundFast,
                        BgQuality::Best => Action::RemoveBackgroundBest,
                    });
                }
            }
            if ui.small_button("Off").on_hover_text("Turn background removal off").clicked() {
                crate::ui::inspector::set_pending_action(Action::RemoveBackgroundOff);
            }
        });
        ui.end_row();

        ui.label("Matte");
        ui.horizontal(|ui| status(ui, &bg, st));
        ui.end_row();

        ui.label("Edge");
        let r = ui
            .add(egui::Slider::new(&mut bg.edge, -1.0..=1.0).fixed_decimals(2))
            .on_hover_text("Below 0 shrinks the matte (harder edge), above 0 feathers it");
        // one undo step per drag (or per click / typed value)
        if r.drag_started() || (r.changed() && !r.dragged()) {
            undo(project);
        }
        if r.changed() {
            commit(project, Some(bg.clone()));
            changed = true;
        }
        ui.end_row();

        ui.label("Show Matte");
        let mut show = bg.show_matte;
        if ui.checkbox(&mut show, "").on_hover_text("Draw the matte (white = kept) instead of the picture").changed() {
            bg.show_matte = show;
            undo(project);
            commit(project, Some(bg.clone()));
            changed = true;
        }
        ui.end_row();
    });
    changed
}

fn status(ui: &mut egui::Ui, bg: &BgRemoval, st: &State) {
    if let Some(p) = bgremove::installing() {
        ui.add(egui::Spinner::new());
        ui.weak(format!("Installing {:.0}%", p.fraction() * 100.0)).on_hover_text(p.status());
        return;
    }
    if !bgremove::available(bg.quality) {
        ui.weak("Not installed");
        if ui.small_button("Install").on_hover_text("Download the add-on for this quality (verified, from the library)").clicked() {
            bgremove::start_install(bg.quality);
        }
        if let Some(e) = bgremove::install_error() {
            ui.colored_label(ui.visuals().error_fg_color, "failed").on_hover_text(e);
        }
        return;
    }
    if let Some(p) = &st.job {
        ui.add(egui::ProgressBar::new(p.fraction()).desired_width(90.0).show_percentage()).on_hover_text(p.status());
        if ui.small_button("Cancel").clicked() {
            p.cancel.store(true, Ordering::Release);
        }
        return;
    }
    match st.coverage {
        Some((have, total)) if have >= total => {
            ui.weak("Ready");
        }
        Some((have, total)) => {
            ui.weak(format!("{have} / {total} frames")).on_hover_text("Frames without a matte draw unkeyed");
            let a = match bg.quality {
                BgQuality::Fast => Action::RemoveBackgroundFast,
                BgQuality::Best => Action::RemoveBackgroundBest,
            };
            if ui.small_button("Compute").clicked() {
                crate::ui::inspector::set_pending_action(a);
            }
        }
        None => {}
    }
}
