//! ---- ws:pro-monitor ----
//! Video scopes: Waveform / Parade / Vectorscope / Histogram over `GpuRenderer::stats()`'s `FrameStats`.
//!
//! deviation (see PR body / `monitor.rs`'s matching note): the plan attributes this to a
//! `gpu.frame_stats()` API that does not exist in `engine::gpu` under that name — CONFIRMED by reading
//! the source. What DOES exist is `GpuRenderer::{set_stats_wanted, stats}` (already consumed by
//! `tools_color.rs`'s `color.auto`/`color.match`/`frame.stats`), fed by `render_preview_texture`'s own
//! internal readback gate. This window is therefore a REAL implementation, not the no-op stub the
//! orchestrating brief anticipated for the missing-API case — it reads the same `FrameStats` type
//! color-engine already computes, just via `stats()` instead of a same-named method.
//!
//! Each scope's geometry is computed by a pure `*_points`/`*_heights` fn (no `egui` needed, directly
//! unit-testable against a synthetic `FrameStats`) and painted by a thin `paint_*` wrapper.

use crate::engine::gpu::FrameStats;
use crate::theme::Palette;
use eframe::egui::{self, pos2, vec2, Color32, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ScopeKind {
    Waveform,
    Parade,
    Vectorscope,
    Histogram,
}

impl ScopeKind {
    pub(crate) const ALL: [ScopeKind; 4] = [Self::Waveform, Self::Parade, Self::Vectorscope, Self::Histogram];
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Waveform => "Waveform",
            Self::Parade => "Parade",
            Self::Vectorscope => "Vectorscope",
            Self::Histogram => "Histogram",
        }
    }
}

/// Rec.709 luma (0..1) of one downsampled RGBA sample.
fn luma01(px: [u8; 4]) -> f32 {
    (px[0] as f32 * 0.2126 + px[1] as f32 * 0.7152 + px[2] as f32 * 0.0722) / 255.0
}

/// Waveform scatter: (x in 0..1 = column position, y in 0..1 = luma) per downsampled pixel — a classic
/// video waveform monitor reads brightness left-to-right the same way the frame does.
pub(crate) fn waveform_points(stats: &FrameStats) -> Vec<(f32, f32)> {
    let w = stats.sample_w.max(1);
    stats.sample.iter().enumerate().map(|(i, &px)| ((i as u32 % w) as f32 / w as f32, luma01(px))).collect()
}

