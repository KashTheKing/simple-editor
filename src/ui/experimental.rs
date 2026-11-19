//! The "Experimental" badge: a small pill in a muted warning colour, with the explanation on hover. One
//! look for every feature still being worked on (the Animator, the Platform Emulator, the Assistant, the
//! AI upscaler, Screen Recording): wherever it is entered - its pane tab, Window menu row, page tab,
//! dialog, Ctrl+P row - `badge` / `paint` draw it, and `pane` / `action` / `page` say which ones wear it.

use crate::hotkeys::Action;
use crate::ui::layout::Pane;
use eframe::egui::{self, vec2, Align2, Color32, FontId, Pos2, Rect, Response, Sense, Ui};

/// The badge's text: just the name (the explanation is the tooltip).
pub const LABEL: &str = "Experimental";
/// Its tooltip.
pub const TIP: &str = "Still being worked on; things may change or break";

const AMBER: Color32 = Color32::from_rgb(0xE0, 0xA4, 0x3A);
const FONT_PX: f32 = 10.0;
const PAD_X: f32 = 5.0;
const HEIGHT: f32 = 15.0;

/// The panes that wear the badge.
pub fn pane(p: Pane) -> bool {
    matches!(p, Pane::Animator | Pane::Emulator | Pane::Assistant | Pane::ScreenRecording)
}

/// The actions (menu rows, Ctrl+P rows) that enter one of those features.
pub fn action(a: Action) -> bool {
    use Action::*;
    matches!(
        a,
        ScreenCapture
            | ToggleAssistant
            | AssistantAddSelection
            | ShowAnimations
            | AiUpscale2Game
            | AiUpscale2Real
            | AiUpscale4Game
            | AiUpscale4Real
            | Workspace4 // its label is "Animate page"
    )
}

/// The pages that wear it (the Animate page tab).
pub fn page(name: &str) -> bool {
    name == "Animate"
}

fn font() -> FontId {
    FontId::proportional(FONT_PX)
}

/// The pill's size.
pub fn size(ui: &Ui) -> egui::Vec2 {
    let w = ui.painter().layout_no_wrap(LABEL.into(), font(), AMBER).size().x;
    vec2(w + 2.0 * PAD_X, HEIGHT)
}

/// Paint the pill with its left edge centred on `left_center`; returns its rect (for a tooltip).
pub fn paint(painter: &egui::Painter, left_center: Pos2) -> Rect {
    let g = painter.layout_no_wrap(LABEL.into(), font(), AMBER);
    let r = Rect::from_min_size(left_center - vec2(0.0, HEIGHT / 2.0), vec2(g.size().x + 2.0 * PAD_X, HEIGHT));
    painter.rect_filled(r, HEIGHT / 2.0, AMBER.gamma_multiply(0.16));
    painter.rect_stroke(r, HEIGHT / 2.0, egui::Stroke::new(1.0, AMBER.gamma_multiply(0.45)), egui::StrokeKind::Inside);
    painter.text(r.center(), Align2::CENTER_CENTER, LABEL, font(), AMBER);
    r
}

/// The tooltip over a painted pill.
pub fn tip(ui: &Ui, rect: Rect, id: egui::Id) {
    ui.interact(rect, id.with("experimental"), Sense::hover()).on_hover_text(TIP);
}

/// The pill as a widget in the layout, tooltip included.
pub fn badge(ui: &mut Ui) -> Response {
    let (rect, r) = ui.allocate_exact_size(size(ui), Sense::hover());
    paint(ui.painter(), rect.left_center());
    r.on_hover_text(TIP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_five_features_wear_it_and_others_do_not() {
        for p in [Pane::Animator, Pane::Emulator, Pane::Assistant, Pane::ScreenRecording] {
            assert!(pane(p), "{p:?}");
        }
        assert!(!pane(Pane::Timeline) && !pane(Pane::Preview));
        assert!(action(Action::ScreenCapture) && action(Action::ToggleAssistant) && action(Action::AiUpscale4Real));
        assert!(action(Action::Workspace4), "the Animate page");
        assert!(!action(Action::Save) && !action(Action::Workspace1));
        assert!(page("Animate") && !page("Edit"));
        assert!(!LABEL.contains(' '), "the label is just the name");
    }
}
