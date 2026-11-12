// @name Halation
// @desc Film halation: a red-orange glow bleeds out around bright highlights, like light scattering in film stock.
// @tags film, halation, glow, highlights, analog, cinematic, warm
// @param threshold 0.78 0 1
// @param radius 32 2 120
// @param strength 1.0 0 3
// @param warmth 0.35 0 1

vec3 ha_bright(vec3 c, float t) {
    float l = luma(c);
    float k = smoothstep(t, min(t + 0.15, 1.0001), l);
    return c * k;
}

vec4 effect(vec4 src, vec2 uv) {
    vec2 px = 1.0 / u_res;
    float rad = max(radius * u_scale, 1.0);
    // rotate the spiral per pixel so the taps blend into a smooth glow instead of rings
    float rot = hash12(floor(uv * u_res)) * 6.2832;
    vec3 acc = vec3(0.0);
    float wsum = 0.0;
    for (int i = 0; i < 48; i++) {
        float fi = float(i) + 0.5;
        float t = fi / 48.0;
        float r = rad * sqrt(t);
        float a = fi * 2.39996 + rot;
        vec3 c = ha_bright(texture(tex, uv + vec2(cos(a), sin(a)) * r * px).rgb, threshold);
        float w = exp(-t * 2.5);
        acc += c * w;
        wsum += w;
    }
    vec3 glow = acc / wsum;
    // halation is a single-colour scatter: take the brightness, colour it red-orange
    float g = luma(glow) * 1.8;
    vec3 tint = mix(vec3(1.0, -0.12, -0.12), vec3(1.0, 0.35, 0.05), warmth);
    // the glow sits around the highlight, so hold it back on the highlight itself
    float self = luma(src.rgb);
    vec3 add = tint * g * strength * (1.0 - 0.85 * smoothstep(threshold, 1.0, self));
    vec3 col = max(src.rgb + add, vec3(0.0));
    // a faint warm lift in the near-highlights, like real stock
    col = mix(col, col * vec3(1.03, 0.99, 0.96), clamp(g * 2.0, 0.0, 1.0) * warmth);
    return vec4(min(col, vec3(1.0)), src.a);
}
