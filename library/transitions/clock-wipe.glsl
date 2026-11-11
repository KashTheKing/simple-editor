// @name Clock Wipe
// @desc The incoming picture is revealed by a hand sweeping clockwise from 12 o'clock around the centre.
// @tags clock, wipe, radial, sweep, angular
// @param softness 0.03 0 0.2
// @param cx 0.5 0 1
// @param cy 0.5 0 1
vec4 transition(vec2 uv) {
    vec2 pos = (uv - vec2(cx, cy)) * vec2(ratio, 1.0);
    float turn = fract(atan(pos.x, pos.y) / 6.28318531);
    float r = max(length(pos), 1e-4);
    float sf = max(softness, 1.5 / (u_res.y * 6.28318531 * r));
    float q = progress * (1.0 + 2.0 * sf) - sf;
    float m = 1.0 - smoothstep(q - sf, q + sf, turn);
    return mix(getFromColor(uv), getToColor(uv), m);
}
