// @name Spin Blur
// @desc Rotational blur around a centre point, like a spinning camera or a whirlpool.
// @tags blur, spin, radial, rotate, motion
// @param center_x 0.5 0 1
// @param center_y 0.5 0 1
// @param angle 6 0 45
// @param clear 0.1 0 0.6
vec4 effect(vec4 src, vec2 uv) {
    float asp = u_res.x / u_res.y;
    vec2 c = vec2(center_x, center_y);
    vec2 v = (uv - c) * vec2(asp, 1.0);
    float r = length(v);
    float total = radians(angle);
    // keep the area right at the centre sharp
    total *= smoothstep(clear * 0.5, clear * 0.5 + 0.3, r);
    vec3 acc = vec3(0.0);
    for (int i = 0; i < 28; i++) {
        float f = float(i) / 27.0 - 0.5;
        float a = f * total;
        float cs = cos(a), sn = sin(a);
        vec2 q = vec2(v.x * cs - v.y * sn, v.x * sn + v.y * cs);
        vec2 p = q / vec2(asp, 1.0) + c;
        acc += texture(tex, p).rgb;
    }
    return vec4(acc / 28.0, src.a);
}
