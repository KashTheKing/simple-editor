// @name Cut-out Outline
// @desc Sticker-style solid outline around the opaque part of a clip with transparency.
// @tags alpha, cutout, outline, sticker, border, stroke
// @param width 16 0 40
// @param hue 0.12 0 1
// @param sat 0 0 1
// @param light 1 0 1
float outline_a(vec2 uv) {
    if (uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0) return 0.0;
    return texture(tex, uv).a;
}
vec4 effect(vec4 src, vec2 uv) {
    vec2 px = width * u_scale / u_res;
    float m = 0.0;
    for (int i = 0; i < 16; i++) {
        float a = (float(i) + 0.5) * 0.39269908;
        m = max(m, outline_a(uv + vec2(cos(a), sin(a)) * px));
    }
    for (int i = 0; i < 8; i++) {
        float a = float(i) * 0.78539816 + 0.2;
        m = max(m, outline_a(uv + vec2(cos(a), sin(a)) * px * 0.5));
    }
    float oa = width > 0.0 ? smoothstep(0.0, 1.0, m) : 0.0;
    vec3 oc = hsl2rgb(vec3(hue, sat, light));
    float a = max(src.a, oa);
    vec3 col = mix(oc, src.rgb, src.a / max(a, 1e-4));
    return vec4(col, a);
}
