// @name TV Switch Off
// @desc Old TV switch-off: the outgoing picture collapses to a bright line, then a dot, on black; the incoming picture switches on the reverse way.
// @tags tv, crt, retro, switch off, line, dot
// @param glow 0.8 0 1.5
vec4 tv_pic(float which, vec2 uv, float q) {
    float a = clamp(q / 0.2, 0.0, 1.0);
    float b = clamp((q - 0.2) / 0.12, 0.0, 1.0);
    float c = clamp((q - 0.32) / 0.18, 0.0, 1.0);
    float sy = max(pow(max(1.0 - a, 1e-4), 2.5), 3.0 / u_res.y);
    float sx = max(pow(max(1.0 - b, 1e-4), 2.0), 3.0 / u_res.x);
    vec2 p2 = (uv - 0.5) / vec2(sx, sy) + 0.5;
    float ex = 0.5 * sx - abs(uv.x - 0.5);
    float ey = 0.5 * sy - abs(uv.y - 0.5);
    float m = smoothstep(0.0, 1.5 / u_res.x, ex) * smoothstep(0.0, 1.5 / u_res.y, ey);
    vec4 pic = which < 0.5 ? getFromColor(clamp(p2, 0.0, 1.0)) : getToColor(clamp(p2, 0.0, 1.0));
    vec3 line = vec3(0.88, 0.95, 1.0);
    vec3 col = mix(pic.rgb * (1.0 + 2.0 * a), line, 0.9 * a * a);
    col *= 1.0 - c * c;
    col *= m;
    float ramp = smoothstep(0.12, 0.2, q) * (1.0 - c);
    float hx = max(abs(uv.x - 0.5) - 0.5 * sx, 0.0) * ratio;
    float dy = uv.y - 0.5;
    float gl = exp(-(hx * hx) / 0.0008) * exp(-(dy * dy) / 0.00004) * ramp * glow;
    col += line * gl;
    return vec4(col, 1.0);
}

vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);
    float q = progress < 0.5 ? progress : 1.0 - progress;
    return tv_pic(progress < 0.5 ? 0.0 : 1.0, uv, q);
}
