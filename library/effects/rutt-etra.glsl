// @name Rutt-Etra
// @desc Analogue video-synth scan lines lifted by brightness, so the picture becomes a glowing 3D landscape of lines.
// @tags rutt etra, scanlines, synth, analog, glitch, 3d, retro, lines
// @param spacing 6 2 24
// @param height 60 0 120
// @param thickness 1.5 0.5 6
// @param color_amount 1 0 1
vec4 effect(vec4 src, vec2 uv) {
    float s = max(spacing * u_scale, 2.0);
    float hpx = height * u_scale;
    vec2 px = uv * u_res;
    vec3 col = vec3(0.0);
    float a = 0.0;
    // the few scan lines whose lifted path can pass through this pixel
    for (int i = 0; i < 12; i++) {
        float row = floor(px.y / s) * s + float(i) * s;
        if (row > u_res.y + s) { break; }
        vec3 c = texture(tex, clamp(vec2(uv.x, row / u_res.y), 0.0, 1.0)).rgb;
        float yline = row - luma(c) * hpx;
        float d = abs(px.y - yline);
        float w = 1.0 - smoothstep(thickness * u_scale * 0.5, thickness * u_scale * 0.5 + 1.0, d);
        if (w > a) {
            a = w;
            col = mix(vec3(luma(c)), c, color_amount) * 1.2;
        }
    }
    return vec4(clamp(col * a, 0.0, 1.0), src.a);
}
