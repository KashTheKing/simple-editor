// @name Film Projector
// @desc An old projector slipping out of frame: the picture jumps and rolls, with the dark frame line between frames.
// @tags projector, film, old film, frame slip, vintage, jitter, roll
// @param slip 0.4 0 1
// @param jitter 0.3 0 1
// @param flicker 0.4 0 1
// @param bar 0.06 0 0.2
vec4 effect(vec4 src, vec2 uv) {
    float t = u_time;
    // occasional slow roll, plus constant small gate jitter
    float roll = slip * 0.5 * max(sin(t * 0.7) * sin(t * 0.31 + 1.0), 0.0) * fract(t * 0.5);
    float jit = jitter * 0.004 * (hash11(floor(t * 24.0)) - 0.5);
    float y = fract(uv.y + roll + jit);
    vec4 c = texture(tex, vec2(uv.x, y));
    // the dark line between frames, at the wrap
    float edge = min(y, 1.0 - y);
    float line = smoothstep(bar * 0.5, bar * 0.5 + 0.01, edge);
    if (bar <= 0.0) { line = 1.0; }
    float fl = 1.0 - flicker * 0.25 * hash11(floor(t * 24.0) + 3.0);
    return vec4(c.rgb * line * fl, src.a);
}
