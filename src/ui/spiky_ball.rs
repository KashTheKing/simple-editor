//! The audio visualizer drawn over the Source monitor whenever it's showing audio with no picture
//! (and no cover art): the library's Neon Orb shader (`library/effects/neon-orb.glsl`), a glowing
//! noise orb in a dark void that swells with the loudness of what is playing. Drawn straight into
//! egui's GL context with a paint callback; the same file is the Neon Orb effect clips use, so the
//! monitor shows exactly what the effect renders. (Module name kept from the visualizers it replaced.)

use crate::media::waveform::Peaks;
use eframe::egui;
use eframe::egui_glow;
use eframe::glow::{self, HasContext};
use std::sync::{Arc, Mutex};

pub const SRC: &str = include_str!("../../library/effects/neon-orb.glsl");

/// Loudness (0..1, unscaled) of what is audible right now: RMS of `playback::SCOPE` when the speakers
/// are playing anything, else the `Peaks` envelope at `t` (no audio device).
pub fn loudness(peaks: &Peaks, t: f64) -> f32 {
    let scope: Vec<f32> = crate::playback::SCOPE.lock().map(|s| s.iter().copied().collect()).unwrap_or_default();
    if scope.iter().any(|&x| x != 0.0) {
        let rms = (scope.iter().map(|x| x * x).sum::<f32>() / scope.len() as f32).sqrt();
        return (rms * 1.4).clamp(0.0, 1.0); // ~ peak scale, like the envelope below
    }
    let (lo, hi) = peaks.range(t, t + 0.02);
    lo.abs().max(hi.abs()).clamp(0.0, 1.0)
}

/// The orb's animation state, carried between frames.
pub struct Orb {
    level: f32,
    /// Slowly falling loudest-recent value: quiet files still fill the orb's range.
    top: f32,
    time: f32,
    gl: Arc<Mutex<Option<Gl>>>,
}

struct Gl {
    prog: Option<glow::Program>,
    vao: Option<glow::VertexArray>,
}

impl Default for Orb {
    fn default() -> Self {
        Self { level: 0.0, top: 0.05, time: 0.0, gl: Arc::default() }
    }
}

impl Orb {
    /// Ease toward this frame's loudness (fast attack, slow release) and advance the shader clock.
    pub fn update(&mut self, loud: f32, dt: f32) {
        // ponytail: fixed attack/decay/auto-gain constants; promote to Settings only if someone asks.
        self.top = loud.max(self.top * (-dt / 4.0).exp()).max(0.05);
        let target = (loud / self.top).clamp(0.0, 1.0);
        let rate = if target > self.level { 25.0 } else { 5.0 };
        self.level += (target - self.level) * (rate * dt).min(1.0);
        self.time += dt;
    }

    pub fn level(&self) -> f32 {
        self.level
    }

    /// Black backdrop plus the orb, drawn by the GPU into `rect`.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect) {
        painter.rect_filled(rect, 0.0, egui::Color32::BLACK);
        if rect.width() < 2.0 || rect.height() < 2.0 {
            return;
        }
        let (gl_state, level, time) = (self.gl.clone(), self.level, self.time);
        let cb = egui_glow::CallbackFn::new(move |info, painter| {
            let gl = painter.gl();
            let mut st = gl_state.lock().unwrap_or_else(|e| e.into_inner());
            let st = st.get_or_insert_with(|| unsafe { build(gl) });
            let (Some(prog), Some(vao)) = (st.prog, st.vao) else { return };
            let vp = info.viewport_in_pixels();
            // header defaults (level, size, hue, glow, rough, speed, dark), with the live level
            let params = [level, 0.35, 0.48, 1.0, 1.0, 0.5, 1.0];
            unsafe {
                gl.use_program(Some(prog));
                gl.uniform_2_f32(gl.get_uniform_location(prog, "u_res").as_ref(), vp.width_px as f32, vp.height_px as f32);
                gl.uniform_1_f32(gl.get_uniform_location(prog, "u_time").as_ref(), time);
                for (i, v) in params.iter().enumerate() {
                    gl.uniform_1_f32(gl.get_uniform_location(prog, &format!("p{i}")).as_ref(), *v);
                }
                gl.bind_vertex_array(Some(vao));
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            }
        });
        painter.add(egui::PaintCallback { rect, callback: Arc::new(cb) });
    }
}

/// Compile the orb once on egui's context. A driver that refuses it leaves `prog` empty: the monitor
/// then shows the plain black backdrop instead of failing.
unsafe fn build(gl: &glow::Context) -> Gl {
    let stage = |kind, src: &str| {
        let s = gl.create_shader(kind).ok()?;
        gl.shader_source(s, src);
        gl.compile_shader(s);
        gl.get_shader_compile_status(s).then_some(s).or_else(|| {
            eprintln!("audio visualizer shader: {}", gl.get_shader_info_log(s));
            None
        })
    };
    let prog = (|| {
        let v = stage(glow::VERTEX_SHADER, crate::engine::shaders::VERT)?;
        let f = stage(glow::FRAGMENT_SHADER, &crate::engine::shaders::user_shader(SRC))?;
        let p = gl.create_program().ok()?;
        gl.attach_shader(p, v);
        gl.attach_shader(p, f);
        gl.link_program(p);
        gl.delete_shader(v);
        gl.delete_shader(f);
        gl.get_program_link_status(p).then_some(p)
    })();
    Gl { prog, vao: gl.create_vertex_array().ok() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orb_level_pulses_and_settles() {
        let mut o = Orb::default();
        for _ in 0..60 {
            o.update(0.6, 1.0 / 60.0);
        }
        assert!(o.level() > 0.95, "steady sound fills the range: {}", o.level());
        o.update(0.0, 1.0 / 60.0);
        assert!(o.level() > 0.8, "releases slowly, no snap to zero");
        for _ in 0..300 {
            o.update(0.0, 1.0 / 60.0);
        }
        assert!(o.level() < 0.01, "silence settles to 0");
        // a quiet file still swings the orb fully (auto-gain)
        let mut q = Orb::default();
        for _ in 0..60 {
            q.update(0.08, 1.0 / 60.0);
        }
        assert!(q.level() > 0.95, "quiet file: {}", q.level());
        assert!(o.level().is_finite() && (0.0..=1.0).contains(&q.level()));
    }

    #[test]
    fn loudness_falls_back_to_peaks() {
        let p = Peaks { min: vec![-0.5; 100], max: vec![0.25; 100] };
        let v = loudness(&p, 0.5);
        assert!((0.0..=1.0).contains(&v));
    }
}
