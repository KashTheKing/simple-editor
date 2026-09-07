//! ---- ws:pro-monitor ----
//! Multicam angle-grid window: lists a multicam `Sequence` clip's angles (`Project::multicam_angles`)
//! as clickable rows; clicking one returns `Some(angle)` for the caller (`preview_pane.rs`) to apply via
//! `Project::multicam_switch`.
//!
//! deviation (see PR body): the plan's signature threads live per-angle textures
//! (`textures: &[(usize, egui::TextureId)]`) for <=4 alt-render thumbnails. Building those needs a
//! dedicated decode pipeline per visible angle (up to 4 extra `Player`s, or a round-robin single decoder)
//! - real engine work this already-large workstream skips. Angle rows are numbered/named/coloured
//! instead (capped at 4, same as the plan's own grid cap): click-to-switch works fully today; live
//! thumbnails are a follow-up (`// ponytail:` note below).

use crate::theme::Palette;
use eframe::egui::{self, vec2};

/// ponytail: text/colour rows, no live per-angle video - see this file's top-of-file deviation note.
/// Upgrade path: a round-robin single extra `Player` (like `monitor.rs`'s `TrimSlot`) cycling through
/// the up-to-4 visible angles, one decode per tick, if the Gallery/UI review wants live previews.
pub(crate) fn angle_grid(
    ctx: &egui::Context,
    open: &mut bool,
    angles: &[(usize, String)],
    cur: usize,
    pal: &Palette,
) -> Option<usize> {
    if !*open {
        return None;
    }
    let mut clicked = None;
    let mut still_open = true;
    egui::Window::new("Multicam Angles").open(&mut still_open).resizable(false).show(ctx, |ui| {
        if angles.is_empty() {
            ui.weak("No multicam clip under the playhead");
            return;
        }
        for (idx, name) in angles.iter().take(4) {
            let active = *idx == cur;
            let color = if active { pal.accent } else { pal.text };
            let label = egui::RichText::new(format!("{}  {name}", idx + 1)).color(color).strong().size(14.0);
            let (rect, resp) =
                ui.allocate_exact_size(vec2(ui.available_width().max(160.0), 26.0), egui::Sense::click());
            let bg = if active { pal.accent.gamma_multiply(0.18) } else { pal.header.gamma_multiply(0.5) };
            ui.painter().rect_filled(rect, 3.0, bg);
            ui.painter().text(
                rect.left_center() + vec2(8.0, 0.0),
                egui::Align2::LEFT_CENTER,
                label.text(),
                egui::TextStyle::Button.resolve(ui.style()),
                color,
            );
            if resp.clicked() {
                clicked = Some(*idx);
            }
        }
    });
    *open = still_open;
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assert_no_idle_repaint_on_multicam_angle_grid() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let pal = Palette::new(true, egui::Color32::WHITE);
        let angles = vec![(0, "V1".to_string()), (1, "V2".to_string()), (2, "V3".to_string())];
        let mut open = true;
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                angle_grid(ctx, &mut open, &angles, 0, &pal);
            });
        }
        assert!(!ctx.has_requested_repaint(), "an idle, unchanged angle list must request no repaint");
    }

    #[test]
    fn angle_grid_returns_none_when_closed_or_no_angles() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let pal = Palette::new(true, egui::Color32::WHITE);
        let mut closed = false;
        assert_eq!(
            angle_grid(&ctx, &mut closed, &[(0, "V1".into())], 0, &pal),
            None,
            "closed window returns None immediately"
        );
        let mut open = true;
        let mut got = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            got = angle_grid(ctx, &mut open, &[], 0, &pal);
        });
        assert_eq!(got, None, "no angles: nothing to click");
    }
}
