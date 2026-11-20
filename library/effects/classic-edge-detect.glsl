// @name Edge Detect
// @desc Classic 2000s: only the outlines of the picture stay, glowing on black, or dark lines on white.
// @tags classic 2000s, edge, outline, sobel, sketch, retro
// @param strength 2 0.5 6
// @param invert 0 0 1
// @param colour 0 0 1
vec4 effect(vec4 src, vec2 uv) {
    vec2 e = u_scale / u_res;
    float tl = luma(texture(tex, uv + vec2(-e.x, -e.y)).rgb);
    float t = luma(texture(tex, uv + vec2(0.0, -e.y)).rgb);
    float tr = luma(texture(tex, uv + vec2(e.x, -e.y)).rgb);
    float l = luma(texture(tex, uv + vec2(-e.x, 0.0)).rgb);
    float r = luma(texture(tex, uv + vec2(e.x, 0.0)).rgb);
    float bl = luma(texture(tex, uv + vec2(-e.x, e.y)).rgb);
    float b = luma(texture(tex, uv + vec2(0.0, e.y)).rgb);
    float br = luma(texture(tex, uv + vec2(e.x, e.y)).rgb);
    float gx = (tr + 2.0 * r + br) - (tl + 2.0 * l + bl);
    float gy = (bl + 2.0 * b + br) - (tl + 2.0 * t + tr);
    float g = clamp(length(vec2(gx, gy)) * strength, 0.0, 1.0);
    vec3 c = mix(vec3(g), src.rgb * g * 1.5, colour);
    c = mix(c, vec3(1.0) - vec3(g), invert);
    return vec4(clamp(c, 0.0, 1.0), src.a);
}
