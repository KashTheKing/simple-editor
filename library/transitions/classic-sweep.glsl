// @name Sweep
// @desc Classic 2000s: an edge swings around a corner like a fan, sweeping the incoming picture in.
// @tags classic 2000s, sweep, fan, radial, wipe, retro
// @param corner 0 0 3
// @param softness 0.02 0 0.2
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    vec2 p = uv;
    float k = floor(corner + 0.5);
    if (k == 1.0 || k == 2.0) p.x = 1.0 - p.x;
    if (k >= 2.0) p.y = 1.0 - p.y;
    p *= vec2(ratio, 1.0);
    float ang = atan(p.y, max(p.x, 0.00001)) / 1.5707963;
    float w = max(softness, 0.002);
    float q = mix(-w, 1.0 + w, progress);
    float m = 1.0 - smoothstep(q - w, q + w, ang);
    return mix(getFromColor(uv), getToColor(uv), m);
}
