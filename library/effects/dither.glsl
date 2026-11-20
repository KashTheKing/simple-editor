// @name Dither
// @desc Reduces the picture to a few levels per channel with an ordered dither pattern, like old computer graphics.
// @tags dither, bayer, retro, 8-bit, pixel, posterize, low color
// @param levels 4 2 16
// @param pixel 2 1 12
// @param mono 0 0 1
vec4 effect(vec4 src, vec2 uv) {
    float cell = max(pixel * u_scale, 1.0);
    vec2 p = floor(uv * u_res / cell);
    vec3 c = texture(tex, clamp((p + 0.5) * cell / u_res, 0.0, 1.0)).rgb;
    if (mono > 0.5) { c = vec3(luma(c)); }
    // 4x4 Bayer threshold, built from two 2x2 levels
    vec2 q = mod(p, 4.0);
    vec2 h = floor(q / 2.0);
    vec2 l = mod(q, 2.0);
    float b2 = 2.0 * l.x + 3.0 * l.y - 4.0 * l.x * l.y;
    float b2h = 2.0 * h.x + 3.0 * h.y - 4.0 * h.x * h.y;
    float t = (4.0 * b2 + b2h + 0.5) / 16.0;
    float n = max(levels - 1.0, 1.0);
    vec3 o = floor(c * n + t) / n;
    return vec4(clamp(o, 0.0, 1.0), src.a);
}
