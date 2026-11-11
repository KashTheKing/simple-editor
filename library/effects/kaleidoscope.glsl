// @name Kaleidoscope
// @desc Mirrored wedges around the centre, slowly rotating.
// @tags kaleidoscope, mirror, symmetry, psychedelic, rotate
// @param segments 6 2 16
// @param speed 0.3 0 2
// @param zoom 2 0.5 4
float kal_fold(float x) {
    float t = mod(x, 2.0);
    return 1.0 - abs(t - 1.0);
}
vec4 effect(vec4 src, vec2 uv) {
    float asp = u_res.x / u_res.y;
    vec2 p = (uv - 0.5) * vec2(asp, 1.0);
    float r = length(p);
    float th = atan(p.y, p.x) + u_time * speed;
    float wedge = 6.2831853 / floor(segments + 0.5);
    th = abs(mod(th, wedge) - wedge * 0.5);
    vec2 q = vec2(cos(th), sin(th)) * r / zoom;
    vec2 suv = q / vec2(asp, 1.0) + 0.5;
    suv = vec2(kal_fold(suv.x), kal_fold(suv.y));
    return texture(tex, suv);
}
