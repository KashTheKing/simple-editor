//! An "electric wave" ball drawn over the video rect of the library asset preview whenever it's
//! showing audio with no picture. Pure `egui::Painter` shapes, no textures or shaders — its outline
//! traces the same `Peaks` amplitude envelope the timeline waveform already uses, sampled around a
//! ring instead of along a line.

use crate::theme::Palette;
use eframe::egui;

/// Samples around the ring. More = smoother wave, fewer = spikier.
pub const SAMPLES: usize = 48;

/// Smoothed per-sample amplitude ring: fast attack / slower decay per sample so the outline surges
/// with beats and eases back, instead of jittering every frame.
pub struct SpikyBall {
    levels: [f32; SAMPLES],
}

impl Default for SpikyBall {
    fn default() -> Self {
        Self { levels: [0.0; SAMPLES] }
    }
}

impl SpikyBall {
    /// Feed this frame's raw per-sample amplitudes (0..1, from `waveform_ring`) and dt in seconds.
    pub fn update(&mut self, raw: &[f32; SAMPLES], dt: f32) {
        // ponytail: fixed attack/decay constants for a snappy-hit / trailing-glow feel; promote to
        // Settings only if someone actually asks to tune it.
        for (l, &r) in self.levels.iter_mut().zip(raw) {
            let r = r.clamp(0.0, 1.0);
            let rate = if r > *l { 20.0 } else { 4.0 };
            *l += (r - *l) * (rate * dt).min(1.0);
        }
    }

    /// Paint the wavy ring, centred on `rect` (the black video letterbox), with a glow built from a
    /// few widening, fading strokes over the same outline.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect, palette: &Palette) {
        let ctr = rect.center();
        let base_r = rect.size().min_elem() * 0.16;
        let pts: Vec<egui::Pos2> = (0..SAMPLES)
            .map(|i| {
                let a = i as f32 / SAMPLES as f32 * std::f32::consts::TAU;
                let r = base_r * (1.0 + self.levels[i] * 1.6);
                ctr + egui::vec2(a.cos(), a.sin()) * r
            })
            .collect();
        for (width, alpha) in [(7.0, 0.15), (4.0, 0.35), (1.75, 1.0)] {
            painter.add(egui::Shape::closed_line(pts.clone(), egui::Stroke::new(width, palette.accent.gamma_multiply(alpha))));
        }
    }
}

/// `SAMPLES` amplitudes evenly spaced across `[t - window/2, t + window/2)`, each the max |peak|
/// over its slice across every given `Peaks` (one per simultaneous audio clip) — the same
/// `Peaks::range` call the timeline waveform already uses, just sampled around a ring instead of
/// along the timeline's x axis.
pub fn waveform_ring(peaks: &[std::sync::Arc<crate::media::waveform::Peaks>], t: f64, window: f64) -> [f32; SAMPLES] {
    let mut out = [0.0f32; SAMPLES];
    let slice = window / SAMPLES as f64;
    let t0 = t - window / 2.0;
    for (i, o) in out.iter_mut().enumerate() {
        let a = t0 + slice * i as f64;
        *o = peaks
            .iter()
            .map(|p| {
                let (lo, hi) = p.range(a, a + slice);
                lo.abs().max(hi.abs())
            })
            .fold(0.0, f32::max);
    }
    out
}
