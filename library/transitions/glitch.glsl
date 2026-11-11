// @name Glitch
// @desc Digital glitch: displaced blocks, RGB split and scanline tearing that peak at the midpoint where the pictures swap.
// @tags glitch, digital, rgb, distort, tear
// @param intensity 1 0 2
// @param split 0.025 0 0.08
vec4 gl_pic(vec2 c, float sw) {
    return sw > 0.5 ? getToColor(c) : getFromColor(c);
}

vec4 transition(vec2 uv) {
    float g = sin(3.14159265 * progress) * intensity;
    float f = floor(progress * 14.0);
    float n1 = mix(8.0, 18.0, hash11(f));
    float rid = floor(uv.y * n1);
    float act = step(0.5, hash12(vec2(rid, f)));
    float off = (hash12(vec2(rid, f + 17.0)) - 0.5) * 0.4 * g * act;
    float ty = fract(f * 0.37 + 0.1);
    off += (1.0 - smoothstep(0.0, 0.03, abs(uv.y - ty))) * 0.12 * g * (hash11(f + 3.0) - 0.5);
    float ln = floor(uv.y * u_res.y * 0.5);
    off += (hash11(ln + f * 13.0) - 0.5) * 0.01 * g;
    float thr = 0.42 + 0.16 * hash12(vec2(rid, 3.0));
    float sw = step(thr, progress);
    float sp = split * g * (0.5 + hash12(vec2(rid, f + 5.0)));
    vec2 q = vec2(uv.x + off, uv.y);
    vec4 cg = gl_pic(q, sw);
    float r = gl_pic(q + vec2(sp, 0.0), sw).r;
    float b = gl_pic(q - vec2(sp, 0.0), sw).b;
    vec4 col = vec4(r, cg.g, b, cg.a);
    // occasional blocky chunks, colour shifted
    vec2 cells = vec2(18.0 * ratio / 1.78, 12.0);
    vec2 cid = floor(uv * cells);
    float hb = hash12(cid + f * 7.1);
    if (hb > 1.0 - 0.1 * g) {
        vec4 bc = gl_pic((cid + 0.5) / cells + vec2(off, 0.0), sw);
        col = vec4(bc.gbr, bc.a);
    }
    return col;
}
