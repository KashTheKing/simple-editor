// @name Rising Sparks
// @desc Glowing embers lifting off the bottom of the frame, flickering and curling as they rise.
// @tags sparks, embers, fire, particles, glow
// @param amount 0.9 0 1
// @param speed 1.0 0 3
// @param size 1.0 0.4 3
// @param hue 0.07 0 1
float spark_layer(vec2 p, float density, float rise, float t, float seed, float amt, float glow) {
    vec2 q = p * density;
    q.y += t * rise * density * 0.1;
    // curl: columns of embers sway as they rise
    vec2 id0 = floor(q);
    q.x += 0.35 * sin(q.y * 0.9 + t * 1.3 + id0.x);
    vec2 id = floor(q);
    vec2 f = fract(q);
    float h = hash12(id + seed);
    float h2 = hash12(id + seed + 17.3);
    float h3 = hash12(id + seed + 41.9);
    float present = step(h, amt);
    vec2 c = vec2(0.5 + 0.3 * (h2 - 0.5), 0.5 + 0.3 * (h3 - 0.5));
    vec2 dv = (f - c) * vec2(1.0, 0.8);
    float d = length(dv);
    float core = 1.0 - smoothstep(0.05, 0.09 + 0.05 * h2, d);
    float halo = exp(-d * d / (glow * glow));
    float flick = 0.55 + 0.45 * sin(t * (6.0 + 10.0 * h3) + h * 40.0);
    return present * (core * 2.0 + halo * 1.1) * flick;
}
vec4 effect(vec4 src, vec2 uv) {
    float asp = u_res.x / u_res.y;
    vec2 p = vec2(uv.x * asp, uv.y);
    float t = u_time;
    float s = 1.0 / size;
    float v = 0.0;
    v += spark_layer(p, 22.0 * s, 2.2 * speed, t, 3.0, amount * 0.55, 0.13) * 0.5;
    v += spark_layer(p + 0.5, 12.0 * s, 3.4 * speed, t, 11.0, amount * 0.5, 0.15);
    v += spark_layer(p + 0.2, 6.0 * s, 4.6 * speed, t, 23.0, amount * 0.4, 0.17) * 1.2;
    // more embers near the bottom (y = 1), fading toward the top
    v *= smoothstep(0.05, 0.95, uv.y) * 1.2;
    vec3 hot = hsl2rgb(vec3(hue, 1.0, 0.55));
    vec3 white = vec3(1.0, 0.92, 0.7);
    vec3 glowc = mix(hot, white, clamp(v - 0.7, 0.0, 1.0));
    vec3 col = src.rgb + glowc * v * 1.6;
    return vec4(col, src.a);
}
