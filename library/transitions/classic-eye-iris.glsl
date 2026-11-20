// @name Eye Iris
// @desc Classic 2000s: an eye-shaped opening widens from a line across the middle until the incoming picture fills the frame.
// @tags classic 2000s, iris, eye, open, shape, retro
// @param softness 0.01 0 0.1
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    vec2 p = (uv - 0.5) * vec2(ratio, 1.0);
    float w = max(softness, 1.5 / u_res.y);
    float a = mix(-w, 0.6 + 0.4 * ratio, progress);
    float d = abs(p.y) + 0.35 * p.x * p.x / max(a, 0.01) - a;
    float m = 1.0 - smoothstep(-w, w, d);
    return mix(getFromColor(uv), getToColor(uv), m);
}
