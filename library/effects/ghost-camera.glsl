// @name Ghost Camera
// @desc The spirit-camera look: anything that moves leaves pale, cool, translucent copies of itself behind.
// @tags ghost, spirit, trail, echo, motion, paranormal, horror
// @param strength 0.65 0 1
// @param trail_tint 0.7 0 1
// @param spread 1.0 0 3
// @param glow 0.5 0 1
// @needs previous

vec4 effect(vec4 src, vec2 uv) {
    if (u_frames < 0.5) return src;
    vec2 c = vec2(0.5);
    // each older copy is pushed a little further out and up, as if drifting away
    vec2 uvN = c + (uv - c) / (1.0 + 0.010 * spread) + vec2(0.0, 0.004 * spread);
    vec2 uvP = c + (uv - c) / (1.0 + 0.025 * spread) + vec2(0.0, 0.010 * spread);
    vec3 n = sample_clamped(u_next, uvN).rgb;
    vec3 p = sample_clamped(u_prev, uvP).rgb;
    // the in-between copy also extrapolates past the older one for a longer trail
    vec2 uvX = c + (uv - c) / (1.0 + 0.04 * spread) + vec2(0.0, 0.016 * spread);
    vec3 x = sample_clamped(u_prev, uvX).rgb;

    // only where something moved at the place each copy comes from: a still background stays clean
    float dn = smoothstep(0.05, 0.25, length(n - sample_clamped(tex, uvN).rgb));
    float dp = smoothstep(0.05, 0.25, length(p - sample_clamped(tex, uvP).rgb));
    float dx = smoothstep(0.05, 0.25, length(x - sample_clamped(tex, uvX).rgb));

    // ghosts are washed out and cold
    vec3 cold = vec3(0.62, 0.86, 1.15);
    vec3 gn = mix(n, vec3(luma(n)) * cold + 0.08, trail_tint);
    vec3 gp = mix(p, vec3(luma(p)) * cold + 0.12, trail_tint);
    vec3 gx = mix(x, vec3(luma(x)) * cold + 0.16, trail_tint);

    vec3 col = src.rgb;
    col = mix(col, gx, dx * strength * 0.3);
    col = mix(col, gp, dp * strength * 0.42);
    col = mix(col, gn, dn * strength * 0.5);
    // a faint luminous fringe makes them read as spirits rather than double exposure
    float ghostness = max(dn, max(dp, dx)) * strength;
    col += vec3(0.05, 0.1, 0.16) * ghostness * glow;
    // slightly desaturate and cool the whole frame so the ghosts belong to it
    col = mix(col, vec3(luma(col)) * vec3(0.95, 1.0, 1.06), 0.15 * trail_tint);
    return vec4(clamp(col, 0.0, 1.0), src.a);
}
