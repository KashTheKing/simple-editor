// @name Light Leak
// @desc A soft warm-to-magenta anamorphic light leak sweeps across the frame and dissolves to the incoming picture under its peak.
// @tags light, leak, flare, anamorphic, warm, dreamy, film
// @param intensity 1 0 2
// @param width 0.7 0.2 1.4
// @param streaks 1 0 2
// @param angle 8 -45 45

float ll_noise(float x) {
    float i = floor(x);
    float f = fract(x);
    return mix(hash11(i), hash11(i + 1.0), f * f * (3.0 - 2.0 * f));
}

vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);

    float s = progress;
    vec2 p = vec2((uv.x - 0.5) * ratio, uv.y - 0.5);
    float an = radians(angle);
    // rotate so the sweep can come in at a slant
    vec2 r = vec2(cos(an) * p.x + sin(an) * p.y, -sin(an) * p.x + cos(an) * p.y);

    float halfW = 0.5 * ratio + width;
    float cx = mix(-halfW - width * 0.5, halfW + width * 0.5, s);
    float x = (r.x - cx) / width;       // position across the leak, 0 at its centre

    float env = sin(3.14159265 * s);
    env = sqrt(env) * env;

    // main soft body, taller than wide, with a gentle vertical falloff
    float body = exp(-x * x * 2.2) * (0.75 + 0.25 * exp(-r.y * r.y * 3.0));
    // anamorphic streaks: thin horizontal lines that run far along x
    float lines = pow(ll_noise(r.y * 38.0 + 3.0), 6.0) * 1.3 + pow(ll_noise(r.y * 11.0 + 9.0), 3.0) * 0.5;
    float streakX = exp(-x * x * 0.25);
    float streak = lines * streakX * streaks;
    // a hot horizontal flare line through the middle
    float flare = exp(-r.y * r.y * 900.0) * exp(-x * x * 0.4) * 0.6 * streaks;

    // colour: warm amber on the leading edge, magenta trailing behind
    vec3 warm = vec3(1.0, 0.62, 0.22);
    vec3 hot = vec3(1.0, 0.9, 0.7);
    vec3 mag = vec3(0.95, 0.22, 0.55);
    vec3 lc = mix(mag, warm, smoothstep(-1.2, 0.8, x));
    lc = mix(lc, hot, exp(-x * x * 6.0) * 0.6);

    vec3 leak = lc * body * 1.3 + mix(warm, hot, 0.5) * streak * 0.35 + hot * flare;
    leak *= intensity * env;

    // the dissolve travels with the leak, hidden under its brightest part
    float m = smoothstep(1.0, -1.0, x);
    vec4 a = getFromColor(uv);
    vec4 b = getToColor(uv);
    vec4 base = mix(a, b, m);

    // gentle lift of the whole frame at the peak, then a screen blend of the leak
    vec3 col = 1.0 - (1.0 - base.rgb) * (1.0 - vec3(0.12, 0.06, 0.08) * env * intensity);
    col = 1.0 - (1.0 - col) * (1.0 - clamp(leak, 0.0, 1.0));
    return vec4(col, mix(base.a, 1.0, clamp(max(max(leak.r, leak.g), leak.b), 0.0, 1.0)));
}
