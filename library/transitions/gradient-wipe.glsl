// @name Gradient Wipe
// @desc The incoming picture fills in from the dark parts of a gradient to the light parts: a built-in pattern, or any greyscale image you pick.
// @tags wipe, gradient, luma, matte, custom, image, pattern
// @param pattern 2 0 7
// @param softness 0.12 0 0.5
// @param invert 0 0 1
// @param detail 6 1 24
// The Inspector's Gradient Image button writes `// @image <path>` into the clip's copy of this file;
// with an image, `pattern` is ignored. Patterns: 0 left to right, 1 top to bottom, 2 centre out,
// 3 clock, 4 diagonal, 5 clouds, 6 blinds, 7 diamonds.
float gw_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash12(i);
    float b = hash12(i + vec2(1.0, 0.0));
    float c = hash12(i + vec2(0.0, 1.0));
    float d = hash12(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}
float gw_pattern(vec2 uv) {
    // uv: y = 0 at the bottom; t = the same point with y = 0 at the top
    vec2 t = vec2(uv.x, 1.0 - uv.y);
    int k = int(floor(pattern + 0.5));
    if (k == 0) { return t.x; }
    if (k == 1) { return t.y; }
    vec2 c = (t - 0.5) * vec2(ratio, 1.0);
    if (k == 2) { return clamp(length(c) / (0.5 * length(vec2(ratio, 1.0))), 0.0, 1.0); }
    if (k == 3) { return fract(atan(c.x, -c.y) / 6.28318531); }
    if (k == 4) { return clamp((t.x * ratio + t.y) / (ratio + 1.0), 0.0, 1.0); }
    if (k == 5) {
        vec2 p = t * vec2(ratio, 1.0) * detail * 0.5;
        float n = 0.5 * gw_noise(p) + 0.25 * gw_noise(p * 2.03) + 0.125 * gw_noise(p * 4.01);
        return clamp((n / 0.875 - 0.15) / 0.7, 0.0, 1.0);
    }
    if (k == 6) { return fract(t.y * detail); }
    vec2 q = abs(fract(t * vec2(ratio, 1.0) * detail * 0.5) - 0.5);
    return clamp(q.x + q.y, 0.0, 1.0);
}
vec4 transition(vec2 uv) {
    float g = (u_has_gradient > 0.5) ? luma(texture(u_gradient, vec2(uv.x, 1.0 - uv.y)).rgb) : gw_pattern(uv);
    if (invert > 0.5) { g = 1.0 - g; }
    float s = max(softness, 1e-4);
    float edge = progress * (1.0 + s) - s;
    float m = smoothstep(g, g + s, edge + s);
    if (progress <= 0.0) { m = 0.0; }
    if (progress >= 1.0) { m = 1.0; }
    return mix(getFromColor(uv), getToColor(uv), m);
}
