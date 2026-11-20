// @name Smart Blur
// @desc Real-time noise reduction: smooths grain and flat areas while keeping edges sharp.
// @tags denoise, noise reduction, smart blur, bilateral, smooth, clean, grain
// @param radius 3 1 8
// @param threshold 0.08 0.01 0.3
// @param amount 1 0 1
vec4 effect(vec4 src, vec2 uv) {
    vec2 px = radius * u_scale / u_res;
    vec3 sum = vec3(0.0);
    float wsum = 0.0;
    for (int y = -3; y <= 3; y++) {
        for (int x = -3; x <= 3; x++) {
            vec2 o = vec2(float(x), float(y)) / 3.0;
            vec3 s = texture(tex, clamp(uv + o * px, 0.0, 1.0)).rgb;
            vec3 d = s - src.rgb;
            float w = exp(-dot(o, o) * 1.5) * exp(-dot(d, d) / (2.0 * threshold * threshold));
            sum += s * w;
            wsum += w;
        }
    }
    return vec4(mix(src.rgb, sum / max(wsum, 1e-4), amount), src.a);
}
