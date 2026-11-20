// @name Flip
// @desc Classic 2000s: the picture turns over like a card on its vertical axis, showing the incoming picture on its back.
// @tags classic 2000s, flip, card, turn, 3d, retro
// @param depth 0.2 0 0.6
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    float a = progress * 3.14159265;
    float sx = abs(cos(a));
    float side = progress < 0.5 ? 1.0 : -1.0;
    float x = (uv.x - 0.5) / max(sx, 0.0001);
    float grow = 1.0 + depth * sin(a) * x * side;
    vec2 q = vec2(x + 0.5, (uv.y - 0.5) / max(grow, 0.0001) + 0.5);
    if (q.x < 0.0 || q.x > 1.0 || q.y < 0.0 || q.y > 1.0) return vec4(0.0, 0.0, 0.0, 1.0);
    return progress < 0.5 ? getFromColor(q) : getToColor(q);
}
