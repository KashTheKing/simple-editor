//! A heartbeat-monitor style trace drawn over the video rect of the library asset preview whenever
//! it's showing audio with no picture. Pure `egui::Painter` shapes, no textures or shaders — its
//! line traces the same `Peaks` amplitude envelope the timeline waveform already uses, sampled
//! across a short time window centred on the playhead.

use crate::theme::Palette;
use eframe::egui;

/// Samples across the trace. More = smoother line, fewer = choppier.
pub const SAMPLES: usize = 64;

/// Smoothed per-sample amplitude trace: fast attack / slower decay per sample so the line surges
/// with beats and eases back, instead of jittering every frame.
pub struct Heartbeat {
    levels: [f32; SAMPLES],
}

impl Default for Heartbeat {
    fn default() -> Self {
        Self { levels: [0.0; SAMPLES] }
    }
}

impl Heartbeat {
    /// Feed this frame's raw per-sample amplitudes (-1..1, from `waveform_trace`) and dt in seconds.
    pub fn update(&mut self, raw: &[f32; SAMPLES], dt: f32) {
        // ponytail: fixed attack/decay constants for a snappy-hit / trailing-glow feel; promote to
        // Settings only if someone actually asks to tune it.
        for (l, &r) in self.levels.iter_mut().zip(raw) {
            let r = r.clamp(-1.0, 1.0);
            let rate = if r.abs() > l.abs() { 20.0 } else { 4.0 };
            *l += (r - *l) * (rate * dt).min(1.0);
        }
    }

    /// Paint a horizontal EKG-style trace across `rect` (the black video letterbox), with a glow
    /// built from a few widening, fading strokes over the same line.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect, palette: &Palette) {
        let mid_y = rect.center().y;
        let half_h = rect.height() * 0.4;
        let pts: Vec<egui::Pos2> = (0..SAMPLES)
            .map(|i| {
                let x = rect.left() + rect.width() * i as f32 / (SAMPLES - 1) as f32;
                egui::pos2(x, mid_y - self.levels[i] * half_h)
            })
            .collect();
        for (width, alpha) in [(7.0, 0.15), (4.0, 0.35), (1.75, 1.0)] {
            painter.add(egui::Shape::line(pts.clone(), egui::Stroke::new(width, palette.accent.gamma_multiply(alpha))));
        }
    }
}

/// `SAMPLES` signed amplitudes evenly spaced across `[t - window/2, t + window/2)`: each is the
/// peak (min or max, whichever has the larger magnitude, sign kept) over its slice across every
/// given `Peaks` (one per simultaneous audio clip) — the same `Peaks::range` call the timeline
/// waveform already uses, just sampled across a fixed window instead of the whole visible timeline.
pub fn waveform_trace(peaks: &[std::sync::Arc<crate::media::waveform::Peaks>], t: f64, window: f64) -> [f32; SAMPLES] {
    let mut out = [0.0f32; SAMPLES];
    let slice = window / SAMPLES as f64;
    let t0 = t - window / 2.0;
    for (i, o) in out.iter_mut().enumerate() {
        let a = t0 + slice * i as f64;
        *o = peaks
            .iter()
            .map(|p| {
                let (lo, hi) = p.range(a, a + slice);
                if lo.abs() > hi.abs() { lo } else { hi }
            })
            .fold(0.0, |acc: f32, v| if v.abs() > acc.abs() { v } else { acc });
    }
    out
}
