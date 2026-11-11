// @name Shockwave
// @desc A refracting ring bursts out from the centre, bending the picture with a chromatic fringe and leaving the incoming picture inside it.
// @tags shockwave, ring, refraction, lens, ripple, impact
// @param center_x 0.5 0 1
// @param center_y 0.5 0 1
// @param ring_width 0.14 0.03 0.4
// @param strength 1 0 3
// @param chroma 1 0 3

vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);

    vec2 asp = vec2(ratio, 1.0);
    vec2 c = vec2(center_x, center_y);
    vec2 dv = (uv - c) * asp;
    float r = length(dv);
    vec2 dirn = dv / max(r, 1e-5);

    // farthest corner
    vec2 far = max(c, 1.0 - c) * asp;
    float maxD = length(far);
    float w = ring_width;
    // a slight ease out: the wave is quick at first and slows as it spreads
    float e = 1.0 - pow(1.0 - progress, 1.15);
    float ringR = mix(-w, maxD + w, e);

    float x = (r - ringR) / w;          // -1..1 across the ring
    float bump = pow(max(1.0 - x * x, 0.0), 2.0);
    // lens: inside the ring pixels are pulled from further out (magnify), and the slope bends them
    float disp = strength * 0.045 * bump * (x * 1.6 + 0.6);
    float ch = chroma * 0.012 * bump * strength;

    vec2 o = dirn / asp;                // back to uv units
    vec2 uvR = uv - o * (disp + ch);
    vec2 uvG = uv - o * disp;
    vec2 uvB = uv - o * (disp - ch);

    // inner half of the ring shows the incoming picture, outer half the outgoing one
    float m = smoothstep(-0.35, 0.35, x);

    vec4 col;
    if (x <= -1.0) {
        col = getToColor(uv);
    } else if (x >= 1.0) {
        col = getFromColor(uv);
    } else {
        vec4 tG = getToColor(uvG);
        vec4 fG = getFromColor(uvG);
        vec3 t = vec3(getToColor(uvR).r, tG.g, getToColor(uvB).b);
        vec3 f = vec3(getFromColor(uvR).r, fG.g, getFromColor(uvB).b);
        col = vec4(mix(t, f, m), mix(tG.a, fG.a, m));
        // a thin bright crest and a darker trough give the ring body
        float crest = exp(-pow((x + 0.15) / 0.18, 2.0)) * bump;
        float trough = exp(-pow((x - 0.6) / 0.25, 2.0)) * bump;
        col.rgb += vec3(0.9, 0.95, 1.0) * 0.18 * crest * strength;
        col.rgb *= 1.0 - 0.18 * trough * min(strength, 1.5);
    }

    return col;
}
