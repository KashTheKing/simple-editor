// @name Elastic Scale
// @desc Stretches the sides more than the middle, so 4:3 footage fills a wide frame without fattening faces.
// @tags elastic, stretch, scale, aspect, 4:3, widescreen, nonlinear
// @param center 0.6 0 1
// @param curve 0.5 0 1
vec4 effect(vec4 src, vec2 uv) {
    // output x in -1..1 -> source x: linear inside the centre band, increasingly squeezed outside it
    float x = uv.x * 2.0 - 1.0;
    float c = clamp(center, 0.0, 0.98);
    float ax = abs(x);
    float sx;
    if (ax <= c) {
        sx = ax * mix(1.0, 0.75, curve);
    } else {
        float f = (ax - c) / (1.0 - c);
        float inner = c * mix(1.0, 0.75, curve);
        sx = inner + (1.0 - inner) * pow(f, mix(1.0, 0.6, curve));
    }
    sx = sign(x) * sx;
    return texture(tex, vec2(sx * 0.5 + 0.5, uv.y));
}
