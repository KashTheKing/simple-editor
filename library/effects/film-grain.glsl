// @name Film Grain
// @desc Animated film grain that bites hardest in the mid-tones and leaves deep shadows and highlights alone.
// @tags grain, film, noise, cinematic, texture
// @param amount 0.5 0 1
// @param size 1.3 0.5 4
// @param color_amount 0.25 0 1
float grain_vn(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash12(i);
    float b = hash12(i + vec2(1.0, 0.0));
    float c = hash12(i + vec2(0.0, 1.0));
    float d = hash12(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}
float grain_layer(vec2 px, float t) {
    // blend two octaves of value noise to get a soft, gaussian-ish grain
    float n = grain_vn(px + t * vec2(37.1, 11.3)) * 0.6 + grain_vn(px * 1.7 + t * vec2(5.3, 71.7) + 9.0) * 0.4;
    return (n - 0.5) * 2.4;
}
vec4 effect(vec4 src, vec2 uv) {
    float t = floor(u_time * 24.0);
    vec2 px = uv * u_res / max(size * u_scale * 1.5, 0.5);
    float g = grain_layer(px, t);
    vec3 gc = vec3(grain_layer(px + 13.7, t), grain_layer(px + 71.3, t), grain_layer(px + 29.9, t));
    vec3 n = mix(vec3(g), gc, color_amount);
    float l = luma(src.rgb);
    float w = 0.35 + 0.65 * 4.0 * l * (1.0 - l);
    vec3 col = src.rgb + n * amount * 0.22 * w;
    return vec4(col, src.a);
}
