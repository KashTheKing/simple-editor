//! A radial "spiky ball" audio visualizer drawn over the Source monitor whenever it's showing audio
//! with no picture (and no cover art). Pure `egui::Painter` meshes, no textures or shaders. Spike
//! lengths come from the same `Peaks` data the timeline waveform already uses.

use crate::media::waveform::{Peaks, PEAKS_PER_SEC};
use crate::theme::Palette;
use eframe::egui;
use std::f32::consts::TAU;

/// Spikes on one side; the other side mirrors them, so the ball has `2 * HALF` spikes.
pub const HALF: usize = 40;

/// Raw spike lengths (0..1) for one half of the ball at time `t`: spike `i` is the |peak| of the
/// 10 ms `Peaks` bucket `i - HALF/2` buckets away from `t`, so the half spans ~0.56 s of audio
/// centred on the playhead and neighbouring spikes differ. Deterministic in (peaks, t).
pub fn spikes(peaks: &Peaks, t: f64) -> [f32; HALF] {
    let step = 1.0 / PEAKS_PER_SEC as f64;
    let raw: [f32; HALF] = std::array::from_fn(|i| {
        let a = t + (i as f64 - (HALF / 2) as f64) * step;
        if a < 0.0 {
            return 0.0;
        }
        let (lo, hi) = peaks.range(a, a + step);
        lo.abs().max(hi.abs()).clamp(0.0, 1.0)
    });
    // stretch to the window's loudest bucket (floored, so near-silence stays small): full-length
    // spikes on any real audio, and the quiet buckets between beats read as short ones
    let top = raw.iter().fold(0.3f32, |m, &s| m.max(s));
    raw.map(|s| (s / top).powi(2))
}

/// The full ring: `half` then its mirror image, so spike `k` and spike `2*HALF-1-k` are equal.
pub fn mirrored(half: &[f32; HALF]) -> [f32; 2 * HALF] {
    std::array::from_fn(|k| if k < HALF { half[k] } else { half[2 * HALF - 1 - k] })
}

/// Smoothed spikes + loudness + rotation, carried between frames.
pub struct SpikyBall {
    spikes: [f32; HALF],
    level: f32,
    angle: f32,
}

impl Default for SpikyBall {
    fn default() -> Self {
        Self { spikes: [0.0; HALF], level: 0.0, angle: 0.0 }
    }
}

impl SpikyBall {
    /// Ease toward this frame's raw spikes (fast attack, slower decay) and turn slowly.
    pub fn update(&mut self, raw: &[f32; HALF], dt: f32) {
        // ponytail: fixed attack/decay/spin constants; promote to Settings only if someone asks.
        let ease = |cur: &mut f32, target: f32| {
            let rate = if target > *cur { 25.0 } else { 6.0 };
            *cur += (target - *cur) * (rate * dt).min(1.0);
        };
        for (s, &r) in self.spikes.iter_mut().zip(raw) {
            ease(s, r);
        }
        let loud = raw.iter().sum::<f32>() / HALF as f32;
        ease(&mut self.level, loud);
        self.angle = (self.angle + dt * 0.25) % TAU;
    }

    /// Paint the ball centred in `rect`: a glow, then the spike star as a centre-fan mesh with a
    /// gradient from solid accent at the ball to faint at the tips, then the pulsing inner ball.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect, palette: &Palette) {
        let c = rect.center();
        let base = rect.width().min(rect.height()) * 0.2;
        let r0 = base * (1.0 + 0.2 * self.level);
        let reach = base * 1.3;
        let ring = mirrored(&self.spikes);
        let n = ring.len();
        let dir = |a: f32| egui::vec2(a.cos(), a.sin());
        // a still ball still looks like a ball: a small static ripple under the live spike lengths
        let len = |k: usize| 0.08 + 0.06 * ((k as f32 * 6.0 / n as f32) * TAU).sin().abs() + ring[k] * 0.86;
        let accent = palette.accent;
        for (scale, alpha) in [(1.3, 0.03), (1.12, 0.06)] {
            painter.circle_filled(c, (r0 + reach * 0.35) * scale, accent.gamma_multiply(alpha));
        }
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(c, accent.gamma_multiply(0.9));
        for k in 0..n {
            let a = self.angle + k as f32 / n as f32 * TAU;
            let w = TAU / n as f32 * 0.35; // spike half-width: a sliver of gap between spikes
            mesh.colored_vertex(c + dir(a - w) * r0, accent.gamma_multiply(0.8));
            mesh.colored_vertex(c + dir(a) * (r0 + reach * len(k)), accent.gamma_multiply(0.2));
            mesh.colored_vertex(c + dir(a + w) * r0, accent.gamma_multiply(0.8));
        }
        let verts = 3 * n as u32;
        for v in 1..=verts {
            mesh.add_triangle(0, v, v % verts + 1);
        }
        painter.add(mesh);
        painter.circle(c, r0 * 0.92, egui::Color32::BLACK.gamma_multiply(0.55), egui::Stroke::new(1.5, accent));
        painter.circle_filled(c, r0 * (0.3 + 0.4 * self.level), accent.gamma_multiply(0.35 + 0.5 * self.level));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peaks() -> Peaks {
        // 2 s of a ramp-ish signal so every bucket differs
        let max: Vec<f32> = (0..200).map(|i| ((i as f32 * 0.37).sin() * 0.9).abs()).collect();
        Peaks { min: max.iter().map(|m| -m * 0.5).collect(), max }
    }

    #[test]
    fn spikes_deterministic_bounded_symmetric() {
        let p = peaks();
        let a = spikes(&p, 1.0);
        assert_eq!(a, spikes(&p, 1.0), "same peaks + time, same spikes");
        assert_ne!(a, spikes(&p, 1.3), "moves with time");
        assert!(a.iter().all(|s| (0.0..=1.0).contains(s)));
        assert!(a.windows(2).any(|w| w[0] != w[1]), "neighbours differ");
        let ring = mirrored(&a);
        assert!((0..2 * HALF).all(|k| ring[k] == ring[2 * HALF - 1 - k]), "mirror symmetric");
        // before the start / past the end: silent, not a panic
        assert!(spikes(&p, -5.0).iter().all(|&s| s == 0.0));
        assert!(spikes(&p, 99.0).iter().all(|&s| s == 0.0));
        // a clipping file still stays bounded
        let loud = Peaks { min: vec![-3.0; 200], max: vec![3.0; 200] };
        assert!(spikes(&loud, 1.0).iter().all(|&s| s == 1.0));
    }

    #[test]
    fn update_eases_and_stays_bounded() {
        let mut b = SpikyBall::default();
        for _ in 0..200 {
            b.update(&[1.0; HALF], 1.0 / 60.0);
        }
        assert!(b.spikes.iter().all(|&s| s > 0.95 && s <= 1.0) && b.level <= 1.0);
        b.update(&[0.0; HALF], 1.0 / 60.0);
        assert!(b.spikes[0] > 0.5, "decays slowly, no snap to zero");
    }
}
