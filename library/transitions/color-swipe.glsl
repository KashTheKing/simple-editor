// @name Color Swipe
// @desc A solid colour panel with a slanted edge sweeps across, covers the outgoing picture, then sweeps off to reveal the incoming one.
// @tags swipe, color, panel, wipe, slant
// @param hue 0.62 0 1
// @param slant 0.25 -0.6 0.6
// @param saturation 0.8 0 1
vec4 transition(vec2 uv) {
    float ext = abs(slant) * 0.5 + 0.03;
    float t = uv.x + (uv.y - 0.5) * slant;
    float a = clamp(progress * 2.0, 0.0, 1.0);
    float b = clamp(progress * 2.0 - 1.0, 0.0, 1.0);
    a = a * a * (3.0 - 2.0 * a);
    b = b * b * (3.0 - 2.0 * b);
    float front = mix(-ext, 1.0 + ext, a);
    float back = mix(-ext, 1.0 + ext, b);
    float w = 1.5 / u_res.x;
    float cover = (1.0 - smoothstep(front - w, front + w, t)) * smoothstep(back - w, back + w, t);

    vec4 under = progress < 0.5 ? getFromColor(uv) : getToColor(uv);
    vec3 base = hsl2rgb(vec3(fract(hue), saturation, 0.5));
    float k = clamp((t - back) / max(front - back, 1e-4), 0.0, 1.0);
    vec3 col = base * (0.82 + 0.28 * k);
    // lighter stripe along the leading edge
    col += 0.22 * smoothstep(0.06, 0.0, front - t) * (1.0 - smoothstep(0.0, 0.02, back - t + 1.0));
    return vec4(mix(under.rgb, col, cover), mix(under.a, 1.0, cover));
}
