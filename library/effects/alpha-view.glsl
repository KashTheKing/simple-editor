// @name Alpha View
// @desc Shows the clip's transparency as black and white (white = opaque), to check a key or a cut-out.
// @tags alpha, matte, transparency, view, key, check, mask
// @param mode 0 0 2
vec4 effect(vec4 src, vec2 uv) {
    if (mode < 0.5) { return vec4(vec3(src.a), 1.0); }
    if (mode < 1.5) {
        // over a checkerboard
        vec2 c = floor(uv * u_res / (16.0 * u_scale));
        float chk = mix(0.4, 0.6, mod(c.x + c.y, 2.0));
        return vec4(mix(vec3(chk), src.rgb, src.a), 1.0);
    }
    // over red, like a matte overlay
    return vec4(mix(vec3(1.0, 0.0, 0.0), src.rgb, src.a), 1.0);
}
