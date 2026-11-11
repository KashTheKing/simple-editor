// @name Zoom Blur
// @desc The outgoing picture zooms in with radial blur, swaps at the peak, and the incoming picture settles from a zoomed, blurred state.
// @tags zoom, blur, radial, punch
// @param zoom 0.6 0 2
// @param blur 0.5 0 0.8
vec4 zb_side(float which, vec2 uv, float e) {
    vec2 c = vec2(0.5);
    float z = 1.0 + zoom * e;
    float bl = blur * e;
    float jit = hash12(uv * u_res);
    vec4 acc = vec4(0.0);
    for (int i = 0; i < 20; i++) {
        float t = (float(i) + jit) / 20.0;
        vec2 q = c + (uv - c) * (1.0 - bl * t) / z;
        acc += which < 0.5 ? getFromColor(q) : getToColor(q);
    }
    return acc / 20.0;
}

vec4 transition(vec2 uv) {
    float s1 = clamp(progress * 2.0, 0.0, 1.0);
    float s2 = clamp((1.0 - progress) * 2.0, 0.0, 1.0);
    float m = smoothstep(0.45, 0.55, progress);
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    vec4 a = vec4(0.0);
    vec4 b = vec4(0.0);
    if (m < 1.0) a = zb_side(0.0, uv, s1 * s1);
    if (m > 0.0) b = zb_side(1.0, uv, s2 * s2);
    return mix(a, b, m);
}
