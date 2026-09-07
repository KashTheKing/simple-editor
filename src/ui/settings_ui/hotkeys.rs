//! The Hotkeys tab: search + group-headed table of every Action with its current binding, rebind/reset
//! controls and the rebind-capture logic; a painted QWERTY map coloured by binding state; an inline
//! Reassign/Keep row when a rebind collides with another action; a keymap-preset combo
//! (Simple Editor/Premiere/Resolve/Avid); a UI-scale slider. Extracted verbatim from settings_ui.rs (see
//! mod.rs's module doc for the whole settings window). `capture` was renamed `capture_key` here to avoid
//! a name clash with capture.rs's `capture_tab` — a local rename only, nothing outside settings_ui.rs
//! called the old private name.
//! ---- ws:command-palette ----: search/group headers, the QWERTY map, Reassign/Keep, keymap presets,
//! the UI-scale slider, and `capture_key`'s honest `conflict_all` (RESERVED-aware) rewrite.

use super::SettingsUi;
use crate::hotkeys::{group, Action, Claim, Hotkeys, RESERVED};
use crate::keymaps;
use crate::settings::Settings;
use eframe::egui::{self, Color32, Event, Key, KeyboardShortcut, Modifiers};

pub(super) fn hotkeys_tab(
    ui: &mut egui::Ui,
    state: &mut SettingsUi,
    hotkeys: &mut Hotkeys,
    settings: &mut Settings,
) -> bool {
    let mut changed = false;
    if let Some(a) = state.rebinding {
        changed |= capture_key(ui.ctx(), state, a, hotkeys);
    }

    ui.horizontal(|ui| {
        if ui.button("Reset all").clicked() {
            hotkeys.reset_all();
            state.rebinding = None;
            state.pending_conflict = None;
            state.note.clear();
            changed = true;
        }
        ui.separator();
        ui.label("Keymap preset");
        let before = settings.keymap_preset.clone();
        egui::ComboBox::from_id_salt("keymap_preset").selected_text(&settings.keymap_preset).show_ui(ui, |ui| {
            for (name, _) in keymaps::PRESETS {
                ui.selectable_value(&mut settings.keymap_preset, name.to_string(), *name);
            }
        });
        if settings.keymap_preset != before {
            if let Err(e) = keymaps::apply(&settings.keymap_preset, hotkeys) {
                state.note = e;
                settings.keymap_preset = before;
            } else {
                state.pending_conflict = None;
                changed = true;
            }
        }
    });

    ui.horizontal(|ui| {
        ui.label("UI scale");
        if ui.add(egui::Slider::new(&mut settings.ui_scale, 0.5..=2.5).fixed_decimals(2)).changed() {
            ui.ctx().set_zoom_factor(settings.ui_scale);
            changed = true;
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label("Search");
        ui.text_edit_singleline(&mut state.hotkeys_search);
        if !state.hotkeys_search.is_empty() && ui.small_button("Clear").clicked() {
            state.hotkeys_search.clear();
        }
    });

    // inline Reassign/Keep row on conflict — replaces the old silent-auto-unbind behaviour
    if let Some((a, ks, other)) = state.pending_conflict.clone() {
        ui.add_space(4.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.colored_label(
                Color32::from_rgb(224, 150, 40),
                format!("{} already uses {} — rebind {} to it too?", other.label(), Hotkeys::format(&ks), a.label()),
            );
            ui.horizontal(|ui| {
                if ui.button("Reassign").clicked() {
                    hotkeys.set(a, Some(ks));
                    state.pending_conflict = None;
                    changed = true;
                }
                if ui.button("Keep existing").clicked() {
                    state.pending_conflict = None;
                }
            });
        });
    }

    draw_qwerty(ui, hotkeys);

    ui.add_space(4.0);
    let filter = state.hotkeys_search.to_ascii_lowercase();
    let mut by_group: Vec<(&'static str, Vec<Action>)> = Vec::new();
    for &a in Action::ALL {
        if !filter.is_empty() && !a.label().to_ascii_lowercase().contains(&filter) && !a.id().contains(&filter) {
            continue;
        }
        let g = group(a);
        match by_group.iter_mut().find(|(name, _)| *name == g) {
            Some((_, v)) => v.push(a),
            None => by_group.push((g, vec![a])),
        }
    }
    egui::ScrollArea::vertical().max_height((ui.available_height() - 40.0).max(0.0)).auto_shrink([false, true]).show(
        ui,
        |ui| {
            for (name, actions) in &by_group {
                ui.strong(*name);
                egui::Grid::new(("hotkeys", name)).num_columns(4).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
                    for &a in actions {
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
                            state.pending_conflict = None;
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
                ui.add_space(6.0);
            }
            if by_group.is_empty() {
                ui.weak("No actions match the search.");
            }
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
/// events so nothing else reacts to them. Returns true if a binding changed. A chord already claimed by
/// a `RESERVED` row (bare `S`, `Ctrl+Y`, ...) is rejected outright (a note explains why); a chord
/// already bound to another `Action` opens the Reassign/Keep row (`state.pending_conflict`) instead of
/// silently stealing it.
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
                    match hotkeys.conflict_all(ks) {
                        Some(Claim::Fixed(name)) => {
                            state.note =
                                format!("{} is reserved by {name} and can't be rebound.", Hotkeys::format(&ks));
                        }
                        Some(Claim::Action(other)) if other != a => {
                            state.pending_conflict = Some((a, ks, other));
                        }
                        _ => {
                            hotkeys.set(a, Some(ks));
                            changed = true;
                        }
                    }
                }
            }
        }
        i.events.retain(|e| !matches!(e, Event::Key { .. } | Event::Text(_)));
    });
    changed
}

