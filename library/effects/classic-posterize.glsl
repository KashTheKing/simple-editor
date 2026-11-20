// @name Posterize
// @desc Classic 2000s: cuts the picture down to a few flat levels of each colour, like a cheap print.
// @tags classic 2000s, posterize, poster, levels, flat, retro
// @param levels 4 2 16
vec4 effect(vec4 src, vec2 uv) {
    float n = max(floor(levels + 0.5) - 1.0, 1.0);
    return vec4(floor(src.rgb * n + 0.5) / n, src.a);
}
