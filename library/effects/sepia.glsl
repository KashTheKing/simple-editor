// @name Sepia
// @desc A warm brown sepia tone, like an old photograph.
// @tags sepia, vintage, old, photo, warm, brown, tone
// @param amount 1 0 1
// @param warmth 0.5 0 1
vec4 effect(vec4 src, vec2 uv) {
    float y = luma(src.rgb);
    vec3 tone = mix(vec3(1.0, 0.9, 0.75), vec3(1.07, 0.84, 0.6), warmth);
    vec3 c = clamp(y * tone + vec3(0.03, 0.01, 0.0) * (1.0 - y), 0.0, 1.0);
    return vec4(mix(src.rgb, c, amount), src.a);
}
