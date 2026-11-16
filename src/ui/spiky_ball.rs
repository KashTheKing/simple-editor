//! The audio visualizer drawn over the Source monitor whenever it's showing audio with no picture
//! (and no cover art): any visualizer style (`engine::visualizer` - Neon Orb by default, the classic
//! Spectrum Bars, or a custom library shader), drawn straight into egui's GL context with a paint
//! callback. The same shader files are the effects a visualizer clip carries, so the monitor shows
//! what the clip renders; here the audio params come live from what the speakers play instead of
//! baked keys. (Module name kept from the visualizers it replaced.)

use crate::engine::visualizer::{self, BANDS};
use crate::media::waveform::Peaks;
use eframe::egui;
use eframe::egui_glow;
use eframe::glow::{self, HasContext};
use std::sync::{Arc, Mutex};

/// What is audible right now: `[peak-ish loudness, band1..band6]` (unscaled) from `playback::SCOPE`
/// when the speakers are playing anything, else the `Peaks` envelope at `t` in every slot (no device).
pub fn live(peaks: &Peaks, t: f64) -> visualizer::Frame {
    let scope: Vec<f32> = crate::playback::SCOPE.lock().map(|s| s.iter().copied().collect()).unwrap_or_default();
    if scope.iter().any(|&x| x != 0.0) {
        let mut f = visualizer::raw_frame(&scope);
        let rms = (scope.iter().map(|x| x * x).sum::<f32>() / scope.len() as f32).sqrt();
        f[0] = (rms * 1.4).clamp(0.0, 1.0); // ~ peak scale, like the envelope below
        return f;
    }
    let (lo, hi) = peaks.range(t, t + 0.02);
    [lo.abs().max(hi.abs()).clamp(0.0, 1.0); 1 + BANDS]
}

/// The visualizer's animation state, carried between frames.
pub struct Orb {
    /// Eased 0..1 `[level, band1..band6]`.
    level: visualizer::Frame,
    /// Slowly falling loudest-recent value per slot: quiet files and weak highs still fill the range.
    top: visualizer::Frame,
    time: f32,
    /// (source the program was built from, program) - rebuilt when the style changes.
    gl: Arc<Mutex<Option<(String, Gl)>>>,
}

struct Gl {
    prog: Option<glow::Program>,
    vao: Option<glow::VertexArray>,
}

impl Default for Orb {
    fn default() -> Self {
        Self { level: [0.0; 1 + BANDS], top: [0.05; 1 + BANDS], time: 0.0, gl: Arc::default() }
    }
}

impl Orb {
    /// Ease toward this frame's audio (fast attack, slow release) and advance the shader clock.
    pub fn update(&mut self, raw: visualizer::Frame, dt: f32) {
        // ponytail: fixed attack/decay/auto-gain constants; promote to Settings only if someone asks.
        for c in 0..1 + BANDS {
            self.top[c] = raw[c].max(self.top[c] * (-dt / 4.0).exp()).max(if c == 0 { 0.05 } else { 1e-3 });
            let target = (raw[c] / self.top[c]).clamp(0.0, 1.0);
            let target = if c == 0 { target } else { target.sqrt() };
            let rate = if target > self.level[c] { 25.0 } else { 5.0 };
            self.level[c] += (target - self.level[c]) * (rate * dt).min(1.0);
        }
        self.time += dt;
    }

    pub fn level(&self) -> f32 {
        self.level[0]
    }

    /// Black backdrop plus the style `src`, drawn by the GPU into `rect`.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect, src: &str) {
        painter.rect_filled(rect, 0.0, egui::Color32::BLACK);
        if rect.width() < 2.0 || rect.height() < 2.0 {
            return;
        }
        // header defaults, with the audio params live
        let meta = crate::engine::shaderlib::parse_meta(src);
        let params: Vec<f32> = meta
            .params
            .iter()
            .map(|p| match visualizer::AUDIO_PARAMS.iter().position(|a| *a == p.id) {
                Some(c) => (p.min + (p.max - p.min) * self.level[c] as f64) as f32,
                None => p.default as f32,
            })
            .collect();
        let (gl_state, time, src) = (self.gl.clone(), self.time, src.to_string());
        let cb = egui_glow::CallbackFn::new(move |info, painter| {
            let gl = painter.gl();
            let mut st = gl_state.lock().unwrap_or_else(|e| e.into_inner());
            if st.as_ref().is_some_and(|(s, _)| *s != src) {
                if let Some((_, old)) = st.take() {
                    unsafe {
                        old.prog.map(|p| gl.delete_program(p));
                        old.vao.map(|v| gl.delete_vertex_array(v));
                    }
                }
            }
            let (_, st) = st.get_or_insert_with(|| (src.clone(), unsafe { build(gl, &src) }));
            let (Some(prog), Some(vao)) = (st.prog, st.vao) else { return };
            let vp = info.viewport_in_pixels();
            unsafe {
                gl.use_program(Some(prog));
                gl.uniform_2_f32(gl.get_uniform_location(prog, "u_res").as_ref(), vp.width_px as f32, vp.height_px as f32);
                gl.uniform_1_f32(gl.get_uniform_location(prog, "u_time").as_ref(), time);
                gl.uniform_1_f32(gl.get_uniform_location(prog, "u_scale").as_ref(), 1.0);
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

/// Compile a style once on egui's context. A driver that refuses it leaves `prog` empty: the monitor
/// then shows the plain black backdrop instead of failing.
unsafe fn build(gl: &glow::Context, src: &str) -> Gl {
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
        let f = stage(glow::FRAGMENT_SHADER, &crate::engine::shaders::user_shader(src))?;
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
            o.update([0.6; 7], 1.0 / 60.0);
        }
        assert!(o.level() > 0.95, "steady sound fills the range: {}", o.level());
        o.update([0.0; 7], 1.0 / 60.0);
        assert!(o.level() > 0.8, "releases slowly, no snap to zero");
        for _ in 0..300 {
            o.update([0.0; 7], 1.0 / 60.0);
        }
        assert!(o.level() < 0.01, "silence settles to 0");
        // a quiet file still swings the orb fully (auto-gain)
        let mut q = Orb::default();
        for _ in 0..60 {
            q.update([0.08; 7], 1.0 / 60.0);
        }
        assert!(q.level() > 0.95, "quiet file: {}", q.level());
        assert!(o.level().is_finite() && (0.0..=1.0).contains(&q.level()));
    }

    #[test]
    fn live_falls_back_to_peaks() {
        let p = Peaks { min: vec![-0.5; 100], max: vec![0.25; 100] };
        assert!(live(&p, 0.5).iter().all(|v| (0.0..=1.0).contains(v)));
    }
}
