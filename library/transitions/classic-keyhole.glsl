// @name Keyhole
// @desc Classic 2000s: a keyhole opens in the middle of the frame and grows until the incoming picture fills it.
// @tags classic 2000s, keyhole, iris, shape, spy, retro
// @param softness 0.01 0 0.1
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    float r = progress * progress * 4.5 * max(ratio, 1.0);
    vec2 p = (uv - 0.5) * vec2(ratio, 1.0);
    float dc = length(p - vec2(0.0, 0.18 * r)) - 0.3 * r;
    float top = 0.18 * r;
    float bot = -0.6 * r;
    float f = clamp((p.y - bot) / max(top - bot, 0.0001), 0.0, 1.0);
    float hw = mix(0.26 * r, 0.1 * r, f);
    float dt = max(abs(p.x) - hw, max(p.y - top, bot - p.y));
    float d = min(dc, dt);
    float w = max(softness, 1.5 / u_res.y);
    float m = 1.0 - smoothstep(-w, w, d);
    return mix(getFromColor(uv), getToColor(uv), m);
}
