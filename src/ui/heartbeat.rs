//! A heartbeat-monitor style pulse drawn over the video rect of the library asset preview whenever
//! it's showing audio with no picture (and no cover art). Pure `egui::Painter` shapes, no textures
//! or shaders. The trace is a fixed EKG-blip silhouette — it never scrolls sideways, only its spike
//! height breathes with the current amplitude, read from the same `Peaks` data the timeline waveform
//! already uses.

use crate::theme::Palette;
use eframe::egui;

/// One EKG-style blip, normalized: x in [0,1] across the rect, y is the spike's relative height
/// (0 = the flat baseline, signed so it swings both ways) before scaling by the current level.
const SHAPE: &[(f32, f32)] = &[
    (0.00, 0.0),
    (0.32, 0.0),
    (0.38, -0.12),
    (0.43, 0.0),
    (0.47, 1.0),
    (0.51, -0.55),
    (0.55, 0.12),
    (0.60, 0.0),
    (0.66, -0.22),
    (0.74, 0.0),
    (1.00, 0.0),
];

/// Smoothed current amplitude: fast attack / slower decay so the spike surges with a beat and eases
/// back, instead of jittering every frame.
#[derive(Default)]
pub struct Heartbeat {
    level: f32,
}

impl Heartbeat {
    /// Feed this frame's raw amplitude (0..1, from `amplitude_at`) and dt in seconds.
    pub fn update(&mut self, raw: f32, dt: f32) {
        let raw = raw.clamp(0.0, 1.0);
        // ponytail: fixed attack/decay constants for a snappy-hit / trailing-ease feel; promote to
        // Settings only if someone actually asks to tune it.
        let rate = if raw > self.level { 20.0 } else { 4.0 };
        self.level += (raw - self.level) * (rate * dt).min(1.0);
    }

    /// Paint the blip across `rect` (the black video letterbox) at fixed x positions — only its
    /// spike height moves, driven by the smoothed level — with a glow built from a few widening,
    /// fading strokes over the same line.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect, palette: &Palette) {
        let mid_y = rect.center().y;
        let half_h = rect.height() * 0.4;
        let scale = 0.15 + self.level * 0.85; // a faint flatline even at rest, full spike at level 1
        let pts: Vec<egui::Pos2> = SHAPE
            .iter()
            .map(|&(x, y)| egui::pos2(rect.left() + rect.width() * x, mid_y - y * half_h * scale))
            .collect();
        for (width, alpha) in [(7.0, 0.15), (4.0, 0.35), (1.75, 1.0)] {
            painter.add(egui::Shape::line(pts.clone(), egui::Stroke::new(width, palette.accent.gamma_multiply(alpha))));
        }
    }
}

/// Current amplitude: max |peak| over `[t, t + window)` across the given `Peaks` (one per
/// simultaneous audio clip) — the same `Peaks::range` call the timeline waveform already uses.
pub fn amplitude_at(peaks: &[std::sync::Arc<crate::media::waveform::Peaks>], t: f64, window: f64) -> f32 {
    peaks
        .iter()
        .map(|p| {
            let (lo, hi) = p.range(t, t + window);
            lo.abs().max(hi.abs())
        })
        .fold(0.0, f32::max)
}
