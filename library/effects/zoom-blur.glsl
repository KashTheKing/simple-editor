// @name Zoom Blur
// @desc Dolly-zoom streaks radiating from a centre point.
// @tags blur, zoom, radial, speed, motion
// @param center_x 0.5 0 1
// @param center_y 0.5 0 1
// @param strength 0.12 0 0.5
// @param clear 0.1 0 0.6
vec4 effect(vec4 src, vec2 uv) {
    float asp = u_res.x / u_res.y;
    vec2 c = vec2(center_x, center_y);
    vec2 v = uv - c;
    float r = length(v * vec2(asp, 1.0));
    float k = strength * smoothstep(clear * 0.5, clear * 0.5 + 0.35, r);
    vec3 acc = vec3(0.0);
    float wsum = 0.0;
    float jit = (hash12(floor(uv * u_res)) - 0.5) / 28.0;
    for (int i = 0; i < 28; i++) {
        float f = float(i) / 27.0 + jit;
        float s = 1.0 - k * f;
        float w = 1.0 - 0.5 * f;
        acc += texture(tex, c + v * s).rgb * w;
        wsum += w;
    }
    return vec4(acc / wsum, src.a);
}
