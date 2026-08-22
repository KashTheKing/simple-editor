//! GLSL sources for every GPU effect (engine/gpu.rs compiles these).
//!
//! Convention: one full-screen triangle vertex shader (`VERT`) plus a fragment shader per effect built
//! from `PRELUDE` + the effect body. Uniforms available to every effect:
//!   sampler2D tex      the layer being processed (straight alpha, top-down)
//!   vec2  u_res        layer size in pixels
//!   float u_time       clip-local seconds
//!   float u_scale      canvas px per project px (pixel-sized params must multiply by this)
//!   float p0..p7       the effect parameters in `EffectKind::params()` order
//!   sampler2D u_mask   mask coverage (1 = apply); `u_has_mask` is 0 when there is none
//! Fragment output is `out_color` (straight alpha).

/// Full-screen triangle; no vertex buffer needed (gl_VertexID based).
pub const VERT: &str = "";

/// Shared header: version, precision, varyings, the uniform block and helpers (rgb<->hsl, luma, hash
/// noise, clamped sampling, mask application).
pub const PRELUDE: &str = "";

/// Body for each `EffectKind` that runs on the GPU. `None` = geometric or CPU-only.
pub fn body(_kind: crate::model::EffectKind) -> Option<&'static str> {
    todo!("engine::shaders::body")
}

/// Blend modes as a GLSL function (`vec3 blend(int mode, vec3 s, vec3 d)`) mirroring engine::blend.
pub const BLEND: &str = "";

/// Mask rasteriser (rect / ellipse / polygon / path with feather, expand, invert, rotation).
pub const MASK: &str = "";

/// Composite/transform shader: samples a layer through an inverse 3x3 (perspective-capable) matrix with
/// the chosen `Scaler` (0 = nearest, 1 = bilinear, 2 = bicubic Catmull-Rom) and blends it onto the canvas.
pub const COMPOSITE: &str = "";

/// Wrap a user shader body (`EffectKind::Shader`) so it sees the same prelude and knobs `u1..u8`.
pub fn user_shader(_src: &str) -> String {
    todo!("engine::shaders::user_shader")
}
