// @name Soft Blur
// @desc Classic 2000s: a soft, even blur over the whole picture.
// @tags classic 2000s, blur, soft, defocus, retro
// @param radius 6 0 30
vec4 effect(vec4 src, vec2 uv) {
    vec2 px = radius * u_scale / u_res / 2.0;
    vec4 sum = vec4(0.0);
    float wsum = 0.0;
    for (int y = -3; y <= 3; y++) {
        for (int x = -3; x <= 3; x++) {
            vec2 o = vec2(float(x), float(y));
            float w = exp(-dot(o, o) / 8.0);
            sum += texture(tex, uv + o * px) * w;
            wsum += w;
        }
    }
    return sum / wsum;
}
