// @name Choppy
// @desc Stop-motion look: the picture holds for a beat before catching up, like footage at a much lower frame rate.
// @tags choppy, stutter, stop motion, frame rate, hold, low fps
// @param fps 8 1 30
// @param amount 1 0 1
// @needs previous 4
vec4 effect(vec4 src, vec2 uv) {
    if (u_frames < 0.5) { return src; }
    // within each step the older samples show first, so motion advances in jumps
    float ph = fract(u_time * fps);
    vec4 c = (ph < 0.34) ? src : (ph < 0.67) ? texture(u_next, uv) : texture(u_prev, uv);
    return vec4(mix(src.rgb, c.rgb, amount), src.a);
}
