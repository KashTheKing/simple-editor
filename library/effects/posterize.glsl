// @name Posterize
// @desc Flattens the picture into a few bands of colour, like a screen print.
// @tags posterize, bands, levels, poster, flat, pop art
// @param levels 5 2 32
// @param amount 1 0 1
vec4 effect(vec4 src, vec2 uv) {
    float n = max(levels - 1.0, 1.0);
    vec3 c = floor(src.rgb * n + 0.5) / n;
    return vec4(mix(src.rgb, c, amount), src.a);
}
