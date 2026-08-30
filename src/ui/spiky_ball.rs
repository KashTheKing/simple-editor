//! A procedural "spiky ball" drawn over the video rect whenever a preview pane is showing audio
//! with no picture (see `Project::has_video_at`). Pure `egui::Painter` shapes, no textures or
//! shaders — driven by the same `Peaks` amplitude envelope the timeline waveform already uses.

use crate::theme::Palette;
use eframe::egui;

/// Smoothed amplitude for one preview's spiky ball: fast attack / slower decay so it pulses with
/// beats instead of jittering every frame.
#[derive(Default)]
pub struct SpikyBall {
    level: f32,
}

impl SpikyBall {
    /// Feed this frame's raw amplitude (0..1, e.g. from `amplitude_at`) and dt in seconds.
    pub fn update(&mut self, raw: f32, dt: f32) {
        let raw = raw.clamp(0.0, 1.0);
        // ponytail: fixed attack/decay constants for a snappy-hit / trailing-pulse feel;
        // promote to Settings only if someone actually asks to tune it.
        let rate = if raw > self.level { 20.0 } else { 4.0 };
        self.level += (raw - self.level) * (rate * dt).min(1.0);
    }

    /// Paint a spiky ball centred on `rect` (the black video letterbox).
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect, palette: &Palette) {
        const SPIKES: usize = 16;
        let ctr = rect.center();
        let base_r = rect.size().min_elem() * 0.12;
        let spike_r = base_r * (1.0 + self.level * 1.8);
        let pts: Vec<_> = (0..SPIKES * 2)
            .map(|i| {
                let a = i as f32 * std::f32::consts::PI / SPIKES as f32;
                let r = if i % 2 == 0 { spike_r } else { base_r };
                ctr + egui::vec2(a.cos(), a.sin()) * r
            })
            .collect();
        painter.add(egui::Shape::convex_polygon(pts, palette.accent.gamma_multiply(0.85), egui::Stroke::NONE));
        painter.circle_stroke(ctr, base_r, egui::Stroke::new(1.5, palette.accent));
    }
}

/// Amplitude at time `t`: max |peak| over `[t, t + window)` across the given `Peaks` (one per
/// simultaneous audio clip) — same `Peaks::range` call the timeline waveform already uses.
pub fn amplitude_at(peaks: &[std::sync::Arc<crate::media::waveform::Peaks>], t: f64, window: f64) -> f32 {
    peaks
        .iter()
        .map(|p| {
            let (lo, hi) = p.range(t, t + window);
            lo.abs().max(hi.abs())
        })
        .fold(0.0, f32::max)
}
