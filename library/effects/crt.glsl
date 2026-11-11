// @name CRT Monitor
// @desc Curved glass, scanlines, RGB phosphor mask, vignette and a faint flicker.
// @tags crt, retro, scanlines, monitor, 80s
// @param curvature 0.25 0 0.6
// @param scanlines 0.35 0 1
// @param phosphor 0.2 0 1
// @param vignette 0.5 0 1
// @param flicker 0.04 0 0.2
vec4 effect(vec4 src, vec2 uv) {
    vec2 c = uv * 2.0 - 1.0;
    vec2 w = c * (1.0 + curvature * 0.25 * dot(c, c) * vec2(0.8, 1.2));
    w = w * (1.0 - curvature * 0.08);
    vec2 sp = w * 0.5 + 0.5;
    vec2 e = abs(w);
    float edge = u_scale * 3.0 / u_res.y;
    float inside = (1.0 - smoothstep(1.0 - 0.004, 1.0, e.x)) * (1.0 - smoothstep(1.0 - 0.004, 1.0, e.y));
    if (inside <= 0.0) return vec4(0.0, 0.0, 0.0, src.a);
    // slight convergence error
    float cv = 0.0012;
    vec3 col = vec3(texture(tex, sp + vec2(cv, 0.0)).r, texture(tex, sp).g, texture(tex, sp - vec2(cv, 0.0)).b);
    // scanlines: ~3 source pixels per line at 1080p => period in screen px
    float period = max(3.0 * u_scale, 2.0);
    float y = sp.y * u_res.y / period;
    float sl = 0.5 + 0.5 * cos(y * 6.2831853);
    col *= 1.0 - scanlines * (1.0 - sl) * 0.9;
    // aperture mask (RGB triads)
    float px = sp.x * u_res.x / max(2.0 * u_scale, 1.0);
    float ph = fract(px / 3.0) * 3.0;
    vec3 m = vec3(smoothstep(0.0, 0.6, 1.0 - abs(ph - 0.5)),
                  smoothstep(0.0, 0.6, 1.0 - abs(ph - 1.5)),
                  smoothstep(0.0, 0.6, 1.0 - abs(ph - 2.5)));
    col *= mix(vec3(1.0), 0.55 + 0.9 * m, phosphor);
    col *= 1.0 + 0.2 * scanlines + 0.15 * phosphor;
    float vg = 1.0 - vignette * dot(c * 0.75, c * 0.75) * 1.4;
    col *= clamp(vg, 0.0, 1.0);
    col *= 1.0 + flicker * sin(u_time * 60.0) * 0.5 + flicker * 0.5 * sin(u_time * 7.3 + sp.y * 3.0);
    col += 0.02 * vec3(0.4, 0.6, 0.5) * (1.0 - vignette * 0.3);
    return vec4(col, src.a);
}
