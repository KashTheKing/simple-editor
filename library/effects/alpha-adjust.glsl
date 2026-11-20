// @name Alpha Adjust
// @desc Grows, shrinks, softens or hardens a clip's transparency edge, or inverts it.
// @tags alpha, matte, choke, spread, feather, edge, transparency, key
// @param grow -16 -40 40
// @param soften 10 0 20
// @param threshold 0 0 1
// @param invert 0 0 1
vec4 effect(vec4 src, vec2 uv) {
    vec2 px = 1.0 / u_res;
    float r = abs(grow) * u_scale;
    float a = src.a;
    if (r > 0.0) {
        // min (shrink) or max (grow) of alpha over a small ring
        for (int i = 0; i < 12; i++) {
            float ang = float(i) * 0.5235988;
            float s = texture(tex, clamp(uv + vec2(cos(ang), sin(ang)) * r * px, 0.0, 1.0)).a;
            a = (grow > 0.0) ? max(a, s) : min(a, s);
        }
    }
    if (soften > 0.0) {
        float sum = a;
        for (int i = 0; i < 8; i++) {
            float ang = float(i) * 0.785398 + 0.39;
            sum += texture(tex, clamp(uv + vec2(cos(ang), sin(ang)) * soften * u_scale * px, 0.0, 1.0)).a;
        }
        a = mix(a, sum / 9.0, 0.8);
    }
    if (threshold > 0.0) { a = smoothstep(threshold - 0.05, threshold + 0.05, a); }
    if (invert > 0.5) { a = 1.0 - a; }
    return vec4(src.rgb, clamp(a, 0.0, 1.0));
}