/// Bare-key (no-modifier) binding state used to colour the QWERTY map.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyState {
    /// The bare key itself is bound to an `Action`.
    Bound,
    /// Free bare, but SOME modifier combo on this key is bound to an `Action`.
    ModifierBound,
    /// Not bound at all, and not reserved.
    Free,
    /// The bare key is one of `hotkeys::RESERVED`'s hard-coded chords.
    Reserved,
}

fn key_state(hotkeys: &Hotkeys, key: Key) -> KeyState {
    if RESERVED.iter().any(|&(_, m, k)| k == key && m == Modifiers::NONE) {
        return KeyState::Reserved;
    }
    if hotkeys.conflict(KeyboardShortcut::new(Modifiers::NONE, key)).is_some() {
        return KeyState::Bound;
    }
    if hotkeys.extra().iter().any(|(_, ks)| ks.logical_key == key && ks.modifiers == Modifiers::NONE) {
        return KeyState::Bound; // a script's own bare-key @hotkey
    }
    let any_modifier_bound = Action::ALL.iter().any(|&a| hotkeys.get(a).is_some_and(|ks| ks.logical_key == key))
        || hotkeys.extra().iter().any(|(_, ks)| ks.logical_key == key);
    if any_modifier_bound {
        KeyState::ModifierBound
    } else {
        KeyState::Free
    }
}

/// Every action (any modifier), live script `@hotkey` and reserved use bound to `key` — the QWERTY
/// cell's hover text.
fn uses_of(hotkeys: &Hotkeys, key: Key) -> Vec<String> {
    let mut out: Vec<String> = Action::ALL
        .iter()
        .filter(|&&a| hotkeys.get(a).is_some_and(|ks| ks.logical_key == key))
        .map(|&a| format!("{} — {}", Hotkeys::format(&hotkeys.get(a).unwrap()), a.label()))
        .collect();
    out.extend(
        hotkeys
            .extra()
            .iter()
            .filter(|(_, ks)| ks.logical_key == key)
            .map(|(name, ks)| format!("{} — script: {name}", Hotkeys::format(ks))),
    );
    out.extend(
        RESERVED
            .iter()
            .filter(|&&(_, _, k)| k == key)
            .map(|&(name, m, k)| format!("{} — {name}", Hotkeys::format(&KeyboardShortcut::new(m, k)))),
    );
    out
}

const QWERTY_ROWS: [&[Key]; 3] = [
    &[Key::Q, Key::W, Key::E, Key::R, Key::T, Key::Y, Key::U, Key::I, Key::O, Key::P],
    &[Key::A, Key::S, Key::D, Key::F, Key::G, Key::H, Key::J, Key::K, Key::L],
    &[Key::Z, Key::X, Key::C, Key::V, Key::B, Key::N, Key::M],
];

/// The QWERTY map: one rect per letter, filled by `key_state`'s colour, hover shows every chord on
/// that key (any modifier) plus any reserved use. Purely visual/informational — clicking a cell does
/// nothing; rebinding still goes through the "Rebind" button + a real key press (real Ctrl/Shift/Alt
/// chords need modifier keys down, which a click can't express).
fn draw_qwerty(ui: &mut egui::Ui, hotkeys: &Hotkeys) {
    let cell = egui::vec2(28.0, 26.0);
    let gap = 3.0;
    ui.add_space(4.0);
    for (row_i, row) in QWERTY_ROWS.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.add_space(row_i as f32 * cell.x * 0.5);
            for &key in *row {
                let (rect, resp) = ui.allocate_exact_size(cell, egui::Sense::hover());
                let state = key_state(hotkeys, key);
                let fill = match state {
                    KeyState::Bound => Color32::from_rgb(70, 130, 90),
                    KeyState::ModifierBound => Color32::from_rgb(70, 100, 140),
                    KeyState::Free => ui.visuals().widgets.inactive.bg_fill,
                    KeyState::Reserved => Color32::from_rgb(150, 70, 60),
                };
                ui.painter().rect_filled(rect, 3.0, fill);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    key.name(),
                    egui::FontId::monospace(12.0),
                    ui.visuals().strong_text_color(),
                );
                if resp.hovered() {
                    let uses = uses_of(hotkeys, key);
                    let text = if uses.is_empty() { "Free".to_string() } else { uses.join("\n") };
                    resp.on_hover_text(text);
                }
                ui.add_space(gap);
            }
        });
    }
    ui.horizontal(|ui| {
        for (color, label) in [
            (Color32::from_rgb(70, 130, 90), "bound"),
            (Color32::from_rgb(70, 100, 140), "bound w/ modifier"),
            (ui.visuals().widgets.inactive.bg_fill, "free"),
            (Color32::from_rgb(150, 70, 60), "reserved"),
        ] {
            let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 2.0, color);
            ui.label(label);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two headless frames, no input between them: the tab (search box, painted QWERTY grid,
    /// keymap-preset combo, UI-scale slider) reports no change on either call, and the second call
    /// requests no repaint — same shape as palette.rs's/cheatsheet.rs's `assert_no_idle_repaint_*` tests.
    #[test]
    fn hotkeys_tab_show_headless_no_change() {
        let ctx = egui::Context::default();
        let mut hk = Hotkeys::defaults();
        let mut settings = Settings::default();
        let mut state = SettingsUi::default();
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    assert!(!hotkeys_tab(ui, &mut state, &mut hk, &mut settings));
                });
            });
        }
        assert!(!ctx.has_requested_repaint(), "idle hotkeys tab requested a repaint");
    }
}
