// @name Burn Through
// @desc Holes burn through the outgoing picture with charred edges and a glowing ember rim.
// @tags burn, fire, ember, char, paper
// @param scale 2.5 0.5 8
// @param ember_width 1 0.2 3
// @param glow 1 0 2
// @param char_width 1 0 3

float bt_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    float a = hash12(i);
    float b = hash12(i + vec2(1.0, 0.0));
    float c = hash12(i + vec2(0.0, 1.0));
    float d = hash12(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float bt_fbm(vec2 p) {
    float s = 0.0;
    float a = 0.5;
    for (int i = 0; i < 5; i++) {
        s += a * bt_noise(p);
        p = mat2(1.6, 1.2, -1.2, 1.6) * p + vec2(5.2, 1.3);
        a *= 0.5;
    }
    return s / 0.96875;
}

vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);

    vec2 p = uv * vec2(ratio, 1.0);
    float n = bt_fbm(p * scale);
    // a fine grain so the edge crackles instead of being smooth
    float grain = bt_noise(p * scale * 14.0);
    n = n * 0.96 + grain * 0.04;
    // spread the bell-shaped fbm out so the burn advances evenly
    n = 0.5 + (n - 0.5) * 1.7;

    float ew = 0.018 * ember_width;
    float cw = 0.05 * char_width;
    // burn front: n below t is gone
    float t = mix(-0.1 - ew - cw, 1.12, progress);
    float d = n - t;

    // pixel-sized smoothing in noise units
    float aa = fwidth(n) * 1.2 + 1e-4;

    vec4 outg = getFromColor(uv);
    vec4 inc = getToColor(uv);

    // charring: the paper browns, then blackens toward the edge
    float charK = 1.0 - smoothstep(ew, ew + cw, d);
    vec3 brown = outg.rgb * vec3(0.55, 0.38, 0.22);
    vec3 col = mix(outg.rgb, brown, smoothstep(0.0, 0.6, charK));
    col = mix(col, vec3(0.04, 0.02, 0.01), smoothstep(0.55, 1.0, charK));

    // ember rim: white-yellow at the hole edge, orange, then deep red into the char
    float e = clamp(1.0 - d / ew, 0.0, 1.0);    // 1 at the hole edge, 0 at the char
    float flick = 0.7 + 0.6 * bt_noise(p * scale * 22.0 + progress * 6.0);
    vec3 ember = mix(vec3(0.6, 0.05, 0.0), vec3(1.0, 0.45, 0.05), smoothstep(0.0, 0.6, e));
    ember = mix(ember, vec3(1.0, 0.92, 0.6), smoothstep(0.75, 1.0, e));
    float emberA = smoothstep(0.0, 0.5, e) * step(0.0, d);
    col = mix(col, ember * flick, emberA * smoothstep(-aa, aa, d));

    // the hole itself
    float hole = 1.0 - smoothstep(-aa, aa, d);
    float alpha = mix(outg.a, inc.a, hole);
    vec3 below = inc.rgb;
    // a little heat glow spills onto the picture behind the hole
    float spill = exp(min(d, 0.0) * 25.0) * hole;
    below += vec3(1.0, 0.35, 0.05) * 0.35 * glow * spill;
    col = mix(col, below, hole);

    // soft bloom around the ember line on both sides
    float env = smoothstep(0.0, 0.08, progress) * (1.0 - smoothstep(0.92, 1.0, progress));
    float bloom = exp(-abs(d - ew * 0.3) * 18.0 / ember_width) * smoothstep(-0.04, 0.0, d);
    col += vec3(1.0, 0.4, 0.08) * bloom * 0.35 * glow * env;

    return vec4(col, alpha);
}
