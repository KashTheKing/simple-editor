// @name Color Swipe Double
// @desc Two to four stacked colour bands chase each other diagonally across the frame, covering the outgoing picture and uncovering the incoming one.
// @tags swipe, color, bands, diagonal, wipe
// @param hue 0.58 0 1
// @param hue_offset 0.08 -0.5 0.5
// @param bands 3 2 4
vec4 csd_strip(float k, float n, float s, float smax, vec2 uv) {
    float st = 0.1;
    float u = clamp((progress - k * st) / (1.0 - (n - 1.0) * st), 0.0, 1.0);
    float a = clamp(u * 2.0, 0.0, 1.0);
    float b = clamp(u * 2.0 - 1.0, 0.0, 1.0);
    a = a * a * (3.0 - 2.0 * a);
    b = b * b * (3.0 - 2.0 * b);
    float ext = 0.03;
    float front = mix(-ext, smax + ext, a);
    float back = mix(-ext, smax + ext, b);
    float w = 1.5 / u_res.y;
    float cover = (1.0 - smoothstep(front - w, front + w, s)) * smoothstep(back - w, back + w, s);
    vec4 under = u < 0.5 ? getFromColor(uv) : getToColor(uv);
    vec3 base = hsl2rgb(vec3(fract(hue + k * hue_offset + 1.0), 0.8, 0.5));
    float kk = clamp((s - back) / max(front - back, 1e-4), 0.0, 1.0);
    vec3 col = base * (0.82 + 0.28 * kk);
    col += 0.2 * smoothstep(0.05, 0.0, front - s);
    return vec4(mix(under.rgb, col, cover), mix(under.a, 1.0, cover));
}

vec4 transition(vec2 uv) {
    float ang = 0.55;
    vec2 d = vec2(cos(ang), sin(ang));
    vec2 pos = vec2(uv.x * ratio, uv.y);
    float s = dot(pos, d);
    float v = dot(pos, vec2(-d.y, d.x));
    float smax = ratio * d.x + d.y;
    float vmin = -ratio * d.y;
    float vmax = d.x;
    float n = clamp(floor(bands + 0.5), 2.0, 4.0);
    float f = clamp((v - vmin) / (vmax - vmin), 0.0, 0.9999) * n;
    float k = floor(f);
    float fr = f - k;
    float aa = 1.5 * n / (u_res.y * (vmax - vmin));
    vec4 c = csd_strip(k, n, s, smax, uv);
    if (fr < aa && k > 0.5) {
        c = mix(csd_strip(k - 1.0, n, s, smax, uv), c, smoothstep(0.0, aa, fr) * 0.5 + 0.5);
    } else if (fr > 1.0 - aa && k < n - 1.5) {
        c = mix(c, csd_strip(k + 1.0, n, s, smax, uv), (1.0 - smoothstep(0.0, aa, 1.0 - fr)) * 0.5);
    }
    return c;
}
