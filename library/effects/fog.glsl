// @name Drifting Fog
// @desc Slow volumetric-looking fog that rolls sideways and pools toward the bottom of the frame.
// @tags fog, mist, atmosphere, haze, horror
// @param density 0.7 0 1.5
// @param speed 0.5 0 3
// @param brightness 0.8 0 1
// @param warmth 0 -1 1
// @param zoom 1.0 0.3 3
float fog_vn(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash12(i), hash12(i + vec2(1.0, 0.0)), f.x),
               mix(hash12(i + vec2(0.0, 1.0)), hash12(i + vec2(1.0, 1.0)), f.x), f.y);
}
float fog_fbm(vec2 p) {
    float a = 0.5;
    float s = 0.0;
    for (int i = 0; i < 5; i++) {
        s += a * fog_vn(p);
        p = p * 2.03 + vec2(17.1, 9.2);
        a *= 0.5;
    }
    return s;
}
vec4 effect(vec4 src, vec2 uv) {
    float asp = u_res.x / u_res.y;
    float t = u_time * speed;
    vec2 p = vec2(uv.x * asp, uv.y) * (2.2 / zoom);
    // domain warp for a billowing, rolling look
    vec2 w = vec2(fog_fbm(p + vec2(t * 0.15, 0.0)), fog_fbm(p + vec2(5.2, 1.3 - t * 0.07)));
    float n = fog_fbm(p + 1.6 * w + vec2(t * 0.25, t * 0.04));
    n = smoothstep(0.25, 0.8, n);
    float h = smoothstep(0.15, 1.0, uv.y);   // y = 1 is the bottom
    float d = n * (0.25 + 0.95 * h) * density;
    d = 1.0 - exp(-d * 1.6);
    vec3 tint = mix(vec3(0.78, 0.86, 1.0), vec3(1.0, 0.9, 0.78), warmth * 0.5 + 0.5);
    vec3 fogc = tint * brightness;
    vec3 col = mix(src.rgb, fogc, clamp(d, 0.0, 0.95));
    return vec4(col, src.a);
}
