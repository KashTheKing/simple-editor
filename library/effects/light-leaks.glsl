// @name Light Leaks
// @desc Slow warm and magenta light leaks drifting in from the frame edges, screen-blended.
// @tags light leak, film, warm, dreamy, overlay
// @param intensity 0.8 0 2
// @param hue 0.04 0 1
// @param speed 0.4 0 2
float leak_vn(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash12(i), hash12(i + vec2(1.0, 0.0)), f.x),
               mix(hash12(i + vec2(0.0, 1.0)), hash12(i + vec2(1.0, 1.0)), f.x), f.y);
}
vec4 effect(vec4 src, vec2 uv) {
    float asp = u_res.x / u_res.y;
    float t = u_time * speed;
    vec2 p = vec2(uv.x * asp, uv.y);
    // blobs wander along the edges
    vec2 b1 = vec2(asp * (0.5 + 0.5 * sin(t * 0.55)), -0.05 + 0.1 * sin(t * 0.9));          // top
    vec2 b2 = vec2(asp * (1.0 + 0.04 * sin(t * 0.7)), 0.5 + 0.4 * sin(t * 0.42 + 1.0));       // right
    vec2 b3 = vec2(asp * (0.0 - 0.04 * sin(t * 0.6)), 0.6 + 0.4 * sin(t * 0.33 + 3.0));       // left
    vec2 b4 = vec2(asp * (0.5 + 0.5 * sin(t * 0.37 + 2.0)), 1.05);                            // bottom
    float g1 = exp(-dot(p - b1, p - b1) / 0.16);
    float g2 = exp(-dot((p - b2) * vec2(1.6, 0.8), (p - b2) * vec2(1.6, 0.8)) / 0.12);
    float g3 = exp(-dot((p - b3) * vec2(1.6, 0.8), (p - b3) * vec2(1.6, 0.8)) / 0.10);
    float g4 = exp(-dot(p - b4, p - b4) / 0.10);
    float n = 0.65 + 0.7 * leak_vn(p * 2.0 + vec2(t * 0.6, -t * 0.3));
    vec3 warm = hsl2rgb(vec3(hue, 1.0, 0.55));
    vec3 mag = hsl2rgb(vec3(fract(hue + 0.88), 0.95, 0.5));
    vec3 yel = hsl2rgb(vec3(fract(hue + 0.08), 1.0, 0.6));
    vec3 leak = (warm * g1 + mag * g2 + yel * g3 * 0.9 + warm * g4 * 0.6 + mag * g1 * 0.3) * n;
    leak *= intensity;
    // slow breathing
    leak *= 0.85 + 0.15 * sin(t * 1.3);
    vec3 col = 1.0 - (1.0 - src.rgb) * (1.0 - clamp(leak, 0.0, 1.0));
    return vec4(col, src.a);
}
