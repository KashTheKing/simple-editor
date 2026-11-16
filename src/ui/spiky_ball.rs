//! The audio visualizer drawn over the Source monitor whenever it's showing audio with no picture
//! (and no cover art): any visualizer style (`engine::visualizer` - Neon Orb by default, Spectrum
//! Bars, Voxel Blocks or a custom library shader), drawn straight into egui's GL context with a paint
//! callback. The same shader files are the effects a visualizer clip carries, and the audio params come
//! from the same cached song analysis the clip's keys are baked from, read at the playhead's source
//! time: the monitor shows exactly what the timeline and the export show at that moment of the song.
//! (Module name kept from the visualizers it replaced.)

use crate::engine::visualizer::{self, BANDS};
use eframe::egui;
use eframe::egui_glow;
use eframe::glow::{self, HasContext};
use std::sync::{Arc, Mutex};

/// The full-screen triangle for drawing STRAIGHT TO THE SCREEN: GL's framebuffer has row 0 at the
/// bottom, so `v_uv` is flipped here to keep every style's `uv.y = 0` at the top - the renderer's
/// convention (`shaders::VERT`, which draws into top-down textures). Without it a paint-callback
/// style comes out upside down (the 1.3.0 Voxel Blocks / Spectrum Bars in the Source monitor).
pub const SCREEN_VERT: &str = r#"#version 330 core
out vec2 v_uv;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_uv = vec2(p.x, 1.0 - p.y);
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
"#;

/// What the monitor draws: the audio params and the shader clock for ONE moment of the song. No state
/// carried between frames - set it from the source time every frame (`show`), so a seek, a stall or a
/// buffering pause can never leave it behind.
pub struct Orb {
    /// 0..1 `[level, band1..band6]` at `time`.
    level: visualizer::Frame,
    /// (`beat`, `beats`, `bpm`) at `time`.
    beat: (f64, f64, Option<f64>),
    /// The source time, which is also the shader's `u_time`.
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
        Self { level: [0.0; 1 + BANDS], beat: (0.0, 0.0, None), time: 0.0, gl: Arc::default() }
    }
}

impl Orb {
    /// Show the song at source second `t`: its analysis there (silence until the analysis exists).
    pub fn show(&mut self, song: Option<&visualizer::SongAnalysis>, t: f64) {
        self.level = song.map_or([0.0; 1 + BANDS], |a| a.at(t));
        self.beat = song.map_or((0.0, 0.0, None), |a| a.beat_at(t));
        self.time = t as f32;
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
                // the beat inputs as `bake` keys them: raw values, clamped to the range
                None => match p.id.as_str() {
                    "beat" => self.beat.0.clamp(p.min, p.max) as f32,
                    "beats" => self.beat.1.clamp(p.min, p.max) as f32,
                    "bpm" => self.beat.2.unwrap_or(p.default).clamp(p.min, p.max) as f32,
                    _ => p.default as f32,
                },
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
        let v = stage(glow::VERTEX_SHADER, SCREEN_VERT)?;
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

    /// The monitor's value is the song analysis at the source time, nothing else: the same after any
    /// history (a seek back, a long stall), and silent before the analysis exists.
    #[test]
    fn the_monitor_shows_the_analysis_at_the_source_time() {
        let mut frames = vec![[0.0; 7]; 300];
        for f in &mut frames[100..150] {
            *f = [1.0; 7];
        }
        let song = visualizer::SongAnalysis { frames, beats: vec![0.5, 1.0, 1.5, 2.0] };
        let mut o = Orb::default();
        o.show(Some(&song), 1.2);
        assert_eq!((o.level(), o.time), (1.0, 1.2));
        o.show(Some(&song), 2.7); // a seek
        assert_eq!(o.level(), 0.0);
        o.show(Some(&song), 1.2); // back again, e.g. after a stall: no leftover
        assert_eq!(o.level(), 1.0);
        assert_eq!(o.beat.1, 2.0, "two beats passed at 1.2 s");
        assert!((o.beat.0 - 0.4).abs() < 1e-9, "40 % into the beat: {}", o.beat.0);
        o.show(None, 1.2);
        assert_eq!(o.level(), 0.0, "no analysis yet: silent, never stale");
    }

    /// The screen-space triangle is the renderer's flipped exactly in y.
    #[test]
    fn screen_vert_flips_only_y() {
        assert!(SCREEN_VERT.contains("v_uv = vec2(p.x, 1.0 - p.y);"));
        assert!(crate::engine::shaders::VERT.contains("v_uv = p;"));
    }
}
