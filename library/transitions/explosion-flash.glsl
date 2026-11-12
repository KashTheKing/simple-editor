// @name Explosion Flash
// @desc A blast from the centre: radial streaks blow the outgoing picture outward into a white-orange flash that clears on the incoming one.
// @tags explosion, blast, flash, impact, action
// @param center_x 0.5 0 1
// @param center_y 0.5 0 1
// @param blast 1 0 2
// @param flash 1 0 1.5

float ex_noise(float x) {
    float i = floor(x);
    float f = fract(x);
    return mix(hash11(i), hash11(i + 1.0), f * f * (3.0 - 2.0 * f));
}

// zoom blur away from the centre: amt is how far the picture is blown outward
vec3 ex_blur(vec2 uv, vec2 c, float amt, float jit, bool from) {
    vec3 acc = vec3(0.0);
    const int N = 24;
    for (int i = 0; i < N; i++) {
        float t = (float(i) + jit) / float(N);
        float s = 1.0 - amt * t;
        vec2 q = c + (uv - c) * s;
        acc += from ? getFromColor(q).rgb : getToColor(q).rgb;
    }
    return acc / float(N);
}

vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);

    vec2 c = vec2(center_x, center_y);
    vec2 dv = (uv - c) * vec2(ratio, 1.0);
    float r = length(dv);
    float ang = atan(dv.y, dv.x);
    float jit = hash12(uv * u_res);

    // streaks: per-angle variation in how hard the blast pushes
    float rays = ex_noise(ang * 9.0 + 3.0) * 0.6 + ex_noise(ang * 31.0) * 0.4;

    vec3 base;
    float alpha;
    if (progress < 0.5) {
        float k = smoothstep(0.05, 0.5, progress);
        float amt = blast * k * k * (0.35 + 0.5 * rays);
        // the picture is also blown outward (magnified from the centre)
        vec2 suv = c + (uv - c) / (1.0 + 0.25 * blast * k * k);
        base = ex_blur(suv, c, amt, jit, true);
        alpha = getFromColor(uv).a;
    } else {
        float k = 1.0 - smoothstep(0.5, 0.95, progress);
        float amt = blast * k * k * (0.25 + 0.4 * rays);
        vec2 suv = c + (uv - c) / (1.0 + 0.12 * blast * k * k);
        base = ex_blur(suv, c, amt, jit, false);
        alpha = getToColor(uv).a;
    }

    // flash: a hot core that swells, plus a burning shock ring, plus a whiteout
    float rise = smoothstep(0.08, 0.5, progress);
    float fall = 1.0 - smoothstep(0.5, 0.9, progress);
    float env = rise * fall;
    float coreR = mix(0.02, 1.4, smoothstep(0.1, 0.5, progress)) * (0.85 + 0.3 * rays);
    float core = exp(-pow(r / max(coreR, 1e-3), 2.0) * 2.0);
    vec3 coreCol = mix(vec3(1.0, 0.45, 0.1), vec3(1.0, 0.97, 0.9), core);

    float ringR = mix(0.0, 1.6, smoothstep(0.1, 0.75, progress));
    float ring = exp(-pow((r - ringR) / 0.08, 2.0)) * (0.6 + 0.6 * rays);

    // whiteout covers the cut between the two pictures
    float white = smoothstep(0.36, 0.5, progress) * (1.0 - smoothstep(0.5, 0.68, progress));

    vec3 col = base;
    col += coreCol * core * env * fall * 1.6 * flash;
    col += vec3(1.0, 0.55, 0.15) * ring * env * flash;
    col = mix(col, mix(vec3(1.0, 0.97, 0.9), vec3(1.0, 0.72, 0.42), smoothstep(0.2, 1.1, r)), white);
    // the incoming picture comes back overexposed and warm, then settles
    float warm = (1.0 - smoothstep(0.5, 0.95, progress)) * step(0.5, progress) * flash;
    col = 1.0 - (1.0 - col) * (1.0 - vec3(0.4, 0.22, 0.08) * warm * warm);
    return vec4(clamp(col, 0.0, 1.0), mix(alpha, 1.0, max(white, env * core)));
}
