//! ---- ws:pro-monitor ----
//! Multicam angles: a multicam `Sequence` clip's angles (`Project::multicam_angles`) as a compact list
//! pinned in the program monitor's top-right corner (ws:viewer-surface; it was a free-floating window);
//! clicking a row returns `Some(angle)` for the caller (`tools_monitor::window_multicam`) to apply via
//! `Project::multicam_switch`.
//!
//! deviation (see PR body): the plan's signature threads live per-angle textures
//! (`textures: &[(usize, egui::TextureId)]`) for <=4 alt-render thumbnails. Building those needs a
//! dedicated decode pipeline per visible angle (up to 4 extra `Player`s, or a round-robin single decoder)
//! - real engine work this already-large workstream skips. Angle rows are numbered/named/coloured
//! instead (capped at 4, same as the plan's own grid cap): click-to-switch works fully today; live
//! thumbnails are a follow-up (`// ponytail:` note below).

use crate::theme::Palette;
use crate::ui::tools::{self, Glyph};
use eframe::egui::{self, vec2};

/// ponytail: text/colour rows, no live per-angle video - see this file's top-of-file deviation note.
/// Upgrade path: a round-robin single extra `Player` (like `monitor.rs`'s `TrimSlot`) cycling through
/// the up-to-4 visible angles, one decode per tick, if the Gallery/UI review wants live previews.
/// `viewer` = the program monitor's picture area, `None` while it is off screen (then nothing shows:
/// switching angles without seeing them makes no sense, and Next / Previous Angle still work).
pub(crate) fn angle_grid(
    ctx: &egui::Context,
    open: &mut bool,
    angles: &[(usize, String)],
    cur: usize,
    pal: &Palette,
    viewer: Option<egui::Rect>,
) -> Option<usize> {
    let viewer = viewer.filter(|r| r.is_positive())?;
    if !*open {
        return None;
    }
    let mut clicked = None;
    egui::Area::new(egui::Id::new("multicam-angles"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::RIGHT_TOP)
        .fixed_pos(viewer.right_top() + vec2(-8.0, 8.0))
        .constrain_to(viewer)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).fill(pal.panel.gamma_multiply(0.92)).show(ui, |ui| {
                ui.set_width(150.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Angles").strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let tip = "Close (Clip ▸ Multicam Angles… opens it again)";
                        if tools::icon_button(ui, pal, ui.id().with("close"), Glyph::Cross, tip, false).clicked() {
                            *open = false;
                        }
                    });
                });
                if angles.is_empty() {
                    ui.weak("No multicam clip under the playhead");
                    return;
                }
                for (idx, name) in angles.iter().take(4) {
                    let active = *idx == cur;
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), egui::Sense::click());
                    let bg = if active {
                        pal.accent
                    } else if resp.hovered() {
                        pal.header
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(rect, 3.0, bg);
                    let fg = if active { tools::on_accent(pal.accent) } else { pal.text };
                    let font = egui::TextStyle::Button.resolve(ui.style());
                    let at = rect.left_center() + vec2(8.0, 0.0);
                    ui.painter().text(at, egui::Align2::LEFT_CENTER, format!("{}", idx + 1), font.clone(), fg);
                    let at = rect.left_center() + vec2(28.0, 0.0);
                    ui.painter().text(at, egui::Align2::LEFT_CENTER, name, font, fg);
                    if resp.on_hover_text("Cut to this angle at the playhead").clicked() {
                        clicked = Some(*idx);
                    }
                }
            });
        });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWER: egui::Rect = egui::Rect { min: egui::pos2(0.0, 0.0), max: egui::pos2(640.0, 360.0) };

    #[test]
    fn assert_no_idle_repaint_on_multicam_angle_grid() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let pal = Palette::new(true, egui::Color32::WHITE);
        let angles = vec![(0, "V1".to_string()), (1, "V2".to_string()), (2, "V3".to_string())];
        let mut open = true;
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                angle_grid(ctx, &mut open, &angles, 0, &pal, Some(VIEWER));
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
            angle_grid(&ctx, &mut closed, &[(0, "V1".into())], 0, &pal, Some(VIEWER)),
            None,
            "closed window returns None immediately"
        );
        let mut open = true;
        let mut got = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            got = angle_grid(ctx, &mut open, &[], 0, &pal, Some(VIEWER));
        });
        assert_eq!(got, None, "no angles: nothing to click");
    }

    // ---- ws:viewer-surface ----
    /// The list sits in the viewer's top-right corner and its rows are the angles: a click on the
    /// second row picks angle 2. With the viewer off screen nothing shows.
    #[test]
    fn angles_sit_in_the_viewer_corner_and_click() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let pal = Palette::new(true, egui::Color32::WHITE);
        let angles = vec![(0, "V1".to_string()), (1, "V2".to_string())];
        let mut open = true;
        let mut run = |events: Vec<egui::Event>, viewer: Option<egui::Rect>| {
            let mut got = None;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 500.0))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| got = angle_grid(ctx, &mut open, &angles, 0, &pal, viewer));
            got
        };
        run(vec![], Some(VIEWER));
        run(vec![], Some(VIEWER));
        let area = ctx.memory(|m| m.area_rect(egui::Id::new("multicam-angles"))).expect("shown");
        assert!(VIEWER.contains_rect(area) && area.left() > VIEWER.center().x && area.top() < 20.0, "{area:?}");
        let at = egui::pos2(area.center().x, area.bottom() - 16.0); // the last row: V2
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        run(vec![egui::Event::PointerMoved(at)], Some(VIEWER));
        run(vec![press(true)], Some(VIEWER));
        assert_eq!(run(vec![press(false)], Some(VIEWER)), Some(1), "the second row is angle 2");
        assert_eq!(run(vec![], None), None, "no viewer on screen: nothing to click");
    }
}
