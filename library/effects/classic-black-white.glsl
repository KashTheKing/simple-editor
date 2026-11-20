// @name Black and White
// @desc Classic 2000s: a plain greyscale picture, with a contrast control.
// @tags classic 2000s, black and white, greyscale, grayscale, mono, retro
// @param contrast 1 0.5 2
// @param brightness 0 -0.5 0.5
vec4 effect(vec4 src, vec2 uv) {
    float y = luma(src.rgb);
    y = clamp((y - 0.5) * contrast + 0.5 + brightness, 0.0, 1.0);
    return vec4(vec3(y), src.a);
}
