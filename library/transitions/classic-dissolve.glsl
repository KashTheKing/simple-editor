// @name Pixel Dissolve
// @desc Classic 2000s: the incoming picture appears a few random pixels at a time until it fills the frame.
// @tags classic 2000s, dissolve, pixels, random, retro
// @param grain 1 1 12
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    float px = max(grain * u_scale, 1.0);
    float n = hash12(floor(uv * u_res / px));
    return n < progress ? getToColor(uv) : getFromColor(uv);
}
