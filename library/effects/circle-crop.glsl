// @name Circle Crop
// @desc Crops the picture to a circle or oval, with a soft edge and a transparent or coloured outside.
// @tags circle, crop, oval, round, frame, mask, vignette
// @param radius 0.45 0 1
// @param feather 0.01 0 0.3
// @param center_x 0.5 0 1
// @param center_y 0.5 0 1
// @param oval 0 0 1
// @param fill 0 0 1
// @param hue 0 0 1
// @param light 0 0 1
vec4 effect(vec4 src, vec2 uv) {
    vec2 d = uv - vec2(center_x, center_y);
    if (oval < 0.5) { d.x *= u_res.x / u_res.y; }
    float r = length(d);
    float inside = 1.0 - smoothstep(radius - feather, radius + 1e-4, r);
    if (fill > 0.5) {
        vec3 bg = hsl2rgb(vec3(hue, 0.6, light));
        return vec4(mix(bg, src.rgb, inside), src.a);
    }
    return vec4(src.rgb, src.a * inside);
}
