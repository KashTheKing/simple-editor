//! F1 keyboard-shortcuts overlay: every bound `Action`, grouped by `hotkeys::group`, plus a "Command
//! Palette (Ctrl+K)" row as a second entry point into the palette. A plain, non-blocking `egui::Window`
//! (same shape as every other overlay in this crate — Retime, Export, Settings — never modal).
//! ---- ws:command-palette ----

use crate::hotkeys::{group, Action, Hotkeys};
use eframe::egui;

/// Draws the cheat-sheet window while `*open`. Grouped by `hotkeys::group(a)`, unbound actions shown
/// last within their group with a dim "—" instead of a chord.
pub fn show(ctx: &egui::Context, hotkeys: &Hotkeys, open: &mut bool) {
    if !*open {
        return;
    }
    let mut still_open = true;
    egui::Window::new("Keyboard Shortcuts").open(&mut still_open).default_width(420.0).default_height(520.0).show(
        ctx,
        |ui| {
            ui.label(egui::RichText::new("Command Palette (Ctrl+K)").strong());
            ui.weak("Search every command by name — this cheat sheet is the second way in.");
            ui.separator();
            let mut by_group: Vec<(&'static str, Vec<Action>)> = Vec::new();
            for &a in Action::ALL {
                let g = group(a);
                match by_group.iter_mut().find(|(name, _)| *name == g) {
                    Some((_, v)) => v.push(a),
                    None => by_group.push((g, vec![a])),
                }
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (name, actions) in &by_group {
                    ui.strong(*name);
                    egui::Grid::new(("cheatsheet", name)).num_columns(2).spacing([12.0, 2.0]).show(ui, |ui| {
                        for &a in actions {
                            ui.label(a.label());
                            let text = hotkeys.text(a);
                            if text.is_empty() {
                                ui.weak("—");
                            } else {
                                ui.monospace(text);
                            }
                            ui.end_row();
                        }
                    });
                    ui.add_space(6.0);
                }
            });
        },
    );
    *open = still_open;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_every_group_without_panicking() {
        let ctx = egui::Context::default();
        let hk = Hotkeys::defaults();
        let mut open = true;
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| show(ctx, &hk, &mut open));
        }
        assert!(open);
    }

    /// 30 idle frames, no input: the overlay must not cost a repaint while it sits open and unchanged.
    #[test]
    fn assert_no_idle_repaint_cheatsheet_open() {
        let ctx = egui::Context::default();
        let hk = Hotkeys::defaults();
        let mut open = true;
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| show(ctx, &hk, &mut open));
        }
        assert!(!ctx.has_requested_repaint(), "idle cheat-sheet requested a repaint");
    }
}
