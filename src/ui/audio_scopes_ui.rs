//! ---- ws:parity-audio ----
//! The Audio Scopes pane: a log-frequency spectrum analyser of what is playing (bars with falling peak
//! ticks), and below it a vectorscope (goniometer: mid up, side across - mono is a vertical line, wide
//! stereo a cloud, out of phase a horizontal line) beside an L/R correlation meter. Drawing only: the
//! numbers come from `engine::audio_tools::Scope`, updated by `ui::app::tools_audio_parity::draw` when
//! new audio arrived - so an idle editor draws the same picture and asks for no repaint.

use crate::engine::audio_tools::{bar_hz, Scope, SPECTRUM_BARS, SPECTRUM_FLOOR};
use crate::theme::Palette;
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke};

/// Frequencies labelled under the spectrum.
const HZ_LABELS: [(f32, &str); 4] = [(50.0, "50"), (200.0, "200"), (1000.0, "1k"), (5000.0, "5k")];
/// dB lines across the spectrum.
const DB_LINES: [f32; 4] = [-12.0, -24.0, -48.0, -72.0];

/// The whole pane. `playing`: say so when there is nothing to show yet.
pub fn body(ui: &mut egui::Ui, scope: &Scope, playing: bool, palette: &Palette) {
    let avail = ui.available_size();
    let spec_h = (avail.y * 0.55).clamp(80.0, 320.0);
    spectrum(ui, scope, palette, vec2(avail.x, spec_h));
    ui.add_space(4.0);
    let side = (ui.available_height() - 4.0).clamp(60.0, 260.0).min(avail.x * 0.6);
    ui.horizontal(|ui| {
        goniometer(ui, scope, palette, side);
        ui.vertical(|ui| {
            correlation(ui, scope.correlation, palette);
            ui.add_space(6.0);
            let db = |v: f32| if v > 1e-6 { format!("{:.1}", 20.0 * v.log10()) } else { "-inf".into() };
            ui.label(egui::RichText::new(format!("Peak L {}  R {} dBFS", db(scope.peak.0), db(scope.peak.1))).small())
                .on_hover_text("Highest sample of the last ~20 ms, per channel");
            if !playing && scope.points.is_empty() {
                ui.weak("Play to see the sound");
            }
        });
    });
}

/// x of a frequency across `r` (20 Hz .. 20 kHz, log).
fn x_of(r: Rect, hz: f32) -> f32 {
    r.left() + (hz.max(20.0) / 20.0).log10() / 3.0 * r.width()
}

fn spectrum(ui: &mut egui::Ui, scope: &Scope, palette: &Palette, size: egui::Vec2) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, palette.bg);
    let label_h = 11.0;
    let plot = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - label_h));
    let y_of = |db: f32| plot.top() + (db / SPECTRUM_FLOOR).clamp(0.0, 1.0) * plot.height();
    let faint = palette.border.gamma_multiply(0.8);
    let font = FontId::proportional(8.0);
    for db in DB_LINES {
        p.line_segment([pos2(plot.left(), y_of(db)), pos2(plot.right(), y_of(db))], Stroke::new(1.0, faint));
        p.text(pos2(plot.left() + 2.0, y_of(db)), Align2::LEFT_BOTTOM, format!("{db:.0}"), font.clone(), palette.text_dim);
    }
    for (hz, name) in HZ_LABELS {
        let x = x_of(rect, hz);
        p.line_segment([pos2(x, plot.top()), pos2(x, plot.bottom())], Stroke::new(1.0, faint));
        p.text(pos2(x, rect.bottom() - label_h), Align2::CENTER_TOP, name, font.clone(), palette.text_dim);
    }
    let w = rect.width() / SPECTRUM_BARS as f32;
    for (i, (&db, &pk)) in scope.bars.iter().zip(&scope.peaks).enumerate() {
        let x0 = rect.left() + i as f32 * w;
        let bar = Rect::from_min_max(pos2(x0 + 1.0, y_of(db)), pos2(x0 + w - 1.0, plot.bottom()));
        if db > SPECTRUM_FLOOR {
            p.rect_filled(bar, 0.0, level_color(db, palette));
        }
        if pk > SPECTRUM_FLOOR {
            let y = y_of(pk);
            p.line_segment([pos2(x0 + 1.0, y), pos2(x0 + w - 1.0, y)], Stroke::new(1.5, palette.text));
        }
    }
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, palette.border), egui::StrokeKind::Inside);
    if let Some(pos) = resp.hover_pos() {
        let i = (((pos.x - rect.left()) / w) as usize).min(SPECTRUM_BARS - 1);
        let hz = bar_hz(i);
        let f = if hz >= 1000.0 { format!("{:.1} kHz", hz / 1000.0) } else { format!("{hz:.0} Hz") };
        resp.on_hover_text(format!("{f}: {:.1} dB", scope.bars[i]));
    }
}

