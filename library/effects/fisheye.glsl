// @name Fisheye
// @desc Fisheye lens: bulges the middle and bends the edges. Negative strength pinches instead.
// @tags lens, fisheye, distortion, barrel, wide
// @param strength 0.5 -1 1
vec4 effect(vec4 src, vec2 uv) {
    float asp = u_res.x / u_res.y;
    vec2 p = (uv - 0.5) * 2.0 * vec2(1.0, 1.0 / asp);
    float k = strength * 0.5;
    float rc = 1.0 + 1.0 / (asp * asp);
    // barrel zooms in just enough that the corners stay inside the picture
    float s = k > 0.0 ? 1.0 / (1.0 + k * rc) : 1.0;
    vec2 q = p * s * (1.0 + k * dot(p, p));
    vec2 suv = q * vec2(1.0, asp) * 0.5 + 0.5;
    vec4 c = sample_clamped(tex, suv);
    vec2 e = abs(suv - 0.5);
    float inside = 1.0 - smoothstep(0.5, 0.505, max(e.x, e.y));
    return vec4(c.rgb, c.a * inside);
}
