// @name Pixelate
// @desc The picture breaks into bigger and bigger square blocks up to the midpoint, swaps, then resolves again.
// @tags pixelate, mosaic, blocks, retro
// @param blocks 12 4 60
vec4 transition(vec2 uv) {
    float e = 1.0 - abs(2.0 * progress - 1.0);
    e = e * e * (3.0 - 2.0 * e);
    bool first = progress < 0.5;
    if (e < 0.003) return first ? getFromColor(uv) : getToColor(uv);
    float cy = exp(mix(log(u_res.y), log(blocks), e));
    vec2 cells = vec2(cy * ratio, cy);
    vec2 q = clamp((floor(uv * cells) + 0.5) / cells, 0.0, 1.0);
    return first ? getFromColor(q) : getToColor(q);
}
