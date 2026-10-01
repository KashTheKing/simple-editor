//! The audio visualizer drawn over the Source monitor whenever it's showing audio with no picture
//! (and no cover art): a mirrored field of vertical bars receding to a horizon, rows scrolling toward
//! the viewer, in alternating blue/green bands - the look of Rahix's visualizer2 "noa-35c3" demo,
//! reimplemented from scratch here (that project is GPL-3.0; no code or shaders are taken from it).
//! Pure `egui::Painter` meshes, no textures or shaders. Bar heights are a small DFT of the audio being
//! played (`spectrum`), low notes by the centre gap; the waveform `Peaks` envelope is the fallback.
//! (Module name kept from the spiky-ball visualizer it replaced.)

use crate::media::waveform::{Peaks, PEAKS_PER_SEC};
use crate::theme::Palette;
use eframe::egui;

/// Bars on one side of the centre gap; the other side mirrors them.
pub const HALF: usize = 40;
/// Rows of history between the viewer and the horizon.
pub const ROWS: usize = 28;

/// One row of bar heights (0..1): the spectrum of what the speakers are playing right now
/// (`playback::SCOPE`) when there is any, else the `Peaks` envelope around `t` (no audio device).
pub fn spikes(peaks: &Peaks, t: f64) -> [f32; HALF] {
    let scope: Vec<f32> = crate::playback::SCOPE.lock().map(|s| s.iter().copied().collect()).unwrap_or_default();
    if scope.len() >= DFT_N && scope.iter().any(|&x| x != 0.0) {
        return spectrum(&scope[scope.len() - DFT_N..]);
    }
    envelope(peaks, t)
}

/// Samples per DFT (~11 ms at 48 kHz).
pub const DFT_N: usize = 512;

/// Log-spaced band magnitudes, 60 Hz (bar 0, by the centre gap) to 8 kHz (outermost bar), of 48 kHz
/// mono `x`: a Hann-windowed naive DFT evaluated at three frequencies per band (the loudest wins), then
/// normalised to the row's loudest band like `envelope`.
// ponytail: naive DFT, HALF*3*DFT_N ~ 60k multiply-adds per frame; an FFT only if bars or N grow a lot.
pub fn spectrum(x: &[f32]) -> [f32; HALF] {
    let n = x.len().max(1) as f32;
    let w: Vec<f32> =
        x.iter().enumerate().map(|(i, &s)| s * (0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n).cos())).collect();
    let at = |f: f32| {
        let k = std::f32::consts::TAU * f / 48_000.0;
        let (re, im) = w.iter().enumerate().fold((0.0f32, 0.0f32), |(re, im), (i, &s)| {
            let a = k * i as f32;
            (re + s * a.cos(), im - s * a.sin())
        });
        (re * re + im * im).sqrt() / n
    };
    let f = |b: f32| 60.0 * (8000.0f32 / 60.0).powf(b / (HALF - 1) as f32);
    let raw: [f32; HALF] = std::array::from_fn(|i| {
        let i = i as f32;
        [f(i - 0.33), f(i), f(i + 0.33)].into_iter().map(at).fold(0.0, f32::max)
    });
    let top = raw.iter().fold(0.02f32, |m, &s| m.max(s));
    raw.map(|s| (s / top).clamp(0.0, 1.0).sqrt())
}

/// One row of bar heights (0..1) at time `t`: bar `i` is the |peak| of the 10 ms `Peaks` bucket `i`
/// buckets before `t`, so the newest audio sits by the centre gap and older audio fans outward.
/// Deterministic in (peaks, t).
pub fn envelope(peaks: &Peaks, t: f64) -> [f32; HALF] {
    let step = 1.0 / PEAKS_PER_SEC as f64;
    let raw: [f32; HALF] = std::array::from_fn(|i| {
        let a = t - i as f64 * step;
        if a < 0.0 {
            return 0.0;
        }
        let (lo, hi) = peaks.range(a, a + step);
        lo.abs().max(hi.abs()).clamp(0.0, 1.0)
    });
    // stretch to the row's loudest bucket (floored, so near-silence stays small), like a per-row
    // normalised spectrum: tall bars on any real audio, short ones between beats
    let top = raw.iter().fold(0.3f32, |m, &s| m.max(s));
    raw.map(|s| (s / top).powi(2))
}

/// The scrolling grid: `rows[0]` is the far (newest) row, carried between frames.
pub struct SpikyBall {
    rows: Vec<[f32; HALF]>,
    live: [f32; HALF],
    /// 0..1 of a row's depth the grid has slid toward the viewer since the last row was added.
    scroll: f32,
    level: f32,
}

impl Default for SpikyBall {
    fn default() -> Self {
        Self { rows: vec![[0.0; HALF]; ROWS], live: [0.0; HALF], scroll: 0.0, level: 0.0 }
    }
}

impl SpikyBall {
    /// Ease the live row toward this frame's heights, slide the grid (faster when loud) and push a
    /// snapshot of the live row onto the far end each time a whole row has passed.
    pub fn update(&mut self, raw: &[f32; HALF], dt: f32) {
        // ponytail: fixed attack/decay/speed constants; promote to Settings only if someone asks.
        let ease = |cur: &mut f32, target: f32| {
            let rate = if target > *cur { 25.0 } else { 6.0 };
            *cur += (target - *cur) * (rate * dt).min(1.0);
        };
        for (s, &r) in self.live.iter_mut().zip(raw) {
            ease(s, r);
        }
        ease(&mut self.level, raw.iter().sum::<f32>() / HALF as f32);
        self.scroll += dt * (4.0 + 10.0 * self.level);
        while self.scroll >= 1.0 {
            self.scroll -= 1.0;
            self.rows.pop();
            self.rows.insert(0, self.live);
        }
    }

