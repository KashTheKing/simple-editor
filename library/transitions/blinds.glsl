// @name Blinds
// @desc Venetian blinds: a row of slats each opens to reveal the incoming picture, with a soft edge.
// @tags blinds, slats, venetian, wipe, stripes
// @param slats 12 2 60
// @param angle 0 0 180
// @param softness 0.15 0 0.5
vec4 transition(vec2 uv) {
    float a = radians(angle);
    float c = cos(a);
    float s = sin(a);
    vec2 pos = (uv - 0.5) * vec2(ratio, 1.0);
    float v = -s * pos.x + c * pos.y;
    float span = ratio * abs(s) + abs(c);
    float f = fract((v / span + 0.5) * slats);
    float sf = max(softness, 1.5 * slats / (u_res.y * span));
    float q = progress * (1.0 + 2.0 * sf) - sf;
    float m = 1.0 - smoothstep(q - sf, q + sf, f);
    return mix(getFromColor(uv), getToColor(uv), m);
}
