// @name Diagonal Wipe
// @desc Classic 2000s: a straight diagonal edge sweeps from one corner to the opposite one.
// @tags classic 2000s, diagonal, wipe, corner, retro
// @param corner 0 0 3
// @param softness 0.02 0 0.3
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    vec2 p = uv;
    float k = floor(corner + 0.5);
    if (k == 1.0 || k == 2.0) p.x = 1.0 - p.x;
    if (k >= 2.0) p.y = 1.0 - p.y;
    float d = (p.x * ratio + (1.0 - p.y)) / (ratio + 1.0);
    float w = max(softness, 1.5 / u_res.y);
    float q = mix(-w, 1.0 + w, progress);
    float m = 1.0 - smoothstep(q - w, q + w, d);
    return mix(getFromColor(uv), getToColor(uv), m);
}
