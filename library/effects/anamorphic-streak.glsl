// @name Anamorphic Streak
// @desc Long horizontal blue lens streaks shoot out of bright lights, like an anamorphic cinema lens.
// @tags anamorphic, streak, flare, lens, cinematic, sci-fi, highlights
// @param threshold 0.85 0 1
// @param streak_length 1.0 0.05 2
// @param intensity 1.0 0 3
// @param tint_hue 0.62 0 1

vec3 an_bright(vec3 c, float t) {
    float l = luma(c);
    return vec3(luma(c)) * smoothstep(t, min(t + 0.08, 1.0001), l);
}

vec4 effect(vec4 src, vec2 uv) {
    // streak_length is a share of the frame width, so it is the same at every resolution
    float L = streak_length * 0.5;
    float jit = hash12(floor(uv * u_res)) - 0.5;
    vec3 acc = vec3(0.0);
    float wsum = 0.0;
    for (int i = 0; i < 32; i++) {
        // taps bunch up near the light (t^2) where the streak is brightest
        float t = (float(i) + 0.5 + jit * 0.9) / 32.0;
        float x = L * t * t;
        float w = t * exp(-2.0 * t) * (1.0 - t * t);
        vec3 a = an_bright(sample_clamped(tex, uv + vec2(x, 0.0)).rgb, threshold);
        vec3 b = an_bright(sample_clamped(tex, uv - vec2(x, 0.0)).rgb, threshold);
        acc += (a + b) * w;
        wsum += 2.0 * w;
    }
    float s = (acc / wsum).r;
    // a thin, saturated core and a paler outer streak
    vec3 tint = hsl2rgb(vec3(tint_hue, 0.9, 0.55));
    vec3 core = mix(tint, vec3(1.0), 0.35);
    vec3 streak = tint * (s * 2.5 + s * s * 20.0) + core * s * s * s * 40.0;
    vec3 col = src.rgb + streak * intensity;
    return vec4(min(col, vec3(1.0)), src.a);
}
