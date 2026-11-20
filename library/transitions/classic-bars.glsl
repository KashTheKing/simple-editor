// @name Bars
// @desc Classic 2000s: thin bars of the incoming picture appear in random order until they fill the frame.
// @tags classic 2000s, bars, stripes, random, wipe, retro
// @param bars 48 4 200
// @param vertical 1 0 1
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    float coord = vertical > 0.5 ? uv.x : 1.0 - uv.y;
    float n = hash11(floor(coord * bars) + 0.5);
    return n < progress ? getToColor(uv) : getFromColor(uv);
}
