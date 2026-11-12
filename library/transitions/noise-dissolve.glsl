// @name Noise Dissolve
// @desc An organic dissolve through smooth noise, with a thin bright line glowing along the dissolve front.
// @tags dissolve, noise, organic, burn, edge
// @param scale 3.5 1 12
// @param softness 0.03 0 0.2
// @param edge_width 0.03 0 0.1
// @param edge_hue 0.09 0 1
float nd_vn(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    float a = hash12(i);
    float b = hash12(i + vec2(1.0, 0.0));
    float c = hash12(i + vec2(0.0, 1.0));
    float d = hash12(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

vec4 transition(vec2 uv) {
    vec2 p = uv * vec2(ratio, 1.0) * scale;
    float f = 0.55 * nd_vn(p) + 0.3 * nd_vn(p * 2.1 + 7.3) + 0.15 * nd_vn(p * 4.3 + 3.1);
    float n = clamp((f - 0.22) / 0.56, 0.0, 1.0);
    float ew = max(edge_width, 1e-4);
    float sf = max(softness, 1.5 / u_res.y);
    float ext = 3.0 * ew + sf;
    float t = progress * (1.0 + 2.0 * ext) - ext;
    float m = smoothstep(-sf, sf, t - n);
    float d = (t - n) / ew;
    float fade = smoothstep(0.0, 0.05, progress) * smoothstep(1.0, 0.95, progress);
    float glow = exp(-d * d) * fade;
    vec4 col = mix(getFromColor(uv), getToColor(uv), m);
    vec3 ec = mix(hsl2rgb(vec3(fract(edge_hue), 1.0, 0.6)), vec3(1.0), 0.55);
    col.rgb += ec * glow * 1.4;
    return col;
}
