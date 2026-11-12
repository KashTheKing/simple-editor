// @name Iris Circle
// @desc A circular iris opens from the centre and reveals the incoming picture.
// @tags iris, circle, wipe, reveal, round
// @param softness 0.04 0 0.3
// @param cx 0.5 0 1
// @param cy 0.5 0 1
vec4 transition(vec2 uv) {
    vec2 c = vec2(cx, cy);
    vec2 pos = (uv - c) * vec2(ratio, 1.0);
    vec2 far = max(c, 1.0 - c) * vec2(ratio, 1.0);
    float maxr = length(far);
    float w = max(softness, 1.5 / u_res.y);
    float r = progress * (maxr + 2.0 * w) - w;
    float m = 1.0 - smoothstep(r - w, r + w, length(pos));
    return mix(getFromColor(uv), getToColor(uv), m);
}
