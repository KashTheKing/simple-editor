// @name Fade Through White
// @desc Classic 2000s: the outgoing picture fades up to white, then the incoming one fades in from it. On a clip edge it is a fade in or out from white.
// @tags classic 2000s, fade, white, dip, flash, retro
// @param hold 0.1 0 0.8
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    vec4 c = progress < 0.5 ? getFromColor(uv) : getToColor(uv);
    float k = clamp((1.0 - abs(progress * 2.0 - 1.0)) / max(1.0 - hold, 0.001), 0.0, 1.0);
    return vec4(mix(c.rgb, vec3(1.0), k), mix(c.a, 1.0, k));
}
