//! GLSL sources for every GPU effect (engine/gpu.rs compiles these).
//!
//! Convention: one full-screen triangle vertex shader (`VERT`) plus a fragment shader per effect built
//! from `PRELUDE` + the effect body + `MAIN`. A **body** defines
//!
//! ```glsl
//! vec4 effect(vec4 src, vec2 uv) { ... }   // straight alpha in, straight alpha out
//! ```
//!
//! and nothing else (it may declare extra uniforms/helpers above it). `MAIN` samples `tex`, calls
//! `effect` and mixes the result back through the mask — so no body ever writes `out_color` itself and
//! every effect gets masking for free. `fragment()` does the assembly.
//!
//! Uniforms available to every effect:
//!   sampler2D tex      the layer being processed (straight alpha, top-down)
//!   vec2  u_res        layer size in pixels
//!   float u_time       clip-local seconds
//!   float u_scale      canvas px per project px (pixel-sized params must multiply by this)
//!   float p0..p7       the effect parameters in `EffectKind::params()` order
//!   sampler2D u_mask   mask coverage (1 = apply); `u_has_mask` is 0 when there is none
//! Fragment output is `out_color` (straight alpha).
//!
//! Textures are uploaded top-down and the vertex shader flips the clip-space Y instead, so `uv.y = 0`
//! is the top of the image everywhere — in the sampler, in `glReadPixels` output and in egui.

use crate::model::EffectKind;

/// Full-screen triangle; no vertex buffer needed (gl_VertexID based).
pub const VERT: &str = r#"#version 330 core
out vec2 v_uv;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_uv = p;
    // uv.y = 0 lands on GL row 0, so a top-down upload stays top-down in the FBO, in glReadPixels
    // output and in egui — no flip anywhere.
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
"#;

/// Shared header: version, precision, varyings, the uniform block and helpers (rgb<->hsl, luma, hash
/// noise, clamped sampling, mask application).
pub const PRELUDE: &str = r#"#version 330 core
precision highp float;

in vec2 v_uv;
out vec4 out_color;

uniform sampler2D tex;
uniform vec2 u_res;
uniform float u_time;
uniform float u_scale;
uniform float p0;
uniform float p1;
uniform float p2;
uniform float p3;
uniform float p4;
uniform float p5;
uniform float p6;
uniform float p7;
uniform sampler2D u_mask;
uniform int u_has_mask;

float luma(vec3 c) { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }

