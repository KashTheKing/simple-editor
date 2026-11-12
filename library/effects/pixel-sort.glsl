// @name Pixel Sort
// @desc Glitch-art pixel sorting: bright rows smear sideways in random bands, streaking toward the darker side.
// @tags glitch, pixel sort, smear, streak, datamosh
// @param threshold 0.38 0 1
// @param reach 0.5 0 1
// @param bands 0.7 0 1
// @param band_size 10 2 80
float luma_of(vec3 c) {
    return dot(c, vec3(0.299, 0.587, 0.114));
}
vec4 effect(vec4 src, vec2 uv) {
    float bandpx = max(band_size * u_scale, 1.0);
    float band = floor(uv.y * u_res.y / bandpx);
    float tt = floor(u_time * 6.0);                 // bands re-roll a few times a second
    float hb = hash12(vec2(band, tt));
    float on = step(1.0 - bands, hb);               // which bands are sorted
    float len = reach * (0.35 + 0.65 * hash12(vec2(band + 7.0, tt)));
    // every pixel in the band uses the same row, so streaks are clean horizontal lines
    float y = (band + 0.5) * bandpx / u_res.y;
    float l0 = luma_of(src.rgb);
    if (on < 0.5 || l0 < threshold) return src;
    vec3 best = src.rgb;
    float bestL = l0;
    float runOk = 1.0;
    float step_uv = len / 32.0;
    float jit = hash12(floor(uv * u_res)) * 0.9;
    for (int i = 1; i <= 32; i++) {
        float x = uv.x - (float(i) - jit) * step_uv;
        vec3 s = texture(tex, vec2(x, y)).rgb;
        float l = luma_of(s);
        // the run ends where the row drops below the threshold
        if (l < threshold) runOk = 0.0;
        float take = runOk * step(bestL, l * (1.0 - 0.012 * float(i)));
        best = mix(best, s, take);
        bestL = mix(bestL, l * (1.0 - 0.012 * float(i)), take);
    }
    // sorted look: darker-to-brighter ramp across the run
    float k = smoothstep(threshold, threshold + 0.1, l0);
    vec3 col = mix(src.rgb, best, k);
    return vec4(col, src.a);
}
