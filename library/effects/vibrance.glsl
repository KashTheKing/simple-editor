// @name Vibrance
// @desc Boosts dull colours more than already-saturated ones, so skin tones stay natural.
// @tags vibrance, saturation, color, vivid, punch
// @param amount 0.5 -1 1
vec4 effect(vec4 src, vec2 uv) {
    vec3 c = src.rgb;
    float mx = max(c.r, max(c.g, c.b));
    float mn = min(c.r, min(c.g, c.b));
    float sat = mx - mn;
    float k = amount * (1.0 - sat);
    vec3 grey = vec3(luma(c));
    return vec4(clamp(mix(grey, c, 1.0 + k), 0.0, 1.0), src.a);
}
