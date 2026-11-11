// @name Swirl
// @desc Both pictures twist into a vortex around the centre, cross-fade inside the twist, then untwist.
// @tags swirl, twirl, vortex, spin, twist
// @param strength 7 0 20
// @param radius 0.9 0.2 2
// @param zoom 0.25 0 0.8
// @param center_x 0.5 0 1
// @param center_y 0.5 0 1

vec2 sw_mirror(vec2 u) {
    // mirrored repeat so rotated corners never smear the frame edge
    vec2 m = mod(u, 2.0);
    return 1.0 - abs(1.0 - m);
}

vec4 transition(vec2 uv) {
    if (progress <= 0.0) return getFromColor(uv);
    if (progress >= 1.0) return getToColor(uv);

    float s = progress;
    vec2 asp = vec2(ratio, 1.0);
    vec2 c = vec2(center_x, center_y);
    vec2 d = (uv - c) * asp;
    float r = length(d);

    float env = sin(3.14159265 * s);
    env = env * env * (3.0 - 2.0 * env);
    // the twist is strongest at the centre and fades out with the radius
    float fall = max(1.0 - r / radius, 0.0);
    fall = fall * fall * (3.0 - 2.0 * fall);
    // wind up toward the middle of the transition, unwind after it
    float a = strength * env * fall * fall;
    float cs = cos(a);
    float sn = sin(a);
    vec2 rd = mat2(cs, -sn, sn, cs) * d;
    // pulled in toward the eye of the vortex
    rd *= 1.0 - zoom * env * fall;
    vec2 suv = sw_mirror(c + rd / asp);

    // the centre swaps first, the outside follows
    float m = smoothstep(0.0, 1.0, clamp((s - 0.5) * 3.0 + 0.5 + (0.35 - r) * 0.8 * env, 0.0, 1.0));
    vec4 f = getFromColor(suv);
    vec4 t = getToColor(suv);
    vec4 col = mix(f, t, m);
    // a little depth: the eye of the vortex is slightly darker
    col.rgb *= 1.0 - 0.25 * env * fall * fall;
    return col;
}
