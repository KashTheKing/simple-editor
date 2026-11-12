// @name Night Vision
// @desc Green phosphor night-vision goggles: boosted light, grain, scanlines and a round vignette.
// @tags night, green, military, grain, scanlines, vignette
// @param gain 1.6 0.5 4
// @param grain 0.3 0 1
// @param scanlines 0.3 0 1
// @param vignette 0.9 0 1
vec4 effect(vec4 src, vec2 uv) {
    vec2 px = 1.5 * u_scale / u_res;
    float l = luma(src.rgb) * 4.0;
    l += luma(texture(tex, uv + vec2(px.x, 0.0)).rgb) + luma(texture(tex, uv - vec2(px.x, 0.0)).rgb);
    l += luma(texture(tex, uv + vec2(0.0, px.y)).rgb) + luma(texture(tex, uv - vec2(0.0, px.y)).rgb);
    l /= 8.0;
    l = 1.0 - exp(-l * gain * 1.2);
    l = pow(l, 1.25);
    float cell = max(1.0, 2.0 * u_scale);
    float n = hash12(floor(uv * u_res / cell) + floor(u_time * 24.0) * 17.13) - 0.5;
    l += n * grain * 0.6;
    float sl = 0.5 + 0.5 * sin(uv.y * 270.0 * 6.2831853);
    l *= 1.0 - scanlines * 0.35 * sl;
    l += 0.012 * sin(uv.y * 6.0 - u_time * 1.5) * scanlines;
    vec2 p = (uv - 0.5) * vec2(u_res.x / u_res.y, 1.0);
    float v = smoothstep(0.98, 0.45, length(p));
    l *= mix(1.0, v, vignette);
    vec3 col = vec3(0.12, 1.0, 0.22) * max(l, 0.0);
    col += vec3(0.05, 0.12, 0.05) * max(l - 0.7, 0.0) * 3.0;
    col += vec3(0.0, 0.015, 0.0);
    return vec4(clamp(col, 0.0, 1.0), src.a);
}
