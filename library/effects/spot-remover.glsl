// @name Spot Remover
// @desc Hides a small blemish, logo or dust spot by filling a rectangle from the picture around it.
// @tags spot, remove, blemish, logo, dust, patch, cleanup, heal
// @param pos_x 0.5 0 1
// @param pos_y 0.5 0 1
// @param width 0.2 0.005 0.5
// @param height 0.2 0.005 0.5
// @param feather 0.3 0 1
vec4 effect(vec4 src, vec2 uv) {
    vec2 lo = vec2(pos_x - width * 0.5, pos_y - height * 0.5);
    vec2 hi = vec2(pos_x + width * 0.5, pos_y + height * 0.5);
    vec2 f = (uv - lo) / max(hi - lo, vec2(1e-4));
    if (f.x < 0.0 || f.x > 1.0 || f.y < 0.0 || f.y > 1.0) { return src; }
    vec2 e = 1.5 / u_res;
    // the four edges of the box at this row / column, mixed by distance (a Coons-style fill)
    vec3 l = texture(tex, clamp(vec2(lo.x - e.x, uv.y), 0.0, 1.0)).rgb;
    vec3 r = texture(tex, clamp(vec2(hi.x + e.x, uv.y), 0.0, 1.0)).rgb;
    vec3 t = texture(tex, clamp(vec2(uv.x, lo.y - e.y), 0.0, 1.0)).rgb;
    vec3 b = texture(tex, clamp(vec2(uv.x, hi.y + e.y), 0.0, 1.0)).rgb;
    vec3 h = mix(l, r, f.x);
    vec3 v = mix(t, b, f.y);
    float wx = 1.0 / max(min(f.x, 1.0 - f.x), 1e-3);
    float wy = 1.0 / max(min(f.y, 1.0 - f.y), 1e-3);
    vec3 fill = (h * wx + v * wy) / (wx + wy);
    fill += (hash12(uv * u_res) - 0.5) * 0.01;
    float edge = min(min(f.x, 1.0 - f.x), min(f.y, 1.0 - f.y)) * 2.0;
    float m = (feather <= 0.0) ? 1.0 : smoothstep(0.0, feather, edge);
    return vec4(mix(src.rgb, fill, m), src.a);
}
