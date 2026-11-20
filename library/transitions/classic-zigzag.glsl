// @name Zig-Zag
// @desc Classic 2000s: a saw-toothed edge wipes across the frame from left to right.
// @tags classic 2000s, zigzag, zig-zag, teeth, wipe, retro
// @param teeth 8 2 30
// @param depth 0.08 0 0.3
// @param softness 0.004 0 0.1
vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    float tri = abs(fract(uv.y * teeth) - 0.5) * 2.0;
    float w = max(softness, 1.5 / u_res.x);
    float q = mix(-w, 1.0 + depth + w, progress);
    float m = 1.0 - smoothstep(q - w, q + w, uv.x + depth * tri);
    return mix(getFromColor(uv), getToColor(uv), m);
}
