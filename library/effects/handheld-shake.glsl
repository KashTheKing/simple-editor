// @name Handheld Shake
// @desc Organic handheld camera movement: smooth drifting, tiny rotation and a slight zoom so edges never show.
// @tags shake, handheld, camera, movement, organic, documentary
// @param amount 1 0 3
// @param speed 1 0.1 4
// @param rotation 1 0 3
float shk_noise(float x, float seed) {
    float i = floor(x);
    float f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash11(i + seed) * 2.0 - 1.0;
    float b = hash11(i + 1.0 + seed) * 2.0 - 1.0;
    return mix(a, b, f);
}
float shk_fbm(float x, float seed) {
    return 0.65 * shk_noise(x, seed) + 0.35 * shk_noise(x * 2.7 + 5.0, seed + 40.0);
}
vec4 effect(vec4 src, vec2 uv) {
    float asp = u_res.x / u_res.y;
    float t = u_time * speed * 2.2;
    float maxT = 0.012 * amount;
    float maxR = 0.012 * rotation;
    vec2 tr = vec2(shk_fbm(t, 1.0), shk_fbm(t + 31.0, 2.0)) * maxT;
    float ang = shk_fbm(t * 0.8 + 77.0, 3.0) * maxR;
    // worst-case edge reach: translation plus the rotated corner, plus a little margin
    float reach = maxT + maxR * 0.6 * (asp > 1.0 ? 1.0 : 1.0) + 0.002;
    float zoom = 1.0 / max(1.0 - 2.0 * reach, 0.5);
    vec2 p = (uv - 0.5) * vec2(asp, 1.0) / zoom;
    float cs = cos(ang);
    float sn = sin(ang);
    p = vec2(cs * p.x - sn * p.y, sn * p.x + cs * p.y);
    vec2 suv = p / vec2(asp, 1.0) + 0.5 + tr;
    return sample_clamped(tex, suv);
}
