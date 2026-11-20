//! ---- ws:forum-preview ----
//! The playback health chip in the viewer's top-left corner (`ui::app::health_ctl` feeds it). Green is a
//! small dot while playing; Amber is a chip: "Choppy" (opens / closes the Performance pane, lit while it is
//! open), the one fix that helps most, and a close button. Labels are just the name; why and what the fix
//! does are in the tooltips. Nothing here animates, so it never asks for a repaint.

use eframe::egui::{self, Align, Color32, Layout, Rect, RichText, Sense, Ui, UiBuilder, Vec2};

/// What the chip shows.
pub struct Chip<'a> {
    pub amber: bool,
    /// Draw the green dot (only while playing).
    pub playing: bool,
    /// Why it is amber, in a sentence (the "Choppy" tooltip).
    pub reason: &'a str,
    /// The fix button: (label, tooltip). None = no fix to offer.
    pub fix: Option<(&'a str, &'a str)>,
    /// The Performance pane is on screen (the "Choppy" button is lit).
    pub perf_open: bool,
}

#[derive(Default, Debug, PartialEq)]
pub struct Out {
    pub fix: bool,
    pub toggle_perf: bool,
    pub dismiss: bool,
}

pub const AMBER: Color32 = Color32::from_rgb(240, 170, 40);
pub const GREEN: Color32 = Color32::from_rgb(70, 190, 100);

/// Draw over `canvas` (the viewer's picture rect).
pub fn draw(ui: &mut Ui, canvas: Rect, chip: &Chip) -> Out {
    let mut out = Out::default();
    if !canvas.is_positive() || !(chip.amber || chip.playing) {
        return out;
    }
    let at = canvas.min + Vec2::splat(8.0);
    if !chip.amber {
        let r = Rect::from_min_size(at, Vec2::splat(10.0));
        let resp = ui.interact(r, ui.id().with("se-health-dot"), Sense::hover());
        ui.painter().circle_filled(r.center(), 4.0, GREEN);
        resp.on_hover_text("Playback is keeping up");
        return out;
    }
    let area = Rect::from_min_size(at, Vec2::new((canvas.width() - 16.0).max(60.0), 28.0));
    let mut child = ui.new_child(UiBuilder::new().max_rect(area).layout(Layout::left_to_right(Align::Center)));
    egui::Frame::popup(child.style()).inner_margin(egui::Margin::symmetric(6, 3)).show(&mut child, |ui| {
        ui.horizontal(|ui| {
            let label = RichText::new("● Choppy").color(AMBER);
            if ui.selectable_label(chip.perf_open, label).on_hover_text(chip.reason).clicked() {
                out.toggle_perf = true;
            }
            if let Some((name, tip)) = chip.fix {
                if ui.button(name).on_hover_text(tip).clicked() {
                    out.fix = true;
                }
            }
            if ui.small_button("✕").on_hover_text("Hide until playback drops frames again").clicked() {
                out.dismiss = true;
            }
        });
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Pos2, RawInput};

    fn run(ctx: &egui::Context, input: RawInput, chip: &Chip) -> Out {
        let mut out = Out::default();
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                out = draw(ui, Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(640.0, 360.0)), chip);
            });
        });
        out
    }

    fn amber<'a>() -> Chip<'a> {
        Chip {
            amber: true,
            playing: true,
            reason: "IMG_0042.MOV is too heavy to decode in real time on this PC",
            fix: Some(("Make proxy", "About 1.2 GB")),
            perf_open: false,
        }
    }

    #[test]
    fn assert_no_idle_repaint_health_chip() {
        for chip in [amber(), Chip { amber: false, ..amber() }, Chip { amber: false, playing: false, ..amber() }] {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::theme::test_fonts());
            for _ in 0..30 {
                run(&ctx, RawInput::default(), &chip);
            }
            assert!(!ctx.has_requested_repaint(), "the health chip must not repaint while idle");
        }
    }

    /// Green and not playing draws nothing; Amber has its three buttons and they answer clicks.
    #[test]
    fn amber_chip_buttons_answer() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let quiet = Chip { amber: false, playing: false, ..amber() };
        let _ = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let before = ui.min_rect();
                draw(ui, Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::new(640.0, 360.0)), &quiet);
                assert_eq!(ui.min_rect(), before, "nothing allocated");
            });
        });
        // the widgets sit in one row from the canvas corner + 8: press and release along it
        let chip = amber();
        let points: Vec<Pos2> = (30..400).step_by(4).map(|x| Pos2::new(x as f32, 36.0)).collect();
        let mut seen = Out::default();
        for p in points {
            for pressed in [true, false] {
                let input = RawInput {
                    events: vec![
                        egui::Event::PointerMoved(p),
                        egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
                    ],
                    ..Default::default()
                };
                let o = run(&ctx, input, &chip);
                seen.fix |= o.fix;
                seen.toggle_perf |= o.toggle_perf;
                seen.dismiss |= o.dismiss;
            }
        }
        assert_eq!(seen, Out { fix: true, toggle_perf: true, dismiss: true });
    }
}
