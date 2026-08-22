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
pub fn body(kind: EffectKind) -> Option<&'static str> {
    match kind {
        // Geometric kinds never get a body — the renderer folds them into the placement matrix.
        EffectKind::Wobble | EffectKind::Plane3d => None,
        // The rest are filled in by the effect bodies (engine/shaders.rs, gpu-effects group).
        _ => None,
    }
}

/// The complete fragment source for one effect, or None when it has no GPU body.
pub fn fragment(kind: EffectKind, user_src: &str) -> Option<String> {
    if kind == EffectKind::Shader {
        return Some(user_shader(user_src));
    }
    Some(format!("{PRELUDE}\n{}\n{MAIN}", body(kind)?))
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
