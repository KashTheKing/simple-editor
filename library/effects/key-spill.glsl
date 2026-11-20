// @name Key Spill
// @desc Removes the green or blue fringe a chroma key leaves on hair and edges. Hue 0.33 is green, 0.66 blue.
// @tags key spill, despill, green screen, blue screen, chroma key, fringe
// @param hue 0.33 0 1
// @param amount 1 0 1
// @param range 0.12 0.02 0.4
// @param keep_luma 1 0 1
vec4 effect(vec4 src, vec2 uv) {
    vec3 c = src.rgb;
    vec3 hsl = rgb2hsl(c);
    float dh = abs(fract(hsl.x - hue + 0.5) - 0.5);
    float w = (1.0 - smoothstep(range * 0.5, range, dh)) * smoothstep(0.02, 0.2, hsl.y);
    // pull the key channel down to the larger of the other two (the classic despill limit)
    vec3 k = hsl2rgb(vec3(hue, 1.0, 0.5));
    float kc = dot(c, k) / max(dot(k, k), 1e-4);
    vec3 others = c - k * kc;
    float lim = max(max(others.r, others.g), others.b);
    vec3 fixed_c = c - k * max(kc - lim, 0.0);
    if (keep_luma > 0.5) { fixed_c += luma(c) - luma(fixed_c); }
    return vec4(mix(c, clamp(fixed_c, 0.0, 1.0), w * amount), src.a);
}
