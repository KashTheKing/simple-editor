// @name Bow Tie
// @desc Classic 2000s: two wedges close in from the left and right edges like a bow tie and meet in the middle.
// @tags classic 2000s, bow tie, wedge, triangle, wipe, retro
// @param slope 1 0 3
// @param softness 0.005 0 0.1
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    float d = 0.5 - abs(uv.x - 0.5) + slope * abs(uv.y - 0.5) / ratio;
    float w = max(softness, 1.5 / u_res.x);
    float q = mix(-w, 0.5 + 0.5 * slope / ratio + w, progress);
    float m = 1.0 - smoothstep(q - w, q + w, d);
    return mix(getFromColor(uv), getToColor(uv), m);
}
