// @name Watercolour
// @desc Classic 2000s: soft washes of colour that pool darker at the edges, on rough paper.
// @tags classic 2000s, watercolour, watercolor, paint, artistic, paper, retro
// @param radius 3 1 8
// @param edges 0.6 0 1
// @param paper 0.5 0 1
vec4 effect(vec4 src, vec2 uv) {
    vec2 px = radius * u_scale / u_res;
    vec3 sum = vec3(0.0);
    for (int i = 0; i < 12; i++) {
        float a = float(i) * 0.5235988;
        float r = (i % 2 == 0) ? 1.0 : 2.0;
        sum += texture(tex, uv + vec2(cos(a), sin(a)) * px * r).rgb;
    }
    vec3 wash = (sum / 12.0 + src.rgb) / 2.0;
    wash = floor(wash * 7.0 + 0.5) / 7.0 * 0.6 + wash * 0.4;
    vec2 e = 1.5 * u_scale / u_res;
    float gx = luma(texture(tex, uv + vec2(e.x, 0.0)).rgb) - luma(texture(tex, uv - vec2(e.x, 0.0)).rgb);
    float gy = luma(texture(tex, uv + vec2(0.0, e.y)).rgb) - luma(texture(tex, uv - vec2(0.0, e.y)).rgb);
    float edge = clamp(length(vec2(gx, gy)) * 3.0, 0.0, 1.0);
    wash *= 1.0 - edge * edges * 0.5;
    vec2 g = uv * u_res / (3.0 * u_scale);
    float grain = hash12(floor(g)) * 0.6 + hash12(floor(g * 0.25)) * 0.4;
    wash = mix(wash, wash * (0.88 + 0.2 * grain), paper);
    wash = mix(wash, vec3(1.0), 0.08 * paper);
    return vec4(clamp(wash, 0.0, 1.0), src.a);
}
