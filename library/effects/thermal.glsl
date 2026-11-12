// @name Thermal Camera
// @desc Heat-camera palette: black, blue, purple, red, orange, yellow, white by brightness.
// @tags thermal, heat, infrared, palette, false color
// @param contrast 1.4 0.5 3
// @param shift 0 -0.4 0.4
// @param blur 3 0 10
vec3 thermal_ramp(float t) {
    t = clamp(t, 0.0, 1.0) * 6.0;
    vec3 c0 = vec3(0.0, 0.0, 0.02);
    vec3 c1 = vec3(0.05, 0.05, 0.65);
    vec3 c2 = vec3(0.5, 0.0, 0.6);
    vec3 c3 = vec3(0.9, 0.1, 0.1);
    vec3 c4 = vec3(1.0, 0.55, 0.0);
    vec3 c5 = vec3(1.0, 0.92, 0.2);
    vec3 c6 = vec3(1.0, 1.0, 0.9);
    if (t < 1.0) return mix(c0, c1, smoothstep(0.0, 1.0, t));
    if (t < 2.0) return mix(c1, c2, smoothstep(0.0, 1.0, t - 1.0));
    if (t < 3.0) return mix(c2, c3, smoothstep(0.0, 1.0, t - 2.0));
    if (t < 4.0) return mix(c3, c4, smoothstep(0.0, 1.0, t - 3.0));
    if (t < 5.0) return mix(c4, c5, smoothstep(0.0, 1.0, t - 4.0));
    return mix(c5, c6, smoothstep(0.0, 1.0, t - 5.0));
}
vec4 effect(vec4 src, vec2 uv) {
    vec2 r = blur * u_scale / u_res;
    float l = luma(src.rgb) * 4.0;
    float w = 4.0;
    for (int i = 0; i < 8; i++) {
        float a = float(i) * 0.785398;
        l += luma(texture(tex, uv + vec2(cos(a), sin(a)) * r).rgb) * 2.0;
        l += luma(texture(tex, uv + vec2(cos(a), sin(a)) * r * 0.5).rgb) * 2.0;
        w += 4.0;
    }
    l /= w;
    l = (l - 0.5) * contrast + 0.5 + shift - 0.1;
    return vec4(thermal_ramp(l), src.a);
}
