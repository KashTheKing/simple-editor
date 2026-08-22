//! "Shader Editor" window for `EffectKind::Shader`: the GLSL body in a monospace editor, an Apply
//! button, and the driver's compile log verbatim (selectable, so it can be pasted somewhere) when the
//! source was rejected. Non-modal — the timeline stays usable while it is open.
//! The window only edits text: the app writes the source back onto the effect, pushes undo and asks
//! `GpuRenderer::check_shader` for the log, then hands it here through `error`.

use crate::model::{Id, DEFAULT_SHADER};
use eframe::egui;

/// What `shaders::user_shader` puts in scope — the knobs are invisible otherwise.
const UNIFORMS: &str = "sampler2D tex — the layer\n\
vec2 u_res — layer size in pixels\n\
float u_time — clip-local seconds\n\
float u_scale — preview scale (multiply pixel-sized amounts by it)\n\
float p0..p7 — the eight knobs, also named u1..u8\n\
sampler2D u_mask, int u_has_mask — the effect's mask (applied for you)";

#[derive(Default)]
pub struct ShaderUi {
    pub open: bool,
    /// Clip + index in its effect stack being edited.
    pub target: Option<(Id, usize)>,
    pub src: String,
    /// The driver's log for the last applied source; empty once it links.
    pub error: String,
}

impl ShaderUi {
    /// Point the window at one effect's source (the Effects panel's "Edit shader…").
    pub fn edit(&mut self, clip: Id, index: usize, src: &str) {
        self.open = true;
        self.target = Some((clip, index));
        self.src = src.to_string();
        self.error.clear();
    }
}

/// Draws the window; true = "Apply" (the app writes the source back and compiles it).
pub fn show(ctx: &egui::Context, state: &mut ShaderUi) -> bool {
    if !state.open {
        return false;
    }
    if state.src.trim().is_empty() {
        state.src = DEFAULT_SHADER.to_string(); // an empty buffer teaches nobody the entry point
    }
    let mut open = state.open;
    let mut apply = false;
    egui::Window::new("Shader Editor").open(&mut open).default_width(560.0).show(ctx, |ui| {
        ui.weak("Define vec4 effect(vec4 src, vec2 uv) — uv is 0..1, colours are straight alpha.");
        egui::ScrollArea::vertical().id_salt("shader_src").max_height(280.0).show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut state.src).code_editor().desired_rows(16).desired_width(f32::INFINITY),
            );
        });
        ui.horizontal(|ui| {
            apply = ui.button("Apply").on_hover_text("Compile this source and use it").clicked();
            if ui.button("Reset").on_hover_text("Back to the default body").clicked() {
                state.src = DEFAULT_SHADER.to_string();
            }
        });
        ui.collapsing("Uniforms", |ui| ui.weak(UNIFORMS));
        if !state.error.is_empty() {
            let red = ui.visuals().error_fg_color;
            ui.colored_label(red, "GLSL error");
            egui::ScrollArea::vertical().id_salt("shader_err").max_height(160.0).show(ui, |ui| {
                // read-only TextEdit: the log stays selectable and verbatim, wrapping and all
                let mut log = state.error.as_str();
                ui.add(egui::TextEdit::multiline(&mut log).code_editor().text_color(red).desired_width(f32::INFINITY));
            });
        }
    });
    state.open = open;
    apply
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text painted in one frame (the same trick library.rs uses to find a button).
    fn painted(out: &egui::FullOutput) -> String {
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::epaint::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Headless: the window lays out, applies nothing on its own, seeds an empty body, and prints the
    /// driver's log verbatim when the app hands one back.
    #[test]
    fn show_headless_reports_the_glsl_log() {
        let ctx = egui::Context::default();
        let mut st = ShaderUi::default();
        st.edit(7, 0, "");
        st.error = "0(12) : error C1503: undefined variable \"foo\"".into();
        let _ = ctx.run(egui::RawInput::default(), |ctx| assert!(!show(ctx, &mut st)));
        // second frame: the window knows its size and the collapsing header is settled
        let out = ctx.run(egui::RawInput::default(), |ctx| assert!(!show(ctx, &mut st)));
        assert!(st.open && st.target == Some((7, 0)));
        assert_eq!(st.src, DEFAULT_SHADER, "an empty body is seeded with the working default");
        let text = painted(&out);
        assert!(text.contains("undefined variable"), "the log must be shown verbatim: {text}");
        assert!(text.contains("Apply") && text.contains("vec4 effect"), "{text}");
    }

    /// Closing the window (or never opening it) is a no-op.
    #[test]
    fn closed_window_draws_nothing() {
        let ctx = egui::Context::default();
        let mut st = ShaderUi::default();
        let out = ctx.run(egui::RawInput::default(), |ctx| assert!(!show(ctx, &mut st)));
        assert!(!painted(&out).contains("Apply"));
    }
}
