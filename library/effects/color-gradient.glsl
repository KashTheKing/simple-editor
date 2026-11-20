// @name Color Gradient
// @desc Lays a two-colour linear or radial gradient over the picture, blended softly or as a colour wash.
// @tags gradient, color, overlay, wash, tint, sky, sunset
// @param angle 90 0 360
// @param radial 0 0 1
// @param hue_a 0.58 0 1
// @param hue_b 0.08 0 1
// @param sat 1 0 1
// @param opacity 0.85 0 1
// @param mode 1 0 2
vec4 effect(vec4 src, vec2 uv) {
    vec2 p = uv - 0.5;
    p.x *= u_res.x / u_res.y;
    float t;
    if (radial > 0.5) {
        t = clamp(length(p) / 0.7, 0.0, 1.0);
    } else {
        float a = radians(angle);
        t = clamp(dot(p, vec2(cos(a), sin(a))) + 0.5, 0.0, 1.0);
    }
    vec3 g = mix(hsl2rgb(vec3(hue_a, sat, 0.5)), hsl2rgb(vec3(hue_b, sat, 0.5)), t);
    vec3 o;
    if (mode < 0.5) { o = g; }                                          // normal
    else if (mode < 1.5) {                                              // soft light
        o = mix(2.0 * src.rgb * g + src.rgb * src.rgb * (1.0 - 2.0 * g),
                sqrt(src.rgb) * (2.0 * g - 1.0) + 2.0 * src.rgb * (1.0 - g), step(0.5, g));
    } else { o = src.rgb * g * 2.0; }                                   // multiply (x2)
    return vec4(mix(src.rgb, clamp(o, 0.0, 1.0), opacity), src.a);
}
