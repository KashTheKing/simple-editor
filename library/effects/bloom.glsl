// @name Bloom
// @desc Soft dreamy bloom: bright parts of the picture glow and spill light into their surroundings.
// @tags bloom, glow, dreamy, soft, highlights, light
// @param threshold 0.72 0 1
// @param radius 90 5 250
// @param intensity 0.9 0 2
// @param warmth 0.3 0 1

vec3 bl_bright(vec3 c, float t) {
    float l = luma(c);
    // soft knee so the glow fades in rather than switching on
    float k = smoothstep(t - 0.1, min(t + 0.3, 1.0001), l);
    return c * k;
}

vec4 effect(vec4 src, vec2 uv) {
    vec2 px = 1.0 / u_res;
    float rad = max(radius * u_scale, 1.5);
    float rot = hash12(floor(uv * u_res)) * 6.2832;
    // two rings of taps: a tight core and a wide soft skirt
    vec3 inner = vec3(0.0);
    vec3 outer = vec3(0.0);
    for (int i = 0; i < 28; i++) {
        float fi = float(i) + 0.5;
        float t = fi / 28.0;
        float a = fi * 2.39996 + rot;
        vec2 d = vec2(cos(a), sin(a));
        inner += bl_bright(texture(tex, uv + d * rad * 0.3 * sqrt(t) * px).rgb, threshold) * exp(-t * 2.0);
        outer += bl_bright(texture(tex, uv + d * rad * sqrt(t) * px).rgb, threshold) * exp(-t * 2.0);
    }
    // sum of exp(-2t) over the taps
    float norm = 28.0 * 0.4323;
    vec3 glow = (inner * 0.55 + outer * 0.45) / norm;
    glow *= mix(vec3(1.0), vec3(1.0, 0.82, 0.6), warmth);
    glow *= intensity * 2.6;
    // screen blend keeps it from clipping hard
    vec3 col = 1.0 - (1.0 - src.rgb) * (1.0 - clamp(glow, 0.0, 1.0));
    return vec4(col, src.a);
}
