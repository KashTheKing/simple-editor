// @name Film Burn
// @desc Warm orange and white burn blobs bloom in from the left, overexpose the frame, then roll off to the right over the incoming picture.
// @tags film, burn, retro, analog, flash, warm
// @param intensity 1 0.3 2
// @param blob_scale 1 0.3 3
// @param redness 1 0 2
// @param flicker 0.5 0 1

float fb_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    float a = hash12(i);
    float b = hash12(i + vec2(1.0, 0.0));
    float c = hash12(i + vec2(0.0, 1.0));
    float d = hash12(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float fb_fbm(vec2 p) {
    float s = 0.0;
    float a = 0.5;
    for (int i = 0; i < 5; i++) {
        s += a * fb_noise(p);
        p = mat2(1.6, 1.2, -1.2, 1.6) * p + vec2(4.1, 2.7);
        a *= 0.5;
    }
    return s / 0.96875;
}

vec3 fb_ramp(float k) {
    vec3 c = mix(vec3(0.55, 0.04, 0.0), vec3(1.0, 0.32, 0.04), smoothstep(0.0, 0.35, k));
    c = mix(c, vec3(1.0, 0.72, 0.3), smoothstep(0.3, 0.65, k));
    c = mix(c, vec3(1.0, 0.97, 0.9), smoothstep(0.6, 1.0, k));
    return c;
}

vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);

    float s = progress;
    vec2 p = uv * vec2(ratio, 1.0);
    float cover = 1.0 - abs(2.0 * s - 1.0);    // 0 -> 1 at the middle -> 0
    cover = cover * cover * (3.0 - 2.0 * cover);

    // the burn enters from the left and leaves to the right
    float side = smoothstep(0.42, 0.58, s);
    float g = mix(uv.x, 1.0 - uv.x, side) * ratio * 0.6;

    // drifting blobs: the film keeps moving through the gate
    vec2 q = p * 1.6 / blob_scale + vec2(-s * 2.5, s * 0.7);
    float blobs = fb_fbm(q);
    float holes = fb_fbm(q * 2.3 + 7.0);
    float peak = smoothstep(0.34, 0.5, s) * (1.0 - smoothstep(0.5, 0.66, s));
    float burn = (cover * cover * 2.4 + peak * 1.6) * intensity - g * 1.7
               + (blobs - 0.5) * 1.6 + (holes - 0.5) * 0.5;
    float I = max(burn, 0.0) * smoothstep(0.0, 0.3, cover);

    // frame-to-frame exposure flicker (driven by progress, so scrubbing is exact)
    float fl = 1.0 + flicker * 0.25 * (hash11(floor(s * 48.0)) - 0.5) * cover;
    I *= fl;

    // the picture underneath swaps while the frame is blown out
    float swap = smoothstep(0.46, 0.54, s);
    vec4 a = getFromColor(uv);
    vec4 b = getToColor(uv);
    vec4 base = mix(a, b, swap);

    // red-orange stain at the fringe, then a screen-blended hot core, then white
    float stain = smoothstep(0.0, 0.35, I) * redness;
    vec3 col = base.rgb * mix(vec3(1.0), vec3(1.0, 0.42, 0.2), clamp(stain * 0.8, 0.0, 1.0));
    vec3 light = fb_ramp(clamp(I / 1.1, 0.0, 1.0)) * smoothstep(0.02, 0.5, I);
    col = 1.0 - (1.0 - col) * (1.0 - light);
    col = mix(col, vec3(1.0, 0.98, 0.94), smoothstep(0.85, 1.25, I));
    // a general warm overexposure that peaks at the middle
    col = 1.0 - (1.0 - col) * (1.0 - vec3(0.35, 0.18, 0.06) * cover * intensity);

    float alpha = mix(base.a, 1.0, smoothstep(0.0, 0.5, I));
    return vec4(clamp(col, 0.0, 1.0), alpha);
}
