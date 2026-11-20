// @name Alpha Strobe
// @desc Blinks the clip on and off, so whatever is below shows through on the off beats.
// @tags strobe, blink, flash, alpha, transparency, flicker
// @param rate 4 0.25 30
// @param duty 0.5 0.05 0.95
// @param low 0 0 1
// @param invert 0 0 1
vec4 effect(vec4 src, vec2 uv) {
    bool on = fract(u_time * rate) < duty;
    if (invert > 0.5) { on = !on; }
    return vec4(src.rgb, src.a * (on ? 1.0 : low));
}