/// Parade: the same column-scatter as `waveform_points`, split per channel (R, G, B) so each channel's
/// distribution is visible on its own instead of blended into luma.
pub(crate) fn parade_points(stats: &FrameStats) -> [Vec<(f32, f32)>; 3] {
    let w = stats.sample_w.max(1);
    let mut out: [Vec<(f32, f32)>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for (i, &px) in stats.sample.iter().enumerate() {
        let x = (i as u32 % w) as f32 / w as f32;
        for c in 0..3 {
            out[c].push((x, px[c] as f32 / 255.0));
        }
    }
    out
}

/// Vectorscope: a simple (B-Y, R-Y) chroma scatter per downsampled pixel, centred at (0, 0) — not
/// broadcast-calibrated (no I/Q rotation or graticule targets), close enough to spot a colour cast or a
/// blown-out saturated channel at a glance.
pub(crate) fn vectorscope_points(stats: &FrameStats) -> Vec<(f32, f32)> {
    stats
        .sample
        .iter()
        .map(|&[r, g, b, _]| {
            let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
            let y = 0.299 * r + 0.587 * g + 0.114 * b;
            ((b - y) * 0.5, (r - y) * 0.5)
        })
        .collect()
}

/// Normalized (0..1) bar heights for one channel's 256-bucket histogram — the tallest bucket is 1.0.
pub(crate) fn histogram_heights(hist: &[u32; 256]) -> [f32; 256] {
    let max = (*hist.iter().max().unwrap_or(&0)).max(1) as f32;
    std::array::from_fn(|i| hist[i] as f32 / max)
}

fn paint_waveform(p: &egui::Painter, rect: Rect, stats: &FrameStats, color: Color32) {
    for (x, y) in waveform_points(stats) {
        let at = pos2(rect.left() + x * rect.width(), rect.bottom() - y * rect.height());
        p.rect_filled(Rect::from_center_size(at, vec2(1.5, 1.5)), 0.0, color);
    }
}

pub(crate) fn paint_waveform_scope(p: &egui::Painter, rect: Rect, stats: &FrameStats, pal: &Palette) {
    paint_waveform(p, rect, stats, pal.text.gamma_multiply(0.35));
}

pub(crate) fn paint_parade(p: &egui::Painter, rect: Rect, stats: &FrameStats) {
    let colors = [
        Color32::from_rgba_unmultiplied(255, 60, 60, 110),
        Color32::from_rgba_unmultiplied(60, 255, 60, 110),
        Color32::from_rgba_unmultiplied(80, 140, 255, 110),
    ];
    let third = rect.width() / 3.0;
    let points = parade_points(stats);
    for (c, pts) in points.iter().enumerate() {
        let sub = Rect::from_min_size(pos2(rect.left() + third * c as f32, rect.top()), vec2(third, rect.height()));
        for &(x, y) in pts {
            let at = pos2(sub.left() + x * sub.width(), sub.bottom() - y * sub.height());
            p.rect_filled(Rect::from_center_size(at, vec2(1.2, 1.2)), 0.0, colors[c]);
        }
    }
}

pub(crate) fn paint_vectorscope(p: &egui::Painter, rect: Rect, stats: &FrameStats, pal: &Palette) {
    let c = rect.center();
    let radius = rect.width().min(rect.height()) / 2.0 - 4.0;
    p.circle_stroke(c, radius, egui::Stroke::new(1.0, pal.text.gamma_multiply(0.3)));
    p.line_segment(
        [c - vec2(radius, 0.0), c + vec2(radius, 0.0)],
        egui::Stroke::new(0.5, pal.text.gamma_multiply(0.2)),
    );
    p.line_segment(
        [c - vec2(0.0, radius), c + vec2(0.0, radius)],
        egui::Stroke::new(0.5, pal.text.gamma_multiply(0.2)),
    );
    for (u, v) in vectorscope_points(stats) {
        let at = c + vec2(u * radius * 2.0, -v * radius * 2.0);
        p.rect_filled(Rect::from_center_size(at, vec2(1.5, 1.5)), 0.0, pal.accent.gamma_multiply(0.5));
    }
}

pub(crate) fn paint_histogram(p: &egui::Painter, rect: Rect, stats: &FrameStats) {
    let colors = [
        Color32::from_rgba_unmultiplied(255, 70, 70, 160),
        Color32::from_rgba_unmultiplied(70, 255, 70, 160),
        Color32::from_rgba_unmultiplied(90, 150, 255, 160),
    ];
    let bar_w = rect.width() / 256.0;
    for c in 0..3 {
        let heights = histogram_heights(&stats.hist[c]);
        for (i, h) in heights.iter().enumerate() {
            let x = rect.left() + i as f32 * bar_w;
            let bar =
                Rect::from_min_max(pos2(x, rect.bottom() - h * rect.height()), pos2(x + bar_w.max(1.0), rect.bottom()));
            p.rect_filled(bar, 0.0, colors[c]);
        }
    }
}

/// The Scopes `egui::Window`: one tab per name in `open` (from `Settings.scopes`), each painting against
/// `stats` (`None` = nothing rendered yet — an empty placeholder, no panic). Adding/removing tabs writes
/// back into `open` directly. Requests no repaint of its own — `App::gpu.stats()` only changes when a new
/// frame is actually decoded, so an idle preview with Scopes open costs nothing extra per frame.
pub(crate) fn window(
    ctx: &egui::Context,
    open: &mut bool,
    tabs: &mut Vec<String>,
    stats: Option<&FrameStats>,
    pal: &Palette,
) {
    if !*open {
        return;
    }
    let mut still_open = true;
    egui::Window::new("Scopes").open(&mut still_open).default_width(360.0).default_height(280.0).show(ctx, |ui| {
        if tabs.is_empty() {
            tabs.push(ScopeKind::Waveform.name().to_string());
        }
        ui.horizontal(|ui| {
            for k in ScopeKind::ALL {
                let on = tabs.iter().any(|t| t == k.name());
                if ui.selectable_label(on, k.name()).clicked() {
                    if on {
                        tabs.retain(|t| t != k.name());
                    } else {
                        tabs.push(k.name().to_string());
                    }
                }
            }
        });
        ui.separator();
        let Some(stats) = stats else {
            ui.weak("No frame rendered yet");
            return;
        };
        for name in tabs.iter() {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 120.0), egui::Sense::hover());
            let p = ui.painter_at(rect);
            p.rect_filled(rect, 2.0, pal.header.gamma_multiply(0.6));
            match ScopeKind::ALL.iter().find(|k| k.name() == name) {
                Some(ScopeKind::Waveform) => paint_waveform_scope(&p, rect, stats, pal),
                Some(ScopeKind::Parade) => paint_parade(&p, rect, stats),
                Some(ScopeKind::Vectorscope) => paint_vectorscope(&p, rect, stats, pal),
                Some(ScopeKind::Histogram) => paint_histogram(&p, rect, stats),
                None => {}
            }
            ui.add_space(6.0);
        }
    });
    *open = still_open;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(px: Vec<[u8; 4]>, w: u32, h: u32) -> FrameStats {
        crate::engine::gpu::compute_stats(&px.iter().flatten().copied().collect::<Vec<u8>>(), w, h)
    }

    #[test]
    fn waveform_points_map_columns_and_luma() {
        // 2x1: a black pixel then a white pixel — column 0 -> luma 0, column 1 -> luma ~1
        let s = stats(vec![[0, 0, 0, 255], [255, 255, 255, 255]], 2, 1);
        let pts = waveform_points(&s);
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[0].0, 0.0);
        assert!(pts[0].1 < 0.05, "black pixel: near-zero luma, got {}", pts[0].1);
        assert_eq!(pts[1].0, 0.5);
        assert!(pts[1].1 > 0.95, "white pixel: near-one luma, got {}", pts[1].1);
    }

    #[test]
    fn parade_points_split_channels() {
        let s = stats(vec![[255, 0, 128, 255]], 1, 1);
        let [r, g, b] = parade_points(&s);
        assert!((r[0].1 - 1.0).abs() < 1e-3);
        assert!((g[0].1 - 0.0).abs() < 1e-3);
        assert!((b[0].1 - 128.0 / 255.0).abs() < 1e-3);
    }

    #[test]
    fn vectorscope_places_neutral_grey_at_center() {
        let s = stats(vec![[128, 128, 128, 255]], 1, 1);
        let pts = vectorscope_points(&s);
        assert!(pts[0].0.abs() < 1e-3 && pts[0].1.abs() < 1e-3, "grey has zero chroma: {:?}", pts[0]);
    }

    #[test]
    fn vectorscope_saturated_red_moves_off_center() {
        let s = stats(vec![[255, 0, 0, 255]], 1, 1);
        let pts = vectorscope_points(&s);
        assert!(pts[0].1 > 0.1, "pure red has positive (R-Y): {:?}", pts[0]);
    }

    /// The plan's own named test: a synthetic histogram with a known peak produces the expected bar
    /// height mapping (the tallest bucket normalizes to 1.0, everything else scales relative to it).
    #[test]
    fn scopes_paint_fns_match_known_histogram() {
        let mut hist = [0u32; 256];
        hist[10] = 5;
        hist[200] = 20; // the peak
        hist[255] = 10;
        let heights = histogram_heights(&hist);
        assert!((heights[200] - 1.0).abs() < 1e-6, "the peak bucket normalizes to 1.0");
        assert!((heights[10] - 0.25).abs() < 1e-6, "5/20 = 0.25");
        assert!((heights[255] - 0.5).abs() < 1e-6, "10/20 = 0.5");
        assert_eq!(heights[0], 0.0, "an empty bucket stays 0");
    }

    #[test]
    fn histogram_heights_handles_an_all_zero_histogram() {
        let heights = histogram_heights(&[0u32; 256]);
        assert!(heights.iter().all(|&h| h == 0.0), "no divide-by-zero when nothing was sampled");
    }

    #[test]
    fn assert_no_idle_repaint_on_scopes_window() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let pal = Palette::new(true, egui::Color32::WHITE);
        let s = stats(vec![[10, 20, 30, 255]; 4], 2, 2);
        let mut open = true;
        let mut tabs = vec![ScopeKind::Waveform.name().to_string(), ScopeKind::Histogram.name().to_string()];
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| window(ctx, &mut open, &mut tabs, Some(&s), &pal));
        }
        assert!(!ctx.has_requested_repaint(), "an unchanged FrameStats must request no repaint");
    }
}
