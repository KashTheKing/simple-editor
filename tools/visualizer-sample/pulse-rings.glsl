// @name Pulse Rings
// @desc Custom audio visualizer sample: six neon rings, bass in the middle to treble outside, each swelling with its band.
// @tags visualizer, audio, rings, sample
// @param level 0.3 0 1
// @param band1 0.5 0 1
// @param band2 0.45 0 1
// @param band3 0.4 0 1
// @param band4 0.35 0 1
// @param band5 0.3 0 1
// @param band6 0.25 0 1
// @param hue 0.85 0 1
// A sample of a user-made visualizer style: save it with library.save (kind effect) or drop it into
// %APPDATA%\SimpleEditor\library\effects and it shows up in the Inspector's Visualizer > Style list.
vec4 effect(vec4 src, vec2 uv) {
    vec2 p = (uv - 0.5) * vec2(u_res.x / u_res.y, 1.0);
    float r = length(p);
    float b[6] = float[6](band1, band2, band3, band4, band5, band6);
    vec3 col = vec3(0.0);
    for (int i = 0; i < 6; i++) {
        float fi = float(i);
        float rad = 0.07 + 0.055 * fi + 0.03 * b[i];
        float w = 0.004 + 0.012 * b[i];
        float ring = exp(-pow((r - rad) / w, 2.0));
        vec3 c = hsl2rgb(vec3(fract(hue + 0.07 * fi), 0.9, 0.55));
        col += c * ring * (0.35 + 1.2 * b[i]);
    }
    col += hsl2rgb(vec3(hue, 0.9, 0.6)) * level * 0.35 * exp(-r * 6.0);
    return vec4(col, 1.0);
}
