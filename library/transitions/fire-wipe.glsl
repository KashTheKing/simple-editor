// @name Fire Wipe
// @desc A wall of flame sweeps up the frame and leaves the incoming picture behind it.
// @tags fire, flame, wipe, heat, smoke
// @param flame_height 0.35 0.1 0.8
// @param turbulence 1 0 2
// @param smoke 0.7 0 1
// @param heat_haze 1 0 3

float fw_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    float a = hash12(i);
    float b = hash12(i + vec2(1.0, 0.0));
    float c = hash12(i + vec2(0.0, 1.0));
    float d = hash12(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float fw_fbm(vec2 p) {
    float s = 0.0;
    float a = 0.5;
    for (int i = 0; i < 5; i++) {
        s += a * fw_noise(p);
        p = mat2(1.7, 1.1, -1.1, 1.7) * p + vec2(2.3, 8.1);
        a *= 0.5;
    }
    return s / 0.96875;
}

vec3 fw_ramp(float k) {
    // black-red -> red -> orange -> yellow -> near white
    vec3 c = mix(vec3(0.25, 0.02, 0.0), vec3(0.85, 0.12, 0.02), smoothstep(0.0, 0.25, k));
    c = mix(c, vec3(1.0, 0.45, 0.05), smoothstep(0.2, 0.5, k));
    c = mix(c, vec3(1.0, 0.8, 0.25), smoothstep(0.45, 0.75, k));
    c = mix(c, vec3(1.0, 0.97, 0.85), smoothstep(0.75, 1.0, k));
    return c;
}

vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);

    float H = flame_height;
    float smokeH = H * 0.9;
    float glowBand = 0.08;
    // the base line of the fire climbs from below the frame to above it
    float base = mix(-H * 1.3 - 0.06, 1.0 + glowBand + 0.02, progress);
    float tm = progress * 7.0;    // flames rise with progress (scrubbable)

    vec2 p = vec2(uv.x * ratio, uv.y);
    // ragged base line
    float baseY = base + (fw_fbm(vec2(p.x * 3.0, 4.2)) - 0.5) * 0.08;
    float h = (uv.y - baseY) / H;       // 0 at the base, 1 at a typical flame tip

    // flame tongues: noise stretched vertically and scrolled upward
    vec2 q = vec2(p.x * 6.0, p.y * 1.4 - tm);
    float warp = fw_fbm(q * 0.6 + vec2(0.0, -tm * 0.3));
    float n = fw_fbm(q + vec2(warp * 1.6 * turbulence, 0.0));
    // tall tongues where this is high, low flames between them
    float tongues = 0.5 + 0.7 * fw_noise(vec2(p.x * 5.0 + warp, -tm * 0.4));
    float fire = (n - 0.35) * 2.2 + 0.25 - h / tongues;   // > 0 burning
    fire = min(fire, (h + 0.15) * 6.0);              // nothing burns below the base

    // heat haze above the flames bends the outgoing picture
    float haze = heat_haze * 0.006 * smoothstep(1.6, 0.6, h) * step(0.0, h);
    vec2 duv = vec2((fw_noise(vec2(p.x * 18.0, p.y * 10.0 - tm * 2.0)) - 0.5) * haze, 0.0);
    vec4 outg = getFromColor(clamp(uv + duv, 0.0, 1.0));
    vec4 inc = getToColor(uv);

    // below the base the incoming picture, warmed by the fire right behind it
    float behind = 1.0 - smoothstep(-0.02, 0.02, uv.y - baseY + 0.01);
    float heat = exp(-max(baseY - uv.y, 0.0) / glowBand * 1.5);
    vec3 incc = inc.rgb + vec3(1.0, 0.35, 0.05) * 0.4 * heat;
    vec4 col = mix(outg, vec4(incc, inc.a), behind);

    // smoke: dark, soft, drifting above the flame tips
    float sn = fw_fbm(vec2(p.x * 2.5, p.y * 2.0 - tm * 0.6) + warp);
    float smk = smoothstep(0.6, 1.0, h) * (1.0 - smoothstep(1.2, 1.2 + smokeH / H, h)) * smoothstep(0.35, 0.75, sn);
    col.rgb = mix(col.rgb, vec3(0.07, 0.06, 0.06), smk * smoke * 0.85 * smoothstep(0.0, 0.2, progress));

    // flames on top
    float k = clamp(fire * 0.9 + 0.1, 0.0, 1.0);
    float aa = fwidth(fire) + 0.002;
    float fa = smoothstep(0.0, aa * 2.0 + 0.1, fire);
    // the base is a solid band of fire so the seam never shows
    float core = smoothstep(0.25, 0.0, h) * smoothstep(-0.25, -0.05, h);
    k = max(k, core * 0.85);
    fa = max(fa, core);
    vec3 fc = fw_ramp(k);
    col.rgb = mix(col.rgb, fc, fa);
    // additive glow around the flames
    col.rgb += vec3(1.0, 0.4, 0.06) * 0.25 * exp(-max(-fire, 0.0) * 6.0) * smoothstep(-1.2, 0.0, h);
    col.a = mix(col.a, 1.0, fa);
    return clamp(col, 0.0, 1.0);
}
