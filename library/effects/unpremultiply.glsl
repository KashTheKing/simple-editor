// @name Unpremultiply Alpha
// @desc Fixes dark fringes on clips whose colours were stored premultiplied by their transparency.
// @tags alpha, unpremultiply, premultiplied, fringe, transparency, fix
// @param amount 1 0 1
vec4 effect(vec4 src, vec2 uv) {
    vec3 c = (src.a > 1e-4) ? clamp(src.rgb / src.a, 0.0, 1.0) : src.rgb;
    return vec4(mix(src.rgb, c, amount), src.a);
}
