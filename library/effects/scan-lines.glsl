// @name Scan Lines
// @desc Dark horizontal lines across the picture, like an old TV or monitor, optionally rolling.
// @tags scan lines, scanlines, tv, crt, monitor, retro, lines
// @param spacing 4 2 20
// @param darkness 0.45 0 1
// @param roll 0 0 60
vec4 effect(vec4 src, vec2 uv) {
    float s = max(spacing * u_scale, 1.5);
    float y = uv.y * u_res.y + u_time * roll * u_scale;
    float l = 0.5 + 0.5 * cos(y / s * 6.28318531);
    return vec4(src.rgb * (1.0 - darkness * l), src.a);
}
