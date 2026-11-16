// @name Neon Orb
// @desc Audio visualizer: a glowing, noise-rippled orb in a dark void with bright neon veins. Right-click an audio clip ▸ Audio ▸ Add Audio Visualizer (or key "level" with audio.bake_keys) and it swells and boils with every beat.
// @tags visualizer, audio, music, orb, neon, glow, noise, sphere, blender
// @param level 0.3 0 1
// @param orb_size 0.35 0.05 1
// @param hue 0.48 0 1
// @param glow 1 0 3
// @param rough 1 0 3
// @param speed 0.5 0 3
// @param dark 1 0 1
float nh(vec3 p) {
    p = fract(p * 0.3183099 + 0.1);
    p *= 17.0;
    return fract(p.x * p.y * p.z * (p.x + p.y + p.z));
}
float vnoise(vec3 x) {
    vec3 i = floor(x), f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(mix(nh(i), nh(i + vec3(1, 0, 0)), f.x), mix(nh(i + vec3(0, 1, 0)), nh(i + vec3(1, 1, 0)), f.x), f.y),
               mix(mix(nh(i + vec3(0, 0, 1)), nh(i + vec3(1, 0, 1)), f.x), mix(nh(i + vec3(0, 1, 1)), nh(i + vec3(1, 1, 1)), f.x), f.y), f.z);
}
float fbm(vec3 p) {
    float a = 0.5, s = 0.0;
    for (int i = 0; i < 4; i++) {
        s += a * vnoise(p);
        p = p * 2.03 + vec3(1.7, 9.2, 3.1);
        a *= 0.5;
    }
    return s;
}
vec3 hsv(float h, float s, float v) {
    vec3 k = clamp(abs(mod(h * 6.0 + vec3(0, 4, 2), 6.0) - 3.0) - 1.0, 0.0, 1.0);
    return v * mix(vec3(1.0), k, s);
}
// signed distance to the rippled orb; the sphere swells and the ripples deepen with `level`
float orb(vec3 p, float tm) {
    float r = 1.0 + 0.22 * level;
    float n = fbm(p * 2.2 + vec3(0.0, 0.0, tm)) - 0.5;
    return length(p) - r - n * (0.25 + 0.3 * level) * rough;
}
vec4 effect(vec4 src, vec2 uv) {
    float tm = u_time * speed;
    vec2 q = (uv - 0.5) * vec2(u_res.x / u_res.y, 1.0) / orb_size;
    vec3 ro = vec3(0.0, 0.0, 3.0);
    vec3 rd = normalize(vec3(q, -2.6));
    float cr = cos(tm * 0.3), sr = sin(tm * 0.3);
    mat2 rot = mat2(cr, -sr, sr, cr);
    vec3 c = hsv(hue, 0.85, 1.0);
    float t = 1.2, md = 1e3;
    bool hit = false;
    vec3 p = ro;
    // ponytail: 64 fixed march steps at 0.6x the distance (the noise breaks the Lipschitz bound)
    for (int i = 0; i < 64; i++) {
        p = ro + rd * t;
        p.xz = rot * p.xz;
        float d = orb(p, tm);
        md = min(md, d);
        if (d < 0.002) { hit = true; break; }
        t += d * 0.6;
        if (t > 5.0) break;
    }
    vec3 col = vec3(0.0);
    if (hit) {
        vec2 e = vec2(0.003, 0.0);
        vec3 n = normalize(vec3(orb(p + e.xyy, tm) - orb(p - e.xyy, tm),
                                orb(p + e.yxy, tm) - orb(p - e.yxy, tm),
                                orb(p + e.yyx, tm) - orb(p - e.yyx, tm)));
        vec3 v = -rd;
        v.xz = rot * v.xz;
        float fr = pow(1.0 - max(dot(n, v), 0.0), 2.5);
        // thin bright ridges of a second noise: the lightning veins
        float vein = pow(1.0 - abs(fbm(p * 4.0 + vec3(tm * 0.7)) * 2.0 - 1.0), 14.0);
        col = c * (0.06 + (1.3 + 0.4 * level) * fr * glow) + c * vein * (0.5 + 0.7 * level) * glow + vec3(pow(fr, 6.0) * 0.6);
    }
    // halo around the silhouette
    col += c * glow * (0.35 + level) * exp(-max(md, 0.0) * 9.0) * 0.5;
    vec3 base = src.rgb * (1.0 - dark);
    float a = max(src.a, max(dark, clamp(max(col.r, max(col.g, col.b)), 0.0, 1.0)));
    return vec4(base + col, a);
}
