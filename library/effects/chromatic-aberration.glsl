// @name Chromatic Aberration
// @desc Splits red and blue apart toward the edges of the frame, like a cheap lens.
// @tags lens, rgb, glitch, split, aberration
// @param amount 1.7 0 4
// @param falloff 1.6 0.5 4
vec4 effect(vec4 src, vec2 uv) {
    float asp = u_res.x / u_res.y;
    vec2 c = uv - 0.5;
    float r = length(c * vec2(asp, 1.0)) / length(vec2(asp, 1.0) * 0.5);
    vec2 d = c * amount * 0.03 * pow(r, falloff);
    vec3 col;
    col.r = texture(tex, uv + d).r;
    col.g = texture(tex, uv + d * 0.0).g;
    col.b = texture(tex, uv - d).b;
    // soften the split with a half-step sample so it is not three hard ghosts
    col.r = 0.5 * (col.r + texture(tex, uv + d * 0.5).r);
    col.b = 0.5 * (col.b + texture(tex, uv - d * 0.5).b);
    return vec4(col, src.a);
}
