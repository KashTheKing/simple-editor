// @name Emboss
// @desc Raises the picture's edges like a stamped metal or paper relief.
// @tags emboss, relief, stamp, metal, edges, 3d
// @param angle 135 0 360
// @param depth 2 0.5 8
// @param color_amount 0 0 1
vec4 effect(vec4 src, vec2 uv) {
    float a = radians(angle);
    vec2 d = vec2(cos(a), -sin(a)) * depth * u_scale / u_res;
    float hi = luma(texture(tex, clamp(uv - d, 0.0, 1.0)).rgb);
    float lo = luma(texture(tex, clamp(uv + d, 0.0, 1.0)).rgb);
    float e = clamp(0.5 + (hi - lo) * 2.0, 0.0, 1.0);
    vec3 c = mix(vec3(e), src.rgb * e * 2.0, color_amount);
    return vec4(clamp(c, 0.0, 1.0), src.a);
}
