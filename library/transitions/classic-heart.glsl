// @name Heart
// @desc Classic 2000s: a heart opens from the centre and grows until the incoming picture fills the frame.
// @tags classic 2000s, heart, iris, shape, love, wedding, retro
// @param softness 0.03 0 0.2
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    float r = progress * 1.7 * max(ratio, 1.0);
    vec2 p = (uv - vec2(0.5, 0.45)) * vec2(ratio, 1.0) / max(r, 0.0001);
    float a = dot(p, p) - 1.0;
    float f = a * a * a - p.x * p.x * p.y * p.y * p.y;
    float w = max(softness, 0.002);
    float m = 1.0 - smoothstep(-w, w, f);
    return mix(getFromColor(uv), getToColor(uv), m);
}
