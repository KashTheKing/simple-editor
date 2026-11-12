// @name Tilt-Shift
// @desc Miniature-world look: a sharp band across the frame with growing blur above and below.
// @tags blur, miniature, tilt shift, focus, toy
// @param center 0.5 0 1
// @param width 0.2 0.02 0.8
// @param blur 1.5 0 3
// @param saturation 0.15 0 1
vec4 effect(vec4 src, vec2 uv) {
    float d = max(abs(uv.y - center) - width * 0.5, 0.0);
    float k = smoothstep(0.0, 0.35, d);
    float rad = k * blur * 16.0 * u_scale;
    vec3 acc = src.rgb;
    float wsum = 1.0;
    if (rad > 0.3) {
        acc = vec3(0.0);
        wsum = 0.0;
        for (int i = 0; i < 32; i++) {
            float fi = float(i) + 0.5;
            float r = sqrt(fi / 32.0);
            float a = fi * 2.39996323;
            vec2 o = vec2(cos(a), sin(a)) * r * rad / u_res;
            vec3 s = texture(tex, uv + o).rgb;
            float w = 1.0 + 2.0 * max(luma(s) - 0.8, 0.0); // highlights bloom a little
            acc += s * w;
            wsum += w - 1.0 + 1.0;
        }
    }
    vec3 col = acc / wsum;
    float l = luma(col);
    col = mix(vec3(l), col, 1.0 + saturation * 0.9);
    col = (col - 0.5) * (1.0 + saturation * 0.12) + 0.5;
    return vec4(col, src.a);
}
