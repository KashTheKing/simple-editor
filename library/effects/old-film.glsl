// @name Old Film
// @desc Silent-film look: sepia or mono, flicker, scratches, dust, gate weave and a vignette.
// @tags film, vintage, retro, sepia, scratches, dust, silent
// @param sepia 0.8 0 1
// @param flicker 0.5 0 1
// @param scratches 0.6 0 1
// @param dust 0.6 0 1
// @param weave 0.5 0 1
// @param vignette 0.6 0 1
vec4 effect(vec4 src, vec2 uv) {
    float tf = floor(u_time * 12.0);
    float asp = u_res.x / u_res.y;
    // gate weave: small per-frame jitter
    vec2 jit = (vec2(hash11(tf * 1.3 + 2.0), hash11(tf * 2.9 + 5.0)) - 0.5) * vec2(0.006, 0.008) * weave;
    vec2 q = (uv - 0.5) * (1.0 - 0.02 * weave) + 0.5 + jit;
    vec4 c = texture(tex, clamp(q, 0.0, 1.0));
    float l = luma(c.rgb);
    l = smoothstep(0.0, 1.0, l * 1.05 - 0.02);
    vec3 mono = vec3(l);
    vec3 tone = vec3(1.0, 0.82, 0.58) * l * 1.05 + vec3(0.04, 0.02, 0.0);
    vec3 col = mix(mono, tone, sepia);
    // exposure flicker and fine grain
    col *= 1.0 + (hash11(tf * 0.7 + 11.0) - 0.5) * 0.35 * flicker;
    float cell = max(1.0, 1.5 * u_scale);
    col += (hash12(floor(uv * u_res / cell) + tf * 9.1) - 0.5) * 0.10;
    // vertical scratches
    for (int i = 0; i < 3; i++) {
        float fi = float(i);
        float on = step(hash11(tf * 3.7 + fi * 7.7 + 1.0), 0.45 * scratches);
        float x = hash11(tf * 5.3 + fi * 13.1 + 4.0);
        float wpx = (0.7 + hash11(fi + tf) * 1.0) * u_scale / u_res.x * 1.0;
        wpx = max(wpx, 0.8 / u_res.x);
        float line = smoothstep(wpx * 1.5, 0.0, abs(uv.x - x));
        float seg = smoothstep(0.2, 0.5, hash12(vec2(floor(uv.y * 6.0), tf + fi)));
        float bright = hash11(fi * 3.3 + tf * 0.3) > 0.5 ? 0.55 : -0.45;
        col += bright * line * on * seg;
    }
    // dust specks
    vec2 p = uv * vec2(asp, 1.0) * 28.0;
    vec2 id = floor(p);
    vec2 f = fract(p);
    float h = hash12(id + tf * 13.0);
    vec2 pos = vec2(hash12(id + tf + 1.3), hash12(id + tf * 1.7 + 7.7));
    float rad = 0.04 + 0.08 * hash12(id + tf + 3.3);
    float speck = smoothstep(rad, rad * 0.4, length(f - pos)) * step(1.0 - 0.06 * dust, h);
    col = mix(col, vec3(hash12(id + 5.0) > 0.4 ? 0.04 : 0.95), speck * 0.85);
    // vignette
    vec2 v = (uv - 0.5) * vec2(asp, 1.0);
    col *= 1.0 - vignette * 0.75 * smoothstep(0.25, 1.0, length(v) * 1.15);
    return vec4(clamp(col, 0.0, 1.0), c.a);
}
