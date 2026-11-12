// @name Zoom Punch
// @desc Punch-in zoom that snaps in fast and eases back, repeating, with a touch of radial blur at the peak.
// @tags zoom, punch, pulse, emphasis, impact, radial blur
// @param amount 0.3 0 0.8
// @param interval 1.5 0.4 10
// @param sharpness 4 1 12
vec4 effect(vec4 src, vec2 uv) {
    float tau = mod(u_time, interval);
    float att = 0.07;
    float env = tau < att ? smoothstep(0.0, att, tau) : exp(-(tau - att) * sharpness);
    float z = 1.0 + amount * env;
    vec2 c = vec2(0.5);
    vec2 zuv = (uv - c) / z + c;
    // radial blur toward the centre, strongest at the peak
    float blur = amount * 0.35 * env * env;
    vec3 acc = vec3(0.0);
    float a = 0.0;
    for (int i = 0; i < 12; i++) {
        float k = float(i) / 11.0;
        vec4 s = sample_clamped(tex, (zuv - c) * (1.0 - blur * k) + c);
        acc += s.rgb * s.a;
        a += s.a;
    }
    return vec4(a > 0.0 ? acc / a : vec3(0.0), a / 12.0);
}
