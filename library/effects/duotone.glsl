// @name Duotone
// @desc Maps the picture to two colours by brightness: one for shadows, one for highlights.
// @tags duotone, color, stylize, two-tone, poster
// @param shadow_hue 0.72 0 1
// @param shadow_sat 0.85 0 1
// @param light_hue 0.05 0 1
// @param light_sat 0.9 0 1
// @param contrast 1.4 0.5 2
vec4 effect(vec4 src, vec2 uv) {
    float l = luma(src.rgb);
    l = clamp((l - 0.5) * contrast + 0.5, 0.0, 1.0);
    vec3 sh = hsl2rgb(vec3(shadow_hue, shadow_sat, 0.14));
    vec3 hi = hsl2rgb(vec3(light_hue, light_sat, 0.78));
    vec3 col = mix(sh, hi, smoothstep(0.0, 1.0, l));
    return vec4(col, src.a);
}
