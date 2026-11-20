// @name Heart
// @desc Classic 2000s: a heart opens from the centre and grows until the incoming picture fills the frame.
// @tags classic 2000s, heart, iris, shape, love, wedding, retro
// @param softness 0.004 0 0.05
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    float r = pow(progress, 1.5) * 1.8 * max(ratio, 1.0);
    vec2 p = (uv - vec2(0.5, 0.45)) * vec2(ratio, 1.0) / max(r, 0.0001);
    float a = dot(p, p) - 1.0;
    float f = a * a * a - p.x * p.x * p.y * p.y * p.y;
    // f is not a distance: divide by its slope so the edge is as soft on screen at any size
    vec2 g = vec2(6.0 * p.x * a * a - 2.0 * p.x * p.y * p.y * p.y, 6.0 * p.y * a * a - 3.0 * p.x * p.x * p.y * p.y);
    float d = f / max(length(g), 0.0001) * r;
    float w = max(softness, 1.5 / u_res.y);
    float m = 1.0 - smoothstep(-w, w, d);
    return mix(getFromColor(uv), getToColor(uv), m);
}
