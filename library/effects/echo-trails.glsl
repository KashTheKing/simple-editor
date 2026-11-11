// @name Echo Trails
// @desc Motion echoes: wherever something moved, bright colour-shifted copies of it trail behind and fade, like a long exposure.
// @tags echo, trail, motion, long exposure, music video, neon, dance
// @param amount 0.85 0 1.5
// @param hue_shift 0.82 0 1
// @param brightness 0.6 0 1
// @param sensitivity 0.5 0 1
// @needs previous

vec3 et_echo(vec3 c, float shift) {
    vec3 h = rgb2hsl(c);
    h.x = fract(h.x + shift);
    h.y = max(h.y, 0.9);
    h.z = clamp(h.z * 0.6 + 0.35, 0.0, 0.78);
    return hsl2rgb(h);
}

vec4 effect(vec4 src, vec2 uv) {
    if (u_frames < 0.5) return src;
    vec3 n = texture(u_next, uv).rgb;
    vec3 p = texture(u_prev, uv).rgb;
    // the key: how much each earlier frame differs from now
    float lo = mix(0.25, 0.03, sensitivity);
    float kn = smoothstep(lo, lo + 0.25, length(n - src.rgb));
    float kp = smoothstep(lo, lo + 0.25, length(p - src.rgb));

    // newer echo: one hue step, brighter; older echo: two steps, dimmer
    vec3 en = et_echo(n, hue_shift) * kn;
    vec3 ep = et_echo(p, hue_shift * 2.0) * kp * 0.65;
    vec3 trail = max(en, ep) * amount;

    // brighten: screen-blend the echoes, then a soft additive kick for the long-exposure glow
    float k = clamp(max(kn, kp * 0.65) * amount, 0.0, 1.0);
    vec3 col = mix(src.rgb, trail / max(amount, 0.01), k * 0.7);
    col = 1.0 - (1.0 - col) * (1.0 - clamp(trail * brightness * 0.5, 0.0, 1.0));
    return vec4(clamp(col, 0.0, 1.0), src.a);
}
