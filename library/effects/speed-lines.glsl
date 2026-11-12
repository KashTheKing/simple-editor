// @name Speed Lines
// @desc Anime-style streaks racing in from the edges toward a focus point, flickering like hand-drawn frames.
// @tags anime, manga, speed, action, lines, comic
// @param focus_x 0.5 0 1
// @param focus_y 0.5 0 1
// @param density 1.0 0.3 2.5
// @param clear 0.4 0.1 0.9
// @param opacity 0.9 0 1
// @param dark 0 0 1
vec4 effect(vec4 src, vec2 uv) {
    float asp = u_res.x / u_res.y;
    vec2 v = (uv - vec2(focus_x, focus_y)) * vec2(asp, 1.0);
    float r = length(v);
    float ang = atan(v.y, v.x);
    float tt = floor(u_time * 12.0);          // stepped: reads like hand-drawn frames
    float lines = 0.0;
    for (int k = 0; k < 2; k++) {
        float fk = float(k);
        float freq = (80.0 + 110.0 * fk) * density;
        float x = ang / 6.2831853 * freq;
        float id = floor(x);
        float seed = id + tt * 37.0 + fk * 101.0;
        float n = hash11(seed + 3.0);
        float thick = mix(0.06, 0.28, hash11(seed + 9.0)) * (1.0 - 0.45 * fk);
        float f = abs(fract(x) - 0.5);
        // streaks get thicker away from the focus
        float taper = smoothstep(clear, clear + 0.7, r);
        float w = thick * (0.25 + 0.75 * taper);
        float line = 1.0 - smoothstep(w - 0.06, w + 0.06, f);
        float start = clear + 0.2 * hash11(seed + 21.0);
        float gate = smoothstep(start, start + 0.15, r);
        line *= gate * step(0.3, n);
        lines = max(lines, line * (1.0 - 0.3 * fk));
    }
    lines *= opacity;
    vec3 ink = mix(vec3(1.0), vec3(0.02), step(0.5, dark));
    vec3 col = mix(src.rgb, ink, lines);
    return vec4(col, src.a);
}
