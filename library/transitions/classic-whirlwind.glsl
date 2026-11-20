// @name Whirlwind
// @desc Classic 2000s: the outgoing picture twists into a whirl and the incoming one untwists out of it.
// @tags classic 2000s, whirlwind, twist, swirl, spin, retro
// @param strength 6 0 20
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    vec2 p = (uv - 0.5) * vec2(ratio, 1.0);
    float r = length(p);
    float k = sin(progress * 3.14159265);
    float ang = k * strength * (1.0 - smoothstep(0.0, 0.5 * length(vec2(ratio, 1.0)), r));
    float c = cos(ang);
    float s = sin(ang);
    vec2 pa = vec2(c * p.x - s * p.y, s * p.x + c * p.y) / vec2(ratio, 1.0) + 0.5;
    vec2 pb = vec2(c * p.x + s * p.y, -s * p.x + c * p.y) / vec2(ratio, 1.0) + 0.5;
    vec4 a = getFromColor(clamp(pa, 0.0, 1.0));
    vec4 b = getToColor(clamp(pb, 0.0, 1.0));
    return mix(a, b, smoothstep(0.3, 0.7, progress));
}
