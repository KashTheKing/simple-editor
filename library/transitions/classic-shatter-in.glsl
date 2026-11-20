// @name Shatter In
// @desc Classic 2000s: the incoming picture flies together from spinning triangle shards that grow into place.
// @tags classic 2000s, shatter, shards, glass, pieces, retro
// @param pieces 7 2 20
// @param spin 2 0 6
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    vec2 cs = vec2(1.0 / (pieces * ratio), 1.0 / pieces);
    vec2 g = uv / cs;
    vec2 cell = floor(g);
    vec2 f = fract(g);
    float flipd = step(0.5, hash12(cell + 7.1));
    float fx = flipd > 0.5 ? 1.0 - f.x : f.x;
    float upper = step(fx, f.y);
    vec2 c0 = upper > 0.5 ? vec2(1.0 / 3.0, 2.0 / 3.0) : vec2(2.0 / 3.0, 1.0 / 3.0);
    if (flipd > 0.5) c0.x = 1.0 - c0.x;
    float id = hash12(cell * 1.7 + upper * 13.3);
    float delay = id * 0.5;
    float lp = clamp((progress - delay) / 0.5, 0.0, 1.0);
    if (lp <= 0.0) return getFromColor(uv);
    float ang = (1.0 - lp) * spin * (id - 0.5) * 6.2831853;
    float sc = lp * lp * (3.0 - 2.0 * lp);
    vec2 d = f - c0;
    float co = cos(ang);
    float si = sin(ang);
    d = vec2(co * d.x + si * d.y, -si * d.x + co * d.y) / max(sc, 0.0001);
    vec2 fq = c0 + d;
    float fqx = flipd > 0.5 ? 1.0 - fq.x : fq.x;
    bool inside = fq.x >= 0.0 && fq.x <= 1.0 && fq.y >= 0.0 && fq.y <= 1.0 && step(fqx, fq.y) == upper;
    if (!inside) return getFromColor(uv);
    return getToColor((cell + fq) * cs);
}
