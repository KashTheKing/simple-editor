// @name Flash
// @desc A quick flash: the outgoing picture blows out to white (or a tint) and the incoming picture comes down from it.
// @tags flash, white, light, exposure, burst
// @param hue 0.1 0 1
// @param saturation 0 0 1
// @param intensity 1 0 2
vec4 transition(vec2 uv) {
    vec4 base = progress < 0.5 ? getFromColor(uv) : getToColor(uv);
    float w = 1.0 - abs(2.0 * progress - 1.0);
    w = w * w * (3.0 - 2.0 * w);
    w = pow(w, 1.4);
    vec3 tint = mix(vec3(1.0), hsl2rgb(vec3(fract(hue), 1.0, 0.62)), saturation);
    float k = w * intensity;
    vec3 exposed = base.rgb * (1.0 + 3.0 * k);
    vec3 col = mix(exposed, tint, clamp(k, 0.0, 1.0));
    return vec4(col, base.a);
}
