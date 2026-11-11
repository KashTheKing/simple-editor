// @name Cut-out Shadow
// @desc Soft drop shadow behind a cut-out subject, with angle, distance, softness and opacity.
// @tags alpha, cutout, shadow, drop shadow, depth
// @param angle 45 0 360
// @param dist 28 0 100
// @param softness 24 0 60
// @param opacity 0.7 0 1
float shadow_a(vec2 uv) {
    if (uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0) return 0.0;
    return texture(tex, uv).a;
}
vec4 effect(vec4 src, vec2 uv) {
    float rad = angle * 0.01745329;
    vec2 off = vec2(cos(rad), sin(rad)) * dist * u_scale / u_res;
    vec2 so = uv - off;
    float R = softness * u_scale;
    float rot = hash12(floor(uv * u_res)) * 6.2831853;
    float acc = 0.0;
    float wsum = 0.0;
    for (int i = 0; i < 24; i++) {
        float f = (float(i) + 0.5) / 24.0;
        float r = sqrt(f);
        float a = float(i) * 2.3999632 + rot * 0.15;
        float w = exp(-2.5 * f);
        acc += shadow_a(so + vec2(cos(a), sin(a)) * r * R / u_res) * w;
        wsum += w;
    }
    float sa = acc / wsum * opacity;
    float a = src.a + sa * (1.0 - src.a);
    vec3 col = a > 1e-4 ? (src.rgb * src.a) / a : vec3(0.0);
    return vec4(col, a);
}
