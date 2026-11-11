// @name Halftone
// @desc Printed halftone dots: CMYK screens at classic angles, or a single-ink newsprint look.
// @tags halftone, comic, print, dots, pop art, retro
// @param size 9 3 30
// @param angle 15 0 90
// @param mono 0 0 1
// @param contrast 1.1 0.5 2
// @param strength 1.0 0 1
vec2 ht_rot(vec2 p, float a) {
    float c = cos(a);
    float s = sin(a);
    return vec2(c * p.x - s * p.y, s * p.x + c * p.y);
}
vec4 effect(vec4 src, vec2 uv) {
    float cell = max(size * u_scale, 2.0);
    vec2 px = uv * u_res;
    float base = radians(angle - 15.0);
    float cov[4];
    for (int i = 0; i < 4; i++) {
        // screen angles: C 15, M 75, Y 0, K 45
        float a0 = (i == 0) ? 15.0 : (i == 1) ? 75.0 : (i == 2) ? 0.0 : 45.0;
        float a = radians(a0) + base;
        vec2 p = ht_rot(px, a) / cell;
        vec2 f = fract(p) - 0.5;
        vec2 cp = (floor(p) + 0.5) * cell;
        vec2 cuv = ht_rot(cp, -a) / u_res;
        vec3 s = texture(tex, clamp(cuv, 0.0, 1.0)).rgb;
        s = clamp((s - 0.5) * contrast + 0.5, 0.0, 1.0);
        float k = 1.0 - max(s.r, max(s.g, s.b));
        float ik = 1.0 / max(1.0 - k, 0.001);
        float ink = (i == 0) ? (1.0 - s.r - k) * ik
                  : (i == 1) ? (1.0 - s.g - k) * ik
                  : (i == 2) ? (1.0 - s.b - k) * ik : k;
        if (mono > 0.5) ink = (i == 3) ? 1.0 - luma(s) : 0.0;
        float rad = sqrt(clamp(ink, 0.0, 1.0)) * 0.66;
        float aa = 0.8 / cell;
        cov[i] = (rad < 0.02) ? 0.0 : 1.0 - smoothstep(rad - aa, rad + aa, length(f));
    }
    vec3 col = vec3(0.98, 0.96, 0.92);
    col *= mix(vec3(1.0), vec3(0.15, 0.72, 0.95), cov[0]);
    col *= mix(vec3(1.0), vec3(0.96, 0.3, 0.66), cov[1]);
    col *= mix(vec3(1.0), vec3(1.0, 0.95, 0.25), cov[2]);
    col *= mix(vec3(1.0), vec3(0.08, 0.07, 0.09), cov[3]);
    return vec4(mix(src.rgb, col, strength), src.a);
}
