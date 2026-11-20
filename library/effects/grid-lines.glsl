// @name Grid Lines
// @desc Draws a grid of lines over the picture, for a blueprint, notebook or layout look.
// @tags grid, lines, blueprint, graph paper, notebook, overlay
// @param columns 8 1 64
// @param rows 6 1 64
// @param width 2 0.5 12
// @param light 1 0 1
// @param opacity 0.6 0 1
vec4 effect(vec4 src, vec2 uv) {
    vec2 cell = u_res / vec2(columns, rows);
    vec2 px = uv * u_res;
    vec2 d = abs(px - floor(px / cell + 0.5) * cell);
    float w = width * u_scale * 0.5;
    float line = 1.0 - smoothstep(w, w + 1.0, min(d.x, d.y));
    return vec4(mix(src.rgb, vec3(light), line * opacity), src.a);
}
