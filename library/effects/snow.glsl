// @name Falling Snow
// @desc Soft round snowflakes drifting down in three parallax layers over the picture.
// @tags snow, winter, weather, christmas, particles
// @param amount 0.5 0 1
// @param speed 1.0 0 3
// @param size 1.0 0.4 3
// @param wind 0.3 -2 2
float snow_layer(vec2 p, float density, float fall, float rad, float t, float seed, float amt, float soft) {
    vec2 q = p * density;
    q.y -= t * fall * density * 0.1;
    q.x += t * wind * density * 0.04;
    vec2 id = floor(q);
    vec2 f = fract(q);
    float h = hash12(id + seed);
    float h2 = hash12(id + seed + 17.3);
    float h3 = hash12(id + seed + 41.9);
    float present = step(h, amt);
    float r = rad * (0.55 + 0.45 * h2);
    float room = 0.5 - r - soft;
    vec2 c = vec2(0.5 + room * sin(t * (0.6 + h3) + h * 6.283) * 0.9,
                  0.5 + room * (h3 - 0.5) * 1.6);
    float d = length(f - c);
    return present * (1.0 - smoothstep(r - soft, r + soft, d));
}
vec4 effect(vec4 src, vec2 uv) {
    float asp = u_res.x / u_res.y;
    vec2 p = vec2(uv.x * asp, uv.y);
    float t = u_time;
    float px = 1.0 / u_res.y; // one pixel in frame-height units
    float a = 0.0;
    // far layer: many tiny, slow, faint flakes
    float d1 = 26.0 / size;
    a += snow_layer(p, d1, 1.0 * speed, 0.30, t, 1.0, amount * 0.6, max(0.1, px * d1 * 1.2)) * 0.5;
    // middle layer
    float d2 = 15.0 / size;
    a += snow_layer(p + 0.37, d2, 1.8 * speed, 0.30, t, 7.0, amount * 0.5, max(0.1, px * d2 * 1.2)) * 0.75;
    // near layer: big, soft, fast
    float d3 = 7.0 / size;
    a += snow_layer(p + 0.71, d3, 3.2 * speed, 0.32, t, 13.0, amount * 0.35, max(0.14, px * d3 * 1.5)) * 1.0;
    a = clamp(a, 0.0, 1.0);
    vec3 col = mix(src.rgb, vec3(0.97, 0.98, 1.0), a);
    return vec4(col, src.a);
}
