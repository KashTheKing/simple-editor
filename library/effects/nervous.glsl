// @name Nervous
// @desc Jittery time: the picture keeps jumping between the current frame and the last few.
// @tags nervous, jitter, stutter, glitch, time, frames
// @param rate 12 1 30
// @param amount 1 0 1
// @needs previous 4
vec4 effect(vec4 src, vec2 uv) {
    if (u_frames < 0.5) { return src; }
    float r = hash11(floor(u_time * rate) + 0.37);
    if (r > amount) { return src; }
    float pick = hash11(floor(u_time * rate) * 1.7 + 4.1);
    vec4 c = (pick < 0.5) ? texture(u_next, uv) : texture(u_prev, uv);
    return vec4(c.rgb, src.a);
}
