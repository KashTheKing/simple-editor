//! ---- ws:perf-baseline ----
//! The frame-time HUD (`Settings::hud`, toggled by `Action::ToggleHud`): a small text overlay at the
//! Preview's top-left. `Hud::end_frame` runs once at the end of `App::update` and records the UI-thread
//! frame time plus the GPU composite / texture-upload time `GpuRenderer` accumulated; render-thread
//! decode and cache numbers are sampled twice a second from `playback::decode_stats` / `Player`.
//! Painting never asks for a repaint: an idle app keeps showing the last numbers.

use crate::engine::gpu::GpuRenderer;
use eframe::egui;
use std::collections::VecDeque;
use std::time::Instant;

/// Rolling window of the last `CAP` samples (ms).
#[derive(Default)]
pub struct Window(VecDeque<f32>);

const CAP: usize = 120;

impl Window {
    pub fn push(&mut self, ms: f32) {
        if self.0.len() == CAP {
            self.0.pop_front();
        }
        self.0.push_back(ms);
    }
    pub fn avg(&self) -> f32 {
        if self.0.is_empty() {
            return 0.0;
        }
        self.0.iter().sum::<f32>() / self.0.len() as f32
    }
    /// Nearest-rank 95th percentile.
    pub fn p95(&self) -> f32 {
        if self.0.is_empty() {
            return 0.0;
        }
        let mut v: Vec<f32> = self.0.iter().copied().collect();
        v.sort_by(|a, b| a.total_cmp(b));
        let rank = ((v.len() as f32 * 0.95).ceil() as usize).clamp(1, v.len());
        v[rank - 1]
    }
}

#[derive(Default)]
pub struct Hud {
    pub frame: Window,
    pub composite: Window,
    pub upload: Window,
    /// Mean / worst layer-set decode (ms) over the last sample period.
    pub decode_avg_ms: f32,
    pub decode_worst_ms: f32,
    /// Share of frames served from the player's cache over the last sample period (0..1).
    pub hit_rate: f32,
    /// (decodes, total decode µs, cache hits) at the last sample.
    last: (u64, u64, u64),
    sampled: Option<Instant>,
}

impl Hud {
    /// End of `App::update`: `started` is when it began. Takes (and zeroes) the GPU accumulators.
    pub fn end_frame(&mut self, started: Instant, gpu: Option<&mut GpuRenderer>, cache_hits: u64) {
        self.frame.push(started.elapsed().as_secs_f32() * 1e3);
        if let Some(g) = gpu {
            let (c, u) = (std::mem::take(&mut g.perf_composite_us), std::mem::take(&mut g.perf_upload_us));
            if c > 0 {
                self.composite.push(c as f32 / 1e3);
                self.upload.push(u as f32 / 1e3);
            }
        }
        if self.sampled.is_some_and(|s| s.elapsed().as_millis() < 500) {
            return;
        }
        self.sampled = Some(Instant::now());
        let (n, us, worst) = crate::playback::decode_stats::take();
        let (dn, dus, dh) = (n - self.last.0, us - self.last.1, cache_hits.saturating_sub(self.last.2));
        self.decode_avg_ms = if dn > 0 { dus as f32 / dn as f32 / 1e3 } else { 0.0 };
        self.decode_worst_ms = worst as f32 / 1e3;
        self.hit_rate = if dn + dh > 0 { dh as f32 / (dn + dh) as f32 } else { 0.0 };
        self.last = (n, us, cache_hits);
    }

    pub fn lines(&self, dropped: u64) -> String {
        format!(
            "UI {:.1} ms avg / {:.1} p95\nGL composite {:.1} ms · upload {:.1} ms\ndecode {:.1} ms avg / {:.1} worst\ncache {:.0} % · dropped {dropped}",
            self.frame.avg(),
            self.frame.p95(),
            self.composite.avg(),
            self.upload.avg(),
            self.decode_avg_ms,
            self.decode_worst_ms,
            self.hit_rate * 100.0,
        )
    }
}

/// Paint the HUD at the top-left of `ui`'s rect (the Preview pane).
pub fn draw(ui: &egui::Ui, hud: &Hud, dropped: u64) {
    let p = ui.painter();
    let at = ui.max_rect().left_top() + egui::vec2(8.0, 8.0);
    let galley = p.layout_no_wrap(hud.lines(dropped), egui::FontId::monospace(11.0), egui::Color32::WHITE);
    let bg = egui::Rect::from_min_size(at, galley.size()).expand(4.0);
    p.rect_filled(bg, 3.0, egui::Color32::from_black_alpha(170));
    p.galley(at, galley, egui::Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_avg_and_p95() {
        let mut w = Window::default();
        for i in 1..=100 {
            w.push(i as f32);
        }
        assert_eq!(w.avg(), 50.5);
        assert_eq!(w.p95(), 95.0);
        for _ in 0..CAP {
            w.push(1.0);
        }
        assert_eq!((w.avg(), w.p95()), (1.0, 1.0), "old samples roll out");
    }

    #[test]
    fn assert_no_idle_repaint_hud() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut hud = Hud::default();
        hud.end_frame(Instant::now(), None, 0);
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| draw(ui, &hud, 0));
            });
        }
        assert!(!ctx.has_requested_repaint(), "the HUD must request no repaint while idle");
    }
}
