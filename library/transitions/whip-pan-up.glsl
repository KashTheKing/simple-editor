// @name Whip Pan Up
// @desc A fast camera whip to the up: both pictures slide with strong directional motion blur that peaks mid-move and lands softly.
// @tags whip, pan, blur, slide, camera, up
// @param blur 0.3 0 0.6
vec4 transition(vec2 uv) {
    float p = progress;
    float o = p * p * p * (p * (p * 6.0 - 15.0) + 10.0);
    float vel = 16.0 * p * p * (1.0 - p) * (1.0 - p);
    float len = blur * vel;
    float a = false ? uv.x : uv.y;
    float m0 = -1.0 > 0.0 ? a : 1.0 - a;
    float jit = hash12(uv * u_res);
    vec4 acc = vec4(0.0);
    for (int i = 0; i < 24; i++) {
        float t = (float(i) + jit) / 24.0 - 0.5;
        float m = m0 - o + t * len;
        float mm = min(m >= 0.0 ? m : m + 1.0, 1.0);
        float aa = -1.0 > 0.0 ? mm : 1.0 - mm;
        vec2 q = false ? vec2(aa, uv.y) : vec2(uv.x, aa);
        acc += m >= 0.0 ? getFromColor(q) : getToColor(q);
    }
    return acc / 24.0;
}
