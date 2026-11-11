// @name Beat Pulse
// @desc Pulses on every beat of a tempo: a quick scale bump, brightness flash and RGB split that fade out.
// @tags beat, pulse, music, rhythm, bpm, flash, rgb
// @param bpm 128 40 200
// @param bump 0.06 0 0.2
// @param flash 0.14 0 1
// @param split 1 0 3
// @param decay 7 1 20
vec4 effect(vec4 src, vec2 uv) {
    float period = 60.0 / bpm;
    float tau = mod(u_time, period);
    float env = exp(-decay * tau);
    vec2 c = vec2(0.5);
    vec2 zuv = (uv - c) / (1.0 + bump * env) + c;
    vec2 d = (zuv - c) * 0.015 * split * env;
    vec4 g = sample_clamped(tex, zuv);
    float r = sample_clamped(tex, zuv + d).r;
    float b = sample_clamped(tex, zuv - d).b;
    vec3 col = vec3(r, g.g, b);
    col += flash * env * (0.25 + col);
    return vec4(col, g.a);
}