/// Bar colour: the accent, warming to amber near full scale and red above -3 dB.
fn level_color(db: f32, palette: &Palette) -> Color32 {
    if db > -3.0 {
        Color32::from_rgb(230, 70, 60)
    } else if db > -12.0 {
        Color32::from_rgb(230, 170, 60)
    } else {
        palette.accent
    }
}

fn goniometer(ui: &mut egui::Ui, scope: &Scope, palette: &Palette, side: f32) {
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, palette.bg);
    let c = rect.center();
    let r = side * 0.5 - 2.0;
    let faint = Stroke::new(1.0, palette.border.gamma_multiply(0.8));
    // the mono axis (M, up) and the out-of-phase axis (S, across), plus the L and R diagonals
    p.line_segment([pos2(c.x, rect.top()), pos2(c.x, rect.bottom())], faint);
    p.line_segment([pos2(rect.left(), c.y), pos2(rect.right(), c.y)], faint);
    p.line_segment([pos2(c.x - r, c.y - r), pos2(c.x + r, c.y + r)], faint);
    p.line_segment([pos2(c.x + r, c.y - r), pos2(c.x - r, c.y + r)], faint);
    let font = FontId::proportional(8.0);
    p.text(pos2(c.x - r + 2.0, c.y - r + 2.0), Align2::LEFT_TOP, "L", font.clone(), palette.text_dim);
    p.text(pos2(c.x + r - 2.0, c.y - r + 2.0), Align2::RIGHT_TOP, "R", font, palette.text_dim);
    let dot = palette.accent.gamma_multiply(0.7);
    for &(s, m) in &scope.points {
        let q = pos2(c.x + s * r, c.y - m * r);
        p.rect_filled(Rect::from_center_size(q, vec2(1.5, 1.5)), 0.0, dot);
    }
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, palette.border), egui::StrokeKind::Inside);
    resp.on_hover_text("Vectorscope: mono is a vertical line, wide stereo a cloud, out of phase a horizontal line");
}

fn correlation(ui: &mut egui::Ui, v: f32, palette: &Palette) {
    let w = ui.available_width().clamp(80.0, 220.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 14.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 2.0, palette.bg);
    let x = rect.center().x + v.clamp(-1.0, 1.0) * rect.width() * 0.5;
    let col = if v < 0.0 { Color32::from_rgb(230, 70, 60) } else { palette.accent };
    let bar = Rect::from_min_max(pos2(rect.center().x.min(x), rect.top() + 2.0), pos2(rect.center().x.max(x), rect.bottom() - 2.0));
    p.rect_filled(bar, 1.0, col);
    p.line_segment([pos2(rect.center().x, rect.top()), pos2(rect.center().x, rect.bottom())], Stroke::new(1.0, palette.text_dim));
    p.rect_stroke(rect, 2.0, Stroke::new(1.0, palette.border), egui::StrokeKind::Inside);
    resp.on_hover_text(format!(
        "Correlation {v:+.2}: +1 mono, 0 wide stereo, below 0 out of phase (it cancels when played in mono)"
    ));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let small = |t: &str| egui::RichText::new(t.to_string()).small().color(palette.text_dim);
        ui.label(small("-1"));
        ui.add_space(w / 2.0 - 14.0);
        ui.label(small("0"));
        ui.add_space(w / 2.0 - 14.0);
        ui.label(small("+1"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Audio Scopes pane (empty, then with a picture) idles without asking for a repaint.
    #[test]
    fn assert_no_idle_repaint_audio_scopes() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let pal = Palette::new(true, egui::Color32::WHITE);
        let empty = Scope::default();
        let mut full = Scope::default();
        let tone: Vec<f32> = (0..4096)
            .flat_map(|i| {
                let s = 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / 48000.0).sin();
                [s, s * 0.5]
            })
            .collect();
        full.update(&tone, 0.0);
        assert!(!full.points.is_empty());
        for (scope, playing) in [(&empty, false), (&full, false), (&full, true)] {
            for _ in 0..30 {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| body(ui, scope, playing, &pal));
                });
            }
            assert!(!ctx.has_requested_repaint(), "an unchanged scope must request no repaint");
        }
    }

    #[test]
    fn log_axis_spans_20_hz_to_20_khz() {
        let r = Rect::from_min_size(pos2(10.0, 0.0), vec2(300.0, 10.0));
        assert!((x_of(r, 20.0) - 10.0).abs() < 1e-3);
        assert!((x_of(r, 20_000.0) - 310.0).abs() < 1e-3);
        assert!((x_of(r, 632.5) - 160.0).abs() < 0.5, "the geometric middle is the centre");
    }
}
