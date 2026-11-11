// @name Iris Diamond
// @desc A diamond-shaped iris opens from the centre and reveals the incoming picture.
// @tags iris, diamond, wipe, reveal, shape
// @param softness 0.04 0 0.3
// @param cx 0.5 0 1
// @param cy 0.5 0 1
vec4 transition(vec2 uv) {
    vec2 c = vec2(cx, cy);
    vec2 pos = abs(uv - c) * vec2(ratio, 1.0);
    vec2 far = max(c, 1.0 - c) * vec2(ratio, 1.0);
    float maxd = (far.x + far.y) * 0.70710678;
    float w = max(softness, 1.5 / u_res.y);
    float r = progress * (maxd + 2.0 * w) - w;
    float d = (pos.x + pos.y) * 0.70710678;
    float m = 1.0 - smoothstep(r - w, r + w, d);
    return mix(getFromColor(uv), getToColor(uv), m);
}
