// @name Spiral
// @desc Classic 2000s: the incoming picture is uncovered along a spiral that winds out from the centre.
// @tags classic 2000s, spiral, swirl, radial, wipe, retro
// @param turns 3 1 10
// @param softness 0.02 0 0.2
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    vec2 p = (uv - 0.5) * vec2(ratio, 1.0);
    float rmax = 0.5 * length(vec2(ratio, 1.0));
    float a01 = (atan(p.y, p.x) + 3.14159265) / 6.2831853;
    float s = (length(p) / rmax * turns + a01) / (turns + 1.0);
    float w = max(softness, 0.002);
    float q = mix(-w, 1.0 + w, progress);
    float m = 1.0 - smoothstep(q - w, q + w, s);
    return mix(getFromColor(uv), getToColor(uv), m);
}
