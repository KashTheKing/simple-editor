// @name Lyric Chrome
// @desc Lyric line as silver (gold after the drop) on a soft dark halo with a light 3D bevel and thin edge, turned in real perspective by yaw (faces the centre when off to a side); streaks with motion blur as it is pulled on and off screen; can wrap and roll around a cylinder.
// @tags text, karaoke, lyrics, gold, 3d, bevel
// @credit Visualizer Helper (MIT)
// @param fill 0 0 1
// @param soft 0.008 0 0.05
// @param gold 0 0 1
// @param smear 0 0 1
// @param curve 0 0 1
// @param roll 0 -1.5 1.5
// @param yaw 0 -1 1
float aAt(vec2 q) { return (q.x < 0.0 || q.x > 1.0 || q.y < 0.0 || q.y > 1.0) ? 0.0 : texture(tex, q).a; }
vec4 effect(vec4 src, vec2 uv) {
    // margin inside the box so curves, edges and the shadow never get cut off (clip scale is raised to match)
    vec2 q = 0.5 + (uv - 0.5) * vec2(1.08, 1.25);
    float pers = 1.0;
    if (curve > 0.001) {
        float A = 1.35 * curve;
        float ang = asin(clamp((q.x - 0.5) * 2.0 * sin(A), -1.0, 1.0));
        q.x = 0.5 + ang / (2.0 * A) + roll;
        if (q.x < 0.0 || q.x > 1.0) return vec4(0.0);
        pers = 1.0 / (1.0 + 0.45 * (1.0 - cos(ang)) / max(1.0 - cos(A), 1e-3) * curve);
        q.y = 0.5 + (q.y - 0.5) / pers;
        if (q.y < 0.0 || q.y > 1.0) return vec4(0.0);
    }
    // real perspective turn about the vertical axis (yaw > 0 faces right)
    float th = yaw * 0.7;
    float sn = sin(th), cs = cos(th);
    if (abs(yaw) > 0.001) {
        float X = q.x - 0.5;
        float xl = X / (cs - 0.7 * X * sn);
        q = vec2(0.5 + xl, 0.5 + (q.y - 0.5) * (1.0 + 0.7 * xl * sn));
        if (q.x < 0.0 || q.x > 1.0 || q.y < 0.0 || q.y > 1.0) return vec4(0.0);
    }
    // edge / bevel / halo widths scale with the text, not the texture's pixel count: the same look at any raster size
    vec2 tx = vec2(u_res.y / 94.0) / u_res;
    vec3 metal = mix(vec3(0.86, 0.88, 0.92), vec3(1.0, 0.78, 0.38), gold);
    vec3 shine = mix(vec3(1.0), vec3(1.0, 0.94, 0.75), gold);
    float a0 = aAt(q);
    float gx = aAt(q + vec2(2.0 * tx.x, 0.0)) - aAt(q - vec2(2.0 * tx.x, 0.0));
    float gy = aAt(q + vec2(0.0, 2.0 * tx.y)) - aAt(q - vec2(0.0, 2.0 * tx.y));
    vec3 n = normalize(vec3(-gx * 1.1, gy * 1.1, 1.0));
    n = vec3(n.x * cs + n.z * sn, n.y, -n.x * sn + n.z * cs);
    vec3 L = normalize(vec3(-0.35, 0.65, 0.68));
    float diff = max(dot(n, L), 0.0);
    float spec = pow(max(dot(reflect(-L, n), vec3(0.0, 0.0, 1.0)), 0.0), 18.0);
    float tt = (q.y - 0.5) * 2.4;
    float ch = 0.88 + 0.12 * cos(tt * 3.6 + 0.6) + 0.18 * exp(-pow((tt + 0.3) * 8.0, 2.0));
    float sweep = exp(-pow((q.x - (fract(u_time * 0.22) * 1.6 - 0.3)) * 7.0, 2.0));
    float k = 1.0 - smoothstep(fill - soft, fill + soft, q.x);
    vec3 face = metal * ch * (0.7 + 0.45 * diff) + shine * spec * 0.6;
    face *= mix(0.96, 1.04, k);
    face += shine * (sweep * 0.2 + 0.12 * exp(-abs(q.x - fill) * 30.0)) * k;
    // thin edge: goes back away from the viewer, so it shows on the side the line turns from
    float side = 0.0;
    if (a0 < 0.99) {
        for (int i = 1; i <= 3; i++) {
            float fi = float(i);
            side = max(side, aAt(q - vec2(-sn * 1.6 * fi, 0.6 * fi) * tx));
        }
    }
    vec3 col = mix(metal * 0.55, face, a0);
    float a = max(a0, side);
    if (smear > 0.002) {
        // moving fast: a motion-blur streak trailing the letters
        float acc = 0.0, wsum = 0.0;
        for (int i = 0; i < 16; i++) {
            float f = float(i) / 15.0;
            float w = 1.0 - 0.85 * f;
            acc += aAt(q + vec2(f * smear * 0.35, 0.0)) * w;
            wsum += w;
        }
        float st = acc / wsum * 0.85;
        col = mix(metal, col, a / max(max(a, st), 1e-4));
        a = max(a, st);
    }
    col *= mix(1.0, 0.55 + 0.45 * pers, curve);
    // dark soft halo behind the letters so they read on bright backgrounds
    // soft shadow glow (blurred, not an outline)
    float halo = 0.0;
    for (int i = 0; i < 12; i++) {
        float an = float(i) * 0.5236 + 0.26;
        vec2 d = vec2(cos(an), sin(an)) * tx;
        halo += aAt(q + d * 3.5) + aAt(q + d * 8.0) * 0.8;
    }
    halo = clamp(halo / 21.6 * 2.6, 0.0, 1.0) * 0.85;
    vec3 dark = mix(vec3(0.03), vec3(0.06, 0.035, 0.0), gold);
    float ao = max(a, halo);
    col = mix(dark, col, a / max(ao, 1e-4));
    return vec4(col, ao);
}