    /// Paint the field into `rect`: black backdrop, then far-to-near rows of bars (a line from a
    /// short stub below the floor to the bar's top, with a dot at each end) in perspective.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect, _palette: &Palette) {
        painter.rect_filled(rect, 0.0, egui::Color32::BLACK);
        let painter = painter.with_clip_rect(rect);
        let horizon = rect.top() + rect.height() * 0.45;
        let focal = rect.height() * 0.9;
        let (cam_z, gap, dx, row_d, near) = (0.5, 0.06, 0.07, 0.3, 0.8);
        // noa's palette: alternating deep blue / green bands, six bars wide
        let colours = [egui::Color32::from_rgb(30, 140, 215), egui::Color32::from_rgb(35, 190, 100)];
        let px = (rect.height() / 500.0).max(1.0);
        let mut mesh = egui::Mesh::default();
        let mut quad = |a: egui::Pos2, b: egui::Pos2, c: egui::Color32| {
            mesh.add_colored_rect(egui::Rect::from_two_pos(a, b), c);
        };
        for (k, row) in self.rows.iter().enumerate() {
            let depth = near + ((ROWS - k) as f32 - self.scroll) * row_d; // k = 0 is the far row
            let fade = (1.15 - (depth - near) / (ROWS as f32 * row_d)).clamp(0.0, 1.0);
            let s = focal / depth;
            for (i, &v) in row.iter().enumerate() {
                // the bars nearest the centre gap stand tallest: the "wings" silhouette
                let v = v * (1.0 - 0.55 * i as f32 / HALF as f32);
                let col = colours[(i / 6) % 2].gamma_multiply(fade);
                let top = horizon - (0.04 + 0.4 * v - cam_z) * s;
                let bot = horizon - (-0.04 - 0.12 * v - cam_z) * s;
                for side in [-1.0f32, 1.0] {
                    let x = rect.center().x + side * (gap + i as f32 * dx) * s;
                    quad(egui::pos2(x - px * 0.5, top), egui::pos2(x + px * 0.5, bot), col.gamma_multiply(0.55));
                    for y in [top, bot] {
                        quad(egui::pos2(x - px, y - px), egui::pos2(x + px, y + px), col);
                    }
                }
            }
        }
        painter.add(mesh);
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
    fn envelope_deterministic_bounded() {
        let p = peaks();
        let a = envelope(&p, 1.0);
        assert_eq!(a, envelope(&p, 1.0), "same peaks + time, same row");
        assert_ne!(a, envelope(&p, 1.3), "moves with time");
        assert!(a.iter().all(|s| (0.0..=1.0).contains(s)));
        assert!(a.windows(2).any(|w| w[0] != w[1]), "neighbours differ");
        // before the start / past the end: silent, not a panic
        assert!(envelope(&p, -5.0).iter().all(|&s| s == 0.0));
        assert!(envelope(&p, 99.0).iter().all(|&s| s == 0.0));
        // a clipping file still stays bounded
        let loud = Peaks { min: vec![-3.0; 200], max: vec![3.0; 200] };
        assert!(envelope(&loud, 1.0).iter().all(|&s| s == 1.0));
    }

    /// A 200 Hz tone lights the low bars by the centre gap, a 5 kHz one the outer bars.
    #[test]
    fn spectrum_follows_pitch() {
        let tone = |f: f32| -> Vec<f32> {
            (0..DFT_N).map(|i| (std::f32::consts::TAU * f * i as f32 / 48_000.0).sin() * 0.5).collect()
        };
        let peak = |row: [f32; HALF]| row.iter().enumerate().fold(0, |b, (i, &v)| if v > row[b] { i } else { b });
        let (lo, hi) = (spectrum(&tone(200.0)), spectrum(&tone(5000.0)));
        assert!(peak(lo) < HALF / 3, "200 Hz peaks at bar {}", peak(lo));
        assert!(peak(hi) > HALF * 2 / 3, "5 kHz peaks at bar {}", peak(hi));
        assert!(lo.iter().chain(&hi).all(|v| (0.0..=1.0).contains(v)));
        assert!(spectrum(&[0.0; DFT_N]).iter().all(|&v| v == 0.0), "silence stays flat");
    }

    #[test]
    fn rows_scroll_toward_the_viewer() {
        let mut b = SpikyBall::default();
        for _ in 0..600 {
            b.update(&[1.0; HALF], 1.0 / 60.0);
        }
        assert_eq!(b.rows.len(), ROWS, "history never grows");
        assert!(b.rows.iter().all(|r| r.iter().all(|&s| s > 0.9 && s <= 1.0)), "loud rows filled the field");
        assert!((0.0..1.0).contains(&b.scroll));
        b.update(&[0.0; HALF], 1.0 / 60.0);
        assert!(b.live[0] > 0.5, "decays slowly, no snap to zero");
        // a paused ball (no update) paints without panicking at any size
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let p = ctx.layer_painter(egui::LayerId::background());
            b.paint(
                &p,
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 360.0)),
                &Palette::new(true, egui::Color32::WHITE),
            );
            b.paint(
                &p,
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(0.0, 0.0)),
                &Palette::new(true, egui::Color32::WHITE),
            );
        });
    }
}
