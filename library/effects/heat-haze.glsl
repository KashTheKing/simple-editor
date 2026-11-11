// @name Heat Haze
// @desc Rising heat shimmer, like air over hot tarmac or a desert: the picture wobbles more toward the bottom.
// @tags heat, haze, shimmer, desert, mirage, distortion, summer
// @param strength 1.4 0 3
// @param bottom 0.8 0 1
// @param speed 1.0 0 3
// @param size 1.0 0.3 3
// @param fringe 0.5 0 1

float hh_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash12(i), hash12(i + vec2(1.0, 0.0)), f.x),
               mix(hash12(i + vec2(0.0, 1.0)), hash12(i + vec2(1.0, 1.0)), f.x), f.y);
}

vec4 effect(vec4 src, vec2 uv) {
    float aspect = u_res.x / u_res.y;
    vec2 p = vec2(uv.x * aspect, uv.y) * 9.0 / size;
    float t = u_time * speed;
    // the pattern rises (toward y = 0, the top) and drifts a little sideways
    vec2 q = p * vec2(1.0, 1.8) + vec2(0.0, t * 2.2);
    float nx = hh_noise(q + vec2(0.0, 0.0)) + 0.5 * hh_noise(q * 2.1 + vec2(5.2, t * 1.3));
    float ny = hh_noise(q + vec2(17.3, 3.1)) + 0.5 * hh_noise(q * 2.1 + vec2(1.7, 9.2 + t * 1.3));
    vec2 d = vec2(nx, ny) / 1.5 - 0.5;
    // stronger toward the bottom of the frame
    float fall = mix(1.0, smoothstep(0.0, 1.0, uv.y) * 1.3, bottom);
    // ~7 px of wobble at 1080p, mostly sideways like real shimmer
    vec2 off = d * vec2(1.0, 0.6) * 14.0 * u_scale * strength * fall / u_res;

    float spread = 0.25 * fringe;
    float r = sample_clamped(tex, uv + off * (1.0 + spread)).r;
    vec4 g = sample_clamped(tex, uv + off);
    float b = sample_clamped(tex, uv + off * (1.0 - spread)).b;
    return vec4(r, g.g, b, src.a);
}
