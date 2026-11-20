// @name Vertigo
// @desc Dizzy spinning feedback: each frame is laid over a slightly rotated and zoomed copy of the frames before it.
// @tags vertigo, spin, feedback, trippy, dizzy, psychedelic, echo
// @param amount 0.6 0 0.95
// @param spin 3 -20 20
// @param zoom 1.03 0.9 1.15
// @needs previous 2
vec2 vt_turn(vec2 uv, float deg, float z) {
    vec2 p = (uv - 0.5) * vec2(u_res.x / u_res.y, 1.0);
    float a = radians(deg);
    p = mat2(cos(a), -sin(a), sin(a), cos(a)) * p / z;
    return p / vec2(u_res.x / u_res.y, 1.0) + 0.5;
}
vec4 effect(vec4 src, vec2 uv) {
    if (u_frames < 0.5) { return src; }
    // the half-way frame turned once, the older one twice: a cheap stand-in for a feedback buffer
    vec3 a = texture(u_next, clamp(vt_turn(uv, spin, zoom), 0.0, 1.0)).rgb;
    vec3 b = texture(u_prev, clamp(vt_turn(uv, spin * 2.0, zoom * zoom), 0.0, 1.0)).rgb;
    vec3 trail = mix(a, b, 0.4);
    return vec4(mix(src.rgb, trail, amount * 0.85), src.a);
}
