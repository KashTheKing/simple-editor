// @name Deband
// @desc Smooths the stepped bands in skies and gradients from heavy compression, leaving real detail alone.
// @tags deband, banding, gradient, compression, smooth, cleanup
// @param threshold 0.03 0.002 0.1
// @param range 12 2 40
// @param grain 1 0 1
vec4 effect(vec4 src, vec2 uv) {
    vec2 px = 1.0 / u_res;
    float r = range * u_scale;
    float a = hash12(uv * u_res + 0.13) * 6.28318531;
    vec3 sum = src.rgb;
    float n = 1.0;
    for (int i = 0; i < 8; i++) {
        float ang = a + float(i) * 0.785398;
        float rr = r * (0.5 + 0.5 * hash12(uv * u_res + float(i) * 7.1));
        vec3 s = texture(tex, clamp(uv + vec2(cos(ang), sin(ang)) * rr * px, 0.0, 1.0)).rgb;
        // only average with neighbours that are almost the same (a band, not an edge)
        if (max(max(abs(s.r - src.r), abs(s.g - src.g)), abs(s.b - src.b)) < threshold) {
            sum += s;
            n += 1.0;
        }
    }
    vec3 c = sum / n;
    // a little dither so the smoothed ramp does not band again on an 8-bit export
    c += (hash12(uv * u_res + fract(u_time)) - 0.5) * grain * 4.0 / 255.0;
    return vec4(c, src.a);
}
