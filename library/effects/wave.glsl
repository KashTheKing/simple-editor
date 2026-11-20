// @name Wave
// @desc Ripples the picture with moving sine waves, horizontally, vertically or both.
// @tags wave, ripple, distort, water, wobble
// @param amplitude 12 0 80
// @param wavelength 120 10 600
// @param speed 1 0 6
// @param direction 2 0 2
vec4 effect(vec4 src, vec2 uv) {
    vec2 px = uv * u_res;
    float a = amplitude * u_scale;
    float k = 6.28318531 / max(wavelength * u_scale, 1.0);
    float ph = u_time * speed * 6.28318531;
    vec2 d = vec2(0.0);
    if (direction < 0.5 || direction > 1.5) { d.x = a * sin(px.y * k + ph); }
    if (direction > 0.5) { d.y = a * sin(px.x * k + ph * 1.13); }
    return texture(tex, clamp((px + d) / u_res, 0.0, 1.0));
}
