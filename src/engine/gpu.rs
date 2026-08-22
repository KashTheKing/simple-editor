//! GPU renderer: the effect chain as OpenGL shaders, using eframe's existing glow context.
//!
//! Why: the CPU compositor is fine for cuts but blur/VHS/motion-blur at 1080p+ are not. Every round-3
//! effect is a fragment shader here; the CPU path (engine/effects.rs) keeps the older, cheap effects so
//! the app still runs (degraded) when no GL context exists.
//!
//! Threading: GL belongs to the thread that owns the context — the UI thread. So:
//!   * preview  → `render_to_texture` during `App::update`, painted straight into the preview pane
//!                (no CPU readback at all).
//!   * export   → the export thread posts `RenderRequest`s; the UI thread serves them with `render_frame`
//!                (readback via `glReadPixels`) so preview and export run the SAME shaders.
//!   * headless (`--selftest`, no GL) → callers fall back to `engine::compose::Compositor`.
//!
//! Pipeline per frame: decode (CPU, player threads) → upload layer textures (`upload`) → for each visual
//! clip: build its chain (node graph when present, else the linear effect stack) → ping-pong FBOs →
//! transform+blend into the canvas FBO → adjustment layers re-process the canvas → subtitles/markers.
//! All textures are RGBA8 with straight alpha, matching `media::Frame`.

use crate::media::Frame;
use crate::model::{Clip, Effect, Mask, NodeGraph, Project, Scaler};
use eframe::glow;
use std::sync::Arc;

/// One reusable off-screen target (texture + FBO).
pub struct Target {
    pub tex: glow::Texture,
    pub fbo: glow::Framebuffer,
    pub w: u32,
    pub h: u32,
}

/// Compiled shader programs, cached by effect kind (and by source hash for `EffectKind::Shader`).
pub struct Programs;

/// The renderer. One per app; lives on the UI thread.
pub struct GpuRenderer {
    #[allow(dead_code)]
    gl: Arc<glow::Context>,
}

impl GpuRenderer {
    /// Compile the shader set. Err (with the GLSL log) when the driver rejects something — the caller
    /// falls back to the CPU compositor and shows the message once.
    pub fn new(_gl: Arc<glow::Context>) -> Result<Self, String> {
        todo!("engine::gpu::GpuRenderer::new")
    }

    /// Upload/refresh a decoded layer as a texture keyed by (clip id, source time bucket). Returns the
    /// texture to use; re-uploads only when the frame actually changed (`Frame` pointer/pts).
    pub fn upload(&mut self, _key: u64, _frame: &Frame) -> glow::Texture {
        todo!("engine::gpu::GpuRenderer::upload")
    }

    /// Render the whole timeline at time `t` into an off-screen target and return it, so the preview can
    /// paint it with `eframe::Frame::register_native_glow_texture`. `quality` (0.25..1.0) scales the
    /// internal resolution; the texture is always presented at `w`×`h`.
    pub fn render_to_texture(
        &mut self,
        _project: &Project,
        _t: f64,
        _w: u32,
        _h: u32,
        _quality: f32,
        _layers: &LayerSet,
    ) -> Option<glow::Texture> {
        todo!("engine::gpu::GpuRenderer::render_to_texture")
    }

    /// Same, but read back into `out` (export / export-frame / MCP `render.frame`).
    pub fn render_frame(
        &mut self,
        _project: &Project,
        _t: f64,
        _w: u32,
        _h: u32,
        _layers: &LayerSet,
        _out: &mut Frame,
    ) {
        todo!("engine::gpu::GpuRenderer::render_frame")
    }

    /// Apply one effect to a texture (used by the effect-thumbnail grid and the node editor previews).
    pub fn apply_effect(
        &mut self,
        _src: glow::Texture,
        _size: (u32, u32),
        _effect: &Effect,
        _t: f64,
    ) -> Option<glow::Texture> {
        todo!("engine::gpu::GpuRenderer::apply_effect")
    }

    /// Rasterise a mask into a single-channel texture (also used by the mask editor overlay).
    pub fn mask_texture(&mut self, _mask: &Mask, _size: (u32, u32), _t: f64, _project_w: u32) -> Option<glow::Texture> {
        todo!("engine::gpu::GpuRenderer::mask_texture")
    }

    /// Evaluate a node graph, returning the output texture.
    pub fn eval_graph(
        &mut self,
        _graph: &NodeGraph,
        _clip: &Clip,
        _t: f64,
        _layers: &LayerSet,
    ) -> Option<glow::Texture> {
        todo!("engine::gpu::GpuRenderer::eval_graph")
    }

    /// Drop cached textures for a clip/asset (source replaced, project closed).
    pub fn invalidate(&mut self, _key: Option<u64>) {
        todo!("engine::gpu::GpuRenderer::invalidate")
    }

    pub fn scaler_supported(&self, _s: Scaler) -> bool {
        true
    }
}

/// Decoded layers for one timeline instant, produced by the player/export thread on the CPU and handed
/// to the renderer: (clip id, decoded frame). Motion-blur effects also get the neighbouring frames.
#[derive(Default)]
pub struct LayerSet {
    pub layers: Vec<(crate::model::Id, Arc<Frame>)>,
    /// Extra samples for shutter-based effects: (clip id, offset seconds, frame).
    pub motion: Vec<(crate::model::Id, f64, Arc<Frame>)>,
}

impl LayerSet {
    pub fn get(&self, id: crate::model::Id) -> Option<&Arc<Frame>> {
        self.layers.iter().find(|(i, _)| *i == id).map(|(_, f)| f)
    }
}
