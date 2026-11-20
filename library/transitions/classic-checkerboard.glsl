// @name Checkerboard
// @desc Classic 2000s: a checkerboard of squares wipes across, the white squares first and the black ones half a square behind.
// @tags classic 2000s, checkerboard, squares, wipe, retro
// @param squares 8 2 30
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    vec2 g = vec2(uv.x * squares * ratio, (1.0 - uv.y) * squares);
    vec2 cell = floor(g);
    vec2 f = fract(g);
    float odd = mod(cell.x + cell.y, 2.0);
    float p = clamp(progress * 2.0 - odd, 0.0, 1.0);
    return f.x < p ? getToColor(uv) : getFromColor(uv);
}
