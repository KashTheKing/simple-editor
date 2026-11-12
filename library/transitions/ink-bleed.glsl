// @name Ink Bleed
// @desc The incoming picture blooms in like ink dropped in water, with feathery tendrils and a darker wet edge.
// @tags ink, water, bleed, organic, watercolor, dissolve
// @param scale 2 0.5 6
// @param swirl 4 0 8
// @param feather 1 0.2 3
// @param wet_edge 0.6 0 1

float ib_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    float a = hash12(i);
    float b = hash12(i + vec2(1.0, 0.0));
    float c = hash12(i + vec2(0.0, 1.0));
    float d = hash12(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float ib_fbm(vec2 p) {
    float s = 0.0;
    float a = 0.5;
    for (int i = 0; i < 5; i++) {
        s += a * ib_noise(p);
        p = mat2(1.6, 1.2, -1.2, 1.6) * p + vec2(1.7, 9.2);
        a *= 0.5;
    }
    return s / 0.96875;
}

vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);

    vec2 asp = vec2(ratio, 1.0);
    vec2 p = uv * asp * scale;
    // the ink keeps curling as it spreads
    float drift = progress * 0.6;

    vec2 q = vec2(ib_fbm(p + vec2(0.0, drift)), ib_fbm(p + vec2(5.2, 1.3 - drift)));
    vec2 r = vec2(ib_fbm(p + swirl * q + vec2(1.7, 9.2) + drift),
                  ib_fbm(p + swirl * q + vec2(8.3, 2.8)));
    float v = ib_fbm(p + swirl * r);

    // two drops of ink: the spread starts from them
    vec2 a = uv * asp;
    float d1 = length(a - vec2(0.38, 0.58) * asp);
    float d2 = length(a - vec2(0.72, 0.36) * asp) + 0.12;
    float rad = min(d1, d2);

    // fine fibres along the edge
    float fib = ib_fbm(p * 9.0 + r * 3.0);
    float F = v * 0.75 - rad * 0.75 + (fib - 0.5) * 0.06 * feather;

    float th = mix(0.58, -0.32, progress);
    float px = fwidth(F) + 1e-4;
    float soft = 0.012 * feather + px;
    float m = smoothstep(th - soft, th + soft, F);
    // any last specks soak through in the final frames
    m = max(m, smoothstep(0.88, 1.0, progress));

    vec4 outg = getFromColor(uv);
    vec4 inc = getToColor(uv);

    // wet edge: pigment piles up just inside the front
    float inside = max(F - th, 0.0);
    float edge = exp(-inside / (0.03 * feather)) * m;
    vec3 incc = inc.rgb * (1.0 - 0.45 * wet_edge * edge);
    // a faint damp halo runs ahead of the ink on the outgoing picture
    float halo = exp(-max(th - F, 0.0) / 0.05) * (1.0 - m);
    vec3 outc = outg.rgb * (1.0 - 0.18 * wet_edge * halo);

    return vec4(mix(outc, incc, m), mix(outg.a, inc.a, m));
}
