// @name Roll
// @desc Classic 2000s: the outgoing picture rolls away around its bottom-left corner, uncovering the incoming one.
// @tags classic 2000s, roll, rotate, swing, retro
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    float a = progress * 1.5707963;
    vec2 p = uv * vec2(ratio, 1.0);
    float c = cos(a);
    float s = sin(a);
    vec2 q = vec2(c * p.x - s * p.y, s * p.x + c * p.y) / vec2(ratio, 1.0);
    if (q.x >= 0.0 && q.x <= 1.0 && q.y >= 0.0 && q.y <= 1.0) return getFromColor(q);
    return getToColor(uv);
}
