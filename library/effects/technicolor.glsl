// @name Technicolor
// @desc Two-strip and three-strip early colour film: rich reds and cyans, deep blacks.
// @tags technicolor, film, vintage, old film, color, two strip, retro
// @param strips 2 2 3
// @param amount 1 0 1
// @param contrast 1.15 0.8 1.6
vec4 effect(vec4 src, vec2 uv) {
    vec3 c = src.rgb;
    vec3 o;
    if (strips < 2.5) {
        // two records: red and a green/blue average; printed as red-orange and blue-green dyes
        float r = c.r;
        float g = (c.g + c.b) * 0.5;
        o = vec3(r, g, g) * vec3(1.0, 0.92, 0.82) + vec3(0.0, 0.02, 0.06) * (1.0 - r);
    } else {
        // three records with a dye cross-talk that deepens the primaries
        o = vec3(c.r - (c.g + c.b) * 0.25, c.g - (c.r + c.b) * 0.25, c.b - (c.r + c.g) * 0.25) * 1.5 + 0.12;
    }
    o = clamp((o - 0.5) * contrast + 0.5, 0.0, 1.0);
    return vec4(mix(c, o, amount), src.a);
}
