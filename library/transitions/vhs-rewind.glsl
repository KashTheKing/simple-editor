// @name VHS Rewind
// @desc VHS rewind: tearing bands roll up the frame, colour bleeds, a noise bar sweeps through and the picture wobbles desaturated at the swap.
// @tags vhs, rewind, retro, tape, glitch, noise
// @param intensity 1 0 2
vec4 vh_pic(vec2 c, float sw) {
    return sw > 0.5 ? getToColor(c) : getFromColor(c);
}

vec4 transition(vec2 uv) {
    float p = progress;
    float g = sin(3.14159265 * p) * intensity;
    float w = smoothstep(0.3, 0.5, p) * smoothstep(0.7, 0.5, p);
    float fr = floor(p * 30.0);
    float ln = floor(uv.y * 240.0);
    float sy = p * 1.3 - 0.15;
    float sw = uv.y < sy ? 1.0 : 0.0;
    float bar = exp(-pow((uv.y - sy) / 0.06, 2.0)) * smoothstep(0.0, 0.1, p) * smoothstep(1.0, 0.9, p);
    float by = fract(p * 2.5);
    float bump = exp(-pow((uv.y - by) / 0.07, 2.0)) + 0.6 * exp(-pow((uv.y - fract(by + 0.55)) / 0.04, 2.0));
    float dx = g * (0.05 * bump * sin(uv.y * 60.0 + fr)
        + (hash11(ln + fr * 7.0) - 0.5) * 0.012 * (0.3 + bump)
        + 0.006 * sin(uv.y * 9.0 + p * 50.0))
        + bar * 0.04 * (hash11(ln + fr) - 0.5) * intensity;
    float cb = 0.006 * g + 0.01 * bar;
    vec2 q = vec2(uv.x + dx, uv.y);
    float r = 0.0;
    float b = 0.0;
    for (int i = 0; i < 3; i++) {
        float o = float(i) * 0.5;
        r += vh_pic(q + vec2(cb * (1.0 + o), 0.0), sw).r;
        b += vh_pic(q - vec2(cb * (1.0 + o), 0.0), sw).b;
    }
    vec4 gc = vh_pic(q, sw);
    vec4 col = vec4(r / 3.0, gc.g, b / 3.0, gc.a);
    col.rgb = mix(col.rgb, vec3(luma(col.rgb)), 0.6 * w);
    float n = hash12(vec2(floor(uv.x * u_res.x * 0.25), ln + fr * 3.0));
    col.rgb = mix(col.rgb, vec3(n * 0.9 + 0.1), bar * 0.85);
    col.rgb += (hash12(uv * u_res + fr) - 0.5) * 0.08 * g;
    return col;
}
