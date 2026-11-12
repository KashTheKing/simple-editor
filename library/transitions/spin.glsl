// @name Spin
// @desc Both pictures rotate around the centre with rotational blur and swap at the midpoint.
// @tags spin, rotate, blur, twist
// @param angle 150 0 360
// @param zoom 0.6 0 2
// @param blur 0.8 0 2
vec2 sp_mirror(vec2 x) {
    return 1.0 - abs(1.0 - mod(x, 2.0));
}

vec4 sp_side(float which, vec2 uv, float s) {
    float ang = radians(angle) * s * s * (which < 0.5 ? 1.0 : -1.0);
    float z = 1.0 + zoom * s * s;
    vec2 pos = (uv - 0.5) * vec2(ratio, 1.0);
    float jit = hash12(uv * u_res);
    vec4 acc = vec4(0.0);
    for (int i = 0; i < 20; i++) {
        float t = (float(i) + jit) / 20.0 - 0.5;
        float a = ang + t * blur * s;
        float c = cos(a);
        float sn = sin(a);
        vec2 r = vec2(c * pos.x - sn * pos.y, sn * pos.x + c * pos.y) / z;
        vec2 q = sp_mirror(r / vec2(ratio, 1.0) + 0.5);
        acc += which < 0.5 ? getFromColor(q) : getToColor(q);
    }
    return acc / 20.0;
}

vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    float s1 = clamp(progress * 2.0, 0.0, 1.0);
    float s2 = clamp((1.0 - progress) * 2.0, 0.0, 1.0);
    float m = smoothstep(0.46, 0.54, progress);
    vec4 a = vec4(0.0);
    vec4 b = vec4(0.0);
    if (m < 1.0) a = sp_side(0.0, uv, s1);
    if (m > 0.0) b = sp_side(1.0, uv, s2);
    return mix(a, b, m);
}
