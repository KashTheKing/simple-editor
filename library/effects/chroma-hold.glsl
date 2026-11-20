// @name Chroma Hold
// @desc Keeps one colour and turns the rest of the picture black and white.
// @tags chroma hold, color pop, color splash, selective color, black and white
// @param hue 0 0 1
// @param tolerance 0.08 0 0.5
// @param softness 0.06 0 0.3
// @param min_sat 0.2 0 1
// @param amount 1 0 1
vec4 effect(vec4 src, vec2 uv) {
    vec3 hsl = rgb2hsl(src.rgb);
    float dh = abs(fract(hsl.x - hue + 0.5) - 0.5);
    float keep = 1.0 - smoothstep(tolerance, tolerance + softness + 1e-4, dh);
    keep *= smoothstep(min_sat * 0.5, min_sat + 1e-4, hsl.y);
    vec3 grey = vec3(luma(src.rgb));
    return vec4(mix(src.rgb, mix(grey, src.rgb, keep), amount), src.a);
}
