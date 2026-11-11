//! `Pane::Assistant`'s App side. pro-schema declares the pane with a placeholder drawer and its
//! Show / Hide Action; ws:assistant-pane replaces the body (backends, sending, the log).

use super::*;

/// PANE_DRAWERS entry.
pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::Assistant {
        return false;
    }
    crate::ui::assistant_ui::draw(ui, &mut app.assistant);
    true
}

/// ACT_HANDLERS entry.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::ToggleAssistant => {
            app.toggle_pane(Pane::Assistant);
            true
        }
        _ => false,
    }
}
