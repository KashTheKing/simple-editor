//! The Hotkeys tab: table of every Action with its current binding, rebind/reset controls, and the
//! rebind-capture logic. Extracted verbatim from settings_ui.rs (see mod.rs's module doc for the whole
//! settings window). `capture` was renamed `capture_key` here to avoid a name clash with capture.rs's
//! `capture_tab` — a local rename only, nothing outside settings_ui.rs called the old private name.

use super::SettingsUi;
use crate::hotkeys::{Action, Hotkeys};
use eframe::egui::{self, Event, Key, KeyboardShortcut, Modifiers};

pub(super) fn hotkeys_tab(ui: &mut egui::Ui, state: &mut SettingsUi, hotkeys: &mut Hotkeys) -> bool {
    let mut changed = false;
    if let Some(a) = state.rebinding {
        changed |= capture_key(ui.ctx(), state, a, hotkeys);
    }
    if ui.button("Reset all").clicked() {
        hotkeys.reset_all();
        state.rebinding = None;
        state.note.clear();
        changed = true;
    }
    ui.add_space(4.0);
    egui::ScrollArea::vertical().max_height((ui.available_height() - 40.0).max(0.0)).auto_shrink([false, true]).show(
        ui,
        |ui| {
            egui::Grid::new("hotkeys").num_columns(4).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
                for &a in Action::ALL {
                    ui.label(a.label());
                    let text = hotkeys.text(a);
                    if state.rebinding == Some(a) {
                        ui.strong("…");
                    } else if text.is_empty() {
                        ui.weak("—");
                    } else {
                        ui.label(text);
                    }
                    if ui.small_button("Rebind").clicked() {
                        state.rebinding = Some(a);
                        state.note.clear();
                    }
                    if ui.small_button("Reset").clicked() {
                        let before = hotkeys.get(a);
                        hotkeys.reset(a);
                        changed |= hotkeys.get(a) != before;
                    }
                    ui.end_row();
                }
            });
        },
    );
    ui.add_space(4.0);
    if let Some(a) = state.rebinding {
        let r = ui.strong(format!("Press keys for \"{}\"… (Esc cancels, Backspace/Delete unbinds)", a.label()));
        // Hold keyboard focus so the app's hotkey polling (which skips while a widget has focus) stays quiet.
        r.request_focus();
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                r.id,
                egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true },
            )
        });
    } else if !state.note.is_empty() {
        ui.label(&state.note);
    }
    ui.weak("Mouse: Ctrl+Scroll zoom, Shift+Scroll pan, Alt+Scroll track height (fixed).");
    changed
}

/// While rebinding: take the first key press this frame, bind/unbind/cancel, and swallow all key/text
/// events so nothing else reacts to them. Returns true if a binding changed.
pub(super) fn capture_key(ctx: &egui::Context, state: &mut SettingsUi, a: Action, hotkeys: &mut Hotkeys) -> bool {
    let mut changed = false;
    ctx.input_mut(|i| {
        let pressed = i.events.iter().find_map(|e| match e {
            Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
            _ => None,
        });
        if let Some((key, m)) = pressed {
            state.rebinding = None;
            match key {
                Key::Escape => {}
                Key::Backspace | Key::Delete => {
                    changed = hotkeys.get(a).is_some();
                    hotkeys.set(a, None);
                }
                key => {
                    let ks = KeyboardShortcut::new(
                        Modifiers {
                            alt: m.alt,
                            ctrl: m.ctrl || m.command,
                            shift: m.shift,
                            mac_cmd: false,
                            command: false,
                        },
                        key,
                    );
                    if let Some(other) = hotkeys.conflict(ks).filter(|&o| o != a) {
                        state.note =
                            format!("{} was using {} and is now unbound.", other.label(), Hotkeys::format(&ks));
                    }
                    hotkeys.set(a, Some(ks));
                    changed = true;
                }
            }
        }
        i.events.retain(|e| !matches!(e, Event::Key { .. } | Event::Text(_)));
    });
    changed
}
