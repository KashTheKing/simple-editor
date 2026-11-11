// @name Gradient Map
// @desc Recolours the picture with a three-colour gradient by brightness: shadows, midtones, highlights.
// @tags gradient map, color, grade, stylize, sunset
// @param shadow_hue 0.7 0 1
// @param mid_hue 0.9 0 1
// @param light_hue 0.12 0 1
// @param sat 0.85 0 1
// @param amount 1 0 1
vec4 effect(vec4 src, vec2 uv) {
    float l = clamp((luma(src.rgb) - 0.08) / 0.8, 0.0, 1.0);
    vec3 a = hsl2rgb(vec3(shadow_hue, sat, 0.12));
    vec3 b = hsl2rgb(vec3(mid_hue, sat, 0.5));
    vec3 c = hsl2rgb(vec3(light_hue, sat, 0.82));
    vec3 g = l < 0.5 ? mix(a, b, smoothstep(0.0, 0.5, l)) : mix(b, c, smoothstep(0.5, 1.0, l));
    return vec4(mix(src.rgb, g, amount), src.a);
}