float hash12(vec2 p) {
    vec3 q = fract(vec3(p.xyx) * 0.1031);
    q += dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

float hash11(float x) { return hash12(vec2(x, x * 1.7 + 0.37)); }

vec3 rgb2hsl(vec3 c) {
    float mx = max(c.r, max(c.g, c.b));
    float mn = min(c.r, min(c.g, c.b));
    float l = (mx + mn) * 0.5;
    float d = mx - mn;
    float h = 0.0;
    float s = 0.0;
    if (d > 1e-6) {
        s = (l > 0.5) ? d / max(2.0 - mx - mn, 1e-6) : d / max(mx + mn, 1e-6);
        if (mx == c.r) {
            h = (c.g - c.b) / d + ((c.g < c.b) ? 6.0 : 0.0);
        } else if (mx == c.g) {
            h = (c.b - c.r) / d + 2.0;
        } else {
            h = (c.r - c.g) / d + 4.0;
        }
        h /= 6.0;
    }
    return vec3(h, s, l);
}

float hue2rgb(float p, float q, float t) {
    t = fract(t);
    if (t < 1.0 / 6.0) { return p + (q - p) * 6.0 * t; }
    if (t < 0.5) { return q; }
    if (t < 2.0 / 3.0) { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
    return p;
}

vec3 hsl2rgb(vec3 c) {
    if (c.y <= 0.0) { return vec3(c.z); }
    float q = (c.z < 0.5) ? c.z * (1.0 + c.y) : c.z + c.y - c.z * c.y;
    float p = 2.0 * c.z - q;
    return vec3(hue2rgb(p, q, c.x + 1.0 / 3.0), hue2rgb(p, q, c.x), hue2rgb(p, q, c.x - 1.0 / 3.0));
}

/// Sample `t` with the uv clamped to the half-texel border of a `u_res`-sized image.
vec4 sample_clamped(sampler2D t, vec2 uv) {
    vec2 half_texel = 0.5 / max(u_res, vec2(1.0));
    return texture(t, clamp(uv, half_texel, 1.0 - half_texel));
}

float mask_at(vec2 uv) { return (u_has_mask == 0) ? 1.0 : texture(u_mask, uv).r; }

/// Blend an effect result back over the untouched source through the mask.
vec4 apply_mask(vec4 src, vec4 fx, vec2 uv) { return mix(src, fx, mask_at(uv)); }
"#;

/// Entry point appended after an effect body: sample, run `effect`, mask.
pub const MAIN: &str = r#"
void main() {
    vec2 uv = v_uv;
    vec4 src = texture(tex, uv);
    out_color = apply_mask(src, effect(src, uv), uv);
}
"#;

/// Body for each `EffectKind` that runs on the GPU. `None` = geometric or CPU-only.
///
/// A body is GLSL that defines exactly `vec4 effect(vec4 src, vec2 uv)` — extra helpers and uniforms
/// above it are fine, a `main()` is not: `fragment()` appends `MAIN`, which samples `tex`, calls
/// `effect` and mixes the result back through the mask, so masking is automatic. Parameters arrive as
/// `p0..p7` in `EffectKind::params()` order; pixel-sized ones must be multiplied by `u_scale`.
pub fn body(kind: crate::model::EffectKind) -> Option<&'static str> {
    use crate::model::EffectKind as K;
    Some(match kind {
        K::Blur => BLUR,
        K::Pixelate => PIXELATE,
        K::Tint => TINT,
        K::Color => COLOR,
        K::Vignette => VIGNETTE,
        K::Sharpen => SHARPEN,
        K::Invert => INVERT,
        K::Grayscale => GRAYSCALE,
        K::Flip => FLIP,
        K::Crop => CROP,
        K::ChromaKey => CHROMA_KEY,
        K::Curves => CURVES,
        K::Levels => LEVELS,
        K::HueShift => HUE_SHIFT,
        K::JpegCompress => JPEG_COMPRESS,
        K::MotionBlur => MOTION_BLUR,
        K::EdgeGlow => EDGE_GLOW,
        K::Threshold => THRESHOLD,
        K::BlobTrack => BLOB_TRACK,
        K::Vhs => VHS,
        K::RecDot => REC_DOT,
        // Wobble / Plane3d move the layer (see engine::effects::wobble / plane3d_matrix);
        // Shader is wrapped by `user_shader` instead.
        K::Wobble | K::Plane3d | K::Shader => return None,
    })
}

/// The complete fragment source for one effect, or None when it has no GPU body.
pub fn fragment(kind: EffectKind, user_src: &str) -> Option<String> {
    if kind == EffectKind::Shader {
        return Some(user_shader(user_src));
    }
    Some(format!("{PRELUDE}
{}
{MAIN}", body(kind)?))
}

/// Blend modes as a GLSL function (`vec3 blend(int mode, vec3 s, vec3 d)`) mirroring engine::blend.
/// The int is `BlendMode::ALL`'s index — see `blend_index` in engine/gpu.rs.
pub const BLEND: &str = r#"
float blend_1(int mode, float s, float d) {
    if (mode == 1) { return s * d; }                                    // Multiply
    if (mode == 2) { return 1.0 - (1.0 - s) * (1.0 - d); }              // Screen
    if (mode == 3) {                                                    // Overlay
        return (d <= 0.5) ? 2.0 * s * d : 1.0 - 2.0 * (1.0 - s) * (1.0 - d);
    }
    if (mode == 4) { return min(s, d); }                                // Darken
    if (mode == 5) { return max(s, d); }                                // Lighten
    if (mode == 6) { return min(s + d, 1.0); }                          // Add
    if (mode == 7) { return max(d - s, 0.0); }                          // Subtract
    if (mode == 8) { return abs(s - d); }                               // Difference
    if (mode == 9) {                                                    // SoftLight
        if (s <= 0.5) { return d - (1.0 - 2.0 * s) * d * (1.0 - d); }
        float g = (d <= 0.25) ? ((16.0 * d - 12.0) * d + 4.0) * d : sqrt(d);
        return d + (2.0 * s - 1.0) * (g - d);
    }
    if (mode == 10) {                                                   // HardLight
        return (s <= 0.5) ? 2.0 * s * d : 1.0 - 2.0 * (1.0 - s) * (1.0 - d);
    }
    if (mode == 11) {                                                   // ColorDodge
        if (d <= 0.0) { return 0.0; }
        if (s >= 1.0) { return 1.0; }
        return min(d / (1.0 - s), 1.0);
    }
    if (mode == 12) {                                                   // ColorBurn
        if (d >= 1.0) { return 1.0; }
        if (s <= 0.0) { return 0.0; }
        return 1.0 - min((1.0 - d) / s, 1.0);
    }
    return s;                                                           // Normal
}

vec3 blend(int mode, vec3 s, vec3 d) {
    if (mode == 0) { return s; }
    return vec3(blend_1(mode, s.r, d.r), blend_1(mode, s.g, d.g), blend_1(mode, s.b, d.b));
}
"#;

/// Mask rasteriser (rect / ellipse / polygon / path with feather, expand, invert, rotation).
/// Appended to `PRELUDE`; everything is in canvas pixels, relative to the layer centre.
/// Writes coverage to every channel so it reads as `.r` (a matte) or as alpha.
pub const MASK: &str = r#"
uniform int m_shape;
uniform vec2 m_center;
uniform vec2 m_radius;
uniform float m_rot;
uniform float m_feather;
uniform float m_expand;
uniform float m_opacity;
uniform int m_invert;
uniform int m_count;
// ponytail: 64 points is plenty for hand-drawn masks; a point texture would lift the cap.
uniform vec2 m_points[64];

float sd_box(vec2 p, vec2 b) {
    vec2 d = abs(p) - b;
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0);
}

// ponytail: cheap ellipse SDF (exact only on circles); good enough for a feathered edge.
float sd_ellipse(vec2 p, vec2 r) {
    vec2 rr = max(r, vec2(1e-4));
    return (length(p / rr) - 1.0) * min(rr.x, rr.y);
}

float sd_poly(vec2 p) {
    int n = min(m_count, 64);
    if (n < 3) { return 1e6; }
    vec2 w0 = p - m_points[0];
    float d = dot(w0, w0);
    float s = 1.0;
    for (int i = 0, j = n - 1; i < n; j = i, i++) {
        vec2 e = m_points[j] - m_points[i];
        vec2 w = p - m_points[i];
        vec2 b = w - e * clamp(dot(w, e) / max(dot(e, e), 1e-9), 0.0, 1.0);
        d = min(d, dot(b, b));
        bvec3 c = bvec3(p.y >= m_points[i].y, p.y < m_points[j].y, e.x * w.y > e.y * w.x);
        if (all(c) || all(not(c))) { s = -s; }
    }
    return s * sqrt(d);
}

void main() {
    vec2 q = v_uv * u_res - m_center;
    float a = radians(m_rot);
    float ca = cos(a);
    float sa = sin(a);
    q = vec2(q.x * ca + q.y * sa, -q.x * sa + q.y * ca);
    float d;
    if (m_shape == 0) {
        d = sd_box(q, m_radius);
    } else if (m_shape == 1) {
        d = sd_ellipse(q, m_radius);
    } else {
        d = sd_poly(q);
    }
    d -= m_expand;
    float cov = clamp(0.5 - d / max(m_feather, 1.0), 0.0, 1.0);
    if (m_invert != 0) { cov = 1.0 - cov; }
    out_color = vec4(cov * m_opacity);
}
"#;

/// Composite/transform shader: samples a layer through an inverse 3x3 (perspective-capable) matrix with
/// the chosen `Scaler` (0 = nearest, 1 = bilinear, 2 = bicubic Catmull-Rom) and blends it onto the canvas.
/// Appended to `PRELUDE` + `BLEND`. `u_inv` maps canvas pixels (y down) to layer uv.
pub const COMPOSITE: &str = r#"
uniform sampler2D u_dst;
uniform vec2 u_layer;
uniform mat3 u_inv;
uniform int u_scaler;
uniform int u_mode;
uniform float u_opacity;

vec4 tap(vec2 ip) {
    return texelFetch(tex, ivec2(clamp(ip, vec2(0.0), u_layer - 1.0)), 0);
}

vec4 catmull_w(float t) {
    float t2 = t * t;
    float t3 = t2 * t;
    return 0.5 * vec4(-t3 + 2.0 * t2 - t, 3.0 * t3 - 5.0 * t2 + 2.0, -3.0 * t3 + 4.0 * t2 + t, t3 - t2);
}

// Alpha-weighted accumulation (matches the CPU compositor: transparent texels must not darken edges).
vec4 sample_layer(vec2 uv) {
    vec2 sp = uv * u_layer - 0.5;
    if (u_scaler == 0) { return tap(floor(sp + 0.5)); }
    vec4 acc = vec4(0.0);
    if (u_scaler == 1) {
        vec2 f = floor(sp);
        vec2 t = sp - f;
        for (int j = 0; j < 2; j++) {
            for (int i = 0; i < 2; i++) {
                float w = (i == 0 ? 1.0 - t.x : t.x) * (j == 0 ? 1.0 - t.y : t.y);
                vec4 c = tap(f + vec2(float(i), float(j)));
                acc += vec4(c.rgb * c.a * w, c.a * w);
            }
        }
    } else {
        vec2 f = floor(sp);
        vec4 wx = catmull_w(sp.x - f.x);
        vec4 wy = catmull_w(sp.y - f.y);
        for (int j = 0; j < 4; j++) {
            for (int i = 0; i < 4; i++) {
                float w = wx[i] * wy[j];
                vec4 c = tap(f + vec2(float(i - 1), float(j - 1)));
                acc += vec4(c.rgb * c.a * w, c.a * w);
            }
        }
    }
    if (acc.a <= 0.0) { return vec4(0.0); }
    return vec4(clamp(acc.rgb / acc.a, 0.0, 1.0), clamp(acc.a, 0.0, 1.0));
}

void main() {
    vec3 h = u_inv * vec3(v_uv * u_res, 1.0);
    float z = (abs(h.z) < 1e-9) ? 1e-9 : h.z;
    vec2 uv = h.xy / z;
    // derivatives must be taken outside any branch
    vec2 e = max(fwidth(uv), vec2(1e-8));
    vec2 dd = min(uv, 1.0 - uv);
    float cov = clamp(min(dd.x / e.x, dd.y / e.y) + 0.5, 0.0, 1.0);
    vec4 d = texture(u_dst, v_uv);
    if (cov <= 0.0 || h.z <= 0.0) { out_color = d; return; }
    vec4 s = sample_layer(uv);
    float a = s.a * u_opacity * cov * mask_at(v_uv);
    if (a <= 0.0) { out_color = d; return; }
    float ao = a + d.a * (1.0 - a);
    if (ao <= 0.0) { out_color = vec4(0.0); return; }
    vec3 b = blend(u_mode, s.rgb, d.rgb);
    vec3 co = ((1.0 - d.a) * a * s.rgb + (1.0 - a) * d.a * d.rgb + a * d.a * b) / ao;
    out_color = vec4(co, ao);
}
"#;

/// Wrap a user shader body (`EffectKind::Shader`) so it sees the same prelude and knobs `u1..u8`.
/// The body must define `vec4 effect(vec4 src, vec2 uv)` (see `model::DEFAULT_SHADER`).
pub fn user_shader(src: &str) -> String {
    let mut s = String::with_capacity(PRELUDE.len() + src.len() + 512);
    s.push_str(PRELUDE);
    for i in 0..8 {
        s.push_str(&format!("#define u{} p{}\n", i + 1, i));
    }
    s.push('\n');
    s.push_str(src);
    s.push_str(MAIN);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn balanced(src: &str) -> bool {
        let mut n = 0i32;
        for c in src.chars() {
            match c {
                '{' => n += 1,
                '}' => {
                    n -= 1;
                    if n < 0 {
                        return false;
                    }
                }
                _ => {}
            }
        }
        n == 0
    }

    /// Identifiers the source declares: uniforms, `#define`s and function names.
    fn declared(src: &str) -> Vec<String> {
        let mut out = Vec::new();
        for line in src.lines() {
            let l = line.trim();
            if let Some(rest) = l.strip_prefix("uniform ") {
                // "uniform vec2 m_points[64];" -> m_points
                if let Some(name) = rest.split_whitespace().nth(1) {
                    out.push(name.trim_end_matches(';').split('[').next().unwrap_or(name).to_string());
                }
            } else if let Some(rest) = l.strip_prefix("#define ") {
                if let Some(name) = rest.split_whitespace().next() {
                    out.push(name.to_string());
                }
            } else if let Some(i) = l.find('(') {
                // "vec4 effect(vec4 src, vec2 uv) {" -> effect
                let head = &l[..i];
                let mut w = head.split_whitespace();
                if let (Some(_ty), Some(name)) = (w.next(), w.next()) {
                    if w.next().is_none() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        out.push(name.to_string());
                    }
                }
            }
            if let Some(rest) = l.strip_prefix("in ").or_else(|| l.strip_prefix("out ")) {
                if let Some(name) = rest.split_whitespace().nth(1) {
                    out.push(name.trim_end_matches(';').to_string());
                }
            }
        }
        out
    }

    /// Every `u_*` / `p0..p7` / `m_*` identifier used by `src` must be declared in it.
    fn undeclared_uniforms(src: &str) -> Vec<String> {
        let decl = declared(src);
        let mut bad = Vec::new();
        let mut word = String::new();
        for c in src.chars().chain(std::iter::once(' ')) {
            if c.is_alphanumeric() || c == '_' {
                word.push(c);
                continue;
            }
            let w = std::mem::take(&mut word);
            let interesting = w.starts_with("u_")
                || w.starts_with("m_")
                || (w.len() == 2 && w.starts_with('p') && w.as_bytes()[1].is_ascii_digit());
            if interesting && !decl.contains(&w) && !bad.contains(&w) {
                bad.push(w);
            }
        }
        bad
    }

    const SAMPLE_BODY: &str = "vec4 effect(vec4 src, vec2 uv) {\n    vec4 c = sample_clamped(tex, uv);\n    \
         return vec4(mix(c.rgb, vec3(luma(c.rgb)), p0) * u_scale, c.a);\n}\n";

    #[test]
    fn prelude_plus_body_is_well_formed() {
        let src = format!("{PRELUDE}\n{SAMPLE_BODY}\n{MAIN}");
        assert!(balanced(&src), "unbalanced braces");
        assert!(src.starts_with("#version 330 core"));
        assert_eq!(undeclared_uniforms(&src), Vec::<String>::new());
        for name in ["tex", "u_res", "u_time", "u_scale", "u_mask", "u_has_mask"] {
            assert!(src.contains(&format!("uniform {name};")) || src.contains(&format!(" {name};")), "{name}");
        }
        for i in 0..8 {
            assert!(src.contains(&format!("uniform float p{i};")), "p{i}");
        }
        // helpers the bodies rely on
        for f in ["luma(", "rgb2hsl(", "hsl2rgb(", "hash12(", "sample_clamped(", "apply_mask("] {
            assert!(src.contains(f), "missing helper {f}");
        }
    }

    #[test]
    fn shared_programs_are_well_formed() {
        for (name, src) in [
            ("vert", VERT.to_string()),
            ("composite", format!("{PRELUDE}{BLEND}{COMPOSITE}")),
            ("mask", format!("{PRELUDE}{MASK}")),
        ] {
            assert!(balanced(&src), "{name}: unbalanced braces");
            assert!(src.contains("void main()"), "{name}: no entry point");
            assert_eq!(undeclared_uniforms(&src), Vec::<String>::new(), "{name}");
        }
    }

    #[test]
    fn user_shader_wraps_body_with_knobs_and_entry_point() {
        let s = user_shader(crate::model::DEFAULT_SHADER);
        assert!(s.starts_with("#version 330 core"));
        assert!(balanced(&s));
        for i in 1..=8 {
            assert!(s.contains(&format!("#define u{} p{}\n", i, i - 1)), "knob u{i}");
        }
        assert!(s.contains("vec4 effect(vec4 src, vec2 uv)"), "entry point missing");
        assert!(s.contains("out_color = apply_mask("), "main must call effect through the mask");
        assert_eq!(undeclared_uniforms(&s), Vec::<String>::new());
        // the user body's own uses resolve through the #defines
        assert!(s.contains("u1") && s.contains("u_time"));
    }

    #[test]
    fn fragment_uses_the_user_source_for_shader_kind() {
        let f = fragment(EffectKind::Shader, "vec4 effect(vec4 s, vec2 uv) { return s * p3; }").unwrap();
        assert!(f.contains("return s * p3;"));
        assert!(f.ends_with(MAIN));
        // geometric kinds never compile a program
        assert!(fragment(EffectKind::Wobble, "").is_none());
        assert!(fragment(EffectKind::Plane3d, "").is_none());
    }
}

// ===========================================================================================
// Per-effect fragment bodies (owned by the gpu-effects group). Everything above this line is
// the shared GPU core.
//
// Rules every body below follows, so `PRELUDE` stays the only shared surface:
//   * texture coordinates come from `gl_FragCoord.xy / u_res`, never from a varying, and
//     **v = 0 is the top of the layer** (frames are top-down; gpu.rs must render the full-screen
//     pass so gl_FragCoord.y grows downward). Only Crop/Flip/RecDot/VHS care about the direction.
//   * helper functions are prefixed `fx_` and defined inside the body that needs them, so a body
//     never depends on a helper name in `PRELUDE`.
//   * the mask is applied explicitly at the end: `mix(src, res, coverage)`.
//   * pixel-sized parameters are multiplied by `u_scale` (preview == export).
//   * kinds with more than 8 parameters (Curves) declare the extra `pN` uniforms themselves;
//     `PRELUDE` must declare exactly p0..p7 and gpu.rs must set one uniform per `params()` entry.
//   * two kinds need extra uniforms gpu.rs has to bind (documented at their constant):
//     MotionBlur -> u_prev / u_next / u_frames / u_motion, BlobTrack -> u_blob.
// ===========================================================================================

/// 24-tap Vogel disc, Gaussian-weighted; radius `p0` project px.
const BLUR: &str = r#"
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    vec4 res = src;
    float r = p0 * u_scale;
    if (r >= 0.5) {
        // ponytail: one-pass Vogel disc instead of a separable Gaussian - if quality matters,
        // add a direction knob and let gpu.rs run this program twice (H then V).
        vec4 acc = vec4(0.0);
        float wsum = 0.0;
        for (int i = 0; i < 24; i++) {
            float fi = float(i) + 0.5;
            float rad = sqrt(fi / 24.0) * r;
            float ang = fi * 2.39996323;
            float w = exp(-2.0 * rad * rad / (r * r));
            acc += texture(tex, uv + vec2(cos(ang), sin(ang)) * rad / u_res) * w;
            wsum += w;
        }
        res = acc / wsum;
    }
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// Snap to a `p0`-project-px block grid and sample its centre.
const PIXELATE: &str = r#"
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    float b = max(p0, 1.0) * max(u_scale, 0.01);
    vec4 res = src;
    if (b > 1.0) {
        vec2 q = (floor(gl_FragCoord.xy / b) + 0.5) * b;
        res = texture(tex, q / u_res);
    }
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// Mix towards (p0,p1,p2) 0..255 by p3.
const TINT: &str = r#"
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    vec4 res = vec4(mix(src.rgb, vec3(p0, p1, p2) / 255.0, clamp(p3, 0.0, 1.0)), src.a);
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// Saturation + hue matrix (same feColorMatrix pair as the CPU path), then brightness/contrast/gamma.
const COLOR: &str = r#"
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    vec3 c = src.rgb;
    float l = dot(c, vec3(0.2126, 0.7152, 0.0722));
    c = mix(vec3(l), c, p2);
    float a = radians(p3), cs = cos(a), sn = sin(a);
    mat3 hm = mat3(
        0.213 + cs * 0.787 - sn * 0.213, 0.213 - cs * 0.213 + sn * 0.143, 0.213 - cs * 0.213 - sn * 0.787,
        0.715 - cs * 0.715 - sn * 0.715, 0.715 + cs * 0.285 + sn * 0.140, 0.715 - cs * 0.715 + sn * 0.715,
        0.072 - cs * 0.072 + sn * 0.928, 0.072 - cs * 0.072 - sn * 0.283, 0.072 + cs * 0.928 + sn * 0.072);
    c = clamp((clamp(hm * c, 0.0, 1.0) - 0.5) * p1 + 0.5 + p0, 0.0, 1.0);
    c = pow(c, vec3(1.0 / max(p4, 0.001)));
    vec4 res = vec4(c, src.a);
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// Darken outside `p0` (fraction of the half diagonal) with `p1` softness by `p2`.
const VIGNETTE: &str = r#"
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    float d = length((uv - 0.5) * u_res) / max(length(u_res * 0.5), 1.0);
    float fall = smoothstep(0.0, 1.0, (d - p0) / max(p1, 0.001));
    float k = 1.0 - clamp(p2, 0.0, 1.0) * fall;
    vec4 res = vec4(src.rgb * k, src.a);
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// Unsharp mask against a 3x3 box at `p1` project px spacing.
const SHARPEN: &str = r#"
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    vec2 o = vec2(max(p1, 0.5) * u_scale) / u_res;
    vec3 b = texture(tex, uv + vec2(-o.x, -o.y)).rgb + texture(tex, uv + vec2(0.0, -o.y)).rgb
           + texture(tex, uv + vec2(o.x, -o.y)).rgb + texture(tex, uv + vec2(-o.x, 0.0)).rgb
           + src.rgb + texture(tex, uv + vec2(o.x, 0.0)).rgb
           + texture(tex, uv + vec2(-o.x, o.y)).rgb + texture(tex, uv + vec2(0.0, o.y)).rgb
           + texture(tex, uv + vec2(o.x, o.y)).rgb;
    vec4 res = vec4(clamp(src.rgb + max(p0, 0.0) * (src.rgb - b / 9.0), 0.0, 1.0), src.a);
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

const INVERT: &str = r#"
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    vec4 res = vec4(mix(src.rgb, 1.0 - src.rgb, clamp(p0, 0.0, 1.0)), src.a);
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

const GRAYSCALE: &str = r#"
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    float l = dot(src.rgb, vec3(0.2126, 0.7152, 0.0722));
    vec4 res = vec4(mix(src.rgb, vec3(l), clamp(p0, 0.0, 1.0)), src.a);
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// p0 = horizontal, p1 = vertical (>= 0.5 = on).
const FLIP: &str = r#"
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    vec2 f = vec2(p0 >= 0.5 ? 1.0 - uv.x : uv.x, p1 >= 0.5 ? 1.0 - uv.y : uv.y);
    vec4 res = texture(tex, f);
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// Left/Right/Top/Bottom fractions cut away (alpha 0) with a `p4` feather.
const CROP: &str = r#"
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    vec2 q = uv * u_res;
    float x0 = clamp(p0, 0.0, 0.5) * u_res.x;
    float x1 = u_res.x - clamp(p1, 0.0, 0.5) * u_res.x;
    float y0 = clamp(p2, 0.0, 0.5) * u_res.y;
    float y1 = u_res.y - clamp(p3, 0.0, 0.5) * u_res.y;
    float d = min(min(q.x - x0, x1 - q.x), min(q.y - y0, y1 - q.y));
    float fe = clamp(p4, 0.0, 0.5) * min(u_res.x, u_res.y);
    float a = d <= 0.0 ? 0.0 : (fe > 0.0 ? min(d / fe, 1.0) : 1.0);
    vec4 res = vec4(src.rgb, src.a * a);
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// Chroma key: distance in the Cb/Cr plane to the key colour, with spill removal, matte erode/dilate
/// (p7 project px, + shrinks) and a show-mask switch (p5).
const CHROMA_KEY: &str = r#"
vec3 fx_ycc(vec3 c) {
    return vec3(dot(c, vec3(0.299, 0.587, 0.114)),
                dot(c, vec3(-0.168736, -0.331264, 0.5)) + 0.5,
                dot(c, vec3(0.5, -0.418688, -0.081312)) + 0.5);
}
vec3 fx_rgb(vec3 y) {
    float cb = y.y - 0.5;
    float cr = y.z - 0.5;
    return vec3(y.x + 1.402 * cr, y.x - 0.344136 * cb - 0.714136 * cr, y.x + 1.772 * cb);
}
float fx_alpha(vec2 uv) {
    vec2 c = fx_ycc(texture(tex, uv).rgb).yz;
    vec2 k = fx_ycc(clamp(vec3(p0, p1, p2) / 255.0, 0.0, 1.0)).yz;
    float sim = clamp(p3, 0.0, 1.0) * 0.5;
    return smoothstep(sim, sim + max(clamp(p4, 0.0, 1.0) * 0.5, 0.0005), distance(c, k));
}
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    float a = fx_alpha(uv);
    float sh = p7 * u_scale;
    if (abs(sh) > 0.01) {
        vec2 step_ = vec2(abs(sh)) / u_res;
        for (int i = 0; i < 8; i++) {
            float ang = float(i) * 0.78539816;
            float n = fx_alpha(uv + vec2(cos(ang), sin(ang)) * step_);
            a = sh > 0.0 ? min(a, n) : max(a, n);
        }
    }
    vec3 rgb = src.rgb;
    float spill = clamp(p6, 0.0, 1.0);
    if (spill > 0.0) {
        vec3 y = fx_ycc(rgb);
        vec2 kc = fx_ycc(clamp(vec3(p0, p1, p2) / 255.0, 0.0, 1.0)).yz - 0.5;
        if (length(kc) > 0.001) {
            vec2 dir = normalize(kc);
            vec2 cc = y.yz - 0.5;
            float proj = dot(cc, dir);
            if (proj > 0.0) {
                cc -= dir * proj * spill;
                rgb = clamp(fx_rgb(vec3(y.x, cc + 0.5)), 0.0, 1.0);
            }
        }
    }
    vec4 res = vec4(rgb, src.a * a);
    if (p5 >= 0.5) {
        res = vec4(vec3(a), 1.0);
    }
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// Twelve knots (master, R, G, B) driving a monotone cubic through (0,0) (.25,a) (.5,b) (.75,c) (1,1).
/// Fritsch-Carlson tangents keep the spline monotone, so a curve never folds back on itself.
/// Declares p8..p11 itself: `PRELUDE` only carries p0..p7.
const CURVES: &str = r#"
uniform float p8;
uniform float p9;
uniform float p10;
uniform float p11;
float fx_curve(float x, float a, float b, float c) {
    float y[5];
    y[0] = 0.0; y[1] = a; y[2] = b; y[3] = c; y[4] = 1.0;
    float d[4];
    for (int i = 0; i < 4; i++) {
        d[i] = (y[i + 1] - y[i]) * 4.0;
    }
    float m[5];
    m[0] = d[0];
    m[4] = d[3];
    for (int i = 1; i < 4; i++) {
        m[i] = (d[i - 1] * d[i] <= 0.0) ? 0.0 : (d[i - 1] + d[i]) * 0.5;
    }
    for (int i = 0; i < 4; i++) {
        if (abs(d[i]) < 1e-6) {
            m[i] = 0.0;
            m[i + 1] = 0.0;
        } else {
            float ai = m[i] / d[i];
            float bi = m[i + 1] / d[i];
            float s = ai * ai + bi * bi;
            if (s > 9.0) {
                float k = 3.0 / sqrt(s);
                m[i] = k * ai * d[i];
                m[i + 1] = k * bi * d[i];
            }
        }
    }
    float xc = clamp(x, 0.0, 1.0);
    int i = int(min(floor(xc * 4.0), 3.0));
    float t = xc * 4.0 - float(i);
    float t2 = t * t;
    float t3 = t2 * t;
    return (2.0 * t3 - 3.0 * t2 + 1.0) * y[i] + (t3 - 2.0 * t2 + t) * 0.25 * m[i]
         + (-2.0 * t3 + 3.0 * t2) * y[i + 1] + (t3 - t2) * 0.25 * m[i + 1];
}
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    vec3 c = vec3(fx_curve(src.r, p3, p4, p5), fx_curve(src.g, p6, p7, p8), fx_curve(src.b, p9, p10, p11));
    c = vec3(fx_curve(c.r, p0, p1, p2), fx_curve(c.g, p0, p1, p2), fx_curve(c.b, p0, p1, p2));
    vec4 res = vec4(clamp(c, 0.0, 1.0), src.a);
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// In black/white, gamma, out black/white (same convention as Color's gamma: v^(1/g)).
const LEVELS: &str = r#"
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    vec3 c = clamp((src.rgb - p0) / max(p1 - p0, 0.0001), 0.0, 1.0);
    c = pow(c, vec3(1.0 / max(p2, 0.01)));
    c = clamp(p3 + c * (p4 - p3), 0.0, 1.0);
    vec4 res = vec4(c, src.a);
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// Hue rotate (p0 degrees), saturation multiply (p1) and lightness offset (p2) in HSL.
const HUE_SHIFT: &str = r#"
vec3 fx_rgb2hsl(vec3 c) {
    float mx = max(c.r, max(c.g, c.b));
    float mn = min(c.r, min(c.g, c.b));
    float l = (mx + mn) * 0.5;
    float h = 0.0;
    float s = 0.0;
    float d = mx - mn;
    if (d > 1e-5) {
        s = d / max(1.0 - abs(2.0 * l - 1.0), 1e-5);
        if (mx == c.r) {
            h = mod((c.g - c.b) / d, 6.0);
        } else if (mx == c.g) {
            h = (c.b - c.r) / d + 2.0;
        } else {
            h = (c.r - c.g) / d + 4.0;
        }
        h /= 6.0;
    }
    return vec3(h, s, l);
}
float fx_h2c(float a, float b, float t) {
    float u = fract(t);
    if (u < 1.0 / 6.0) return a + (b - a) * 6.0 * u;
    if (u < 0.5) return b;
    if (u < 2.0 / 3.0) return a + (b - a) * (2.0 / 3.0 - u) * 6.0;
    return a;
}
vec3 fx_hsl2rgb(vec3 hsl) {
    float l = clamp(hsl.z, 0.0, 1.0);
    float s = clamp(hsl.y, 0.0, 1.0);
    if (s <= 0.0) return vec3(l);
    float q = l < 0.5 ? l * (1.0 + s) : l + s - l * s;
    float a = 2.0 * l - q;
    return vec3(fx_h2c(a, q, hsl.x + 1.0 / 3.0), fx_h2c(a, q, hsl.x), fx_h2c(a, q, hsl.x - 1.0 / 3.0));
}
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    vec3 hsl = fx_rgb2hsl(clamp(src.rgb, 0.0, 1.0));
    hsl.x = fract(hsl.x + p0 / 360.0);
    hsl.y = clamp(hsl.y * max(p1, 0.0), 0.0, 1.0);
    hsl.z = clamp(hsl.z + p2, 0.0, 1.0);
    vec4 res = vec4(clamp(fx_hsl2rgb(hsl), 0.0, 1.0), src.a);
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// JPEG-ish: the block DC (4 taps) is quantised coarsely, the AC residual more coarsely still, and the
/// chroma optionally read from a 2x2 grid (4:2:0). This is an *approximation* - there is no DCT here,
/// but the visible artefacts (blocking, colour smear, banding) track `Quality` the same way.
const JPEG_COMPRESS: &str = r#"
vec3 fx_ycc(vec3 c) {
    return vec3(dot(c, vec3(0.299, 0.587, 0.114)),
                dot(c, vec3(-0.168736, -0.331264, 0.5)) + 0.5,
                dot(c, vec3(0.5, -0.418688, -0.081312)) + 0.5);
}
vec3 fx_rgb(vec3 y) {
    float cb = y.y - 0.5;
    float cr = y.z - 0.5;
    return vec3(y.x + 1.402 * cr, y.x - 0.344136 * cb - 0.714136 * cr, y.x + 1.772 * cb);
}
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    float sc = max(u_scale, 0.05);
    float b = max(p1, 2.0) * sc;
    vec2 org = floor(gl_FragCoord.xy / b) * b;
    vec3 dc = texture(tex, (org + b * vec2(0.25, 0.25)) / u_res).rgb
            + texture(tex, (org + b * vec2(0.75, 0.25)) / u_res).rgb
            + texture(tex, (org + b * vec2(0.25, 0.75)) / u_res).rgb
            + texture(tex, (org + b * vec2(0.75, 0.75)) / u_res).rgb;
    dc *= 0.25;
    float q = clamp(p0, 1.0, 100.0);
    float qs = q < 50.0 ? 50.0 / q : 2.0 - q / 50.0;
    float step_ = clamp(qs * 0.05, 0.004, 0.6);
    vec3 ydc = fx_ycc(dc);
    vec3 ypx = fx_ycc(src.rgb);
    float dcs = step_ * 0.5;
    float luma = floor(ydc.x / dcs + 0.5) * dcs + floor((ypx.x - ydc.x) / step_ + 0.5) * step_;
    vec2 ch = ypx.yz;
    if (p2 >= 0.5) {
        vec2 sub = (floor(gl_FragCoord.xy / (2.0 * sc)) + 0.5) * (2.0 * sc);
        ch = fx_ycc(texture(tex, sub / u_res).rgb).yz;
    }
    float cstep = step_ * 2.0;
    ch = floor((ch - 0.5) / cstep + 0.5) * cstep + 0.5;
    vec4 res = vec4(clamp(fx_rgb(vec3(luma, ch)), 0.0, 1.0), src.a);
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// Shutter-window average of the frames gpu.rs supplies.
///
/// Extra uniforms gpu.rs must bind (this kind reports `EffectKind::needs_motion()`):
///   sampler2D u_prev / u_next  the layer one frame before / after (bind `tex` when missing)
///   float     u_frames         how many of those are real: 0, 1 (prev only) or 2
///   vec2      u_motion         the layer's motion in layer pixels per frame, for the 0-frame fallback
const MOTION_BLUR: &str = r#"
uniform sampler2D u_prev;
uniform sampler2D u_next;
uniform float u_frames;
uniform vec2 u_motion;
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    float sh = clamp(p0, 0.0, 360.0) / 360.0;
    int n = int(clamp(p1, 2.0, 32.0));
    vec4 acc = vec4(0.0);
    vec4 prev = src;
    vec4 next = src;
    if (u_frames > 0.5) {
        prev = texture(u_prev, uv);
        next = u_frames > 1.5 ? texture(u_next, uv) : prev;
        for (int i = 0; i < 32; i++) {
            if (i >= n) break;
            float o = ((float(i) + 0.5) / float(n) - 0.5) * sh;
            acc += o < 0.0 ? mix(src, prev, clamp(-o, 0.0, 1.0)) : mix(src, next, clamp(o, 0.0, 1.0));
        }
    } else {
        vec2 d = u_motion / u_res * sh;
        for (int i = 0; i < 32; i++) {
            if (i >= n) break;
            acc += texture(tex, uv + d * ((float(i) + 0.5) / float(n) - 0.5));
        }
    }
    vec4 res = acc / float(n);
    if (p2 >= 0.5 && u_frames > 0.5) {
        float mv = length(prev.rgb - src.rgb) + length(next.rgb - src.rgb);
        res = mix(src, res, clamp(mv * 4.0, 0.0, 1.0));
    }
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// Sobel on luma with the tap spacing set by `Width` (p1) - a wider kernel is the cheap stand-in for
/// dilating the edge - then a coloured glow, optionally over the source.
const EDGE_GLOW: &str = r#"
float fx_l(vec2 uv) {
    return dot(texture(tex, uv).rgb, vec3(0.2126, 0.7152, 0.0722));
}
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    vec2 o = vec2(max(p1, 0.5) * max(u_scale, 0.05)) / u_res;
    float tl = fx_l(uv + vec2(-o.x, -o.y));
    float tc = fx_l(uv + vec2(0.0, -o.y));
    float tr = fx_l(uv + vec2(o.x, -o.y));
    float ml = fx_l(uv + vec2(-o.x, 0.0));
    float mr = fx_l(uv + vec2(o.x, 0.0));
    float bl = fx_l(uv + vec2(-o.x, o.y));
    float bc = fx_l(uv + vec2(0.0, o.y));
    float br = fx_l(uv + vec2(o.x, o.y));
    float gx = (tr + 2.0 * mr + br) - (tl + 2.0 * ml + bl);
    float gy = (bl + 2.0 * bc + br) - (tl + 2.0 * tc + tr);
    float e = length(vec2(gx, gy)) * 0.25;
    float edge = smoothstep(p0, p0 + 0.15, e);
    vec3 glow = vec3(p3, p4, p5) / 255.0 * max(p2, 0.0) * edge;
    vec4 res;
    if (p6 >= 0.5) {
        res = vec4(clamp(src.rgb + glow, 0.0, 1.0), src.a);
    } else {
        res = vec4(clamp(glow, 0.0, 1.0), edge * src.a);
    }
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// Hard/soft threshold on luma, or per channel when p2 >= 0.5.
const THRESHOLD: &str = r#"
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    float sm = max(p1, 0.0001) * 0.5;
    vec3 c;
    if (p2 >= 0.5) {
        c = smoothstep(vec3(p0 - sm), vec3(p0 + sm), src.rgb);
    } else {
        c = vec3(smoothstep(p0 - sm, p0 + sm, dot(src.rgb, vec3(0.2126, 0.7152, 0.0722))));
    }
    vec4 res = vec4(c, src.a);
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// Overlay for the colour tracker: tints everything within `Tolerance` of the target and draws a
/// crosshair at the centroid.
///
/// Extra uniform gpu.rs must bind: `vec3 u_blob` = (cx, cy, area) from `engine::effects::track`,
/// cx/cy in 0..1 layer coordinates and area <= 0 when nothing was found (then only the mask shows).
const BLOB_TRACK: &str = r#"
uniform vec3 u_blob;
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    vec4 res = src;
    if (p4 >= 0.5) {
        float d = distance(src.rgb, clamp(vec3(p0, p1, p2) / 255.0, 0.0, 1.0));
        float tol = clamp(p3, 0.0, 1.0) * 1.2;
        float hit = 1.0 - smoothstep(tol, tol + 0.05, d);
        res = vec4(mix(src.rgb, vec3(1.0, 0.25, 0.25), hit * 0.5), src.a);
        if (u_blob.z > 0.0) {
            vec2 dxy = abs(uv - u_blob.xy) * u_res;
            float lw = max(1.0, u_scale);
            float arm = 16.0 * max(u_scale, 0.25);
            float cross_ = ((dxy.y <= lw && dxy.x <= arm) || (dxy.x <= lw && dxy.y <= arm)) ? 1.0 : 0.0;
            res = vec4(mix(res.rgb, vec3(1.0, 1.0, 0.2), cross_), max(res.a, cross_));
        }
    }
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// One-pass VHS: per-line tracking jitter, chroma bleed to the right of an edge, luma noise,
/// scanlines, sharpening ringing, a head-switching band at the bottom and tape-wear dropouts.
/// Cheap approximations of what ntsc-rs models properly - no line-rate filtering, no real comb filter.
const VHS: &str = r#"
float fx_hash(vec2 p) {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}
vec3 fx_ycc(vec3 c) {
    return vec3(dot(c, vec3(0.299, 0.587, 0.114)),
                dot(c, vec3(-0.168736, -0.331264, 0.5)) + 0.5,
                dot(c, vec3(0.5, -0.418688, -0.081312)) + 0.5);
}
vec3 fx_rgb(vec3 y) {
    float cb = y.y - 0.5;
    float cr = y.z - 0.5;
    return vec3(y.x + 1.402 * cr, y.x - 0.344136 * cb - 0.714136 * cr, y.x + 1.772 * cb);
}
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    float full = p6 >= 0.5 ? 0.0 : 1.0;   // "colour bleed only" keeps just the chroma stage
    float noise = clamp(p0, 0.0, 1.0) * full;
    float bleed = clamp(p1, 0.0, 1.0);
    float scan = clamp(p2, 0.0, 1.0) * full;
    float jit = clamp(p3, 0.0, 1.0) * full;
    float head = clamp(p4, 0.0, 1.0) * full;
    float ring = clamp(p5, 0.0, 1.0);
    float wear = clamp(p7, 0.0, 1.0) * full;
    float sc = max(u_scale, 0.1);
    float line = floor(gl_FragCoord.y / sc);
    float t = u_time;
    float band = 1.0 - smoothstep(0.0, max(0.05 * head, 0.001), 1.0 - uv.y);
    band *= step(0.001, head);
    float j = (fx_hash(vec2(line, floor(t * 30.0))) - 0.5) * jit * 0.02
            + sin(line * 0.31 + t * 6.0) * jit * 0.002
            + band * head * 0.08 * (fx_hash(vec2(line, floor(t * 12.0))) * 2.0 - 1.0);
    vec2 suv = clamp(vec2(uv.x + j, uv.y), vec2(0.0), vec2(1.0));
    vec3 base = texture(tex, suv).rgb;
    float bpx = bleed * 6.0 * sc / max(u_res.x, 1.0);
    vec3 b1 = texture(tex, clamp(suv - vec2(bpx, 0.0), 0.0, 1.0)).rgb;
    vec3 b2 = texture(tex, clamp(suv - vec2(bpx * 2.0, 0.0), 0.0, 1.0)).rgb;
    vec3 y = fx_ycc(base);
    y.yz = mix(y.yz, fx_ycc((b1 + b2) * 0.5).yz, bleed);
    y.x += (y.x - fx_ycc(b1).x) * ring * 1.5;
    y.x *= 1.0 - scan * 0.5 * (0.5 + 0.5 * cos(gl_FragCoord.y * 3.14159265 / sc));
    y.x += (fx_hash(gl_FragCoord.xy + vec2(t * 97.0, t * 31.0)) - 0.5) * noise * 0.25;
    if (wear > 0.0 && fx_hash(vec2(floor(line * 0.5), floor(t * 6.0))) > 1.0 - wear * 0.06) {
        float drop = fx_hash(vec2(floor(uv.x * 40.0), line));
        y.x = mix(y.x, 0.75 + 0.25 * drop, 0.7);
        y.yz = mix(y.yz, vec2(0.5), 0.8);
    }
    y.x = mix(y.x, 0.45 + 0.55 * fx_hash(gl_FragCoord.xy + vec2(t)), band * 0.8);
    y.yz = mix(y.yz, vec2(0.5), band);
    vec4 res = vec4(clamp(fx_rgb(y), 0.0, 1.0), src.a);
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

/// Blinking record dot in a corner (p2: 0 = top-left, 1 = top-right, 2 = bottom-left, 3 = bottom-right)
/// plus an optional HH:MM:SS timecode drawn as seven-segment quads from the clip-local time.
const REC_DOT: &str = r#"
float fx_rect(vec2 q, vec2 c, vec2 h) {
    vec2 d = abs(q - c) - h;
    return max(d.x, d.y) < 0.0 ? 1.0 : 0.0;
}
float fx_digit(vec2 q, int d) {
    if (q.x < 0.0 || q.x > 1.0 || q.y < 0.0 || q.y > 1.0) return 0.0;
    int seg[10] = int[10](63, 6, 91, 79, 102, 109, 125, 7, 127, 111);
    int m = seg[clamp(d, 0, 9)];
    float t = 0.11;
    float o = 0.0;
    if ((m & 1) != 0) o = max(o, fx_rect(q, vec2(0.5, t), vec2(0.5 - t, t)));
    if ((m & 2) != 0) o = max(o, fx_rect(q, vec2(1.0 - t, 0.25), vec2(t, 0.25 - t)));
    if ((m & 4) != 0) o = max(o, fx_rect(q, vec2(1.0 - t, 0.75), vec2(t, 0.25 - t)));
    if ((m & 8) != 0) o = max(o, fx_rect(q, vec2(0.5, 1.0 - t), vec2(0.5 - t, t)));
    if ((m & 16) != 0) o = max(o, fx_rect(q, vec2(t, 0.75), vec2(t, 0.25 - t)));
    if ((m & 32) != 0) o = max(o, fx_rect(q, vec2(t, 0.25), vec2(t, 0.25 - t)));
    if ((m & 64) != 0) o = max(o, fx_rect(q, vec2(0.5, 0.5), vec2(0.5 - t, t)));
    return o;
}
void main() {
    vec2 uv = gl_FragCoord.xy / u_res;
    vec4 src = texture(tex, uv);
    vec2 q = gl_FragCoord.xy;
    float sc = max(u_scale, 0.05);
    float sz = max(p0, 2.0) * sc;
    float mg = max(p4, 0.0) * sc;
    int corner = int(clamp(p2, 0.0, 3.0) + 0.5);
    bool right = (corner == 1 || corner == 3);
    bool bottom = (corner == 2 || corner == 3);
    vec2 anchor = vec2(right ? u_res.x - mg - sz * 0.5 : mg + sz * 0.5,
                       bottom ? u_res.y - mg - sz * 0.5 : mg + sz * 0.5);
    float hz = max(p1, 0.0);
    float on = hz <= 0.0 ? 1.0 : (fract(u_time * hz) < 0.5 ? 1.0 : 0.0);
    float dot_ = 1.0 - smoothstep(sz * 0.5 - 1.0, sz * 0.5 + 1.0, distance(q, anchor));
    vec3 c = mix(src.rgb, vec3(0.9, 0.12, 0.12), dot_ * on);
    float cover = dot_ * on;
    if (p3 >= 0.5) {
        float dh = sz;
        float dw = sz * 0.55;
        float gap = sz * 0.12;
        float cw = dw + gap;
        float colw = sz * 0.3 + gap;
        float total = 6.0 * cw + 2.0 * colw;
        float ox = right ? anchor.x - sz * 0.5 - gap * 2.0 - total : anchor.x + sz * 0.5 + gap * 2.0;
        float oy = anchor.y - dh * 0.5;
        int tot = int(max(u_time, 0.0));
        int hh = (tot / 3600) - (tot / 360000) * 100;
        int mm = (tot / 60) - (tot / 3600) * 60;
        int ss = tot - (tot / 60) * 60;
        int digs[6] = int[6](hh / 10, hh - (hh / 10) * 10, mm / 10, mm - (mm / 10) * 10,
                             ss / 10, ss - (ss / 10) * 10);
        float ink = 0.0;
        float x = ox;
        for (int i = 0; i < 6; i++) {
            ink = max(ink, fx_digit(vec2((q.x - x) / dw, (q.y - oy) / dh), digs[i]));
            x += cw;
            if (i == 1 || i == 3) {
                float cx = x + colw * 0.5;
                ink = max(ink, fx_rect(q, vec2(cx, oy + dh * 0.33), vec2(sz * 0.07)));
                ink = max(ink, fx_rect(q, vec2(cx, oy + dh * 0.7), vec2(sz * 0.07)));
                x += colw;
            }
        }
        c = mix(c, vec3(1.0), ink * 0.9);
        cover = max(cover, ink * 0.9);
    }
    vec4 res = vec4(c, max(src.a, cover));
    float m = u_has_mask > 0.5 ? texture(u_mask, uv).r : 1.0;
    out_color = mix(src, res, m);
}
"#;

#[cfg(test)]
mod shader_body_tests {
    use super::*;
    use crate::model::EffectKind as K;

    /// Uniforms `PRELUDE` guarantees (p0..p7 handled separately).
    const PRELUDE_UNIFORMS: [&str; 5] = ["tex", "u_res", "u_time", "u_scale", "u_mask"];

    /// Parameters a body deliberately ignores (the CPU tracker consumes BlobTrack's smoothing).
    const UNUSED_PARAMS: [(K, usize); 1] = [(K::BlobTrack, 5)];

    fn bodies() -> Vec<(K, &'static str)> {
        K::ALL.iter().filter_map(|&k| body(k).map(|s| (k, s))).collect()
    }

    /// Every `u_xxx` / `tex` identifier the body reads, and every `pN` index it reads.
    fn identifiers(src: &str) -> (Vec<String>, Vec<usize>) {
        let mut words: Vec<String> = Vec::new();
        let mut cur = String::new();
        for ch in src.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                cur.push(ch);
            } else if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
        }
        if !cur.is_empty() {
            words.push(cur);
        }
        let params = words
            .iter()
            .filter_map(|w| w.strip_prefix('p').and_then(|n| n.parse::<usize>().ok().filter(|_| !n.is_empty())))
            .collect();
        (words, params)
    }

    /// Uniform names the body declares itself (`uniform <type> <name>;`).
    fn declared(src: &str) -> Vec<String> {
        src.split("uniform ")
            .skip(1)
            .filter_map(|rest| {
                let mut it = rest.split_whitespace();
                let _ty = it.next()?;
                Some(it.next()?.trim_end_matches(';').to_string())
            })
            .collect()
    }

    #[test]
    fn every_gpu_kind_has_a_body() {
        for k in K::ALL {
            let has = body(k).is_some();
            let want = !matches!(k, K::Wobble | K::Plane3d | K::Shader);
            assert_eq!(has, want, "{:?}", k);
        }
        assert_eq!(bodies().len(), 21);
    }

    #[test]
    fn bodies_are_well_formed_glsl_text() {
        for (k, src) in bodies() {
            assert!(!src.contains("#version"), "{k:?}: the prelude owns #version");
            assert!(!src.contains("precision "), "{k:?}: the prelude owns precision");
            assert!(src.contains("void main()"), "{k:?}: no entry point");
            assert!(src.contains("out_color ="), "{k:?}: never writes out_color");
            for (open, close) in [('{', '}'), ('(', ')'), ('[', ']')] {
                let mut depth = 0i32;
                for ch in src.chars() {
                    if ch == open {
                        depth += 1;
                    } else if ch == close {
                        depth -= 1;
                        assert!(depth >= 0, "{k:?}: unbalanced {close}");
                    }
                }
                assert_eq!(depth, 0, "{k:?}: unbalanced {open}{close}");
            }
            // one statement per line keeps the driver logs readable; no tabs, no trailing space
            for line in src.lines() {
                assert!(!line.contains('\t'), "{k:?}: tab in {line:?}");
                assert_eq!(line.trim_end(), line, "{k:?}: trailing space in {line:?}");
            }
        }
    }

    #[test]
    fn bodies_only_touch_declared_uniforms() {
        for (k, src) in bodies() {
            let own = declared(src);
            let (words, _) = identifiers(src);
            for w in words.iter().filter(|w| w.starts_with("u_") || *w == "tex") {
                let ok = PRELUDE_UNIFORMS.contains(&w.as_str()) || w == "u_has_mask" || own.contains(w);
                assert!(ok, "{k:?}: undeclared uniform {w}");
            }
        }
    }

    #[test]
    fn bodies_match_the_parameter_list() {
        for (k, src) in bodies() {
            let n = k.params().len();
            let own = declared(src);
            let (_, used) = identifiers(src);
            for i in &used {
                assert!(*i < n, "{k:?}: reads p{i} but only has {n} params");
                // > 8 params means the body has to declare the extras itself
                if *i >= 8 {
                    assert!(own.contains(&format!("p{i}")), "{k:?}: p{i} is past the prelude and undeclared");
                }
            }
            for i in 0..n {
                let known_unused = UNUSED_PARAMS.contains(&(k, i));
                assert!(used.contains(&i) || known_unused, "{k:?}: {} (p{i}) is never read", k.params()[i].name);
            }
        }
    }

    #[test]
    fn masked_and_scaled_where_it_matters() {
        for (k, src) in bodies() {
            assert!(src.contains("u_has_mask"), "{k:?}: ignores its mask");
        }
        // pixel-sized parameters must go through u_scale or preview and export disagree
        for k in [K::Blur, K::Pixelate, K::Sharpen, K::EdgeGlow, K::RecDot, K::ChromaKey, K::Vhs, K::JpegCompress] {
            assert!(body(k).unwrap().contains("u_scale"), "{k:?}: pixel sizes not scaled");
        }
    }

    #[test]
    fn extra_uniforms_are_documented() {
        // the two bodies that need gpu.rs to bind more than the standard set
        assert_eq!(declared(MOTION_BLUR), ["u_prev", "u_next", "u_frames", "u_motion"]);
        assert_eq!(declared(BLOB_TRACK), ["u_blob"]);
        assert_eq!(declared(CURVES), ["p8", "p9", "p10", "p11"]);
        for (_, src) in bodies() {
            if src != MOTION_BLUR && src != BLOB_TRACK && src != CURVES {
                assert!(declared(src).is_empty(), "undocumented extra uniform in {:?}", declared(src));
            }
        }
    }
}
